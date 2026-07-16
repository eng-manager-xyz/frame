use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use gst::prelude::*;
use gstreamer as gst;
use thiserror::Error;

use crate::{
    CancellationToken, LifecycleError, PIPELINE_PROTOCOL_VERSION, PipelineCommand, PipelineFault,
    PipelineLifecycle, PipelineState, RUNTIME_MANIFEST_VERSION, ReadyRuntime,
    pipeline_has_only_declared_authored_factories, pipeline_has_trusted_factory_provenance,
};

pub const PIPELINE_DIAGNOSTIC_SCHEMA_VERSION: u16 = 1;
pub const PIPELINE_COMMAND_CAPACITY: usize = 16;
pub const PIPELINE_EVENT_CAPACITY: usize = 128;

/// A bounded, low-cardinality identifier used to join pipeline diagnostics.
///
/// This deliberately accepts neither whitespace nor path separators so callers
/// cannot accidentally turn a device label, filename, or URL into telemetry.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PipelineCorrelationId(String);

impl PipelineCorrelationId {
    pub fn new(value: impl Into<String>) -> Result<Self, SupervisorError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(SupervisorError::InvalidCorrelationId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PipelineCorrelationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisorPolicy {
    /// Cooperative execution budget checked between plugin calls; bus polls
    /// and state confirmations never wait past it. Inline plugin callbacks are
    /// not preemptible. Null teardown has the separate
    /// `null_state_confirmation_timeout` budget after terminal state.
    pub deadline: Duration,
    pub poll_interval: Duration,
    pub stall_timeout: Duration,
    pub null_state_confirmation_timeout: Duration,
    pub state_change_confirmation_timeout: Duration,
    pub max_bus_messages: u64,
    pub max_warnings: u64,
    pub max_progress_buffers: u64,
    pub max_progress_bytes: u64,
    pub max_queue_buffers: u32,
    pub max_queue_bytes: u64,
    pub max_queue_time: Duration,
}

impl Default for SupervisorPolicy {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(15),
            poll_interval: Duration::from_millis(25),
            stall_timeout: Duration::from_secs(5),
            null_state_confirmation_timeout: Duration::from_secs(2),
            state_change_confirmation_timeout: Duration::from_secs(2),
            max_bus_messages: 20_000,
            max_warnings: 128,
            max_progress_buffers: 5_000_000,
            max_progress_bytes: 1024 * 1024 * 1024,
            max_queue_buffers: 1_024,
            max_queue_bytes: 256 * 1024 * 1024,
            max_queue_time: Duration::from_secs(10),
        }
    }
}

impl SupervisorPolicy {
    pub fn validate(self) -> Result<Self, SupervisorError> {
        if self.deadline.is_zero()
            || self.poll_interval.is_zero()
            || self.stall_timeout.is_zero()
            || self.null_state_confirmation_timeout.is_zero()
            || self.state_change_confirmation_timeout.is_zero()
            || self.poll_interval > self.deadline
            || self.stall_timeout > self.deadline
            || self.null_state_confirmation_timeout > self.deadline
            || self.state_change_confirmation_timeout > self.deadline
            || self.max_bus_messages == 0
            || self.max_warnings == 0
            || self.max_progress_buffers == 0
            || self.max_progress_bytes == 0
            || self.max_queue_buffers == 0
            || self.max_queue_bytes == 0
            || self.max_queue_time.is_zero()
        {
            return Err(SupervisorError::InvalidPolicy);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineTerminalOutcome {
    Completed,
    Failed(PipelineFault),
    Cancelled,
}

impl PipelineTerminalOutcome {
    #[must_use]
    pub const fn state(self) -> PipelineState {
        match self {
            Self::Completed => PipelineState::Completed,
            Self::Failed(_) => PipelineState::Failed,
            Self::Cancelled => PipelineState::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineStateDuration {
    pub state: PipelineState,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineTeardown {
    NullReached,
    NullStateFailed,
    NullStateUnconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineDiagnostics {
    pub schema_version: u16,
    pub protocol_version: u16,
    pub manifest_version: u16,
    pub correlation_id: PipelineCorrelationId,
    pub runtime_version: String,
    pub factories: Vec<String>,
    pub negotiated_media_types: Vec<String>,
    pub buffers_observed: u64,
    pub bytes_observed: u64,
    pub bus_messages: u64,
    pub warnings: u64,
    pub latency_messages: u64,
    pub pipeline_state_changes: u64,
    pub state_durations: Vec<PipelineStateDuration>,
    pub av_timing: Option<AvTimingDiagnostics>,
    pub queues: Vec<RuntimeQueueDiagnostics>,
    pub events_dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvTimingDiagnostics {
    pub audio_start_pts_ns: u64,
    pub video_start_pts_ns: u64,
    pub audio_end_pts_ns: u64,
    pub video_end_pts_ns: u64,
    /// `video - audio` at stream start.
    pub start_offset_ns: i64,
    /// `video - audio` at the final observed buffer boundary.
    pub end_offset_ns: i64,
    /// Change in A/V offset over the observed synthetic run.
    pub drift_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeQueueOverflow {
    Block,
    DropNewest,
    DropOldest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeQueueDiagnostics {
    pub index: u32,
    pub max_buffers: u32,
    pub max_bytes: u64,
    pub max_time_ns: u64,
    pub overflow: RuntimeQueueOverflow,
    pub peak_buffers: u32,
    pub peak_bytes: u64,
    pub peak_time_ns: u64,
    pub overrun_events: u64,
    pub underrun_events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineRunReport {
    pub outcome: PipelineTerminalOutcome,
    pub elapsed_ms: u64,
    pub teardown_elapsed_ms: u64,
    pub teardown: PipelineTeardown,
    pub diagnostics: PipelineDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineControlCommand {
    Pause,
    Resume,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineEvent {
    Transition(crate::PipelineTransition),
    CommandRejected {
        command: PipelineControlCommand,
        state: PipelineState,
    },
    Warning {
        count: u64,
    },
    Terminal(PipelineTerminalOutcome),
}

#[derive(Clone)]
pub struct PipelineControl {
    commands: SyncSender<PipelineControlCommand>,
    cancellation: CancellationToken,
}

impl fmt::Debug for PipelineControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PipelineControl")
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl PipelineControl {
    pub fn try_pause(&self) -> Result<(), PipelineControlError> {
        self.try_command(PipelineControlCommand::Pause)
    }

    pub fn try_resume(&self) -> Result<(), PipelineControlError> {
        self.try_command(PipelineControlCommand::Resume)
    }

    pub fn try_finish(&self) -> Result<(), PipelineControlError> {
        self.try_command(PipelineControlCommand::Finish)
    }

    pub fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }

    fn try_command(&self, command: PipelineControlCommand) -> Result<(), PipelineControlError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => PipelineControlError::QueueFull,
                TrySendError::Disconnected(_) => PipelineControlError::OwnerStopped,
            })
    }
}

pub struct PipelineTask {
    control: PipelineControl,
    events: Receiver<PipelineEvent>,
    join: Option<thread::JoinHandle<Result<PipelineRunReport, SupervisorError>>>,
}

impl fmt::Debug for PipelineTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PipelineTask")
            .field("control", &self.control)
            .field(
                "finished",
                &self
                    .join
                    .as_ref()
                    .is_none_or(thread::JoinHandle::is_finished),
            )
            .finish_non_exhaustive()
    }
}

impl PipelineTask {
    #[must_use]
    pub fn control(&self) -> PipelineControl {
        self.control.clone()
    }

    pub fn try_event(&self) -> Result<Option<PipelineEvent>, PipelineControlError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(PipelineControlError::OwnerStopped),
        }
    }

    pub fn event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<PipelineEvent>, PipelineControlError> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(PipelineControlError::OwnerStopped)
            }
        }
    }

    pub fn wait(mut self) -> Result<PipelineRunReport, SupervisorError> {
        let join = self
            .join
            .take()
            .ok_or(SupervisorError::OwnerThreadPanicked)?;
        join.join()
            .map_err(|_| SupervisorError::OwnerThreadPanicked)?
    }
}

impl Drop for PipelineTask {
    fn drop(&mut self) {
        let _ = self.control.cancel();
    }
}

impl PipelineRunReport {
    #[must_use]
    pub const fn completed(&self) -> bool {
        matches!(self.outcome, PipelineTerminalOutcome::Completed)
            && matches!(self.teardown, PipelineTeardown::NullReached)
    }
}

/// Owns a GStreamer graph, its bus, its lifecycle state, and a bounded progress
/// watchdog. Operational failures are returned as a terminal report rather
/// than an error so metrics remain available for every run.
pub struct PipelineSupervisor {
    pipeline: gst::Pipeline,
    bus: gst::Bus,
    lifecycle: PipelineLifecycle,
    policy: SupervisorPolicy,
    correlation_id: PipelineCorrelationId,
    progress_pad: gst::Pad,
    progress_buffers: Arc<AtomicU64>,
    progress_bytes: Arc<AtomicU64>,
    factories: Vec<String>,
    av_timing: Option<AvTimingProbes>,
    queue_probes: Vec<RuntimeQueueProbe>,
    events: Option<EventSink>,
    #[cfg(test)]
    before_message_classification: Option<Box<dyn FnMut(gst::MessageType) + Send>>,
    teardown_attempted: bool,
}

impl fmt::Debug for PipelineSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PipelineSupervisor")
            .field("state", &self.lifecycle.state())
            .field("policy", &self.policy)
            .field("correlation_id", &self.correlation_id)
            .field("factories", &self.factories)
            .finish_non_exhaustive()
    }
}

impl PipelineSupervisor {
    pub fn new(
        _runtime: &ReadyRuntime,
        pipeline: gst::Pipeline,
        progress_element_name: &'static str,
        correlation_id: PipelineCorrelationId,
        policy: SupervisorPolicy,
    ) -> Result<Self, SupervisorError> {
        let policy = policy.validate()?;
        if !pipeline_has_only_declared_authored_factories(&pipeline) {
            return Err(SupervisorError::UndeclaredAuthoredFactory);
        }
        if !pipeline_has_trusted_factory_provenance(&pipeline) {
            return Err(SupervisorError::UntrustedFactoryProvenance);
        }
        let bus = pipeline.bus().ok_or(SupervisorError::MissingBus)?;
        let progress_element = pipeline.by_name(progress_element_name).ok_or(
            SupervisorError::MissingProgressElement(progress_element_name),
        )?;
        let progress_pad = progress_element
            .static_pad("src")
            .ok_or(SupervisorError::MissingProgressPad)?;
        let progress_buffers = Arc::new(AtomicU64::new(0));
        let progress_bytes = Arc::new(AtomicU64::new(0));
        let callback_counter = Arc::clone(&progress_buffers);
        let callback_bytes = Arc::clone(&progress_bytes);
        progress_pad
            .add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if let Some(gst::PadProbeData::Buffer(buffer)) = info.data.as_ref() {
                    atomic_saturating_add(&callback_counter, 1);
                    atomic_saturating_add(
                        &callback_bytes,
                        u64::try_from(buffer.size()).unwrap_or(u64::MAX),
                    );
                }
                gst::PadProbeReturn::Ok
            })
            .ok_or(SupervisorError::ProgressProbeRejected)?;

        let elements = pipeline
            .iterate_elements()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SupervisorError::TopologyChanged)?;
        let mut factories = elements
            .iter()
            .filter_map(|element| element.factory())
            .map(|factory| safe_public_label(factory.name().as_str(), "unknown-factory"))
            .collect::<Vec<_>>();
        factories.sort_unstable();
        factories.dedup();
        let queue_probes = elements
            .into_iter()
            .filter(|element| {
                element
                    .factory()
                    .is_some_and(|factory| factory.name().as_str() == "queue")
            })
            .map(|element| RuntimeQueueProbe::attach(element, &policy))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            pipeline,
            bus,
            lifecycle: PipelineLifecycle::new(),
            policy,
            correlation_id,
            progress_pad,
            progress_buffers,
            progress_bytes,
            factories,
            av_timing: None,
            queue_probes,
            events: None,
            #[cfg(test)]
            before_message_classification: None,
            teardown_attempted: false,
        })
    }

    /// Adds privacy-safe PTS probes for a named audio and video element. Only
    /// numeric timestamps are retained; no buffers or device metadata cross
    /// the supervisor boundary.
    pub fn with_av_timing_probes(
        mut self,
        audio_element_name: &'static str,
        video_element_name: &'static str,
    ) -> Result<Self, SupervisorError> {
        let audio = attach_timestamp_probe(&self.pipeline, audio_element_name)?;
        let video = attach_timestamp_probe(&self.pipeline, video_element_name)?;
        self.av_timing = Some(AvTimingProbes { audio, video });
        Ok(self)
    }

    pub fn run(
        self,
        cancellation: &CancellationToken,
    ) -> Result<PipelineRunReport, SupervisorError> {
        self.run_owned(cancellation, None)
    }

    #[cfg(test)]
    fn with_before_message_classification(
        mut self,
        hook: impl FnMut(gst::MessageType) + Send + 'static,
    ) -> Self {
        self.before_message_classification = Some(Box::new(hook));
        self
    }

    /// Starts a dedicated pipeline owner thread with bounded command and event
    /// channels. Raw frames never cross this boundary.
    pub fn spawn(mut self) -> Result<PipelineTask, SupervisorError> {
        let (command_sender, command_receiver) = sync_channel(PIPELINE_COMMAND_CAPACITY);
        let (event_sender, event_receiver) = sync_channel(PIPELINE_EVENT_CAPACITY);
        let cancellation = CancellationToken::new();
        let owner_cancellation = cancellation.clone();
        self.events = Some(EventSink {
            sender: event_sender,
            dropped: 0,
            nonterminal_sent: 0,
        });
        let join = thread::Builder::new()
            .name("frame-media-pipeline-owner".into())
            .spawn(move || self.run_owned(&owner_cancellation, Some(&command_receiver)))
            .map_err(|_| SupervisorError::OwnerThreadSpawn)?;
        Ok(PipelineTask {
            control: PipelineControl {
                commands: command_sender,
                cancellation,
            },
            events: event_receiver,
            join: Some(join),
        })
    }

    fn run_owned(
        mut self,
        cancellation: &CancellationToken,
        commands: Option<&Receiver<PipelineControlCommand>>,
    ) -> Result<PipelineRunReport, SupervisorError> {
        let run_started = Instant::now();
        let mut timings = StateTimingTracker::new(PipelineState::Idle, run_started);
        let mut counters = BusCounters::default();

        if cancellation.is_cancelled() {
            self.transition(PipelineCommand::Cancel, &mut timings)?;
            return Ok(self.finish_report(
                run_started,
                &mut timings,
                counters,
                PipelineTerminalOutcome::Cancelled,
            ));
        }

        self.transition(PipelineCommand::Prepare, &mut timings)?;
        if !self.set_and_confirm_state(gst::State::Playing, run_started) {
            if cancellation.is_cancelled() {
                self.transition(PipelineCommand::Cancel, &mut timings)?;
                return Ok(self.finish_report(
                    run_started,
                    &mut timings,
                    counters,
                    PipelineTerminalOutcome::Cancelled,
                ));
            }
            let fault = if run_started.elapsed() >= self.policy.deadline {
                PipelineFault::timeout()
            } else {
                PipelineFault::pipeline()
            };
            self.transition(PipelineCommand::Fail(fault), &mut timings)?;
            return Ok(self.finish_report(
                run_started,
                &mut timings,
                counters,
                PipelineTerminalOutcome::Failed(fault),
            ));
        }
        self.transition(PipelineCommand::Start, &mut timings)?;

        let mut previous_progress = self.progress_buffers.load(Ordering::Relaxed);
        let mut last_progress_at = Instant::now();
        let outcome = loop {
            for queue in &self.queue_probes {
                queue.sample();
            }
            if let Some(outcome) = self.preempted(cancellation, run_started, &mut timings)? {
                break outcome;
            }
            if let Some(commands) = commands
                && let Some(outcome) =
                    self.handle_command(commands, cancellation, run_started, &mut timings)?
            {
                break outcome;
            }
            if let Some(outcome) = self.preempted(cancellation, run_started, &mut timings)? {
                break outcome;
            }

            let progress = self.progress_buffers.load(Ordering::Relaxed);
            if self.progress_limit_exceeded() {
                let fault = PipelineFault::resource_limit();
                self.transition(PipelineCommand::Fail(fault), &mut timings)?;
                break PipelineTerminalOutcome::Failed(fault);
            }
            if progress != previous_progress {
                previous_progress = progress;
                last_progress_at = Instant::now();
            } else if self.lifecycle.state() == PipelineState::Paused {
                // Pausing intentionally stops buffer flow. Keep the watchdog
                // armed for resume without misclassifying a user pause as a
                // blocked sink.
                last_progress_at = Instant::now();
            } else if last_progress_at.elapsed() >= self.policy.stall_timeout {
                let fault = PipelineFault::sink_blocked();
                self.transition(PipelineCommand::Fail(fault), &mut timings)?;
                break PipelineTerminalOutcome::Failed(fault);
            }

            let remaining = self.policy.deadline.saturating_sub(run_started.elapsed());
            let poll = clock_time(self.policy.poll_interval.min(remaining));
            let Some(message) = self.bus.timed_pop(poll) else {
                continue;
            };
            #[cfg(test)]
            if let Some(hook) = &mut self.before_message_classification {
                hook(message.type_());
            }
            if let Some(outcome) = self.preempted(cancellation, run_started, &mut timings)? {
                break outcome;
            }
            counters.bus_messages = counters.bus_messages.saturating_add(1);
            if counters.bus_messages > self.policy.max_bus_messages {
                let fault = PipelineFault::resource_limit();
                self.transition(PipelineCommand::Fail(fault), &mut timings)?;
                break PipelineTerminalOutcome::Failed(fault);
            }
            if self.progress_limit_exceeded() {
                let fault = PipelineFault::resource_limit();
                self.transition(PipelineCommand::Fail(fault), &mut timings)?;
                break PipelineTerminalOutcome::Failed(fault);
            }

            match message.view() {
                gst::MessageView::Eos(_) => {
                    if self.lifecycle.state() != PipelineState::Finalizing {
                        self.transition(PipelineCommand::BeginFinalize, &mut timings)?;
                    }
                    self.transition(PipelineCommand::Complete, &mut timings)?;
                    break PipelineTerminalOutcome::Completed;
                }
                gst::MessageView::Error(error) => {
                    let fault = classify_bus_error(&error.error());
                    self.transition(PipelineCommand::Fail(fault), &mut timings)?;
                    break PipelineTerminalOutcome::Failed(fault);
                }
                gst::MessageView::Warning(_) => {
                    counters.warnings = counters.warnings.saturating_add(1);
                    self.emit(PipelineEvent::Warning {
                        count: counters.warnings,
                    });
                    if counters.warnings > self.policy.max_warnings {
                        let fault = PipelineFault::resource_limit();
                        self.transition(PipelineCommand::Fail(fault), &mut timings)?;
                        break PipelineTerminalOutcome::Failed(fault);
                    }
                }
                gst::MessageView::Latency(_) => {
                    counters.latency_messages = counters.latency_messages.saturating_add(1);
                    let _ = self.pipeline.recalculate_latency();
                }
                gst::MessageView::StateChanged(_)
                    if message_from_pipeline(&message, &self.pipeline) =>
                {
                    counters.pipeline_state_changes =
                        counters.pipeline_state_changes.saturating_add(1);
                }
                _ => {}
            }
        };

        Ok(self.finish_report(run_started, &mut timings, counters, outcome))
    }

    fn handle_command(
        &mut self,
        commands: &Receiver<PipelineControlCommand>,
        cancellation: &CancellationToken,
        run_started: Instant,
        timings: &mut StateTimingTracker,
    ) -> Result<Option<PipelineTerminalOutcome>, SupervisorError> {
        let command = match commands.try_recv() {
            Ok(command) => command,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(None),
        };
        match (self.lifecycle.state(), command) {
            (PipelineState::Running, PipelineControlCommand::Pause) => {
                if !self.set_and_confirm_state(gst::State::Paused, run_started) {
                    return self.fail_state_change(cancellation, run_started, timings);
                }
                if let Some(outcome) = self.preempted(cancellation, run_started, timings)? {
                    return Ok(Some(outcome));
                }
                self.transition(PipelineCommand::Pause, timings)?;
            }
            (PipelineState::Paused, PipelineControlCommand::Resume) => {
                if !self.set_and_confirm_state(gst::State::Playing, run_started) {
                    return self.fail_state_change(cancellation, run_started, timings);
                }
                if let Some(outcome) = self.preempted(cancellation, run_started, timings)? {
                    return Ok(Some(outcome));
                }
                self.transition(PipelineCommand::Resume, timings)?;
            }
            (PipelineState::Running | PipelineState::Paused, PipelineControlCommand::Finish) => {
                if self.lifecycle.state() == PipelineState::Paused {
                    if !self.set_and_confirm_state(gst::State::Playing, run_started) {
                        return self.fail_state_change(cancellation, run_started, timings);
                    }
                    if let Some(outcome) = self.preempted(cancellation, run_started, timings)? {
                        return Ok(Some(outcome));
                    }
                    self.transition(PipelineCommand::Resume, timings)?;
                }
                self.transition(PipelineCommand::BeginFinalize, timings)?;
                if !self.pipeline.send_event(gst::event::Eos::new()) {
                    let fault = PipelineFault::pipeline();
                    self.transition(PipelineCommand::Fail(fault), timings)?;
                    return Ok(Some(PipelineTerminalOutcome::Failed(fault)));
                }
            }
            (state, command) => {
                self.emit(PipelineEvent::CommandRejected { command, state });
            }
        }
        Ok(None)
    }

    fn preempted(
        &mut self,
        cancellation: &CancellationToken,
        run_started: Instant,
        timings: &mut StateTimingTracker,
    ) -> Result<Option<PipelineTerminalOutcome>, SupervisorError> {
        if cancellation.is_cancelled() {
            self.transition(PipelineCommand::Cancel, timings)?;
            return Ok(Some(PipelineTerminalOutcome::Cancelled));
        }
        if run_started.elapsed() >= self.policy.deadline {
            let fault = PipelineFault::timeout();
            self.transition(PipelineCommand::Fail(fault), timings)?;
            return Ok(Some(PipelineTerminalOutcome::Failed(fault)));
        }
        Ok(None)
    }

    fn fail_state_change(
        &mut self,
        cancellation: &CancellationToken,
        run_started: Instant,
        timings: &mut StateTimingTracker,
    ) -> Result<Option<PipelineTerminalOutcome>, SupervisorError> {
        if cancellation.is_cancelled() {
            self.transition(PipelineCommand::Cancel, timings)?;
            return Ok(Some(PipelineTerminalOutcome::Cancelled));
        }
        let fault = if run_started.elapsed() >= self.policy.deadline {
            PipelineFault::timeout()
        } else {
            PipelineFault::pipeline()
        };
        self.transition(PipelineCommand::Fail(fault), timings)?;
        Ok(Some(PipelineTerminalOutcome::Failed(fault)))
    }

    fn progress_limit_exceeded(&self) -> bool {
        self.progress_buffers.load(Ordering::Relaxed) > self.policy.max_progress_buffers
            || self.progress_bytes.load(Ordering::Relaxed) > self.policy.max_progress_bytes
    }

    fn set_and_confirm_state(&self, state: gst::State, run_started: Instant) -> bool {
        if self.pipeline.set_state(state).is_err() {
            return false;
        }
        let timeout = self
            .policy
            .deadline
            .saturating_sub(run_started.elapsed())
            .min(self.policy.state_change_confirmation_timeout);
        if timeout.is_zero() {
            return false;
        }
        let (result, current, _) = self.pipeline.state(clock_time(timeout));
        result.is_ok() && current == state
    }

    fn transition(
        &mut self,
        command: PipelineCommand,
        timings: &mut StateTimingTracker,
    ) -> Result<(), SupervisorError> {
        let transition = self.lifecycle.apply(command)?;
        timings.transition(transition.to, Instant::now());
        self.emit(PipelineEvent::Transition(transition));
        if transition.to.is_terminal() {
            let outcome = match transition.command {
                PipelineCommand::Complete => PipelineTerminalOutcome::Completed,
                PipelineCommand::Fail(fault) => PipelineTerminalOutcome::Failed(fault),
                PipelineCommand::Cancel => PipelineTerminalOutcome::Cancelled,
                _ => return Err(SupervisorError::TerminalTransitionMismatch),
            };
            self.emit(PipelineEvent::Terminal(outcome));
        }
        Ok(())
    }

    fn emit(&mut self, event: PipelineEvent) {
        if let Some(events) = &mut self.events {
            events.emit(event);
        }
    }

    fn finish_report(
        mut self,
        run_started: Instant,
        timings: &mut StateTimingTracker,
        counters: BusCounters,
        outcome: PipelineTerminalOutcome,
    ) -> PipelineRunReport {
        let terminal_elapsed = run_started.elapsed();
        timings.finish(Instant::now());
        let negotiated_media_types = negotiated_media_types(&self.progress_pad);
        let teardown_started = Instant::now();
        self.teardown_attempted = true;
        let teardown = if self.pipeline.set_state(gst::State::Null).is_err() {
            PipelineTeardown::NullStateFailed
        } else {
            let (result, state, _) = self
                .pipeline
                .state(clock_time(self.policy.null_state_confirmation_timeout));
            if result.is_ok() && state == gst::State::Null {
                PipelineTeardown::NullReached
            } else {
                PipelineTeardown::NullStateUnconfirmed
            }
        };

        PipelineRunReport {
            outcome,
            elapsed_ms: duration_ms(terminal_elapsed),
            teardown_elapsed_ms: duration_ms(teardown_started.elapsed()),
            teardown,
            diagnostics: PipelineDiagnostics {
                schema_version: PIPELINE_DIAGNOSTIC_SCHEMA_VERSION,
                protocol_version: PIPELINE_PROTOCOL_VERSION,
                manifest_version: RUNTIME_MANIFEST_VERSION,
                correlation_id: self.correlation_id.clone(),
                runtime_version: public_runtime_version(),
                factories: self.factories.clone(),
                negotiated_media_types,
                buffers_observed: self.progress_buffers.load(Ordering::Relaxed),
                bytes_observed: self.progress_bytes.load(Ordering::Relaxed),
                bus_messages: counters.bus_messages,
                warnings: counters.warnings,
                latency_messages: counters.latency_messages,
                pipeline_state_changes: counters.pipeline_state_changes,
                state_durations: timings.durations.clone(),
                av_timing: self
                    .av_timing
                    .as_ref()
                    .and_then(AvTimingProbes::diagnostics),
                queues: self
                    .queue_probes
                    .iter()
                    .enumerate()
                    .map(|(index, queue)| {
                        queue.sample();
                        queue.diagnostics(u32::try_from(index).unwrap_or(u32::MAX))
                    })
                    .collect(),
                events_dropped: self.events.as_ref().map_or(0, |events| events.dropped),
            },
        }
    }
}

impl Drop for PipelineSupervisor {
    fn drop(&mut self) {
        // This is a last-resort safety net for constructor/run errors. Normal
        // terminal paths record the explicit teardown result in the report.
        if !self.teardown_attempted {
            let _ = self.pipeline.set_state(gst::State::Null);
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct BusCounters {
    bus_messages: u64,
    warnings: u64,
    latency_messages: u64,
    pipeline_state_changes: u64,
}

struct EventSink {
    sender: SyncSender<PipelineEvent>,
    dropped: u64,
    nonterminal_sent: usize,
}

impl EventSink {
    fn emit(&mut self, event: PipelineEvent) {
        let terminal = matches!(event, PipelineEvent::Terminal(_));
        if !terminal && self.nonterminal_sent >= PIPELINE_EVENT_CAPACITY.saturating_sub(1) {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        match self.sender.try_send(event) {
            Ok(()) if !terminal => {
                self.nonterminal_sent = self.nonterminal_sent.saturating_add(1);
            }
            Ok(()) => {}
            Err(_) => {
                self.dropped = self.dropped.saturating_add(1);
            }
        }
    }
}

#[derive(Debug)]
struct StateTimingTracker {
    current: PipelineState,
    entered: Instant,
    durations: Vec<PipelineStateDuration>,
}

#[derive(Debug)]
struct TimestampProbe {
    first_pts_ns: AtomicU64,
    last_end_pts_ns: AtomicU64,
}

impl TimestampProbe {
    fn new() -> Self {
        Self {
            first_pts_ns: AtomicU64::new(u64::MAX),
            last_end_pts_ns: AtomicU64::new(0),
        }
    }

    fn observe(&self, pts_ns: u64, duration_ns: u64) {
        let _ = self.first_pts_ns.compare_exchange(
            u64::MAX,
            pts_ns,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        self.last_end_pts_ns
            .fetch_max(pts_ns.saturating_add(duration_ns), Ordering::Relaxed);
    }

    fn values(&self) -> Option<(u64, u64)> {
        let first = self.first_pts_ns.load(Ordering::Relaxed);
        let last = self.last_end_pts_ns.load(Ordering::Relaxed);
        (first != u64::MAX && last >= first).then_some((first, last))
    }
}

#[derive(Debug)]
struct AvTimingProbes {
    audio: Arc<TimestampProbe>,
    video: Arc<TimestampProbe>,
}

#[derive(Debug)]
struct RuntimeQueueProbe {
    element: gst::Element,
    max_buffers: u32,
    max_bytes: u64,
    max_time_ns: u64,
    overflow: RuntimeQueueOverflow,
    peak_buffers: AtomicU64,
    peak_bytes: AtomicU64,
    peak_time_ns: AtomicU64,
    overrun_events: Arc<AtomicU64>,
    underrun_events: Arc<AtomicU64>,
}

impl RuntimeQueueProbe {
    fn attach(element: gst::Element, policy: &SupervisorPolicy) -> Result<Self, SupervisorError> {
        let max_buffers = element.property::<u32>("max-size-buffers");
        let max_bytes = u64::from(element.property::<u32>("max-size-bytes"));
        let max_time_ns = element.property::<u64>("max-size-time");
        if max_buffers == 0 && max_bytes == 0 && max_time_ns == 0 {
            return Err(SupervisorError::UnboundedRuntimeQueue);
        }
        if max_buffers > policy.max_queue_buffers
            || max_bytes > policy.max_queue_bytes
            || max_time_ns > u64::try_from(policy.max_queue_time.as_nanos()).unwrap_or(u64::MAX)
        {
            return Err(SupervisorError::RuntimeQueueLimitExceeded);
        }
        let overflow_value = element.property_value("leaky");
        let (_, overflow_value) = gst::glib::EnumValue::from_value(&overflow_value)
            .ok_or(SupervisorError::InvalidQueueOverflowPolicy)?;
        let overflow = match overflow_value.nick() {
            "no" => RuntimeQueueOverflow::Block,
            "upstream" => RuntimeQueueOverflow::DropNewest,
            "downstream" => RuntimeQueueOverflow::DropOldest,
            _ => return Err(SupervisorError::InvalidQueueOverflowPolicy),
        };
        let overrun_events = Arc::new(AtomicU64::new(0));
        let overrun_callback = Arc::clone(&overrun_events);
        let _overrun_handler = element.connect("overrun", false, move |_| {
            overrun_callback.fetch_add(1, Ordering::Relaxed);
            None
        });
        let underrun_events = Arc::new(AtomicU64::new(0));
        let underrun_callback = Arc::clone(&underrun_events);
        let _underrun_handler = element.connect("underrun", false, move |_| {
            underrun_callback.fetch_add(1, Ordering::Relaxed);
            None
        });
        Ok(Self {
            element,
            max_buffers,
            max_bytes,
            max_time_ns,
            overflow,
            peak_buffers: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            peak_time_ns: AtomicU64::new(0),
            overrun_events,
            underrun_events,
        })
    }

    fn sample(&self) {
        self.peak_buffers.fetch_max(
            u64::from(self.element.property::<u32>("current-level-buffers")),
            Ordering::Relaxed,
        );
        self.peak_bytes.fetch_max(
            u64::from(self.element.property::<u32>("current-level-bytes")),
            Ordering::Relaxed,
        );
        self.peak_time_ns.fetch_max(
            self.element.property::<u64>("current-level-time"),
            Ordering::Relaxed,
        );
    }

    fn diagnostics(&self, index: u32) -> RuntimeQueueDiagnostics {
        RuntimeQueueDiagnostics {
            index,
            max_buffers: self.max_buffers,
            max_bytes: self.max_bytes,
            max_time_ns: self.max_time_ns,
            overflow: self.overflow,
            peak_buffers: u32::try_from(self.peak_buffers.load(Ordering::Relaxed))
                .unwrap_or(u32::MAX),
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            peak_time_ns: self.peak_time_ns.load(Ordering::Relaxed),
            overrun_events: self.overrun_events.load(Ordering::Relaxed),
            underrun_events: self.underrun_events.load(Ordering::Relaxed),
        }
    }
}

impl AvTimingProbes {
    fn diagnostics(&self) -> Option<AvTimingDiagnostics> {
        let (audio_start_pts_ns, audio_end_pts_ns) = self.audio.values()?;
        let (video_start_pts_ns, video_end_pts_ns) = self.video.values()?;
        let start_offset_ns = signed_difference(video_start_pts_ns, audio_start_pts_ns);
        let end_offset_ns = signed_difference(video_end_pts_ns, audio_end_pts_ns);
        Some(AvTimingDiagnostics {
            audio_start_pts_ns,
            video_start_pts_ns,
            audio_end_pts_ns,
            video_end_pts_ns,
            start_offset_ns,
            end_offset_ns,
            drift_ns: end_offset_ns.saturating_sub(start_offset_ns),
        })
    }
}

impl StateTimingTracker {
    fn new(current: PipelineState, entered: Instant) -> Self {
        Self {
            current,
            entered,
            durations: Vec::new(),
        }
    }

    fn transition(&mut self, next: PipelineState, now: Instant) {
        self.push_elapsed(now);
        self.current = next;
        self.entered = now;
    }

    fn finish(&mut self, now: Instant) {
        self.push_elapsed(now);
    }

    fn push_elapsed(&mut self, now: Instant) {
        self.durations.push(PipelineStateDuration {
            state: self.current,
            elapsed_ms: duration_ms(now.saturating_duration_since(self.entered)),
        });
    }
}

fn negotiated_media_types(pad: &gst::Pad) -> Vec<String> {
    let mut values = pad
        .current_caps()
        .map(|caps| {
            caps.iter()
                .map(|structure| safe_public_label(structure.name().as_str(), "unknown-media-type"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    values.sort_unstable();
    values.dedup();
    values
}

fn attach_timestamp_probe(
    pipeline: &gst::Pipeline,
    element_name: &'static str,
) -> Result<Arc<TimestampProbe>, SupervisorError> {
    let element = pipeline
        .by_name(element_name)
        .ok_or(SupervisorError::MissingTimingElement(element_name))?;
    let pad = element
        .static_pad("src")
        .ok_or(SupervisorError::MissingTimingPad(element_name))?;
    let probe = Arc::new(TimestampProbe::new());
    let callback_probe = Arc::clone(&probe);
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
        if let Some(gst::PadProbeData::Buffer(buffer)) = info.data.as_ref()
            && let Some(pts) = buffer.pts()
        {
            let duration = buffer.duration().map_or(0, |value| value.nseconds());
            callback_probe.observe(pts.nseconds(), duration);
        }
        gst::PadProbeReturn::Ok
    })
    .ok_or(SupervisorError::TimingProbeRejected(element_name))?;
    Ok(probe)
}

fn signed_difference(left: u64, right: u64) -> i64 {
    let difference = i128::from(left) - i128::from(right);
    i64::try_from(difference).unwrap_or(if difference.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

fn atomic_saturating_add(value: &AtomicU64, increment: u64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(increment))
    });
}

fn message_from_pipeline(message: &gst::MessageRef, pipeline: &gst::Pipeline) -> bool {
    message
        .src()
        .is_some_and(|source| source == pipeline.upcast_ref::<gst::Object>())
}

fn classify_bus_error(error: &gst::glib::Error) -> PipelineFault {
    if error.matches(gst::CoreError::Negotiation)
        || error.matches(gst::CoreError::Caps)
        || error.matches(gst::StreamError::WrongType)
        || error.matches(gst::StreamError::Format)
    {
        PipelineFault::negotiation()
    } else if error.matches(gst::CoreError::MissingPlugin)
        || error.matches(gst::StreamError::CodecNotFound)
    {
        PipelineFault::missing_factory()
    } else if error.matches(gst::ResourceError::NoSpaceLeft) {
        PipelineFault::resource_limit()
    } else if error.matches(gst::ResourceError::Busy) {
        PipelineFault::sink_blocked()
    } else if error.matches(gst::ResourceError::NotFound)
        || error.matches(gst::ResourceError::OpenRead)
        || error.matches(gst::ResourceError::Read)
    {
        PipelineFault::source_lost()
    } else {
        PipelineFault::pipeline()
    }
}

fn public_runtime_version() -> String {
    let version = gst::version();
    format!("{}.{}.{}", version.0, version.1, version.2)
}

fn safe_public_label(value: &str, fallback: &str) -> String {
    if !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+' | b'/')
        })
    {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn clock_time(duration: Duration) -> gst::ClockTime {
    let nanoseconds = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX - 1);
    gst::ClockTime::from_nseconds(nanoseconds)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PipelineControlError {
    #[error("pipeline command queue is full")]
    QueueFull,
    #[error("pipeline owner has stopped")]
    OwnerStopped,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorError {
    #[error("pipeline correlation ID is invalid")]
    InvalidCorrelationId,
    #[error("pipeline supervisor policy is invalid or unbounded")]
    InvalidPolicy,
    #[error("pipeline has no bus")]
    MissingBus,
    #[error("pipeline is missing the declared progress element {0}")]
    MissingProgressElement(&'static str),
    #[error("pipeline progress element has no static source pad")]
    MissingProgressPad,
    #[error("pipeline progress probe was rejected")]
    ProgressProbeRejected,
    #[error("pipeline topology changed while it was being audited")]
    TopologyChanged,
    #[error("pipeline contains a factory outside the build-time plugin root")]
    UntrustedFactoryProvenance,
    #[error("pipeline contains a Frame-authored factory absent from the runtime manifest")]
    UndeclaredAuthoredFactory,
    #[error("pipeline is missing the declared timing element {0}")]
    MissingTimingElement(&'static str),
    #[error("pipeline timing element {0} has no static source pad")]
    MissingTimingPad(&'static str),
    #[error("pipeline timing probe for {0} was rejected")]
    TimingProbeRejected(&'static str),
    #[error("pipeline contains an unbounded runtime queue")]
    UnboundedRuntimeQueue,
    #[error("pipeline queue has an unknown overflow policy")]
    InvalidQueueOverflowPolicy,
    #[error("pipeline queue bound exceeds the supervisor policy")]
    RuntimeQueueLimitExceeded,
    #[error("pipeline owner thread could not be started")]
    OwnerThreadSpawn,
    #[error("pipeline owner thread stopped unexpectedly")]
    OwnerThreadPanicked,
    #[error("pipeline emitted a terminal state from a non-terminal command")]
    TerminalTransitionMismatch,
    #[error("pipeline lifecycle failed: {0}")]
    Lifecycle(#[from] LifecycleError),
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::{PipelineFaultCode, prepare_runtime};

    fn pipeline(description: &str) -> gst::Pipeline {
        gst::parse::launch(description)
            .expect("parse pipeline")
            .downcast::<gst::Pipeline>()
            .expect("pipeline")
    }

    fn supervisor(description: &str, policy: SupervisorPolicy) -> PipelineSupervisor {
        let runtime = prepare_runtime().expect("trusted GStreamer runtime");
        PipelineSupervisor::new(
            &runtime,
            pipeline(description),
            "progress",
            PipelineCorrelationId::new("supervisor-test").expect("correlation"),
            policy,
        )
        .expect("supervisor")
    }

    #[test]
    fn state_confirmation_timeouts_cannot_exceed_the_global_deadline() {
        let deadline = Duration::from_millis(10);
        assert!(matches!(
            SupervisorPolicy {
                deadline,
                state_change_confirmation_timeout: Duration::from_millis(11),
                poll_interval: Duration::from_millis(1),
                stall_timeout: deadline,
                null_state_confirmation_timeout: deadline,
                ..SupervisorPolicy::default()
            }
            .validate(),
            Err(SupervisorError::InvalidPolicy)
        ));
        assert!(matches!(
            SupervisorPolicy {
                deadline,
                null_state_confirmation_timeout: Duration::from_millis(11),
                poll_interval: Duration::from_millis(1),
                stall_timeout: deadline,
                state_change_confirmation_timeout: deadline,
                ..SupervisorPolicy::default()
            }
            .validate(),
            Err(SupervisorError::InvalidPolicy)
        ));
    }

    #[test]
    fn terminal_event_has_a_reserved_slot_under_notification_pressure() {
        let (sender, receiver) = sync_channel(PIPELINE_EVENT_CAPACITY);
        let mut sink = EventSink {
            sender,
            dropped: 0,
            nonterminal_sent: 0,
        };
        for count in 0..(PIPELINE_EVENT_CAPACITY * 2) {
            sink.emit(PipelineEvent::Warning {
                count: u64::try_from(count).unwrap_or(u64::MAX),
            });
        }
        sink.emit(PipelineEvent::Terminal(PipelineTerminalOutcome::Cancelled));
        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), PIPELINE_EVENT_CAPACITY);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, PipelineEvent::Terminal(_)))
                .count(),
            1
        );
        assert!(sink.dropped > 0);
    }

    #[test]
    fn completed_run_reports_one_terminal_and_null_teardown() {
        let report = supervisor(
            "videotestsrc num-buffers=5 ! identity name=progress ! fakesink",
            SupervisorPolicy::default(),
        )
        .run(&CancellationToken::new())
        .expect("run");
        assert!(report.completed());
        assert_eq!(report.outcome.state(), PipelineState::Completed);
        assert!(report.diagnostics.buffers_observed >= 5);
        assert!(report.diagnostics.factories.contains(&"fakesink".into()));
        assert_eq!(
            report
                .diagnostics
                .state_durations
                .last()
                .map(|item| item.state),
            Some(PipelineState::Completed)
        );
    }

    #[test]
    fn runtime_rejects_an_actually_unbounded_queue() {
        let runtime = prepare_runtime().expect("trusted GStreamer runtime");
        let result = PipelineSupervisor::new(
            &runtime,
            pipeline(
                "videotestsrc num-buffers=1 ! queue max-size-buffers=0 max-size-bytes=0 max-size-time=0 ! identity name=progress ! fakesink",
            ),
            "progress",
            PipelineCorrelationId::new("unbounded-queue-test").expect("correlation"),
            SupervisorPolicy::default(),
        );
        assert!(matches!(
            result,
            Err(SupervisorError::UnboundedRuntimeQueue)
        ));
    }

    #[test]
    fn runtime_rejects_a_trusted_but_undeclared_authored_factory() {
        let runtime = prepare_runtime().expect("trusted GStreamer runtime");
        let tee = gst::ElementFactory::find("tee").expect("trusted core tee factory");
        assert!(tee.plugin().and_then(|plugin| plugin.filename()).is_some());
        let graph =
            pipeline("videotestsrc num-buffers=1 ! tee ! identity name=progress ! fakesink");
        assert!(pipeline_has_trusted_factory_provenance(&graph));
        let result = PipelineSupervisor::new(
            &runtime,
            graph,
            "progress",
            PipelineCorrelationId::new("undeclared-factory-test").expect("correlation"),
            SupervisorPolicy::default(),
        );
        assert!(matches!(
            result,
            Err(SupervisorError::UndeclaredAuthoredFactory)
        ));
    }

    #[test]
    fn runtime_rejects_a_queue_bound_above_its_supervisor_budget() {
        let runtime = prepare_runtime().expect("trusted GStreamer runtime");
        let result = PipelineSupervisor::new(
            &runtime,
            pipeline(
                "videotestsrc num-buffers=1 ! queue max-size-buffers=20 max-size-bytes=0 max-size-time=0 ! identity name=progress ! fakesink",
            ),
            "progress",
            PipelineCorrelationId::new("oversized-queue-test").expect("correlation"),
            SupervisorPolicy {
                max_queue_buffers: 10,
                ..SupervisorPolicy::default()
            },
        );
        assert!(matches!(
            result,
            Err(SupervisorError::RuntimeQueueLimitExceeded)
        ));
    }

    #[test]
    fn cancellation_is_terminal_and_still_reaches_null() {
        let cancellation = CancellationToken::new();
        let trigger = cancellation.clone();
        let thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(75));
            trigger.cancel();
        });
        let report = supervisor(
            "videotestsrc is-live=true ! identity name=progress ! fakesink sync=false",
            SupervisorPolicy {
                deadline: Duration::from_secs(2),
                stall_timeout: Duration::from_secs(1),
                ..SupervisorPolicy::default()
            },
        )
        .run(&cancellation)
        .expect("run");
        thread.join().expect("cancellation thread");
        assert_eq!(report.outcome, PipelineTerminalOutcome::Cancelled);
        assert_eq!(report.teardown, PipelineTeardown::NullReached);
    }

    #[test]
    fn cancellation_after_eos_is_popped_wins_before_message_classification() {
        let cancellation = CancellationToken::new();
        let trigger = cancellation.clone();
        let report = supervisor(
            "videotestsrc num-buffers=1 ! identity name=progress ! fakesink",
            SupervisorPolicy::default(),
        )
        .with_before_message_classification(move |message_type| {
            if message_type == gst::MessageType::Eos {
                trigger.cancel();
            }
        })
        .run(&cancellation)
        .expect("run");
        assert_eq!(report.outcome, PipelineTerminalOutcome::Cancelled);
        assert_eq!(report.teardown, PipelineTeardown::NullReached);
    }

    #[test]
    fn dedicated_owner_accepts_pause_resume_and_bounded_finish_commands() {
        let task = supervisor(
            "videotestsrc is-live=true ! identity name=progress ! fakesink sync=false",
            SupervisorPolicy {
                deadline: Duration::from_secs(3),
                poll_interval: Duration::from_millis(10),
                stall_timeout: Duration::from_millis(75),
                ..SupervisorPolicy::default()
            },
        )
        .spawn()
        .expect("spawn owner");
        let control = task.control();
        control.try_pause().expect("pause command");
        let mut transitions = Vec::new();
        loop {
            match task
                .event_timeout(Duration::from_millis(500))
                .expect("pause event boundary")
            {
                Some(PipelineEvent::Transition(transition)) => {
                    transitions.push(transition);
                    if transition.to == PipelineState::Paused {
                        break;
                    }
                }
                Some(PipelineEvent::Terminal(outcome)) => {
                    panic!("owner terminated while pausing: {outcome:?}")
                }
                Some(_) => {}
                None => panic!("pause transition timed out"),
            }
        }
        thread::sleep(Duration::from_millis(150));
        control.try_resume().expect("resume command");
        loop {
            match task
                .event_timeout(Duration::from_millis(500))
                .expect("resume event boundary")
            {
                Some(PipelineEvent::Transition(transition)) => {
                    transitions.push(transition);
                    if transition.from == PipelineState::Paused
                        && transition.to == PipelineState::Running
                    {
                        break;
                    }
                }
                Some(PipelineEvent::Terminal(outcome)) => {
                    panic!("owner terminated while paused: {outcome:?}")
                }
                Some(_) => {}
                None => panic!("resume transition timed out"),
            }
        }
        control.try_finish().expect("finish command");

        let mut terminal_events = 0;
        loop {
            match task
                .event_timeout(Duration::from_millis(250))
                .expect("event boundary")
            {
                Some(PipelineEvent::Transition(transition)) => transitions.push(transition),
                Some(PipelineEvent::Terminal(PipelineTerminalOutcome::Completed)) => {
                    terminal_events += 1;
                    break;
                }
                Some(_) => {}
                None => continue,
            }
        }
        let report = task.wait().expect("owner result");
        assert!(report.completed());
        assert_eq!(terminal_events, 1);
        assert!(
            transitions
                .iter()
                .any(|item| item.to == PipelineState::Paused)
        );
        assert!(transitions.iter().any(|item| {
            item.from == PipelineState::Paused && item.to == PipelineState::Running
        }));
        assert_eq!(report.diagnostics.events_dropped, 0);
    }

    #[test]
    fn dropping_an_async_owner_requests_cancellation() {
        let task = supervisor(
            "videotestsrc is-live=true ! identity name=progress ! fakesink sync=false",
            SupervisorPolicy {
                deadline: Duration::from_secs(2),
                stall_timeout: Duration::from_secs(1),
                ..SupervisorPolicy::default()
            },
        )
        .spawn()
        .expect("spawn owner");
        let control = task.control();
        drop(task);
        assert!(control.cancellation.is_cancelled());
    }

    #[test]
    fn live_graph_with_progress_hits_deadline_instead_of_stall_gate() {
        let report = supervisor(
            "videotestsrc is-live=true ! identity name=progress ! fakesink sync=false",
            SupervisorPolicy {
                deadline: Duration::from_millis(175),
                poll_interval: Duration::from_millis(10),
                stall_timeout: Duration::from_millis(150),
                null_state_confirmation_timeout: Duration::from_millis(100),
                state_change_confirmation_timeout: Duration::from_millis(100),
                ..SupervisorPolicy::default()
            },
        )
        .run(&CancellationToken::new())
        .expect("run");
        assert!(matches!(
            report.outcome,
            PipelineTerminalOutcome::Failed(PipelineFault {
                code: PipelineFaultCode::Timeout,
                ..
            })
        ));
        assert!(report.elapsed_ms <= 250);
        assert!(report.teardown_elapsed_ms <= 150);
        assert_eq!(report.teardown, PipelineTeardown::NullReached);
    }

    #[test]
    fn progress_buffer_budget_stops_an_unbounded_producer() {
        let report = supervisor(
            "videotestsrc num-buffers=100 ! identity name=progress ! fakesink sync=false",
            SupervisorPolicy {
                max_progress_buffers: 2,
                deadline: Duration::from_secs(2),
                stall_timeout: Duration::from_secs(1),
                ..SupervisorPolicy::default()
            },
        )
        .run(&CancellationToken::new())
        .expect("run");
        assert!(matches!(
            report.outcome,
            PipelineTerminalOutcome::Failed(PipelineFault {
                code: PipelineFaultCode::ResourceLimit,
                ..
            })
        ));
        assert_eq!(report.teardown, PipelineTeardown::NullReached);
    }

    #[test]
    fn blocked_sink_is_detected_before_the_global_deadline() {
        let report = supervisor(
            "videotestsrc is-live=true ! queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 ! identity sleep-time=200000 ! identity name=progress ! fakesink sync=false",
            SupervisorPolicy {
                deadline: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
                stall_timeout: Duration::from_millis(75),
                ..SupervisorPolicy::default()
            },
        )
        .run(&CancellationToken::new())
        .expect("run");
        assert!(matches!(
            report.outcome,
            PipelineTerminalOutcome::Failed(PipelineFault {
                code: PipelineFaultCode::SinkBlocked,
                ..
            })
        ));
        assert_eq!(report.teardown, PipelineTeardown::NullReached);
    }

    #[test]
    fn startup_error_is_reported_without_copying_private_paths() {
        let runtime = prepare_runtime().expect("trusted GStreamer runtime");
        let pipeline = pipeline("filesrc name=input ! identity name=progress ! fakesink");
        let private_path = "/Users/person/Secret/customer-recording.mov";
        pipeline
            .by_name("input")
            .expect("input")
            .set_property("location", private_path);
        let report = PipelineSupervisor::new(
            &runtime,
            pipeline,
            "progress",
            PipelineCorrelationId::new("safe-error-test").expect("correlation"),
            SupervisorPolicy::default(),
        )
        .expect("supervisor")
        .run(&CancellationToken::new())
        .expect("run");
        assert!(matches!(report.outcome, PipelineTerminalOutcome::Failed(_)));
        let rendered = format!("{report:?}");
        assert!(!rendered.contains("customer-recording"));
        assert!(!rendered.contains("/Users/"));
    }

    #[test]
    fn streaming_fault_reaches_the_bus_and_is_terminal() {
        let report = supervisor(
            "videotestsrc num-buffers=10 ! identity error-after=1 ! identity name=progress ! fakesink",
            SupervisorPolicy::default(),
        )
        .run(&CancellationToken::new())
        .expect("run");
        assert!(matches!(
            report.outcome,
            PipelineTerminalOutcome::Failed(PipelineFault {
                code: PipelineFaultCode::Internal,
                ..
            })
        ));
        assert_eq!(report.teardown, PipelineTeardown::NullReached);
    }

    #[test]
    fn negotiation_and_missing_plugin_domains_map_to_actionable_faults() {
        let negotiation = gst::glib::Error::new(
            gst::CoreError::Negotiation,
            "private caps diagnostic must not be retained",
        );
        assert_eq!(
            classify_bus_error(&negotiation),
            PipelineFault::negotiation()
        );

        let missing = gst::glib::Error::new(
            gst::CoreError::MissingPlugin,
            "private registry diagnostic must not be retained",
        );
        assert_eq!(
            classify_bus_error(&missing),
            PipelineFault::missing_factory()
        );
    }

    #[test]
    fn policy_and_correlation_labels_must_be_bounded() {
        assert_eq!(
            PipelineCorrelationId::new("../../private"),
            Err(SupervisorError::InvalidCorrelationId)
        );
        assert_eq!(
            SupervisorPolicy {
                deadline: Duration::ZERO,
                ..SupervisorPolicy::default()
            }
            .validate(),
            Err(SupervisorError::InvalidPolicy)
        );
    }
}
