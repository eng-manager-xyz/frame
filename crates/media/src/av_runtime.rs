//! Bounded native A/V orchestration and the CPU-backed GStreamer appsrc edge.
//!
//! This module intentionally supports preview/runtime validation only. The
//! native stop acknowledgement does not yet authenticate a callback tail, so
//! callers must not treat this runtime as evidence that audio was losslessly
//! muxed into a final recording artifact. Poll reports currently coalesce only
//! privacy-safe source-status and timing events; GStreamer `level` messages and
//! mixed-audio/camera appsink output are not consumed by this slice.

use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use thiserror::Error;

use crate::{
    AV_DIAGNOSTIC_VERSION, AvActionExecution, AvAppSrcDownstreamFailure, AvAppSrcInput,
    AvAppSrcPushFailure, AvAppSrcRejection, AvBackpressurePolicy, AvCapabilityBucket,
    AvCaptureError, AvCaptureSession, AvDiagnostic, AvEventOutcome, AvFormat, AvLocalAppSrcAdapter,
    AvQueueSpec, AvSessionState, AvSourceClass, AvSourceStamp, AvStableCode, AvSyncPolicy,
    AvUiEvent, AvUiEventCoalescer, BoundNativeAvBridge, CalibrationConfidence,
    DEFAULT_UI_EVENT_INTERVAL_NS, MAX_AV_GRAPH_BUS_MESSAGES, MonotonicTimeNs, NativeAvBridge,
    NativeAvFailure, NativeAvFailureCode, NativeAvGraphFailure, NativeAvGraphState,
    NativeAvGraphTerminal, NativeAvGstreamerGraph, TimingBucket,
};

pub const MAX_AV_RUNTIME_NATIVE_EVENTS: u16 = 256;
pub const MAX_AV_RUNTIME_BUFFERS: u16 = 512;
pub const MAX_AV_RUNTIME_DIAGNOSTICS: u16 = 256;
pub const MAX_AV_RUNTIME_UI_EVENTS: usize = 11;
pub const DEFAULT_AV_RUNTIME_EOS_TIMEOUT_NS: u64 = 5_000_000_000;
pub const MAX_AV_RUNTIME_EOS_TIMEOUT_NS: u64 = 30_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvRuntimePolicy {
    pub max_native_events_per_poll: u16,
    pub max_buffers_per_poll: u16,
    pub max_bus_messages_per_poll: u16,
    pub max_diagnostics_per_poll: u16,
    pub ui_interval_ns: u64,
    pub eos_timeout_ns: u64,
}

impl Default for AvRuntimePolicy {
    fn default() -> Self {
        Self {
            max_native_events_per_poll: 64,
            max_buffers_per_poll: 64,
            max_bus_messages_per_poll: 64,
            max_diagnostics_per_poll: 64,
            ui_interval_ns: DEFAULT_UI_EVENT_INTERVAL_NS,
            eos_timeout_ns: DEFAULT_AV_RUNTIME_EOS_TIMEOUT_NS,
        }
    }
}

impl AvRuntimePolicy {
    pub fn validate(self) -> Result<Self, NativeAvRuntimeError> {
        if self.max_native_events_per_poll == 0
            || self.max_native_events_per_poll > MAX_AV_RUNTIME_NATIVE_EVENTS
            || self.max_buffers_per_poll == 0
            || self.max_buffers_per_poll > MAX_AV_RUNTIME_BUFFERS
            || self.max_bus_messages_per_poll == 0
            || self.max_bus_messages_per_poll > MAX_AV_GRAPH_BUS_MESSAGES
            || self.max_diagnostics_per_poll == 0
            || self.max_diagnostics_per_poll > MAX_AV_RUNTIME_DIAGNOSTICS
            || self.eos_timeout_ns == 0
            || self.eos_timeout_ns > MAX_AV_RUNTIME_EOS_TIMEOUT_NS
        {
            return Err(NativeAvRuntimeError::InvalidPolicy);
        }
        AvUiEventCoalescer::new(self.ui_interval_ns)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvRuntimeState {
    Playing,
    EosRequested,
    NullConfirmed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvRuntimeFailure {
    CaptureContract,
    Native(NativeAvFailureCode),
    Graph(NativeAvGraphFailure),
    AppSrcRejected(AvAppSrcRejection),
    AppSrcDownstream(AvAppSrcDownstreamFailure),
    EosDeadlineExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvGraphTeardown {
    NullReached,
    NullFailed(NativeAvGraphFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvSourceTeardown {
    Confirmed,
    NativeFailed(NativeAvFailureCode),
    ContractFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvRuntimeOutcome {
    Completed,
    Cancelled,
    Failed(NativeAvRuntimeFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAvTermination {
    pub outcome: NativeAvRuntimeOutcome,
    pub source_teardown: NativeAvSourceTeardown,
    pub graph_teardown: NativeAvGraphTeardown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvRuntimePollReport {
    pub native_events_polled: u16,
    pub buffers_pushed: u16,
    pub bus_messages_polled: u16,
    pub diagnostics: Vec<AvDiagnostic>,
    pub diagnostics_truncated: bool,
    pub ui_events: Vec<AvUiEvent>,
    pub more_work_possible: bool,
    pub termination: Option<NativeAvTermination>,
}

impl AvRuntimePollReport {
    fn empty() -> Self {
        Self {
            native_events_polled: 0,
            buffers_pushed: 0,
            bus_messages_polled: 0,
            diagnostics: Vec::new(),
            diagnostics_truncated: false,
            ui_events: Vec::new(),
            more_work_possible: false,
            termination: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum NativeAvRuntimeError {
    #[error("native A/V runtime policy is invalid")]
    InvalidPolicy,
    #[error("native A/V runtime requires an active recording session")]
    SessionNotRecording,
    #[error("native A/V session and GStreamer graph sources do not match")]
    SourceGraphMismatch,
    #[error("native A/V runtime transition is invalid")]
    InvalidTransition,
    #[error(
        "native A/V runtime attachment failed: {failure:?}; source teardown: {source_teardown:?}; graph teardown: {graph_teardown:?}"
    )]
    Attach {
        failure: NativeAvRuntimeFailure,
        source_teardown: NativeAvSourceTeardown,
        graph_teardown: NativeAvGraphTeardown,
    },
    #[error(
        "native A/V graph control failed: {failure:?}; source teardown: {source_teardown:?}; graph teardown: {teardown:?}"
    )]
    GraphControl {
        failure: NativeAvGraphFailure,
        source_teardown: NativeAvSourceTeardown,
        teardown: NativeAvGraphTeardown,
    },
    #[error(
        "native A/V source control failed: {failure:?}; source teardown: {source_teardown:?}; graph teardown: {teardown:?}"
    )]
    SourceControl {
        failure: NativeAvRuntimeFailure,
        source_teardown: NativeAvSourceTeardown,
        teardown: NativeAvGraphTeardown,
    },
    #[error(transparent)]
    Capture(#[from] AvCaptureError),
    #[error(transparent)]
    Native(#[from] NativeAvFailure),
}

struct GstOwnedAvInput(AvAppSrcInput);

impl AsRef<[u8]> for GstOwnedAvInput {
    fn as_ref(&self) -> &[u8] {
        self.0
            .payload()
            .bytes()
            .expect("invariant: CPU appsrc ownership is created only for byte payloads")
    }
}

/// A concrete CPU-copy appsrc edge.
///
/// The complete [`AvAppSrcInput`] is the backing owner of the GStreamer
/// buffer. Its native lease therefore survives until GStreamer releases that
/// buffer. Opaque payloads are rejected before constructing a GStreamer
/// buffer and the exact input is returned to the caller.
pub struct NativeAvAppSrc {
    appsrc: gst_app::AppSrc,
    downstream_queue: gst::Element,
    queue_overrun_handler: Option<gst::glib::SignalHandlerId>,
    queue_overruns: Arc<AtomicU64>,
    observed_queue_overruns: u64,
    pending_discontinuity: bool,
    overload_unreported: bool,
    stamp: AvSourceStamp,
    format: AvFormat,
}

impl std::fmt::Debug for NativeAvAppSrc {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeAvAppSrc")
            .field("stamp", &self.stamp)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl NativeAvAppSrc {
    pub fn new(
        appsrc: gst_app::AppSrc,
        downstream_queue: gst::Element,
        stamp: AvSourceStamp,
        format: AvFormat,
        expected_appsrc: AvQueueSpec,
        expected_downstream: AvQueueSpec,
    ) -> Result<Self, NativeAvRuntimeError> {
        format.validate_for(stamp.class())?;
        let expected_appsrc = expected_appsrc.validate()?;
        let expected_downstream = expected_downstream.validate()?;
        if !appsrc.property::<bool>("is-live")
            || appsrc.property::<bool>("do-timestamp")
            || appsrc.property::<bool>("block")
            || appsrc.format() != gst::Format::Time
            || appsrc.max_buffers() == 0
            || appsrc.max_bytes() == 0
            || appsrc.max_time().is_zero()
            || appsrc.leaky_type() != gst_app::AppLeakyType::Downstream
            || appsrc.max_buffers() != u64::from(expected_appsrc.max_buffers)
            || appsrc.max_bytes() != expected_appsrc.max_bytes
            || appsrc.max_time().nseconds() != expected_appsrc.max_age_ns
            || expected_appsrc.backpressure != AvBackpressurePolicy::DropOldest
            || expected_appsrc.producer_blocks
            || downstream_queue.find_property("max-size-buffers").is_none()
            || downstream_queue.find_property("max-size-bytes").is_none()
            || downstream_queue.find_property("max-size-time").is_none()
            || downstream_queue.find_property("leaky").is_none()
            || downstream_queue.find_property("silent").is_none()
            || downstream_queue.property::<u32>("max-size-buffers") == 0
            || downstream_queue.property::<u32>("max-size-bytes") == 0
            || downstream_queue.property::<u64>("max-size-time") == 0
            || downstream_queue.property::<u32>("max-size-buffers")
                != u32::from(expected_downstream.max_buffers)
            || u64::from(downstream_queue.property::<u32>("max-size-bytes"))
                != expected_downstream.max_bytes
            || downstream_queue.property::<u64>("max-size-time") != expected_downstream.max_age_ns
            || expected_downstream.backpressure != AvBackpressurePolicy::DropOldest
            || expected_downstream.producer_blocks
            || downstream_queue.property::<bool>("silent")
        {
            return Err(NativeAvRuntimeError::SourceGraphMismatch);
        }
        let leaky_value = downstream_queue.property_value("leaky");
        let (_, leaky_value) = gst::glib::EnumValue::from_value(&leaky_value)
            .ok_or(NativeAvRuntimeError::SourceGraphMismatch)?;
        if leaky_value.nick() != "downstream" {
            return Err(NativeAvRuntimeError::SourceGraphMismatch);
        }
        let queue_overruns = Arc::new(AtomicU64::new(0));
        let overrun_counter = Arc::clone(&queue_overruns);
        let queue_overrun_handler = downstream_queue.connect("overrun", false, move |_| {
            let _ = overrun_counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
            None
        });
        Ok(Self {
            appsrc,
            downstream_queue,
            queue_overrun_handler: Some(queue_overrun_handler),
            queue_overruns,
            observed_queue_overruns: 0,
            pending_discontinuity: false,
            overload_unreported: false,
            stamp,
            format,
        })
    }

    /// Samples bounded GStreamer pressure observations. One boolean coalesces
    /// any number of appsrc saturation events and queue overruns since the
    /// prior sample; the next transferred buffer is marked discontinuous.
    #[must_use]
    pub fn take_overload_observation(&mut self) -> bool {
        self.observe_overload();
        std::mem::take(&mut self.overload_unreported)
    }

    fn observe_overload(&mut self) {
        let queue_overruns = self.queue_overruns.load(Ordering::Relaxed);
        if queue_overruns > self.observed_queue_overruns {
            self.pending_discontinuity = true;
            self.overload_unreported = true;
        }
        self.observed_queue_overruns = queue_overruns;
    }

    fn appsrc_would_overflow(&self, input: &AvAppSrcInput) -> bool {
        // The minimum supported GStreamer 1.22 / v1_20 binding floor does not
        // expose an appsrc drop counter. This adapter is the sole producer, so
        // sampling its bounded queue immediately before a nonblocking push
        // cannot hide loss. A concurrent drain can only make this conservative
        // (an extra DISCONT), never fail open.
        self.appsrc.current_level_buffers() >= self.appsrc.max_buffers()
            || self
                .appsrc
                .current_level_bytes()
                .saturating_add(input.payload().retained_bytes())
                > self.appsrc.max_bytes()
            || self
                .appsrc
                .current_level_time()
                .nseconds()
                .saturating_add(input.timestamp().duration_ns)
                > self.appsrc.max_time().nseconds()
    }
}

impl Drop for NativeAvAppSrc {
    fn drop(&mut self) {
        if let Some(handler) = self.queue_overrun_handler.take() {
            self.downstream_queue.disconnect(handler);
        }
    }
}

impl AvLocalAppSrcAdapter for NativeAvAppSrc {
    fn push(&mut self, input: AvAppSrcInput) -> Result<(), AvAppSrcPushFailure> {
        if input.stamp() != self.stamp {
            return Err(AvAppSrcPushFailure::rejected(
                AvAppSrcRejection::SourceMismatch,
                input,
            ));
        }
        if input.format() != self.format {
            return Err(AvAppSrcPushFailure::rejected(
                AvAppSrcRejection::FormatMismatch,
                input,
            ));
        }
        if input.payload().bytes().is_none() {
            return Err(AvAppSrcPushFailure::rejected(
                AvAppSrcRejection::OpaquePayload,
                input,
            ));
        }
        if self.appsrc.current_state() != gst::State::Playing {
            return Err(AvAppSrcPushFailure::rejected(
                AvAppSrcRejection::RuntimeNotPlaying,
                input,
            ));
        }

        self.observe_overload();
        if self.appsrc_would_overflow(&input) {
            self.pending_discontinuity = true;
            self.overload_unreported = true;
        }
        let timestamp = input.timestamp();
        let mut buffer = gst::Buffer::from_slice(GstOwnedAvInput(input));
        let buffer_ref = buffer
            .get_mut()
            .expect("invariant: a newly constructed GStreamer buffer is uniquely owned");
        buffer_ref.set_pts(gst::ClockTime::from_nseconds(timestamp.pts_ns));
        buffer_ref.set_duration(gst::ClockTime::from_nseconds(timestamp.duration_ns));
        if timestamp.discontinuity || self.pending_discontinuity {
            buffer_ref.set_flags(gst::BufferFlags::DISCONT);
        }
        self.pending_discontinuity = false;
        self.appsrc
            .push_buffer(buffer)
            .map(|_| ())
            .map_err(|error| {
                AvAppSrcPushFailure::downstream(match error {
                    gst::FlowError::Flushing => AvAppSrcDownstreamFailure::Flushing,
                    gst::FlowError::Eos => AvAppSrcDownstreamFailure::EndOfStream,
                    gst::FlowError::NotNegotiated => AvAppSrcDownstreamFailure::NotNegotiated,
                    gst::FlowError::NotLinked => AvAppSrcDownstreamFailure::NotLinked,
                    gst::FlowError::NotSupported => AvAppSrcDownstreamFailure::NotSupported,
                    gst::FlowError::Error
                    | gst::FlowError::CustomError
                    | gst::FlowError::CustomError1
                    | gst::FlowError::CustomError2 => AvAppSrcDownstreamFailure::Fault,
                })
            })
    }
}

pub struct NativeAvRuntime<B: NativeAvBridge> {
    source: BoundNativeAvBridge<B>,
    session: AvCaptureSession,
    graph: NativeAvGstreamerGraph,
    appsrc: BTreeMap<AvSourceClass, NativeAvAppSrc>,
    ui: AvUiEventCoalescer,
    policy: AvRuntimePolicy,
    state: NativeAvRuntimeState,
    next_source_index: usize,
    last_time_ns: Option<u64>,
    eos_deadline_ns: Option<u64>,
}

impl<B: NativeAvBridge> std::fmt::Debug for NativeAvRuntime<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeAvRuntime")
            .field("session", &self.session)
            .field("graph", &self.graph)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl<B: NativeAvBridge> NativeAvRuntime<B> {
    pub fn attach(
        source: BoundNativeAvBridge<B>,
        session: AvCaptureSession,
        graph: NativeAvGstreamerGraph,
        sync_policy: AvSyncPolicy,
        policy: AvRuntimePolicy,
    ) -> Result<Self, NativeAvRuntimeError> {
        let mut runtime = Self {
            source,
            session,
            graph,
            appsrc: BTreeMap::new(),
            ui: AvUiEventCoalescer::new(DEFAULT_UI_EVENT_INTERVAL_NS)
                .expect("invariant: the default UI interval is valid"),
            policy,
            state: NativeAvRuntimeState::Failed,
            next_source_index: 0,
            last_time_ns: None,
            eos_deadline_ns: None,
        };
        let initialization = runtime.initialize(sync_policy);
        match initialization {
            Ok(()) => Ok(runtime),
            Err(failure) => {
                let source_teardown = runtime.quiesce_native();
                let graph_teardown = confirm_graph_null(&mut runtime.graph);
                Err(NativeAvRuntimeError::Attach {
                    failure,
                    source_teardown,
                    graph_teardown,
                })
            }
        }
    }

    #[must_use]
    pub const fn state(&self) -> NativeAvRuntimeState {
        self.state
    }

    #[must_use]
    pub const fn session(&self) -> &AvCaptureSession {
        &self.session
    }

    #[must_use]
    pub const fn graph(&self) -> &NativeAvGstreamerGraph {
        &self.graph
    }

    pub fn poll(
        &mut self,
        now: MonotonicTimeNs,
    ) -> Result<AvRuntimePollReport, NativeAvRuntimeError> {
        if !matches!(
            self.state,
            NativeAvRuntimeState::Playing | NativeAvRuntimeState::EosRequested
        ) {
            return Err(NativeAvRuntimeError::InvalidTransition);
        }
        let mut report = AvRuntimePollReport::empty();

        if self
            .last_time_ns
            .is_some_and(|last_time_ns| now.get() < last_time_ns)
        {
            report.termination = Some(self.fail(NativeAvRuntimeFailure::CaptureContract));
            self.finish_report_fail_closed(now, &mut report);
            return Ok(report);
        }
        self.last_time_ns = Some(now.get());
        if self.state == NativeAvRuntimeState::EosRequested {
            let deadline_expired = self
                .eos_deadline_ns
                .is_none_or(|deadline_ns| now.get() >= deadline_ns);
            if deadline_expired {
                report.termination = Some(self.fail(NativeAvRuntimeFailure::EosDeadlineExceeded));
                self.finish_report_fail_closed(now, &mut report);
                return Ok(report);
            }
        }
        if let Some(failure) = self.record_gstreamer_overloads(now, &mut report) {
            report.termination = Some(self.fail(failure));
            self.finish_report_fail_closed(now, &mut report);
            return Ok(report);
        }

        if self.state == NativeAvRuntimeState::Playing {
            if let Some(failure) = self.poll_native_events(now, &mut report) {
                report.termination = Some(self.fail(failure));
                self.finish_report_fail_closed(now, &mut report);
                return Ok(report);
            }
            if let Some(failure) = self.push_queued_buffers(now, &mut report) {
                report.termination = Some(self.fail(failure));
                self.finish_report_fail_closed(now, &mut report);
                return Ok(report);
            }
            if let Some(failure) = self.record_gstreamer_overloads(now, &mut report) {
                report.termination = Some(self.fail(failure));
                self.finish_report_fail_closed(now, &mut report);
                return Ok(report);
            }
        }

        match self.graph.poll_bus(self.policy.max_bus_messages_per_poll) {
            Ok(graph_report) => {
                report.bus_messages_polled = graph_report.messages_polled;
                report.more_work_possible |= graph_report.limit_reached;
                match graph_report.terminal {
                    Some(NativeAvGraphTerminal::EndOfStream) => {
                        if self.state != NativeAvRuntimeState::EosRequested {
                            report.termination = Some(self.fail(NativeAvRuntimeFailure::Graph(
                                NativeAvGraphFailure::PipelineError,
                            )));
                            self.finish_report_fail_closed(now, &mut report);
                            return Ok(report);
                        }
                        let teardown = confirm_graph_null(&mut self.graph);
                        self.eos_deadline_ns = None;
                        let outcome = if teardown == NativeAvGraphTeardown::NullReached {
                            self.state = NativeAvRuntimeState::NullConfirmed;
                            if self.ui.push(now, AvUiEvent::Stopped).is_ok() {
                                NativeAvRuntimeOutcome::Completed
                            } else {
                                self.state = NativeAvRuntimeState::Failed;
                                NativeAvRuntimeOutcome::Failed(
                                    NativeAvRuntimeFailure::CaptureContract,
                                )
                            }
                        } else {
                            self.state = NativeAvRuntimeState::Failed;
                            NativeAvRuntimeOutcome::Failed(NativeAvRuntimeFailure::Graph(
                                NativeAvGraphFailure::NullNotReached,
                            ))
                        };
                        report.termination = Some(NativeAvTermination {
                            outcome,
                            source_teardown: NativeAvSourceTeardown::Confirmed,
                            graph_teardown: teardown,
                        });
                    }
                    Some(NativeAvGraphTerminal::Failed(failure)) => {
                        report.termination =
                            Some(self.fail(NativeAvRuntimeFailure::Graph(failure)));
                    }
                    None => {}
                }
            }
            Err(failure) => {
                report.termination = Some(self.fail(NativeAvRuntimeFailure::Graph(failure)));
            }
        }
        self.finish_report_fail_closed(now, &mut report);
        Ok(report)
    }

    /// Stops the native preview source, then asks every appsrc to emit EOS.
    /// This is a bounded preview teardown, not a lossless recording-tail proof.
    pub fn request_stop(&mut self, now: MonotonicTimeNs) -> Result<(), NativeAvRuntimeError> {
        if self.state != NativeAvRuntimeState::Playing {
            return Err(NativeAvRuntimeError::InvalidTransition);
        }
        let deadline = now.get().checked_add(self.policy.eos_timeout_ns);
        if self
            .last_time_ns
            .is_some_and(|last_time_ns| now.get() < last_time_ns)
            || deadline.is_none()
        {
            let source_teardown = self.quiesce_native();
            let teardown = confirm_graph_null(&mut self.graph);
            self.state = NativeAvRuntimeState::Failed;
            return Err(NativeAvRuntimeError::SourceControl {
                failure: NativeAvRuntimeFailure::CaptureContract,
                source_teardown,
                teardown,
            });
        }
        self.last_time_ns = Some(now.get());
        let action = match self.session.request_stop() {
            Ok(action) => action,
            Err(error) => {
                let failure = capture_error_failure(error);
                let source_teardown = self.quiesce_native();
                let teardown = confirm_graph_null(&mut self.graph);
                self.state = NativeAvRuntimeState::Failed;
                return Err(NativeAvRuntimeError::SourceControl {
                    failure,
                    source_teardown,
                    teardown,
                });
            }
        };
        if let Some(action) = action
            && let Err(error) = execute_action(&mut self.session, &mut self.source, action)
        {
            let failure = runtime_error_failure(&error);
            let source_teardown = self.quiesce_native();
            let teardown = confirm_graph_null(&mut self.graph);
            self.state = NativeAvRuntimeState::Failed;
            return Err(NativeAvRuntimeError::SourceControl {
                failure,
                source_teardown,
                teardown,
            });
        }
        if let Err(failure) = self.graph.request_eos() {
            let teardown = confirm_graph_null(&mut self.graph);
            self.state = NativeAvRuntimeState::Failed;
            return Err(NativeAvRuntimeError::GraphControl {
                failure,
                source_teardown: NativeAvSourceTeardown::Confirmed,
                teardown,
            });
        }
        self.state = NativeAvRuntimeState::EosRequested;
        self.eos_deadline_ns = deadline;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<NativeAvTermination, NativeAvRuntimeError> {
        self.quiesce()
    }

    /// Makes one bounded best-effort attempt to quiesce native authority and
    /// confirm that the GStreamer graph reached `Null`.
    ///
    /// Unlike an ordinary product outcome, this cleanup operation may be
    /// retried after a prior failed teardown. A failed native attempt leaves
    /// the session's terminal request and unconfirmed stamps intact so a later
    /// call can reconcile the same authority rather than minting a new stop.
    /// The native adapter must honor the timeout carried by its operation
    /// ticket; safe Rust cannot preempt an adapter that violates that contract.
    pub fn quiesce(&mut self) -> Result<NativeAvTermination, NativeAvRuntimeError> {
        if !matches!(
            self.state,
            NativeAvRuntimeState::Playing
                | NativeAvRuntimeState::EosRequested
                | NativeAvRuntimeState::Failed
        ) {
            return Err(NativeAvRuntimeError::InvalidTransition);
        }
        Ok(self.quiesce_once())
    }

    fn quiesce_once(&mut self) -> NativeAvTermination {
        // Drop must never unwind because a native adapter or GStreamer wrapper
        // violated its no-panic contract. Each owner is isolated so a source
        // panic cannot prevent the independent graph from being driven to
        // Null. The resulting unconfirmed source authority stays sticky in the
        // session and is never reported as a successful cancellation.
        let source_teardown = catch_unwind(AssertUnwindSafe(|| self.quiesce_native()))
            .unwrap_or(NativeAvSourceTeardown::ContractFailed);
        let graph_teardown = catch_unwind(AssertUnwindSafe(|| confirm_graph_null(&mut self.graph)))
            .unwrap_or(NativeAvGraphTeardown::NullFailed(
                NativeAvGraphFailure::NullNotReached,
            ));
        self.eos_deadline_ns = None;
        let outcome = if source_teardown == NativeAvSourceTeardown::Confirmed
            && graph_teardown == NativeAvGraphTeardown::NullReached
        {
            self.state = NativeAvRuntimeState::NullConfirmed;
            NativeAvRuntimeOutcome::Cancelled
        } else {
            self.state = NativeAvRuntimeState::Failed;
            NativeAvRuntimeOutcome::Failed(match source_teardown {
                NativeAvSourceTeardown::Confirmed => match graph_teardown {
                    NativeAvGraphTeardown::NullReached => {
                        NativeAvRuntimeFailure::Graph(NativeAvGraphFailure::NullNotReached)
                    }
                    NativeAvGraphTeardown::NullFailed(failure) => {
                        NativeAvRuntimeFailure::Graph(failure)
                    }
                },
                NativeAvSourceTeardown::NativeFailed(code) => NativeAvRuntimeFailure::Native(code),
                NativeAvSourceTeardown::ContractFailed => NativeAvRuntimeFailure::CaptureContract,
            })
        };
        NativeAvTermination {
            outcome,
            source_teardown,
            graph_teardown,
        }
    }

    fn poll_native_events(
        &mut self,
        now: MonotonicTimeNs,
        report: &mut AvRuntimePollReport,
    ) -> Option<NativeAvRuntimeFailure> {
        while report.native_events_polled < self.policy.max_native_events_per_poll {
            let outcome = match self.session.poll_source(&mut self.source) {
                Ok(Some(outcome)) => outcome,
                Ok(None) => break,
                Err(AvCaptureError::Native(failure)) => {
                    return Some(NativeAvRuntimeFailure::Native(failure.code));
                }
                Err(_) => return Some(NativeAvRuntimeFailure::CaptureContract),
            };
            report.native_events_polled = report.native_events_polled.saturating_add(1);
            if let Err(failure) = self.record_outcome(now, outcome, report) {
                return Some(failure);
            }
        }
        report.more_work_possible |=
            report.native_events_polled == self.policy.max_native_events_per_poll;
        None
    }

    fn push_queued_buffers(
        &mut self,
        now: MonotonicTimeNs,
        report: &mut AvRuntimePollReport,
    ) -> Option<NativeAvRuntimeFailure> {
        while report.buffers_pushed < self.policy.max_buffers_per_poll {
            let mut made_progress = false;
            let classes = source_classes();
            let round_start = self.next_source_index;
            for offset in 0..classes.len() {
                if report.buffers_pushed >= self.policy.max_buffers_per_poll {
                    break;
                }
                let index = (round_start + offset) % classes.len();
                let class = classes[index];
                let buffer = match self.session.pop_buffer(class, now) {
                    Ok(Some(buffer)) => buffer,
                    Ok(None) => continue,
                    Err(_) => return Some(NativeAvRuntimeFailure::CaptureContract),
                };
                made_progress = true;
                let timing = buffer.timing();
                let Some(timestamp) = buffer.timestamp() else {
                    return Some(NativeAvRuntimeFailure::CaptureContract);
                };
                let Some(capture_master) =
                    timing.arrival.get().checked_sub(timing.latency.reported_ns)
                else {
                    return Some(NativeAvRuntimeFailure::CaptureContract);
                };
                let confidence = self
                    .session
                    .source_calibration(class)
                    .map_or(CalibrationConfidence::Low, |value| value.confidence);
                if self
                    .ui
                    .push(
                        now,
                        AvUiEvent::Timing {
                            class,
                            offset: TimingBucket::from_abs_ns(
                                capture_master.abs_diff(timestamp.pts_ns),
                            ),
                            confidence,
                        },
                    )
                    .is_err()
                {
                    return Some(NativeAvRuntimeFailure::CaptureContract);
                }
                let input = match buffer.into_appsrc_input() {
                    Ok(input) => input,
                    Err(_) => return Some(NativeAvRuntimeFailure::CaptureContract),
                };
                let Some(adapter) = self.appsrc.get_mut(&class) else {
                    input.release();
                    return Some(NativeAvRuntimeFailure::CaptureContract);
                };
                match adapter.push(input) {
                    Ok(()) => {
                        report.buffers_pushed = report.buffers_pushed.saturating_add(1);
                        self.next_source_index = (index + 1) % classes.len();
                    }
                    Err(failure) => {
                        let runtime_failure = match failure.rejection() {
                            Some(reason) => NativeAvRuntimeFailure::AppSrcRejected(reason),
                            None => NativeAvRuntimeFailure::AppSrcDownstream(
                                failure
                                    .downstream_code()
                                    .expect("invariant: appsrc failure has one classified phase"),
                            ),
                        };
                        if let Some(input) = failure.into_rejected_input() {
                            input.release();
                        }
                        return Some(runtime_failure);
                    }
                }
            }
            if !made_progress {
                break;
            }
        }
        report.more_work_possible |= report.buffers_pushed == self.policy.max_buffers_per_poll;
        None
    }

    fn record_outcome(
        &mut self,
        now: MonotonicTimeNs,
        outcome: AvEventOutcome,
        report: &mut AvRuntimePollReport,
    ) -> Result<(), NativeAvRuntimeFailure> {
        for class in outcome.disabled_sources {
            self.ui
                .push(
                    now,
                    AvUiEvent::SourceStatus {
                        class,
                        code: AvStableCode::OptionalSourceDisabled,
                    },
                )
                .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;
        }
        // A specific diagnostic for the same source is inserted last and
        // therefore wins the coalescer key over the generic disabled status.
        for diagnostic in outcome.diagnostics {
            if let Some(class) = diagnostic.class {
                self.ui
                    .push(
                        now,
                        AvUiEvent::SourceStatus {
                            class,
                            code: diagnostic.code,
                        },
                    )
                    .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;
            }
            if report.diagnostics.len() < usize::from(self.policy.max_diagnostics_per_poll) {
                report.diagnostics.push(diagnostic);
            } else {
                report.diagnostics_truncated = true;
            }
        }
        Ok(())
    }

    fn record_gstreamer_overloads(
        &mut self,
        now: MonotonicTimeNs,
        report: &mut AvRuntimePollReport,
    ) -> Option<NativeAvRuntimeFailure> {
        for class in source_classes() {
            let observed = self
                .appsrc
                .get_mut(&class)
                .is_some_and(NativeAvAppSrc::take_overload_observation);
            if !observed {
                continue;
            }
            if self
                .ui
                .push(
                    now,
                    AvUiEvent::SourceStatus {
                        class,
                        code: AvStableCode::IngressOverload,
                    },
                )
                .is_err()
            {
                return Some(NativeAvRuntimeFailure::CaptureContract);
            }
            let Some(format) = self.session.source_format(class) else {
                return Some(NativeAvRuntimeFailure::CaptureContract);
            };
            let diagnostic = AvDiagnostic {
                version: AV_DIAGNOSTIC_VERSION,
                class: Some(class),
                route: None,
                capability: Some(AvCapabilityBucket::from_format(format)),
                timing: None,
                code: AvStableCode::IngressOverload,
            };
            if report.diagnostics.len() < usize::from(self.policy.max_diagnostics_per_poll) {
                report.diagnostics.push(diagnostic);
            } else {
                report.diagnostics_truncated = true;
            }
        }
        None
    }

    fn finish_report(
        &mut self,
        now: MonotonicTimeNs,
        report: &mut AvRuntimePollReport,
    ) -> Result<(), NativeAvRuntimeFailure> {
        report.ui_events = self
            .ui
            .drain_ready(now)
            .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;
        if report.ui_events.len() > MAX_AV_RUNTIME_UI_EVENTS {
            return Err(NativeAvRuntimeFailure::CaptureContract);
        }
        Ok(())
    }

    fn finish_report_fail_closed(
        &mut self,
        now: MonotonicTimeNs,
        report: &mut AvRuntimePollReport,
    ) {
        if let Err(failure) = self.finish_report(now, report) {
            if let Some(termination) = report.termination.as_mut() {
                termination.outcome = NativeAvRuntimeOutcome::Failed(failure);
                self.state = NativeAvRuntimeState::Failed;
            } else {
                report.termination = Some(self.fail(failure));
            }
            report.ui_events.clear();
        }
    }

    fn fail(&mut self, failure: NativeAvRuntimeFailure) -> NativeAvTermination {
        let source_teardown = self.quiesce_native();
        let graph_teardown = confirm_graph_null(&mut self.graph);
        self.eos_deadline_ns = None;
        self.state = NativeAvRuntimeState::Failed;
        NativeAvTermination {
            outcome: NativeAvRuntimeOutcome::Failed(failure),
            source_teardown,
            graph_teardown,
        }
    }

    fn initialize(&mut self, sync_policy: AvSyncPolicy) -> Result<(), NativeAvRuntimeFailure> {
        self.policy = self
            .policy
            .validate()
            .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;
        if self.session.state() != AvSessionState::Recording {
            return Err(NativeAvRuntimeFailure::CaptureContract);
        }
        if !self.graph.live_contract_is_intact() {
            return Err(NativeAvRuntimeFailure::CaptureContract);
        }
        self.ui = AvUiEventCoalescer::new(self.policy.ui_interval_ns)
            .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;

        for class in source_classes() {
            let session_budget = self.session.source_ingress_budget(class);
            let graph_budget = self.graph.source_ingress_budget(class);
            if session_budget != graph_budget {
                return Err(NativeAvRuntimeFailure::CaptureContract);
            }
            let partition = if let Some(total) = session_budget {
                let partition = total
                    .partition_ingress()
                    .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;
                if self.session.source_ingress_queue_spec(class) != Some(partition.session) {
                    return Err(NativeAvRuntimeFailure::CaptureContract);
                }
                Some(partition)
            } else {
                None
            };
            match (
                self.session.source_stamp(class),
                self.session.source_format(class),
                self.graph.source_appsrc(class),
                self.graph.source_queue(class),
                self.graph.source_format(class),
            ) {
                (None, None, None, None, None) => {}
                (
                    Some(stamp),
                    Some(format),
                    Some(gstreamer_source),
                    Some(downstream_queue),
                    Some(graph_format),
                ) if format == graph_format => {
                    let partition = partition.ok_or(NativeAvRuntimeFailure::CaptureContract)?;
                    let batch = self
                        .source
                        .startup_calibration(stamp)
                        .map_err(capture_error_failure)?;
                    self.session
                        .calibrate_source(stamp, sync_policy, batch.samples())
                        .map_err(capture_error_failure)?;
                    let adapter = NativeAvAppSrc::new(
                        gstreamer_source,
                        downstream_queue,
                        stamp,
                        format,
                        partition.appsrc,
                        partition.downstream,
                    )
                    .map_err(|_| NativeAvRuntimeFailure::CaptureContract)?;
                    self.appsrc.insert(class, adapter);
                }
                _ => return Err(NativeAvRuntimeFailure::CaptureContract),
            }
        }
        if self.appsrc.is_empty() {
            return Err(NativeAvRuntimeFailure::CaptureContract);
        }
        self.graph
            .start_playing()
            .map_err(NativeAvRuntimeFailure::Graph)?;
        self.state = NativeAvRuntimeState::Playing;
        Ok(())
    }

    fn quiesce_native(&mut self) -> NativeAvSourceTeardown {
        let action = match self.session.state() {
            AvSessionState::Stopped | AvSessionState::Cancelled => {
                return NativeAvSourceTeardown::Confirmed;
            }
            AvSessionState::Stopping | AvSessionState::TeardownRequired => {
                match self.session.retry_teardown() {
                    Ok(action) => action,
                    Err(_) => return NativeAvSourceTeardown::ContractFailed,
                }
            }
            _ => match self.session.cancel() {
                Ok(Some(action)) => action,
                Ok(None) => return NativeAvSourceTeardown::Confirmed,
                Err(_) => return NativeAvSourceTeardown::ContractFailed,
            },
        };
        let execution = match action.execute_source(&mut self.session, &mut self.source) {
            Ok(execution) => execution,
            Err(_) => return NativeAvSourceTeardown::ContractFailed,
        };
        match execution {
            AvActionExecution::Acknowledged(acknowledgement) => {
                if self.session.complete(acknowledgement).is_ok() {
                    NativeAvSourceTeardown::Confirmed
                } else {
                    NativeAvSourceTeardown::ContractFailed
                }
            }
            AvActionExecution::Failed(failure) => match self.session.complete_failure(failure) {
                Ok(failure) => NativeAvSourceTeardown::NativeFailed(failure.code),
                Err(_) => NativeAvSourceTeardown::ContractFailed,
            },
        }
    }
}

impl<B: NativeAvBridge> Drop for NativeAvRuntime<B> {
    fn drop(&mut self) {
        if matches!(
            self.state,
            NativeAvRuntimeState::Playing | NativeAvRuntimeState::EosRequested
        ) {
            let _ = self.quiesce_once();
        }
    }
}

fn execute_action<B: NativeAvBridge>(
    session: &mut AvCaptureSession,
    source: &mut BoundNativeAvBridge<B>,
    action: crate::AvSessionAction,
) -> Result<(), NativeAvRuntimeError> {
    match action.execute_source(session, source)? {
        AvActionExecution::Acknowledged(acknowledgement) => {
            session.complete(acknowledgement)?;
            Ok(())
        }
        AvActionExecution::Failed(failure) => {
            let failure = session.complete_failure(failure)?;
            Err(NativeAvRuntimeError::Native(failure))
        }
    }
}

fn confirm_graph_null(graph: &mut NativeAvGstreamerGraph) -> NativeAvGraphTeardown {
    match graph.confirm_null() {
        Ok(()) if graph.state() == NativeAvGraphState::Null => NativeAvGraphTeardown::NullReached,
        Ok(()) => NativeAvGraphTeardown::NullFailed(NativeAvGraphFailure::NullNotReached),
        Err(failure) => NativeAvGraphTeardown::NullFailed(failure),
    }
}

fn runtime_error_failure(error: &NativeAvRuntimeError) -> NativeAvRuntimeFailure {
    match error {
        NativeAvRuntimeError::Native(failure) => NativeAvRuntimeFailure::Native(failure.code),
        NativeAvRuntimeError::Attach { failure, .. } => *failure,
        NativeAvRuntimeError::GraphControl { failure, .. } => {
            NativeAvRuntimeFailure::Graph(*failure)
        }
        NativeAvRuntimeError::SourceControl { failure, .. } => *failure,
        NativeAvRuntimeError::InvalidPolicy
        | NativeAvRuntimeError::SessionNotRecording
        | NativeAvRuntimeError::SourceGraphMismatch
        | NativeAvRuntimeError::InvalidTransition
        | NativeAvRuntimeError::Capture(_) => NativeAvRuntimeFailure::CaptureContract,
    }
}

fn capture_error_failure(error: AvCaptureError) -> NativeAvRuntimeFailure {
    match error {
        AvCaptureError::Native(failure) => NativeAvRuntimeFailure::Native(failure.code),
        _ => NativeAvRuntimeFailure::CaptureContract,
    }
}

fn source_classes() -> [AvSourceClass; 3] {
    [
        AvSourceClass::Microphone,
        AvSourceClass::SystemAudio,
        AvSourceClass::Camera,
    ]
}
