use std::fmt;

use thiserror::Error;

pub const CAPTURE_CONTRACT_VERSION: u16 = 1;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SourceId(String);

impl SourceId {
    pub fn new(value: impl Into<String>) -> Result<Self, CaptureError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
        {
            return Err(CaptureError::InvalidSourceId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_private_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SourceId(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::EmptyGeometry);
        }
        let _ = x
            .checked_add(i32::try_from(width).map_err(|_| CaptureError::GeometryOverflow)?)
            .ok_or(CaptureError::GeometryOverflow)?;
        let _ = y
            .checked_add(i32::try_from(height).map_err(|_| CaptureError::GeometryOverflow)?)
            .ok_or(CaptureError::GeometryOverflow)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub fn contains(self, x: i32, y: i32) -> bool {
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        i64::from(x) >= i64::from(self.x)
            && i64::from(x) < right
            && i64::from(y) >= i64::from(self.y)
            && i64::from(y) < bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleFactor {
    pub numerator: u16,
    pub denominator: u16,
}

impl ScaleFactor {
    pub fn new(numerator: u16, denominator: u16) -> Result<Self, CaptureError> {
        if numerator == 0 || denominator == 0 {
            return Err(CaptureError::InvalidScale);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn physical_size(self, logical: u32) -> Result<u32, CaptureError> {
        let scaled = u64::from(logical)
            .checked_mul(u64::from(self.numerator))
            .ok_or(CaptureError::GeometryOverflow)?
            / u64::from(self.denominator);
        u32::try_from(scaled).map_err(|_| CaptureError::GeometryOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    Degrees0,
    Degrees90,
    Degrees180,
    Degrees270,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Display {
        id: SourceId,
        geometry: Rect,
        scale: ScaleFactor,
        rotation: Rotation,
    },
    Window {
        id: SourceId,
        geometry: Rect,
        scale: ScaleFactor,
    },
    Region {
        display_id: SourceId,
        geometry: Rect,
        scale: ScaleFactor,
    },
}

impl CaptureTarget {
    #[must_use]
    pub const fn geometry(&self) -> Rect {
        match self {
            Self::Display { geometry, .. }
            | Self::Window { geometry, .. }
            | Self::Region { geometry, .. } => *geometry,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
    Nv12,
    I420,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    DisplayP3,
    Bt709Limited,
    Bt709Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameMemory {
    Cpu,
    DmaBuf,
    Direct3D11,
    CoreVideo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFrameSpec {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub color_space: ColorSpace,
    pub nominal_frame_duration_ns: u64,
    pub memory: FrameMemory,
}

impl VideoFrameSpec {
    pub fn validate(self) -> Result<Self, CaptureError> {
        if self.width == 0 || self.height == 0 || self.nominal_frame_duration_ns == 0 {
            return Err(CaptureError::InvalidFrameSpec);
        }
        let pixels = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or(CaptureError::GeometryOverflow)?;
        if pixels > 132_710_400 {
            // 16K is a deliberate hard safety ceiling, not a product promise.
            return Err(CaptureError::FrameTooLarge);
        }
        Ok(self)
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorSample {
    visible: bool,
    x: u32,
    y: u32,
    image_revision: u64,
    primary_click: bool,
}

impl fmt::Debug for CursorSample {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CursorSample(<redacted>)")
    }
}

/// Converts desktop cursor coordinates to target-local coordinates. A cursor
/// outside the selected geometry is represented as hidden, preventing metadata
/// from leaking activity elsewhere on the desktop.
#[must_use]
#[allow(dead_code)]
pub(crate) fn normalize_cursor(
    target: &CaptureTarget,
    desktop_x: i32,
    desktop_y: i32,
    image_revision: u64,
    primary_click: bool,
) -> CursorSample {
    let geometry = target.geometry();
    if !geometry.contains(desktop_x, desktop_y) {
        return CursorSample {
            visible: false,
            x: 0,
            y: 0,
            image_revision: 0,
            primary_click: false,
        };
    }

    CursorSample {
        visible: true,
        x: desktop_x.abs_diff(geometry.x),
        y: desktop_y.abs_diff(geometry.y),
        image_revision,
        primary_click,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Unknown,
    PromptRequired,
    Granted,
    Denied,
    Restricted,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    Screen,
    Microphone,
    SystemAudio,
    Camera,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub id: SourceId,
    pub class: SourceClass,
    pub is_default: bool,
    pub permission: PermissionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEvent {
    Available,
    Unavailable,
    DefaultChanged,
    PermissionChanged(PermissionState),
    TargetLost,
    Sleep,
    Wake,
    FormatChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u8,
    pub sample_format: AudioSampleFormat,
}

impl AudioFormat {
    pub fn validate(self) -> Result<Self, CaptureError> {
        if !(8_000..=384_000).contains(&self.sample_rate) || !(1..=32).contains(&self.channels) {
            return Err(CaptureError::InvalidAudioFormat);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AudioSampleFormat {
    Signed16,
    Signed32,
    Float32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CameraFormat {
    pub width: u32,
    pub height: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub pixel_format: PixelFormat,
}

impl CameraFormat {
    pub fn validate(self) -> Result<Self, CaptureError> {
        if self.width == 0
            || self.height == 0
            || self.frame_rate_numerator == 0
            || self.frame_rate_denominator == 0
        {
            return Err(CaptureError::InvalidCameraFormat);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTimestamp {
    pub pts_ns: u64,
    pub duration_ns: u64,
    pub discontinuity: bool,
}

impl FrameTimestamp {
    pub fn new(pts_ns: u64, duration_ns: u64) -> Result<Self, CaptureError> {
        if duration_ns == 0 || pts_ns.checked_add(duration_ns).is_none() {
            return Err(CaptureError::InvalidTimestamp);
        }
        Ok(Self {
            pts_ns,
            duration_ns,
            discontinuity: false,
        })
    }

    #[must_use]
    pub const fn end_ns(self) -> u64 {
        self.pts_ns.saturating_add(self.duration_ns)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPolicy {
    pub max_offset_ns: u64,
    pub max_drift_ppm: u32,
    pub max_correction_ns_per_second: u64,
    pub discontinuity_ns: u64,
}

impl SyncPolicy {
    pub fn validate(self) -> Result<Self, CaptureError> {
        if self.max_offset_ns == 0
            || self.max_drift_ppm == 0
            || self.max_correction_ns_per_second == 0
            || self.discontinuity_ns <= self.max_offset_ns
        {
            return Err(CaptureError::InvalidSyncPolicy);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDecision {
    InSync {
        offset_ns: i64,
        drift_ppm: i64,
    },
    AdjustAudio {
        adjustment_ns: i64,
        observed_offset_ns: i64,
        drift_ppm: i64,
    },
    Discontinuity {
        offset_ns: i64,
    },
}

#[derive(Debug, Clone)]
pub struct AvSyncController {
    policy: SyncPolicy,
    last_elapsed_ns: u64,
    calibration_offset_ns: i64,
}

impl AvSyncController {
    pub fn new(policy: SyncPolicy) -> Result<Self, CaptureError> {
        Ok(Self {
            policy: policy.validate()?,
            last_elapsed_ns: 0,
            calibration_offset_ns: 0,
        })
    }

    /// Records a measured startup offset that should not be mistaken for
    /// accumulating drift. The returned value is `video - audio` in nanoseconds.
    pub fn calibrate(&mut self, audio_pts_ns: u64, video_pts_ns: u64) -> Result<i64, CaptureError> {
        let offset = i128::from(video_pts_ns) - i128::from(audio_pts_ns);
        self.calibration_offset_ns =
            i64::try_from(offset).map_err(|_| CaptureError::TimestampRange)?;
        self.last_elapsed_ns = 0;
        Ok(self.calibration_offset_ns)
    }

    pub fn observe(
        &mut self,
        audio_pts_ns: u64,
        video_pts_ns: u64,
        elapsed_ns: u64,
    ) -> Result<SyncDecision, CaptureError> {
        if elapsed_ns == 0 || elapsed_ns < self.last_elapsed_ns {
            return Err(CaptureError::NonMonotonicClock);
        }
        let interval_ns = elapsed_ns - self.last_elapsed_ns;
        self.last_elapsed_ns = elapsed_ns;

        let raw_offset = i128::from(video_pts_ns)
            - i128::from(audio_pts_ns)
            - i128::from(self.calibration_offset_ns);
        let offset = i64::try_from(raw_offset).map_err(|_| CaptureError::TimestampRange)?;
        let absolute = offset.unsigned_abs();
        if absolute >= self.policy.discontinuity_ns {
            return Ok(SyncDecision::Discontinuity { offset_ns: offset });
        }

        let drift_ppm_raw = raw_offset
            .checked_mul(1_000_000)
            .ok_or(CaptureError::TimestampRange)?
            / i128::from(elapsed_ns);
        let drift_ppm = i64::try_from(drift_ppm_raw).map_err(|_| CaptureError::TimestampRange)?;
        if absolute <= self.policy.max_offset_ns
            && drift_ppm.unsigned_abs() <= u64::from(self.policy.max_drift_ppm)
        {
            return Ok(SyncDecision::InSync {
                offset_ns: offset,
                drift_ppm,
            });
        }

        let maximum = self
            .policy
            .max_correction_ns_per_second
            .saturating_mul(interval_ns)
            / 1_000_000_000;
        let maximum = i64::try_from(maximum.max(1)).map_err(|_| CaptureError::TimestampRange)?;
        Ok(SyncDecision::AdjustAudio {
            adjustment_ns: offset.clamp(-maximum, maximum),
            observed_offset_ns: offset,
            drift_ppm,
        })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    #[error("capture source ID is invalid")]
    InvalidSourceId,
    #[error("capture geometry must have non-zero dimensions")]
    EmptyGeometry,
    #[error("capture geometry overflows its coordinate space")]
    GeometryOverflow,
    #[error("capture scale factor must be non-zero")]
    InvalidScale,
    #[error("video frame specification is invalid")]
    InvalidFrameSpec,
    #[error("video frame exceeds the hard safety limit")]
    FrameTooLarge,
    #[error("audio format is outside supported structural limits")]
    InvalidAudioFormat,
    #[error("camera format is invalid")]
    InvalidCameraFormat,
    #[error("frame timestamp is invalid")]
    InvalidTimestamp,
    #[error("A/V synchronization policy is invalid")]
    InvalidSyncPolicy,
    #[error("capture clock moved backwards")]
    NonMonotonicClock,
    #[error("capture timestamp exceeds the supported range")]
    TimestampRange,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> CaptureTarget {
        CaptureTarget::Region {
            display_id: SourceId::new("display-1").expect("source"),
            geometry: Rect::new(-100, 50, 200, 100).expect("rect"),
            scale: ScaleFactor::new(2, 1).expect("scale"),
        }
    }

    #[test]
    fn source_ids_are_redacted_in_debug_output() {
        let source = SourceId::new("private-device-serial").expect("source");
        assert_eq!(format!("{source:?}"), "SourceId(<redacted>)");
    }

    #[test]
    fn cursor_outside_region_is_hidden() {
        let outside = normalize_cursor(&target(), 101, 70, 2, true);
        assert!(!outside.visible);
        assert!(!outside.primary_click);
        assert_eq!(outside.image_revision, 0);

        let inside = normalize_cursor(&target(), -50, 75, 3, true);
        assert_eq!(inside.x, 50);
        assert_eq!(inside.y, 25);
        assert!(inside.visible);
        assert!(inside.primary_click);
        assert_eq!(format!("{inside:?}"), "CursorSample(<redacted>)");
    }

    #[test]
    fn sync_controller_bounds_gradual_correction() {
        let mut controller = AvSyncController::new(SyncPolicy {
            max_offset_ns: 5_000_000,
            max_drift_ppm: 100,
            max_correction_ns_per_second: 1_000_000,
            discontinuity_ns: 100_000_000,
        })
        .expect("policy");
        let decision = controller
            .observe(1_000_000_000, 1_020_000_000, 10_000_000_000)
            .expect("decision");
        assert_eq!(
            decision,
            SyncDecision::AdjustAudio {
                adjustment_ns: 10_000_000,
                observed_offset_ns: 20_000_000,
                drift_ppm: 2_000,
            }
        );
    }

    #[test]
    fn sync_controller_marks_large_jumps_as_discontinuities() {
        let mut controller = AvSyncController::new(SyncPolicy {
            max_offset_ns: 5,
            max_drift_ppm: 100,
            max_correction_ns_per_second: 10,
            discontinuity_ns: 50,
        })
        .expect("policy");
        assert_eq!(
            controller.observe(100, 200, 1_000).expect("decision"),
            SyncDecision::Discontinuity { offset_ns: 100 }
        );
    }

    #[test]
    fn calibrated_startup_offset_is_not_counted_as_drift() {
        let mut controller = AvSyncController::new(SyncPolicy {
            max_offset_ns: 5,
            max_drift_ppm: 100,
            max_correction_ns_per_second: 10,
            discontinuity_ns: 50,
        })
        .expect("policy");
        assert_eq!(controller.calibrate(100, 120).expect("calibrate"), 20);
        assert_eq!(
            controller.observe(1_000, 1_020, 1_000).expect("decision"),
            SyncDecision::InSync {
                offset_ns: 0,
                drift_ppm: 0,
            }
        );
    }
}
