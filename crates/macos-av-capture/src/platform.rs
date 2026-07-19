use std::{
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    time::Duration,
};

use apple_cf::dispatch_queue::{DispatchQoS, DispatchQueue, dispatch_async};
use core_graphics::access::ScreenCaptureAccess;
use frame_media::{PermissionPreflight, PermissionState, SettingsGuidance};
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferExt, CMTime},
    prelude::{
        SCContentFilter, SCError, SCShareableContent, SCStream, SCStreamConfiguration,
        SCStreamDelegateTrait, SCStreamOutputType,
    },
};
use zeroize::Zeroizing;

use crate::{
    AUDIO_CALLBACK_QUEUE_CAPACITY, AudioPlane, MAX_AUDIO_CHUNK_FRAMES, MacOsSystemAudioChunk,
    MacOsSystemAudioDevice, MacOsSystemAudioDiagnostics, MacOsSystemAudioError,
    MacOsSystemAudioStopError, NativeAudioDescription, SYSTEM_AUDIO_SAMPLE_RATE_HZ,
    derive_system_audio_device_id, extract_stereo_f32le, validate_audio_description,
};

mod native_call;

use native_call::{
    BoundedNativeCall, NativeCallLaunchError, PendingNativeCall, run_bounded_native_call,
};

const CALLBACK_QUEUE_LABEL: &str = "xyz.eng-manager.frame.system-audio";
const CALLBACK_QUEUE_FENCE_TIMEOUT: Duration = Duration::from_secs(1);
const DELEGATE_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(1);
const NATIVE_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const TIMESTAMP_GAP_DISCONTINUITY_NS: u64 = 2_000_000_000;

#[derive(Default)]
struct DiagnosticCounters {
    dropped_callback_chunks: AtomicU64,
    callback_chunks_after_stop: AtomicU64,
    invalid_callback_chunks: AtomicU64,
    unexpected_native_stops: AtomicU64,
}

impl DiagnosticCounters {
    fn snapshot(&self) -> MacOsSystemAudioDiagnostics {
        MacOsSystemAudioDiagnostics {
            dropped_callback_chunks: self.dropped_callback_chunks.load(Ordering::Relaxed),
            callback_chunks_after_stop: self.callback_chunks_after_stop.load(Ordering::Relaxed),
            invalid_callback_chunks: self.invalid_callback_chunks.load(Ordering::Relaxed),
            unexpected_native_stops: self.unexpected_native_stops.load(Ordering::Relaxed),
        }
    }
}

fn increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

fn deliver_callback_chunk<T>(
    sender: &SyncSender<T>,
    chunk: T,
    diagnostics: &DiagnosticCounters,
) -> bool {
    match sender.try_send(chunk) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            increment(&diagnostics.dropped_callback_chunks);
            false
        }
        Err(TrySendError::Disconnected(_)) => {
            increment(&diagnostics.callback_chunks_after_stop);
            false
        }
    }
}

#[derive(Default)]
struct CallbackState {
    sequence: AtomicU64,
    last_end_ns: AtomicU64,
    force_discontinuity: AtomicBool,
}

struct ActiveCapture {
    stream: Option<SCStream>,
    _pending_native_call: Option<PendingNativeCall>,
    callback_queue: DispatchQueue,
    output_handler_id: Option<usize>,
    receiver: Receiver<MacOsSystemAudioChunk>,
    delegate_dropped: Receiver<()>,
    unexpected_stop: Arc<AtomicBool>,
}

struct DetachedCaptureTail {
    chunks: Vec<MacOsSystemAudioChunk>,
    output_handler_registered: bool,
}

struct CaptureDelegate {
    unexpected_stop: Arc<AtomicBool>,
    dropped: SyncSender<()>,
}

impl SCStreamDelegateTrait for CaptureDelegate {
    fn did_stop_with_error(&self, _error: SCError) {
        self.unexpected_stop.store(true, Ordering::Release);
    }
}

impl Drop for CaptureDelegate {
    fn drop(&mut self) {
        // Capacity one is sufficient because this delegate is dropped once.
        let _ = self.dropped.try_send(());
    }
}

enum NativeCaptureLifecycle<Active> {
    Ready,
    Running(Active),
    StopUnconfirmed {
        active: Active,
        error: MacOsSystemAudioStopError,
    },
    NativeOperationUnconfirmed {
        _pending: PendingNativeCall,
        error: MacOsSystemAudioStopError,
    },
}

impl<Active> NativeCaptureLifecycle<Active> {
    fn retain_unconfirmed(self, error: MacOsSystemAudioStopError) -> Self {
        match self {
            Self::Running(active) => Self::StopUnconfirmed { active, error },
            other => other,
        }
    }

    fn take_for_stop(self) -> Result<Option<Active>, (Self, MacOsSystemAudioStopError)> {
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

const fn map_native_call_launch_error(error: NativeCallLaunchError) -> MacOsSystemAudioError {
    match error {
        NativeCallLaunchError::CapacityUnavailable => {
            MacOsSystemAudioError::NativeOperationCapacityUnavailable
        }
        NativeCallLaunchError::WorkerUnavailable => {
            MacOsSystemAudioError::NativeOperationWorkerUnavailable
        }
    }
}

fn run_stream_native_call<R, F>(active: &mut ActiveCapture, operation: F) -> StreamNativeCall<R>
where
    R: Send + 'static,
    F: FnOnce(&SCStream) -> R + Send + 'static,
{
    let Some(stream) = active.stream.take() else {
        return StreamNativeCall::Unconfirmed;
    };
    match run_bounded_native_call(stream, NATIVE_CALL_TIMEOUT, move |stream| {
        let result = operation(&stream);
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

pub struct MacOsSystemAudioSource {
    device_id: frame_media::AvDeviceId,
    permission_requested: bool,
    permission_was_granted: bool,
    diagnostics: Arc<DiagnosticCounters>,
    capture: NativeCaptureLifecycle<ActiveCapture>,
}

impl MacOsSystemAudioSource {
    pub fn new(installation_secret: [u8; 32]) -> Result<Self, MacOsSystemAudioError> {
        let installation_secret = Zeroizing::new(installation_secret);
        Ok(Self {
            device_id: derive_system_audio_device_id(&installation_secret)?,
            permission_requested: false,
            permission_was_granted: false,
            diagnostics: Arc::new(DiagnosticCounters::default()),
            capture: NativeCaptureLifecycle::Ready,
        })
    }

    #[must_use]
    pub fn device(&mut self) -> MacOsSystemAudioDevice {
        MacOsSystemAudioDevice {
            id: self.device_id,
            permission: permission_state(self.preflight_permission()),
        }
    }

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

    /// Trigger Screen Recording permission only in response to an explicit
    /// user action. System audio uses the same TCC category as screen capture.
    pub fn request_permission(&mut self) -> PermissionPreflight {
        self.permission_requested = true;
        if ScreenCaptureAccess.request() {
            self.permission_was_granted = true;
            PermissionPreflight::Granted
        } else {
            PermissionPreflight::Denied(SettingsGuidance::OpenSystemSettings)
        }
    }

    pub fn start(&mut self) -> Result<(), MacOsSystemAudioError> {
        match &self.capture {
            NativeCaptureLifecycle::Ready => {}
            NativeCaptureLifecycle::Running(_) => {
                return Err(MacOsSystemAudioError::AlreadyRunning);
            }
            NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                return Err(MacOsSystemAudioError::CaptureTeardownUnconfirmed);
            }
        }
        if !ScreenCaptureAccess.preflight() {
            return Err(MacOsSystemAudioError::PermissionDenied);
        }
        self.permission_was_granted = true;

        let content = match run_bounded_native_call((), NATIVE_CALL_TIMEOUT, |owner| {
            (owner, SCShareableContent::get().ok())
        }) {
            BoundedNativeCall::Completed {
                result: Some(content),
                ..
            } => content,
            BoundedNativeCall::Completed { result: None, .. } => {
                return Err(MacOsSystemAudioError::ShareableContentUnavailable);
            }
            BoundedNativeCall::NotStarted { error, .. } => {
                return Err(map_native_call_launch_error(error));
            }
            BoundedNativeCall::Unconfirmed(pending) => {
                let error = MacOsSystemAudioStopError::NativeStopUnconfirmed(
                    MacOsSystemAudioError::NativeOperationTimedOut,
                );
                self.capture = NativeCaptureLifecycle::NativeOperationUnconfirmed {
                    _pending: pending,
                    error,
                };
                return Err(MacOsSystemAudioError::CaptureStartTeardownUnconfirmed);
            }
        };
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or(MacOsSystemAudioError::NoDisplayAvailable)?;
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();
        let configuration = SCStreamConfiguration::new()
            .with_captures_audio(true)
            .with_excludes_current_process_audio(true)
            .with_sample_rate(48_000)
            .with_channel_count(2);

        let (sender, receiver) = sync_channel(AUDIO_CALLBACK_QUEUE_CAPACITY);
        let (delegate_dropped_sender, delegate_dropped) = sync_channel(1);
        let unexpected_stop = Arc::new(AtomicBool::new(false));
        let delegate = CaptureDelegate {
            unexpected_stop: Arc::clone(&unexpected_stop),
            dropped: delegate_dropped_sender,
        };
        let stream = SCStream::new_with_delegate(&filter, &configuration, delegate);
        let callback_queue = DispatchQueue::new(CALLBACK_QUEUE_LABEL, DispatchQoS::UserInteractive);
        let callback_state = Arc::new(CallbackState::default());
        let callback_diagnostics = Arc::clone(&self.diagnostics);
        let mut active = ActiveCapture {
            stream: Some(stream),
            _pending_native_call: None,
            callback_queue,
            output_handler_id: None,
            receiver,
            delegate_dropped,
            unexpected_stop,
        };
        let output_handler_id = active.stream.as_mut().and_then(|stream| {
            stream.add_output_handler_with_queue(
                move |sample, output_type| {
                    if output_type != SCStreamOutputType::Audio {
                        increment(&callback_diagnostics.invalid_callback_chunks);
                        callback_state
                            .force_discontinuity
                            .store(true, Ordering::Release);
                        return;
                    }
                    process_callback_sample(
                        &sample,
                        &sender,
                        &callback_state,
                        &callback_diagnostics,
                    );
                },
                SCStreamOutputType::Audio,
                Some(&active.callback_queue),
            )
        });
        let Some(output_handler_id) = output_handler_id else {
            return self.fail_capture_start(
                active,
                MacOsSystemAudioError::OutputHandlerRegistrationFailed,
            );
        };
        active.output_handler_id = Some(output_handler_id);
        match run_stream_native_call(&mut active, |stream| stream.start_capture().is_ok()) {
            StreamNativeCall::Completed(true) => {}
            StreamNativeCall::Completed(false) => {
                return self.fail_capture_start(active, MacOsSystemAudioError::CaptureStartFailed);
            }
            StreamNativeCall::NotStarted(error) => {
                return self.fail_capture_start(active, map_native_call_launch_error(error));
            }
            StreamNativeCall::Unconfirmed => {
                let error = MacOsSystemAudioStopError::NativeStopUnconfirmed(
                    MacOsSystemAudioError::NativeOperationTimedOut,
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(MacOsSystemAudioError::CaptureStartTeardownUnconfirmed);
            }
        }
        self.capture = NativeCaptureLifecycle::Running(active);
        Ok(())
    }

    fn fail_capture_start(
        &mut self,
        mut active: ActiveCapture,
        start_error: MacOsSystemAudioError,
    ) -> Result<(), MacOsSystemAudioError> {
        match detach_capture_bridge(&mut active) {
            Ok(_) => Err(start_error),
            Err(teardown_error) => {
                self.capture = NativeCaptureLifecycle::StopUnconfirmed {
                    active,
                    error: MacOsSystemAudioStopError::CallbackQuiescenceUnconfirmed(teardown_error),
                };
                Err(MacOsSystemAudioError::CaptureStartTeardownUnconfirmed)
            }
        }
    }

    fn retain_unconfirmed_stop(&mut self, error: MacOsSystemAudioStopError) {
        let capture = mem::replace(&mut self.capture, NativeCaptureLifecycle::Ready);
        self.capture = capture.retain_unconfirmed(error);
    }

    pub fn poll_chunk(&mut self) -> Result<Option<MacOsSystemAudioChunk>, MacOsSystemAudioError> {
        match &self.capture {
            NativeCaptureLifecycle::Ready => return Err(MacOsSystemAudioError::NotRunning),
            NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                return Err(MacOsSystemAudioError::CaptureTeardownUnconfirmed);
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
            self.retain_unconfirmed_stop(MacOsSystemAudioStopError::NativeStopUnconfirmed(
                MacOsSystemAudioError::UnexpectedStreamStop,
            ));
            return Err(MacOsSystemAudioError::UnexpectedStreamStop);
        }
        let received = match &self.capture {
            NativeCaptureLifecycle::Running(active) => active.receiver.try_recv(),
            NativeCaptureLifecycle::Ready
            | NativeCaptureLifecycle::StopUnconfirmed { .. }
            | NativeCaptureLifecycle::NativeOperationUnconfirmed { .. } => {
                return Err(MacOsSystemAudioError::CaptureTeardownUnconfirmed);
            }
        };
        match received {
            Ok(chunk) => Ok(Some(chunk)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.retain_unconfirmed_stop(MacOsSystemAudioStopError::NativeStopUnconfirmed(
                    MacOsSystemAudioError::CallbackQueueDisconnected,
                ));
                Err(MacOsSystemAudioError::CallbackQueueDisconnected)
            }
        }
    }

    /// Stop and return the complete bounded callback tail. Recording is not
    /// integrated in this slice; callers must not imply that these chunks were
    /// muxed into an artifact.
    pub fn stop_and_drain_chunks(
        &mut self,
    ) -> Result<Vec<MacOsSystemAudioChunk>, MacOsSystemAudioStopError> {
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
            let error = MacOsSystemAudioStopError::NativeStopUnconfirmed(
                MacOsSystemAudioError::UnexpectedStreamStop,
            );
            self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
            return Err(error);
        }
        match run_stream_native_call(&mut active, |stream| stream.stop_capture().is_ok()) {
            StreamNativeCall::Completed(true) => {}
            StreamNativeCall::Completed(false) => {
                let error = MacOsSystemAudioStopError::NativeStopUnconfirmed(
                    MacOsSystemAudioError::CaptureStopFailed,
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
            StreamNativeCall::NotStarted(launch_error) => {
                let error = MacOsSystemAudioStopError::NativeStopUnconfirmed(
                    map_native_call_launch_error(launch_error),
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
            StreamNativeCall::Unconfirmed => {
                let error = MacOsSystemAudioStopError::NativeStopUnconfirmed(
                    MacOsSystemAudioError::NativeOperationTimedOut,
                );
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
        }
        let detached = match detach_capture_bridge(&mut active) {
            Ok(detached) => detached,
            Err(error) => {
                let error = MacOsSystemAudioStopError::CallbackQuiescenceUnconfirmed(error);
                self.capture = NativeCaptureLifecycle::StopUnconfirmed { active, error };
                return Err(error);
            }
        };
        let post_teardown_error =
            if observe_unexpected_stop(&active.unexpected_stop, &self.diagnostics) {
                Some(MacOsSystemAudioError::UnexpectedStreamStop)
            } else if !detached.output_handler_registered {
                Some(MacOsSystemAudioError::OutputHandlerReleaseUnconfirmed)
            } else {
                None
            };
        self.capture = NativeCaptureLifecycle::Ready;
        if let Some(error) = post_teardown_error {
            return Err(MacOsSystemAudioStopError::CaptureFailedAfterTeardown(error));
        }
        Ok(detached.chunks)
    }

    /// Compatibility stop for callers intentionally discarding the bounded
    /// callback tail.
    pub fn stop(&mut self) -> Result<(), MacOsSystemAudioError> {
        self.stop_and_drain_chunks()
            .map(drop)
            .map_err(MacOsSystemAudioStopError::capture_error)
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
    pub fn diagnostics(&self) -> MacOsSystemAudioDiagnostics {
        self.diagnostics.snapshot()
    }
}

impl Drop for MacOsSystemAudioSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

const fn permission_state(permission: PermissionPreflight) -> PermissionState {
    match permission {
        PermissionPreflight::Granted => PermissionState::Granted,
        PermissionPreflight::PromptRequired => PermissionState::PromptRequired,
        PermissionPreflight::Denied(_) => PermissionState::Denied,
        PermissionPreflight::Restricted => PermissionState::Restricted,
        PermissionPreflight::Revoked(_) => PermissionState::Revoked,
    }
}

fn observe_unexpected_stop(unexpected_stop: &AtomicBool, diagnostics: &DiagnosticCounters) -> bool {
    let unexpected_stop = unexpected_stop.swap(false, Ordering::AcqRel);
    if unexpected_stop {
        increment(&diagnostics.unexpected_native_stops);
    }
    unexpected_stop
}

fn process_callback_sample(
    sample: &CMSampleBuffer,
    sender: &SyncSender<MacOsSystemAudioChunk>,
    state: &CallbackState,
    diagnostics: &DiagnosticCounters,
) {
    let extracted = match extract_native_audio(sample) {
        Ok(extracted) => extracted,
        Err(_) => {
            increment(&diagnostics.invalid_callback_chunks);
            state.force_discontinuity.store(true, Ordering::Release);
            return;
        }
    };
    let sequence =
        match state
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            }) {
            Ok(previous) => previous.saturating_add(1),
            Err(_) => {
                increment(&diagnostics.invalid_callback_chunks);
                state.force_discontinuity.store(true, Ordering::Release);
                return;
            }
        };
    let Some(end_ns) = extracted.source_pts_ns.checked_add(extracted.duration_ns) else {
        increment(&diagnostics.invalid_callback_chunks);
        state.force_discontinuity.store(true, Ordering::Release);
        return;
    };
    let previous_end = state.last_end_ns.swap(end_ns, Ordering::AcqRel);
    let timing_discontinuity = previous_end != 0
        && (extracted.source_pts_ns < previous_end
            || extracted.source_pts_ns.saturating_sub(previous_end)
                > TIMESTAMP_GAP_DISCONTINUITY_NS);
    let discontinuity =
        state.force_discontinuity.swap(false, Ordering::AcqRel) || timing_discontinuity;
    let chunk = MacOsSystemAudioChunk {
        sequence,
        source_pts_ns: extracted.source_pts_ns,
        duration_ns: extracted.duration_ns,
        discontinuity,
        samples_f32le: extracted.samples_f32le,
    };
    if !deliver_callback_chunk(sender, chunk, diagnostics) {
        state.force_discontinuity.store(true, Ordering::Release);
    }
}

struct ExtractedNativeAudio {
    source_pts_ns: u64,
    duration_ns: u64,
    samples_f32le: Vec<u8>,
}

fn extract_native_audio(
    sample: &CMSampleBuffer,
) -> Result<ExtractedNativeAudio, MacOsSystemAudioError> {
    if !sample.is_valid() || !sample.data_is_ready() {
        return Err(MacOsSystemAudioError::InvalidSampleBuffer);
    }
    let frames = u32::try_from(sample.num_samples())
        .map_err(|_| MacOsSystemAudioError::AudioChunkTooLarge)?;
    if frames == 0 || frames > MAX_AUDIO_CHUNK_FRAMES {
        return Err(MacOsSystemAudioError::AudioChunkTooLarge);
    }
    let description = sample
        .format_description()
        .ok_or(MacOsSystemAudioError::UnexpectedAudioFormat)?;
    validate_audio_description(NativeAudioDescription {
        sample_rate_hz: description
            .audio_sample_rate()
            .ok_or(MacOsSystemAudioError::UnexpectedAudioFormat)?,
        channels: description
            .audio_channel_count()
            .ok_or(MacOsSystemAudioError::UnexpectedAudioFormat)?,
        bits_per_channel: description
            .audio_bits_per_channel()
            .ok_or(MacOsSystemAudioError::UnexpectedAudioFormat)?,
        pcm: description.is_pcm(),
        float: description.audio_is_float(),
        big_endian: description.audio_is_big_endian(),
    })?;
    let buffers = sample
        .audio_buffer_list()
        .ok_or(MacOsSystemAudioError::MissingAudioBuffer)?;
    let samples_f32le = match buffers.num_buffers() {
        1 => {
            let buffer = buffers
                .get(0)
                .ok_or(MacOsSystemAudioError::MissingAudioBuffer)?;
            extract_stereo_f32le(
                frames,
                &[AudioPlane {
                    channels: buffer.number_channels,
                    bytes: buffer.data(),
                }],
            )?
        }
        2 => {
            let left = buffers
                .get(0)
                .ok_or(MacOsSystemAudioError::MissingAudioBuffer)?;
            let right = buffers
                .get(1)
                .ok_or(MacOsSystemAudioError::MissingAudioBuffer)?;
            extract_stereo_f32le(
                frames,
                &[
                    AudioPlane {
                        channels: left.number_channels,
                        bytes: left.data(),
                    },
                    AudioPlane {
                        channels: right.number_channels,
                        bytes: right.data(),
                    },
                ],
            )?
        }
        _ => return Err(MacOsSystemAudioError::InvalidAudioBufferLayout),
    };
    let duration_ns = u64::from(frames)
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_div(u64::from(SYSTEM_AUDIO_SAMPLE_RATE_HZ)))
        .filter(|duration| *duration > 0)
        .ok_or(MacOsSystemAudioError::InvalidTimestamp)?;
    Ok(ExtractedNativeAudio {
        source_pts_ns: media_time_ns(sample.presentation_timestamp())?,
        duration_ns,
        samples_f32le,
    })
}

fn media_time_ns(time: CMTime) -> Result<u64, MacOsSystemAudioError> {
    if !time.is_valid()
        || time.is_indefinite()
        || time.is_positive_infinity()
        || time.is_negative_infinity()
        || time.value < 0
        || time.timescale <= 0
        || time.epoch != 0
    {
        return Err(MacOsSystemAudioError::InvalidTimestamp);
    }
    u128::try_from(time.value)
        .ok()
        .and_then(|value| value.checked_mul(1_000_000_000))
        .and_then(|value| value.checked_div(u128::try_from(time.timescale).ok()?))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(MacOsSystemAudioError::InvalidTimestamp)
}

fn detach_capture_bridge(
    active: &mut ActiveCapture,
) -> Result<DetachedCaptureTail, MacOsSystemAudioError> {
    // Keep the Rust handler installed while releasing SCStream. The delegate
    // drop proves every context/callback owner is gone; a second fence and a
    // disconnected bounded channel then independently prove a complete tail.
    fence_callback_queue(&active.callback_queue, CALLBACK_QUEUE_FENCE_TIMEOUT)?;
    let output_handler_registered = active.output_handler_id.take().is_some();
    let Some(stream) = active.stream.take() else {
        return Err(MacOsSystemAudioError::DelegateQuiescenceUnconfirmed);
    };
    drop(stream);
    await_delegate_quiescence(&active.delegate_dropped, DELEGATE_QUIESCENCE_TIMEOUT)?;
    fence_callback_queue(&active.callback_queue, CALLBACK_QUEUE_FENCE_TIMEOUT)?;
    let chunks = drain_quiescent_callback_tail(&active.receiver)?;
    Ok(DetachedCaptureTail {
        chunks,
        output_handler_registered,
    })
}

fn fence_callback_queue(
    queue: &DispatchQueue,
    timeout: Duration,
) -> Result<(), MacOsSystemAudioError> {
    let (completion, completed) = sync_channel(1);
    dispatch_async(queue, move || {
        // A timed-out caller drops `completed`. This closure owns no stream,
        // delegate, receiver, or borrowed queue state, and `try_send` lets a
        // late fence retire without blocking after teardown has failed closed.
        let _ = completion.try_send(());
    });
    match completed.recv_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            Err(MacOsSystemAudioError::CallbackQueueFenceTimedOut)
        }
    }
}

fn await_delegate_quiescence(
    delegate_dropped: &Receiver<()>,
    timeout: Duration,
) -> Result<(), MacOsSystemAudioError> {
    match delegate_dropped.recv_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            Err(MacOsSystemAudioError::DelegateQuiescenceUnconfirmed)
        }
    }
}

fn drain_quiescent_callback_tail<T>(
    receiver: &Receiver<T>,
) -> Result<Vec<T>, MacOsSystemAudioError> {
    let mut tail = Vec::with_capacity(AUDIO_CALLBACK_QUEUE_CAPACITY);
    for _ in 0..AUDIO_CALLBACK_QUEUE_CAPACITY {
        match receiver.try_recv() {
            Ok(chunk) => tail.push(chunk),
            Err(TryRecvError::Disconnected) => return Ok(tail),
            Err(TryRecvError::Empty) => {
                return Err(MacOsSystemAudioError::OutputHandlerReleaseUnconfirmed);
            }
        }
    }
    match receiver.try_recv() {
        Err(TryRecvError::Disconnected) => Ok(tail),
        Ok(_) | Err(TryRecvError::Empty) => {
            Err(MacOsSystemAudioError::OutputHandlerReleaseUnconfirmed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_started_stop_is_idempotent_and_source_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<MacOsSystemAudioSource>();

        let mut source = MacOsSystemAudioSource::new([9; 32]).expect("source");
        assert_eq!(source.stop(), Ok(()));
        assert_eq!(source.stop(), Ok(()));
    }

    #[test]
    fn unconfirmed_stop_remains_sticky() {
        for error in [
            MacOsSystemAudioStopError::NativeStopUnconfirmed(
                MacOsSystemAudioError::CaptureStopFailed,
            ),
            MacOsSystemAudioStopError::CallbackQuiescenceUnconfirmed(
                MacOsSystemAudioError::CallbackQueueFenceTimedOut,
            ),
        ] {
            let mut lifecycle = NativeCaptureLifecycle::Running(7_u8).retain_unconfirmed(error);
            for _ in 0..2 {
                let (retained, observed) = lifecycle
                    .take_for_stop()
                    .expect_err("unconfirmed native or callback authority cannot be retried");
                assert_eq!(observed, error);
                lifecycle = retained;
            }
        }
    }

    #[test]
    fn callback_queue_is_nonblocking_bounded_and_observable() {
        let diagnostics = DiagnosticCounters::default();
        let (sender, receiver) = sync_channel(1);
        assert!(deliver_callback_chunk(&sender, 1_u8, &diagnostics));
        assert!(!deliver_callback_chunk(&sender, 2_u8, &diagnostics));
        assert_eq!(receiver.try_recv(), Ok(1));
        drop(receiver);
        assert!(!deliver_callback_chunk(&sender, 3_u8, &diagnostics));
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.dropped_callback_chunks, 1);
        assert_eq!(snapshot.callback_chunks_after_stop, 1);
    }

    #[test]
    fn callback_tail_requires_sender_release() {
        let (sender, receiver) = sync_channel::<u8>(AUDIO_CALLBACK_QUEUE_CAPACITY);
        assert_eq!(
            drain_quiescent_callback_tail(&receiver),
            Err(MacOsSystemAudioError::OutputHandlerReleaseUnconfirmed)
        );
        sender.send(1).expect("bounded test tail");
        drop(sender);
        assert_eq!(drain_quiescent_callback_tail(&receiver), Ok(vec![1]));
    }

    #[test]
    fn callback_queue_fence_completes_inside_its_deadline() {
        let queue = DispatchQueue::new(
            "xyz.eng-manager.frame.system-audio.fence-success-test",
            DispatchQoS::UserInteractive,
        );
        assert_eq!(fence_callback_queue(&queue, Duration::from_secs(5)), Ok(()));
    }

    #[test]
    fn callback_queue_fence_timeout_is_bounded_and_late_ack_is_inert() {
        let queue = DispatchQueue::new(
            "xyz.eng-manager.frame.system-audio.fence-timeout-test",
            DispatchQoS::UserInteractive,
        );
        let (entered, wait_until_entered) = sync_channel(1);
        let (release, wait_until_released) = sync_channel::<()>(1);
        dispatch_async(&queue, move || {
            entered.try_send(()).expect("report blocked queue entry");
            // Dropping `release` also unblocks this receive during unwinding.
            let _ = wait_until_released.recv();
        });
        wait_until_entered
            .recv_timeout(Duration::from_secs(5))
            .expect("serial queue blocker must start");

        assert_eq!(
            fence_callback_queue(&queue, Duration::from_millis(10)),
            Err(MacOsSystemAudioError::CallbackQueueFenceTimedOut)
        );

        // The timed-out fence is still queued. Releasing the blocker lets its
        // nonblocking send observe a gone receiver, after which a fresh fence
        // proves the serial queue remains usable and no late closure was joined.
        drop(release);
        assert_eq!(fence_callback_queue(&queue, Duration::from_secs(5)), Ok(()));
    }
}
