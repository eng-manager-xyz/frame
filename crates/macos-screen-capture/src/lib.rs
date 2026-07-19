//! Safe macOS ScreenCaptureKit adapter primitives.
//!
//! The production implementation is only exported on macOS. Portable frame
//! validation and timing normalization live here so their invariants remain
//! testable on every workspace host.

#![forbid(unsafe_code)]

use std::fmt;

use frame_media::{
    CaptureError, ColorSpace, CursorCaptureMode, FrameMemory, FrameTimestamp, PixelFormat,
    ScreenTargetBinding, VideoFrameSpec,
};
use thiserror::Error;

mod target_catalog;

pub use target_catalog::MacOsRegionSelection;

#[cfg(target_os = "macos")]
mod platform;

#[cfg(target_os = "macos")]
pub use platform::MacOsScreenCaptureSource;

/// ScreenCaptureKit's callback queue and native stream queue depth.
pub const CALLBACK_QUEUE_CAPACITY: usize = 3;
/// Initial production ceiling. This is a resource bound, not a device promise.
pub const MAX_CAPTURE_WIDTH: u32 = 7_680;
/// Initial production ceiling. This is a resource bound, not a device promise.
pub const MAX_CAPTURE_HEIGHT: u32 = 4_320;
/// Maximum owned BGRA allocation for one frame (256 MiB).
pub const MAX_OWNED_FRAME_BYTES: usize = 256 * 1024 * 1024;
const MIN_FRAME_DURATION_NS: u64 = 1_000_000;
const MAX_FRAME_DURATION_NS: u64 = 1_000_000_000;
#[cfg(any(target_os = "macos", test))]
const TIMESTAMP_GAP_DISCONTINUITY_NS: u64 = 2_000_000_000;

/// Why this crate does not implement [`frame_media::ScreenCaptureSource`].
///
/// The provider-free contract requires an exact protected-content event.
/// ScreenCaptureKit 8 exposes `Blank` and `Suspended`, but neither status is an
/// exact DRM/protected-content signal. Treating either as protected content
/// would lie to the contract and can misclassify ordinary display state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameMediaContractStatus {
    BlockedByMissingProtectedContentSignal,
}

pub const FRAME_MEDIA_CONTRACT_STATUS: FrameMediaContractStatus =
    FrameMediaContractStatus::BlockedByMissingProtectedContentSignal;

/// Configuration for one catalog-bound display, window, or region BGRA capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacOsCaptureConfig {
    target: ScreenTargetBinding,
    output: VideoFrameSpec,
    cursor: CursorCaptureMode,
}

impl MacOsCaptureConfig {
    pub fn new(
        target: ScreenTargetBinding,
        output: VideoFrameSpec,
        cursor: CursorCaptureMode,
    ) -> Result<Self, MacOsCaptureError> {
        output
            .validate()
            .map_err(MacOsCaptureError::InvalidFrameSpec)?;
        if output.width > MAX_CAPTURE_WIDTH || output.height > MAX_CAPTURE_HEIGHT {
            return Err(MacOsCaptureError::FrameDimensionsExceedLimit);
        }
        let retained_bytes = bgra_frame_bytes(output.width, output.height)?;
        if retained_bytes > MAX_OWNED_FRAME_BYTES {
            return Err(MacOsCaptureError::FrameAllocationExceedsLimit);
        }
        if output.pixel_format != PixelFormat::Bgra8
            || output.color_space != ColorSpace::Srgb
            || output.memory != FrameMemory::Cpu
        {
            return Err(MacOsCaptureError::UnsupportedOutputProfile);
        }
        if !(MIN_FRAME_DURATION_NS..=MAX_FRAME_DURATION_NS)
            .contains(&output.nominal_frame_duration_ns)
        {
            return Err(MacOsCaptureError::InvalidFrameDuration);
        }
        if !matches!(
            cursor,
            CursorCaptureMode::Hidden | CursorCaptureMode::EmbeddedInFrame
        ) {
            return Err(MacOsCaptureError::UnsupportedCursorMode);
        }
        Ok(Self {
            target,
            output,
            cursor,
        })
    }

    #[must_use]
    pub const fn target(self) -> ScreenTargetBinding {
        self.target
    }

    #[must_use]
    pub const fn output(self) -> VideoFrameSpec {
        self.output
    }

    #[must_use]
    pub const fn cursor(self) -> CursorCaptureMode {
        self.cursor
    }
}

/// One tightly packed, owned BGRA frame.
pub struct MacOsCaptureFrame {
    target: ScreenTargetBinding,
    sequence: u64,
    source_pts_ns: Option<u64>,
    timestamp: FrameTimestamp,
    spec: VideoFrameSpec,
    pixels: Vec<u8>,
}

impl MacOsCaptureFrame {
    #[must_use]
    pub const fn target(&self) -> ScreenTargetBinding {
        self.target
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Raw ScreenCaptureKit presentation time in the shared epoch-zero media
    /// clock. `None` means this sample cannot be compared with another native
    /// source and therefore must not enter a shared-clock A/V graph.
    #[must_use]
    pub const fn source_pts_ns(&self) -> Option<u64> {
        self.source_pts_ns
    }

    #[must_use]
    pub const fn timestamp(&self) -> FrameTimestamp {
        self.timestamp
    }

    #[must_use]
    pub const fn spec(&self) -> VideoFrameSpec {
        self.spec
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

impl fmt::Debug for MacOsCaptureFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacOsCaptureFrame")
            .field("target", &self.target)
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field("spec", &self.spec)
            .field("retained_bytes", &self.pixels.len())
            .finish()
    }
}

/// Monotonic, low-cardinality counters for one adapter instance.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MacOsCaptureDiagnostics {
    pub dropped_callback_frames: u64,
    pub callback_frames_after_stop: u64,
    pub ignored_non_content_samples: u64,
    pub invalid_samples: u64,
    pub duration_fallbacks: u64,
    pub timestamp_discontinuities: u64,
    pub unexpected_native_stops: u64,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MacOsCaptureError {
    #[error("the capture session secret must contain entropy")]
    InvalidSessionSecret,
    #[error("the media frame specification is invalid")]
    InvalidFrameSpec(#[source] CaptureError),
    #[error("capture dimensions exceed the adapter limit")]
    FrameDimensionsExceedLimit,
    #[error("the owned BGRA frame allocation exceeds the adapter limit")]
    FrameAllocationExceedsLimit,
    #[error("only CPU-backed BGRA/sRGB output is supported")]
    UnsupportedOutputProfile,
    #[error("the nominal frame duration is outside 1 ms..=1 s")]
    InvalidFrameDuration,
    #[error("only hidden or frame-embedded cursor capture is supported")]
    UnsupportedCursorMode,
    #[error("screen-recording permission is not granted")]
    PermissionDenied,
    #[error("the active display catalog could not be read")]
    DisplayCatalogUnavailable,
    #[error("display geometry from Core Graphics is invalid")]
    InvalidDisplayGeometry,
    #[error("the display token derivation collided")]
    TargetTokenCollision,
    #[error("the display topology generation exhausted its range")]
    TopologyGenerationExhausted,
    #[error("the display target is stale or did not come from this adapter")]
    StaleOrForeignTarget,
    #[error("a region selection must reference an opaque display target")]
    RegionRequiresDisplayTarget,
    #[error("the region's display token is stale or came from another adapter session")]
    StaleOrForeignRegionDisplay,
    #[error("the selected region is not wholly contained by its display")]
    InvalidRegionGeometry,
    #[error("the requested output aspect ratio does not match the selected target")]
    OutputAspectRatioDoesNotMatchTarget,
    #[error("the native target catalog contains a duplicate identity")]
    DuplicateNativeTarget,
    #[error("the native target catalog exceeds the normalized 256-target bound")]
    TargetCatalogLimitExceeded,
    #[error("the selected target changed after it was enumerated")]
    StaleTargetTopology,
    #[error("a capture stream is already running")]
    AlreadyRunning,
    #[error("no capture stream is running")]
    NotRunning,
    #[error("ScreenCaptureKit shareable content is unavailable")]
    ShareableContentUnavailable,
    #[error("another bounded ScreenCaptureKit operation still owns native-call capacity")]
    NativeOperationCapacityUnavailable,
    #[error("the bounded ScreenCaptureKit operation worker could not be started")]
    NativeOperationWorkerUnavailable,
    #[error("the ScreenCaptureKit operation did not complete before the native-call deadline")]
    NativeOperationTimedOut,
    #[error("the selected display is no longer shareable")]
    TargetNoLongerAvailable,
    #[error("the current process identifier cannot be represented by ScreenCaptureKit")]
    CurrentProcessIdOutOfRange,
    #[error("ScreenCaptureKit did not expose the current running application")]
    CurrentApplicationUnavailable,
    #[error("ScreenCaptureKit exposed more than one running application for the current process")]
    AmbiguousCurrentApplication,
    #[error("ScreenCaptureKit rejected the output handler")]
    OutputHandlerRegistrationFailed,
    #[error("ScreenCaptureKit could not start capture")]
    CaptureStartFailed,
    #[error("capture start failed and ScreenCaptureKit delegate teardown was not confirmed")]
    CaptureStartTeardownUnconfirmed,
    #[error("ScreenCaptureKit reported an unexpected stream stop")]
    UnexpectedStreamStop,
    #[error("ScreenCaptureKit could not stop capture")]
    CaptureStopFailed,
    #[error("capture teardown remains unconfirmed and the source cannot be reused")]
    CaptureTeardownUnconfirmed,
    #[error("ScreenCaptureKit output-handler release could not be confirmed")]
    OutputHandlerRemovalFailed,
    #[error("the ScreenCaptureKit delegate did not quiesce before the teardown deadline")]
    DelegateQuiescenceUnconfirmed,
    #[error("the native callback queue disconnected")]
    CallbackQueueDisconnected,
    #[error("the sample does not contain an image buffer")]
    MissingImageBuffer,
    #[error("the sample does not contain a ScreenCaptureKit frame status")]
    MissingFrameStatus,
    #[error("the ScreenCaptureKit sample is invalid or its Complete-frame data is not ready")]
    InvalidSampleBuffer,
    #[error("the sample is not BGRA")]
    UnexpectedPixelFormat,
    #[error(
        "the sample dimensions {actual_width}x{actual_height} differ from the negotiated output {expected_width}x{expected_height}"
    )]
    UnexpectedFrameDimensions {
        expected_width: usize,
        expected_height: usize,
        actual_width: usize,
        actual_height: usize,
    },
    #[error("the Core Video pixel buffer could not be locked")]
    PixelBufferLockFailed,
    #[error("the BGRA row stride is smaller than one visible row")]
    InvalidRowStride,
    #[error("the BGRA pixel buffer is shorter than its declared layout")]
    PixelBufferTooShort,
    #[error("the sample timestamp is invalid or overflows nanoseconds")]
    InvalidTimestamp,
    #[error("the frame sequence exhausted its representable range")]
    SequenceExhausted,
    #[error("the frame-media catalog contract rejected native display data")]
    MediaCatalogRejected,
}

/// Failure while stopping one ScreenCaptureKit stream and draining its bounded
/// callback tail.
///
/// [`Self::CallbackQuiescenceUnconfirmed`] proves that ScreenCaptureKit
/// accepted the native stop but not that its output handler is quiescent. Only
/// [`Self::CaptureFailedAfterTeardown`] and [`Self::TailProcessingFailed`]
/// prove complete source teardown; callers may then release session authority
/// after also confirming downstream teardown, even though the capture failure
/// makes the recording artifact unusable.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MacOsCaptureStopError {
    #[error("ScreenCaptureKit native stop was not confirmed: {0}")]
    NativeStopUnconfirmed(#[source] MacOsCaptureError),
    #[error("ScreenCaptureKit stopped but callback quiescence was not confirmed: {0}")]
    CallbackQuiescenceUnconfirmed(#[source] MacOsCaptureError),
    #[error("ScreenCaptureKit capture failed before its teardown completed: {0}")]
    CaptureFailedAfterTeardown(#[source] MacOsCaptureError),
    #[error("ScreenCaptureKit stopped but callback-tail processing failed: {0}")]
    TailProcessingFailed(#[source] MacOsCaptureError),
}

impl MacOsCaptureStopError {
    /// Whether `SCStream::stop_capture()` returned successfully before this
    /// failure occurred.
    #[must_use]
    pub const fn native_stop_confirmed(&self) -> bool {
        match self {
            Self::NativeStopUnconfirmed(_) => false,
            Self::CallbackQuiescenceUnconfirmed(_)
            | Self::CaptureFailedAfterTeardown(_)
            | Self::TailProcessingFailed(_) => true,
        }
    }

    /// Whether native stop and the final stream-context/delegate drop proof
    /// completed before this failure occurred.
    #[must_use]
    pub const fn capture_teardown_confirmed(&self) -> bool {
        match self {
            Self::NativeStopUnconfirmed(_) | Self::CallbackQuiescenceUnconfirmed(_) => false,
            Self::CaptureFailedAfterTeardown(_) | Self::TailProcessingFailed(_) => true,
        }
    }

    #[must_use]
    pub const fn capture_error(&self) -> &MacOsCaptureError {
        match self {
            Self::NativeStopUnconfirmed(error)
            | Self::CallbackQuiescenceUnconfirmed(error)
            | Self::CaptureFailedAfterTeardown(error)
            | Self::TailProcessingFailed(error) => error,
        }
    }

    #[must_use]
    pub fn into_capture_error(self) -> MacOsCaptureError {
        match self {
            Self::NativeStopUnconfirmed(error)
            | Self::CallbackQuiescenceUnconfirmed(error)
            | Self::CaptureFailedAfterTeardown(error)
            | Self::TailProcessingFailed(error) => error,
        }
    }
}

fn bgra_frame_bytes(width: u32, height: u32) -> Result<usize, MacOsCaptureError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .and_then(|row| {
            usize::try_from(height)
                .ok()
                .and_then(|height| row.checked_mul(height))
        })
        .ok_or(MacOsCaptureError::FrameAllocationExceedsLimit)
}

#[cfg(any(target_os = "macos", test))]
fn copy_bgra_rows(
    bytes: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Result<Vec<u8>, MacOsCaptureError> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or(MacOsCaptureError::FrameAllocationExceedsLimit)?;
    if bytes_per_row < row_bytes {
        return Err(MacOsCaptureError::InvalidRowStride);
    }
    let output_len = row_bytes
        .checked_mul(height)
        .ok_or(MacOsCaptureError::FrameAllocationExceedsLimit)?;
    if output_len > MAX_OWNED_FRAME_BYTES {
        return Err(MacOsCaptureError::FrameAllocationExceedsLimit);
    }
    let required_len = if height == 0 {
        0
    } else {
        bytes_per_row
            .checked_mul(height - 1)
            .and_then(|prefix| prefix.checked_add(row_bytes))
            .ok_or(MacOsCaptureError::FrameAllocationExceedsLimit)?
    };
    if bytes.len() < required_len {
        return Err(MacOsCaptureError::PixelBufferTooShort);
    }

    let mut output = Vec::with_capacity(output_len);
    for row in 0..height {
        let start = row
            .checked_mul(bytes_per_row)
            .ok_or(MacOsCaptureError::FrameAllocationExceedsLimit)?;
        output.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    Ok(output)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy)]
struct RawMediaTime {
    value: i64,
    timescale: i32,
    epoch: i64,
    numeric: bool,
}

#[cfg(any(target_os = "macos", test))]
impl RawMediaTime {
    #[cfg(test)]
    const fn numeric(value: i64, timescale: i32, epoch: i64) -> Self {
        Self {
            value,
            timescale,
            epoch,
            numeric: true,
        }
    }

    #[cfg(test)]
    const fn invalid() -> Self {
        Self {
            value: 0,
            timescale: 0,
            epoch: 0,
            numeric: false,
        }
    }

    fn nanoseconds(self) -> Result<i128, MacOsCaptureError> {
        if !self.numeric || self.timescale <= 0 {
            return Err(MacOsCaptureError::InvalidTimestamp);
        }
        i128::from(self.value)
            .checked_mul(1_000_000_000)
            .map(|scaled| scaled / i128::from(self.timescale))
            .ok_or(MacOsCaptureError::InvalidTimestamp)
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy)]
struct NormalizedTimestamp {
    source_pts_ns: Option<u64>,
    timestamp: FrameTimestamp,
    used_nominal_duration: bool,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Default)]
struct TimestampNormalizer {
    segment_epoch: Option<i64>,
    segment_raw_origin_ns: i128,
    segment_output_origin_ns: u64,
    last_raw_pts_ns: Option<i128>,
    last_output_end_ns: u64,
}

#[cfg(any(target_os = "macos", test))]
impl TimestampNormalizer {
    #[cfg(target_os = "macos")]
    const fn new() -> Self {
        Self {
            segment_epoch: None,
            segment_raw_origin_ns: 0,
            segment_output_origin_ns: 0,
            last_raw_pts_ns: None,
            last_output_end_ns: 0,
        }
    }

    fn normalize(
        &mut self,
        pts: RawMediaTime,
        duration: RawMediaTime,
        nominal_duration_ns: u64,
        discontinuity_hint: bool,
    ) -> Result<NormalizedTimestamp, MacOsCaptureError> {
        if !(MIN_FRAME_DURATION_NS..=MAX_FRAME_DURATION_NS).contains(&nominal_duration_ns) {
            return Err(MacOsCaptureError::InvalidFrameDuration);
        }
        let raw_pts_ns = pts.nanoseconds()?;
        let raw_duration_ns = duration.nanoseconds().ok().and_then(|value| {
            u64::try_from(value)
                .ok()
                .filter(|value| (MIN_FRAME_DURATION_NS..=MAX_FRAME_DURATION_NS).contains(value))
        });
        let used_nominal_duration = raw_duration_ns.is_none();
        let duration_ns = raw_duration_ns.unwrap_or(nominal_duration_ns);

        self.normalize_duration(
            pts,
            raw_pts_ns,
            duration_ns,
            discontinuity_hint,
            used_nominal_duration,
        )
    }

    #[cfg(target_os = "macos")]
    fn normalize_nominal(
        &mut self,
        pts: RawMediaTime,
        nominal_duration_ns: u64,
        discontinuity_hint: bool,
    ) -> Result<NormalizedTimestamp, MacOsCaptureError> {
        if !(MIN_FRAME_DURATION_NS..=MAX_FRAME_DURATION_NS).contains(&nominal_duration_ns) {
            return Err(MacOsCaptureError::InvalidFrameDuration);
        }
        self.normalize_duration(
            pts,
            pts.nanoseconds()?,
            nominal_duration_ns,
            discontinuity_hint,
            false,
        )
    }

    fn normalize_duration(
        &mut self,
        pts: RawMediaTime,
        raw_pts_ns: i128,
        duration_ns: u64,
        discontinuity_hint: bool,
        used_nominal_duration: bool,
    ) -> Result<NormalizedTimestamp, MacOsCaptureError> {
        let mut discontinuity = discontinuity_hint;
        let output_pts_ns = match (self.segment_epoch, self.last_raw_pts_ns) {
            (None, None) => {
                self.segment_epoch = Some(pts.epoch);
                self.segment_raw_origin_ns = raw_pts_ns;
                self.segment_output_origin_ns = 0;
                0
            }
            (Some(segment_epoch), Some(last_raw_pts_ns))
                if segment_epoch != pts.epoch || raw_pts_ns <= last_raw_pts_ns =>
            {
                discontinuity = true;
                self.segment_epoch = Some(pts.epoch);
                self.segment_raw_origin_ns = raw_pts_ns;
                self.segment_output_origin_ns = self.last_output_end_ns;
                self.last_output_end_ns
            }
            (Some(_), Some(_)) => {
                let delta = raw_pts_ns
                    .checked_sub(self.segment_raw_origin_ns)
                    .and_then(|delta| u64::try_from(delta).ok())
                    .ok_or(MacOsCaptureError::InvalidTimestamp)?;
                let candidate = self
                    .segment_output_origin_ns
                    .checked_add(delta)
                    .ok_or(MacOsCaptureError::InvalidTimestamp)?;
                if candidate < self.last_output_end_ns {
                    discontinuity = true;
                    self.segment_raw_origin_ns = raw_pts_ns;
                    self.segment_output_origin_ns = self.last_output_end_ns;
                    self.last_output_end_ns
                } else {
                    if candidate.saturating_sub(self.last_output_end_ns)
                        > TIMESTAMP_GAP_DISCONTINUITY_NS
                    {
                        discontinuity = true;
                    }
                    candidate
                }
            }
            _ => return Err(MacOsCaptureError::InvalidTimestamp),
        };

        let mut timestamp = FrameTimestamp::new(output_pts_ns, duration_ns)
            .map_err(|_| MacOsCaptureError::InvalidTimestamp)?;
        timestamp.discontinuity = discontinuity;
        self.last_raw_pts_ns = Some(raw_pts_ns);
        self.last_output_end_ns = timestamp.end_ns();
        Ok(NormalizedTimestamp {
            source_pts_ns: (pts.epoch == 0)
                .then(|| u64::try_from(raw_pts_ns).ok())
                .flatten(),
            timestamp,
            used_nominal_duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame_media::{
        ScreenSourceInstanceId, ScreenTargetEpoch, ScreenTargetId, ScreenTargetKind,
    };

    fn target() -> ScreenTargetBinding {
        ScreenTargetBinding::new(
            ScreenSourceInstanceId::new([1; 16]).expect("source"),
            1,
            ScreenTargetEpoch::new(1).expect("epoch"),
            ScreenTargetId::new(ScreenTargetKind::Display, [2; 16]).expect("target"),
        )
        .expect("binding")
    }

    fn spec() -> VideoFrameSpec {
        VideoFrameSpec {
            width: 1_920,
            height: 1_080,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: 33_333_333,
            memory: FrameMemory::Cpu,
        }
    }

    #[test]
    fn configuration_rejects_profiles_the_native_path_does_not_implement() {
        let mut output = spec();
        output.pixel_format = PixelFormat::Nv12;
        assert_eq!(
            MacOsCaptureConfig::new(target(), output, CursorCaptureMode::Hidden),
            Err(MacOsCaptureError::UnsupportedOutputProfile)
        );
        assert_eq!(
            MacOsCaptureConfig::new(target(), spec(), CursorCaptureMode::Metadata),
            Err(MacOsCaptureError::UnsupportedCursorMode)
        );
    }

    #[test]
    fn configuration_accepts_exact_hidden_and_embedded_cursor_modes() {
        assert!(MacOsCaptureConfig::new(target(), spec(), CursorCaptureMode::Hidden).is_ok());
        assert!(
            MacOsCaptureConfig::new(target(), spec(), CursorCaptureMode::EmbeddedInFrame).is_ok()
        );
    }

    #[test]
    fn stop_failures_distinguish_native_callback_and_tail_authority() {
        let native =
            MacOsCaptureStopError::NativeStopUnconfirmed(MacOsCaptureError::CaptureStopFailed);
        let callbacks = MacOsCaptureStopError::CallbackQuiescenceUnconfirmed(
            MacOsCaptureError::OutputHandlerRemovalFailed,
        );
        let capture = MacOsCaptureStopError::CaptureFailedAfterTeardown(
            MacOsCaptureError::UnexpectedStreamStop,
        );
        let tail =
            MacOsCaptureStopError::TailProcessingFailed(MacOsCaptureError::InvalidSampleBuffer);

        assert!(!native.native_stop_confirmed());
        assert!(!native.capture_teardown_confirmed());
        assert_eq!(
            native.capture_error(),
            &MacOsCaptureError::CaptureStopFailed
        );
        assert!(callbacks.native_stop_confirmed());
        assert!(!callbacks.capture_teardown_confirmed());
        assert!(capture.native_stop_confirmed());
        assert!(capture.capture_teardown_confirmed());
        assert!(tail.native_stop_confirmed());
        assert!(tail.capture_teardown_confirmed());
        assert_eq!(
            tail.capture_error(),
            &MacOsCaptureError::InvalidSampleBuffer
        );
    }

    #[test]
    fn padded_bgra_rows_are_copied_without_padding() {
        let bytes = [
            1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16,
        ];
        assert_eq!(
            copy_bgra_rows(&bytes, 2, 2, 12).expect("copy"),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn row_copy_rejects_short_stride_and_truncated_storage() {
        assert_eq!(
            copy_bgra_rows(&[0; 16], 3, 1, 8),
            Err(MacOsCaptureError::InvalidRowStride)
        );
        assert_eq!(
            copy_bgra_rows(&[0; 15], 2, 2, 8),
            Err(MacOsCaptureError::PixelBufferTooShort)
        );
    }

    #[test]
    fn timestamps_rebase_and_use_valid_native_duration() {
        let mut normalizer = TimestampNormalizer::default();
        let first = normalizer
            .normalize(
                RawMediaTime::numeric(900, 30, 0),
                RawMediaTime::numeric(1, 30, 0),
                40_000_000,
                false,
            )
            .expect("first");
        let second = normalizer
            .normalize(
                RawMediaTime::numeric(901, 30, 0),
                RawMediaTime::numeric(1, 30, 0),
                40_000_000,
                false,
            )
            .expect("second");
        assert_eq!(first.timestamp.pts_ns, 0);
        assert_eq!(first.source_pts_ns, Some(30_000_000_000));
        assert_eq!(first.timestamp.duration_ns, 33_333_333);
        assert!(!first.used_nominal_duration);
        assert_eq!(second.timestamp.pts_ns, 33_333_333);
        assert!(!second.timestamp.discontinuity);
    }

    #[test]
    fn invalid_duration_falls_back_and_backward_pts_marks_discontinuity() {
        let mut normalizer = TimestampNormalizer::default();
        normalizer
            .normalize(
                RawMediaTime::numeric(100, 1_000, 0),
                RawMediaTime::invalid(),
                10_000_000,
                false,
            )
            .expect("first");
        let reset = normalizer
            .normalize(
                RawMediaTime::numeric(50, 1_000, 0),
                RawMediaTime::invalid(),
                10_000_000,
                false,
            )
            .expect("reset");
        assert_eq!(reset.timestamp.pts_ns, 10_000_000);
        assert!(reset.timestamp.discontinuity);
        assert!(reset.used_nominal_duration);
    }

    #[test]
    fn overlapping_native_timestamps_are_clamped_to_the_previous_end() {
        let mut normalizer = TimestampNormalizer::default();
        normalizer
            .normalize(
                RawMediaTime::numeric(0, 1_000, 0),
                RawMediaTime::numeric(40, 1_000, 0),
                40_000_000,
                false,
            )
            .expect("first");
        let overlap = normalizer
            .normalize(
                RawMediaTime::numeric(30, 1_000, 0),
                RawMediaTime::numeric(40, 1_000, 0),
                40_000_000,
                false,
            )
            .expect("overlap");
        assert_eq!(overlap.timestamp.pts_ns, 40_000_000);
        assert!(overlap.timestamp.discontinuity);
    }

    #[test]
    fn epoch_changes_and_large_gaps_are_discontinuities() {
        let mut normalizer = TimestampNormalizer::default();
        normalizer
            .normalize(
                RawMediaTime::numeric(0, 1, 0),
                RawMediaTime::numeric(1, 30, 0),
                33_333_333,
                false,
            )
            .expect("first");
        let gap = normalizer
            .normalize(
                RawMediaTime::numeric(3, 1, 0),
                RawMediaTime::numeric(1, 30, 0),
                33_333_333,
                false,
            )
            .expect("gap");
        assert_eq!(gap.timestamp.pts_ns, 3_000_000_000);
        assert!(gap.timestamp.discontinuity);
        let epoch = normalizer
            .normalize(
                RawMediaTime::numeric(1, 1, 1),
                RawMediaTime::numeric(1, 30, 1),
                33_333_333,
                false,
            )
            .expect("epoch reset");
        assert!(epoch.timestamp.discontinuity);
        assert_eq!(epoch.timestamp.pts_ns, gap.timestamp.end_ns());
        assert_eq!(epoch.source_pts_ns, None);
    }
}
