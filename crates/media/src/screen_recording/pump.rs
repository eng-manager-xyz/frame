//! Bounded, exclusive owner-side pump from normalized capture ingress into appsrc.

use std::fmt;

use thiserror::Error;

use crate::{
    BoundScreenCaptureSource, CancellationToken, ScreenAppSrcPlan, ScreenCaptureError,
    ScreenCaptureIngress, ScreenCapturePhase, ScreenCaptureQueueDiagnostics, ScreenCaptureSession,
    ScreenCaptureSource, ScreenFrameAdmission, ScreenFramePayload, ScreenGracefulProofError,
    ScreenGracefulStop, ScreenGracefulStopAbort, ScreenGracefulStopAbortCompletion,
    ScreenGracefulStopCompletion, ScreenGracefulStopCompletionOutcome,
    ScreenGracefulStopRetryOutcome, ScreenIngressOutcome, ScreenIngressOwner,
    ScreenIngressPopOutcome, ScreenIngressTransition, ScreenOperationAck, ScreenOperationBudget,
    ScreenRecording, ScreenRecordingArtifact, ScreenRecordingError, ScreenRecordingIngressStatus,
    ScreenRecordingSpec, ScreenSourceFailureEnvelope, ScreenStreamStamp,
};

const MAX_STOP_TAIL_FRAMES: u16 = 16;

#[derive(Debug, Error)]
pub enum ScreenPumpError {
    #[error("the capture appsrc plan does not match the recording graph or live session")]
    InvalidPlan,
    #[error("the capture appsrc plan was invalid and recording teardown failed")]
    InvalidPlanAndTeardown { teardown: Box<ScreenRecordingError> },
    #[error("screen capture ingress rejected the pump operation")]
    Capture(#[from] ScreenCaptureError),
    #[error("screen capture ingress rejected pump construction and graph teardown failed")]
    CaptureAndTeardown {
        operation: Box<ScreenCaptureError>,
        teardown: Box<ScreenRecordingError>,
    },
    #[error("screen recording rejected an ingress frame")]
    Recording(#[source] ScreenRecordingError),
    #[error("screen recording failed and its graph teardown also failed")]
    RecordingAndTeardown {
        operation: Box<ScreenRecordingError>,
        teardown: Box<ScreenRecordingError>,
    },
    #[error("screen recording teardown failed")]
    Teardown(#[source] ScreenRecordingError),
    #[error("capture cancellation retired ingress but recording teardown failed")]
    CancelledTeardown(Box<ScreenPumpCancellationTeardown>),
    #[error("an authenticated capture transition retired the segment but teardown failed")]
    TransitionTeardown {
        transition: Box<ScreenIngressTransition>,
        teardown: Box<ScreenRecordingError>,
    },
    #[error("a graceful Stop became abort-only and graph teardown failed")]
    GracefulAbortTeardown {
        abort: Box<ScreenGracefulStopAbort>,
        teardown: Box<ScreenRecordingError>,
    },
    #[error("a consumed invalidated Stop acknowledgement could not tear down the graph")]
    InvalidatedCompletionTeardown {
        completion: Box<ScreenGracefulStopAbortCompletion>,
        teardown: Box<ScreenRecordingError>,
    },
    #[error("a graceful Stop proof correlation was rejected or failed")]
    GracefulStopProof(#[source] Box<ScreenGracefulProofError<ScreenGracefulStop>>),
    #[error("an abort-only Stop proof correlation was rejected or failed")]
    GracefulAbortProof(#[source] Box<ScreenGracefulProofError<ScreenGracefulStopAbort>>),
    #[error("the pump operation belongs to another capture segment")]
    OwnerChanged,
    #[error("recording finish authority was invalid and graph teardown failed")]
    AuthorityAndTeardown { teardown: Box<ScreenRecordingError> },
    #[error("the capture stream changed inside one recording segment")]
    StreamChanged,
    #[error("a terminal graph failure retired capture and forbids an artifact")]
    Terminal(Box<ScreenPumpTerminalFailure>),
    #[error("a terminal graph failure could not retire its capture session")]
    RetirementFailed(Box<ScreenPumpRetirementFailure>),
    #[error("the screen recording pump is no longer active")]
    InvalidLifecycle,
    #[error("the stopped source callback tail failed")]
    StopTailSource {
        failure: Box<ScreenSourceFailureEnvelope>,
        teardown: ScreenPumpTeardownStatus,
    },
    #[error("the stopped source exceeded the bounded callback tail")]
    StopTailExceeded { teardown: ScreenPumpTeardownStatus },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenPumpReport {
    pub popped_frames: u16,
    pub submitted_bytes: u64,
    pub submitted_discontinuities: u16,
    pub total_submitted_frames: u64,
    pub reached_iteration_bound: bool,
    pub queue: ScreenCaptureQueueDiagnostics,
    pub last_ingress: Option<ScreenRecordingIngressStatus>,
}

#[derive(Debug)]
pub struct ScreenPumpCancellationTeardown {
    pub transition: Box<ScreenIngressTransition>,
    pub report: ScreenPumpReport,
    pub teardown: Box<ScreenRecordingError>,
}

#[derive(Debug)]
pub enum ScreenPumpTeardownStatus {
    Confirmed,
    Unconfirmed(Box<ScreenRecordingError>),
}

#[derive(Debug)]
pub struct ScreenPumpTerminalFailure {
    pub transition: Box<ScreenIngressTransition>,
    pub operation: Box<ScreenRecordingError>,
    pub teardown: ScreenPumpTeardownStatus,
}

#[derive(Debug)]
pub struct ScreenPumpRetirementFailure {
    pub operation: Box<ScreenRecordingError>,
    pub retirement: Box<ScreenCaptureError>,
    pub teardown: ScreenPumpTeardownStatus,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScreenPumpOutcome {
    Drained(ScreenPumpReport),
    Cancelled {
        transition: Box<ScreenIngressTransition>,
        report: ScreenPumpReport,
    },
}

/// Owns the only appsrc recording graph and an exclusive mutable claim over
/// one exact capture ingress for its whole lifetime. Each drain performs
/// bounded work and peeks actual frame allocation/timing before transferring
/// the lease out of the upstream queue.
///
/// A second pump, or a competing direct `try_pop`, cannot be constructed in
/// safe code while the first pump is live:
///
/// ```compile_fail
/// use frame_media::{
///     ScreenAppSrcPlan, ScreenCaptureIngress, ScreenCaptureSession,
///     ScreenRecording, ScreenRecordingPump,
/// };
///
/// fn duplicate<'a>(
///     ingress: &'a mut ScreenCaptureIngress<Box<[u8]>, Box<[u8]>>,
///     session: &ScreenCaptureSession,
///     plan: ScreenAppSrcPlan,
///     first_graph: ScreenRecording,
///     second_graph: ScreenRecording,
/// ) {
///     let first = ScreenRecordingPump::new(first_graph, plan, session, ingress).unwrap();
///     let second = ScreenRecordingPump::new(second_graph, plan, session, ingress).unwrap();
///     drop((first, second));
/// }
/// ```
pub struct ScreenRecordingPump<'ingress, FramePayload, CursorImagePayload>
where
    FramePayload: ScreenFramePayload,
    CursorImagePayload: ScreenFramePayload,
{
    ingress: &'ingress mut ScreenCaptureIngress<FramePayload, CursorImagePayload>,
    recording: Option<ScreenRecording>,
    max_frames_per_drain: u16,
    owner: ScreenIngressOwner,
    active_stream: ScreenStreamStamp,
    last_sequence: Option<u64>,
    submitted_frames: u64,
    accepting_frames: bool,
    sealing_stream: Option<ScreenStreamStamp>,
}

impl<FramePayload, CursorImagePayload> fmt::Debug
    for ScreenRecordingPump<'_, FramePayload, CursorImagePayload>
where
    FramePayload: ScreenFramePayload,
    CursorImagePayload: ScreenFramePayload,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenRecordingPump")
            .field("active", &self.recording.is_some())
            .field("max_frames_per_drain", &self.max_frames_per_drain)
            .field("stream_active", &true)
            .field("last_sequence", &self.last_sequence)
            .field("submitted_frames", &self.submitted_frames)
            .field("accepting_frames", &self.accepting_frames)
            .field("sealing", &self.sealing_stream.is_some())
            .finish()
    }
}

impl<'ingress, FramePayload, CursorImagePayload>
    ScreenRecordingPump<'ingress, FramePayload, CursorImagePayload>
where
    FramePayload: ScreenFramePayload,
    CursorImagePayload: ScreenFramePayload,
{
    pub fn new(
        mut recording: ScreenRecording,
        plan: ScreenAppSrcPlan,
        session: &ScreenCaptureSession,
        ingress: &'ingress mut ScreenCaptureIngress<FramePayload, CursorImagePayload>,
    ) -> Result<Self, ScreenPumpError> {
        let expected = ScreenRecordingSpec::from_appsrc_plan(plan);
        let plan_matches = expected.as_ref().is_ok_and(|expected| {
            *expected == recording.spec() && plan == session.negotiated().ingress()
        });
        if !plan_matches || !recording.is_pristine_running() {
            return match recording.abort() {
                Ok(()) => Err(ScreenPumpError::InvalidPlan),
                Err(teardown) => Err(ScreenPumpError::InvalidPlanAndTeardown {
                    teardown: Box::new(teardown),
                }),
            };
        }
        let owner = match ingress.recording_owner(session) {
            Ok(owner) => owner,
            Err(operation) => {
                return match recording.abort() {
                    Ok(()) => Err(ScreenPumpError::Capture(operation)),
                    Err(teardown) => Err(ScreenPumpError::CaptureAndTeardown {
                        operation: Box::new(operation),
                        teardown: Box::new(teardown),
                    }),
                };
            }
        };
        let Some(active_stream) = owner.active_stream() else {
            return match recording.abort() {
                Ok(()) => Err(ScreenPumpError::Capture(
                    ScreenCaptureError::UnexpectedSourceData,
                )),
                Err(teardown) => Err(ScreenPumpError::CaptureAndTeardown {
                    operation: Box::new(ScreenCaptureError::UnexpectedSourceData),
                    teardown: Box::new(teardown),
                }),
            };
        };
        let downstream_max =
            u16::try_from(recording.spec().ingress_max_frames()).unwrap_or(u16::MAX);
        Ok(Self {
            ingress,
            recording: Some(recording),
            max_frames_per_drain: plan.queue.max_frames().min(downstream_max),
            owner,
            active_stream,
            last_sequence: None,
            submitted_frames: 0,
            accepting_frames: true,
            sealing_stream: None,
        })
    }

    pub fn drain_available(
        &mut self,
        session: &mut ScreenCaptureSession,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenPumpOutcome, ScreenPumpError> {
        self.drain_queue(session, now_ns, cancellation)
    }

    fn drain_queue(
        &mut self,
        session: &mut ScreenCaptureSession,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenPumpOutcome, ScreenPumpError> {
        if self.recording.is_none() || !self.accepting_frames {
            return Err(ScreenPumpError::InvalidLifecycle);
        }
        if self.ingress.recording_owner(session)? != self.owner {
            return Err(ScreenPumpError::OwnerChanged);
        }
        let mut report = ScreenPumpReport {
            total_submitted_frames: self.submitted_frames,
            queue: self.ingress.queue_diagnostics(),
            ..ScreenPumpReport::default()
        };
        for index in 0..self.max_frames_per_drain {
            if !cancellation.is_cancelled() {
                let Some(admission) = self.ingress.peek_next_frame(session, now_ns)? else {
                    report.queue = self.ingress.queue_diagnostics();
                    return Ok(ScreenPumpOutcome::Drained(report));
                };
                if admission.retained_bytes != self.recording_spec()?.frame_bytes() {
                    return Err(self.terminalize_recording_failure(
                        session,
                        ScreenRecordingError::InvalidFrame,
                    ));
                }
                if !self.recording_can_accept(admission)? {
                    report.queue = self.ingress.queue_diagnostics();
                    break;
                }
            }

            let outcome = self.ingress.try_pop(session, now_ns, cancellation)?;
            let queue_after_pop = self.ingress.queue_diagnostics();
            match outcome {
                ScreenIngressPopOutcome::Empty => {
                    report.queue = queue_after_pop;
                    return Ok(ScreenPumpOutcome::Drained(report));
                }
                ScreenIngressPopOutcome::Cancelled(transition) => {
                    if transition.owner() != self.owner {
                        return Err(ScreenPumpError::OwnerChanged);
                    }
                    report.queue = queue_after_pop;
                    return match self.abort_active_recording() {
                        Ok(()) => Ok(ScreenPumpOutcome::Cancelled { transition, report }),
                        Err(teardown) => Err(ScreenPumpError::CancelledTeardown(Box::new(
                            ScreenPumpCancellationTeardown {
                                transition,
                                report,
                                teardown: Box::new(teardown),
                            },
                        ))),
                    };
                }
                ScreenIngressPopOutcome::Frame(mut frame) => {
                    if frame.stream() != self.active_stream {
                        return Err(self.terminalize_recording_failure(
                            session,
                            ScreenRecordingError::InvalidFrame,
                        ));
                    }
                    let prefix_was_dropped = self.last_sequence.is_none()
                        && (queue_after_pop.dropped_oldest > 0
                            || queue_after_pop.dropped_expired > 0
                            || queue_after_pop.dropped_oversized > 0);
                    let sequence_gap = self
                        .last_sequence
                        .and_then(|sequence| sequence.checked_add(1))
                        .is_some_and(|expected| expected != frame.sequence());
                    if prefix_was_dropped || sequence_gap {
                        frame.force_discontinuity();
                    }
                    let sequence = frame.sequence();
                    let retained_bytes = frame.retained_bytes();
                    let discontinuity = frame.timestamp().discontinuity;
                    let status = match self.recording_mut()?.push_screen_frame(frame) {
                        Ok(status) => status,
                        Err(operation) => {
                            return Err(self.terminalize_recording_failure(session, operation));
                        }
                    };
                    self.last_sequence = Some(sequence);
                    self.submitted_frames = status.submitted_frames;
                    report.popped_frames = report.popped_frames.saturating_add(1);
                    report.submitted_bytes = report.submitted_bytes.saturating_add(retained_bytes);
                    if discontinuity {
                        report.submitted_discontinuities =
                            report.submitted_discontinuities.saturating_add(1);
                    }
                    report.total_submitted_frames = self.submitted_frames;
                    report.last_ingress = Some(status);
                    report.reached_iteration_bound = index + 1 == self.max_frames_per_drain;
                    report.queue = queue_after_pop;
                    if status.at_capacity {
                        break;
                    }
                }
            }
        }
        Ok(ScreenPumpOutcome::Drained(report))
    }

    /// Ingests the finite callback tail retained by a source after native Stop
    /// succeeded but before its acknowledgement is applied to the session.
    /// The frames have already passed the source's bounded callback queue, so
    /// they are owner- and stream-validated and submitted directly without
    /// entering the capture queue that the stop transition intentionally
    /// retired.
    pub fn drain_stopped_source_tail<S>(
        &mut self,
        session: &mut ScreenCaptureSession,
        source: &mut BoundScreenCaptureSource<S>,
        budget: &ScreenOperationBudget<'_>,
        _now_ns: u64,
    ) -> Result<ScreenPumpReport, ScreenPumpError>
    where
        S: ScreenCaptureSource<
                FramePayload = FramePayload,
                CursorImagePayload = CursorImagePayload,
            >,
    {
        if self.recording.is_none()
            || self.accepting_frames
            || self.sealing_stream != Some(self.active_stream)
            || session.phase() != ScreenCapturePhase::Stopping
            || source.binding() != self.owner.source_session_binding()
        {
            return Err(ScreenPumpError::InvalidLifecycle);
        }

        let mut combined = ScreenPumpReport {
            total_submitted_frames: self.submitted_frames,
            queue: self.ingress.queue_diagnostics(),
            ..ScreenPumpReport::default()
        };
        for _ in 0..MAX_STOP_TAIL_FRAMES {
            let event = match source.poll_owned_stopped_event(budget) {
                Ok(event) => event,
                Err(failure) => {
                    let teardown = self.teardown_status();
                    return Err(ScreenPumpError::StopTailSource {
                        failure: Box::new(failure),
                        teardown,
                    });
                }
            };
            let Some(event) = event else {
                return Ok(combined);
            };
            let frame = match event.into_stopped_frame(self.owner.source_session_binding()) {
                Ok(frame) => frame,
                Err(error) => {
                    let teardown = self.teardown_status();
                    return Err(match teardown {
                        ScreenPumpTeardownStatus::Confirmed => ScreenPumpError::Capture(error),
                        ScreenPumpTeardownStatus::Unconfirmed(teardown) => {
                            ScreenPumpError::CaptureAndTeardown {
                                operation: Box::new(error),
                                teardown,
                            }
                        }
                    });
                }
            };
            if let Err(error) =
                self.ingress
                    .validate_stopped_tail_frame(session, self.active_stream, &frame)
            {
                let teardown = self.teardown_status();
                return Err(match teardown {
                    ScreenPumpTeardownStatus::Confirmed => ScreenPumpError::Capture(error),
                    ScreenPumpTeardownStatus::Unconfirmed(teardown) => {
                        ScreenPumpError::CaptureAndTeardown {
                            operation: Box::new(error),
                            teardown,
                        }
                    }
                });
            }
            let admission = ScreenFrameAdmission {
                retained_bytes: frame.retained_bytes(),
                duration_ns: frame.timestamp().duration_ns,
            };
            if !self.recording_can_accept(admission)? {
                return Err(
                    self.terminalize_recording_failure(session, ScreenRecordingError::Backpressure)
                );
            }
            let mut frame = frame;
            let sequence_gap = self
                .last_sequence
                .and_then(|sequence| sequence.checked_add(1))
                .is_some_and(|expected| expected != frame.sequence());
            if sequence_gap {
                frame.force_discontinuity();
            }
            let sequence = frame.sequence();
            let retained_bytes = frame.retained_bytes();
            let discontinuity = frame.timestamp().discontinuity;
            let status = match self.recording_mut()?.push_screen_frame(frame) {
                Ok(status) => status,
                Err(operation) => {
                    return Err(self.terminalize_recording_failure(session, operation));
                }
            };
            self.last_sequence = Some(sequence);
            self.submitted_frames = status.submitted_frames;
            combined.popped_frames = combined.popped_frames.saturating_add(1);
            combined.submitted_bytes = combined.submitted_bytes.saturating_add(retained_bytes);
            if discontinuity {
                combined.submitted_discontinuities =
                    combined.submitted_discontinuities.saturating_add(1);
            }
            combined.total_submitted_frames = self.submitted_frames;
            combined.last_ingress = Some(status);
        }

        match source.poll_owned_stopped_event(budget) {
            Ok(None) => Ok(combined),
            Ok(Some(_)) => {
                let teardown = self.teardown_status();
                Err(ScreenPumpError::StopTailExceeded { teardown })
            }
            Err(failure) => {
                let teardown = self.teardown_status();
                Err(ScreenPumpError::StopTailSource {
                    failure: Box::new(failure),
                    teardown,
                })
            }
        }
    }

    /// Polls through the exclusively owned ingress. Any authenticated epoch
    /// drain aborts the graph before the transition is returned to the caller.
    pub fn poll_source<S>(
        &mut self,
        session: &mut ScreenCaptureSession,
        source: &mut BoundScreenCaptureSource<S>,
        budget: &ScreenOperationBudget<'_>,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<Option<ScreenIngressOutcome>, ScreenPumpError>
    where
        S: ScreenCaptureSource<
                FramePayload = FramePayload,
                CursorImagePayload = CursorImagePayload,
            >,
    {
        let outcome = self
            .ingress
            .poll_source(session, source, budget, now_ns, cancellation)?;
        match outcome {
            Some(ScreenIngressOutcome::Session(transition)) if transition.drain.is_some() => {
                self.accepting_frames = false;
                if self.recording.is_some()
                    && let Err(teardown) = self.abort_active_recording()
                {
                    return Err(ScreenPumpError::TransitionTeardown {
                        transition,
                        teardown: Box::new(teardown),
                    });
                }
                Ok(Some(ScreenIngressOutcome::Session(transition)))
            }
            outcome => Ok(outcome),
        }
    }

    /// Requests the only transition that can seal this segment. The upstream
    /// queue must already be empty; the returned proof owns the exact Stop
    /// action and post-request seal epoch.
    pub fn request_graceful_stop(
        &mut self,
        session: &mut ScreenCaptureSession,
    ) -> Result<ScreenGracefulStop, ScreenPumpError> {
        if self.recording.is_none() || !self.accepting_frames {
            return Err(ScreenPumpError::InvalidLifecycle);
        }
        let stop = self.ingress.request_graceful_stop(session)?;
        self.accepting_frames = false;
        self.sealing_stream = Some(self.active_stream);
        Ok(stop)
    }

    pub fn retry_graceful_stop(
        &mut self,
        session: &mut ScreenCaptureSession,
        stop: ScreenGracefulStop,
        failure: ScreenSourceFailureEnvelope,
    ) -> Result<ScreenGracefulStopRetryOutcome, ScreenPumpError> {
        let outcome = self
            .ingress
            .retry_graceful_stop(session, stop, failure)
            .map_err(|error| ScreenPumpError::GracefulStopProof(Box::new(error)))?;
        if let ScreenGracefulStopRetryOutcome::AbortOnly(abort) = outcome {
            self.accepting_frames = false;
            if self.recording.is_none() {
                return Ok(ScreenGracefulStopRetryOutcome::AbortOnly(abort));
            }
            return match self.abort_active_recording() {
                Ok(()) => Ok(ScreenGracefulStopRetryOutcome::AbortOnly(abort)),
                Err(teardown) => Err(ScreenPumpError::GracefulAbortTeardown {
                    abort: Box::new(abort),
                    teardown: Box::new(teardown),
                }),
            };
        }
        Ok(outcome)
    }

    pub fn retry_graceful_abort(
        &mut self,
        session: &mut ScreenCaptureSession,
        abort: ScreenGracefulStopAbort,
        failure: ScreenSourceFailureEnvelope,
    ) -> Result<ScreenGracefulStopAbort, ScreenPumpError> {
        self.ingress
            .retry_graceful_abort(session, abort, failure)
            .map_err(|error| ScreenPumpError::GracefulAbortProof(Box::new(error)))
    }

    pub fn complete_graceful_abort(
        &mut self,
        session: &mut ScreenCaptureSession,
        abort: ScreenGracefulStopAbort,
        acknowledgement: ScreenOperationAck,
    ) -> Result<ScreenGracefulStopAbortCompletion, ScreenPumpError> {
        self.ingress
            .complete_graceful_abort(session, abort, acknowledgement)
            .map_err(|error| ScreenPumpError::GracefulAbortProof(Box::new(error)))
    }

    /// Applies a Stop acknowledgement retained by cancellation or another
    /// abort-only transition while the pump keeps its exclusive ingress claim.
    pub fn complete_abort_operation(
        &mut self,
        session: &mut ScreenCaptureSession,
        acknowledgement: ScreenOperationAck,
    ) -> Result<ScreenIngressTransition, ScreenPumpError> {
        self.ingress
            .complete_operation(session, acknowledgement)
            .map_err(ScreenPumpError::Capture)
    }

    pub fn complete_graceful_stop(
        &mut self,
        session: &mut ScreenCaptureSession,
        stop: ScreenGracefulStop,
        acknowledgement: ScreenOperationAck,
    ) -> Result<ScreenGracefulStopCompletionOutcome, ScreenPumpError> {
        let outcome = self
            .ingress
            .complete_graceful_stop(session, stop, acknowledgement)
            .map_err(|error| ScreenPumpError::GracefulStopProof(Box::new(error)))?;
        if let ScreenGracefulStopCompletionOutcome::AbortOnly(completion) = outcome {
            self.accepting_frames = false;
            if self.recording.is_none() {
                return Ok(ScreenGracefulStopCompletionOutcome::AbortOnly(completion));
            }
            return match self.abort_active_recording() {
                Ok(()) => Ok(ScreenGracefulStopCompletionOutcome::AbortOnly(completion)),
                Err(teardown) => Err(ScreenPumpError::InvalidatedCompletionTeardown {
                    completion: Box::new(completion),
                    teardown: Box::new(teardown),
                }),
            };
        }
        Ok(outcome)
    }

    /// Consumes the one-shot completion proof. It cannot be reused to publish
    /// a second graph:
    ///
    /// ```compile_fail
    /// use frame_media::{CancellationToken, ScreenGracefulStopCompletion, ScreenRecordingPump};
    ///
    /// fn reuse<Frame, Cursor>(
    ///     first: ScreenRecordingPump<'_, Frame, Cursor>,
    ///     second: ScreenRecordingPump<'_, Frame, Cursor>,
    ///     completion: ScreenGracefulStopCompletion,
    /// ) where
    ///     Frame: frame_media::ScreenFramePayload,
    ///     Cursor: frame_media::ScreenFramePayload,
    /// {
    ///     let cancellation = CancellationToken::new();
    ///     let _ = first.finish(completion, &cancellation);
    ///     let _ = second.finish(completion, &cancellation);
    /// }
    /// ```
    pub fn finish(
        mut self,
        completion: ScreenGracefulStopCompletion,
        cancellation: &CancellationToken,
    ) -> Result<ScreenRecordingArtifact, ScreenPumpError> {
        if self.accepting_frames
            || self.sealing_stream != Some(self.active_stream)
            || completion.owner() != self.owner
            || completion.stopped_stream() != self.active_stream
        {
            return Err(self.fail_authority());
        }
        let mut recording = self
            .recording
            .take()
            .ok_or(ScreenPumpError::InvalidLifecycle)?;
        if let Err(operation) = recording.end_of_stream() {
            return match recording.abort() {
                Ok(()) => Err(ScreenPumpError::Recording(operation)),
                Err(teardown) => Err(ScreenPumpError::RecordingAndTeardown {
                    operation: Box::new(operation),
                    teardown: Box::new(teardown),
                }),
            };
        }
        recording
            .finish(cancellation)
            .map_err(ScreenPumpError::Recording)
    }

    pub fn cancel_session(
        &mut self,
        session: &mut ScreenCaptureSession,
    ) -> Result<Box<ScreenIngressTransition>, ScreenPumpError> {
        let transition = Box::new(self.ingress.cancel_session(session)?);
        self.accepting_frames = false;
        if self.recording.is_none() {
            return Ok(transition);
        }
        match self.abort_active_recording() {
            Ok(()) => Ok(transition),
            Err(teardown) => Err(ScreenPumpError::TransitionTeardown {
                transition,
                teardown: Box::new(teardown),
            }),
        }
    }

    pub fn abort(mut self) -> Result<(), ScreenPumpError> {
        self.abort_active_recording()
            .map_err(ScreenPumpError::Teardown)
    }

    /// Confirms graph teardown after any pump operation, including operations
    /// that already aborted the graph as part of their failure path.
    ///
    /// Unlike [`Self::abort`], this cleanup boundary is deliberately
    /// idempotent so production workers can preserve a primary capture error
    /// without guessing whether a nested transition already retired appsrc.
    pub fn teardown(mut self) -> Result<(), ScreenPumpError> {
        let Some(recording) = self.recording.take() else {
            return Ok(());
        };
        recording.abort().map_err(ScreenPumpError::Teardown)
    }

    fn recording_spec(&self) -> Result<ScreenRecordingSpec, ScreenPumpError> {
        self.recording
            .as_ref()
            .map(ScreenRecording::spec)
            .ok_or(ScreenPumpError::InvalidLifecycle)
    }

    fn recording_mut(&mut self) -> Result<&mut ScreenRecording, ScreenPumpError> {
        self.recording
            .as_mut()
            .ok_or(ScreenPumpError::InvalidLifecycle)
    }

    fn recording_can_accept(
        &self,
        admission: ScreenFrameAdmission,
    ) -> Result<bool, ScreenPumpError> {
        let recording = self
            .recording
            .as_ref()
            .ok_or(ScreenPumpError::InvalidLifecycle)?;
        Ok(!super::would_exceed_ingress(
            recording.spec(),
            recording.ingress_levels(),
            admission.retained_bytes,
            admission.duration_ns,
        ))
    }

    fn abort_active_recording(&mut self) -> Result<(), ScreenRecordingError> {
        let recording = self
            .recording
            .take()
            .ok_or(ScreenRecordingError::InvalidLifecycle)?;
        recording.abort()
    }

    fn teardown_status(&mut self) -> ScreenPumpTeardownStatus {
        match self.abort_active_recording() {
            Ok(()) => ScreenPumpTeardownStatus::Confirmed,
            Err(error) => ScreenPumpTeardownStatus::Unconfirmed(Box::new(error)),
        }
    }

    fn terminalize_recording_failure(
        &mut self,
        session: &mut ScreenCaptureSession,
        operation: ScreenRecordingError,
    ) -> ScreenPumpError {
        let retirement = self.ingress.cancel_session(session);
        self.accepting_frames = false;
        let teardown = self.teardown_status();
        match retirement {
            Ok(transition) => ScreenPumpError::Terminal(Box::new(ScreenPumpTerminalFailure {
                transition: Box::new(transition),
                operation: Box::new(operation),
                teardown,
            })),
            Err(retirement) => {
                ScreenPumpError::RetirementFailed(Box::new(ScreenPumpRetirementFailure {
                    operation: Box::new(operation),
                    retirement: Box::new(retirement),
                    teardown,
                }))
            }
        }
    }

    fn fail_authority(&mut self) -> ScreenPumpError {
        match self.abort_active_recording() {
            Ok(()) => ScreenPumpError::OwnerChanged,
            Err(teardown) => ScreenPumpError::AuthorityAndTeardown {
                teardown: Box::new(teardown),
            },
        }
    }
}
