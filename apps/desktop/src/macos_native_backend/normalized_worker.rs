//! Owner-bound, platform-neutral screen-only production worker.
//!
//! Platform modules adapt their native source to [`NativeScreenSource`]. This
//! worker then owns the shared capture session, bounded appsrc ingress,
//! lossless stop tail, and GStreamer/WebM graph without bypassing the
//! provider-neutral contracts.

use std::{
    sync::mpsc::{Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use frame_media::{
    BoundScreenCaptureSource, CancellationToken, CaptureQueueOverflow, CursorCaptureMode,
    CursorPolicy, ProtectedContentPolicy, ScreenCaptureIngress, ScreenCapturePhase,
    ScreenCaptureQueuePolicy, ScreenCaptureRequest, ScreenCaptureRequestSpec, ScreenCaptureSession,
    ScreenGracefulStopCompletionOutcome, ScreenIngressOutcome, ScreenOperationBudget,
    ScreenOperationExecutionError, ScreenPumpError, ScreenPumpOutcome, ScreenRecording,
    ScreenRecordingArtifact, ScreenRecordingPump, ScreenSessionFailureCode, ScreenSessionId,
    ScreenSessionIntent, ScreenSourceFailureCode, ScreenTargetDescriptor, ScreenTargetSnapshot,
    TargetRecoveryPolicy, VideoFrameSpec, negotiate_screen_capture,
};

use crate::NativeDesktopBackendError;

const SOURCE_CALL_TIMEOUT: Duration = Duration::from_secs(1);
const STOP_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const QUEUE_MAX_FRAMES: u16 = 3;
const QUEUE_MAX_AGE_NS: u64 = 500_000_000;
const IDLE_POLL: Duration = Duration::from_millis(2);

pub(crate) trait NativeScreenSource:
    frame_media::ScreenCaptureSource<FramePayload = Box<[u8]>, CursorImagePayload = Box<[u8]>>
    + Sized
    + 'static
{
    type RawSource: Send + 'static;
    type Diagnostics: Copy + Send + 'static;

    fn normalize(
        source: Self::RawSource,
        snapshot: ScreenTargetSnapshot,
    ) -> Result<Self, NativeDesktopBackendError>;
    fn native_is_running(&self) -> bool;
    fn diagnostics(&self) -> Self::Diagnostics;
    fn diagnostics_failed(baseline: Self::Diagnostics, current: Self::Diagnostics) -> bool;
    fn protected_content_policy() -> ProtectedContentPolicy;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerControl {
    Stop,
    Cancel,
}

pub(crate) struct WorkerCompletion {
    pub(crate) outcome: WorkerOutcome,
}

pub(crate) enum WorkerOutcome {
    Finished(CompletedRecordingArtifact),
    Cancelled,
    Failed {
        error: NativeDesktopBackendError,
        teardown_confirmed: bool,
    },
}

impl WorkerOutcome {
    pub(crate) const fn teardown_confirmed(&self) -> bool {
        match self {
            Self::Finished(_) | Self::Cancelled => true,
            Self::Failed {
                teardown_confirmed, ..
            } => *teardown_confirmed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletedRecordingArtifact {
    pub(crate) path: std::path::PathBuf,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
    pub(crate) duration_ns: u64,
}

impl From<ScreenRecordingArtifact> for CompletedRecordingArtifact {
    fn from(artifact: ScreenRecordingArtifact) -> Self {
        Self {
            path: artifact.path,
            bytes: artifact.bytes,
            sha256: artifact.sha256,
            duration_ns: artifact.end_pts_ns.saturating_sub(artifact.first_pts_ns),
        }
    }
}

pub(crate) struct ScreenWorkerStart<S: NativeScreenSource> {
    source: BoundScreenCaptureSource<S>,
    session: ScreenCaptureSession,
    ingress: ScreenCaptureIngress<Box<[u8]>, Box<[u8]>>,
    recording: ScreenRecording,
    diagnostic_baseline: S::Diagnostics,
}

pub(crate) struct ScreenWorkerSetupFailure {
    pub(super) error: NativeDesktopBackendError,
    pub(super) teardown_confirmed: bool,
}

impl<S: NativeScreenSource> ScreenWorkerStart<S> {
    pub(crate) fn prepare(
        source: S::RawSource,
        snapshot: ScreenTargetSnapshot,
        target: ScreenTargetDescriptor,
        frame_spec: VideoFrameSpec,
        recording: ScreenRecording,
        session_id: ScreenSessionId,
    ) -> Result<Self, ScreenWorkerSetupFailure> {
        let normalized = match S::normalize(source, snapshot.clone()) {
            Ok(normalized) => normalized,
            Err(error) => {
                return Err(setup_failure(recording, error, true));
            }
        };
        let diagnostic_baseline = normalized.diagnostics();
        let mut source = match BoundScreenCaptureSource::new(normalized, session_id) {
            Ok(source) => source,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Internal,
                    true,
                ));
            }
        };
        let Some(frame_bytes) = u64::from(frame_spec.width)
            .checked_mul(u64::from(frame_spec.height))
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return Err(setup_failure(
                recording,
                NativeDesktopBackendError::Internal,
                true,
            ));
        };
        let Some(queue_bytes) = frame_bytes.checked_mul(u64::from(QUEUE_MAX_FRAMES)) else {
            return Err(setup_failure(
                recording,
                NativeDesktopBackendError::Internal,
                true,
            ));
        };
        let cursor = match CursorPolicy::new(CursorCaptureMode::EmbeddedInFrame, false, false) {
            Ok(cursor) => cursor,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Internal,
                    true,
                ));
            }
        };
        let queue = match ScreenCaptureQueuePolicy::new(
            QUEUE_MAX_FRAMES,
            queue_bytes,
            QUEUE_MAX_AGE_NS,
            CaptureQueueOverflow::DropOldest,
        ) {
            Ok(queue) => queue,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Internal,
                    true,
                ));
            }
        };
        let request = ScreenCaptureRequest::new(ScreenCaptureRequestSpec {
            target,
            output: frame_spec,
            cursor,
            excluded_windows: Vec::new(),
            queue,
            recovery: TargetRecoveryPolicy::FailClosed,
            // ScreenCaptureKit does not expose an exact protected-content
            // signal. Blank/suspended native frames are surfaced as a
            // terminal ContentUnavailable failure instead of being mislabeled.
            protected_content: S::protected_content_policy(),
        });
        let request = match request {
            Ok(request) => request,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Internal,
                    true,
                ));
            }
        };
        let negotiated = match negotiate_screen_capture(source.capabilities(), &snapshot, request) {
            Ok(negotiated) => negotiated,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Unavailable,
                    true,
                ));
            }
        };
        let mut session = match ScreenCaptureSession::new(negotiated, source.binding()) {
            Ok(session) => session,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Internal,
                    true,
                ));
            }
        };
        let mut ingress = match ScreenCaptureIngress::new(&session) {
            Ok(ingress) => ingress,
            Err(_) => {
                return Err(setup_failure(
                    recording,
                    NativeDesktopBackendError::Internal,
                    true,
                ));
            }
        };
        let cancellation = CancellationToken::new();
        let budget = match operation_budget(&cancellation) {
            Ok(budget) => budget,
            Err(error) => {
                let capture_teardown_confirmed = !source.adapter().native_is_running();
                return Err(setup_failure(recording, error, capture_teardown_confirmed));
            }
        };
        let initial = session.initial_action();
        if let Err(error) =
            ingress.execute_control_action(&mut session, &initial, &mut source, &budget)
        {
            let primary = map_control_error(&error);
            let teardown_confirmed =
                teardown_setup(&mut source, &mut session, &mut ingress, recording);
            return Err(ScreenWorkerSetupFailure {
                error: primary,
                teardown_confirmed,
            });
        }
        if session.phase() != ScreenCapturePhase::Ready {
            let teardown_confirmed =
                teardown_setup(&mut source, &mut session, &mut ingress, recording);
            return Err(ScreenWorkerSetupFailure {
                error: NativeDesktopBackendError::PermissionDenied,
                teardown_confirmed,
            });
        }

        let mut start = match ingress.apply_intent(&mut session, ScreenSessionIntent::Start) {
            Ok(start) => start,
            Err(_) => {
                let teardown_confirmed =
                    teardown_setup(&mut source, &mut session, &mut ingress, recording);
                return Err(ScreenWorkerSetupFailure {
                    error: NativeDesktopBackendError::Internal,
                    teardown_confirmed,
                });
            }
        };
        let acknowledgement =
            match start
                .transition
                .action
                .execute_source(&session, &mut source, &budget)
            {
                Ok(Some(acknowledgement)) => acknowledgement,
                Ok(None) => {
                    let teardown_confirmed =
                        teardown_setup(&mut source, &mut session, &mut ingress, recording);
                    return Err(ScreenWorkerSetupFailure {
                        error: NativeDesktopBackendError::Internal,
                        teardown_confirmed,
                    });
                }
                Err(error) => {
                    let primary = map_operation_error(&error);
                    let teardown_confirmed =
                        teardown_setup(&mut source, &mut session, &mut ingress, recording);
                    return Err(ScreenWorkerSetupFailure {
                        error: primary,
                        teardown_confirmed,
                    });
                }
            };
        if ingress
            .complete_operation(&mut session, acknowledgement)
            .is_err()
            || session.phase() != ScreenCapturePhase::Capturing
        {
            let teardown_confirmed =
                teardown_setup(&mut source, &mut session, &mut ingress, recording);
            return Err(ScreenWorkerSetupFailure {
                error: NativeDesktopBackendError::Internal,
                teardown_confirmed,
            });
        }

        Ok(Self {
            source,
            session,
            ingress,
            recording,
            diagnostic_baseline,
        })
    }

    pub(super) fn run(self, control: Receiver<WorkerControl>) -> WorkerCompletion {
        let Self {
            mut source,
            mut session,
            mut ingress,
            recording,
            diagnostic_baseline,
        } = self;
        let plan = session.negotiated().ingress();
        let mut pump = match ScreenRecordingPump::new(recording, plan, &session, &mut ingress) {
            Ok(pump) => pump,
            Err(error) => {
                let capture_teardown_confirmed =
                    cancel_without_pump(&mut source, &mut session, &mut ingress);
                return WorkerCompletion {
                    outcome: WorkerOutcome::Failed {
                        error: map_pump_error(&error),
                        teardown_confirmed: capture_teardown_confirmed,
                    },
                };
            }
        };
        let cancellation = CancellationToken::new();
        let started = Instant::now();

        loop {
            match control.try_recv() {
                Ok(WorkerControl::Stop) => {
                    return WorkerCompletion {
                        outcome: finish(
                            &mut source,
                            &mut session,
                            pump,
                            diagnostic_baseline,
                            &cancellation,
                            started,
                        ),
                    };
                }
                Ok(WorkerControl::Cancel) | Err(TryRecvError::Disconnected) => {
                    return WorkerCompletion {
                        outcome: cancel(&mut source, &mut session, pump, diagnostic_baseline),
                    };
                }
                Err(TryRecvError::Empty) => {}
            }

            let now_ns = elapsed_ns(started);
            let budget = match operation_budget(&cancellation) {
                Ok(budget) => budget,
                Err(error) => {
                    return WorkerCompletion {
                        outcome: fail_active(
                            &mut source,
                            &mut session,
                            pump,
                            error,
                            diagnostic_baseline,
                        ),
                    };
                }
            };
            let polled =
                match pump.poll_source(&mut session, &mut source, &budget, now_ns, &cancellation) {
                    Ok(polled) => polled,
                    Err(mut error) => {
                        execute_pump_retirement(
                            &mut error,
                            &mut source,
                            &mut session,
                            &mut pump,
                            &budget,
                        );
                        let primary = map_pump_error(&error);
                        return WorkerCompletion {
                            outcome: fail_active(
                                &mut source,
                                &mut session,
                                pump,
                                primary,
                                diagnostic_baseline,
                            ),
                        };
                    }
                };
            if let Some(ScreenIngressOutcome::Session(mut transition)) = polled {
                let primary = map_terminal_phase(transition.transition.to);
                execute_retirement_action(
                    &mut transition.transition.action,
                    &mut source,
                    &mut session,
                    &mut pump,
                    &budget,
                );
                return WorkerCompletion {
                    outcome: fail_active(
                        &mut source,
                        &mut session,
                        pump,
                        primary,
                        diagnostic_baseline,
                    ),
                };
            }
            match pump.drain_available(&mut session, now_ns, &cancellation) {
                Ok(ScreenPumpOutcome::Drained(_)) => {}
                Ok(ScreenPumpOutcome::Cancelled { mut transition, .. }) => {
                    execute_retirement_action(
                        &mut transition.transition.action,
                        &mut source,
                        &mut session,
                        &mut pump,
                        &budget,
                    );
                    return WorkerCompletion {
                        outcome: fail_active(
                            &mut source,
                            &mut session,
                            pump,
                            NativeDesktopBackendError::Cancelled,
                            diagnostic_baseline,
                        ),
                    };
                }
                Err(mut error) => {
                    execute_pump_retirement(
                        &mut error,
                        &mut source,
                        &mut session,
                        &mut pump,
                        &budget,
                    );
                    let primary = map_pump_error(&error);
                    return WorkerCompletion {
                        outcome: fail_active(
                            &mut source,
                            &mut session,
                            pump,
                            primary,
                            diagnostic_baseline,
                        ),
                    };
                }
            }
            if polled.is_none() {
                thread::park_timeout(IDLE_POLL);
            }
        }
    }

    pub(super) fn teardown(self) -> bool {
        let Self {
            mut source,
            mut session,
            mut ingress,
            recording,
            ..
        } = self;
        teardown_setup(&mut source, &mut session, &mut ingress, recording)
    }
}

fn finish<S: NativeScreenSource>(
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    mut pump: ScreenRecordingPump<'_, Box<[u8]>, Box<[u8]>>,
    diagnostic_baseline: S::Diagnostics,
    cancellation: &CancellationToken,
    started: Instant,
) -> WorkerOutcome {
    let drain_deadline = Instant::now() + STOP_DRAIN_TIMEOUT;
    loop {
        match pump.drain_available(session, elapsed_ns(started), cancellation) {
            Ok(ScreenPumpOutcome::Drained(report)) if report.queue.queued_frames == 0 => break,
            Ok(ScreenPumpOutcome::Drained(_)) if Instant::now() < drain_deadline => {
                thread::park_timeout(IDLE_POLL);
            }
            Ok(ScreenPumpOutcome::Drained(_)) => {
                return fail_active(
                    source,
                    session,
                    pump,
                    NativeDesktopBackendError::Internal,
                    diagnostic_baseline,
                );
            }
            Ok(ScreenPumpOutcome::Cancelled { .. }) => {
                return fail_active(
                    source,
                    session,
                    pump,
                    NativeDesktopBackendError::Cancelled,
                    diagnostic_baseline,
                );
            }
            Err(error) => {
                let primary = map_pump_error(&error);
                return fail_active(source, session, pump, primary, diagnostic_baseline);
            }
        }
    }

    let mut stop = match pump.request_graceful_stop(session) {
        Ok(stop) => stop,
        Err(error) => {
            let primary = map_pump_error(&error);
            return fail_active(source, session, pump, primary, diagnostic_baseline);
        }
    };
    let budget = match operation_budget(cancellation) {
        Ok(budget) => budget,
        Err(error) => return fail_active(source, session, pump, error, diagnostic_baseline),
    };
    let acknowledgement = match stop.action_mut().execute_source(session, source, &budget) {
        Ok(Some(acknowledgement)) => acknowledgement,
        Ok(None) => {
            return fail_active(
                source,
                session,
                pump,
                NativeDesktopBackendError::Internal,
                diagnostic_baseline,
            );
        }
        Err(error) => {
            let primary = map_operation_error(&error);
            return fail_active(source, session, pump, primary, diagnostic_baseline);
        }
    };
    if let Err(error) =
        pump.drain_stopped_source_tail(session, source, &budget, elapsed_ns(started))
    {
        let primary = map_pump_error(&error);
        return fail_active(source, session, pump, primary, diagnostic_baseline);
    }
    let completion = match pump.complete_graceful_stop(session, stop, acknowledgement) {
        Ok(ScreenGracefulStopCompletionOutcome::Completed(completion)) => completion,
        Ok(ScreenGracefulStopCompletionOutcome::AbortOnly(_)) => {
            return fail_active(
                source,
                session,
                pump,
                NativeDesktopBackendError::Internal,
                diagnostic_baseline,
            );
        }
        Err(error) => {
            let primary = map_pump_error(&error);
            return fail_active(source, session, pump, primary, diagnostic_baseline);
        }
    };
    if S::diagnostics_failed(diagnostic_baseline, source.adapter().diagnostics()) {
        return fail_active(
            source,
            session,
            pump,
            NativeDesktopBackendError::Internal,
            diagnostic_baseline,
        );
    }
    match pump.finish(*completion, cancellation) {
        Ok(artifact) => WorkerOutcome::Finished(artifact.into()),
        Err(error) => WorkerOutcome::Failed {
            error: map_pump_error(&error),
            teardown_confirmed: !source.adapter().native_is_running(),
        },
    }
}

fn cancel<S: NativeScreenSource>(
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    mut pump: ScreenRecordingPump<'_, Box<[u8]>, Box<[u8]>>,
    diagnostic_baseline: S::Diagnostics,
) -> WorkerOutcome {
    let mut transition = match pump.cancel_session(session) {
        Ok(transition) => transition,
        Err(error) => {
            let primary = map_pump_error(&error);
            return fail_active(source, session, pump, primary, diagnostic_baseline);
        }
    };
    let cancellation = CancellationToken::new();
    let budget = match operation_budget(&cancellation) {
        Ok(budget) => budget,
        Err(error) => return fail_active(source, session, pump, error, diagnostic_baseline),
    };
    if !execute_retirement_action(
        &mut transition.transition.action,
        source,
        session,
        &mut pump,
        &budget,
    ) {
        return fail_active(
            source,
            session,
            pump,
            NativeDesktopBackendError::Internal,
            diagnostic_baseline,
        );
    }
    let graph_teardown_confirmed = pump.teardown().is_ok();
    let capture_teardown_confirmed = !source.adapter().native_is_running();
    if graph_teardown_confirmed
        && capture_teardown_confirmed
        && !S::diagnostics_failed(diagnostic_baseline, source.adapter().diagnostics())
    {
        WorkerOutcome::Cancelled
    } else {
        WorkerOutcome::Failed {
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: graph_teardown_confirmed && capture_teardown_confirmed,
        }
    }
}

fn fail_active<S: NativeScreenSource>(
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    mut pump: ScreenRecordingPump<'_, Box<[u8]>, Box<[u8]>>,
    primary: NativeDesktopBackendError,
    diagnostic_baseline: S::Diagnostics,
) -> WorkerOutcome {
    if source.adapter().native_is_running()
        && !session.phase().is_terminal()
        && let Ok(mut transition) = pump.cancel_session(session)
        && let Ok(budget) = operation_budget(&CancellationToken::new())
    {
        let _ = execute_retirement_action(
            &mut transition.transition.action,
            source,
            session,
            &mut pump,
            &budget,
        );
    }
    let graph_teardown_confirmed = pump.teardown().is_ok();
    let capture_teardown_confirmed = !source.adapter().native_is_running();
    if S::diagnostics_failed(diagnostic_baseline, source.adapter().diagnostics()) {
        eprintln!("Frame normalized capture worker observed terminal native diagnostics");
    }
    WorkerOutcome::Failed {
        error: primary,
        teardown_confirmed: graph_teardown_confirmed && capture_teardown_confirmed,
    }
}

fn execute_pump_retirement<S: NativeScreenSource>(
    error: &mut ScreenPumpError,
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    pump: &mut ScreenRecordingPump<'_, Box<[u8]>, Box<[u8]>>,
    budget: &ScreenOperationBudget<'_>,
) {
    let action = match error {
        ScreenPumpError::CancelledTeardown(failure) => {
            Some(&mut failure.transition.transition.action)
        }
        ScreenPumpError::TransitionTeardown { transition, .. } => {
            Some(&mut transition.transition.action)
        }
        ScreenPumpError::Terminal(failure) => Some(&mut failure.transition.transition.action),
        _ => None,
    };
    if let Some(action) = action {
        let _ = execute_retirement_action(action, source, session, pump, budget);
    }
}

fn execute_retirement_action<S: NativeScreenSource>(
    action: &mut frame_media::ScreenSessionAction,
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    pump: &mut ScreenRecordingPump<'_, Box<[u8]>, Box<[u8]>>,
    budget: &ScreenOperationBudget<'_>,
) -> bool {
    match action.execute_source(session, source, budget) {
        Ok(Some(acknowledgement)) => pump
            .complete_abort_operation(session, acknowledgement)
            .is_ok(),
        Ok(None) => !source.adapter().native_is_running(),
        Err(error) => {
            eprintln!("Frame normalized capture teardown failed: {error}");
            false
        }
    }
}

fn cancel_without_pump<S: NativeScreenSource>(
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    ingress: &mut ScreenCaptureIngress<Box<[u8]>, Box<[u8]>>,
) -> bool {
    if !source.adapter().native_is_running() {
        return true;
    }
    // Pump construction already consumed and explicitly aborted the graph, so
    // only native ownership remains here. The session action still mints the
    // exact stop ticket; no raw source escape hatch is used.
    if let Ok(mut transition) = ingress.cancel_session(session)
        && let Ok(budget) = operation_budget(&CancellationToken::new())
        && let Ok(Some(acknowledgement)) = transition
            .transition
            .action
            .execute_source(session, source, &budget)
    {
        let _ = ingress.complete_operation(session, acknowledgement);
    }
    !source.adapter().native_is_running()
}

fn teardown_setup<S: NativeScreenSource>(
    source: &mut BoundScreenCaptureSource<S>,
    session: &mut ScreenCaptureSession,
    ingress: &mut ScreenCaptureIngress<Box<[u8]>, Box<[u8]>>,
    recording: ScreenRecording,
) -> bool {
    if source.adapter().native_is_running()
        && let Ok(mut transition) = ingress.cancel_session(session)
        && let Ok(budget) = operation_budget(&CancellationToken::new())
        && let Ok(Some(acknowledgement)) = transition
            .transition
            .action
            .execute_source(session, source, &budget)
    {
        let _ = ingress.complete_operation(session, acknowledgement);
    }
    let capture_teardown_confirmed = !source.adapter().native_is_running();
    let recording_teardown_confirmed = recording.abort().is_ok();
    capture_teardown_confirmed && recording_teardown_confirmed
}

fn setup_failure(
    recording: ScreenRecording,
    error: NativeDesktopBackendError,
    capture_teardown_confirmed: bool,
) -> ScreenWorkerSetupFailure {
    ScreenWorkerSetupFailure {
        error,
        teardown_confirmed: capture_teardown_confirmed && recording.abort().is_ok(),
    }
}

fn operation_budget(
    cancellation: &CancellationToken,
) -> Result<ScreenOperationBudget<'_>, NativeDesktopBackendError> {
    ScreenOperationBudget::new(cancellation, SOURCE_CALL_TIMEOUT)
        .map_err(|_| NativeDesktopBackendError::Internal)
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn map_control_error(
    error: &frame_media::ScreenControlExecutionError,
) -> NativeDesktopBackendError {
    match error {
        frame_media::ScreenControlExecutionError::Source(failure) => {
            map_source_failure(failure.code())
        }
        frame_media::ScreenControlExecutionError::Contract(_) => {
            NativeDesktopBackendError::Internal
        }
    }
}

fn map_operation_error(error: &ScreenOperationExecutionError) -> NativeDesktopBackendError {
    match error {
        ScreenOperationExecutionError::Source(failure) => map_source_failure(failure.code()),
        ScreenOperationExecutionError::Contract(_)
        | ScreenOperationExecutionError::TicketConsumed
        | ScreenOperationExecutionError::StaleOperationAction
        | ScreenOperationExecutionError::FailureBindingMismatch => {
            NativeDesktopBackendError::Internal
        }
    }
}

const fn map_source_failure(code: ScreenSourceFailureCode) -> NativeDesktopBackendError {
    match code {
        ScreenSourceFailureCode::PermissionDenied
        | ScreenSourceFailureCode::PermissionRestricted => {
            NativeDesktopBackendError::PermissionDenied
        }
        ScreenSourceFailureCode::TargetLost | ScreenSourceFailureCode::ContentUnavailable => {
            NativeDesktopBackendError::TargetUnavailable
        }
        ScreenSourceFailureCode::AdapterUnavailable | ScreenSourceFailureCode::DeadlineExceeded => {
            NativeDesktopBackendError::Unavailable
        }
        ScreenSourceFailureCode::Cancelled => NativeDesktopBackendError::Cancelled,
        ScreenSourceFailureCode::ProtectedContent
        | ScreenSourceFailureCode::InvalidNativeFrame
        | ScreenSourceFailureCode::NativeOperationFailed => NativeDesktopBackendError::Internal,
    }
}

const fn map_terminal_phase(phase: ScreenCapturePhase) -> NativeDesktopBackendError {
    match phase {
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::Source(code)) => {
            map_source_failure(code)
        }
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::TargetLost)
        | ScreenCapturePhase::Failed(ScreenSessionFailureCode::RecoveryExhausted) => {
            NativeDesktopBackendError::TargetUnavailable
        }
        ScreenCapturePhase::Cancelled => NativeDesktopBackendError::Cancelled,
        ScreenCapturePhase::Failed(ScreenSessionFailureCode::ContractInvalidated)
        | ScreenCapturePhase::Failed(ScreenSessionFailureCode::ProtectedContent)
        | ScreenCapturePhase::AwaitingPreflight
        | ScreenCapturePhase::AwaitingPermissionRequest
        | ScreenCapturePhase::AwaitingPermissionResult
        | ScreenCapturePhase::Ready
        | ScreenCapturePhase::Starting
        | ScreenCapturePhase::Capturing
        | ScreenCapturePhase::Reconfiguring
        | ScreenCapturePhase::Suspended(_)
        | ScreenCapturePhase::Stopping
        | ScreenCapturePhase::Stopped => NativeDesktopBackendError::Internal,
    }
}

fn map_pump_error(error: &ScreenPumpError) -> NativeDesktopBackendError {
    match error {
        ScreenPumpError::StopTailSource { failure, .. } => map_source_failure(failure.code()),
        _ => NativeDesktopBackendError::Internal,
    }
}
