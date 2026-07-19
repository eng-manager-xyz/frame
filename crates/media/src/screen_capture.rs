//! Provider-free contracts for native screen capture.
//!
//! Platform adapters keep ownership of native capture objects and publish
//! frames through the bounded queue in this module. This layer deliberately
//! does not claim that an OS adapter, permission flow, or physical display has
//! passed conformance; it makes those capabilities explicit and rejects silent
//! degradation before a source starts.

use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
    time::{Duration, Instant},
};

use thiserror::Error;

use crate::{
    CancellationToken, ColorSpace, FrameMemory, FrameTimestamp, PixelFormat, Rotation,
    RuntimeCapability, VideoFrameSpec,
};

pub const SCREEN_CAPTURE_CONTRACT_VERSION: u16 = 1;
pub const SCREEN_CAPTURE_DIAGNOSTIC_VERSION: u16 = 1;
pub const MAX_SCREEN_TARGETS: usize = 256;
pub const MAX_EXCLUDED_WINDOWS: usize = 32;
pub const MAX_CAPTURE_QUEUE_FRAMES: u16 = 256;
pub const MAX_CAPTURE_QUEUE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_CAPTURE_QUEUE_AGE_NS: u64 = 10_000_000_000;
pub const MAX_CAPTURE_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
pub const SCREEN_CAPTURE_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_CURSOR_IMAGE_DIMENSION: u16 = 512;
pub const MAX_CURSOR_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SCREEN_FRAME_PROFILES: usize = 32;

/// The kind is part of identity so a platform handle cannot be accidentally
/// reinterpreted as another target class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScreenTargetKind {
    Display,
    Window,
    Region,
}

/// Opaque identity for one live native adapter instance. Restarting an adapter
/// must create a different value even when it enumerates the same OS handles.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenSourceInstanceId([u8; 16]);

impl ScreenSourceInstanceId {
    pub fn new(opaque: [u8; 16]) -> Result<Self, ScreenCaptureError> {
        if opaque.iter().all(|byte| *byte == 0) {
            return Err(ScreenCaptureError::InvalidSourceInstanceId);
        }
        Ok(Self(opaque))
    }
}

impl fmt::Debug for ScreenSourceInstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScreenSourceInstanceId(<redacted>)")
    }
}

/// Collision-resistant identity for one capture-session incarnation.
///
/// Production callers must supply 128 bits from a cryptographically secure
/// random-number generator. The provider-free media crate deliberately does
/// not own an OS RNG. Reusing bytes defeats delayed-event isolation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenSessionId([u8; 16]);

impl ScreenSessionId {
    pub fn from_csprng(bytes: [u8; 16]) -> Result<Self, ScreenCaptureError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(ScreenCaptureError::InvalidSessionId);
        }
        Ok(Self(bytes))
    }
}

impl fmt::Debug for ScreenSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScreenSessionId(<redacted>)")
    }
}

/// Opaque proof that one source object belongs to one capture session. The
/// library constructs this value from the source instance and collision-
/// resistant session identity; adapters may compare/store it but cannot mint
/// another session's binding.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenSourceSessionBinding {
    source_instance: ScreenSourceInstanceId,
    session_id: ScreenSessionId,
}

impl ScreenSourceSessionBinding {
    #[must_use]
    pub const fn source_instance(self) -> ScreenSourceInstanceId {
        self.source_instance
    }
}

impl fmt::Debug for ScreenSourceSessionBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScreenSourceSessionBinding(<redacted>)")
    }
}

/// Non-cloneable ownership ticket presented before a source object's first
/// platform operation. Its private field prevents callers from rebinding a
/// source to an arbitrary session.
pub struct ScreenSourceSessionTicket {
    binding: ScreenSourceSessionBinding,
}

impl ScreenSourceSessionTicket {
    #[must_use]
    pub const fn binding(&self) -> ScreenSourceSessionBinding {
        self.binding
    }
}

impl fmt::Debug for ScreenSourceSessionTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenSourceSessionTicket")
            .field("binding", &self.binding)
            .finish()
    }
}

/// Borrowed proof attached to every non-operation adapter call. Only the
/// library-owned bound source handle can construct it.
pub struct ScreenSourceCallTicket<'a> {
    binding: &'a ScreenSourceSessionBinding,
}

impl ScreenSourceCallTicket<'_> {
    #[must_use]
    pub const fn binding(&self) -> ScreenSourceSessionBinding {
        *self.binding
    }
}

impl fmt::Debug for ScreenSourceCallTicket<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenSourceCallTicket")
            .field("binding", self.binding)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenTargetEpoch(u64);

impl ScreenTargetEpoch {
    pub fn new(value: u64) -> Result<Self, ScreenCaptureError> {
        if value == 0 {
            return Err(ScreenCaptureError::InvalidTargetEpoch);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ScreenTargetEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScreenTargetEpoch(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CaptureEpoch(u64);

impl CaptureEpoch {
    pub fn new(value: u64) -> Result<Self, ScreenCaptureError> {
        if value == 0 {
            return Err(ScreenCaptureError::InvalidCaptureEpoch);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Result<Self, ScreenCaptureError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ScreenCaptureError::CaptureEpochExhausted)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenOperationId(u64);

impl fmt::Debug for ScreenOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScreenOperationId(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenStreamId {
    session: ScreenSessionId,
    sequence: u64,
}

impl fmt::Debug for ScreenStreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenStreamId")
            .field("session", &self.session)
            .field("sequence", &"<redacted>")
            .finish()
    }
}

/// Exact identity carried by every native frame, cursor image, and bound
/// source failure. Safe callers can obtain it only from a live operation
/// ticket received inside `ScreenCaptureSource`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenStreamStamp {
    source_instance: ScreenSourceInstanceId,
    target: ScreenTargetBinding,
    stream: ScreenStreamId,
    capture_epoch: CaptureEpoch,
}

impl ScreenStreamStamp {
    #[must_use]
    pub const fn source_instance(self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    #[must_use]
    pub const fn target(self) -> ScreenTargetBinding {
        self.target
    }

    #[must_use]
    pub const fn stream_id(self) -> ScreenStreamId {
        self.stream
    }

    #[must_use]
    pub const fn capture_epoch(self) -> CaptureEpoch {
        self.capture_epoch
    }
}

impl fmt::Debug for ScreenStreamStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenStreamStamp")
            .field("source_instance", &self.source_instance)
            .field("target", &self.target)
            .field("stream", &self.stream)
            .field("capture_epoch", &self.capture_epoch)
            .finish()
    }
}

/// A stable, host-local opaque identity supplied by a platform adapter.
///
/// Adapters must derive the same value for the same native target while that
/// target remains valid. Raw handles, titles, bundle identifiers, and process
/// names must not be used as telemetry labels.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenTargetId {
    kind: ScreenTargetKind,
    opaque: [u8; 16],
}

impl ScreenTargetId {
    pub fn new(kind: ScreenTargetKind, opaque: [u8; 16]) -> Result<Self, ScreenCaptureError> {
        if opaque.iter().all(|byte| *byte == 0) {
            return Err(ScreenCaptureError::InvalidTargetId);
        }
        Ok(Self { kind, opaque })
    }

    #[must_use]
    pub const fn kind(self) -> ScreenTargetKind {
        self.kind
    }
}

impl fmt::Debug for ScreenTargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenTargetId")
            .field("kind", &self.kind)
            .field("opaque", &"<redacted>")
            .finish()
    }
}

/// Source- and topology-bound proof that a target came from the current
/// adapter catalog. A binding alone is not sufficient to start; the
/// library-owned operation executor checks it against a live snapshot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenTargetBinding {
    source_instance: ScreenSourceInstanceId,
    topology_generation: u64,
    target_epoch: ScreenTargetEpoch,
    id: ScreenTargetId,
}

impl ScreenTargetBinding {
    pub fn new(
        source_instance: ScreenSourceInstanceId,
        topology_generation: u64,
        target_epoch: ScreenTargetEpoch,
        id: ScreenTargetId,
    ) -> Result<Self, ScreenCaptureError> {
        if topology_generation == 0 {
            return Err(ScreenCaptureError::InvalidTopologyGeneration);
        }
        Ok(Self {
            source_instance,
            topology_generation,
            target_epoch,
            id,
        })
    }

    #[must_use]
    pub const fn source_instance(self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    #[must_use]
    pub const fn topology_generation(self) -> u64 {
        self.topology_generation
    }

    #[must_use]
    pub const fn target_epoch(self) -> ScreenTargetEpoch {
        self.target_epoch
    }

    #[must_use]
    pub const fn id(self) -> ScreenTargetId {
        self.id
    }
}

impl fmt::Debug for ScreenTargetBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenTargetBinding")
            .field("source_instance", &self.source_instance)
            .field("topology_generation", &"<redacted>")
            .field("target_epoch", &self.target_epoch)
            .field("id", &self.id)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl LogicalRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, ScreenCaptureError> {
        validate_rect(x, y, width, height)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn contains_point(self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(x) < self.right()
            && i64::from(y) >= i64::from(self.y)
            && i64::from(y) < self.bottom()
    }

    #[must_use]
    pub fn contains_rect(self, other: Self) -> bool {
        i64::from(other.x) >= i64::from(self.x)
            && other.right() <= self.right()
            && i64::from(other.y) >= i64::from(self.y)
            && other.bottom() <= self.bottom()
    }

    const fn right(self) -> i64 {
        self.x as i64 + self.width as i64
    }

    const fn bottom(self) -> i64 {
        self.y as i64 + self.height as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl PhysicalRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, ScreenCaptureError> {
        validate_rect(x, y, width, height)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

fn validate_rect(x: i32, y: i32, width: u32, height: u32) -> Result<(), ScreenCaptureError> {
    if width == 0 || height == 0 {
        return Err(ScreenCaptureError::EmptyGeometry);
    }
    i64::from(x)
        .checked_add(i64::from(width))
        .filter(|right| *right <= i64::from(i32::MAX))
        .ok_or(ScreenCaptureError::GeometryOverflow)?;
    i64::from(y)
        .checked_add(i64::from(height))
        .filter(|bottom| *bottom <= i64::from(i32::MAX))
        .ok_or(ScreenCaptureError::GeometryOverflow)?;
    Ok(())
}

/// An exact rational OS scale factor. Values outside 1/16x..16x are rejected
/// as corrupt adapter input, not interpreted as unusual hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiScale {
    numerator: u32,
    denominator: u32,
}

impl DpiScale {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, ScreenCaptureError> {
        if numerator == 0 || denominator == 0 {
            return Err(ScreenCaptureError::InvalidDpiScale);
        }
        let numerator64 = u64::from(numerator);
        let denominator64 = u64::from(denominator);
        if numerator64 > denominator64.saturating_mul(16)
            || denominator64 > numerator64.saturating_mul(16)
        {
            return Err(ScreenCaptureError::InvalidDpiScale);
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    fn floor(self, logical: u64) -> Result<u64, ScreenCaptureError> {
        logical
            .checked_mul(u64::from(self.numerator))
            .map(|scaled| scaled / u64::from(self.denominator))
            .ok_or(ScreenCaptureError::GeometryOverflow)
    }

    fn ceil(self, logical: u64) -> Result<u64, ScreenCaptureError> {
        logical
            .checked_mul(u64::from(self.numerator))
            .and_then(|scaled| scaled.checked_add(u64::from(self.denominator) - 1))
            .map(|scaled| scaled / u64::from(self.denominator))
            .ok_or(ScreenCaptureError::GeometryOverflow)
    }
}

const fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Maps a display's logical desktop coordinates to its rotated physical pixel
/// coordinates. Rectangle edges use floor/ceil coverage so fractional DPI
/// never clips a selected logical pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayGeometryTransform {
    logical_bounds: LogicalRect,
    physical_bounds: PhysicalRect,
    scale: DpiScale,
    rotation: Rotation,
}

impl DisplayGeometryTransform {
    pub fn new(
        logical_bounds: LogicalRect,
        physical_bounds: PhysicalRect,
        scale: DpiScale,
        rotation: Rotation,
    ) -> Result<Self, ScreenCaptureError> {
        let unrotated_width = scale.ceil(u64::from(logical_bounds.width))?;
        let unrotated_height = scale.ceil(u64::from(logical_bounds.height))?;
        let (expected_width, expected_height) = match rotation {
            Rotation::Degrees0 | Rotation::Degrees180 => (unrotated_width, unrotated_height),
            Rotation::Degrees90 | Rotation::Degrees270 => (unrotated_height, unrotated_width),
        };
        if u64::from(physical_bounds.width) != expected_width
            || u64::from(physical_bounds.height) != expected_height
        {
            return Err(ScreenCaptureError::InconsistentDisplayGeometry);
        }
        Ok(Self {
            logical_bounds,
            physical_bounds,
            scale,
            rotation,
        })
    }

    #[must_use]
    pub const fn logical_bounds(self) -> LogicalRect {
        self.logical_bounds
    }

    #[must_use]
    pub const fn physical_bounds(self) -> PhysicalRect {
        self.physical_bounds
    }

    #[must_use]
    pub const fn scale(self) -> DpiScale {
        self.scale
    }

    #[must_use]
    pub const fn rotation(self) -> Rotation {
        self.rotation
    }

    pub fn logical_rect_to_physical(
        self,
        logical: LogicalRect,
    ) -> Result<PhysicalRect, ScreenCaptureError> {
        if !self.logical_bounds.contains_rect(logical) {
            return Err(ScreenCaptureError::GeometryOutsideDisplay);
        }

        let left = u64::try_from(i64::from(logical.x) - i64::from(self.logical_bounds.x))
            .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
        let top = u64::try_from(i64::from(logical.y) - i64::from(self.logical_bounds.y))
            .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
        let right = left
            .checked_add(u64::from(logical.width))
            .ok_or(ScreenCaptureError::GeometryOverflow)?;
        let bottom = top
            .checked_add(u64::from(logical.height))
            .ok_or(ScreenCaptureError::GeometryOverflow)?;
        let x0 = self.scale.floor(left)?;
        let y0 = self.scale.floor(top)?;
        let x1 = self.scale.ceil(right)?;
        let y1 = self.scale.ceil(bottom)?;
        let source_width = self.scale.ceil(u64::from(self.logical_bounds.width))?;
        let source_height = self.scale.ceil(u64::from(self.logical_bounds.height))?;

        let (rotated_x0, rotated_y0, rotated_x1, rotated_y1) = match self.rotation {
            Rotation::Degrees0 => (x0, y0, x1, y1),
            Rotation::Degrees90 => (source_height - y1, x0, source_height - y0, x1),
            Rotation::Degrees180 => (
                source_width - x1,
                source_height - y1,
                source_width - x0,
                source_height - y0,
            ),
            Rotation::Degrees270 => (y0, source_width - x1, y1, source_width - x0),
        };
        physical_rect_from_edges(
            self.physical_bounds.x,
            self.physical_bounds.y,
            rotated_x0,
            rotated_y0,
            rotated_x1,
            rotated_y1,
        )
    }

    pub fn logical_point_to_physical(
        self,
        logical_x: i32,
        logical_y: i32,
    ) -> Result<(i32, i32), ScreenCaptureError> {
        if !self.logical_bounds.contains_point(logical_x, logical_y) {
            return Err(ScreenCaptureError::GeometryOutsideDisplay);
        }
        let relative_x = u64::try_from(i64::from(logical_x) - i64::from(self.logical_bounds.x))
            .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
        let relative_y = u64::try_from(i64::from(logical_y) - i64::from(self.logical_bounds.y))
            .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
        let scaled_x = self.scale.floor(relative_x)?;
        let scaled_y = self.scale.floor(relative_y)?;
        let source_width = self.scale.ceil(u64::from(self.logical_bounds.width))?;
        let source_height = self.scale.ceil(u64::from(self.logical_bounds.height))?;
        let (rotated_x, rotated_y) = match self.rotation {
            Rotation::Degrees0 => (scaled_x, scaled_y),
            Rotation::Degrees90 => (source_height - 1 - scaled_y, scaled_x),
            Rotation::Degrees180 => (source_width - 1 - scaled_x, source_height - 1 - scaled_y),
            Rotation::Degrees270 => (scaled_y, source_width - 1 - scaled_x),
        };
        let x = checked_physical_coordinate(self.physical_bounds.x, rotated_x)?;
        let y = checked_physical_coordinate(self.physical_bounds.y, rotated_y)?;
        Ok((x, y))
    }
}

fn physical_rect_from_edges(
    origin_x: i32,
    origin_y: i32,
    x0: u64,
    y0: u64,
    x1: u64,
    y1: u64,
) -> Result<PhysicalRect, ScreenCaptureError> {
    let x = checked_physical_coordinate(origin_x, x0)?;
    let y = checked_physical_coordinate(origin_y, y0)?;
    let width = u32::try_from(
        x1.checked_sub(x0)
            .ok_or(ScreenCaptureError::GeometryOverflow)?,
    )
    .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
    let height = u32::try_from(
        y1.checked_sub(y0)
            .ok_or(ScreenCaptureError::GeometryOverflow)?,
    )
    .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
    PhysicalRect::new(x, y, width, height)
}

fn checked_physical_coordinate(origin: i32, offset: u64) -> Result<i32, ScreenCaptureError> {
    i64::from(origin)
        .checked_add(i64::try_from(offset).map_err(|_| ScreenCaptureError::GeometryOverflow)?)
        .and_then(|coordinate| i32::try_from(coordinate).ok())
        .ok_or(ScreenCaptureError::GeometryOverflow)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScreenTargetGeometry {
    Display(DisplayGeometryTransform),
    Window(LogicalRect),
    Region {
        display: ScreenTargetBinding,
        bounds: LogicalRect,
        transform: DisplayGeometryTransform,
    },
}

/// A label-free target descriptor safe to pass between enumeration and start.
#[derive(Clone, PartialEq, Eq)]
pub struct ScreenTargetDescriptor {
    binding: ScreenTargetBinding,
    geometry: ScreenTargetGeometry,
}

impl fmt::Debug for ScreenTargetDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenTargetDescriptor")
            .field("binding", &self.binding)
            .field("geometry", &"<redacted>")
            .finish()
    }
}

impl ScreenTargetDescriptor {
    pub fn display(
        binding: ScreenTargetBinding,
        transform: DisplayGeometryTransform,
    ) -> Result<Self, ScreenCaptureError> {
        validate_target_binding(binding, ScreenTargetKind::Display)?;
        Ok(Self {
            binding,
            geometry: ScreenTargetGeometry::Display(transform),
        })
    }

    pub fn window(
        binding: ScreenTargetBinding,
        logical_bounds: LogicalRect,
    ) -> Result<Self, ScreenCaptureError> {
        validate_target_binding(binding, ScreenTargetKind::Window)?;
        Ok(Self {
            binding,
            geometry: ScreenTargetGeometry::Window(logical_bounds),
        })
    }

    pub fn region(
        binding: ScreenTargetBinding,
        display: ScreenTargetBinding,
        logical_bounds: LogicalRect,
        transform: DisplayGeometryTransform,
    ) -> Result<Self, ScreenCaptureError> {
        validate_target_binding(binding, ScreenTargetKind::Region)?;
        if display.id.kind != ScreenTargetKind::Display
            || display.source_instance != binding.source_instance
            || display.topology_generation != binding.topology_generation
        {
            return Err(ScreenCaptureError::TargetKindMismatch);
        }
        if !transform.logical_bounds.contains_rect(logical_bounds) {
            return Err(ScreenCaptureError::GeometryOutsideDisplay);
        }
        Ok(Self {
            binding,
            geometry: ScreenTargetGeometry::Region {
                display,
                bounds: logical_bounds,
                transform,
            },
        })
    }

    #[must_use]
    pub const fn id(&self) -> ScreenTargetId {
        self.binding.id
    }

    #[must_use]
    pub const fn binding(&self) -> ScreenTargetBinding {
        self.binding
    }

    #[must_use]
    pub const fn target_epoch(&self) -> ScreenTargetEpoch {
        self.binding.target_epoch
    }

    #[must_use]
    pub const fn kind(&self) -> ScreenTargetKind {
        self.binding.id.kind
    }

    #[must_use]
    pub const fn logical_bounds(&self) -> LogicalRect {
        match &self.geometry {
            ScreenTargetGeometry::Display(transform) => transform.logical_bounds,
            ScreenTargetGeometry::Window(bounds) | ScreenTargetGeometry::Region { bounds, .. } => {
                *bounds
            }
        }
    }

    /// Returns the exact display transform for a display target.
    ///
    /// Platform composition roots use this privacy-safe geometry to construct
    /// coarse target summaries without exposing native display handles. Window
    /// and region targets deliberately return `None` because their geometry has
    /// different ownership and clipping semantics.
    #[must_use]
    pub const fn display_transform(&self) -> Option<DisplayGeometryTransform> {
        match &self.geometry {
            ScreenTargetGeometry::Display(transform) => Some(*transform),
            ScreenTargetGeometry::Window(_) | ScreenTargetGeometry::Region { .. } => None,
        }
    }

    #[must_use]
    pub const fn containing_display_id(&self) -> Option<ScreenTargetId> {
        match &self.geometry {
            ScreenTargetGeometry::Display(_) | ScreenTargetGeometry::Window(_) => None,
            ScreenTargetGeometry::Region { display, .. } => Some(display.id),
        }
    }

    #[must_use]
    pub const fn containing_display_binding(&self) -> Option<ScreenTargetBinding> {
        match &self.geometry {
            ScreenTargetGeometry::Display(_) | ScreenTargetGeometry::Window(_) => None,
            ScreenTargetGeometry::Region { display, .. } => Some(*display),
        }
    }

    fn physical_selection(&self) -> Result<Option<PhysicalRect>, ScreenCaptureError> {
        match &self.geometry {
            ScreenTargetGeometry::Display(transform) => Ok(Some(transform.physical_bounds)),
            ScreenTargetGeometry::Window(_) => Ok(None),
            ScreenTargetGeometry::Region {
                bounds, transform, ..
            } => transform.logical_rect_to_physical(*bounds).map(Some),
        }
    }

    fn physical_cursor_point(
        &self,
        desktop_x: i32,
        desktop_y: i32,
    ) -> Result<Option<(u64, u64, u64, u64)>, ScreenCaptureError> {
        let bounds = self.logical_bounds();
        if !bounds.contains_point(desktop_x, desktop_y) {
            return Ok(None);
        }
        match &self.geometry {
            ScreenTargetGeometry::Display(transform)
            | ScreenTargetGeometry::Region { transform, .. } => {
                let selection = self
                    .physical_selection()?
                    .ok_or(ScreenCaptureError::InconsistentDisplayGeometry)?;
                let (x, y) = transform.logical_point_to_physical(desktop_x, desktop_y)?;
                let local_x = u64::try_from(i64::from(x) - i64::from(selection.x))
                    .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
                let local_y = u64::try_from(i64::from(y) - i64::from(selection.y))
                    .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
                Ok(Some((
                    local_x,
                    local_y,
                    u64::from(selection.width),
                    u64::from(selection.height),
                )))
            }
            ScreenTargetGeometry::Window(bounds) => {
                let local_x = u64::try_from(i64::from(desktop_x) - i64::from(bounds.x))
                    .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
                let local_y = u64::try_from(i64::from(desktop_y) - i64::from(bounds.y))
                    .map_err(|_| ScreenCaptureError::GeometryOverflow)?;
                Ok(Some((
                    local_x,
                    local_y,
                    u64::from(bounds.width),
                    u64::from(bounds.height),
                )))
            }
        }
    }
}

fn validate_target_binding(
    binding: ScreenTargetBinding,
    expected: ScreenTargetKind,
) -> Result<(), ScreenCaptureError> {
    if binding.id.kind != expected {
        return Err(ScreenCaptureError::TargetKindMismatch);
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScreenTargetSnapshot {
    source_instance: ScreenSourceInstanceId,
    generation: u64,
    targets: Vec<ScreenTargetDescriptor>,
}

impl ScreenTargetSnapshot {
    pub fn new(
        source_instance: ScreenSourceInstanceId,
        generation: u64,
        targets: Vec<ScreenTargetDescriptor>,
    ) -> Result<Self, ScreenCaptureError> {
        if generation == 0 {
            return Err(ScreenCaptureError::InvalidTopologyGeneration);
        }
        if targets.len() > MAX_SCREEN_TARGETS {
            return Err(ScreenCaptureError::TooManyTargets);
        }
        let unique = targets
            .iter()
            .map(ScreenTargetDescriptor::id)
            .collect::<BTreeSet<_>>();
        if unique.len() != targets.len() {
            return Err(ScreenCaptureError::DuplicateTarget);
        }
        if targets.iter().any(|target| {
            target.binding.source_instance != source_instance
                || target.binding.topology_generation != generation
        }) {
            return Err(ScreenCaptureError::TargetCatalogBindingMismatch);
        }
        for target in &targets {
            if let ScreenTargetGeometry::Region {
                display,
                bounds,
                transform,
            } = &target.geometry
            {
                let Some(candidate) = targets
                    .iter()
                    .find(|candidate| candidate.binding() == *display)
                else {
                    return Err(ScreenCaptureError::MissingContainingDisplay);
                };
                let ScreenTargetGeometry::Display(canonical) = &candidate.geometry else {
                    return Err(ScreenCaptureError::MissingContainingDisplay);
                };
                if canonical != transform || !canonical.logical_bounds().contains_rect(*bounds) {
                    return Err(ScreenCaptureError::ForgedRegionTransform);
                }
            }
        }
        Ok(Self {
            source_instance,
            generation,
            targets,
        })
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn source_instance(&self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    #[must_use]
    pub fn targets(&self) -> &[ScreenTargetDescriptor] {
        &self.targets
    }

    #[must_use]
    pub fn find(&self, id: ScreenTargetId) -> Option<&ScreenTargetDescriptor> {
        self.targets.iter().find(|target| target.id() == id)
    }

    #[must_use]
    pub fn find_binding(&self, binding: ScreenTargetBinding) -> Option<&ScreenTargetDescriptor> {
        self.targets
            .iter()
            .find(|target| target.binding() == binding)
    }
}

impl fmt::Debug for ScreenTargetSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenTargetSnapshot")
            .field("source_instance", &self.source_instance)
            .field("generation", &"<redacted>")
            .field("target_count", &self.targets.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorCaptureMode {
    Hidden,
    EmbeddedInFrame,
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPolicy {
    mode: CursorCaptureMode,
    include_image_revision: bool,
    include_clicks: bool,
}

impl CursorPolicy {
    pub fn new(
        mode: CursorCaptureMode,
        include_image_revision: bool,
        include_clicks: bool,
    ) -> Result<Self, ScreenCaptureError> {
        if mode != CursorCaptureMode::Metadata && (include_image_revision || include_clicks) {
            return Err(ScreenCaptureError::InvalidCursorPolicy);
        }
        Ok(Self {
            mode,
            include_image_revision,
            include_clicks,
        })
    }

    #[must_use]
    pub const fn mode(self) -> CursorCaptureMode {
        self.mode
    }

    #[must_use]
    pub const fn include_image_revision(self) -> bool {
        self.include_image_revision
    }

    #[must_use]
    pub const fn include_clicks(self) -> bool {
        self.include_clicks
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RawCursorPosition {
    /// Global logical desktop coordinates, suitable for a display or a region
    /// whose validated display transform is available.
    DesktopLogical { x: i32, y: i32 },
    /// Coordinates already normalized by the native API to the negotiated
    /// output frame. This is required for windows that can span mixed-DPI
    /// displays because one desktop scale cannot map them correctly.
    TargetFramePhysical { x: u32, y: u32 },
}

impl fmt::Debug for RawCursorPosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RawCursorPosition(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RawCursorObservation {
    pub visible: bool,
    pub position: RawCursorPosition,
    pub image_revision: Option<u64>,
    pub primary_click: bool,
    pub secondary_click: bool,
}

impl fmt::Debug for RawCursorObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RawCursorObservation(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CursorFrameMetadata {
    visible: bool,
    frame_x: u32,
    frame_y: u32,
    image_revision: Option<u64>,
    primary_click: bool,
    secondary_click: bool,
}

impl CursorFrameMetadata {
    #[must_use]
    pub const fn visible(self) -> bool {
        self.visible
    }

    #[must_use]
    pub const fn frame_position(self) -> Option<(u32, u32)> {
        if self.visible {
            Some((self.frame_x, self.frame_y))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn image_revision(self) -> Option<u64> {
        self.image_revision
    }

    #[must_use]
    pub const fn primary_click(self) -> bool {
        self.primary_click
    }

    #[must_use]
    pub const fn secondary_click(self) -> bool {
        self.secondary_click
    }

    const fn hidden() -> Self {
        Self {
            visible: false,
            frame_x: 0,
            frame_y: 0,
            image_revision: None,
            primary_click: false,
            secondary_click: false,
        }
    }
}

impl fmt::Debug for CursorFrameMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CursorFrameMetadata")
            .field("visible", &self.visible)
            .field("metadata", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CursorImageDescriptor {
    revision: u64,
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    pixel_format: PixelFormat,
    retained_bytes: u64,
}

impl CursorImageDescriptor {
    pub fn new(
        revision: u64,
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        pixel_format: PixelFormat,
        retained_bytes: u64,
    ) -> Result<Self, ScreenCaptureError> {
        let minimum_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ScreenCaptureError::InvalidCursorImage)?;
        if revision == 0
            || width == 0
            || height == 0
            || width > MAX_CURSOR_IMAGE_DIMENSION
            || height > MAX_CURSOR_IMAGE_DIMENSION
            || hotspot_x >= width
            || hotspot_y >= height
            || !matches!(pixel_format, PixelFormat::Bgra8 | PixelFormat::Rgba8)
            || retained_bytes < minimum_bytes
            || retained_bytes > MAX_CURSOR_IMAGE_BYTES
        {
            return Err(ScreenCaptureError::InvalidCursorImage);
        }
        Ok(Self {
            revision,
            width,
            height,
            hotspot_x,
            hotspot_y,
            pixel_format,
            retained_bytes,
        })
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn dimensions(self) -> (u16, u16) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn hotspot(self) -> (u16, u16) {
        (self.hotspot_x, self.hotspot_y)
    }

    #[must_use]
    pub const fn pixel_format(self) -> PixelFormat {
        self.pixel_format
    }

    #[must_use]
    pub const fn retained_bytes(self) -> u64 {
        self.retained_bytes
    }
}

impl fmt::Debug for CursorImageDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CursorImageDescriptor")
            .field("revision", &"<redacted>")
            .field("dimensions", &self.dimensions())
            .field("hotspot", &"<redacted>")
            .field("pixel_format", &self.pixel_format)
            .field("retained_bytes", &self.retained_bytes)
            .finish()
    }
}

/// An owned cursor-image update backed by an exact CPU allocation. Dropping it
/// releases that allocation under the same rule as a screen frame.
pub struct ScreenCursorImage<T> {
    stream: ScreenStreamStamp,
    descriptor: CursorImageDescriptor,
    payload: T,
}

impl<T: ScreenFramePayload> ScreenCursorImage<T> {
    pub fn new(
        stream: ScreenStreamStamp,
        descriptor: CursorImageDescriptor,
        payload: T,
    ) -> Result<Self, ScreenCaptureError> {
        if payload.exact_retained_bytes() != Some(descriptor.retained_bytes()) {
            return Err(ScreenCaptureError::CursorPayloadAccountingMismatch);
        }
        Ok(Self {
            stream,
            descriptor,
            payload,
        })
    }
}

impl<T> ScreenCursorImage<T> {
    #[must_use]
    pub const fn stream(&self) -> ScreenStreamStamp {
        self.stream
    }

    #[must_use]
    pub const fn descriptor(&self) -> CursorImageDescriptor {
        self.descriptor
    }

    #[must_use]
    pub fn into_payload(self) -> T {
        self.payload
    }
}

impl<T> fmt::Debug for ScreenCursorImage<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenCursorImage")
            .field("stream", &self.stream)
            .field("descriptor", &self.descriptor)
            .field("payload", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenCursorCacheDrain {
    pub images: u8,
    pub bytes: u64,
}

/// A one-image cache with explicit epoch and revision rules. Replacing or
/// resetting the cached image drops the previous payload lease immediately.
struct BoundedCursorImageCache<T> {
    capture_epoch: CaptureEpoch,
    target_epoch: ScreenTargetEpoch,
    image: Option<ScreenCursorImage<T>>,
}

impl<T> fmt::Debug for BoundedCursorImageCache<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedCursorImageCache")
            .field("capture_epoch", &self.capture_epoch)
            .field("target_epoch", &self.target_epoch)
            .field("occupied", &self.image.is_some())
            .finish()
    }
}

impl<T> BoundedCursorImageCache<T> {
    #[must_use]
    const fn new(capture_epoch: CaptureEpoch, target_epoch: ScreenTargetEpoch) -> Self {
        Self {
            capture_epoch,
            target_epoch,
            image: None,
        }
    }

    fn apply(&mut self, image: ScreenCursorImage<T>) -> Result<(), ScreenCaptureError>
    where
        T: ScreenFramePayload,
    {
        if image.payload.exact_retained_bytes() != Some(image.descriptor.retained_bytes()) {
            return Err(ScreenCaptureError::CursorPayloadAccountingMismatch);
        }
        if image.stream.capture_epoch != self.capture_epoch {
            return Err(ScreenCaptureError::CaptureEpochMismatch);
        }
        if image.stream.target.target_epoch() != self.target_epoch {
            return Err(ScreenCaptureError::TargetEpochMismatch);
        }
        if self
            .image
            .as_ref()
            .is_some_and(|current| image.descriptor.revision <= current.descriptor.revision)
        {
            return Err(ScreenCaptureError::NonMonotonicCursorImageRevision);
        }
        self.image = Some(image);
        Ok(())
    }

    /// Checks the frame's reference against the single live lease. A visible
    /// cursor that requested image revisions may never silently omit one.
    fn validate_metadata(
        &self,
        cursor: Option<CursorFrameMetadata>,
        require_image_revision: bool,
    ) -> Result<(), ScreenCaptureError> {
        let Some(cursor) = cursor.filter(|cursor| cursor.visible) else {
            return Ok(());
        };
        let Some(revision) = cursor.image_revision else {
            return if require_image_revision {
                Err(ScreenCaptureError::MissingCursorImageRevision)
            } else {
                Ok(())
            };
        };
        let Some(current) = self.image.as_ref() else {
            return Err(ScreenCaptureError::MissingCursorImage);
        };
        match revision.cmp(&current.descriptor.revision) {
            std::cmp::Ordering::Less => Err(ScreenCaptureError::StaleCursorImageRevision),
            std::cmp::Ordering::Equal => Ok(()),
            std::cmp::Ordering::Greater => Err(ScreenCaptureError::MissingCursorImage),
        }
    }

    fn reset(
        &mut self,
        capture_epoch: CaptureEpoch,
        target_epoch: ScreenTargetEpoch,
    ) -> ScreenCursorCacheDrain {
        let drain = self
            .image
            .take()
            .map_or_else(ScreenCursorCacheDrain::default, |image| {
                ScreenCursorCacheDrain {
                    images: 1,
                    bytes: image.descriptor.retained_bytes,
                }
            });
        self.capture_epoch = capture_epoch;
        self.target_epoch = target_epoch;
        drain
    }

    #[must_use]
    fn descriptor(&self) -> Option<CursorImageDescriptor> {
        self.image.as_ref().map(|image| image.descriptor)
    }
}

/// Normalizes global cursor input into the selected frame. Metadata outside the
/// target is replaced with a fully hidden sample, including clicks and image
/// revision, so activity elsewhere on the desktop cannot leak.
pub fn normalize_screen_cursor(
    target: &ScreenTargetDescriptor,
    output: VideoFrameSpec,
    policy: CursorPolicy,
    observation: RawCursorObservation,
) -> Result<Option<CursorFrameMetadata>, ScreenCaptureError> {
    output
        .validate()
        .map_err(|error| ScreenCaptureError::InvalidVideoFrameSpec(Box::new(error)))?;
    if policy.mode != CursorCaptureMode::Metadata {
        return Ok(None);
    }
    if !observation.visible {
        return Ok(Some(CursorFrameMetadata::hidden()));
    }
    let (frame_x, frame_y) = match observation.position {
        RawCursorPosition::TargetFramePhysical { x, y } => {
            if x >= output.width || y >= output.height {
                return Ok(Some(CursorFrameMetadata::hidden()));
            }
            (x, y)
        }
        RawCursorPosition::DesktopLogical { x, y } => {
            if target.kind() == ScreenTargetKind::Window {
                return Err(ScreenCaptureError::UnsupportedCursorCoordinateSpace);
            }
            let Some((source_x, source_y, source_width, source_height)) =
                target.physical_cursor_point(x, y)?
            else {
                return Ok(Some(CursorFrameMetadata::hidden()));
            };
            if source_width == 0 || source_height == 0 {
                return Err(ScreenCaptureError::EmptyGeometry);
            }
            let frame_x = source_x
                .checked_mul(u64::from(output.width))
                .ok_or(ScreenCaptureError::GeometryOverflow)?
                / source_width;
            let frame_y = source_y
                .checked_mul(u64::from(output.height))
                .ok_or(ScreenCaptureError::GeometryOverflow)?
                / source_height;
            (
                u32::try_from(frame_x)
                    .map_err(|_| ScreenCaptureError::GeometryOverflow)?
                    .min(output.width - 1),
                u32::try_from(frame_y)
                    .map_err(|_| ScreenCaptureError::GeometryOverflow)?
                    .min(output.height - 1),
            )
        }
    };
    Ok(Some(CursorFrameMetadata {
        visible: true,
        frame_x,
        frame_y,
        image_revision: policy
            .include_image_revision
            .then_some(observation.image_revision)
            .flatten()
            .filter(|revision| *revision > 0),
        primary_click: policy.include_clicks && observation.primary_click,
        secondary_click: policy.include_clicks && observation.secondary_click,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenPermissionState {
    Unchecked,
    PromptRequired,
    Requesting,
    Granted,
    Denied,
    Restricted,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsGuidance {
    Unavailable,
    OpenSystemSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPreflight {
    Granted,
    PromptRequired,
    Denied(SettingsGuidance),
    Restricted,
    Revoked(SettingsGuidance),
}

impl PermissionPreflight {
    #[must_use]
    pub const fn state(self) -> ScreenPermissionState {
        match self {
            Self::Granted => ScreenPermissionState::Granted,
            Self::PromptRequired => ScreenPermissionState::PromptRequired,
            Self::Denied(_) => ScreenPermissionState::Denied,
            Self::Restricted => ScreenPermissionState::Restricted,
            Self::Revoked(_) => ScreenPermissionState::Revoked,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenControlEpoch(u64);

impl ScreenControlEpoch {
    pub fn new(value: u64) -> Result<Self, ScreenCaptureError> {
        if value == 0 {
            return Err(ScreenCaptureError::InvalidControlEpoch);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ScreenControlEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScreenControlEpoch(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ScreenControlStamp {
    source_instance: ScreenSourceInstanceId,
    epoch: ScreenControlEpoch,
    sequence: u64,
}

impl ScreenControlStamp {
    pub fn new(
        source_instance: ScreenSourceInstanceId,
        epoch: ScreenControlEpoch,
        sequence: u64,
    ) -> Result<Self, ScreenCaptureError> {
        if sequence == 0 {
            return Err(ScreenCaptureError::InvalidControlSequence);
        }
        Ok(Self {
            source_instance,
            epoch,
            sequence,
        })
    }

    #[must_use]
    pub const fn source_instance(self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    #[must_use]
    pub const fn epoch(self) -> ScreenControlEpoch {
        self.epoch
    }

    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

impl fmt::Debug for ScreenControlStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenControlStamp")
            .field("source_instance", &self.source_instance)
            .field("epoch", &self.epoch)
            .field("sequence", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenPermissionObservation {
    pub stamp: ScreenControlStamp,
    pub permission: PermissionPreflight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCapturePlatform {
    MacOs,
    Windows,
    Linux,
    Unsupported,
}

impl ScreenCapturePlatform {
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Unsupported
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformScreenSource {
    ScreenCaptureKit,
    WindowsGraphicsCapture,
    PipeWirePortal,
    X11Native,
}

impl PlatformScreenSource {
    #[must_use]
    pub const fn platform(self) -> ScreenCapturePlatform {
        match self {
            Self::ScreenCaptureKit => ScreenCapturePlatform::MacOs,
            Self::WindowsGraphicsCapture => ScreenCapturePlatform::Windows,
            Self::PipeWirePortal | Self::X11Native => ScreenCapturePlatform::Linux,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenTargetKinds(u8);

impl ScreenTargetKinds {
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn with(self, kind: ScreenTargetKind) -> Self {
        let bit = match kind {
            ScreenTargetKind::Display => 1,
            ScreenTargetKind::Window => 2,
            ScreenTargetKind::Region => 4,
        };
        Self(self.0 | bit)
    }

    #[must_use]
    pub const fn contains(self, kind: ScreenTargetKind) -> bool {
        let bit = match kind {
            ScreenTargetKind::Display => 1,
            ScreenTargetKind::Window => 2,
            ScreenTargetKind::Region => 4,
        };
        self.0 & bit != 0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCursorModes(u8);

impl ScreenCursorModes {
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn with(self, mode: CursorCaptureMode) -> Self {
        let bit = match mode {
            CursorCaptureMode::Hidden => 1,
            CursorCaptureMode::EmbeddedInFrame => 2,
            CursorCaptureMode::Metadata => 4,
        };
        Self(self.0 | bit)
    }

    #[must_use]
    pub const fn contains(self, mode: CursorCaptureMode) -> bool {
        let bit = match mode {
            CursorCaptureMode::Hidden => 1,
            CursorCaptureMode::EmbeddedInFrame => 2,
            CursorCaptureMode::Metadata => 4,
        };
        self.0 & bit != 0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenFrameProfile {
    pub pixel_format: PixelFormat,
    pub color_space: ColorSpace,
    pub memory: FrameMemory,
    pub max_width: u32,
    pub max_height: u32,
    pub max_frames_per_second: u16,
}

impl ScreenFrameProfile {
    fn validate(self, platform: ScreenCapturePlatform) -> Result<(), ScreenCaptureError> {
        if self.max_width == 0
            || self.max_height == 0
            || self.max_width > 16_384
            || self.max_height > 16_384
            || !(1..=240).contains(&self.max_frames_per_second)
            || !memory_matches_platform(self.memory, platform)
        {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        Ok(())
    }

    fn supports(self, output: VideoFrameSpec) -> Result<bool, ScreenCaptureError> {
        let frames_per_second = 1_000_000_000_u64
            .checked_add(output.nominal_frame_duration_ns - 1)
            .ok_or(ScreenCaptureError::InvalidFrameRate)?
            / output.nominal_frame_duration_ns;
        Ok(self.pixel_format == output.pixel_format
            && self.color_space == output.color_space
            && self.memory == output.memory
            && output.width <= self.max_width
            && output.height <= self.max_height
            && frames_per_second > 0
            && frames_per_second <= u64::from(self.max_frames_per_second))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScreenSourceCapabilitySpec {
    pub contract_version: u16,
    pub source: PlatformScreenSource,
    pub source_instance: ScreenSourceInstanceId,
    pub topology_generation: u64,
    pub control_epoch: ScreenControlEpoch,
    pub control_sequence: u64,
    pub targets: ScreenTargetKinds,
    pub cursor_modes: ScreenCursorModes,
    pub cursor_image_metadata: bool,
    pub cursor_click_metadata: bool,
    pub cursor_desktop_logical_coordinates: bool,
    pub cursor_frame_physical_coordinates: bool,
    pub frame_profiles: Vec<ScreenFrameProfile>,
    pub permission_preflight: bool,
    pub topology_events: bool,
    pub target_recovery: bool,
    pub protected_content_events: bool,
    pub window_exclusion: bool,
    pub max_excluded_windows: u8,
    pub bounded_appsrc_ingress: bool,
}

impl fmt::Debug for ScreenSourceCapabilitySpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenSourceCapabilitySpec")
            .field("contract_version", &self.contract_version)
            .field("source", &self.source)
            .field("source_instance", &self.source_instance)
            .field("topology_generation", &"<redacted>")
            .field("control_epoch", &self.control_epoch)
            .field("control_sequence", &"<redacted>")
            .field("targets", &self.targets)
            .field("cursor_modes", &self.cursor_modes)
            .field("frame_profiles", &self.frame_profiles)
            .field("permission_preflight", &self.permission_preflight)
            .field("topology_events", &self.topology_events)
            .field("target_recovery", &self.target_recovery)
            .field("protected_content_events", &self.protected_content_events)
            .field("window_exclusion", &self.window_exclusion)
            .field("max_excluded_windows", &self.max_excluded_windows)
            .field("bounded_appsrc_ingress", &self.bounded_appsrc_ingress)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScreenSourceCapabilities {
    spec: ScreenSourceCapabilitySpec,
}

impl fmt::Debug for ScreenSourceCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ScreenSourceCapabilities")
            .field(&self.spec)
            .finish()
    }
}

impl ScreenSourceCapabilities {
    pub fn new(spec: ScreenSourceCapabilitySpec) -> Result<Self, ScreenCaptureError> {
        if spec.contract_version != SCREEN_CAPTURE_CONTRACT_VERSION {
            return Err(ScreenCaptureError::IncompatibleContract);
        }
        if spec.topology_generation == 0
            || spec.control_sequence == 0
            || spec.targets.is_empty()
            || spec.cursor_modes.is_empty()
            || spec.frame_profiles.is_empty()
            || spec.frame_profiles.len() > MAX_SCREEN_FRAME_PROFILES
        {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        if has_duplicates(&spec.frame_profiles) {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        for profile in &spec.frame_profiles {
            profile.validate(spec.source.platform())?;
        }
        if spec.target_recovery && !spec.topology_events {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        if (spec.cursor_image_metadata
            || spec.cursor_click_metadata
            || spec.cursor_desktop_logical_coordinates
            || spec.cursor_frame_physical_coordinates)
            && !spec.cursor_modes.contains(CursorCaptureMode::Metadata)
        {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        if spec.cursor_modes.contains(CursorCaptureMode::Metadata)
            && !spec.cursor_desktop_logical_coordinates
            && !spec.cursor_frame_physical_coordinates
        {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        if spec.window_exclusion {
            if !spec.targets.contains(ScreenTargetKind::Window)
                || spec.max_excluded_windows == 0
                || usize::from(spec.max_excluded_windows) > MAX_EXCLUDED_WINDOWS
            {
                return Err(ScreenCaptureError::InvalidCapabilities);
            }
        } else if spec.max_excluded_windows != 0 {
            return Err(ScreenCaptureError::InvalidCapabilities);
        }
        Ok(Self { spec })
    }

    #[must_use]
    pub const fn source(&self) -> PlatformScreenSource {
        self.spec.source
    }

    #[must_use]
    pub const fn source_instance(&self) -> ScreenSourceInstanceId {
        self.spec.source_instance
    }

    #[must_use]
    pub const fn topology_generation(&self) -> u64 {
        self.spec.topology_generation
    }

    #[must_use]
    pub const fn control_epoch(&self) -> ScreenControlEpoch {
        self.spec.control_epoch
    }

    #[must_use]
    pub const fn control_sequence(&self) -> u64 {
        self.spec.control_sequence
    }

    #[must_use]
    pub const fn target_kinds(&self) -> ScreenTargetKinds {
        self.spec.targets
    }

    #[must_use]
    pub const fn cursor_modes(&self) -> ScreenCursorModes {
        self.spec.cursor_modes
    }

    #[must_use]
    pub fn frame_profiles(&self) -> &[ScreenFrameProfile] {
        &self.spec.frame_profiles
    }

    #[must_use]
    pub const fn permission_preflight(&self) -> bool {
        self.spec.permission_preflight
    }

    #[must_use]
    pub const fn topology_events(&self) -> bool {
        self.spec.topology_events
    }

    #[must_use]
    pub const fn protected_content_events(&self) -> bool {
        self.spec.protected_content_events
    }

    #[must_use]
    pub const fn window_exclusion(&self) -> bool {
        self.spec.window_exclusion
    }

    #[must_use]
    pub const fn spec(&self) -> &ScreenSourceCapabilitySpec {
        &self.spec
    }

    #[must_use]
    pub const fn platform(&self) -> ScreenCapturePlatform {
        self.spec.source.platform()
    }

    pub fn validate_for_platform(
        &self,
        platform: ScreenCapturePlatform,
    ) -> Result<(), ScreenCaptureError> {
        if self.platform() != platform {
            return Err(ScreenCaptureError::SourcePlatformMismatch);
        }
        Ok(())
    }
}

const fn memory_matches_platform(memory: FrameMemory, platform: ScreenCapturePlatform) -> bool {
    match memory {
        FrameMemory::Cpu => true,
        FrameMemory::DmaBuf => matches!(platform, ScreenCapturePlatform::Linux),
        FrameMemory::Direct3D11 => matches!(platform, ScreenCapturePlatform::Windows),
        FrameMemory::CoreVideo => matches!(platform, ScreenCapturePlatform::MacOs),
    }
}

fn has_duplicates<T: Eq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureQueueOverflow {
    DropNewest,
    DropOldest,
}

/// A non-blocking producer policy for callbacks owned by native capture APIs.
/// Blocking is intentionally absent: a platform callback must never wait on
/// downstream GStreamer work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCaptureQueuePolicy {
    max_frames: u16,
    max_bytes: u64,
    max_age_ns: u64,
    overflow: CaptureQueueOverflow,
}

impl ScreenCaptureQueuePolicy {
    pub fn new(
        max_frames: u16,
        max_bytes: u64,
        max_age_ns: u64,
        overflow: CaptureQueueOverflow,
    ) -> Result<Self, ScreenCaptureError> {
        if max_frames == 0
            || max_frames > MAX_CAPTURE_QUEUE_FRAMES
            || max_bytes == 0
            || max_bytes > MAX_CAPTURE_QUEUE_BYTES
            || max_age_ns == 0
            || max_age_ns > MAX_CAPTURE_QUEUE_AGE_NS
        {
            return Err(ScreenCaptureError::InvalidQueuePolicy);
        }
        Ok(Self {
            max_frames,
            max_bytes,
            max_age_ns,
            overflow,
        })
    }

    #[must_use]
    pub const fn max_frames(self) -> u16 {
        self.max_frames
    }

    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    #[must_use]
    pub const fn max_age_ns(self) -> u64 {
        self.max_age_ns
    }

    #[must_use]
    pub const fn overflow(self) -> CaptureQueueOverflow {
        self.overflow
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRecoveryPolicy {
    FailClosed,
    ResumeSameTarget { max_attempts: u8 },
}

impl TargetRecoveryPolicy {
    fn validate(self) -> Result<Self, ScreenCaptureError> {
        if let Self::ResumeSameTarget { max_attempts } = self
            && !(1..=10).contains(&max_attempts)
        {
            return Err(ScreenCaptureError::InvalidRecoveryPolicy);
        }
        Ok(self)
    }

    const fn max_attempts(self) -> u8 {
        match self {
            Self::FailClosed => 0,
            Self::ResumeSameTarget { max_attempts } => max_attempts,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedContentPolicy {
    SuspendUntilClear,
    FailSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureRequestSpec {
    pub target: ScreenTargetDescriptor,
    pub output: VideoFrameSpec,
    pub cursor: CursorPolicy,
    pub excluded_windows: Vec<ScreenTargetBinding>,
    pub queue: ScreenCaptureQueuePolicy,
    pub recovery: TargetRecoveryPolicy,
    pub protected_content: ProtectedContentPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureRequest {
    spec: ScreenCaptureRequestSpec,
}

impl ScreenCaptureRequest {
    pub fn new(spec: ScreenCaptureRequestSpec) -> Result<Self, ScreenCaptureError> {
        spec.output
            .validate()
            .map_err(|error| ScreenCaptureError::InvalidVideoFrameSpec(Box::new(error)))?;
        spec.recovery.validate()?;
        if spec.excluded_windows.len() > MAX_EXCLUDED_WINDOWS {
            return Err(ScreenCaptureError::TooManyExcludedWindows);
        }
        if has_duplicates(&spec.excluded_windows)
            || spec.excluded_windows.iter().any(|binding| {
                binding.id().kind() != ScreenTargetKind::Window
                    || binding.id() == spec.target.id()
                    || binding.source_instance() != spec.target.binding().source_instance()
                    || binding.topology_generation() != spec.target.binding().topology_generation()
            })
        {
            return Err(ScreenCaptureError::InvalidWindowExclusion);
        }
        Ok(Self { spec })
    }

    #[must_use]
    pub const fn target(&self) -> &ScreenTargetDescriptor {
        &self.spec.target
    }

    #[must_use]
    pub const fn spec(&self) -> &ScreenCaptureRequestSpec {
        &self.spec
    }

    #[must_use]
    pub const fn output(&self) -> VideoFrameSpec {
        self.spec.output
    }

    #[must_use]
    pub const fn cursor(&self) -> CursorPolicy {
        self.spec.cursor
    }

    #[must_use]
    pub fn excluded_windows(&self) -> &[ScreenTargetBinding] {
        &self.spec.excluded_windows
    }

    #[must_use]
    pub const fn queue(&self) -> ScreenCaptureQueuePolicy {
        self.spec.queue
    }

    #[must_use]
    pub const fn recovery(&self) -> TargetRecoveryPolicy {
        self.spec.recovery
    }

    #[must_use]
    pub const fn protected_content(&self) -> ProtectedContentPolicy {
        self.spec.protected_content
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSrcBufferLifetime {
    /// The bridge retains the exact CPU allocation until GStreamer releases
    /// the corresponding buffer. Native GPU/DMA memory needs a future bounded
    /// payload contract and is not negotiated by this provider-neutral slice.
    OwnedUntilDownstreamRelease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenAppSrcPlan {
    pub factory: &'static str,
    pub required_runtime_capability: RuntimeCapability,
    pub is_live: bool,
    pub time_format: bool,
    pub do_timestamp: bool,
    pub block: bool,
    pub buffer_lifetime: AppSrcBufferLifetime,
    pub frame_spec: VideoFrameSpec,
    pub queue: ScreenCaptureQueuePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedScreenCapture {
    source: PlatformScreenSource,
    capabilities: ScreenSourceCapabilities,
    catalog: ScreenTargetSnapshot,
    request: ScreenCaptureRequest,
    ingress: ScreenAppSrcPlan,
}

impl NegotiatedScreenCapture {
    #[must_use]
    pub const fn source(&self) -> PlatformScreenSource {
        self.source
    }

    #[must_use]
    pub const fn request(&self) -> &ScreenCaptureRequest {
        &self.request
    }

    #[must_use]
    pub const fn ingress(&self) -> ScreenAppSrcPlan {
        self.ingress
    }

    #[must_use]
    pub const fn capabilities(&self) -> &ScreenSourceCapabilities {
        &self.capabilities
    }

    #[must_use]
    pub const fn catalog(&self) -> &ScreenTargetSnapshot {
        &self.catalog
    }

    /// Prevents a cached plan from being used after an adapter's advertised
    /// capability set changed (for example after a portal/session restart).
    pub fn validate_source(
        &self,
        capabilities: &ScreenSourceCapabilities,
    ) -> Result<(), ScreenCaptureError> {
        if &self.capabilities != capabilities {
            return Err(ScreenCaptureError::SourceCapabilitiesChanged);
        }
        Ok(())
    }
}

fn validate_catalog_binding(
    capabilities: &ScreenSourceCapabilities,
    catalog: &ScreenTargetSnapshot,
) -> Result<(), ScreenCaptureError> {
    if capabilities.source_instance() != catalog.source_instance()
        || capabilities.topology_generation() != catalog.generation()
    {
        return Err(ScreenCaptureError::TargetCatalogBindingMismatch);
    }
    Ok(())
}

fn validate_request_catalog(
    request: &ScreenCaptureRequest,
    catalog: &ScreenTargetSnapshot,
) -> Result<(), ScreenCaptureError> {
    let selected = catalog
        .find_binding(request.target().binding())
        .ok_or(ScreenCaptureError::UnknownTargetBinding)?;
    if selected != request.target() {
        return Err(ScreenCaptureError::ForgedTargetDescriptor);
    }
    for binding in request.excluded_windows() {
        let target = catalog
            .find_binding(*binding)
            .ok_or(ScreenCaptureError::UnknownTargetBinding)?;
        if target.kind() != ScreenTargetKind::Window {
            return Err(ScreenCaptureError::InvalidWindowExclusion);
        }
    }
    Ok(())
}

/// Performs exact capability negotiation. This function never downgrades a
/// requested cursor mode, exclusion promise, recovery behavior, pixel format,
/// color space, or memory type.
pub fn negotiate_screen_capture(
    capabilities: &ScreenSourceCapabilities,
    catalog: &ScreenTargetSnapshot,
    request: ScreenCaptureRequest,
) -> Result<NegotiatedScreenCapture, ScreenCaptureError> {
    capabilities.validate_for_platform(ScreenCapturePlatform::current())?;
    validate_catalog_binding(capabilities, catalog)?;
    validate_request_catalog(&request, catalog)?;
    let spec = &capabilities.spec;
    let output = request.output();
    if !spec.permission_preflight || !spec.bounded_appsrc_ingress {
        return Err(ScreenCaptureError::RequiredCapabilityUnavailable);
    }
    if !spec.targets.contains(request.target().kind()) {
        return Err(ScreenCaptureError::UnsupportedTargetKind);
    }
    if !spec.cursor_modes.contains(request.cursor().mode())
        || (request.cursor().include_image_revision() && !spec.cursor_image_metadata)
        || (request.cursor().include_clicks() && !spec.cursor_click_metadata)
        || (request.cursor().mode() == CursorCaptureMode::Metadata
            && match request.target().kind() {
                ScreenTargetKind::Window => !spec.cursor_frame_physical_coordinates,
                ScreenTargetKind::Display | ScreenTargetKind::Region => {
                    !spec.cursor_frame_physical_coordinates
                        && !spec.cursor_desktop_logical_coordinates
                }
            })
    {
        return Err(ScreenCaptureError::UnsupportedCursorPolicy);
    }
    let mut supported_profile = false;
    // The sealed payload contract currently authenticates complete owned CPU
    // allocations only. Do not negotiate native-memory profiles until their
    // lease size/lifetime can be represented without unsafe or caller-declared
    // sidecar accounting.
    if output.memory != FrameMemory::Cpu {
        return Err(ScreenCaptureError::UnsupportedFrameSpec);
    }
    for profile in spec.frame_profiles.iter().copied() {
        supported_profile |= profile.supports(output)?;
    }
    if !supported_profile {
        return Err(ScreenCaptureError::UnsupportedFrameSpec);
    }
    if !request.excluded_windows().is_empty()
        && (!spec.window_exclusion
            || request.excluded_windows().len() > usize::from(spec.max_excluded_windows))
    {
        return Err(ScreenCaptureError::UnsupportedWindowExclusion);
    }
    if matches!(
        request.recovery(),
        TargetRecoveryPolicy::ResumeSameTarget { .. }
    ) && (!spec.topology_events || !spec.target_recovery)
    {
        return Err(ScreenCaptureError::UnsupportedRecoveryPolicy);
    }
    if !spec.protected_content_events {
        return Err(ScreenCaptureError::ProtectedContentSignalUnavailable);
    }

    Ok(NegotiatedScreenCapture {
        source: spec.source,
        capabilities: capabilities.clone(),
        catalog: catalog.clone(),
        ingress: ScreenAppSrcPlan {
            factory: "appsrc",
            required_runtime_capability: RuntimeCapability::AppSourceBridge,
            is_live: true,
            time_format: true,
            // Platform timestamps are normalized before ingress; allowing
            // appsrc to stamp again would destroy cross-source timing.
            do_timestamp: false,
            // Backpressure happens in BoundedScreenFrameQueue, never inside an
            // OS callback or appsrc push.
            block: false,
            buffer_lifetime: AppSrcBufferLifetime::OwnedUntilDownstreamRelease,
            frame_spec: output,
            queue: request.queue(),
        },
        request,
    })
}

/// Normalized frame timing, identity, allocation size, and cursor metadata.
/// The separately owned CPU payload is never included in `Debug`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenFrameEnvelope {
    pub stream: ScreenStreamStamp,
    pub sequence: u64,
    pub timestamp: FrameTimestamp,
    pub spec: VideoFrameSpec,
    pub retained_bytes: u64,
    pub cursor: Option<CursorFrameMetadata>,
}

pub(crate) mod screen_payload_seal {
    pub trait Sealed {}
}

/// Owned frame storage whose complete retained allocation is visible to the
/// capture queue's accounting.
///
/// The trait is sealed: platform sources may use an exact-capacity `Vec<u8>`
/// or `Box<[u8]>`, but cannot hide sidecar allocations behind a declared byte
/// count. A vector with spare capacity is rejected because that capacity is
/// retained for as long as the frame payload.
///
/// ```compile_fail
/// use frame_media::ScreenFramePayload;
///
/// struct PayloadWithSidecar {
///     pixels: Box<[u8]>,
///     sidecar: Vec<u8>,
/// }
///
/// impl AsRef<[u8]> for PayloadWithSidecar {
///     fn as_ref(&self) -> &[u8] { &self.pixels }
/// }
///
/// impl ScreenFramePayload for PayloadWithSidecar {
///     fn exact_retained_bytes(&self) -> Option<u64> {
///         u64::try_from(self.pixels.len()).ok()
///     }
/// }
/// ```
pub trait ScreenFramePayload: screen_payload_seal::Sealed + AsRef<[u8]> + Send + 'static {
    #[doc(hidden)]
    fn exact_retained_bytes(&self) -> Option<u64>;
}

impl screen_payload_seal::Sealed for Vec<u8> {}

impl ScreenFramePayload for Vec<u8> {
    fn exact_retained_bytes(&self) -> Option<u64> {
        if self.capacity() != self.len() {
            return None;
        }
        u64::try_from(self.len()).ok()
    }
}

impl screen_payload_seal::Sealed for Box<[u8]> {}

impl ScreenFramePayload for Box<[u8]> {
    fn exact_retained_bytes(&self) -> Option<u64> {
        u64::try_from(self.len()).ok()
    }
}

pub struct ScreenFrame<T> {
    stream: ScreenStreamStamp,
    sequence: u64,
    timestamp: FrameTimestamp,
    spec: VideoFrameSpec,
    retained_bytes: u64,
    cursor: Option<CursorFrameMetadata>,
    payload: T,
}

impl<T: ScreenFramePayload> ScreenFrame<T> {
    pub fn new(envelope: ScreenFrameEnvelope, payload: T) -> Result<Self, ScreenCaptureError> {
        let ScreenFrameEnvelope {
            stream,
            sequence,
            timestamp,
            spec,
            retained_bytes,
            cursor,
        } = envelope;
        spec.validate()
            .map_err(|error| ScreenCaptureError::InvalidVideoFrameSpec(Box::new(error)))?;
        if spec.memory != FrameMemory::Cpu {
            return Err(ScreenCaptureError::FramePayloadMemoryMismatch);
        }
        if sequence == 0
            || retained_bytes == 0
            || timestamp.duration_ns == 0
            || timestamp
                .pts_ns
                .checked_add(timestamp.duration_ns)
                .is_none()
        {
            return Err(ScreenCaptureError::InvalidFrameEnvelope);
        }
        if payload.exact_retained_bytes() != Some(retained_bytes) {
            return Err(ScreenCaptureError::FramePayloadAccountingMismatch);
        }
        if let Some(cursor) = cursor
            && cursor.visible
            && (cursor.frame_x >= spec.width || cursor.frame_y >= spec.height)
        {
            return Err(ScreenCaptureError::InvalidCursorMetadata);
        }
        Ok(Self {
            stream,
            sequence,
            timestamp,
            spec,
            retained_bytes,
            cursor,
            payload,
        })
    }
}

impl<T> ScreenFrame<T> {
    #[must_use]
    pub const fn stream(&self) -> ScreenStreamStamp {
        self.stream
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
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
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    #[must_use]
    pub const fn cursor(&self) -> Option<CursorFrameMetadata> {
        self.cursor
    }

    #[must_use]
    pub fn into_payload(self) -> T {
        self.payload
    }

    pub(crate) fn force_discontinuity(&mut self) {
        self.timestamp.discontinuity = true;
    }
}

impl<T> fmt::Debug for ScreenFrame<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenFrame")
            .field("stream", &self.stream)
            .field("sequence", &self.sequence)
            .field("timestamp", &self.timestamp)
            .field("spec", &self.spec)
            .field("retained_bytes", &self.retained_bytes)
            .field("cursor", &self.cursor)
            .field("payload", &"<redacted>")
            .finish()
    }
}

struct QueuedScreenFrame<T> {
    enqueued_ns: u64,
    frame: ScreenFrame<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenCaptureQueueDiagnostics {
    pub queued_frames: u16,
    pub queued_bytes: u64,
    pub peak_frames: u16,
    pub peak_bytes: u64,
    pub accepted: u64,
    pub dropped_newest: u64,
    pub dropped_oldest: u64,
    pub dropped_expired: u64,
    pub dropped_oversized: u64,
    pub cancellation_drains: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenQueuePushOutcome {
    Accepted,
    AcceptedAfterDropping { frames: u16, bytes: u64 },
    DroppedNewest,
    DroppedOversized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenQueueDrainReport {
    pub frames: u16,
    pub bytes: u64,
}

#[derive(Debug)]
pub enum ScreenQueuePopOutcome<T> {
    Frame(ScreenFrame<T>),
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenFrameAdmission {
    pub retained_bytes: u64,
    pub duration_ns: u64,
}

/// Single-owner, non-blocking queue between a native capture callback and the
/// appsrc owner thread. Both frame count and retained CPU allocation are bounded.
struct BoundedScreenFrameQueue<T> {
    policy: ScreenCaptureQueuePolicy,
    expected_spec: VideoFrameSpec,
    capture_epoch: CaptureEpoch,
    target_epoch: ScreenTargetEpoch,
    expected_stream: Option<ScreenStreamStamp>,
    frames: VecDeque<QueuedScreenFrame<T>>,
    retained_bytes: u64,
    last_now_ns: Option<u64>,
    last_sequence: Option<u64>,
    last_timestamp_end_ns: Option<u64>,
    diagnostics: ScreenCaptureQueueDiagnostics,
}

impl<T> fmt::Debug for BoundedScreenFrameQueue<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedScreenFrameQueue")
            .field("policy", &self.policy)
            .field("expected_spec", &self.expected_spec)
            .field("capture_epoch", &self.capture_epoch)
            .field("target_epoch", &self.target_epoch)
            .field("stream_active", &self.expected_stream.is_some())
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

impl<T> BoundedScreenFrameQueue<T> {
    fn new(
        policy: ScreenCaptureQueuePolicy,
        expected_spec: VideoFrameSpec,
        capture_epoch: CaptureEpoch,
        target_epoch: ScreenTargetEpoch,
    ) -> Result<Self, ScreenCaptureError> {
        expected_spec
            .validate()
            .map_err(|error| ScreenCaptureError::InvalidVideoFrameSpec(Box::new(error)))?;
        Ok(Self {
            policy,
            expected_spec,
            capture_epoch,
            target_epoch,
            expected_stream: None,
            frames: VecDeque::with_capacity(usize::from(policy.max_frames)),
            retained_bytes: 0,
            last_now_ns: None,
            last_sequence: None,
            last_timestamp_end_ns: None,
            diagnostics: ScreenCaptureQueueDiagnostics::default(),
        })
    }

    fn try_push(
        &mut self,
        frame: ScreenFrame<T>,
        now_ns: u64,
    ) -> Result<ScreenQueuePushOutcome, ScreenCaptureError>
    where
        T: ScreenFramePayload,
    {
        // Re-authenticate allocation accounting at the queue boundary. This
        // is intentionally independent of `ScreenFrame::new`: the retained
        // lease changes authority here and queue bounds must not trust only a
        // caller-supplied envelope.
        if frame.payload.exact_retained_bytes() != Some(frame.retained_bytes) {
            return Err(ScreenCaptureError::FramePayloadAccountingMismatch);
        }
        if frame.stream.capture_epoch != self.capture_epoch {
            return Err(ScreenCaptureError::CaptureEpochMismatch);
        }
        if frame.stream.target.target_epoch() != self.target_epoch {
            return Err(ScreenCaptureError::TargetEpochMismatch);
        }
        if self.expected_stream != Some(frame.stream) {
            return Err(ScreenCaptureError::StreamIdentityMismatch);
        }
        self.observe_clock(now_ns)?;
        self.expire(now_ns)?;
        if frame.spec != self.expected_spec {
            return Err(ScreenCaptureError::FrameSpecChangedWithoutNegotiation);
        }
        if self
            .last_sequence
            .is_some_and(|sequence| frame.sequence <= sequence)
        {
            return Err(ScreenCaptureError::NonMonotonicFrameSequence);
        }
        if !frame.timestamp.discontinuity
            && self
                .last_timestamp_end_ns
                .is_some_and(|end| frame.timestamp.pts_ns < end)
        {
            return Err(ScreenCaptureError::NonMonotonicFrameTimestamp);
        }
        self.last_sequence = Some(frame.sequence);
        self.last_timestamp_end_ns = Some(frame.timestamp.end_ns());

        if frame.retained_bytes > self.policy.max_bytes {
            self.diagnostics.dropped_oversized =
                self.diagnostics.dropped_oversized.saturating_add(1);
            return Ok(ScreenQueuePushOutcome::DroppedOversized);
        }

        let would_exceed = |frames: usize, bytes: u64| {
            frames >= usize::from(self.policy.max_frames)
                || bytes
                    .checked_add(frame.retained_bytes)
                    .is_none_or(|total| total > self.policy.max_bytes)
        };
        if would_exceed(self.frames.len(), self.retained_bytes)
            && self.policy.overflow == CaptureQueueOverflow::DropNewest
        {
            self.diagnostics.dropped_newest = self.diagnostics.dropped_newest.saturating_add(1);
            return Ok(ScreenQueuePushOutcome::DroppedNewest);
        }

        let mut dropped_frames = 0_u16;
        let mut dropped_bytes = 0_u64;
        while would_exceed(self.frames.len(), self.retained_bytes) {
            let Some(dropped) = self.frames.pop_front() else {
                return Err(ScreenCaptureError::QueueAccountingCorrupt);
            };
            self.retained_bytes = self
                .retained_bytes
                .checked_sub(dropped.frame.retained_bytes)
                .ok_or(ScreenCaptureError::QueueAccountingCorrupt)?;
            dropped_frames = dropped_frames.saturating_add(1);
            dropped_bytes = dropped_bytes.saturating_add(dropped.frame.retained_bytes);
            self.diagnostics.dropped_oldest = self.diagnostics.dropped_oldest.saturating_add(1);
        }

        self.retained_bytes = self
            .retained_bytes
            .checked_add(frame.retained_bytes)
            .ok_or(ScreenCaptureError::QueueAccountingCorrupt)?;
        self.frames.push_back(QueuedScreenFrame {
            enqueued_ns: now_ns,
            frame,
        });
        self.diagnostics.accepted = self.diagnostics.accepted.saturating_add(1);
        self.refresh_diagnostics();
        if dropped_frames == 0 {
            Ok(ScreenQueuePushOutcome::Accepted)
        } else {
            Ok(ScreenQueuePushOutcome::AcceptedAfterDropping {
                frames: dropped_frames,
                bytes: dropped_bytes,
            })
        }
    }

    fn try_pop(&mut self, now_ns: u64) -> Result<ScreenQueuePopOutcome<T>, ScreenCaptureError> {
        self.observe_clock(now_ns)?;
        self.expire(now_ns)?;
        let Some(queued) = self.frames.pop_front() else {
            self.refresh_diagnostics();
            return Ok(ScreenQueuePopOutcome::Empty);
        };
        self.retained_bytes = self
            .retained_bytes
            .checked_sub(queued.frame.retained_bytes)
            .ok_or(ScreenCaptureError::QueueAccountingCorrupt)?;
        self.refresh_diagnostics();
        Ok(ScreenQueuePopOutcome::Frame(queued.frame))
    }

    fn peek_admission(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<ScreenFrameAdmission>, ScreenCaptureError> {
        self.observe_clock(now_ns)?;
        self.expire(now_ns)?;
        Ok(self.frames.front().map(|queued| ScreenFrameAdmission {
            retained_bytes: queued.frame.retained_bytes,
            duration_ns: queued.frame.timestamp.duration_ns,
        }))
    }

    fn record_cancellation(&mut self) {
        self.diagnostics.cancellation_drains =
            self.diagnostics.cancellation_drains.saturating_add(1);
    }

    /// Releases every queued lease and resets the clock/sequence envelope.
    /// The next epoch must begin strictly after the previous capture epoch and
    /// may restart source frame sequence numbering at one.
    fn reset_for_epoch(
        &mut self,
        capture_epoch: CaptureEpoch,
        target_epoch: ScreenTargetEpoch,
    ) -> Result<ScreenQueueDrainReport, ScreenCaptureError> {
        if capture_epoch <= self.capture_epoch {
            return Err(ScreenCaptureError::NonMonotonicCaptureEpoch);
        }
        let report = self.drain_and_reset_tracking();
        self.capture_epoch = capture_epoch;
        self.target_epoch = target_epoch;
        self.expected_stream = None;
        Ok(report)
    }

    fn activate_stream(&mut self, stream: ScreenStreamStamp) -> Result<(), ScreenCaptureError> {
        if self.expected_stream.is_some()
            || stream.capture_epoch != self.capture_epoch
            || stream.target.target_epoch() != self.target_epoch
        {
            return Err(ScreenCaptureError::StreamIdentityMismatch);
        }
        self.expected_stream = Some(stream);
        Ok(())
    }

    #[must_use]
    const fn diagnostics(&self) -> ScreenCaptureQueueDiagnostics {
        self.diagnostics
    }

    fn drain_and_reset_tracking(&mut self) -> ScreenQueueDrainReport {
        let report = ScreenQueueDrainReport {
            frames: u16::try_from(self.frames.len()).unwrap_or(u16::MAX),
            bytes: self.retained_bytes,
        };
        self.frames.clear();
        self.retained_bytes = 0;
        self.last_now_ns = None;
        self.last_sequence = None;
        self.last_timestamp_end_ns = None;
        self.refresh_diagnostics();
        report
    }

    fn observe_clock(&mut self, now_ns: u64) -> Result<(), ScreenCaptureError> {
        if self.last_now_ns.is_some_and(|last| now_ns < last) {
            return Err(ScreenCaptureError::QueueClockMovedBackwards);
        }
        self.last_now_ns = Some(now_ns);
        Ok(())
    }

    fn expire(&mut self, now_ns: u64) -> Result<(), ScreenCaptureError> {
        while self.frames.front().is_some_and(|queued| {
            now_ns.saturating_sub(queued.enqueued_ns) >= self.policy.max_age_ns
        }) {
            if let Some(expired) = self.frames.pop_front() {
                self.retained_bytes = self
                    .retained_bytes
                    .checked_sub(expired.frame.retained_bytes)
                    .ok_or(ScreenCaptureError::QueueAccountingCorrupt)?;
                self.diagnostics.dropped_expired =
                    self.diagnostics.dropped_expired.saturating_add(1);
            }
        }
        self.refresh_diagnostics();
        Ok(())
    }

    fn refresh_diagnostics(&mut self) {
        self.diagnostics.queued_frames = u16::try_from(self.frames.len()).unwrap_or(u16::MAX);
        self.diagnostics.queued_bytes = self.retained_bytes;
        self.diagnostics.peak_frames = self
            .diagnostics
            .peak_frames
            .max(self.diagnostics.queued_frames);
        self.diagnostics.peak_bytes = self
            .diagnostics
            .peak_bytes
            .max(self.diagnostics.queued_bytes);
    }
}

/// Cooperative operation deadline passed to every platform-source call.
pub struct ScreenOperationBudget<'a> {
    cancellation: &'a CancellationToken,
    deadline: Instant,
}

impl fmt::Debug for ScreenOperationBudget<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenOperationBudget")
            .field("cancelled", &self.cancellation.is_cancelled())
            .field("remaining", &self.remaining())
            .finish()
    }
}

impl<'a> ScreenOperationBudget<'a> {
    pub fn new(
        cancellation: &'a CancellationToken,
        timeout: Duration,
    ) -> Result<Self, ScreenCaptureError> {
        if timeout.is_zero() || timeout > MAX_CAPTURE_OPERATION_TIMEOUT {
            return Err(ScreenCaptureError::InvalidOperationTimeout);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(ScreenCaptureError::InvalidOperationTimeout)?;
        Ok(Self {
            cancellation,
            deadline,
        })
    }

    pub fn check(&self) -> Result<(), ScreenSourceFailure> {
        if self.cancellation.is_cancelled() {
            return Err(ScreenSourceFailure::new(
                ScreenSourceFailureCode::Cancelled,
                false,
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(ScreenSourceFailure::new(
                ScreenSourceFailureCode::DeadlineExceeded,
                true,
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    #[must_use]
    pub const fn cancellation(&self) -> &CancellationToken {
        self.cancellation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetLossReason {
    DisplayDisconnected,
    WindowClosed,
    WindowMinimized,
    AccessRevoked,
    SourceUnavailable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ScreenTopologyStamp {
    source_instance: ScreenSourceInstanceId,
    generation: u64,
    sequence: u64,
}

impl ScreenTopologyStamp {
    pub fn new(
        source_instance: ScreenSourceInstanceId,
        generation: u64,
        sequence: u64,
    ) -> Result<Self, ScreenCaptureError> {
        if generation == 0 || sequence == 0 {
            return Err(ScreenCaptureError::InvalidTopologyEvent);
        }
        Ok(Self {
            source_instance,
            generation,
            sequence,
        })
    }

    #[must_use]
    pub const fn source_instance(self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

impl fmt::Debug for ScreenTopologyStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenTopologyStamp")
            .field("source_instance", &self.source_instance)
            .field("generation", &"<redacted>")
            .field("sequence", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
enum ScreenTargetChangeKind {
    Added(ScreenTargetDescriptor),
    Removed {
        target: ScreenTargetBinding,
        reason: TargetLossReason,
    },
    Reconfigured(ScreenTargetDescriptor),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScreenTargetChange {
    stamp: ScreenTopologyStamp,
    kind: ScreenTargetChangeKind,
    capabilities: ScreenSourceCapabilities,
    catalog: ScreenTargetSnapshot,
}

impl ScreenTargetChange {
    pub fn added(
        stamp: ScreenTopologyStamp,
        target: ScreenTargetDescriptor,
        capabilities: ScreenSourceCapabilities,
        catalog: ScreenTargetSnapshot,
    ) -> Result<Self, ScreenCaptureError> {
        validate_changed_target(stamp, &target)?;
        validate_topology_snapshot(stamp, &capabilities, &catalog)?;
        if catalog.find_binding(target.binding()) != Some(&target) {
            return Err(ScreenCaptureError::ForgedTargetDescriptor);
        }
        Ok(Self {
            stamp,
            kind: ScreenTargetChangeKind::Added(target),
            capabilities,
            catalog,
        })
    }

    pub fn removed(
        stamp: ScreenTopologyStamp,
        target: ScreenTargetBinding,
        reason: TargetLossReason,
        capabilities: ScreenSourceCapabilities,
        catalog: ScreenTargetSnapshot,
    ) -> Result<Self, ScreenCaptureError> {
        if stamp.source_instance != target.source_instance() {
            return Err(ScreenCaptureError::CrossSourceEvent);
        }
        validate_topology_snapshot(stamp, &capabilities, &catalog)?;
        Ok(Self {
            stamp,
            kind: ScreenTargetChangeKind::Removed { target, reason },
            capabilities,
            catalog,
        })
    }

    pub fn reconfigured(
        stamp: ScreenTopologyStamp,
        target: ScreenTargetDescriptor,
        capabilities: ScreenSourceCapabilities,
        catalog: ScreenTargetSnapshot,
    ) -> Result<Self, ScreenCaptureError> {
        validate_changed_target(stamp, &target)?;
        validate_topology_snapshot(stamp, &capabilities, &catalog)?;
        if catalog.find_binding(target.binding()) != Some(&target) {
            return Err(ScreenCaptureError::ForgedTargetDescriptor);
        }
        Ok(Self {
            stamp,
            kind: ScreenTargetChangeKind::Reconfigured(target),
            capabilities,
            catalog,
        })
    }

    #[must_use]
    pub const fn stamp(&self) -> ScreenTopologyStamp {
        self.stamp
    }

    #[must_use]
    pub const fn target_id(&self) -> ScreenTargetId {
        match &self.kind {
            ScreenTargetChangeKind::Added(target)
            | ScreenTargetChangeKind::Reconfigured(target) => target.id(),
            ScreenTargetChangeKind::Removed { target, .. } => target.id(),
        }
    }
}

fn validate_topology_snapshot(
    stamp: ScreenTopologyStamp,
    capabilities: &ScreenSourceCapabilities,
    catalog: &ScreenTargetSnapshot,
) -> Result<(), ScreenCaptureError> {
    if capabilities.source_instance() != stamp.source_instance
        || capabilities.topology_generation() != stamp.generation
        || catalog.source_instance() != stamp.source_instance
        || catalog.generation() != stamp.generation
    {
        return Err(ScreenCaptureError::TargetCatalogBindingMismatch);
    }
    Ok(())
}

fn validate_changed_target(
    stamp: ScreenTopologyStamp,
    target: &ScreenTargetDescriptor,
) -> Result<(), ScreenCaptureError> {
    if stamp.source_instance != target.binding().source_instance() {
        return Err(ScreenCaptureError::CrossSourceEvent);
    }
    if stamp.generation != target.binding().topology_generation() {
        return Err(ScreenCaptureError::TargetCatalogBindingMismatch);
    }
    Ok(())
}

impl fmt::Debug for ScreenTargetChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.kind {
            ScreenTargetChangeKind::Added(_) => "Added",
            ScreenTargetChangeKind::Removed { .. } => "Removed",
            ScreenTargetChangeKind::Reconfigured(_) => "Reconfigured",
        };
        formatter
            .debug_struct("ScreenTargetChange")
            .field("stamp", &self.stamp)
            .field("kind", &kind)
            .field("target", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenOperationKind {
    Start,
    Reconfigure,
    Stop,
}

/// Live-validated, non-cloneable operation ticket. Only the library-owned
/// action executor can construct this value, immediately before dispatch to a
/// source trait method.
pub struct ScreenOperationTicket {
    kind: ScreenOperationKind,
    operation_id: ScreenOperationId,
    session_binding: ScreenSourceSessionBinding,
    stream: ScreenStreamStamp,
    predecessor_stream: Option<ScreenStreamStamp>,
    catalog_generation: u64,
    negotiated: NegotiatedScreenCapture,
}

impl ScreenOperationTicket {
    #[must_use]
    pub const fn kind(&self) -> ScreenOperationKind {
        self.kind
    }

    #[must_use]
    pub const fn operation_id(&self) -> ScreenOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn session_binding(&self) -> ScreenSourceSessionBinding {
        self.session_binding
    }

    #[must_use]
    pub const fn stream(&self) -> ScreenStreamStamp {
        self.stream
    }

    /// Previously acknowledged stream that may still own native resources
    /// when this ticket supersedes an unacknowledged reconfiguration.
    #[must_use]
    pub const fn predecessor_stream(&self) -> Option<ScreenStreamStamp> {
        self.predecessor_stream
    }

    #[must_use]
    pub const fn catalog_generation(&self) -> u64 {
        self.catalog_generation
    }

    #[must_use]
    pub const fn negotiated(&self) -> &NegotiatedScreenCapture {
        &self.negotiated
    }

    #[must_use]
    pub fn failure(&self, code: ScreenSourceFailureCode, retryable: bool) -> ScreenSourceFailure {
        ScreenSourceFailure::for_operation(self.operation_id, self.stream, code, retryable)
    }
}

impl fmt::Debug for ScreenOperationTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenOperationTicket")
            .field("kind", &self.kind)
            .field("operation_id", &self.operation_id)
            .field("session_binding", &self.session_binding)
            .field("stream", &self.stream)
            .field("predecessor_stream", &self.predecessor_stream)
            .field("catalog_generation", &"<redacted>")
            .finish()
    }
}

#[derive(PartialEq, Eq)]
struct ScreenOperationRequest {
    kind: ScreenOperationKind,
    operation_id: ScreenOperationId,
    session_binding: ScreenSourceSessionBinding,
    stream: ScreenStreamStamp,
    predecessor_stream: Option<ScreenStreamStamp>,
    expected: Box<NegotiatedScreenCapture>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ScreenOperationAck {
    kind: ScreenOperationKind,
    operation_id: ScreenOperationId,
    session_binding: ScreenSourceSessionBinding,
    stream: ScreenStreamStamp,
}

impl ScreenOperationAck {
    #[must_use]
    pub const fn kind(self) -> ScreenOperationKind {
        self.kind
    }

    #[must_use]
    pub const fn operation_id(self) -> ScreenOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn stream(self) -> ScreenStreamStamp {
        self.stream
    }
}

impl fmt::Debug for ScreenOperationAck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenOperationAck")
            .field("kind", &self.kind)
            .field("operation_id", &self.operation_id)
            .field("session_binding", &self.session_binding)
            .field("stream", &self.stream)
            .finish()
    }
}

pub enum ScreenSourceEvent<FramePayload, CursorImagePayload> {
    Frame(ScreenFrame<FramePayload>),
    CursorImage(ScreenCursorImage<CursorImagePayload>),
    PermissionChanged(ScreenPermissionObservation),
    TargetChanged(Box<ScreenTargetChange>),
    Sleep(ScreenControlStamp),
    Wake(ScreenControlStamp),
    ProtectedContentDetected(ScreenControlStamp),
    ProtectedContentCleared(ScreenControlStamp),
    Failure(ScreenSourceFailure),
}

impl<FramePayload, CursorImagePayload> fmt::Debug
    for ScreenSourceEvent<FramePayload, CursorImagePayload>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(frame) => formatter.debug_tuple("Frame").field(frame).finish(),
            Self::CursorImage(image) => formatter.debug_tuple("CursorImage").field(image).finish(),
            Self::PermissionChanged(permission) => formatter
                .debug_tuple("PermissionChanged")
                .field(permission)
                .finish(),
            Self::TargetChanged(change) => formatter
                .debug_tuple("TargetChanged")
                .field(change)
                .finish(),
            Self::Sleep(stamp) => formatter.debug_tuple("Sleep").field(stamp).finish(),
            Self::Wake(stamp) => formatter.debug_tuple("Wake").field(stamp).finish(),
            Self::ProtectedContentDetected(stamp) => formatter
                .debug_tuple("ProtectedContentDetected")
                .field(stamp)
                .finish(),
            Self::ProtectedContentCleared(stamp) => formatter
                .debug_tuple("ProtectedContentCleared")
                .field(stamp)
                .finish(),
            Self::Failure(failure) => formatter.debug_tuple("Failure").field(failure).finish(),
        }
    }
}

pub type ScreenSourcePollResult<FramePayload, CursorImagePayload> =
    Result<Option<ScreenSourceEvent<FramePayload, CursorImagePayload>>, ScreenSourceFailure>;

/// Opaque owner-stamped result of one bound adapter poll. Its private fields
/// prevent callers from relabeling a raw event as another session's event.
pub struct ScreenSourceEventEnvelope<FramePayload, CursorImagePayload> {
    owner: ScreenSourceSessionBinding,
    event: ScreenSourceEvent<FramePayload, CursorImagePayload>,
}

impl<FramePayload, CursorImagePayload> fmt::Debug
    for ScreenSourceEventEnvelope<FramePayload, CursorImagePayload>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenSourceEventEnvelope")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// Opaque owner-stamped source failure. Only bound native-call and operation
/// execution paths can construct one; callers may inspect its low-cardinality
/// fields but cannot change its owner or inner correlation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("owner-bound {failure}")]
pub struct ScreenSourceFailureEnvelope {
    owner: ScreenSourceSessionBinding,
    failure: ScreenSourceFailure,
}

impl ScreenSourceFailureEnvelope {
    #[must_use]
    pub const fn code(&self) -> ScreenSourceFailureCode {
        self.failure.code()
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.failure.retryable()
    }

    #[must_use]
    pub const fn operation_id(&self) -> Option<ScreenOperationId> {
        self.failure.operation_id()
    }

    #[must_use]
    pub fn stream(&self) -> Option<ScreenStreamStamp> {
        self.failure.stream()
    }
}

pub type ScreenSourceEnvelopePollResult<FramePayload, CursorImagePayload> = Result<
    Option<ScreenSourceEventEnvelope<FramePayload, CursorImagePayload>>,
    ScreenSourceFailureEnvelope,
>;

struct ScreenPermissionObservationEnvelope {
    owner: ScreenSourceSessionBinding,
    observation: ScreenPermissionObservation,
}

/// Adapter boundary for ScreenCaptureKit, Windows Graphics Capture, PipeWire
/// portal, and explicitly approved X11 implementations. Calls are synchronous
/// but must cooperate with the supplied cancellation/deadline budget.
pub trait ScreenCaptureSource {
    type FramePayload: ScreenFramePayload;
    type CursorImagePayload: ScreenFramePayload;

    /// Pure identity metadata used to construct the pre-negotiation binding.
    /// This getter must not call a platform API.
    fn source_instance(&self) -> ScreenSourceInstanceId;

    /// Returns the immutable session owner previously accepted by
    /// `bind_session`. Reading this value must not call a platform API.
    fn session_binding(&self) -> Option<ScreenSourceSessionBinding>;

    /// Claims this source object for exactly one session. This is a pure owner
    /// guard and must run before any native dispatch. Repeating the same claim
    /// is idempotent; a different claim must be rejected without touching the
    /// native backend.
    fn bind_session(&mut self, ticket: ScreenSourceSessionTicket)
    -> Result<(), ScreenCaptureError>;

    fn capabilities<'a>(
        &'a self,
        ticket: &ScreenSourceCallTicket<'_>,
    ) -> &'a ScreenSourceCapabilities;

    fn preflight(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure>;

    /// Performs the OS-owned prompt only after the state machine emits
    /// `RequestPermission`; enumeration and preflight must never prompt.
    fn request_permission(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservation, ScreenSourceFailure>;

    fn enumerate_targets(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenTargetSnapshot, ScreenSourceFailure>;

    fn start(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure>;

    fn reconfigure(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure>;

    fn poll_event(
        &mut self,
        ticket: &ScreenSourceCallTicket<'_>,
        budget: &ScreenOperationBudget<'_>,
    ) -> ScreenSourcePollResult<Self::FramePayload, Self::CursorImagePayload>;

    /// Stops all and only native capture resources owned by the ticket's
    /// session binding before returning success. `stream` identifies the
    /// invalidated operation and `predecessor_stream` identifies the previously
    /// acknowledged native stream, if any. Implementations must quiesce either
    /// or both when present, remain idempotent, and never stop another binding.
    fn stop(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure>;
}

/// The only safe native-call handle. Binding happens before capability
/// discovery or enumeration, so negotiation and the eventual session share
/// one collision-resistant owner from bootstrap through teardown.
///
/// The adapter is intentionally exposed only through a shared reference. A
/// safe caller cannot replace the adapter while retaining this wrapper's
/// binding because the wrapper implements neither `DerefMut` nor a mutable
/// adapter accessor:
///
/// ```compile_fail
/// use frame_media::{BoundScreenCaptureSource, ScreenCaptureSource};
///
/// fn replace_adapter<S: ScreenCaptureSource>(
///     bound: &mut BoundScreenCaptureSource<S>,
///     replacement: S,
/// ) {
///     let _old = std::mem::replace(&mut **bound, replacement);
/// }
/// ```
pub struct BoundScreenCaptureSource<S: ScreenCaptureSource> {
    source: S,
    binding: ScreenSourceSessionBinding,
}

impl<S: ScreenCaptureSource> BoundScreenCaptureSource<S> {
    pub fn new(mut source: S, session_id: ScreenSessionId) -> Result<Self, ScreenCaptureError> {
        let binding = ScreenSourceSessionBinding {
            source_instance: source.source_instance(),
            session_id,
        };
        match source.session_binding() {
            Some(current) if current != binding => {
                return Err(ScreenCaptureError::SourceSessionOwnershipMismatch);
            }
            Some(_) => {}
            None => source.bind_session(ScreenSourceSessionTicket { binding })?,
        }
        if source.session_binding() != Some(binding) {
            return Err(ScreenCaptureError::SourceSessionOwnershipMismatch);
        }
        Ok(Self { source, binding })
    }

    #[must_use]
    pub const fn binding(&self) -> ScreenSourceSessionBinding {
        self.binding
    }

    /// Bound capability discovery used before negotiation.
    #[must_use]
    pub fn capabilities(&self) -> &ScreenSourceCapabilities {
        let ticket = ScreenSourceCallTicket {
            binding: &self.binding,
        };
        self.source.capabilities(&ticket)
    }

    /// Bound target discovery used before negotiation and for live operation
    /// revalidation. Raw source implementations cannot be enumerated without
    /// the private call ticket created here.
    pub fn enumerate_targets(
        &mut self,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenTargetSnapshot, ScreenSourceFailure> {
        let ticket = ScreenSourceCallTicket {
            binding: &self.binding,
        };
        self.source.enumerate_targets(&ticket, budget)
    }

    #[must_use]
    pub const fn adapter(&self) -> &S {
        &self.source
    }

    fn preflight(
        &mut self,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservationEnvelope, ScreenSourceFailureEnvelope> {
        let ticket = ScreenSourceCallTicket {
            binding: &self.binding,
        };
        self.source
            .preflight(&ticket, budget)
            .map(|observation| ScreenPermissionObservationEnvelope {
                owner: self.binding,
                observation,
            })
            .map_err(|failure| self.wrap_failure(failure))
    }

    fn request_permission(
        &mut self,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenPermissionObservationEnvelope, ScreenSourceFailureEnvelope> {
        let ticket = ScreenSourceCallTicket {
            binding: &self.binding,
        };
        self.source
            .request_permission(&ticket, budget)
            .map(|observation| ScreenPermissionObservationEnvelope {
                owner: self.binding,
                observation,
            })
            .map_err(|failure| self.wrap_failure(failure))
    }

    /// Performs one owner-bound native poll. Both events and failures are
    /// sealed in private-field envelopes before leaving the wrapper.
    pub fn poll_owned_event(
        &mut self,
        budget: &ScreenOperationBudget<'_>,
    ) -> ScreenSourceEnvelopePollResult<S::FramePayload, S::CursorImagePayload> {
        let ticket = ScreenSourceCallTicket {
            binding: &self.binding,
        };
        self.source
            .poll_event(&ticket, budget)
            .map(|event| {
                event.map(|event| ScreenSourceEventEnvelope {
                    owner: self.binding,
                    event,
                })
            })
            .map_err(|failure| self.wrap_failure(failure))
    }

    fn wrap_failure(&self, failure: ScreenSourceFailure) -> ScreenSourceFailureEnvelope {
        ScreenSourceFailureEnvelope {
            owner: self.binding,
            failure,
        }
    }

    fn start(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.source.start(ticket, budget)
    }

    fn reconfigure(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.source.reconfigure(ticket, budget)
    }

    fn stop(
        &mut self,
        ticket: ScreenOperationTicket,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<(), ScreenSourceFailure> {
        self.source.stop(ticket, budget)
    }
}

impl<S: ScreenCaptureSource> fmt::Debug for BoundScreenCaptureSource<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundScreenCaptureSource")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<S: ScreenCaptureSource> std::ops::Deref for BoundScreenCaptureSource<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSourceFailureCode {
    Cancelled,
    DeadlineExceeded,
    AdapterUnavailable,
    PermissionDenied,
    PermissionRestricted,
    TargetLost,
    ProtectedContent,
    InvalidNativeFrame,
    NativeOperationFailed,
}

/// Low-cardinality source failure safe for logs. Native error text, target IDs,
/// titles, and process information remain at the platform boundary.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("screen capture source failed with {code:?}")]
pub struct ScreenSourceFailure {
    code: ScreenSourceFailureCode,
    retryable: bool,
    operation_id: Option<ScreenOperationId>,
    stream: Option<Box<ScreenStreamStamp>>,
}

impl ScreenSourceFailure {
    #[must_use]
    pub const fn new(code: ScreenSourceFailureCode, retryable: bool) -> Self {
        Self {
            code,
            retryable,
            operation_id: None,
            stream: None,
        }
    }

    fn for_operation(
        operation_id: ScreenOperationId,
        stream: ScreenStreamStamp,
        code: ScreenSourceFailureCode,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            retryable,
            operation_id: Some(operation_id),
            stream: Some(Box::new(stream)),
        }
    }

    #[must_use]
    pub const fn code(&self) -> ScreenSourceFailureCode {
        self.code
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub const fn operation_id(&self) -> Option<ScreenOperationId> {
        self.operation_id
    }

    #[must_use]
    pub fn stream(&self) -> Option<ScreenStreamStamp> {
        self.stream.as_deref().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSuspensionReason {
    TargetLost,
    Sleeping,
    ProtectedContent,
    PermissionBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSessionFailureCode {
    TargetLost,
    RecoveryExhausted,
    ContractInvalidated,
    ProtectedContent,
    Source(ScreenSourceFailureCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCapturePhase {
    AwaitingPreflight,
    AwaitingPermissionRequest,
    AwaitingPermissionResult,
    Ready,
    Starting,
    Capturing,
    Reconfiguring,
    Suspended(ScreenSuspensionReason),
    Stopping,
    Stopped,
    Cancelled,
    Failed(ScreenSessionFailureCode),
}

impl ScreenCapturePhase {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Cancelled | Self::Failed(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSourceCommand {
    None,
    Start {
        operation_id: ScreenOperationId,
        stream: ScreenStreamStamp,
    },
    Reconfigure {
        operation_id: ScreenOperationId,
        stream: ScreenStreamStamp,
    },
    Stop {
        operation_id: ScreenOperationId,
        stream: ScreenStreamStamp,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenControlCommand {
    None,
    RunPermissionPreflight,
    RequestPermission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenEpochTransition {
    session_binding: ScreenSourceSessionBinding,
    retired_capture_epoch: CaptureEpoch,
    active_capture_epoch: CaptureEpoch,
    source_instance: ScreenSourceInstanceId,
    target: ScreenTargetBinding,
}

impl ScreenEpochTransition {
    #[must_use]
    pub const fn retired_capture_epoch(self) -> CaptureEpoch {
        self.retired_capture_epoch
    }

    #[must_use]
    pub const fn active_capture_epoch(self) -> CaptureEpoch {
        self.active_capture_epoch
    }

    #[must_use]
    pub const fn source_instance(self) -> ScreenSourceInstanceId {
        self.source_instance
    }

    #[must_use]
    pub const fn target(self) -> ScreenTargetBinding {
        self.target
    }
}

/// A transition may require a source command, a permission command, and an
/// appsrc/queue flush at the same time. `ScreenCaptureIngress` applies `flush`
/// atomically to its frame queue and cursor cache before returning the
/// transition; the appsrc owner uses the same handoff for downstream state.
#[derive(PartialEq, Eq)]
pub struct ScreenSessionAction {
    owner: ScreenSourceSessionBinding,
    source: ScreenSourceCommand,
    control: ScreenControlCommand,
    flush: Option<ScreenEpochTransition>,
    activate_stream: Option<ScreenStreamStamp>,
    operation: Option<ScreenOperationRequest>,
}

impl ScreenSessionAction {
    fn none(owner: ScreenSourceSessionBinding) -> Self {
        Self {
            owner,
            source: ScreenSourceCommand::None,
            control: ScreenControlCommand::None,
            flush: None,
            activate_stream: None,
            operation: None,
        }
    }

    fn control(owner: ScreenSourceSessionBinding, control: ScreenControlCommand) -> Self {
        Self {
            owner,
            source: ScreenSourceCommand::None,
            control,
            flush: None,
            activate_stream: None,
            operation: None,
        }
    }

    #[must_use]
    pub const fn source_command(&self) -> ScreenSourceCommand {
        self.source
    }

    #[must_use]
    pub const fn control_command(&self) -> ScreenControlCommand {
        self.control
    }

    #[must_use]
    pub const fn flush(&self) -> Option<ScreenEpochTransition> {
        self.flush
    }

    #[must_use]
    pub const fn activated_stream(&self) -> Option<ScreenStreamStamp> {
        self.activate_stream
    }

    /// Performs the unavoidable execution-time capability/catalog validation,
    /// constructs the only ticket accepted by the source trait, and consumes
    /// this action's one-shot request.
    pub fn execute_source<S: ScreenCaptureSource>(
        &mut self,
        session: &ScreenCaptureSession,
        source: &mut BoundScreenCaptureSource<S>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<Option<ScreenOperationAck>, ScreenOperationExecutionError> {
        if self.owner != session.source_session_binding {
            return Err(ScreenOperationExecutionError::Contract(
                ScreenCaptureError::ActionSessionOwnershipMismatch,
            ));
        }
        if source.binding() != self.owner {
            return Err(ScreenOperationExecutionError::Contract(
                ScreenCaptureError::SourceSessionOwnershipMismatch,
            ));
        }
        let Some(request) = self.operation.take() else {
            return if self.source == ScreenSourceCommand::None {
                Ok(None)
            } else {
                Err(ScreenOperationExecutionError::TicketConsumed)
            };
        };
        if !session.matches_operation_request(&request) {
            return Err(ScreenOperationExecutionError::StaleOperationAction);
        }
        request.validate_session_identity()?;
        if request.session_binding != self.owner {
            return Err(ScreenOperationExecutionError::Contract(
                ScreenCaptureError::ActionSessionOwnershipMismatch,
            ));
        }
        if request.kind != ScreenOperationKind::Stop {
            let live_catalog = match source.enumerate_targets(budget) {
                Ok(catalog) => catalog,
                Err(failure) => {
                    return Err(ScreenOperationExecutionError::Source(
                        ScreenSourceFailureEnvelope {
                            owner: request.session_binding,
                            failure: request.bind_source_failure(failure)?,
                        },
                    ));
                }
            };
            let live_capabilities = source.capabilities().clone();
            if live_capabilities.source_instance() != request.stream.source_instance() {
                return Err(ScreenOperationExecutionError::Contract(
                    ScreenCaptureError::CrossSourceEvent,
                ));
            }
            request.validate_live(&live_capabilities, &live_catalog)?;
        }
        let ack = ScreenOperationAck {
            kind: request.kind,
            operation_id: request.operation_id,
            session_binding: request.session_binding,
            stream: request.stream,
        };
        let ticket = ScreenOperationTicket {
            kind: request.kind,
            operation_id: request.operation_id,
            session_binding: request.session_binding,
            stream: request.stream,
            predecessor_stream: request.predecessor_stream,
            catalog_generation: request.expected.catalog().generation(),
            negotiated: request.expected.as_ref().clone(),
        };
        let result = match request.kind {
            ScreenOperationKind::Start => source.start(ticket, budget),
            ScreenOperationKind::Reconfigure => source.reconfigure(ticket, budget),
            ScreenOperationKind::Stop => {
                // Session cancellation must not cancel the teardown it caused.
                // Give Stop its own bounded token/deadline so a caller may use
                // one cancellation token for polling and session lifetime.
                let teardown_cancellation = CancellationToken::new();
                let teardown_budget = ScreenOperationBudget::new(
                    &teardown_cancellation,
                    SCREEN_CAPTURE_TEARDOWN_TIMEOUT,
                )
                .map_err(ScreenOperationExecutionError::Contract)?;
                source.stop(ticket, &teardown_budget)
            }
        };
        match result {
            Ok(()) => Ok(Some(ack)),
            Err(failure) => Err(ScreenOperationExecutionError::Source(
                ScreenSourceFailureEnvelope {
                    owner: request.session_binding,
                    failure: request.bind_source_failure(failure)?,
                },
            )),
        }
    }
}

impl fmt::Debug for ScreenSessionAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenSessionAction")
            .field("owner", &self.owner)
            .field("source", &self.source)
            .field("control", &self.control)
            .field("flush", &self.flush)
            .field("activate_stream", &self.activate_stream)
            .field("operation", &self.operation.as_ref().map(|_| "<one-shot>"))
            .finish()
    }
}

impl ScreenOperationRequest {
    fn validate_session_identity(&self) -> Result<(), ScreenOperationExecutionError> {
        let stream_matches = self.session_binding.source_instance == self.stream.source_instance
            && self.session_binding.session_id == self.stream.stream.session;
        let predecessor_matches = self.predecessor_stream.is_none_or(|predecessor| {
            predecessor.source_instance == self.session_binding.source_instance
                && predecessor.stream.session == self.session_binding.session_id
        });
        if !stream_matches || !predecessor_matches {
            return Err(ScreenOperationExecutionError::Contract(
                ScreenCaptureError::StreamIdentityMismatch,
            ));
        }
        Ok(())
    }

    fn bind_source_failure(
        &self,
        failure: ScreenSourceFailure,
    ) -> Result<ScreenSourceFailure, ScreenOperationExecutionError> {
        if failure
            .operation_id()
            .is_some_and(|id| id != self.operation_id)
            || failure.stream().is_some_and(|stream| stream != self.stream)
        {
            return Err(ScreenOperationExecutionError::FailureBindingMismatch);
        }
        Ok(ScreenSourceFailure::for_operation(
            self.operation_id,
            self.stream,
            failure.code(),
            failure.retryable(),
        ))
    }

    fn validate_live(
        &self,
        capabilities: &ScreenSourceCapabilities,
        catalog: &ScreenTargetSnapshot,
    ) -> Result<(), ScreenOperationExecutionError> {
        if self.session_binding.source_instance() != self.stream.source_instance
            || capabilities.source_instance() != self.stream.source_instance
            || catalog.source_instance() != self.stream.source_instance
        {
            return Err(ScreenOperationExecutionError::Contract(
                ScreenCaptureError::CrossSourceEvent,
            ));
        }
        if self.kind != ScreenOperationKind::Stop {
            self.expected
                .validate_source(capabilities)
                .map_err(ScreenOperationExecutionError::Contract)?;
            if self.expected.catalog() != catalog {
                return Err(ScreenOperationExecutionError::Contract(
                    ScreenCaptureError::SourceCatalogChanged,
                ));
            }
            validate_request_catalog(self.expected.request(), catalog)
                .map_err(ScreenOperationExecutionError::Contract)?;
            if self.expected.request().target().binding() != self.stream.target {
                return Err(ScreenOperationExecutionError::Contract(
                    ScreenCaptureError::TargetCatalogBindingMismatch,
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScreenOperationExecutionError {
    #[error("screen operation contract validation failed")]
    Contract(#[source] ScreenCaptureError),
    #[error("screen operation source execution failed")]
    Source(#[source] ScreenSourceFailureEnvelope),
    #[error("screen operation ticket was already consumed")]
    TicketConsumed,
    #[error("screen operation action no longer matches the session's pending operation")]
    StaleOperationAction,
    #[error("screen source returned a failure bound to another operation")]
    FailureBindingMismatch,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScreenControlExecutionError {
    #[error("screen control action does not match the live session")]
    Contract(#[source] ScreenCaptureError),
    #[error("screen control source call failed; the action remains retryable")]
    Source(#[source] ScreenSourceFailureEnvelope),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenDiagnosticEventCode {
    Created,
    Preflight,
    PermissionRequest,
    PermissionChanged,
    StartRequested,
    SourceStarted,
    TargetHotplug,
    TargetLost,
    TargetRestored,
    TargetReconfigured,
    Sleep,
    Wake,
    ProtectedContent,
    ProtectedContentCleared,
    StopRequested,
    SourceStopped,
    Cancelled,
    SourceFailed,
    QueueAccepted,
    QueueDropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCaptureDiagnostics {
    pub schema_version: u16,
    pub phase: ScreenCapturePhase,
    pub source: PlatformScreenSource,
    pub target_kind: ScreenTargetKind,
    pub permission: ScreenPermissionState,
    pub capture_epoch: CaptureEpoch,
    pub target_events_observed: u32,
    pub control_events_observed: u32,
    pub recovery_attempts: u8,
    pub frames_accepted: u64,
    pub frames_dropped: u64,
    pub last_event: ScreenDiagnosticEventCode,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScreenSessionTransition {
    pub from: ScreenCapturePhase,
    pub to: ScreenCapturePhase,
    pub action: ScreenSessionAction,
}

/// Locally initiated session commands. This is deliberately the only public
/// enum accepted as freely constructible state-machine input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSessionIntent {
    RequestPermission,
    Start,
    Stop,
    Cancel,
}

/// Private state-machine events, classified by their only valid producer:
///
/// - local intent: request permission, start, stop, cancel;
/// - bound source result/observation: preflight, permission, topology,
///   sleep/wake, protected content;
/// - library operation result: operation acknowledgement or bound failure.
///
/// Public entry points below accept only the corresponding narrow type and
/// validate its opaque owner before constructing one of these variants.
#[derive(Debug)]
enum ScreenSessionEvent {
    PreflightCompleted(ScreenPermissionObservation),
    RequestPermission,
    PermissionRequestCompleted(ScreenPermissionObservation),
    PermissionChanged(ScreenPermissionObservation),
    StartRequested,
    OperationCompleted(ScreenOperationAck),
    TargetChanged(Box<ScreenTargetChange>),
    Sleep(ScreenControlStamp),
    Wake(ScreenControlStamp),
    ProtectedContentDetected(ScreenControlStamp),
    ProtectedContentCleared(ScreenControlStamp),
    StopRequested,
    Cancel,
    SourceFailed(ScreenSourceFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingScreenOperation {
    kind: ScreenOperationKind,
    operation_id: ScreenOperationId,
    stream: ScreenStreamStamp,
    predecessor_stream: Option<ScreenStreamStamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveScreenStream {
    operation_id: ScreenOperationId,
    stream: ScreenStreamStamp,
}

/// Deterministic permission, operation, stream, and recovery state machine.
/// All mutations are applied through `ScreenCaptureIngress`, which couples
/// transition flushes to queue and cursor-cache retirement.
pub struct ScreenCaptureSession {
    negotiated: NegotiatedScreenCapture,
    session_id: ScreenSessionId,
    source_session_binding: ScreenSourceSessionBinding,
    target: ScreenTargetDescriptor,
    phase: ScreenCapturePhase,
    permission: ScreenPermissionState,
    capture_epoch: CaptureEpoch,
    next_operation_sequence: u64,
    next_stream_sequence: u64,
    pending_operation: Option<PendingScreenOperation>,
    active_stream: Option<ActiveScreenStream>,
    topology_generation: u64,
    last_topology_sequence: u64,
    control_epoch: ScreenControlEpoch,
    last_control_sequence: u64,
    target_events_observed: u32,
    control_events_observed: u32,
    recovery_attempts: u8,
    desired_running: bool,
    stop_requested: bool,
    target_available: bool,
    sleeping: bool,
    protected_content_active: bool,
    fresh_preflight_required: bool,
    frames_accepted: u64,
    frames_dropped: u64,
    last_event: ScreenDiagnosticEventCode,
}

impl fmt::Debug for ScreenCaptureSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenCaptureSession")
            .field("session_id", &self.session_id)
            .field("diagnostics", &self.diagnostics())
            .field("pending_operation", &self.pending_operation)
            .field("active_stream", &self.active_stream)
            .finish_non_exhaustive()
    }
}

impl ScreenCaptureSession {
    pub fn new(
        negotiated: NegotiatedScreenCapture,
        source_session_binding: ScreenSourceSessionBinding,
    ) -> Result<Self, ScreenCaptureError> {
        if source_session_binding.source_instance != negotiated.capabilities.source_instance() {
            return Err(ScreenCaptureError::CrossSourceEvent);
        }
        let session_id = source_session_binding.session_id;
        let target = negotiated.request.target().clone();
        let topology_generation = negotiated.capabilities.topology_generation();
        let control_epoch = negotiated.capabilities.control_epoch();
        let last_control_sequence = negotiated.capabilities.control_sequence();
        Ok(Self {
            negotiated,
            session_id,
            source_session_binding,
            target,
            phase: ScreenCapturePhase::AwaitingPreflight,
            permission: ScreenPermissionState::Unchecked,
            capture_epoch: CaptureEpoch(1),
            next_operation_sequence: 0,
            next_stream_sequence: 0,
            pending_operation: None,
            active_stream: None,
            topology_generation,
            last_topology_sequence: 0,
            control_epoch,
            last_control_sequence,
            target_events_observed: 0,
            control_events_observed: 0,
            recovery_attempts: 0,
            desired_running: false,
            stop_requested: false,
            target_available: true,
            sleeping: false,
            protected_content_active: false,
            fresh_preflight_required: false,
            frames_accepted: 0,
            frames_dropped: 0,
            last_event: ScreenDiagnosticEventCode::Created,
        })
    }

    #[must_use]
    pub fn initial_action(&self) -> ScreenSessionAction {
        ScreenSessionAction::control(
            self.source_session_binding,
            ScreenControlCommand::RunPermissionPreflight,
        )
    }

    #[must_use]
    pub const fn session_id(&self) -> ScreenSessionId {
        self.session_id
    }

    #[must_use]
    pub const fn phase(&self) -> ScreenCapturePhase {
        self.phase
    }

    #[must_use]
    pub const fn target(&self) -> &ScreenTargetDescriptor {
        &self.target
    }

    #[must_use]
    pub const fn negotiated(&self) -> &NegotiatedScreenCapture {
        &self.negotiated
    }

    #[must_use]
    pub const fn capture_epoch(&self) -> CaptureEpoch {
        self.capture_epoch
    }

    #[must_use]
    pub const fn active_stream(&self) -> Option<ScreenStreamStamp> {
        match self.active_stream {
            Some(active) => Some(active.stream),
            None => None,
        }
    }

    #[must_use]
    pub const fn pending_operation_kind(&self) -> Option<ScreenOperationKind> {
        match self.pending_operation {
            Some(pending) => Some(pending.kind),
            None => None,
        }
    }

    fn pending_stop_matches(
        &self,
        operation_id: ScreenOperationId,
        stream: ScreenStreamStamp,
    ) -> bool {
        self.pending_operation.is_some_and(|pending| {
            pending.kind == ScreenOperationKind::Stop
                && pending.operation_id == operation_id
                && pending.stream == stream
        })
    }

    #[must_use]
    pub const fn diagnostics(&self) -> ScreenCaptureDiagnostics {
        ScreenCaptureDiagnostics {
            schema_version: SCREEN_CAPTURE_DIAGNOSTIC_VERSION,
            phase: self.phase,
            source: self.negotiated.source,
            target_kind: self.target.kind(),
            permission: self.permission,
            capture_epoch: self.capture_epoch,
            target_events_observed: self.target_events_observed,
            control_events_observed: self.control_events_observed,
            recovery_attempts: self.recovery_attempts,
            frames_accepted: self.frames_accepted,
            frames_dropped: self.frames_dropped,
            last_event: self.last_event,
        }
    }

    fn next_operation_id(&mut self) -> Result<ScreenOperationId, ScreenCaptureError> {
        self.next_operation_sequence = self
            .next_operation_sequence
            .checked_add(1)
            .ok_or(ScreenCaptureError::OperationSequenceExhausted)?;
        Ok(ScreenOperationId(self.next_operation_sequence))
    }

    fn next_stream_stamp(&mut self) -> Result<ScreenStreamStamp, ScreenCaptureError> {
        self.next_stream_sequence = self
            .next_stream_sequence
            .checked_add(1)
            .ok_or(ScreenCaptureError::StreamSequenceExhausted)?;
        Ok(ScreenStreamStamp {
            source_instance: self.negotiated.capabilities.source_instance(),
            target: self.target.binding(),
            stream: ScreenStreamId {
                session: self.session_id,
                sequence: self.next_stream_sequence,
            },
            capture_epoch: self.capture_epoch,
        })
    }

    fn issue_start(&mut self) -> Result<ScreenSessionAction, ScreenCaptureError> {
        if self.pending_operation.is_some() || self.active_stream.is_some() {
            return Err(ScreenCaptureError::OperationAlreadyPending);
        }
        let operation_id = self.next_operation_id()?;
        let stream = self.next_stream_stamp()?;
        self.pending_operation = Some(PendingScreenOperation {
            kind: ScreenOperationKind::Start,
            operation_id,
            stream,
            predecessor_stream: None,
        });
        Ok(self.operation_action(ScreenOperationKind::Start, operation_id, stream, None))
    }

    fn issue_reconfigure(&mut self) -> Result<ScreenSessionAction, ScreenCaptureError> {
        if self.pending_operation.is_some() || self.active_stream.is_none() {
            return Err(ScreenCaptureError::OperationAlreadyPending);
        }
        let operation_id = self.next_operation_id()?;
        let stream = self.next_stream_stamp()?;
        let predecessor_stream = self.active_stream.map(|active| active.stream);
        self.pending_operation = Some(PendingScreenOperation {
            kind: ScreenOperationKind::Reconfigure,
            operation_id,
            stream,
            predecessor_stream,
        });
        Ok(self.operation_action(
            ScreenOperationKind::Reconfigure,
            operation_id,
            stream,
            predecessor_stream,
        ))
    }

    fn issue_stop(&mut self) -> Result<ScreenSessionAction, ScreenCaptureError> {
        if self
            .pending_operation
            .is_some_and(|pending| pending.kind == ScreenOperationKind::Stop)
        {
            return Ok(ScreenSessionAction::none(self.source_session_binding));
        }
        let Some((stream, predecessor_stream)) = self
            .pending_operation
            .map(|pending| (pending.stream, pending.predecessor_stream))
            .or_else(|| self.active_stream.map(|active| (active.stream, None)))
        else {
            self.pending_operation = None;
            return Ok(ScreenSessionAction::none(self.source_session_binding));
        };
        let operation_id = self.next_operation_id()?;
        self.pending_operation = Some(PendingScreenOperation {
            kind: ScreenOperationKind::Stop,
            operation_id,
            stream,
            predecessor_stream,
        });
        Ok(self.operation_action(
            ScreenOperationKind::Stop,
            operation_id,
            stream,
            predecessor_stream,
        ))
    }

    fn retry_pending_stop(&mut self) -> Result<ScreenSessionAction, ScreenCaptureError> {
        let pending = self
            .pending_operation
            .filter(|pending| pending.kind == ScreenOperationKind::Stop)
            .ok_or(ScreenCaptureError::InvalidSessionTransition)?;
        let operation_id = self.next_operation_id()?;
        self.pending_operation = Some(PendingScreenOperation {
            kind: ScreenOperationKind::Stop,
            operation_id,
            stream: pending.stream,
            predecessor_stream: pending.predecessor_stream,
        });
        Ok(self.operation_action(
            ScreenOperationKind::Stop,
            operation_id,
            pending.stream,
            pending.predecessor_stream,
        ))
    }

    fn operation_action(
        &self,
        kind: ScreenOperationKind,
        operation_id: ScreenOperationId,
        stream: ScreenStreamStamp,
        predecessor_stream: Option<ScreenStreamStamp>,
    ) -> ScreenSessionAction {
        let source = match kind {
            ScreenOperationKind::Start => ScreenSourceCommand::Start {
                operation_id,
                stream,
            },
            ScreenOperationKind::Reconfigure => ScreenSourceCommand::Reconfigure {
                operation_id,
                stream,
            },
            ScreenOperationKind::Stop => ScreenSourceCommand::Stop {
                operation_id,
                stream,
            },
        };
        ScreenSessionAction {
            owner: self.source_session_binding,
            source,
            control: ScreenControlCommand::None,
            flush: None,
            activate_stream: None,
            operation: Some(ScreenOperationRequest {
                kind,
                operation_id,
                session_binding: self.source_session_binding,
                stream,
                predecessor_stream,
                expected: Box::new(self.negotiated.clone()),
            }),
        }
    }

    fn next_epoch(&mut self) -> Result<ScreenEpochTransition, ScreenCaptureError> {
        let retired_capture_epoch = self.capture_epoch;
        let active_capture_epoch = retired_capture_epoch.next()?;
        self.capture_epoch = active_capture_epoch;
        Ok(ScreenEpochTransition {
            session_binding: self.source_session_binding,
            retired_capture_epoch,
            active_capture_epoch,
            source_instance: self.negotiated.capabilities.source_instance(),
            target: self.target.binding(),
        })
    }

    fn with_flush(
        mut action: ScreenSessionAction,
        mut flush: ScreenEpochTransition,
        source_instance: ScreenSourceInstanceId,
        target: ScreenTargetBinding,
    ) -> ScreenSessionAction {
        flush.source_instance = source_instance;
        flush.target = target;
        action.flush = Some(flush);
        action
    }

    fn blocker_phase(&self) -> Option<ScreenCapturePhase> {
        if self.sleeping {
            return Some(ScreenCapturePhase::Suspended(
                ScreenSuspensionReason::Sleeping,
            ));
        }
        match self.permission {
            ScreenPermissionState::PromptRequired => {
                return Some(ScreenCapturePhase::AwaitingPermissionRequest);
            }
            ScreenPermissionState::Requesting => {
                return Some(ScreenCapturePhase::AwaitingPermissionResult);
            }
            ScreenPermissionState::Denied
            | ScreenPermissionState::Restricted
            | ScreenPermissionState::Revoked => {
                return Some(ScreenCapturePhase::Suspended(
                    ScreenSuspensionReason::PermissionBlocked,
                ));
            }
            ScreenPermissionState::Unchecked => {
                return Some(ScreenCapturePhase::AwaitingPreflight);
            }
            ScreenPermissionState::Granted => {}
        }
        if self.fresh_preflight_required {
            return Some(ScreenCapturePhase::AwaitingPreflight);
        }
        if !self.target_available {
            return Some(ScreenCapturePhase::Suspended(
                ScreenSuspensionReason::TargetLost,
            ));
        }
        if self.protected_content_active {
            return Some(ScreenCapturePhase::Suspended(
                ScreenSuspensionReason::ProtectedContent,
            ));
        }
        None
    }

    fn pending_phase(&self) -> Option<ScreenCapturePhase> {
        self.pending_operation.map(|pending| match pending.kind {
            ScreenOperationKind::Start => ScreenCapturePhase::Starting,
            ScreenOperationKind::Reconfigure => ScreenCapturePhase::Reconfiguring,
            ScreenOperationKind::Stop => ScreenCapturePhase::Stopping,
        })
    }

    fn settle(&mut self) -> Result<(ScreenCapturePhase, ScreenSessionAction), ScreenCaptureError> {
        if let Some(blocker) = self.blocker_phase() {
            return Ok((
                blocker,
                ScreenSessionAction::none(self.source_session_binding),
            ));
        }
        if let Some(pending) = self.pending_phase() {
            return Ok((
                pending,
                ScreenSessionAction::none(self.source_session_binding),
            ));
        }
        if self.stop_requested {
            return Ok((
                ScreenCapturePhase::Stopped,
                ScreenSessionAction::none(self.source_session_binding),
            ));
        }
        if self.desired_running {
            if self.active_stream.is_some() {
                Ok((
                    ScreenCapturePhase::Capturing,
                    ScreenSessionAction::none(self.source_session_binding),
                ))
            } else {
                Ok((ScreenCapturePhase::Starting, self.issue_start()?))
            }
        } else {
            Ok((
                ScreenCapturePhase::Ready,
                ScreenSessionAction::none(self.source_session_binding),
            ))
        }
    }

    fn validate_control_stamp(&self, stamp: ScreenControlStamp) -> Result<(), ScreenCaptureError> {
        if stamp.source_instance != self.negotiated.capabilities.source_instance() {
            return Err(ScreenCaptureError::CrossSourceEvent);
        }
        if stamp.epoch != self.control_epoch {
            return Err(ScreenCaptureError::StaleControlEpoch);
        }
        if stamp.sequence <= self.last_control_sequence {
            return Err(ScreenCaptureError::StaleControlEvent);
        }
        Ok(())
    }

    fn invalidated_action(
        &mut self,
        flush: ScreenEpochTransition,
        control: ScreenControlCommand,
    ) -> Result<ScreenSessionAction, ScreenCaptureError> {
        let mut action = self.issue_stop()?;
        action.control = control;
        Ok(Self::with_flush(
            action,
            flush,
            self.negotiated.capabilities.source_instance(),
            self.target.binding(),
        ))
    }

    fn apply(
        &mut self,
        event: ScreenSessionEvent,
    ) -> Result<ScreenSessionTransition, ScreenCaptureError> {
        let from = self.phase;
        let terminal_stop_retry = matches!(&event, ScreenSessionEvent::SourceFailed(failure)
            if self.pending_operation.is_some_and(|pending|
                pending.kind == ScreenOperationKind::Stop
                    && failure.operation_id() == Some(pending.operation_id)
                    && failure.stream() == Some(pending.stream)));
        let terminal_stop_completion = matches!(&event, ScreenSessionEvent::OperationCompleted(ack)
            if self.pending_operation.is_some_and(|pending|
                pending.kind == ScreenOperationKind::Stop
                    && ack.kind == pending.kind
                    && ack.operation_id == pending.operation_id
                    && ack.stream == pending.stream));
        if self.phase.is_terminal()
            && matches!(
                event,
                ScreenSessionEvent::StopRequested | ScreenSessionEvent::Cancel
            )
        {
            return Ok(ScreenSessionTransition {
                from,
                to: from,
                action: ScreenSessionAction::none(self.source_session_binding),
            });
        }
        if self.phase.is_terminal() && !terminal_stop_retry && !terminal_stop_completion {
            return Err(ScreenCaptureError::InvalidSessionTransition);
        }
        let control_stamp = event.control_stamp();
        if let Some(stamp) = control_stamp {
            self.validate_control_stamp(stamp)?;
        }

        let (to, action, diagnostic) = match event {
            ScreenSessionEvent::PreflightCompleted(observation) => {
                if !self.fresh_preflight_required
                    && self.permission != ScreenPermissionState::Unchecked
                {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                self.permission = observation.permission.state();
                match observation.permission {
                    PermissionPreflight::Granted => self.fresh_preflight_required = false,
                    PermissionPreflight::PromptRequired => {}
                    PermissionPreflight::Denied(_)
                    | PermissionPreflight::Restricted
                    | PermissionPreflight::Revoked(_) => self.fresh_preflight_required = true,
                }
                let (phase, action) = self.settle()?;
                (phase, action, ScreenDiagnosticEventCode::Preflight)
            }
            ScreenSessionEvent::RequestPermission => {
                if self.permission != ScreenPermissionState::PromptRequired {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                self.permission = ScreenPermissionState::Requesting;
                (
                    ScreenCapturePhase::AwaitingPermissionResult,
                    ScreenSessionAction::control(
                        self.source_session_binding,
                        ScreenControlCommand::RequestPermission,
                    ),
                    ScreenDiagnosticEventCode::PermissionRequest,
                )
            }
            ScreenSessionEvent::PermissionRequestCompleted(observation) => {
                if self.permission != ScreenPermissionState::Requesting {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                if observation.permission == PermissionPreflight::Granted {
                    self.permission = ScreenPermissionState::Granted;
                    let (phase, action) = self.settle()?;
                    (phase, action, ScreenDiagnosticEventCode::PermissionChanged)
                } else {
                    let flush = self.next_epoch()?;
                    self.permission = observation.permission.state();
                    self.fresh_preflight_required = true;
                    let action = self
                        .invalidated_action(flush, ScreenControlCommand::RunPermissionPreflight)?;
                    (
                        ScreenCapturePhase::Suspended(ScreenSuspensionReason::PermissionBlocked),
                        action,
                        ScreenDiagnosticEventCode::PermissionChanged,
                    )
                }
            }
            ScreenSessionEvent::PermissionChanged(observation) => {
                if observation.permission == PermissionPreflight::Granted {
                    self.permission = ScreenPermissionState::Granted;
                    let (phase, action) = if self.fresh_preflight_required {
                        (
                            ScreenCapturePhase::AwaitingPreflight,
                            ScreenSessionAction::control(
                                self.source_session_binding,
                                ScreenControlCommand::RunPermissionPreflight,
                            ),
                        )
                    } else if let Some(pending) = self.pending_phase() {
                        (
                            pending,
                            ScreenSessionAction::none(self.source_session_binding),
                        )
                    } else {
                        self.settle()?
                    };
                    (phase, action, ScreenDiagnosticEventCode::PermissionChanged)
                } else {
                    let flush = self.next_epoch()?;
                    self.permission = observation.permission.state();
                    self.fresh_preflight_required = true;
                    let action = self
                        .invalidated_action(flush, ScreenControlCommand::RunPermissionPreflight)?;
                    (
                        ScreenCapturePhase::Suspended(ScreenSuspensionReason::PermissionBlocked),
                        action,
                        ScreenDiagnosticEventCode::PermissionChanged,
                    )
                }
            }
            ScreenSessionEvent::StartRequested => {
                if self.phase != ScreenCapturePhase::Ready
                    || self.blocker_phase().is_some()
                    || self.pending_operation.is_some()
                    || self.active_stream.is_some()
                {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                self.desired_running = true;
                self.stop_requested = false;
                (
                    ScreenCapturePhase::Starting,
                    self.issue_start()?,
                    ScreenDiagnosticEventCode::StartRequested,
                )
            }
            ScreenSessionEvent::OperationCompleted(ack) => {
                let Some(pending) = self.pending_operation else {
                    return Err(ScreenCaptureError::MismatchedOperationAck);
                };
                if pending.kind != ack.kind
                    || pending.operation_id != ack.operation_id
                    || pending.stream != ack.stream
                {
                    return Err(ScreenCaptureError::MismatchedOperationAck);
                }
                match ack.kind {
                    ScreenOperationKind::Start | ScreenOperationKind::Reconfigure => {
                        if !self.desired_running || self.blocker_phase().is_some() {
                            return Err(ScreenCaptureError::MismatchedOperationAck);
                        }
                        self.pending_operation = None;
                        self.active_stream = Some(ActiveScreenStream {
                            operation_id: ack.operation_id,
                            stream: ack.stream,
                        });
                        let mut action = ScreenSessionAction::none(self.source_session_binding);
                        action.activate_stream = Some(ack.stream);
                        (
                            ScreenCapturePhase::Capturing,
                            action,
                            if ack.kind == ScreenOperationKind::Start {
                                ScreenDiagnosticEventCode::SourceStarted
                            } else {
                                ScreenDiagnosticEventCode::TargetReconfigured
                            },
                        )
                    }
                    ScreenOperationKind::Stop => {
                        self.pending_operation = None;
                        self.active_stream = None;
                        let (phase, action) = if from.is_terminal() {
                            (from, ScreenSessionAction::none(self.source_session_binding))
                        } else {
                            self.settle()?
                        };
                        (phase, action, ScreenDiagnosticEventCode::SourceStopped)
                    }
                }
            }
            ScreenSessionEvent::TargetChanged(change) => {
                return self.apply_target_change(from, *change);
            }
            ScreenSessionEvent::Sleep(_) => {
                if self.sleeping {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                let flush = self.next_epoch()?;
                self.sleeping = true;
                let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
                (
                    ScreenCapturePhase::Suspended(ScreenSuspensionReason::Sleeping),
                    action,
                    ScreenDiagnosticEventCode::Sleep,
                )
            }
            ScreenSessionEvent::Wake(_) => {
                if !self.sleeping {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                self.sleeping = false;
                self.permission = ScreenPermissionState::Unchecked;
                self.fresh_preflight_required = true;
                (
                    ScreenCapturePhase::AwaitingPreflight,
                    ScreenSessionAction::control(
                        self.source_session_binding,
                        ScreenControlCommand::RunPermissionPreflight,
                    ),
                    ScreenDiagnosticEventCode::Wake,
                )
            }
            ScreenSessionEvent::ProtectedContentDetected(_) => {
                if self.protected_content_active {
                    let (phase, _) = self.settle()?;
                    (
                        phase,
                        ScreenSessionAction::none(self.source_session_binding),
                        ScreenDiagnosticEventCode::ProtectedContent,
                    )
                } else {
                    let flush = self.next_epoch()?;
                    self.protected_content_active = true;
                    let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
                    match self.negotiated.request.protected_content() {
                        ProtectedContentPolicy::SuspendUntilClear => {
                            let phase = self
                                .blocker_phase()
                                .ok_or(ScreenCaptureError::InvalidSessionTransition)?;
                            (phase, action, ScreenDiagnosticEventCode::ProtectedContent)
                        }
                        ProtectedContentPolicy::FailSession => {
                            self.desired_running = false;
                            (
                                ScreenCapturePhase::Failed(
                                    ScreenSessionFailureCode::ProtectedContent,
                                ),
                                action,
                                ScreenDiagnosticEventCode::ProtectedContent,
                            )
                        }
                    }
                }
            }
            ScreenSessionEvent::ProtectedContentCleared(_) => {
                if !self.protected_content_active
                    || self.negotiated.request.protected_content()
                        != ProtectedContentPolicy::SuspendUntilClear
                {
                    return Err(ScreenCaptureError::InvalidSessionTransition);
                }
                self.protected_content_active = false;
                let (phase, action) = self.settle()?;
                (
                    phase,
                    action,
                    ScreenDiagnosticEventCode::ProtectedContentCleared,
                )
            }
            ScreenSessionEvent::StopRequested => {
                self.desired_running = false;
                self.stop_requested = true;
                let flush = self.next_epoch()?;
                let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
                let phase = if self.pending_operation.is_some() {
                    ScreenCapturePhase::Stopping
                } else {
                    ScreenCapturePhase::Stopped
                };
                (phase, action, ScreenDiagnosticEventCode::StopRequested)
            }
            ScreenSessionEvent::Cancel => {
                self.desired_running = false;
                let flush = self.next_epoch()?;
                let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
                (
                    ScreenCapturePhase::Cancelled,
                    action,
                    ScreenDiagnosticEventCode::Cancelled,
                )
            }
            ScreenSessionEvent::SourceFailed(failure) => {
                if !self.failure_matches_live_operation(&failure) {
                    return Err(ScreenCaptureError::StaleSourceFailure);
                }
                if self
                    .pending_operation
                    .is_some_and(|pending| pending.kind == ScreenOperationKind::Stop)
                {
                    (
                        from,
                        self.retry_pending_stop()?,
                        ScreenDiagnosticEventCode::SourceFailed,
                    )
                } else {
                    self.desired_running = false;
                    let flush = self.next_epoch()?;
                    let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
                    (
                        ScreenCapturePhase::Failed(ScreenSessionFailureCode::Source(
                            failure.code(),
                        )),
                        action,
                        ScreenDiagnosticEventCode::SourceFailed,
                    )
                }
            }
        };

        self.phase = to;
        self.last_event = diagnostic;
        if let Some(stamp) = control_stamp {
            self.last_control_sequence = stamp.sequence;
            self.control_events_observed = self.control_events_observed.saturating_add(1);
        }
        Ok(ScreenSessionTransition { from, to, action })
    }

    fn failure_matches_live_operation(&self, failure: &ScreenSourceFailure) -> bool {
        let Some(operation_id) = failure.operation_id() else {
            return false;
        };
        let Some(stream) = failure.stream() else {
            return false;
        };
        self.pending_operation
            .is_some_and(|pending| pending.operation_id == operation_id && pending.stream == stream)
            || self.active_stream.is_some_and(|active| {
                active.operation_id == operation_id && active.stream == stream
            })
    }

    fn matches_operation_request(&self, request: &ScreenOperationRequest) -> bool {
        self.pending_operation.is_some_and(|pending| {
            pending.kind == request.kind
                && pending.operation_id == request.operation_id
                && pending.stream == request.stream
                && pending.predecessor_stream == request.predecessor_stream
                && request.session_binding == self.source_session_binding
        })
    }

    fn bind_failure_to_live_operation(
        &self,
        failure: ScreenSourceFailure,
    ) -> Result<ScreenSourceFailure, ScreenCaptureError> {
        if failure.operation_id().is_some() || failure.stream().is_some() {
            return if self.failure_matches_live_operation(&failure) {
                Ok(failure)
            } else {
                Err(ScreenCaptureError::StaleSourceFailure)
            };
        }
        let operation = self
            .pending_operation
            .map(|pending| (pending.operation_id, pending.stream))
            .or_else(|| {
                self.active_stream
                    .map(|active| (active.operation_id, active.stream))
            })
            .ok_or(ScreenCaptureError::StaleSourceFailure)?;
        Ok(ScreenSourceFailure::for_operation(
            operation.0,
            operation.1,
            failure.code(),
            failure.retryable(),
        ))
    }

    fn validate_topology_change(
        &self,
        change: &ScreenTargetChange,
    ) -> Result<(), ScreenCaptureError> {
        let stamp = change.stamp;
        if stamp.source_instance != self.negotiated.capabilities.source_instance() {
            return Err(ScreenCaptureError::CrossSourceEvent);
        }
        if stamp.generation < self.topology_generation
            || (stamp.generation == self.topology_generation
                && stamp.sequence <= self.last_topology_sequence)
        {
            return Err(ScreenCaptureError::StaleTopologyEvent);
        }
        validate_topology_snapshot(stamp, &change.capabilities, &change.catalog)?;
        if change.capabilities.source() != self.negotiated.source
            || change.capabilities.control_epoch() != self.control_epoch
            || change.capabilities.control_sequence() < self.last_control_sequence
        {
            return Err(ScreenCaptureError::InvalidNegotiationRefresh);
        }
        Ok(())
    }

    fn refreshed_negotiation(
        &self,
        capabilities: &ScreenSourceCapabilities,
        catalog: &ScreenTargetSnapshot,
        selected: &ScreenTargetDescriptor,
    ) -> Result<NegotiatedScreenCapture, ScreenCaptureError> {
        if capabilities.source_instance() != self.negotiated.capabilities.source_instance()
            || capabilities.source() != self.negotiated.source
            || capabilities.control_epoch() != self.control_epoch
            || capabilities.control_sequence() < self.last_control_sequence
            || selected.id() != self.target.id()
        {
            return Err(ScreenCaptureError::InvalidNegotiationRefresh);
        }

        let mut excluded_windows =
            Vec::with_capacity(self.negotiated.request.excluded_windows().len());
        for previous in self.negotiated.request.excluded_windows() {
            let replacement = catalog
                .find(previous.id())
                .ok_or(ScreenCaptureError::UnknownTargetBinding)?;
            if replacement.kind() != ScreenTargetKind::Window {
                return Err(ScreenCaptureError::InvalidWindowExclusion);
            }
            excluded_windows.push(replacement.binding());
        }
        let request = ScreenCaptureRequest::new(ScreenCaptureRequestSpec {
            target: selected.clone(),
            output: self.negotiated.request.output(),
            cursor: self.negotiated.request.cursor(),
            excluded_windows,
            queue: self.negotiated.request.queue(),
            recovery: self.negotiated.request.recovery(),
            protected_content: self.negotiated.request.protected_content(),
        })?;
        negotiate_screen_capture(capabilities, catalog, request)
    }

    fn commit_topology_observation(&mut self, change: &ScreenTargetChange) {
        self.topology_generation = change.stamp.generation;
        self.last_topology_sequence = change.stamp.sequence;
        self.last_control_sequence = self
            .last_control_sequence
            .max(change.capabilities.control_sequence());
        self.target_events_observed = self.target_events_observed.saturating_add(1);
    }

    fn topology_capture_semantics_changed(
        &self,
        capabilities: &ScreenSourceCapabilities,
        selected: &ScreenTargetDescriptor,
        catalog: &ScreenTargetSnapshot,
    ) -> Result<bool, ScreenCaptureError> {
        let mut previous_capabilities = self.negotiated.capabilities.spec().clone();
        previous_capabilities.topology_generation = capabilities.topology_generation();
        previous_capabilities.control_sequence = capabilities.control_sequence();
        let mut changed = &previous_capabilities != capabilities.spec()
            || target_capture_semantics_changed(&self.target, selected)?;
        for previous_binding in self.negotiated.request.excluded_windows() {
            let previous = self
                .negotiated
                .catalog
                .find_binding(*previous_binding)
                .ok_or(ScreenCaptureError::UnknownTargetBinding)?;
            let current = catalog
                .find(previous.id())
                .ok_or(ScreenCaptureError::UnknownTargetBinding)?;
            changed |= target_capture_semantics_changed(previous, current)?;
        }
        Ok(changed)
    }

    fn finish_target_change(
        &mut self,
        from: ScreenCapturePhase,
        to: ScreenCapturePhase,
        action: ScreenSessionAction,
        diagnostic: ScreenDiagnosticEventCode,
        change: &ScreenTargetChange,
    ) -> ScreenSessionTransition {
        self.commit_topology_observation(change);
        self.phase = to;
        self.last_event = diagnostic;
        ScreenSessionTransition { from, to, action }
    }

    fn fail_closed_topology_contract(
        &mut self,
        from: ScreenCapturePhase,
        change: &ScreenTargetChange,
        diagnostic: ScreenDiagnosticEventCode,
    ) -> Result<ScreenSessionTransition, ScreenCaptureError> {
        let flush = self.next_epoch()?;
        self.desired_running = false;
        self.stop_requested = true;
        let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
        Ok(self.finish_target_change(
            from,
            ScreenCapturePhase::Failed(ScreenSessionFailureCode::ContractInvalidated),
            action,
            diagnostic,
            change,
        ))
    }

    fn apply_target_change(
        &mut self,
        from: ScreenCapturePhase,
        change: ScreenTargetChange,
    ) -> Result<ScreenSessionTransition, ScreenCaptureError> {
        self.validate_topology_change(&change)?;
        let selected_id = self.target.id();
        let changed_id = change.target_id();

        if changed_id != selected_id {
            let Some(selected) = change.catalog.find(selected_id).cloned() else {
                return self.fail_closed_topology_contract(
                    from,
                    &change,
                    ScreenDiagnosticEventCode::TargetHotplug,
                );
            };
            let refreshed = match self.refreshed_negotiation(
                &change.capabilities,
                &change.catalog,
                &selected,
            ) {
                Ok(refreshed) => refreshed,
                Err(error) if is_topology_contract_loss(&error) => {
                    return self.fail_closed_topology_contract(
                        from,
                        &change,
                        ScreenDiagnosticEventCode::TargetHotplug,
                    );
                }
                Err(error) => return Err(error),
            };
            let capture_semantics_changed = self.topology_capture_semantics_changed(
                &change.capabilities,
                &selected,
                &change.catalog,
            )?;
            self.target = selected;
            self.negotiated = refreshed;

            if !capture_semantics_changed
                && self.active_stream.is_some()
                && self.pending_operation.is_none()
            {
                return Ok(self.finish_target_change(
                    from,
                    self.phase,
                    ScreenSessionAction::none(self.source_session_binding),
                    ScreenDiagnosticEventCode::TargetHotplug,
                    &change,
                ));
            }

            let flush = self.next_epoch()?;
            let (to, action) = match self.pending_operation.map(|pending| pending.kind) {
                Some(ScreenOperationKind::Stop) => (
                    ScreenCapturePhase::Stopping,
                    ScreenSessionAction::none(self.source_session_binding),
                ),
                Some(ScreenOperationKind::Start | ScreenOperationKind::Reconfigure) => {
                    (ScreenCapturePhase::Stopping, self.issue_stop()?)
                }
                None if self.active_stream.is_some()
                    && self.desired_running
                    && self.blocker_phase().is_none() =>
                {
                    (ScreenCapturePhase::Reconfiguring, self.issue_reconfigure()?)
                }
                None if self.active_stream.is_some() => {
                    (ScreenCapturePhase::Stopping, self.issue_stop()?)
                }
                None => self.settle()?,
            };
            let action = Self::with_flush(
                action,
                flush,
                self.negotiated.capabilities.source_instance(),
                self.target.binding(),
            );
            return Ok(self.finish_target_change(
                from,
                to,
                action,
                ScreenDiagnosticEventCode::TargetHotplug,
                &change,
            ));
        }

        match &change.kind {
            ScreenTargetChangeKind::Removed { target, reason } => {
                if *target != self.target.binding() {
                    return Err(ScreenCaptureError::StaleTargetEpoch);
                }
                let flush = self.next_epoch()?;
                self.target_available = false;
                if *reason == TargetLossReason::AccessRevoked {
                    self.permission = ScreenPermissionState::Revoked;
                    self.fresh_preflight_required = true;
                }
                let recovery = self.negotiated.request.recovery();
                let (to, control) = match recovery {
                    TargetRecoveryPolicy::FailClosed => {
                        self.desired_running = false;
                        self.stop_requested = true;
                        // Terminal fail-closed loss never schedules a prompt or
                        // preflight, including an AccessRevoked loss.
                        (
                            ScreenCapturePhase::Failed(ScreenSessionFailureCode::TargetLost),
                            ScreenControlCommand::None,
                        )
                    }
                    TargetRecoveryPolicy::ResumeSameTarget { .. } => {
                        let phase = if *reason == TargetLossReason::AccessRevoked {
                            ScreenCapturePhase::Suspended(ScreenSuspensionReason::PermissionBlocked)
                        } else {
                            ScreenCapturePhase::Suspended(ScreenSuspensionReason::TargetLost)
                        };
                        let control = if *reason == TargetLossReason::AccessRevoked {
                            ScreenControlCommand::RunPermissionPreflight
                        } else {
                            ScreenControlCommand::None
                        };
                        (phase, control)
                    }
                };
                let action = self.invalidated_action(flush, control)?;
                Ok(self.finish_target_change(
                    from,
                    to,
                    action,
                    ScreenDiagnosticEventCode::TargetLost,
                    &change,
                ))
            }
            ScreenTargetChangeKind::Added(target) => {
                if self.target_available
                    || target.binding().target_epoch() <= self.target.binding().target_epoch()
                {
                    return Err(ScreenCaptureError::StaleTargetEpoch);
                }
                let max_attempts = self.negotiated.request.recovery().max_attempts();
                if max_attempts == 0 || self.recovery_attempts >= max_attempts {
                    self.desired_running = false;
                    self.stop_requested = true;
                    let flush = self.next_epoch()?;
                    let action = self.invalidated_action(flush, ScreenControlCommand::None)?;
                    return Ok(self.finish_target_change(
                        from,
                        ScreenCapturePhase::Failed(ScreenSessionFailureCode::RecoveryExhausted),
                        action,
                        ScreenDiagnosticEventCode::TargetRestored,
                        &change,
                    ));
                }
                let refreshed =
                    match self.refreshed_negotiation(&change.capabilities, &change.catalog, target)
                    {
                        Ok(refreshed) => refreshed,
                        Err(error) if is_topology_contract_loss(&error) => {
                            return self.fail_closed_topology_contract(
                                from,
                                &change,
                                ScreenDiagnosticEventCode::TargetRestored,
                            );
                        }
                        Err(error) => return Err(error),
                    };
                self.target = target.clone();
                self.negotiated = refreshed;
                self.target_available = true;
                self.recovery_attempts = self.recovery_attempts.saturating_add(1);
                let flush = self.next_epoch()?;
                let (to, action) = self.settle()?;
                let action = Self::with_flush(
                    action,
                    flush,
                    self.negotiated.capabilities.source_instance(),
                    self.target.binding(),
                );
                Ok(self.finish_target_change(
                    from,
                    to,
                    action,
                    ScreenDiagnosticEventCode::TargetRestored,
                    &change,
                ))
            }
            ScreenTargetChangeKind::Reconfigured(target) => {
                if !self.target_available
                    || target.binding().target_epoch() <= self.target.binding().target_epoch()
                {
                    return Err(ScreenCaptureError::StaleTargetEpoch);
                }
                let refreshed =
                    match self.refreshed_negotiation(&change.capabilities, &change.catalog, target)
                    {
                        Ok(refreshed) => refreshed,
                        Err(error) if is_topology_contract_loss(&error) => {
                            return self.fail_closed_topology_contract(
                                from,
                                &change,
                                ScreenDiagnosticEventCode::TargetReconfigured,
                            );
                        }
                        Err(error) => return Err(error),
                    };
                self.target = target.clone();
                self.negotiated = refreshed;
                let flush = self.next_epoch()?;

                let (to, action) = match self.pending_operation.map(|pending| pending.kind) {
                    Some(ScreenOperationKind::Stop) => (
                        ScreenCapturePhase::Stopping,
                        ScreenSessionAction::none(self.source_session_binding),
                    ),
                    Some(ScreenOperationKind::Start | ScreenOperationKind::Reconfigure) => {
                        (ScreenCapturePhase::Stopping, self.issue_stop()?)
                    }
                    None if self.active_stream.is_some()
                        && self.desired_running
                        && self.blocker_phase().is_none() =>
                    {
                        (ScreenCapturePhase::Reconfiguring, self.issue_reconfigure()?)
                    }
                    None if self.active_stream.is_some() => {
                        (ScreenCapturePhase::Stopping, self.issue_stop()?)
                    }
                    None => self.settle()?,
                };
                let action = Self::with_flush(
                    action,
                    flush,
                    self.negotiated.capabilities.source_instance(),
                    self.target.binding(),
                );
                Ok(self.finish_target_change(
                    from,
                    to,
                    action,
                    ScreenDiagnosticEventCode::TargetReconfigured,
                    &change,
                ))
            }
        }
    }
}

fn is_topology_contract_loss(error: &ScreenCaptureError) -> bool {
    matches!(
        error,
        ScreenCaptureError::UnknownTargetBinding
            | ScreenCaptureError::InvalidWindowExclusion
            | ScreenCaptureError::IncompatibleContract
            | ScreenCaptureError::SourcePlatformMismatch
            | ScreenCaptureError::RequiredCapabilityUnavailable
            | ScreenCaptureError::UnsupportedTargetKind
            | ScreenCaptureError::UnsupportedCursorPolicy
            | ScreenCaptureError::UnsupportedFrameSpec
            | ScreenCaptureError::UnsupportedWindowExclusion
            | ScreenCaptureError::UnsupportedRecoveryPolicy
            | ScreenCaptureError::ProtectedContentSignalUnavailable
    )
}

fn target_capture_semantics_changed(
    previous: &ScreenTargetDescriptor,
    current: &ScreenTargetDescriptor,
) -> Result<bool, ScreenCaptureError> {
    if previous.id() != current.id()
        || previous.binding().source_instance() != current.binding().source_instance()
    {
        return Err(ScreenCaptureError::InvalidNegotiationRefresh);
    }
    let same_geometry = match (&previous.geometry, &current.geometry) {
        (ScreenTargetGeometry::Display(previous), ScreenTargetGeometry::Display(current)) => {
            previous == current
        }
        (ScreenTargetGeometry::Window(previous), ScreenTargetGeometry::Window(current)) => {
            previous == current
        }
        (
            ScreenTargetGeometry::Region {
                display: previous_display,
                bounds: previous_bounds,
                transform: previous_transform,
            },
            ScreenTargetGeometry::Region {
                display: current_display,
                bounds: current_bounds,
                transform: current_transform,
            },
        ) => {
            previous_display.source_instance() == current_display.source_instance()
                && previous_display.id() == current_display.id()
                && previous_display.target_epoch() == current_display.target_epoch()
                && previous_bounds == current_bounds
                && previous_transform == current_transform
        }
        _ => false,
    };
    let previous_epoch = previous.target_epoch();
    let current_epoch = current.target_epoch();
    if same_geometry && previous_epoch == current_epoch {
        return Ok(false);
    }
    if current_epoch <= previous_epoch {
        return Err(ScreenCaptureError::StaleTargetEpoch);
    }
    Ok(true)
}

impl ScreenSessionEvent {
    const fn control_stamp(&self) -> Option<ScreenControlStamp> {
        match self {
            Self::PreflightCompleted(observation)
            | Self::PermissionRequestCompleted(observation)
            | Self::PermissionChanged(observation) => Some(observation.stamp),
            Self::Sleep(stamp)
            | Self::Wake(stamp)
            | Self::ProtectedContentDetected(stamp)
            | Self::ProtectedContentCleared(stamp) => Some(*stamp),
            Self::StartRequested
            | Self::RequestPermission
            | Self::OperationCompleted(_)
            | Self::TargetChanged(_)
            | Self::StopRequested
            | Self::Cancel
            | Self::SourceFailed(_) => None,
        }
    }
}

impl ScreenCaptureSession {
    fn record_queue_outcome(&mut self, outcome: ScreenQueuePushOutcome) {
        match outcome {
            ScreenQueuePushOutcome::Accepted
            | ScreenQueuePushOutcome::AcceptedAfterDropping { .. } => {
                self.frames_accepted = self.frames_accepted.saturating_add(1);
                self.last_event = ScreenDiagnosticEventCode::QueueAccepted;
            }
            ScreenQueuePushOutcome::DroppedNewest | ScreenQueuePushOutcome::DroppedOversized => {
                self.frames_dropped = self.frames_dropped.saturating_add(1);
                self.last_event = ScreenDiagnosticEventCode::QueueDropped;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenIngressDrainReport {
    pub queue: ScreenQueueDrainReport,
    pub cursor: ScreenCursorCacheDrain,
}

/// Private proof of the exact logical ingress incarnation that owned a
/// transition. The capture epoch and active stream are intentionally included:
/// a transition minted after a flush must not authorize a graph for the
/// retired segment, even when both segments share one native source binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenIngressOwner {
    source_session_binding: ScreenSourceSessionBinding,
    session_id: ScreenSessionId,
    source_instance: ScreenSourceInstanceId,
    capture_epoch: CaptureEpoch,
    active_stream: Option<ScreenStreamStamp>,
}

impl ScreenIngressOwner {
    #[must_use]
    pub(crate) const fn active_stream(self) -> Option<ScreenStreamStamp> {
        self.active_stream
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScreenIngressTransition {
    pub transition: ScreenSessionTransition,
    pub drain: Option<ScreenIngressDrainReport>,
    owner: ScreenIngressOwner,
}

impl ScreenIngressTransition {
    #[must_use]
    pub(crate) const fn owner(&self) -> ScreenIngressOwner {
        self.owner
    }
}

/// One exact graceful-stop request for a live recording segment.
///
/// Only [`ScreenCaptureIngress::request_graceful_stop`] can mint this value.
/// It exposes only the action mutably so the caller can execute the one-shot
/// native `Stop` without altering the authenticated transition or drain.
#[derive(Debug)]
pub struct ScreenGracefulStop {
    owner: ScreenIngressOwner,
    seal_epoch: CaptureEpoch,
    seal_revision: u64,
    expected_stream: ScreenStreamStamp,
    transition: ScreenIngressTransition,
}

impl ScreenGracefulStop {
    #[must_use]
    pub const fn transition(&self) -> &ScreenIngressTransition {
        &self.transition
    }

    #[must_use]
    pub const fn action_mut(&mut self) -> &mut ScreenSessionAction {
        &mut self.transition.transition.action
    }

    fn expected_operation_id(&self) -> Option<ScreenOperationId> {
        match self.transition.transition.action.source_command() {
            ScreenSourceCommand::Stop {
                operation_id,
                stream,
            } if stream == self.expected_stream => Some(operation_id),
            ScreenSourceCommand::None
            | ScreenSourceCommand::Start { .. }
            | ScreenSourceCommand::Reconfigure { .. }
            | ScreenSourceCommand::Stop { .. } => None,
        }
    }
}

/// Failure while correlating a one-shot graceful-Stop proof.
///
/// `Rejected` means no session or ingress mutation occurred and returns the
/// exact proof so the caller can submit the correct acknowledgement or failure.
/// `Transition` means correlation succeeded but the authenticated state
/// transition itself failed; the old proof is no longer returned as reusable
/// authority.
#[derive(Debug)]
pub enum ScreenGracefulProofError<Proof> {
    Rejected {
        proof: Box<Proof>,
        error: ScreenCaptureError,
    },
    Transition(ScreenCaptureError),
}

impl<Proof> ScreenGracefulProofError<Proof> {
    fn rejected(proof: Proof, error: ScreenCaptureError) -> Self {
        Self::Rejected {
            proof: Box::new(proof),
            error,
        }
    }

    /// Returns the rejected proof and correlation error when no mutation
    /// occurred. A transition failure deliberately has no reusable proof.
    #[must_use]
    pub fn into_rejected(self) -> Option<(Proof, ScreenCaptureError)> {
        match self {
            Self::Rejected { proof, error } => Some((*proof, error)),
            Self::Transition(_) => None,
        }
    }

    #[must_use]
    pub const fn capture_error(&self) -> &ScreenCaptureError {
        match self {
            Self::Rejected { error, .. } | Self::Transition(error) => error,
        }
    }
}

impl<Proof> fmt::Display for ScreenGracefulProofError<Proof> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected { error, .. } => {
                write!(formatter, "graceful Stop correlation was rejected: {error}")
            }
            Self::Transition(error) => {
                write!(formatter, "graceful Stop transition failed: {error}")
            }
        }
    }
}

impl<Proof: fmt::Debug> std::error::Error for ScreenGracefulProofError<Proof> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.capture_error())
    }
}

/// Result of rebinding one failed native Stop attempt.
///
/// `Retrying` preserves publication authority in the exact seal epoch.
/// `AbortOnly` retains the newly issued Stop action when a suspension or other
/// epoch handoff invalidated that authority while the failing call was in
/// flight.
#[derive(Debug)]
pub enum ScreenGracefulStopRetryOutcome {
    Retrying(ScreenGracefulStop),
    AbortOnly(ScreenGracefulStopAbort),
}

/// Opaque authority for finishing native teardown after publication became
/// impossible. It can be retried and acknowledged, but can never be converted
/// into [`ScreenGracefulStopCompletion`].
#[derive(Debug)]
pub struct ScreenGracefulStopAbort {
    owner: ScreenIngressOwner,
    seal_epoch: CaptureEpoch,
    seal_revision: u64,
    expected_stream: ScreenStreamStamp,
    transition: ScreenIngressTransition,
}

impl ScreenGracefulStopAbort {
    #[must_use]
    pub const fn transition(&self) -> &ScreenIngressTransition {
        &self.transition
    }

    #[must_use]
    pub const fn action_mut(&mut self) -> &mut ScreenSessionAction {
        &mut self.transition.transition.action
    }

    fn expected_operation_id(&self) -> Option<ScreenOperationId> {
        match self.transition.transition.action.source_command() {
            ScreenSourceCommand::Stop {
                operation_id,
                stream,
            } if stream == self.expected_stream => Some(operation_id),
            ScreenSourceCommand::None
            | ScreenSourceCommand::Start { .. }
            | ScreenSourceCommand::Reconfigure { .. }
            | ScreenSourceCommand::Stop { .. } => None,
        }
    }
}

/// Proof that the exact native `Stop` requested for a recording segment was
/// acknowledged and applied to its owning capture session.
///
/// The private fields make this a non-forgeable finish authority. Cancellation,
/// suspension, and source-failure transitions cannot construct it.
#[derive(Debug)]
pub struct ScreenGracefulStopCompletion {
    owner: ScreenIngressOwner,
    stopped_stream: ScreenStreamStamp,
    transition: ScreenIngressTransition,
}

impl ScreenGracefulStopCompletion {
    #[must_use]
    pub const fn transition(&self) -> &ScreenIngressTransition {
        &self.transition
    }

    #[must_use]
    pub(crate) const fn owner(&self) -> ScreenIngressOwner {
        self.owner
    }

    #[must_use]
    pub(crate) const fn stopped_stream(&self) -> ScreenStreamStamp {
        self.stopped_stream
    }
}

/// Result of consuming a native Stop acknowledgement after graceful
/// publication lineage had already changed.
#[derive(Debug)]
pub struct ScreenGracefulStopAbortCompletion {
    seal_epoch: CaptureEpoch,
    stopped_stream: ScreenStreamStamp,
    transition: Option<Box<ScreenIngressTransition>>,
}

impl ScreenGracefulStopAbortCompletion {
    /// A transition is present when the acknowledgement still matched the
    /// session's current Stop operation and was therefore applied to settle
    /// native teardown. A stale, already-replaced Stop acknowledgement is
    /// still consumed as teardown evidence but cannot mutate session state.
    #[must_use]
    pub fn transition(&self) -> Option<&ScreenIngressTransition> {
        self.transition.as_deref()
    }

    #[must_use]
    pub const fn seal_epoch(&self) -> CaptureEpoch {
        self.seal_epoch
    }

    #[must_use]
    pub const fn stopped_stream(&self) -> ScreenStreamStamp {
        self.stopped_stream
    }
}

#[derive(Debug)]
pub enum ScreenGracefulStopCompletionOutcome {
    Completed(Box<ScreenGracefulStopCompletion>),
    AbortOnly(ScreenGracefulStopAbortCompletion),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScreenIngressOutcome {
    Frame(ScreenQueuePushOutcome),
    CursorImageAccepted,
    Session(Box<ScreenIngressTransition>),
}

#[derive(Debug)]
pub enum ScreenIngressPopOutcome<T> {
    Frame(ScreenFrame<T>),
    Empty,
    Cancelled(Box<ScreenIngressTransition>),
}

/// The only public frame/cursor ingress for a capture session. It owns the
/// negotiated cursor policy, queue, cursor-image cache, epoch, and active
/// stream as one fail-closed boundary.
///
/// A source result cannot be forged through the former generic event API:
/// `ScreenSessionEvent` is private, `apply_session_event` does not exist, and
/// source-event envelope fields are private.
///
/// ```compile_fail
/// use frame_media::{
///     ScreenCaptureIngress, ScreenCaptureSession, ScreenPermissionObservation,
/// };
///
/// fn forge_granted_preflight<Frame, Cursor>(
///     ingress: &mut ScreenCaptureIngress<Frame, Cursor>,
///     session: &mut ScreenCaptureSession,
///     observation: ScreenPermissionObservation,
/// ) {
///     ingress.apply_session_event(
///         session,
///         frame_media::ScreenSessionEvent::PreflightCompleted(observation),
///     );
/// }
/// ```
pub struct ScreenCaptureIngress<FramePayload, CursorImagePayload> {
    session_id: ScreenSessionId,
    source_session_binding: ScreenSourceSessionBinding,
    source_instance: ScreenSourceInstanceId,
    target: ScreenTargetBinding,
    capture_epoch: CaptureEpoch,
    active_stream: Option<ScreenStreamStamp>,
    transition_revision: u64,
    cursor_policy: CursorPolicy,
    queue: BoundedScreenFrameQueue<FramePayload>,
    cursor_cache: BoundedCursorImageCache<CursorImagePayload>,
}

impl<FramePayload, CursorImagePayload> fmt::Debug
    for ScreenCaptureIngress<FramePayload, CursorImagePayload>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScreenCaptureIngress")
            .field("session_id", &self.session_id)
            .field("source_session_binding", &self.source_session_binding)
            .field("source_instance", &self.source_instance)
            .field("target", &self.target)
            .field("capture_epoch", &self.capture_epoch)
            .field("active_stream", &self.active_stream)
            .field("cursor_policy", &self.cursor_policy)
            .field("queue", &self.queue)
            .field("cursor_cache", &self.cursor_cache)
            .finish()
    }
}

impl<FramePayload: ScreenFramePayload, CursorImagePayload: ScreenFramePayload>
    ScreenCaptureIngress<FramePayload, CursorImagePayload>
{
    pub fn new(session: &ScreenCaptureSession) -> Result<Self, ScreenCaptureError> {
        let target = session.target.binding();
        let capture_epoch = session.capture_epoch;
        Ok(Self {
            session_id: session.session_id,
            source_session_binding: session.source_session_binding,
            source_instance: session.negotiated.capabilities.source_instance(),
            target,
            capture_epoch,
            active_stream: None,
            transition_revision: 0,
            cursor_policy: session.negotiated.request.cursor(),
            queue: BoundedScreenFrameQueue::new(
                session.negotiated.request.queue(),
                session.negotiated.request.output(),
                capture_epoch,
                target.target_epoch(),
            )?,
            cursor_cache: BoundedCursorImageCache::new(capture_epoch, target.target_epoch()),
        })
    }

    #[must_use]
    pub const fn capture_epoch(&self) -> CaptureEpoch {
        self.capture_epoch
    }

    #[must_use]
    pub const fn active_stream(&self) -> Option<ScreenStreamStamp> {
        self.active_stream
    }

    #[must_use]
    pub const fn queue_diagnostics(&self) -> ScreenCaptureQueueDiagnostics {
        self.queue.diagnostics()
    }

    #[must_use]
    pub fn cursor_descriptor(&self) -> Option<CursorImageDescriptor> {
        self.cursor_cache.descriptor()
    }

    const fn current_owner(&self) -> ScreenIngressOwner {
        ScreenIngressOwner {
            source_session_binding: self.source_session_binding,
            session_id: self.session_id,
            source_instance: self.source_instance,
            capture_epoch: self.capture_epoch,
            active_stream: self.active_stream,
        }
    }

    /// Returns the opaque owner of one currently capturing segment. Both the
    /// session and ingress must agree on the active stream before a recording
    /// graph may bind to it.
    pub(crate) fn recording_owner(
        &self,
        session: &ScreenCaptureSession,
    ) -> Result<ScreenIngressOwner, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if session.phase() != ScreenCapturePhase::Capturing
            || self.active_stream.is_none()
            || session.active_stream() != self.active_stream
        {
            return Err(ScreenCaptureError::UnexpectedSourceData);
        }
        Ok(self.current_owner())
    }

    /// Applies an authentic epoch handoff. Replaying a handoff is rejected
    /// before either queue or cache is mutated.
    pub fn apply_epoch_transition(
        &mut self,
        transition: ScreenEpochTransition,
    ) -> Result<ScreenIngressDrainReport, ScreenCaptureError> {
        if transition.session_binding != self.source_session_binding {
            return Err(ScreenCaptureError::EpochTransitionOwnershipMismatch);
        }
        if transition.source_instance != self.source_instance {
            return Err(ScreenCaptureError::CrossSourceEvent);
        }
        if transition.retired_capture_epoch != self.capture_epoch
            || transition.active_capture_epoch != transition.retired_capture_epoch.next()?
        {
            return Err(ScreenCaptureError::NonMonotonicCaptureEpoch);
        }
        if transition.target.source_instance() != self.source_instance
            || transition.target.id() != self.target.id()
        {
            return Err(ScreenCaptureError::TargetCatalogBindingMismatch);
        }

        // All fallible identity checks happen before the first lease is
        // released, so the two drains are one atomic owner-side handoff.
        let queue = self.queue.reset_for_epoch(
            transition.active_capture_epoch,
            transition.target.target_epoch(),
        )?;
        let cursor = self.cursor_cache.reset(
            transition.active_capture_epoch,
            transition.target.target_epoch(),
        );
        self.capture_epoch = transition.active_capture_epoch;
        self.target = transition.target;
        self.active_stream = None;
        Ok(ScreenIngressDrainReport { queue, cursor })
    }

    fn apply_action(
        &mut self,
        action: &ScreenSessionAction,
    ) -> Result<Option<ScreenIngressDrainReport>, ScreenCaptureError> {
        if action.owner != self.source_session_binding {
            return Err(ScreenCaptureError::ActionSessionOwnershipMismatch);
        }
        let drain = action
            .flush
            .map(|transition| self.apply_epoch_transition(transition))
            .transpose()?;
        if let Some(stream) = action.activate_stream {
            if self.active_stream.is_some()
                || stream.source_instance() != self.source_instance
                || stream.capture_epoch() != self.capture_epoch
                || stream.target() != self.target
            {
                return Err(ScreenCaptureError::StreamIdentityMismatch);
            }
            self.queue.activate_stream(stream)?;
            self.active_stream = Some(stream);
        }
        Ok(drain)
    }

    fn validate_session_identity(
        &self,
        session: &ScreenCaptureSession,
    ) -> Result<(), ScreenCaptureError> {
        if session.session_id != self.session_id
            || session.source_session_binding != self.source_session_binding
            || session.negotiated.capabilities.source_instance() != self.source_instance
            || session.capture_epoch != self.capture_epoch
        {
            return Err(ScreenCaptureError::IngressSessionMismatch);
        }
        Ok(())
    }

    fn validate_bound_source<S: ScreenCaptureSource>(
        &self,
        session: &ScreenCaptureSession,
        source: &BoundScreenCaptureSource<S>,
    ) -> Result<(), ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if source.binding() != session.source_session_binding {
            return Err(ScreenCaptureError::SourceSessionOwnershipMismatch);
        }
        Ok(())
    }

    fn apply_internal_event(
        &mut self,
        session: &mut ScreenCaptureSession,
        event: ScreenSessionEvent,
    ) -> Result<ScreenIngressTransition, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        let transition_revision = self
            .transition_revision
            .checked_add(1)
            .ok_or(ScreenCaptureError::IngressTransitionSequenceExhausted)?;
        let owner = self.current_owner();
        let transition = session.apply(event)?;
        // Advance before applying the action: even an unexpected downstream
        // action failure must invalidate a previously minted publication
        // lineage after the session itself changed.
        self.transition_revision = transition_revision;
        let drain = self.apply_action(&transition.action)?;
        Ok(ScreenIngressTransition {
            transition,
            drain,
            owner,
        })
    }

    /// Applies a freely constructible local/user intent. Source observations
    /// and library operation results have separate owner-bound entry points.
    pub fn apply_intent(
        &mut self,
        session: &mut ScreenCaptureSession,
        intent: ScreenSessionIntent,
    ) -> Result<ScreenIngressTransition, ScreenCaptureError> {
        match intent {
            ScreenSessionIntent::RequestPermission => {
                self.apply_internal_event(session, ScreenSessionEvent::RequestPermission)
            }
            ScreenSessionIntent::Start => {
                self.apply_internal_event(session, ScreenSessionEvent::StartRequested)
            }
            ScreenSessionIntent::Stop => {
                self.apply_internal_event(session, ScreenSessionEvent::StopRequested)
            }
            ScreenSessionIntent::Cancel => self.cancel_session(session),
        }
    }

    /// Starts the only capture-session transition that may eventually seal a
    /// recording artifact. The upstream frame queue must already be empty;
    /// otherwise the epoch flush would silently discard frames that never
    /// reached the recording graph.
    pub fn request_graceful_stop(
        &mut self,
        session: &mut ScreenCaptureSession,
    ) -> Result<ScreenGracefulStop, ScreenCaptureError> {
        let owner = self.recording_owner(session)?;
        let queue = self.queue.diagnostics();
        if queue.queued_frames != 0 || queue.queued_bytes != 0 {
            return Err(ScreenCaptureError::GracefulStopRequiresDrainedIngress);
        }
        let expected_stream = owner
            .active_stream()
            .ok_or(ScreenCaptureError::UnexpectedSourceData)?;
        let transition = self.apply_internal_event(session, ScreenSessionEvent::StopRequested)?;
        Ok(ScreenGracefulStop {
            owner,
            seal_epoch: self.capture_epoch,
            seal_revision: self.transition_revision,
            expected_stream,
            transition,
        })
    }

    fn preflight_graceful_transition(
        &self,
        session: &ScreenCaptureSession,
        issues_retry_operation: bool,
    ) -> Result<(), ScreenCaptureError> {
        self.transition_revision
            .checked_add(1)
            .ok_or(ScreenCaptureError::IngressTransitionSequenceExhausted)?;
        if issues_retry_operation {
            session
                .next_operation_sequence
                .checked_add(1)
                .ok_or(ScreenCaptureError::OperationSequenceExhausted)?;
        }
        Ok(())
    }

    /// Applies the acknowledgement produced by executing the exact native
    /// `Stop` action inside `stop`. Only this path can mint recording finish
    /// authority.
    pub fn complete_graceful_stop(
        &mut self,
        session: &mut ScreenCaptureSession,
        stop: ScreenGracefulStop,
        acknowledgement: ScreenOperationAck,
    ) -> Result<ScreenGracefulStopCompletionOutcome, ScreenGracefulProofError<ScreenGracefulStop>>
    {
        if let Err(error) = self.validate_session_identity(session) {
            return Err(ScreenGracefulProofError::rejected(stop, error));
        }
        let Some(expected_operation_id) = stop.expected_operation_id() else {
            return Err(ScreenGracefulProofError::rejected(
                stop,
                ScreenCaptureError::InvalidSessionTransition,
            ));
        };
        if self.source_session_binding != stop.owner.source_session_binding
            || self.session_id != stop.owner.session_id
            || self.source_instance != stop.owner.source_instance
            || acknowledgement.kind() != ScreenOperationKind::Stop
            || acknowledgement.operation_id() != expected_operation_id
            || acknowledgement.stream() != stop.expected_stream
        {
            return Err(ScreenGracefulProofError::rejected(
                stop,
                ScreenCaptureError::MismatchedOperationAck,
            ));
        }
        let pending_matches =
            session.pending_stop_matches(expected_operation_id, stop.expected_stream);
        if pending_matches && let Err(error) = self.preflight_graceful_transition(session, false) {
            return Err(ScreenGracefulProofError::rejected(stop, error));
        }
        let publication_lineage_valid = self.capture_epoch == stop.seal_epoch
            && session.capture_epoch == stop.seal_epoch
            && self.transition_revision == stop.seal_revision
            && session.phase() == ScreenCapturePhase::Stopping
            && pending_matches;
        if !publication_lineage_valid {
            let transition = if pending_matches {
                let revision_before = self.transition_revision;
                match self.complete_operation(session, acknowledgement) {
                    Ok(transition) => Some(Box::new(transition)),
                    Err(error)
                        if self.transition_revision == revision_before
                            && session.pending_stop_matches(
                                expected_operation_id,
                                stop.expected_stream,
                            ) =>
                    {
                        return Err(ScreenGracefulProofError::rejected(stop, error));
                    }
                    Err(error) => return Err(ScreenGracefulProofError::Transition(error)),
                }
            } else {
                None
            };
            return Ok(ScreenGracefulStopCompletionOutcome::AbortOnly(
                ScreenGracefulStopAbortCompletion {
                    seal_epoch: stop.seal_epoch,
                    stopped_stream: stop.expected_stream,
                    transition,
                },
            ));
        }
        let revision_before = self.transition_revision;
        let transition = match self.complete_operation(session, acknowledgement) {
            Ok(transition) => transition,
            Err(error)
                if self.transition_revision == revision_before
                    && session
                        .pending_stop_matches(expected_operation_id, stop.expected_stream) =>
            {
                return Err(ScreenGracefulProofError::rejected(stop, error));
            }
            Err(error) => return Err(ScreenGracefulProofError::Transition(error)),
        };
        if session.phase() != ScreenCapturePhase::Stopped
            || session.pending_operation_kind().is_some()
            || session.active_stream().is_some()
            || transition.transition.to != ScreenCapturePhase::Stopped
        {
            return Ok(ScreenGracefulStopCompletionOutcome::AbortOnly(
                ScreenGracefulStopAbortCompletion {
                    seal_epoch: stop.seal_epoch,
                    stopped_stream: stop.expected_stream,
                    transition: Some(Box::new(transition)),
                },
            ));
        }
        Ok(ScreenGracefulStopCompletionOutcome::Completed(Box::new(
            ScreenGracefulStopCompletion {
                owner: stop.owner,
                stopped_stream: stop.expected_stream,
                transition,
            },
        )))
    }

    /// Rebinds a graceful-stop proof to the exact retry action produced after
    /// its current native `Stop` attempt failed.
    ///
    /// The owner, retired recording stream, and already-drained ingress remain
    /// immutable. Only the one-shot operation id and authenticated action are
    /// replaced. Calling the generic failure API instead would strand the
    /// original finish proof on a consumed operation id.
    pub fn retry_graceful_stop(
        &mut self,
        session: &mut ScreenCaptureSession,
        stop: ScreenGracefulStop,
        failure: ScreenSourceFailureEnvelope,
    ) -> Result<ScreenGracefulStopRetryOutcome, ScreenGracefulProofError<ScreenGracefulStop>> {
        if let Err(error) = self.validate_session_identity(session) {
            return Err(ScreenGracefulProofError::rejected(stop, error));
        }
        let Some(expected_operation_id) = stop.expected_operation_id() else {
            return Err(ScreenGracefulProofError::rejected(
                stop,
                ScreenCaptureError::InvalidSessionTransition,
            ));
        };
        let pending_matches =
            session.pending_stop_matches(expected_operation_id, stop.expected_stream);
        if stop.owner.source_session_binding != self.source_session_binding
            || stop.owner.session_id != self.session_id
            || stop.owner.source_instance != self.source_instance
            || stop.owner.active_stream() != Some(stop.expected_stream)
            || failure.owner != self.source_session_binding
            || failure.operation_id() != Some(expected_operation_id)
            || failure.stream() != Some(stop.expected_stream)
            || !pending_matches
        {
            return Err(ScreenGracefulProofError::rejected(
                stop,
                ScreenCaptureError::StaleSourceFailure,
            ));
        }

        if let Err(error) = self.preflight_graceful_transition(session, true) {
            return Err(ScreenGracefulProofError::rejected(stop, error));
        }

        let revision_before = self.transition_revision;
        let transition = match self.apply_operation_failure(session, failure) {
            Ok(transition) => transition,
            Err(error)
                if self.transition_revision == revision_before
                    && session
                        .pending_stop_matches(expected_operation_id, stop.expected_stream) =>
            {
                return Err(ScreenGracefulProofError::rejected(stop, error));
            }
            Err(error) => return Err(ScreenGracefulProofError::Transition(error)),
        };
        let retry_stop_matches = matches!(
            transition.transition.action.source_command(),
            ScreenSourceCommand::Stop { stream, .. } if stream == stop.expected_stream
        );
        let retry_owner = transition.owner();
        let retry_identity_valid = retry_owner.source_session_binding
            == self.source_session_binding
            && retry_owner.session_id == self.session_id
            && retry_owner.source_instance == self.source_instance
            && retry_owner.capture_epoch == self.capture_epoch
            && retry_owner.active_stream().is_none()
            && transition.drain.is_none();
        if retry_identity_valid
            && retry_stop_matches
            && self.capture_epoch == stop.seal_epoch
            && stop.seal_revision.checked_add(1) == Some(self.transition_revision)
            && transition.transition.to == ScreenCapturePhase::Stopping
        {
            return Ok(ScreenGracefulStopRetryOutcome::Retrying(
                ScreenGracefulStop {
                    seal_revision: self.transition_revision,
                    transition,
                    ..stop
                },
            ));
        }
        Ok(ScreenGracefulStopRetryOutcome::AbortOnly(
            ScreenGracefulStopAbort {
                owner: stop.owner,
                seal_epoch: stop.seal_epoch,
                seal_revision: stop.seal_revision,
                expected_stream: stop.expected_stream,
                transition,
            },
        ))
    }

    /// Rebinds an abort-only Stop after an exact failure. This keeps the new
    /// one-shot action owned by the returned proof even while the session is
    /// suspended or terminal.
    pub fn retry_graceful_abort(
        &mut self,
        session: &mut ScreenCaptureSession,
        abort: ScreenGracefulStopAbort,
        failure: ScreenSourceFailureEnvelope,
    ) -> Result<ScreenGracefulStopAbort, ScreenGracefulProofError<ScreenGracefulStopAbort>> {
        if let Err(error) = self.validate_session_identity(session) {
            return Err(ScreenGracefulProofError::rejected(abort, error));
        }
        let Some(expected_operation_id) = abort.expected_operation_id() else {
            return Err(ScreenGracefulProofError::rejected(
                abort,
                ScreenCaptureError::InvalidSessionTransition,
            ));
        };
        let pending_matches =
            session.pending_stop_matches(expected_operation_id, abort.expected_stream);
        if abort.owner.source_session_binding != self.source_session_binding
            || abort.owner.session_id != self.session_id
            || abort.owner.source_instance != self.source_instance
            || self.transition_revision < abort.seal_revision
            || failure.owner != self.source_session_binding
            || failure.operation_id() != Some(expected_operation_id)
            || failure.stream() != Some(abort.expected_stream)
            || !pending_matches
        {
            return Err(ScreenGracefulProofError::rejected(
                abort,
                ScreenCaptureError::StaleSourceFailure,
            ));
        }
        if let Err(error) = self.preflight_graceful_transition(session, true) {
            return Err(ScreenGracefulProofError::rejected(abort, error));
        }
        let revision_before = self.transition_revision;
        let transition = match self.apply_operation_failure(session, failure) {
            Ok(transition) => transition,
            Err(error)
                if self.transition_revision == revision_before
                    && session
                        .pending_stop_matches(expected_operation_id, abort.expected_stream) =>
            {
                return Err(ScreenGracefulProofError::rejected(abort, error));
            }
            Err(error) => return Err(ScreenGracefulProofError::Transition(error)),
        };
        Ok(ScreenGracefulStopAbort {
            seal_revision: self.transition_revision,
            transition,
            ..abort
        })
    }

    /// Applies the exact current abort-only Stop acknowledgement. The result
    /// deliberately carries no artifact publication authority.
    pub fn complete_graceful_abort(
        &mut self,
        session: &mut ScreenCaptureSession,
        abort: ScreenGracefulStopAbort,
        acknowledgement: ScreenOperationAck,
    ) -> Result<ScreenGracefulStopAbortCompletion, ScreenGracefulProofError<ScreenGracefulStopAbort>>
    {
        if let Err(error) = self.validate_session_identity(session) {
            return Err(ScreenGracefulProofError::rejected(abort, error));
        }
        let Some(expected_operation_id) = abort.expected_operation_id() else {
            return Err(ScreenGracefulProofError::rejected(
                abort,
                ScreenCaptureError::InvalidSessionTransition,
            ));
        };
        let pending_matches =
            session.pending_stop_matches(expected_operation_id, abort.expected_stream);
        if abort.owner.source_session_binding != self.source_session_binding
            || abort.owner.session_id != self.session_id
            || abort.owner.source_instance != self.source_instance
            || acknowledgement.kind() != ScreenOperationKind::Stop
            || acknowledgement.operation_id() != expected_operation_id
            || acknowledgement.stream() != abort.expected_stream
            || !pending_matches
        {
            return Err(ScreenGracefulProofError::rejected(
                abort,
                ScreenCaptureError::MismatchedOperationAck,
            ));
        }
        if let Err(error) = self.preflight_graceful_transition(session, false) {
            return Err(ScreenGracefulProofError::rejected(abort, error));
        }
        let revision_before = self.transition_revision;
        let transition = match self.complete_operation(session, acknowledgement) {
            Ok(transition) => transition,
            Err(error)
                if self.transition_revision == revision_before
                    && session
                        .pending_stop_matches(expected_operation_id, abort.expected_stream) =>
            {
                return Err(ScreenGracefulProofError::rejected(abort, error));
            }
            Err(error) => return Err(ScreenGracefulProofError::Transition(error)),
        };
        Ok(ScreenGracefulStopAbortCompletion {
            seal_epoch: abort.seal_epoch,
            stopped_stream: abort.expected_stream,
            transition: Some(Box::new(transition)),
        })
    }

    /// Applies the unforgeable acknowledgement returned by one exact
    /// `ScreenSessionAction::execute_source` call.
    pub fn complete_operation(
        &mut self,
        session: &mut ScreenCaptureSession,
        acknowledgement: ScreenOperationAck,
    ) -> Result<ScreenIngressTransition, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if acknowledgement.session_binding != self.source_session_binding {
            return Err(ScreenCaptureError::SourceEventOwnershipMismatch);
        }
        self.apply_internal_event(
            session,
            ScreenSessionEvent::OperationCompleted(acknowledgement),
        )
    }

    /// Applies an owner-stamped failure returned by operation execution.
    pub fn apply_operation_failure(
        &mut self,
        session: &mut ScreenCaptureSession,
        failure: ScreenSourceFailureEnvelope,
    ) -> Result<ScreenIngressTransition, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if failure.owner != self.source_session_binding {
            return Err(ScreenCaptureError::SourceEventOwnershipMismatch);
        }
        self.apply_internal_event(session, ScreenSessionEvent::SourceFailed(failure.failure))
    }

    fn validate_data_stream(
        &self,
        session: &ScreenCaptureSession,
        stream: ScreenStreamStamp,
    ) -> Result<(), ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if session.phase != ScreenCapturePhase::Capturing
            || session.active_stream() != Some(stream)
            || self.active_stream != Some(stream)
            || stream.source_instance() != self.source_instance
            || stream.target() != self.target
            || stream.capture_epoch() != self.capture_epoch
        {
            return Err(ScreenCaptureError::UnexpectedSourceData);
        }
        Ok(())
    }

    fn validate_frame_cursor(
        &self,
        cursor: Option<CursorFrameMetadata>,
    ) -> Result<(), ScreenCaptureError> {
        match self.cursor_policy.mode() {
            CursorCaptureMode::Hidden | CursorCaptureMode::EmbeddedInFrame => {
                if cursor.is_some() {
                    return Err(ScreenCaptureError::CursorMetadataNotNegotiated);
                }
            }
            CursorCaptureMode::Metadata => {
                if let Some(cursor) = cursor {
                    if !self.cursor_policy.include_image_revision()
                        && cursor.image_revision().is_some()
                    {
                        return Err(ScreenCaptureError::CursorImageNotNegotiated);
                    }
                    if !self.cursor_policy.include_clicks()
                        && (cursor.primary_click() || cursor.secondary_click())
                    {
                        return Err(ScreenCaptureError::CursorClickMetadataNotNegotiated);
                    }
                }
                self.cursor_cache
                    .validate_metadata(cursor, self.cursor_policy.include_image_revision())?;
            }
        }
        Ok(())
    }

    /// Executes a permission control action without consuming it on source
    /// failure. Errors therefore leave both the session and action retryable.
    pub fn execute_control_action<S: ScreenCaptureSource>(
        &mut self,
        session: &mut ScreenCaptureSession,
        action: &ScreenSessionAction,
        source: &mut BoundScreenCaptureSource<S>,
        budget: &ScreenOperationBudget<'_>,
    ) -> Result<ScreenIngressTransition, ScreenControlExecutionError> {
        self.validate_session_identity(session)
            .map_err(ScreenControlExecutionError::Contract)?;
        if action.owner != self.source_session_binding
            || action.owner != session.source_session_binding
        {
            return Err(ScreenControlExecutionError::Contract(
                ScreenCaptureError::ActionSessionOwnershipMismatch,
            ));
        }
        if source.binding() != action.owner {
            return Err(ScreenControlExecutionError::Contract(
                ScreenCaptureError::SourceSessionOwnershipMismatch,
            ));
        }
        let event = match action.control_command() {
            ScreenControlCommand::RunPermissionPreflight
                if session.phase == ScreenCapturePhase::AwaitingPreflight =>
            {
                let result = source
                    .preflight(budget)
                    .map_err(ScreenControlExecutionError::Source)?;
                if result.owner != action.owner {
                    return Err(ScreenControlExecutionError::Contract(
                        ScreenCaptureError::SourceEventOwnershipMismatch,
                    ));
                }
                ScreenSessionEvent::PreflightCompleted(result.observation)
            }
            ScreenControlCommand::RequestPermission
                if session.phase == ScreenCapturePhase::AwaitingPermissionResult =>
            {
                let result = source
                    .request_permission(budget)
                    .map_err(ScreenControlExecutionError::Source)?;
                if result.owner != action.owner {
                    return Err(ScreenControlExecutionError::Contract(
                        ScreenCaptureError::SourceEventOwnershipMismatch,
                    ));
                }
                ScreenSessionEvent::PermissionRequestCompleted(result.observation)
            }
            ScreenControlCommand::None
            | ScreenControlCommand::RunPermissionPreflight
            | ScreenControlCommand::RequestPermission => {
                return Err(ScreenControlExecutionError::Contract(
                    ScreenCaptureError::InvalidSessionTransition,
                ));
            }
        };
        self.apply_internal_event(session, event)
            .map_err(ScreenControlExecutionError::Contract)
    }

    /// Transitions the session to `Cancelled`, atomically applies its one
    /// epoch drain, and returns the exact stop action for the native source.
    /// Repeating this call after cancellation is side-effect free.
    pub fn cancel_session(
        &mut self,
        session: &mut ScreenCaptureSession,
    ) -> Result<ScreenIngressTransition, ScreenCaptureError> {
        let transition = self.apply_internal_event(session, ScreenSessionEvent::Cancel)?;
        if transition.drain.is_some() {
            self.queue.record_cancellation();
        }
        Ok(transition)
    }

    /// Applies one opaque event minted by a bound adapter poll. The envelope's
    /// owner is checked before cancellation or event-specific mutation.
    pub fn apply_source_event(
        &mut self,
        session: &mut ScreenCaptureSession,
        envelope: ScreenSourceEventEnvelope<FramePayload, CursorImagePayload>,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenIngressOutcome, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if envelope.owner != self.source_session_binding {
            return Err(ScreenCaptureError::SourceEventOwnershipMismatch);
        }
        self.handle_validated_source_event(session, envelope.event, now_ns, cancellation)
    }

    fn handle_validated_source_event(
        &mut self,
        session: &mut ScreenCaptureSession,
        event: ScreenSourceEvent<FramePayload, CursorImagePayload>,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenIngressOutcome, ScreenCaptureError> {
        if cancellation.is_cancelled() {
            return Ok(ScreenIngressOutcome::Session(Box::new(
                self.cancel_session(session)?,
            )));
        }
        match event {
            ScreenSourceEvent::Frame(frame) => {
                self.validate_data_stream(session, frame.stream())?;
                self.validate_frame_cursor(frame.cursor())?;
                let outcome = self.queue.try_push(frame, now_ns)?;
                session.record_queue_outcome(outcome);
                Ok(ScreenIngressOutcome::Frame(outcome))
            }
            ScreenSourceEvent::CursorImage(image) => {
                self.validate_data_stream(session, image.stream())?;
                if self.cursor_policy.mode() != CursorCaptureMode::Metadata
                    || !self.cursor_policy.include_image_revision()
                {
                    return Err(ScreenCaptureError::CursorImageNotNegotiated);
                }
                self.cursor_cache.apply(image)?;
                Ok(ScreenIngressOutcome::CursorImageAccepted)
            }
            ScreenSourceEvent::PermissionChanged(observation) => Ok(ScreenIngressOutcome::Session(
                Box::new(self.apply_internal_event(
                    session,
                    ScreenSessionEvent::PermissionChanged(observation),
                )?),
            )),
            ScreenSourceEvent::TargetChanged(change) => {
                Ok(ScreenIngressOutcome::Session(Box::new(
                    self.apply_internal_event(session, ScreenSessionEvent::TargetChanged(change))?,
                )))
            }
            ScreenSourceEvent::Sleep(stamp) => Ok(ScreenIngressOutcome::Session(Box::new(
                self.apply_internal_event(session, ScreenSessionEvent::Sleep(stamp))?,
            ))),
            ScreenSourceEvent::Wake(stamp) => Ok(ScreenIngressOutcome::Session(Box::new(
                self.apply_internal_event(session, ScreenSessionEvent::Wake(stamp))?,
            ))),
            ScreenSourceEvent::ProtectedContentDetected(stamp) => Ok(
                ScreenIngressOutcome::Session(Box::new(self.apply_internal_event(
                    session,
                    ScreenSessionEvent::ProtectedContentDetected(stamp),
                )?)),
            ),
            ScreenSourceEvent::ProtectedContentCleared(stamp) => Ok(ScreenIngressOutcome::Session(
                Box::new(self.apply_internal_event(
                    session,
                    ScreenSessionEvent::ProtectedContentCleared(stamp),
                )?),
            )),
            ScreenSourceEvent::Failure(failure) => Ok(ScreenIngressOutcome::Session(Box::new(
                self.apply_internal_event(session, ScreenSessionEvent::SourceFailed(failure))?,
            ))),
        }
    }

    /// Polls a source through the normalized ingress boundary. Raw poll
    /// failures are bound to the exact live operation before they can affect
    /// session state; cancellation takes the single terminal cancel path.
    pub fn poll_source<S>(
        &mut self,
        session: &mut ScreenCaptureSession,
        source: &mut BoundScreenCaptureSource<S>,
        budget: &ScreenOperationBudget<'_>,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<Option<ScreenIngressOutcome>, ScreenCaptureError>
    where
        S: ScreenCaptureSource<
                FramePayload = FramePayload,
                CursorImagePayload = CursorImagePayload,
            >,
    {
        self.validate_bound_source(session, source)?;
        if cancellation.is_cancelled() {
            return Ok(Some(ScreenIngressOutcome::Session(Box::new(
                self.cancel_session(session)?,
            ))));
        }
        match source.poll_owned_event(budget) {
            Ok(None) => Ok(None),
            Ok(Some(ScreenSourceEventEnvelope {
                owner,
                event: ScreenSourceEvent::Failure(failure),
            })) => {
                if owner != self.source_session_binding {
                    return Err(ScreenCaptureError::SourceEventOwnershipMismatch);
                }
                let failure = session.bind_failure_to_live_operation(failure)?;
                self.apply_source_event(
                    session,
                    ScreenSourceEventEnvelope {
                        owner,
                        event: ScreenSourceEvent::Failure(failure),
                    },
                    now_ns,
                    cancellation,
                )
                .map(Some)
            }
            Ok(Some(envelope)) => self
                .apply_source_event(session, envelope, now_ns, cancellation)
                .map(Some),
            Err(failure) if failure.owner != self.source_session_binding => {
                Err(ScreenCaptureError::SourceEventOwnershipMismatch)
            }
            Err(failure) if failure.code() == ScreenSourceFailureCode::Cancelled => Ok(Some(
                ScreenIngressOutcome::Session(Box::new(self.cancel_session(session)?)),
            )),
            Err(failure) => {
                let failure = session.bind_failure_to_live_operation(failure.failure)?;
                self.apply_source_event(
                    session,
                    ScreenSourceEventEnvelope {
                        owner: self.source_session_binding,
                        event: ScreenSourceEvent::Failure(failure),
                    },
                    now_ns,
                    cancellation,
                )
                .map(Some)
            }
        }
    }

    pub fn try_pop(
        &mut self,
        session: &mut ScreenCaptureSession,
        now_ns: u64,
        cancellation: &CancellationToken,
    ) -> Result<ScreenIngressPopOutcome<FramePayload>, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        if cancellation.is_cancelled() {
            return Ok(ScreenIngressPopOutcome::Cancelled(Box::new(
                self.cancel_session(session)?,
            )));
        }
        match self.queue.try_pop(now_ns)? {
            ScreenQueuePopOutcome::Frame(frame) => Ok(ScreenIngressPopOutcome::Frame(frame)),
            ScreenQueuePopOutcome::Empty => Ok(ScreenIngressPopOutcome::Empty),
        }
    }

    pub(crate) fn peek_next_frame(
        &mut self,
        session: &ScreenCaptureSession,
        now_ns: u64,
    ) -> Result<Option<ScreenFrameAdmission>, ScreenCaptureError> {
        self.validate_session_identity(session)?;
        self.queue.peek_admission(now_ns)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScreenCaptureError {
    #[error("screen session identity is invalid")]
    InvalidSessionId,
    #[error("screen source instance identity is invalid")]
    InvalidSourceInstanceId,
    #[error("screen source object is owned by another capture session")]
    SourceSessionOwnershipMismatch,
    #[error("screen source event/result is owned by another capture session")]
    SourceEventOwnershipMismatch,
    #[error("screen action is owned by another capture session")]
    ActionSessionOwnershipMismatch,
    #[error("screen epoch transition is owned by another capture session")]
    EpochTransitionOwnershipMismatch,
    #[error("screen target identity is invalid")]
    InvalidTargetId,
    #[error("screen target epoch is invalid")]
    InvalidTargetEpoch,
    #[error("screen capture epoch is invalid")]
    InvalidCaptureEpoch,
    #[error("screen capture epoch is exhausted")]
    CaptureEpochExhausted,
    #[error("screen target kind does not match the descriptor")]
    TargetKindMismatch,
    #[error("screen target topology generation is invalid")]
    InvalidTopologyGeneration,
    #[error("screen topology event envelope is invalid")]
    InvalidTopologyEvent,
    #[error("screen target catalog is bound to another source or generation")]
    TargetCatalogBindingMismatch,
    #[error("screen target binding is absent from the current catalog")]
    UnknownTargetBinding,
    #[error("screen target descriptor does not match the current catalog")]
    ForgedTargetDescriptor,
    #[error("screen target snapshot exceeds its bounded capacity")]
    TooManyTargets,
    #[error("screen target snapshot contains a duplicate identity")]
    DuplicateTarget,
    #[error("screen region target references a display absent from its catalog")]
    MissingContainingDisplay,
    #[error("screen region target embeds a forged display transform")]
    ForgedRegionTransform,
    #[error("screen geometry must have non-zero dimensions")]
    EmptyGeometry,
    #[error("screen geometry overflows its coordinate space")]
    GeometryOverflow,
    #[error("screen geometry falls outside its display")]
    GeometryOutsideDisplay,
    #[error("logical and physical display geometry are inconsistent")]
    InconsistentDisplayGeometry,
    #[error("screen DPI scale is invalid")]
    InvalidDpiScale,
    #[error("cursor policy is invalid")]
    InvalidCursorPolicy,
    #[error("cursor metadata falls outside the negotiated frame")]
    InvalidCursorMetadata,
    #[error("cursor image descriptor is invalid")]
    InvalidCursorImage,
    #[error("cursor image payload allocation does not match its declared retained bytes")]
    CursorPayloadAccountingMismatch,
    #[error("visible cursor metadata omitted its required image revision")]
    MissingCursorImageRevision,
    #[error("cursor metadata references an image that is not cached")]
    MissingCursorImage,
    #[error("cursor metadata references a stale image revision")]
    StaleCursorImageRevision,
    #[error("cursor image revisions are not strictly monotonic")]
    NonMonotonicCursorImageRevision,
    #[error("cursor metadata was not negotiated for this stream")]
    CursorMetadataNotNegotiated,
    #[error("cursor image revisions were not negotiated for this stream")]
    CursorImageNotNegotiated,
    #[error("cursor click metadata was not negotiated for this stream")]
    CursorClickMetadataNotNegotiated,
    #[error("cursor coordinate space is unsupported for the selected target")]
    UnsupportedCursorCoordinateSpace,
    #[error("screen source capability contract is incompatible")]
    IncompatibleContract,
    #[error("screen source capabilities are invalid")]
    InvalidCapabilities,
    #[error("screen source is bound to another platform")]
    SourcePlatformMismatch,
    #[error("screen source capabilities changed after negotiation")]
    SourceCapabilitiesChanged,
    #[error("screen source catalog changed after negotiation")]
    SourceCatalogChanged,
    #[error("screen negotiation refresh does not match the live session")]
    InvalidNegotiationRefresh,
    #[error("screen capture queue policy is invalid")]
    InvalidQueuePolicy,
    #[error("screen capture recovery policy is invalid")]
    InvalidRecoveryPolicy,
    #[error("too many windows were requested for exclusion")]
    TooManyExcludedWindows,
    #[error("window exclusion contains an invalid target")]
    InvalidWindowExclusion,
    #[error("screen video frame specification is invalid")]
    InvalidVideoFrameSpec(#[source] Box<crate::CaptureError>),
    #[error("screen frame rate is invalid")]
    InvalidFrameRate,
    #[error("a required screen-source capability is unavailable")]
    RequiredCapabilityUnavailable,
    #[error("screen target kind is unsupported")]
    UnsupportedTargetKind,
    #[error("cursor policy is unsupported")]
    UnsupportedCursorPolicy,
    #[error("screen frame specification is unsupported")]
    UnsupportedFrameSpec,
    #[error("window exclusion is unsupported")]
    UnsupportedWindowExclusion,
    #[error("target recovery policy is unsupported")]
    UnsupportedRecoveryPolicy,
    #[error("protected-content signaling is unavailable")]
    ProtectedContentSignalUnavailable,
    #[error("screen frame envelope is invalid")]
    InvalidFrameEnvelope,
    #[error("screen frame payload allocation does not match its declared retained bytes")]
    FramePayloadAccountingMismatch,
    #[error("owned CPU frame payload cannot claim a native-memory frame type")]
    FramePayloadMemoryMismatch,
    #[error("screen frame specification changed without renegotiation")]
    FrameSpecChangedWithoutNegotiation,
    #[error("screen frame belongs to another capture epoch")]
    CaptureEpochMismatch,
    #[error("screen frame belongs to another target epoch")]
    TargetEpochMismatch,
    #[error("screen capture epoch did not advance")]
    NonMonotonicCaptureEpoch,
    #[error("screen frame sequence is not monotonic")]
    NonMonotonicFrameSequence,
    #[error("screen frame timestamp is not monotonic")]
    NonMonotonicFrameTimestamp,
    #[error("screen frame queue clock moved backwards")]
    QueueClockMovedBackwards,
    #[error("screen frame queue accounting is inconsistent")]
    QueueAccountingCorrupt,
    #[error("screen capture ingress must be drained before graceful stop")]
    GracefulStopRequiresDrainedIngress,
    #[error("screen capture operation timeout is invalid")]
    InvalidOperationTimeout,
    #[error("screen operation sequence is exhausted")]
    OperationSequenceExhausted,
    #[error("screen stream sequence is exhausted")]
    StreamSequenceExhausted,
    #[error("screen ingress transition sequence is exhausted")]
    IngressTransitionSequenceExhausted,
    #[error("another screen operation is already pending")]
    OperationAlreadyPending,
    #[error("screen operation acknowledgement does not match the pending operation")]
    MismatchedOperationAck,
    #[error("screen source failure is unbound, stale, or replayed")]
    StaleSourceFailure,
    #[error("screen data belongs to another stream identity")]
    StreamIdentityMismatch,
    #[error("screen capture session transition is invalid")]
    InvalidSessionTransition,
    #[error("screen source emitted frame data outside a capturing phase")]
    UnexpectedSourceData,
    #[error("screen ingress belongs to another capture session")]
    IngressSessionMismatch,
    #[error("screen control epoch is invalid")]
    InvalidControlEpoch,
    #[error("screen control sequence is invalid")]
    InvalidControlSequence,
    #[error("screen source event belongs to another source instance")]
    CrossSourceEvent,
    #[error("screen control event belongs to a stale epoch")]
    StaleControlEpoch,
    #[error("screen control event is stale or replayed")]
    StaleControlEvent,
    #[error("screen target topology event is stale")]
    StaleTopologyEvent,
    #[error("screen target epoch is stale")]
    StaleTargetEpoch,
}
