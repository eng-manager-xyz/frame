//! Safe Windows Graphics Capture adapter primitives.
//!
//! Native WinRT and Direct3D code is compiled only on Windows. Portable value,
//! geometry, allocation, and timestamp validation remains testable on every
//! workspace host.

#![forbid(unsafe_code)]

use std::fmt;

use frame_media::{
    CaptureError, ColorSpace, CursorCaptureMode, FrameMemory, FrameTimestamp, LogicalRect,
    PixelFormat, ScreenTargetBinding, ScreenTargetKind, VideoFrameSpec,
};
use thiserror::Error;

#[cfg(target_os = "windows")]
mod cursor;
#[cfg(target_os = "windows")]
mod normalized;
#[cfg(target_os = "windows")]
mod platform;
#[cfg(any(target_os = "windows", test))]
mod target_catalog;

#[cfg(target_os = "windows")]
pub use normalized::WindowsNormalizedScreenCaptureSource;
#[cfg(target_os = "windows")]
pub use platform::WindowsScreenCaptureSource;

/// Maximum number of owned frames retained between the WGC callback and poll.
pub const CALLBACK_QUEUE_CAPACITY: usize = 3;
/// Initial source ceiling. Hardware evidence may justify a later revision.
pub const MAX_CAPTURE_WIDTH: u32 = 7_680;
/// Initial source ceiling. Hardware evidence may justify a later revision.
pub const MAX_CAPTURE_HEIGHT: u32 = 4_320;
/// Maximum exact CPU allocation retained by one BGRA frame (256 MiB).
pub const MAX_OWNED_FRAME_BYTES: usize = 256 * 1024 * 1024;
const MIN_FRAME_DURATION_NS: u64 = 1_000_000;
const MAX_FRAME_DURATION_NS: u64 = 1_000_000_000;
#[cfg(any(target_os = "windows", test))]
const TIMESTAMP_GAP_DISCONTINUITY_NS: u64 = 2_000_000_000;

/// One display-relative logical region selected from an opaque catalog.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WindowsRegionSelection {
    display: ScreenTargetBinding,
    logical_bounds: LogicalRect,
}

impl WindowsRegionSelection {
    pub fn new(
        display: ScreenTargetBinding,
        logical_bounds: LogicalRect,
    ) -> Result<Self, WindowsCaptureError> {
        if display.id().kind() != ScreenTargetKind::Display {
            return Err(WindowsCaptureError::RegionRequiresDisplayTarget);
        }
        Ok(Self {
            display,
            logical_bounds,
        })
    }

    #[must_use]
    pub const fn display(self) -> ScreenTargetBinding {
        self.display
    }

    #[must_use]
    pub const fn logical_bounds(self) -> LogicalRect {
        self.logical_bounds
    }
}

impl fmt::Debug for WindowsRegionSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsRegionSelection")
            .field("display", &self.display)
            .field("logical_bounds", &"<redacted>")
            .finish()
    }
}

/// Exact configuration for one catalog-bound display, window, or region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsCaptureConfig {
    target: ScreenTargetBinding,
    output: VideoFrameSpec,
    cursor: CursorCaptureMode,
}

impl WindowsCaptureConfig {
    pub fn new(
        target: ScreenTargetBinding,
        output: VideoFrameSpec,
        cursor: CursorCaptureMode,
    ) -> Result<Self, WindowsCaptureError> {
        output
            .validate()
            .map_err(WindowsCaptureError::InvalidFrameSpec)?;
        if output.width > MAX_CAPTURE_WIDTH || output.height > MAX_CAPTURE_HEIGHT {
            return Err(WindowsCaptureError::FrameDimensionsExceedLimit);
        }
        if bgra_frame_bytes(output.width, output.height)? > MAX_OWNED_FRAME_BYTES {
            return Err(WindowsCaptureError::FrameAllocationExceedsLimit);
        }
        if output.pixel_format != PixelFormat::Bgra8
            || output.color_space != ColorSpace::Srgb
            || output.memory != FrameMemory::Cpu
        {
            return Err(WindowsCaptureError::UnsupportedOutputProfile);
        }
        if !(MIN_FRAME_DURATION_NS..=MAX_FRAME_DURATION_NS)
            .contains(&output.nominal_frame_duration_ns)
        {
            return Err(WindowsCaptureError::InvalidFrameDuration);
        }
        if !matches!(
            cursor,
            CursorCaptureMode::Hidden
                | CursorCaptureMode::EmbeddedInFrame
                | CursorCaptureMode::Metadata
        ) {
            return Err(WindowsCaptureError::UnsupportedCursorMode);
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

/// One tightly packed, exactly owned CPU BGRA frame.
pub struct WindowsCaptureFrame {
    target: ScreenTargetBinding,
    sequence: u64,
    source_pts_ns: Option<u64>,
    timestamp: FrameTimestamp,
    spec: VideoFrameSpec,
    pixels: Vec<u8>,
}

impl WindowsCaptureFrame {
    #[must_use]
    pub const fn target(&self) -> ScreenTargetBinding {
        self.target
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// WGC render time in Frame's process-wide monotonic capture clock. `None`
    /// means the safe wrapper could not represent the platform timestamp.
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

impl fmt::Debug for WindowsCaptureFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsCaptureFrame")
            .field("target", &self.target)
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field("spec", &self.spec)
            .field("retained_bytes", &self.pixels.len())
            .finish()
    }
}

/// Low-cardinality counters for one adapter instance.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WindowsCaptureDiagnostics {
    pub dropped_callback_frames: u64,
    pub callback_frames_after_stop: u64,
    pub invalid_native_frames: u64,
    pub timestamp_discontinuities: u64,
    pub target_closed_events: u64,
    pub unexpected_native_stops: u64,
    pub start_timeouts: u64,
    pub stop_timeouts: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WindowsCaptureError {
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
    #[error("the requested cursor capture mode is unsupported")]
    UnsupportedCursorMode,
    #[error("cursor metadata could not be sampled")]
    CursorMetadataUnavailable,
    #[error("the cursor image exceeds the 256-by-256 metadata bound")]
    CursorImageExceedsLimit,
    #[error("the cursor image revision exhausted its range")]
    CursorRevisionExhausted,
    #[error("Windows Graphics Capture is unavailable")]
    AdapterUnavailable,
    #[error("the display target is stale or did not come from this adapter")]
    StaleOrForeignTarget,
    #[error("a region selection must reference an opaque display target")]
    RegionRequiresDisplayTarget,
    #[error("the region's display token is stale or came from another adapter session")]
    StaleOrForeignRegionDisplay,
    #[error("the selected region is not wholly contained by its display")]
    InvalidRegionGeometry,
    #[error("the native target catalog contains a duplicate identity")]
    DuplicateNativeTarget,
    #[error("the native target catalog exceeds the normalized 256-target bound")]
    TargetCatalogLimitExceeded,
    #[error("the display topology generation exhausted its range")]
    TopologyGenerationExhausted,
    #[error("the source identity or target token could not be created")]
    IdentityUnavailable,
    #[error("the frame-media catalog rejected Windows geometry")]
    MediaCatalogRejected,
    #[error("the selected target changed after it was enumerated")]
    StaleTargetTopology,
    #[error("the requested output dimensions do not exactly match the native selection")]
    OutputDimensionsDoNotMatchTarget,
    #[error("a capture stream is already running")]
    AlreadyRunning,
    #[error("no capture stream is running")]
    NotRunning,
    #[error("the selected display or window is no longer available")]
    TargetNoLongerAvailable,
    #[error("the native callback queue disconnected")]
    CallbackQueueDisconnected,
    #[error("the native callback returned an invalid BGRA frame")]
    InvalidNativeFrame,
    #[error("the native frame row stride is invalid")]
    InvalidRowStride,
    #[error("the native frame storage is shorter than its declared layout")]
    NativeBufferTooShort,
    #[error("the native timestamp is invalid")]
    InvalidTimestamp,
    #[error("the frame sequence exhausted its range")]
    SequenceExhausted,
    #[error("the Windows capture worker could not be started")]
    CaptureStartFailed,
    #[error("capture startup exceeded its operation deadline")]
    CaptureStartTimedOut,
    #[error("the Windows capture worker could not be stopped")]
    CaptureStopFailed,
    #[error("capture teardown exceeded its operation deadline")]
    CaptureStopTimedOut,
    #[error("the adapter is poisoned because native teardown is unconfirmed")]
    CaptureTeardownUnconfirmed,
    #[error("the Windows capture worker stopped unexpectedly")]
    UnexpectedStreamStop,
}

fn bgra_frame_bytes(width: u32, height: u32) -> Result<usize, WindowsCaptureError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .and_then(|row| {
            usize::try_from(height)
                .ok()
                .and_then(|height| row.checked_mul(height))
        })
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)
}

#[cfg(test)]
fn copy_bgra_rows(
    bytes: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Result<Vec<u8>, WindowsCaptureError> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
    if bytes_per_row < row_bytes {
        return Err(WindowsCaptureError::InvalidRowStride);
    }
    let output_len = row_bytes
        .checked_mul(height)
        .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
    if output_len > MAX_OWNED_FRAME_BYTES {
        return Err(WindowsCaptureError::FrameAllocationExceedsLimit);
    }
    let required_len = if height == 0 {
        0
    } else {
        bytes_per_row
            .checked_mul(height - 1)
            .and_then(|prefix| prefix.checked_add(row_bytes))
            .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?
    };
    if bytes.len() < required_len {
        return Err(WindowsCaptureError::NativeBufferTooShort);
    }
    let mut output = Vec::with_capacity(output_len);
    for row in 0..height {
        let start = row
            .checked_mul(bytes_per_row)
            .ok_or(WindowsCaptureError::FrameAllocationExceedsLimit)?;
        output.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    Ok(output)
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Default)]
struct TimestampNormalizer {
    raw_origin_ns: Option<u64>,
    output_origin_ns: u64,
    last_raw_ns: Option<u64>,
    last_output_end_ns: u64,
}

#[cfg(any(target_os = "windows", test))]
impl TimestampNormalizer {
    fn normalize(
        &mut self,
        raw_ns: u64,
        nominal_duration_ns: u64,
    ) -> Result<FrameTimestamp, WindowsCaptureError> {
        if !(MIN_FRAME_DURATION_NS..=MAX_FRAME_DURATION_NS).contains(&nominal_duration_ns) {
            return Err(WindowsCaptureError::InvalidTimestamp);
        }
        let mut discontinuity = false;
        let output_pts_ns = match (self.raw_origin_ns, self.last_raw_ns) {
            (None, None) => {
                self.raw_origin_ns = Some(raw_ns);
                self.output_origin_ns = 0;
                0
            }
            (Some(origin), Some(last)) if raw_ns > last => {
                let delta = raw_ns
                    .checked_sub(origin)
                    .ok_or(WindowsCaptureError::InvalidTimestamp)?;
                let candidate = self
                    .output_origin_ns
                    .checked_add(delta)
                    .ok_or(WindowsCaptureError::InvalidTimestamp)?;
                if candidate < self.last_output_end_ns {
                    discontinuity = true;
                    self.raw_origin_ns = Some(raw_ns);
                    self.output_origin_ns = self.last_output_end_ns;
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
            (Some(_), Some(_)) => {
                discontinuity = true;
                self.raw_origin_ns = Some(raw_ns);
                self.output_origin_ns = self.last_output_end_ns;
                self.last_output_end_ns
            }
            _ => return Err(WindowsCaptureError::InvalidTimestamp),
        };
        let mut timestamp = FrameTimestamp::new(output_pts_ns, nominal_duration_ns)
            .map_err(|_| WindowsCaptureError::InvalidTimestamp)?;
        timestamp.discontinuity = discontinuity;
        self.last_raw_ns = Some(raw_ns);
        self.last_output_end_ns = timestamp.end_ns();
        Ok(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use frame_media::{
        ScreenSourceInstanceId, ScreenTargetEpoch, ScreenTargetId, ScreenTargetKind,
    };

    use super::*;

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
    fn configuration_rejects_unimplemented_profiles_and_accepts_cursor_metadata() {
        let mut output = spec();
        output.pixel_format = PixelFormat::Nv12;
        assert_eq!(
            WindowsCaptureConfig::new(target(), output, CursorCaptureMode::Hidden),
            Err(WindowsCaptureError::UnsupportedOutputProfile)
        );
        assert!(WindowsCaptureConfig::new(target(), spec(), CursorCaptureMode::Metadata).is_ok());
    }

    #[test]
    fn padded_bgra_rows_are_tightly_copied() {
        let bytes = [
            1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16,
        ];
        assert_eq!(
            copy_bgra_rows(&bytes, 2, 2, 12).expect("copy"),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn timestamps_rebase_and_fail_closed_on_clock_regression() {
        let mut normalizer = TimestampNormalizer::default();
        let first = normalizer
            .normalize(5_000_000_000, 20_000_000)
            .expect("first");
        let second = normalizer
            .normalize(5_020_000_000, 20_000_000)
            .expect("second");
        let reset = normalizer.normalize(10, 20_000_000).expect("reset");
        assert_eq!(first.pts_ns, 0);
        assert_eq!(second.pts_ns, 20_000_000);
        assert!(!second.discontinuity);
        assert_eq!(reset.pts_ns, second.end_ns());
        assert!(reset.discontinuity);
    }

    #[test]
    fn frame_debug_redacts_pixel_bytes() {
        let frame = WindowsCaptureFrame {
            target: target(),
            sequence: 1,
            source_pts_ns: Some(5_000),
            timestamp: FrameTimestamp::new(0, 1_000_000).expect("timestamp"),
            spec: VideoFrameSpec {
                width: 1,
                height: 1,
                ..spec()
            },
            pixels: vec![11, 22, 33, 44],
        };
        let debug = format!("{frame:?}");
        assert!(!debug.contains("11"));
        assert!(debug.contains("retained_bytes"));
    }
}
