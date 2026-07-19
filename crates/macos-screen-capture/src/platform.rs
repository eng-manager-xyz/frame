use std::{
    collections::BTreeMap,
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    time::Duration,
};

use apple_cf::dispatch_queue::{DispatchQoS, DispatchQueue, dispatch_async_and_wait};
use core_graphics::{access::ScreenCaptureAccess, display::CGDisplay};
use frame_media::{
    DisplayGeometryTransform, DpiScale, LogicalRect, MAX_SCREEN_TARGETS, PermissionPreflight,
    PhysicalRect, Rotation, ScreenSourceInstanceId, ScreenTargetBinding, ScreenTargetSnapshot,
    SettingsGuidance,
};
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferExt, CMSampleBufferSCExt, CMTime, SCFrameStatus},
    cv::CVPixelBufferLockFlags,
    prelude::{
        CGRect, PixelFormat as NativePixelFormat, SCContentFilter, SCDisplay, SCError,
        SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegateTrait,
        SCStreamOutputType, SCWindow,
    },
};
use zeroize::Zeroizing;

use crate::{
    CALLBACK_QUEUE_CAPACITY, MacOsCaptureConfig, MacOsCaptureDiagnostics, MacOsCaptureError,
    MacOsCaptureFrame, MacOsCaptureStopError, MacOsRegionSelection, RawMediaTime, copy_bgra_rows,
    target_catalog::{
        NativeDisplayRecord, NativeTargetRecord, NativeWindowRecord, assemble_records,
        build_catalog, exclude_current_process_windows, resolve_region_selections,
    },
};

mod frame_assembly;
mod native_call;

use frame_assembly::FrameAssembler;
use native_call::{
    BoundedNativeCall, NativeCallLaunchError, PendingNativeCall, run_bounded_native_call,
};

const CALLBACK_QUEUE_LABEL: &str = "xyz.eng-manager.frame.screen-capture";
const DELEGATE_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(1);
const NATIVE_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const GEOMETRY_INTEGER_TOLERANCE: f64 = 0.000_001;
const SRGB_COLOR_SPACE_NAME: &str = "kCGColorSpaceSRGB";

#[derive(Default)]
struct DiagnosticCounters {
    dropped_callback_frames: AtomicU64,
    callback_frames_after_stop: AtomicU64,
    ignored_non_content_samples: AtomicU64,
    invalid_samples: AtomicU64,
    duration_fallbacks: AtomicU64,
    timestamp_discontinuities: AtomicU64,
    unexpected_native_stops: AtomicU64,
}

impl DiagnosticCounters {
    fn snapshot(&self) -> MacOsCaptureDiagnostics {
        MacOsCaptureDiagnostics {
            dropped_callback_frames: self.dropped_callback_frames.load(Ordering::Relaxed),
            callback_frames_after_stop: self.callback_frames_after_stop.load(Ordering::Relaxed),
            ignored_non_content_samples: self.ignored_non_content_samples.load(Ordering::Relaxed),
            invalid_samples: self.invalid_samples.load(Ordering::Relaxed),
            duration_fallbacks: self.duration_fallbacks.load(Ordering::Relaxed),
            timestamp_discontinuities: self.timestamp_discontinuities.load(Ordering::Relaxed),
            unexpected_native_stops: self.unexpected_native_stops.load(Ordering::Relaxed),
        }
    }
}

fn increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

fn deliver_callback_sample<T>(sender: &SyncSender<T>, sample: T, diagnostics: &DiagnosticCounters) {
    match sender.try_send(sample) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => increment(&diagnostics.dropped_callback_frames),
        Err(TrySendError::Disconnected(_)) => {
            increment(&diagnostics.callback_frames_after_stop);
        }
    }
}

struct ActiveCapture {
    stream: Option<SCStream>,
    _pending_native_call: Option<PendingNativeCall>,
    callback_queue: DispatchQueue,
    output_handler_id: Option<usize>,
    receiver: Receiver<CMSampleBuffer>,
    delegate_dropped: Receiver<()>,
    unexpected_stop: Arc<AtomicBool>,
    frames: FrameAssembler,
}

struct DetachedCaptureTail {
    samples: Vec<CMSampleBuffer>,
    output_handler_registered: bool,
}

struct CaptureDelegate {
    unexpected_stop: Arc<AtomicBool>,
    dropped: SyncSender<()>,
}

impl SCStreamDelegateTrait for CaptureDelegate {
    fn did_stop_with_error(&self, _error: SCError) {
        // The callback owns no session diagnostics. The serial worker records
        // the event only after observing it, so a failed start cannot mutate a
        // later session's diagnostic baseline.
        self.unexpected_stop.store(true, Ordering::Release);
    }
}

impl Drop for CaptureDelegate {
    fn drop(&mut self) {
        // Capacity one is sufficient because this delegate is dropped once.
        // A full channel already contains the required proof signal.
        let _ = self.dropped.try_send(());
    }
}

enum NativeCaptureLifecycle<Active> {
    Ready,
    Running(Active),
    StopUnconfirmed {
        active: Active,
        error: MacOsCaptureStopError,
    },
    NativeOperationUnconfirmed {
        _pending: PendingNativeCall,
        error: MacOsCaptureStopError,
    },
}

impl<Active> NativeCaptureLifecycle<Active> {
    fn retain_unconfirmed(self, error: MacOsCaptureStopError) -> Self {
        match self {
            Self::Running(active) => Self::StopUnconfirmed { active, error },
            other => other,
        }
    }

    fn take_for_stop(self) -> Result<Option<Active>, (Self, MacOsCaptureStopError)> {
        match self {
            Self::Ready => Ok(None),
            Self::Running(active) => Ok(Some(active)),
            Self::StopUnconfirmed { active, error } => {
                let retained = Self::StopUnconfirmed { active, error };
                Err((retained, error))
            }
            Self::NativeOperationUnconfirmed {
                _pending: pending,
                error,
            } => {
                let retained = Self::NativeOperationUnconfirmed {
                    _pending: pending,
                    error,
                };
                Err((retained, error))
            }
        }
    }
}

enum StreamNativeCall<R> {
    Completed(R),
    NotStarted(NativeCallLaunchError),
    Unconfirmed,
}

const fn map_native_call_launch_error(error: NativeCallLaunchError) -> MacOsCaptureError {
    match error {
        NativeCallLaunchError::CapacityUnavailable => {
            MacOsCaptureError::NativeOperationCapacityUnavailable
        }
        NativeCallLaunchError::WorkerUnavailable => {
            MacOsCaptureError::NativeOperationWorkerUnavailable
        }
    }
}

fn run_stream_native_call<R, F>(active: &mut ActiveCapture, operation: F) -> StreamNativeCall<R>
where
    R: Send + 'static,
    F: FnOnce(&mut SCStream) -> R + Send + 'static,
{
    let Some(stream) = active.stream.take() else {
        return StreamNativeCall::Unconfirmed;
    };
    match run_bounded_native_call(stream, NATIVE_CALL_TIMEOUT, move |mut stream| {
        let result = operation(&mut stream);
        (stream, result)
    }) {
        BoundedNativeCall::Completed {
            owner: stream,
            result,
        } => {
            active.stream = Some(stream);
            StreamNativeCall::Completed(result)
        }
        BoundedNativeCall::NotStarted {
            owner: stream,
            error,
        } => {
            active.stream = Some(stream);
            StreamNativeCall::NotStarted(error)
        }
        BoundedNativeCall::Unconfirmed(pending) => {
            active._pending_native_call = Some(pending);
            StreamNativeCall::Unconfirmed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameStatusDisposition {
    Content,
    RepeatLastContent,
    IgnoreWithDiscontinuity,
    Terminal,
}

const fn frame_status_disposition(status: SCFrameStatus) -> FrameStatusDisposition {
    match status {
        SCFrameStatus::Complete => FrameStatusDisposition::Content,
        SCFrameStatus::Idle => FrameStatusDisposition::RepeatLastContent,
        SCFrameStatus::Blank | SCFrameStatus::Suspended | SCFrameStatus::Started => {
            FrameStatusDisposition::IgnoreWithDiscontinuity
        }
        SCFrameStatus::Stopped => FrameStatusDisposition::Terminal,
    }
}

/// Safe display, window, and single-display-region ScreenCaptureKit source.
///
/// Construct one instance per recording session. `session_secret` must be 32
/// fresh CSPRNG bytes; it binds opaque display tokens to this session without
/// exposing raw display/window identifiers outside this module.
pub struct MacOsScreenCaptureSource {
    source_instance: ScreenSourceInstanceId,
    session_secret: Zeroizing<[u8; 32]>,
    topology_generation: u64,
    catalog_records: Option<Vec<NativeTargetRecord>>,
    target_map: BTreeMap<ScreenTargetBinding, NativeTargetRecord>,
    permission_requested: bool,
    permission_was_granted: bool,
    diagnostics: Arc<DiagnosticCounters>,
    capture: NativeCaptureLifecycle<ActiveCapture>,
}

impl MacOsScreenCaptureSource {
    pub fn new(
        source_instance: ScreenSourceInstanceId,
        session_secret: [u8; 32],
    ) -> Result<Self, MacOsCaptureError> {
        if session_secret.iter().all(|byte| *byte == 0) {
            return Err(MacOsCaptureError::InvalidSessionSecret);
        }
        Ok(Self {
            source_instance,
            session_secret: Zeroizing::new(session_secret),
            topology_generation: 0,
            catalog_records: None,
            target_map: BTreeMap::new(),
            permission_requested: false,
            permission_was_granted: false,
            diagnostics: Arc::new(DiagnosticCounters::default()),
            capture: NativeCaptureLifecycle::Ready,
        })
    }

    #[must_use]
    pub const fn source_instance(&self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    /// Read permission without triggering the system prompt.
    pub fn preflight_permission(&mut self) -> PermissionPreflight {
        if ScreenCaptureAccess.preflight() {
            self.permission_was_granted = true;
            PermissionPreflight::Granted
        } else if self.permission_was_granted {
            PermissionPreflight::Revoked(SettingsGuidance::OpenSystemSettings)
        } else if self.permission_requested {
            PermissionPreflight::Denied(SettingsGuidance::OpenSystemSettings)
        } else {
            PermissionPreflight::PromptRequired
        }
    }

    /// Trigger the macOS screen-recording permission request.
    pub fn request_permission(&mut self) -> PermissionPreflight {
        self.permission_requested = true;
        if ScreenCaptureAccess.request() {
            self.permission_was_granted = true;
            PermissionPreflight::Granted
        } else {
            PermissionPreflight::Denied(SettingsGuidance::OpenSystemSettings)
        }
    }

    /// Enumerate active displays into a label-free, session-bound catalog.
    ///
    /// Core Graphics enumeration is intentionally used here so callers can
    /// present a display catalog before ScreenCaptureKit permission is granted.
    pub fn enumerate_displays(&mut self) -> Result<ScreenTargetSnapshot, MacOsCaptureError> {
        self.ensure_catalog_mutable()?;
        let records = assemble_records(active_display_records()?, Vec::new(), &[])?;
        self.install_catalog(records)
    }

    /// Enumerate active displays and non-Frame, on-screen application windows.
    /// Optional regions must reference an opaque, topology-bound display from
    /// a prior catalog and be wholly contained by that unchanged display.
    ///
    /// Window enumeration requires an already granted Screen Recording TCC
    /// permission. Titles, application names, native handles, and process IDs
    /// remain inside the adapter and are never copied into the returned catalog.
    pub fn enumerate_targets(
        &mut self,
        regions: &[MacOsRegionSelection],
    ) -> Result<ScreenTargetSnapshot, MacOsCaptureError> {
        self.ensure_catalog_mutable()?;
        let regions = resolve_region_selections(&self.target_map, regions)?;
        if !ScreenCaptureAccess.preflight() {
            return Err(MacOsCaptureError::PermissionDenied);
        }
        self.permission_was_granted = true;
        let content = self.shareable_content()?;
        let current_pid = current_process_id()?;
        // A display or region filter promises to exclude the complete current
        // application, including Frame windows created after enumeration.
        select_current_application(content.applications(), current_pid, |application| {
            application.process_id()
        })?;
        let windows = shareable_window_records(&content, current_pid);
        let records = assemble_records(active_display_records()?, windows, &regions)?;
        self.install_catalog(records)
    }

    fn ensure_catalog_mutable(&self) -> Result<(), MacOsCaptureError> {
        match &self.capture {
            NativeCaptureLifecycle::Ready => Ok(()),
            NativeCaptureLifecycle::Running(_) => Err(MacOsCaptureError::AlreadyRunning),
            NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                Err(MacOsCaptureError::CaptureTeardownUnconfirmed)
            }
        }
    }

    fn install_catalog(
        &mut self,
        records: Vec<NativeTargetRecord>,
    ) -> Result<ScreenTargetSnapshot, MacOsCaptureError> {
        let generation = if self.catalog_records.as_ref() == Some(&records) {
            self.topology_generation
        } else {
            self.topology_generation
                .checked_add(1)
                .ok_or(MacOsCaptureError::TopologyGenerationExhausted)?
        };
        let catalog = build_catalog(
            &self.session_secret,
            self.source_instance,
            generation,
            &records,
        )?;
        let (snapshot, target_map) = catalog.into_parts();
        self.topology_generation = generation;
        self.catalog_records = Some(records);
        self.target_map = target_map;
        Ok(snapshot)
    }

    fn shareable_content(&mut self) -> Result<SCShareableContent, MacOsCaptureError> {
        match run_bounded_native_call((), NATIVE_CALL_TIMEOUT, |owner| {
            (owner, SCShareableContent::get().ok())
        }) {
            BoundedNativeCall::Completed {
                result: Some(content),
                ..
            } => Ok(content),
            BoundedNativeCall::Completed { result: None, .. } => {
                Err(MacOsCaptureError::ShareableContentUnavailable)
            }
            BoundedNativeCall::NotStarted { error, .. } => Err(map_native_call_launch_error(error)),
            BoundedNativeCall::Unconfirmed(pending) => {
                let error = MacOsCaptureStopError::NativeStopUnconfirmed(
                    MacOsCaptureError::NativeOperationTimedOut,
                );
                self.capture = NativeCaptureLifecycle::NativeOperationUnconfirmed {
                    _pending: pending,
                    error,
                };
                Err(MacOsCaptureError::CaptureStartTeardownUnconfirmed)
            }
        }
    }

    pub fn start(&mut self, config: MacOsCaptureConfig) -> Result<(), MacOsCaptureError> {
        match &self.capture {
            NativeCaptureLifecycle::Ready => {}
            NativeCaptureLifecycle::Running(_) => return Err(MacOsCaptureError::AlreadyRunning),
            NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                return Err(MacOsCaptureError::CaptureTeardownUnconfirmed);
            }
        }
        if config.target().source_instance() != self.source_instance {
            return Err(MacOsCaptureError::StaleOrForeignTarget);
        }
        let target = self
            .target_map
            .get(&config.target())
            .copied()
            .ok_or(MacOsCaptureError::StaleOrForeignTarget)?;
        target.validate_output(config.output())?;
        if !ScreenCaptureAccess.preflight() {
            return Err(MacOsCaptureError::PermissionDenied);
        }
        self.permission_was_granted = true;

        let content = self.shareable_content()?;
        let resolved = resolve_capture_target(&content, target, current_process_id()?)?;
        let output = config.output();
        let interval_value = i64::try_from(output.nominal_frame_duration_ns)
            .map_err(|_| MacOsCaptureError::InvalidFrameDuration)?;
        let interval = CMTime::new(interval_value, 1_000_000_000);
        let mut configuration = SCStreamConfiguration::new()
            .with_width(output.width)
            .with_height(output.height)
            .with_pixel_format(NativePixelFormat::BGRA)
            .with_color_space_name(SRGB_COLOR_SPACE_NAME)
            .with_scales_to_fit(true)
            .with_shows_cursor(matches!(
                config.cursor(),
                frame_media::CursorCaptureMode::EmbeddedInFrame
            ))
            .with_minimum_frame_interval(&interval)
            .with_queue_depth(
                u32::try_from(CALLBACK_QUEUE_CAPACITY)
                    .map_err(|_| MacOsCaptureError::CaptureStartFailed)?,
            );
        if let Some(source_rect) = resolved.source_rect {
            configuration.set_source_rect(source_rect);
        }

        let (sender, receiver) = sync_channel(CALLBACK_QUEUE_CAPACITY);
        let (delegate_dropped_sender, delegate_dropped) = sync_channel(1);
        let unexpected_stop = Arc::new(AtomicBool::new(false));
        let delegate = CaptureDelegate {
            unexpected_stop: Arc::clone(&unexpected_stop),
            dropped: delegate_dropped_sender,
        };
        let stream = SCStream::new_with_delegate(&resolved.filter, &configuration, delegate);
        let callback_queue = DispatchQueue::new(CALLBACK_QUEUE_LABEL, DispatchQoS::UserInteractive);
        let callback_diagnostics = Arc::clone(&self.diagnostics);
        let mut active = ActiveCapture {
            stream: Some(stream),
            _pending_native_call: None,
            callback_queue,
            output_handler_id: None,
            receiver,
            delegate_dropped,
            unexpected_stop,
            frames: FrameAssembler::new(config.target(), output),
        };
        let output_handler_id = active.stream.as_mut().and_then(|stream| {
            stream.add_output_handler_with_queue(
                move |sample, output_type| {
                    if output_type != SCStreamOutputType::Screen {
                        increment(&callback_diagnostics.invalid_samples);
                        return;
                    }
                    deliver_callback_sample(&sender, sample, &callback_diagnostics);
                },
                SCStreamOutputType::Screen,
                Some(&active.callback_queue),
            )
        });
        let Some(output_handler_id) = output_handler_id else {
            return self
                .fail_capture_start(active, MacOsCaptureError::OutputHandlerRegistrationFailed);
        };
        active.output_handler_id = Some(output_handler_id);
        match run_stream_native_call(&mut active, |stream| stream.start_capture().is_ok()) {
            StreamNativeCall::Completed(true) => {}
            StreamNativeCall::Completed(false) => {
                return self.fail_capture_start(active, MacOsCaptureError::CaptureStartFailed);
            }
            StreamNativeCall::NotStarted(error) => {
                return self.fail_capture_start(active, map_native_call_launch_error(error));
            }
            StreamNativeCall::Unconfirmed => {
                let error = MacOsCaptureStopError::NativeStopUnconfirmed(
                    MacOsCaptureError::NativeOperationTimedOut,
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(MacOsCaptureError::CaptureStartTeardownUnconfirmed);
            }
        }
        self.capture = NativeCaptureLifecycle::Running(active);
        Ok(())
    }

    fn fail_capture_start(
        &mut self,
        mut active: ActiveCapture,
        start_error: MacOsCaptureError,
    ) -> Result<(), MacOsCaptureError> {
        match detach_capture_bridge(&mut active) {
            Ok(_) => Err(start_error),
            Err(teardown_error) => {
                self.capture = NativeCaptureLifecycle::StopUnconfirmed {
                    active,
                    error: MacOsCaptureStopError::CallbackQuiescenceUnconfirmed(teardown_error),
                };
                Err(MacOsCaptureError::CaptureStartTeardownUnconfirmed)
            }
        }
    }

    fn retain_unconfirmed_stop(&mut self, error: MacOsCaptureStopError) {
        let capture = mem::replace(&mut self.capture, NativeCaptureLifecycle::Ready);
        self.capture = capture.retain_unconfirmed(error);
    }

    /// Drain at most the three callback-queued samples and return one frame.
    /// Pixel locking and row copies happen here, never in the native callback.
    pub fn poll_frame(&mut self) -> Result<Option<MacOsCaptureFrame>, MacOsCaptureError> {
        match &self.capture {
            NativeCaptureLifecycle::Ready => return Err(MacOsCaptureError::NotRunning),
            NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                return Err(MacOsCaptureError::CaptureTeardownUnconfirmed);
            }
            NativeCaptureLifecycle::Running(_) => {}
        }

        let unexpected_stop = match &self.capture {
            NativeCaptureLifecycle::Running(active) => {
                observe_unexpected_stop(&active.unexpected_stop, &self.diagnostics)
            }
            NativeCaptureLifecycle::Ready
            | NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => false,
        };
        if unexpected_stop {
            self.retain_unconfirmed_stop(MacOsCaptureStopError::NativeStopUnconfirmed(
                MacOsCaptureError::UnexpectedStreamStop,
            ));
            return Err(MacOsCaptureError::UnexpectedStreamStop);
        }

        for _ in 0..CALLBACK_QUEUE_CAPACITY {
            let received = match &self.capture {
                NativeCaptureLifecycle::Running(active) => active.receiver.try_recv(),
                NativeCaptureLifecycle::Ready
                | NativeCaptureLifecycle::StopUnconfirmed { .. }
                | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                    return Err(MacOsCaptureError::CaptureTeardownUnconfirmed);
                }
            };
            let sample = match received {
                Ok(sample) => sample,
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => {
                    self.retain_unconfirmed_stop(MacOsCaptureStopError::NativeStopUnconfirmed(
                        MacOsCaptureError::CallbackQueueDisconnected,
                    ));
                    return Err(MacOsCaptureError::CallbackQueueDisconnected);
                }
            };
            let processed = match &mut self.capture {
                NativeCaptureLifecycle::Running(active) => {
                    process_sample(active, &sample, &self.diagnostics)
                }
                NativeCaptureLifecycle::Ready
                | NativeCaptureLifecycle::StopUnconfirmed { .. }
                | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                    return Err(MacOsCaptureError::CaptureTeardownUnconfirmed);
                }
            };
            match processed {
                Ok(ProcessedSample::Frame(frame)) => return Ok(Some(frame)),
                Ok(ProcessedSample::Ignored) => continue,
                Ok(ProcessedSample::Terminal) => {
                    self.retain_unconfirmed_stop(MacOsCaptureStopError::NativeStopUnconfirmed(
                        MacOsCaptureError::UnexpectedStreamStop,
                    ));
                    return Err(MacOsCaptureError::UnexpectedStreamStop);
                }
                Err(error) => {
                    increment(&self.diagnostics.invalid_samples);
                    return Err(error);
                }
            }
        }
        Ok(None)
    }

    /// Stop capture and return the bounded native callback tail.
    ///
    /// A successful call returns at most [`CALLBACK_QUEUE_CAPACITY`] frames.
    /// Callers finalizing a recording must ingest every returned frame before
    /// sending encoder EOS; otherwise the artifact can truncate a static
    /// trailing interval. Calling this repeatedly is safe and returns an empty
    /// tail after the first call.
    pub fn stop_and_drain_frames(
        &mut self,
    ) -> Result<Vec<MacOsCaptureFrame>, MacOsCaptureStopError> {
        let capture = mem::replace(&mut self.capture, NativeCaptureLifecycle::Ready);
        let mut active = match capture.take_for_stop() {
            Ok(None) => return Ok(Vec::new()),
            Ok(Some(active)) => active,
            Err((capture, error)) => {
                self.capture = capture;
                return Err(error);
            }
        };
        if observe_unexpected_stop(&active.unexpected_stop, &self.diagnostics) {
            let error = MacOsCaptureStopError::NativeStopUnconfirmed(
                MacOsCaptureError::UnexpectedStreamStop,
            );
            self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
            return Err(error);
        }
        match run_stream_native_call(&mut active, |stream| stream.stop_capture().is_ok()) {
            StreamNativeCall::Completed(true) => {}
            StreamNativeCall::Completed(false) => {
                let error = MacOsCaptureStopError::NativeStopUnconfirmed(
                    MacOsCaptureError::CaptureStopFailed,
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
            StreamNativeCall::NotStarted(launch_error) => {
                let error = MacOsCaptureStopError::NativeStopUnconfirmed(
                    map_native_call_launch_error(launch_error),
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
            StreamNativeCall::Unconfirmed => {
                let error = MacOsCaptureStopError::NativeStopUnconfirmed(
                    MacOsCaptureError::NativeOperationTimedOut,
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
        }
        let detached = match detach_capture_bridge(&mut active) {
            Ok(detached) => detached,
            Err(error) => {
                let error = MacOsCaptureStopError::CallbackQuiescenceUnconfirmed(error);
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
        };
        let post_teardown_error =
            if observe_unexpected_stop(&active.unexpected_stop, &self.diagnostics) {
                Some(MacOsCaptureError::UnexpectedStreamStop)
            } else if !detached.output_handler_registered {
                Some(MacOsCaptureError::OutputHandlerRemovalFailed)
            } else {
                None
            };
        if let Some(error) = post_teardown_error {
            self.capture = NativeCaptureLifecycle::Ready;
            return Err(MacOsCaptureStopError::CaptureFailedAfterTeardown(error));
        }
        let mut tail = Vec::with_capacity(detached.samples.len());
        for sample in detached.samples {
            match process_sample(&mut active, &sample, &self.diagnostics) {
                Ok(ProcessedSample::Frame(frame)) => tail.push(frame),
                Ok(ProcessedSample::Ignored) => {}
                Ok(ProcessedSample::Terminal) => break,
                Err(error) => {
                    increment(&self.diagnostics.invalid_samples);
                    self.capture = NativeCaptureLifecycle::Ready;
                    return Err(MacOsCaptureStopError::TailProcessingFailed(error));
                }
            }
        }
        self.capture = NativeCaptureLifecycle::Ready;
        Ok(tail)
    }

    /// Stop capture and deliberately discard its bounded callback tail.
    ///
    /// Recording finalizers should use [`Self::stop_and_drain_frames`] instead.
    pub fn stop(&mut self) -> Result<(), MacOsCaptureError> {
        self.stop_and_drain_frames()
            .map(drop)
            .map_err(MacOsCaptureStopError::into_capture_error)
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        match &self.capture {
            NativeCaptureLifecycle::Ready => false,
            NativeCaptureLifecycle::Running(_)
            | NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => true,
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> MacOsCaptureDiagnostics {
        self.diagnostics.snapshot()
    }
}

impl Drop for MacOsScreenCaptureSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct ResolvedCaptureTarget {
    filter: SCContentFilter,
    source_rect: Option<CGRect>,
}

fn current_process_id() -> Result<i32, MacOsCaptureError> {
    let pid = i32::try_from(std::process::id())
        .map_err(|_| MacOsCaptureError::CurrentProcessIdOutOfRange)?;
    if pid <= 0 {
        return Err(MacOsCaptureError::CurrentProcessIdOutOfRange);
    }
    Ok(pid)
}

fn resolve_capture_target(
    content: &SCShareableContent,
    target: NativeTargetRecord,
    current_pid: i32,
) -> Result<ResolvedCaptureTarget, MacOsCaptureError> {
    match target {
        NativeTargetRecord::Display(expected) => {
            let display = resolve_display(content, expected)?;
            let filter =
                display_filter_excluding_current_application(content, &display, current_pid)?;
            Ok(ResolvedCaptureTarget {
                filter,
                source_rect: None,
            })
        }
        NativeTargetRecord::Window(expected) => {
            let window = content
                .windows()
                .into_iter()
                .find(|window| window.window_id() == expected.window_id())
                .ok_or(MacOsCaptureError::TargetNoLongerAvailable)?;
            let observed = shareable_window_record(&window)
                .filter(|record| record.owner_pid() != current_pid)
                .ok_or(MacOsCaptureError::TargetNoLongerAvailable)?;
            if observed != expected {
                return Err(MacOsCaptureError::StaleTargetTopology);
            }
            let filter = SCContentFilter::create()
                .with_window(&window)
                .try_build()
                .map_err(|_| MacOsCaptureError::CaptureStartFailed)?;
            Ok(ResolvedCaptureTarget {
                filter,
                source_rect: None,
            })
        }
        NativeTargetRecord::Region {
            display: expected,
            logical_bounds,
        } => {
            let display = resolve_display(content, expected)?;
            let filter =
                display_filter_excluding_current_application(content, &display, current_pid)?;
            Ok(ResolvedCaptureTarget {
                filter,
                source_rect: Some(region_source_rect(expected, logical_bounds)?),
            })
        }
    }
}

fn resolve_display(
    content: &SCShareableContent,
    expected: NativeDisplayRecord,
) -> Result<SCDisplay, MacOsCaptureError> {
    let observed = active_display_records()?
        .into_iter()
        .find(|display| display.display_id() == expected.display_id())
        .ok_or(MacOsCaptureError::TargetNoLongerAvailable)?;
    if observed != expected {
        return Err(MacOsCaptureError::StaleTargetTopology);
    }
    content
        .displays()
        .into_iter()
        .find(|display| display.display_id() == expected.display_id())
        .ok_or(MacOsCaptureError::TargetNoLongerAvailable)
}

fn display_filter_excluding_current_application(
    content: &SCShareableContent,
    display: &SCDisplay,
    current_pid: i32,
) -> Result<SCContentFilter, MacOsCaptureError> {
    let current_application =
        select_current_application(content.applications(), current_pid, |application| {
            application.process_id()
        })?;
    SCContentFilter::create()
        .with_display(display)
        .with_excluding_applications(&[&current_application], &[])
        .try_build()
        .map_err(|_| MacOsCaptureError::CaptureStartFailed)
}

fn region_source_rect(
    display: NativeDisplayRecord,
    bounds: LogicalRect,
) -> Result<CGRect, MacOsCaptureError> {
    let display_bounds = display.transform().logical_bounds();
    if !display_bounds.contains_rect(bounds) {
        return Err(MacOsCaptureError::InvalidRegionGeometry);
    }
    let x = i64::from(bounds.x())
        .checked_sub(i64::from(display_bounds.x()))
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(MacOsCaptureError::InvalidRegionGeometry)?;
    let y = i64::from(bounds.y())
        .checked_sub(i64::from(display_bounds.y()))
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(MacOsCaptureError::InvalidRegionGeometry)?;
    // ScreenCaptureKit source rectangles are display-local logical points. The
    // normalized transform independently validated containment and the exact
    // physical output dimensions before this conversion.
    Ok(CGRect::new(
        f64::from(x),
        f64::from(y),
        f64::from(bounds.width()),
        f64::from(bounds.height()),
    ))
}

fn shareable_window_records(
    content: &SCShareableContent,
    current_pid: i32,
) -> Vec<NativeWindowRecord> {
    let windows = content
        .windows()
        .into_iter()
        .filter_map(|window| shareable_window_record(&window));
    exclude_current_process_windows(windows, current_pid)
}

fn shareable_window_record(window: &SCWindow) -> Option<NativeWindowRecord> {
    if !window.is_on_screen() || window.window_layer() != 0 || window.window_id() == 0 {
        return None;
    }
    let owner_pid = window.owning_application()?.process_id();
    if owner_pid <= 0 {
        return None;
    }
    let frame = window.frame();
    let logical_bounds = LogicalRect::new(
        integral_i32(frame.origin.x).ok()?,
        integral_i32(frame.origin.y).ok()?,
        integral_u32(frame.size.width).ok()?,
        integral_u32(frame.size.height).ok()?,
    )
    .ok()?;
    Some(NativeWindowRecord::new(
        window.window_id(),
        owner_pid,
        logical_bounds,
    ))
}

fn observe_unexpected_stop(unexpected_stop: &AtomicBool, diagnostics: &DiagnosticCounters) -> bool {
    let unexpected_stop = unexpected_stop.swap(false, Ordering::AcqRel);
    if unexpected_stop {
        increment(&diagnostics.unexpected_native_stops);
    }
    unexpected_stop
}

fn detach_capture_bridge(
    active: &mut ActiveCapture,
) -> Result<DetachedCaptureTail, MacOsCaptureError> {
    // Keep the Rust handler installed while releasing SCStream. The pinned
    // bridge retains StreamContext for both its output object and every
    // in-flight callback, so CaptureDelegate::drop cannot signal until a sample
    // queued after this first fence has run through the handler. The final
    // fence and disconnected bounded channel independently confirm the tail is
    // complete before it is processed.
    dispatch_async_and_wait(&active.callback_queue, || {});
    let output_handler_registered = active.output_handler_id.take().is_some();
    let Some(stream) = active.stream.take() else {
        return Err(MacOsCaptureError::DelegateQuiescenceUnconfirmed);
    };
    drop(stream);
    await_delegate_quiescence(&active.delegate_dropped, DELEGATE_QUIESCENCE_TIMEOUT)?;
    dispatch_async_and_wait(&active.callback_queue, || {});
    let samples = drain_quiescent_callback_tail(&active.receiver)?;
    Ok(DetachedCaptureTail {
        samples,
        output_handler_registered,
    })
}

fn drain_quiescent_callback_tail<T>(receiver: &Receiver<T>) -> Result<Vec<T>, MacOsCaptureError> {
    let mut tail = Vec::with_capacity(CALLBACK_QUEUE_CAPACITY);
    for _ in 0..CALLBACK_QUEUE_CAPACITY {
        match receiver.try_recv() {
            Ok(sample) => tail.push(sample),
            Err(TryRecvError::Disconnected) => return Ok(tail),
            Err(TryRecvError::Empty) => {
                return Err(MacOsCaptureError::OutputHandlerRemovalFailed);
            }
        }
    }
    match receiver.try_recv() {
        Err(TryRecvError::Disconnected) => Ok(tail),
        Ok(_) | Err(TryRecvError::Empty) => Err(MacOsCaptureError::OutputHandlerRemovalFailed),
    }
}

fn await_delegate_quiescence(
    delegate_dropped: &Receiver<()>,
    timeout: Duration,
) -> Result<(), MacOsCaptureError> {
    match delegate_dropped.recv_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            Err(MacOsCaptureError::DelegateQuiescenceUnconfirmed)
        }
    }
}

enum ProcessedSample {
    Frame(MacOsCaptureFrame),
    Ignored,
    Terminal,
}

fn process_sample(
    active: &mut ActiveCapture,
    sample: &CMSampleBuffer,
    diagnostics: &DiagnosticCounters,
) -> Result<ProcessedSample, MacOsCaptureError> {
    let status = sample
        .frame_status()
        .ok_or(MacOsCaptureError::MissingFrameStatus)?;
    let assembly = match frame_status_disposition(status) {
        FrameStatusDisposition::Content => {
            let (pixels, pts, duration) = extract_owned_bgra(sample, active.frames.spec())?;
            Some(active.frames.accept_complete(pixels, pts, duration)?)
        }
        FrameStatusDisposition::RepeatLastContent => {
            if !sample.is_valid() {
                return Err(MacOsCaptureError::InvalidSampleBuffer);
            }
            let pts = raw_media_time(sample.presentation_timestamp());
            active.frames.accept_idle(pts)?
        }
        FrameStatusDisposition::IgnoreWithDiscontinuity => {
            increment(&diagnostics.ignored_non_content_samples);
            active.frames.mark_non_content_discontinuity();
            return Ok(ProcessedSample::Ignored);
        }
        FrameStatusDisposition::Terminal => return Ok(ProcessedSample::Terminal),
    };
    let Some(assembly) = assembly else {
        increment(&diagnostics.ignored_non_content_samples);
        return Ok(ProcessedSample::Ignored);
    };
    if assembly.used_nominal_duration {
        increment(&diagnostics.duration_fallbacks);
    }
    if assembly.frame.timestamp().discontinuity {
        increment(&diagnostics.timestamp_discontinuities);
    }
    Ok(ProcessedSample::Frame(assembly.frame))
}

fn select_current_application<T>(
    applications: Vec<T>,
    current_pid: i32,
    mut process_id: impl FnMut(&T) -> i32,
) -> Result<T, MacOsCaptureError> {
    if current_pid <= 0 {
        return Err(MacOsCaptureError::CurrentProcessIdOutOfRange);
    }
    let mut current_application = None;
    for application in applications {
        if process_id(&application) == current_pid {
            if current_application.is_some() {
                return Err(MacOsCaptureError::AmbiguousCurrentApplication);
            }
            current_application = Some(application);
        }
    }
    current_application.ok_or(MacOsCaptureError::CurrentApplicationUnavailable)
}

fn active_display_records() -> Result<Vec<NativeDisplayRecord>, MacOsCaptureError> {
    let mut display_ids =
        CGDisplay::active_displays().map_err(|_| MacOsCaptureError::DisplayCatalogUnavailable)?;
    display_ids.sort_unstable();
    display_ids
        .into_iter()
        .take(MAX_SCREEN_TARGETS.saturating_add(1))
        .map(|display_id| {
            let display = CGDisplay::new(display_id);
            let bounds = display.bounds();
            let logical_bounds = LogicalRect::new(
                integral_i32(bounds.origin.x)?,
                integral_i32(bounds.origin.y)?,
                integral_u32(bounds.size.width)?,
                integral_u32(bounds.size.height)?,
            )
            .map_err(|_| MacOsCaptureError::InvalidDisplayGeometry)?;
            let physical_width = u32::try_from(display.pixels_wide())
                .map_err(|_| MacOsCaptureError::InvalidDisplayGeometry)?;
            let physical_height = u32::try_from(display.pixels_high())
                .map_err(|_| MacOsCaptureError::InvalidDisplayGeometry)?;
            let rotation = rotation_from_degrees(display.rotation())?;
            let transform =
                display_transform(logical_bounds, physical_width, physical_height, rotation)?;
            Ok(NativeDisplayRecord::new(display_id, transform))
        })
        .collect()
}

fn display_transform(
    logical_bounds: LogicalRect,
    physical_width: u32,
    physical_height: u32,
    rotation: Rotation,
) -> Result<DisplayGeometryTransform, MacOsCaptureError> {
    // Core Graphics reports drawable physical axes. A quarter-turn swaps the
    // logical axis used to derive the exact scale; the media contract then
    // validates the other axis and rejects inconsistent native data.
    let scale_denominator = match rotation {
        Rotation::Degrees0 | Rotation::Degrees180 => logical_bounds.width(),
        Rotation::Degrees90 | Rotation::Degrees270 => logical_bounds.height(),
    };
    let scale = DpiScale::new(physical_width, scale_denominator)
        .map_err(|_| MacOsCaptureError::InvalidDisplayGeometry)?;
    let physical_bounds = PhysicalRect::new(0, 0, physical_width, physical_height)
        .map_err(|_| MacOsCaptureError::InvalidDisplayGeometry)?;
    DisplayGeometryTransform::new(logical_bounds, physical_bounds, scale, rotation)
        .map_err(|_| MacOsCaptureError::InvalidDisplayGeometry)
}

#[allow(clippy::cast_possible_truncation)]
fn integral_i32(value: f64) -> Result<i32, MacOsCaptureError> {
    if !value.is_finite() {
        return Err(MacOsCaptureError::InvalidDisplayGeometry);
    }
    let rounded = value.round();
    if (value - rounded).abs() > GEOMETRY_INTEGER_TOLERANCE
        || rounded < f64::from(i32::MIN)
        || rounded > f64::from(i32::MAX)
    {
        return Err(MacOsCaptureError::InvalidDisplayGeometry);
    }
    Ok(rounded as i32)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn integral_u32(value: f64) -> Result<u32, MacOsCaptureError> {
    if !value.is_finite() {
        return Err(MacOsCaptureError::InvalidDisplayGeometry);
    }
    let rounded = value.round();
    if (value - rounded).abs() > GEOMETRY_INTEGER_TOLERANCE
        || rounded < 1.0
        || rounded > f64::from(u32::MAX)
    {
        return Err(MacOsCaptureError::InvalidDisplayGeometry);
    }
    Ok(rounded as u32)
}

fn rotation_from_degrees(value: f64) -> Result<Rotation, MacOsCaptureError> {
    if !value.is_finite() {
        return Err(MacOsCaptureError::InvalidDisplayGeometry);
    }
    let normalized = value.rem_euclid(360.0);
    if normalized <= GEOMETRY_INTEGER_TOLERANCE
        || (360.0 - normalized) <= GEOMETRY_INTEGER_TOLERANCE
    {
        Ok(Rotation::Degrees0)
    } else if (normalized - 90.0).abs() <= GEOMETRY_INTEGER_TOLERANCE {
        Ok(Rotation::Degrees90)
    } else if (normalized - 180.0).abs() <= GEOMETRY_INTEGER_TOLERANCE {
        Ok(Rotation::Degrees180)
    } else if (normalized - 270.0).abs() <= GEOMETRY_INTEGER_TOLERANCE {
        Ok(Rotation::Degrees270)
    } else {
        Err(MacOsCaptureError::InvalidDisplayGeometry)
    }
}

fn extract_owned_bgra(
    sample: &CMSampleBuffer,
    spec: frame_media::VideoFrameSpec,
) -> Result<(Vec<u8>, RawMediaTime, RawMediaTime), MacOsCaptureError> {
    if !sample.is_valid() || !sample.data_is_ready() {
        return Err(MacOsCaptureError::InvalidSampleBuffer);
    }
    let pts = raw_media_time(sample.presentation_timestamp());
    let duration = raw_media_time(sample.duration());
    let image = sample
        .image_buffer()
        .ok_or(MacOsCaptureError::MissingImageBuffer)?;
    if NativePixelFormat::from(image.pixel_format()) != NativePixelFormat::BGRA {
        return Err(MacOsCaptureError::UnexpectedPixelFormat);
    }
    let expected_width =
        usize::try_from(spec.width).map_err(|_| MacOsCaptureError::FrameAllocationExceedsLimit)?;
    let expected_height =
        usize::try_from(spec.height).map_err(|_| MacOsCaptureError::FrameAllocationExceedsLimit)?;
    if image.width() != expected_width || image.height() != expected_height {
        return Err(MacOsCaptureError::UnexpectedFrameDimensions {
            expected_width,
            expected_height,
            actual_width: image.width(),
            actual_height: image.height(),
        });
    }
    let guard = image
        .lock(CVPixelBufferLockFlags::READ_ONLY)
        .map_err(|_| MacOsCaptureError::PixelBufferLockFailed)?;
    guard
        .height()
        .checked_mul(guard.bytes_per_row())
        .ok_or(MacOsCaptureError::FrameAllocationExceedsLimit)?;
    let pixels = copy_bgra_rows(
        guard.as_slice(),
        guard.width(),
        guard.height(),
        guard.bytes_per_row(),
    )?;
    Ok((pixels, pts, duration))
}

fn raw_media_time(value: CMTime) -> RawMediaTime {
    RawMediaTime {
        value: value.value,
        timescale: value.timescale,
        epoch: value.epoch,
        numeric: value.is_valid()
            && !value.is_indefinite()
            && !value.is_positive_infinity()
            && !value.is_negative_infinity(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apple_cf::dispatch_queue::dispatch_async;
    use frame_media::{ScreenTargetEpoch, ScreenTargetId, ScreenTargetKind};

    #[test]
    fn application_filter_selects_exactly_the_current_pid() {
        let applications = vec![(11_u8, 100), (12, 200), (13, 300)];
        assert_eq!(
            select_current_application(applications, 200, |application| application.1),
            Ok((12, 200))
        );
    }

    #[test]
    fn application_filter_fails_closed_on_missing_ambiguous_or_invalid_current_pid() {
        assert_eq!(
            select_current_application(vec![(1_u8, 100)], 200, |application| application.1),
            Err(MacOsCaptureError::CurrentApplicationUnavailable)
        );
        assert_eq!(
            select_current_application(vec![(1_u8, 200), (2, 200)], 200, |application| application
                .1,),
            Err(MacOsCaptureError::AmbiguousCurrentApplication)
        );
        assert_eq!(
            select_current_application(vec![(1_u8, 200)], 0, |application| application.1),
            Err(MacOsCaptureError::CurrentProcessIdOutOfRange)
        );
    }

    #[test]
    fn rotation_accepts_only_quarter_turns() {
        assert_eq!(rotation_from_degrees(0.0), Ok(Rotation::Degrees0));
        assert_eq!(rotation_from_degrees(-90.0), Ok(Rotation::Degrees270));
        assert_eq!(rotation_from_degrees(450.0), Ok(Rotation::Degrees90));
        assert_eq!(
            rotation_from_degrees(45.0),
            Err(MacOsCaptureError::InvalidDisplayGeometry)
        );
    }

    #[test]
    fn geometry_conversion_rejects_fractional_or_non_finite_values() {
        assert_eq!(integral_i32(-1.0), Ok(-1));
        assert_eq!(integral_u32(1.0), Ok(1));
        assert_eq!(
            integral_u32(1.5),
            Err(MacOsCaptureError::InvalidDisplayGeometry)
        );
        assert_eq!(
            integral_i32(f64::NAN),
            Err(MacOsCaptureError::InvalidDisplayGeometry)
        );
    }

    #[test]
    fn region_source_rect_is_display_local_and_preserves_negative_desktop_origins() {
        let logical = LogicalRect::new(-1_920, -40, 1_920, 1_080).expect("logical");
        let display = NativeDisplayRecord::new(
            7,
            display_transform(logical, 3_840, 2_160, Rotation::Degrees0).expect("transform"),
        );
        let rect = region_source_rect(
            display,
            LogicalRect::new(-1_820, 10, 640, 480).expect("region"),
        )
        .expect("source rect");
        assert_eq!(rect.origin.x, 100.0);
        assert_eq!(rect.origin.y, 50.0);
        assert_eq!(rect.size.width, 640.0);
        assert_eq!(rect.size.height, 480.0);
    }

    #[test]
    fn stop_is_idempotent_before_start() {
        let source_id = ScreenSourceInstanceId::new([7; 16]).expect("source");
        let mut source = MacOsScreenCaptureSource::new(source_id, [8; 32]).expect("adapter");
        assert_eq!(source.stop(), Ok(()));
        assert_eq!(source.stop(), Ok(()));
    }

    #[test]
    fn unconfirmed_stop_state_rejects_every_repeated_stop_attempt() {
        let stop_error =
            MacOsCaptureStopError::NativeStopUnconfirmed(MacOsCaptureError::CaptureStopFailed);
        let mut capture = NativeCaptureLifecycle::Running(7_u8).retain_unconfirmed(stop_error);

        for _ in 0..2 {
            let (retained, observed) = capture
                .take_for_stop()
                .expect_err("unconfirmed teardown cannot become an idempotent stop");
            assert_eq!(observed, stop_error);
            capture = retained;
        }
        let NativeCaptureLifecycle::StopUnconfirmed {
            active,
            error: retained,
        } = capture
        else {
            panic!("unconfirmed stop state must retain native authority");
        };
        assert_eq!(active, 7);
        assert_eq!(retained, stop_error);
    }

    #[test]
    fn callback_queue_fences_expose_late_delivery_and_diagnostics() {
        let queue = DispatchQueue::new(
            "xyz.eng-manager.frame.screen-capture.test",
            DispatchQoS::UserInteractive,
        );
        let diagnostics = Arc::new(DiagnosticCounters::default());
        let (sender, receiver) = sync_channel(CALLBACK_QUEUE_CAPACITY);
        let dispatch_sample = |sample| {
            let sender = sender.clone();
            let diagnostics = Arc::clone(&diagnostics);
            dispatch_async(&queue, move || {
                deliver_callback_sample(&sender, sample, &diagnostics);
            });
        };

        dispatch_sample(1_u8);
        dispatch_async_and_wait(&queue, || {});
        dispatch_sample(2_u8);
        dispatch_async_and_wait(&queue, || {});
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), vec![1, 2]);

        drop(receiver);
        dispatch_sample(3_u8);
        dispatch_async_and_wait(&queue, || {});
        let diagnostics = diagnostics.snapshot();
        assert_eq!(diagnostics.callback_frames_after_stop, 1);
        assert_eq!(diagnostics.dropped_callback_frames, 0);
    }

    #[test]
    fn callback_queued_between_first_fence_and_teardown_is_in_the_tail() {
        let queue = DispatchQueue::new(
            "xyz.eng-manager.frame.screen-capture.tail-race-test",
            DispatchQoS::UserInteractive,
        );
        let diagnostics = Arc::new(DiagnosticCounters::default());
        let (sender, receiver) = sync_channel(CALLBACK_QUEUE_CAPACITY);
        let dispatch_sample = |sample| {
            let sender = sender.clone();
            let diagnostics = Arc::clone(&diagnostics);
            dispatch_async(&queue, move || {
                deliver_callback_sample(&sender, sample, &diagnostics);
            });
        };

        dispatch_sample(1_u8);
        dispatch_async_and_wait(&queue, || {});
        dispatch_sample(2_u8);

        // Releasing the stream can drop the registered sender, but the queued
        // callback retains its bridge/context authority until it runs.
        drop(sender);
        dispatch_async_and_wait(&queue, || {});

        assert_eq!(drain_quiescent_callback_tail(&receiver), Ok(vec![1, 2]));
        assert_eq!(diagnostics.snapshot().dropped_callback_frames, 0);
    }

    #[test]
    fn callback_tail_requires_output_sender_release() {
        let (sender, receiver) = sync_channel::<u8>(CALLBACK_QUEUE_CAPACITY);
        assert_eq!(
            drain_quiescent_callback_tail(&receiver),
            Err(MacOsCaptureError::OutputHandlerRemovalFailed)
        );
        drop(sender);
        assert_eq!(drain_quiescent_callback_tail(&receiver), Ok(Vec::new()));
    }

    #[test]
    fn delegate_drop_proves_quiescence_and_defers_diagnostics_to_the_worker() {
        let diagnostics = DiagnosticCounters::default();
        let unexpected_stop = Arc::new(AtomicBool::new(false));
        let (dropped, delegate_dropped) = sync_channel(1);
        let delegate = CaptureDelegate {
            unexpected_stop: Arc::clone(&unexpected_stop),
            dropped,
        };

        delegate.did_stop_with_error(SCError::StreamError("test stop".to_owned()));
        assert!(unexpected_stop.load(Ordering::Acquire));
        assert_eq!(diagnostics.snapshot().unexpected_native_stops, 0);

        drop(delegate);
        assert_eq!(
            await_delegate_quiescence(&delegate_dropped, Duration::ZERO),
            Ok(())
        );
        assert!(observe_unexpected_stop(&unexpected_stop, &diagnostics));
        assert!(!observe_unexpected_stop(&unexpected_stop, &diagnostics));
        assert_eq!(diagnostics.snapshot().unexpected_native_stops, 1);
    }

    #[test]
    fn delegate_quiescence_wait_fails_closed_at_its_bound() {
        let (dropped, delegate_dropped) = sync_channel(1);
        assert_eq!(
            await_delegate_quiescence(&delegate_dropped, Duration::ZERO),
            Err(MacOsCaptureError::DelegateQuiescenceUnconfirmed)
        );
        drop(dropped);
        assert_eq!(
            await_delegate_quiescence(&delegate_dropped, Duration::ZERO),
            Err(MacOsCaptureError::DelegateQuiescenceUnconfirmed)
        );
    }

    #[test]
    fn statuses_distinguish_pixels_repetition_discontinuities_and_terminal_stop() {
        assert_eq!(
            frame_status_disposition(SCFrameStatus::Complete),
            FrameStatusDisposition::Content
        );
        assert_eq!(
            frame_status_disposition(SCFrameStatus::Idle),
            FrameStatusDisposition::RepeatLastContent
        );
        for status in [
            SCFrameStatus::Blank,
            SCFrameStatus::Suspended,
            SCFrameStatus::Started,
        ] {
            assert_eq!(
                frame_status_disposition(status),
                FrameStatusDisposition::IgnoreWithDiscontinuity
            );
        }
        assert_eq!(
            frame_status_disposition(SCFrameStatus::Stopped),
            FrameStatusDisposition::Terminal
        );
    }

    #[test]
    fn idle_tail_repeats_last_complete_frame_with_monotonic_nominal_timing() {
        let source = ScreenSourceInstanceId::new([3; 16]).expect("source");
        let epoch = ScreenTargetEpoch::new(1).expect("epoch");
        let target_id = ScreenTargetId::new(ScreenTargetKind::Display, [4; 16]).expect("target");
        let target = ScreenTargetBinding::new(source, 1, epoch, target_id).expect("binding");
        let spec = frame_media::VideoFrameSpec {
            width: 1,
            height: 1,
            pixel_format: frame_media::PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: frame_media::FrameMemory::Cpu,
        };
        let mut frames = FrameAssembler::new(target, spec);
        let pixels = vec![1, 2, 3, 4];
        let complete = frames
            .accept_complete(
                pixels.clone(),
                RawMediaTime::numeric(0, 30, 0),
                RawMediaTime::numeric(1, 30, 0),
            )
            .expect("complete");
        let first_idle = frames
            .accept_idle(RawMediaTime::numeric(1, 30, 0))
            .expect("first idle")
            .expect("cached frame");
        let second_idle = frames
            .accept_idle(RawMediaTime::numeric(2, 30, 0))
            .expect("second idle")
            .expect("cached frame");

        assert_eq!(complete.frame.sequence(), 1);
        assert_eq!(first_idle.frame.sequence(), 2);
        assert_eq!(second_idle.frame.sequence(), 3);
        assert_eq!(complete.frame.pixels(), pixels);
        assert_eq!(first_idle.frame.pixels(), pixels);
        assert_eq!(second_idle.frame.pixels(), pixels);
        assert_eq!(complete.frame.timestamp().pts_ns, 0);
        assert_eq!(first_idle.frame.timestamp().pts_ns, 33_333_333);
        assert_eq!(second_idle.frame.timestamp().pts_ns, 66_666_666);
        assert_eq!(first_idle.frame.timestamp().duration_ns, 33_333_333);
        assert_eq!(second_idle.frame.timestamp().duration_ns, 33_333_333);
        assert!(!first_idle.used_nominal_duration);
        assert!(!second_idle.used_nominal_duration);
    }

    #[test]
    fn idle_before_first_complete_frame_is_bounded_and_ignored() {
        let source = ScreenSourceInstanceId::new([5; 16]).expect("source");
        let epoch = ScreenTargetEpoch::new(1).expect("epoch");
        let target_id = ScreenTargetId::new(ScreenTargetKind::Display, [6; 16]).expect("target");
        let target = ScreenTargetBinding::new(source, 1, epoch, target_id).expect("binding");
        let spec = frame_media::VideoFrameSpec {
            width: 1,
            height: 1,
            pixel_format: frame_media::PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: frame_media::FrameMemory::Cpu,
        };
        let mut frames = FrameAssembler::new(target, spec);
        assert!(
            frames
                .accept_idle(RawMediaTime::numeric(1, 30, 0))
                .expect("idle")
                .is_none()
        );
    }

    #[test]
    fn skipped_non_content_state_marks_the_next_idle_duplicate_discontinuous() {
        let source = ScreenSourceInstanceId::new([11; 16]).expect("source");
        let epoch = ScreenTargetEpoch::new(1).expect("epoch");
        let target_id = ScreenTargetId::new(ScreenTargetKind::Display, [12; 16]).expect("target");
        let target = ScreenTargetBinding::new(source, 1, epoch, target_id).expect("binding");
        let spec = frame_media::VideoFrameSpec {
            width: 1,
            height: 1,
            pixel_format: frame_media::PixelFormat::Bgra8,
            color_space: frame_media::ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: frame_media::FrameMemory::Cpu,
        };
        let mut frames = FrameAssembler::new(target, spec);
        frames
            .accept_complete(
                vec![1, 2, 3, 4],
                RawMediaTime::numeric(0, 30, 0),
                RawMediaTime::numeric(1, 30, 0),
            )
            .expect("complete");
        frames.mark_non_content_discontinuity();
        let idle = frames
            .accept_idle(RawMediaTime::numeric(1, 30, 0))
            .expect("idle")
            .expect("cached frame");

        assert_eq!(idle.frame.sequence(), 2);
        assert_eq!(idle.frame.pixels(), &[1, 2, 3, 4]);
        assert!(idle.frame.timestamp().discontinuity);
    }

    #[test]
    fn source_can_move_to_a_serial_worker_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<MacOsScreenCaptureSource>();
    }

    #[test]
    fn rotated_display_geometry_swaps_axes_and_rejects_mismatches() {
        let logical = LogicalRect::new(-4, 2, 4, 2).expect("logical");
        let transform =
            display_transform(logical, 3, 6, Rotation::Degrees90).expect("quarter-turn transform");
        assert_eq!(transform.scale(), DpiScale::new(3, 2).expect("scale"));
        assert_eq!(transform.physical_bounds().width(), 3);
        assert_eq!(transform.physical_bounds().height(), 6);
        assert_eq!(
            display_transform(logical, 3, 5, Rotation::Degrees90),
            Err(MacOsCaptureError::InvalidDisplayGeometry)
        );
    }

    #[test]
    #[ignore = "requires an interactive macOS display session"]
    fn live_display_catalog_is_well_formed_and_bound_to_the_source() {
        let source_id = ScreenSourceInstanceId::new([9; 16]).expect("source");
        let mut source = MacOsScreenCaptureSource::new(source_id, [10; 32]).expect("adapter");
        let catalog = source.enumerate_displays().expect("display catalog");
        assert_eq!(catalog.source_instance(), source_id);
        assert!(
            catalog
                .targets()
                .iter()
                .all(|target| target.kind() == ScreenTargetKind::Display)
        );
    }
}
