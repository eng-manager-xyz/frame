use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, TryRecvError, TrySendError, sync_channel},
    },
};

use core_graphics::{access::ScreenCaptureAccess, display::CGDisplay};
use frame_media::{
    DisplayGeometryTransform, DpiScale, LogicalRect, PermissionPreflight, PhysicalRect, Rotation,
    ScreenSourceInstanceId, ScreenTargetBinding, ScreenTargetDescriptor, ScreenTargetEpoch,
    ScreenTargetId, ScreenTargetKind, ScreenTargetSnapshot, SettingsGuidance,
};
use ring::hmac;
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferExt, CMSampleBufferSCExt, CMTime, SCFrameStatus},
    cv::CVPixelBufferLockFlags,
    prelude::{
        ErrorHandler, PixelFormat as NativePixelFormat, SCContentFilter, SCShareableContent,
        SCStream, SCStreamConfiguration, SCStreamOutputType,
    },
};
use zeroize::Zeroizing;

use crate::{
    CALLBACK_QUEUE_CAPACITY, MacOsCaptureConfig, MacOsCaptureDiagnostics, MacOsCaptureError,
    MacOsCaptureFrame, RawMediaTime, copy_bgra_rows,
};

mod frame_assembly;

use frame_assembly::FrameAssembler;

const TARGET_TOKEN_DOMAIN: &[u8] = b"frame/macos-display-token/v1\0";
const GEOMETRY_INTEGER_TOLERANCE: f64 = 0.000_001;
const SRGB_COLOR_SPACE_NAME: &str = "kCGColorSpaceSRGB";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeDisplayRecord {
    display_id: u32,
    transform: DisplayGeometryTransform,
}

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

struct ActiveCapture {
    stream: SCStream,
    receiver: Receiver<CMSampleBuffer>,
    unexpected_stop: Arc<AtomicBool>,
    frames: FrameAssembler,
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

/// Safe full-display ScreenCaptureKit source.
///
/// Construct one instance per recording session. `session_secret` must be 32
/// fresh CSPRNG bytes; it binds opaque display tokens to this session without
/// exposing raw `CGDirectDisplayID` values outside this module.
pub struct MacOsScreenCaptureSource {
    source_instance: ScreenSourceInstanceId,
    session_secret: Zeroizing<[u8; 32]>,
    topology_generation: u64,
    catalog_records: Option<Vec<NativeDisplayRecord>>,
    target_map: BTreeMap<ScreenTargetBinding, u32>,
    permission_requested: bool,
    permission_was_granted: bool,
    diagnostics: Arc<DiagnosticCounters>,
    active: Option<ActiveCapture>,
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
            active: None,
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
        if self.active.is_some() {
            return Err(MacOsCaptureError::AlreadyRunning);
        }
        let records = active_display_records()?;
        if self.catalog_records.as_ref() != Some(&records) {
            self.topology_generation = self
                .topology_generation
                .checked_add(1)
                .ok_or(MacOsCaptureError::TopologyGenerationExhausted)?;
        }

        let generation = self.topology_generation;
        let epoch = ScreenTargetEpoch::new(generation)
            .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
        let mut target_map = BTreeMap::new();
        let mut targets = Vec::with_capacity(records.len());
        for record in &records {
            let target_id = derive_target_id(&self.session_secret, record.display_id)?;
            let binding =
                ScreenTargetBinding::new(self.source_instance, generation, epoch, target_id)
                    .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
            if target_map.insert(binding, record.display_id).is_some() {
                return Err(MacOsCaptureError::TargetTokenCollision);
            }
            targets.push(
                ScreenTargetDescriptor::display(binding, record.transform)
                    .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?,
            );
        }
        let snapshot = ScreenTargetSnapshot::new(self.source_instance, generation, targets)
            .map_err(|_| MacOsCaptureError::MediaCatalogRejected)?;
        self.catalog_records = Some(records);
        self.target_map = target_map;
        Ok(snapshot)
    }

    pub fn start(&mut self, config: MacOsCaptureConfig) -> Result<(), MacOsCaptureError> {
        if self.active.is_some() {
            return Err(MacOsCaptureError::AlreadyRunning);
        }
        if config.target().source_instance() != self.source_instance {
            return Err(MacOsCaptureError::StaleOrForeignTarget);
        }
        let display_id = self
            .target_map
            .get(&config.target())
            .copied()
            .ok_or(MacOsCaptureError::StaleOrForeignTarget)?;
        if !ScreenCaptureAccess.preflight() {
            return Err(MacOsCaptureError::PermissionDenied);
        }
        self.permission_was_granted = true;

        let content = SCShareableContent::get()
            .map_err(|_| MacOsCaptureError::ShareableContentUnavailable)?;
        let display = content
            .displays()
            .into_iter()
            .find(|display| display.display_id() == display_id)
            .ok_or(MacOsCaptureError::TargetNoLongerAvailable)?;
        let current_pid = i32::try_from(std::process::id())
            .map_err(|_| MacOsCaptureError::CurrentProcessIdOutOfRange)?;
        let current_application =
            select_current_application(content.applications(), current_pid, |application| {
                application.process_id()
            })?;
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_applications(&[&current_application], &[])
            .build();
        let output = config.output();
        let interval_value = i64::try_from(output.nominal_frame_duration_ns)
            .map_err(|_| MacOsCaptureError::InvalidFrameDuration)?;
        let interval = CMTime::new(interval_value, 1_000_000_000);
        let configuration = SCStreamConfiguration::new()
            .with_width(output.width)
            .with_height(output.height)
            .with_pixel_format(NativePixelFormat::BGRA)
            .with_color_space_name(SRGB_COLOR_SPACE_NAME)
            .with_shows_cursor(matches!(
                config.cursor(),
                frame_media::CursorCaptureMode::EmbeddedInFrame
            ))
            .with_minimum_frame_interval(&interval)
            .with_queue_depth(
                u32::try_from(CALLBACK_QUEUE_CAPACITY)
                    .map_err(|_| MacOsCaptureError::CaptureStartFailed)?,
            );

        let (sender, receiver) = sync_channel(CALLBACK_QUEUE_CAPACITY);
        let unexpected_stop = Arc::new(AtomicBool::new(false));
        let delegate_stop = Arc::clone(&unexpected_stop);
        let delegate_diagnostics = Arc::clone(&self.diagnostics);
        let delegate = ErrorHandler::new(move |_error| {
            delegate_stop.store(true, Ordering::Release);
            increment(&delegate_diagnostics.unexpected_native_stops);
        });
        let mut stream = SCStream::new_with_delegate(&filter, &configuration, delegate);
        let callback_diagnostics = Arc::clone(&self.diagnostics);
        if stream
            .add_output_handler(
                move |sample, output_type| {
                    if output_type != SCStreamOutputType::Screen {
                        increment(&callback_diagnostics.invalid_samples);
                        return;
                    }
                    match sender.try_send(sample) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            increment(&callback_diagnostics.dropped_callback_frames);
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            increment(&callback_diagnostics.callback_frames_after_stop);
                        }
                    }
                },
                SCStreamOutputType::Screen,
            )
            .is_none()
        {
            return Err(MacOsCaptureError::OutputHandlerRegistrationFailed);
        }
        stream
            .start_capture()
            .map_err(|_| MacOsCaptureError::CaptureStartFailed)?;
        self.active = Some(ActiveCapture {
            stream,
            receiver,
            unexpected_stop,
            frames: FrameAssembler::new(config.target(), output),
        });
        Ok(())
    }

    /// Drain at most the three callback-queued samples and return one frame.
    /// Pixel locking and row copies happen here, never in the native callback.
    pub fn poll_frame(&mut self) -> Result<Option<MacOsCaptureFrame>, MacOsCaptureError> {
        if self.active.is_none() {
            return Err(MacOsCaptureError::NotRunning);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.unexpected_stop.load(Ordering::Acquire))
        {
            self.active.take();
            return Err(MacOsCaptureError::UnexpectedStreamStop);
        }

        for _ in 0..CALLBACK_QUEUE_CAPACITY {
            let sample = match self
                .active
                .as_ref()
                .ok_or(MacOsCaptureError::NotRunning)?
                .receiver
                .try_recv()
            {
                Ok(sample) => sample,
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => {
                    self.active.take();
                    return Err(MacOsCaptureError::CallbackQueueDisconnected);
                }
            };
            let processed = process_sample(
                self.active.as_mut().ok_or(MacOsCaptureError::NotRunning)?,
                &sample,
                &self.diagnostics,
            );
            match processed {
                Ok(ProcessedSample::Frame(frame)) => return Ok(Some(frame)),
                Ok(ProcessedSample::Ignored) => continue,
                Ok(ProcessedSample::Terminal) => {
                    self.active.take();
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
    pub fn stop_and_drain_frames(&mut self) -> Result<Vec<MacOsCaptureFrame>, MacOsCaptureError> {
        let Some(mut active) = self.active.take() else {
            return Ok(Vec::new());
        };
        if active.unexpected_stop.load(Ordering::Acquire) {
            return Err(MacOsCaptureError::UnexpectedStreamStop);
        }
        active
            .stream
            .stop_capture()
            .map_err(|_| MacOsCaptureError::CaptureStopFailed)?;

        let mut tail = Vec::with_capacity(CALLBACK_QUEUE_CAPACITY);
        for _ in 0..CALLBACK_QUEUE_CAPACITY {
            let sample = match active.receiver.try_recv() {
                Ok(sample) => sample,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            match process_sample(&mut active, &sample, &self.diagnostics) {
                Ok(ProcessedSample::Frame(frame)) => tail.push(frame),
                Ok(ProcessedSample::Ignored) => {}
                Ok(ProcessedSample::Terminal) => break,
                Err(error) => {
                    increment(&self.diagnostics.invalid_samples);
                    return Err(error);
                }
            }
        }
        Ok(tail)
    }

    /// Stop capture and deliberately discard its bounded callback tail.
    ///
    /// Recording finalizers should use [`Self::stop_and_drain_frames`] instead.
    pub fn stop(&mut self) -> Result<(), MacOsCaptureError> {
        self.stop_and_drain_frames().map(drop)
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.active.is_some()
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

fn derive_target_id(
    session_secret: &[u8; 32],
    display_id: u32,
) -> Result<ScreenTargetId, MacOsCaptureError> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, session_secret);
    let mut context = hmac::Context::with_key(&key);
    context.update(TARGET_TOKEN_DOMAIN);
    context.update(&display_id.to_be_bytes());
    let tag = context.sign();
    let mut opaque = [0_u8; 16];
    opaque.copy_from_slice(&tag.as_ref()[..16]);
    ScreenTargetId::new(ScreenTargetKind::Display, opaque)
        .map_err(|_| MacOsCaptureError::TargetTokenCollision)
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
            Ok(NativeDisplayRecord {
                display_id,
                transform,
            })
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

    #[test]
    fn target_tokens_are_stable_within_one_session_and_change_across_sessions() {
        let first = derive_target_id(&[1; 32], 42).expect("first");
        assert_eq!(first, derive_target_id(&[1; 32], 42).expect("repeat"));
        assert_ne!(first, derive_target_id(&[2; 32], 42).expect("new session"));
        assert_ne!(first, derive_target_id(&[1; 32], 43).expect("new display"));
    }

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
    fn stop_is_idempotent_before_start() {
        let source_id = ScreenSourceInstanceId::new([7; 16]).expect("source");
        let mut source = MacOsScreenCaptureSource::new(source_id, [8; 32]).expect("adapter");
        assert_eq!(source.stop(), Ok(()));
        assert_eq!(source.stop(), Ok(()));
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
