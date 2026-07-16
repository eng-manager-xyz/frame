//! Provider-neutral Studio Mode contracts.
//!
//! The module owns durable, versioned project state, exact timeline semantics,
//! recording/recovery fencing, and render orchestration. Durable filesystem
//! reference adapters execute storage, recording sinks, import, preview-plan,
//! and render-receipt flows; platform bridges still provide native capture and
//! playable codec/compositor implementations behind the narrow ports here.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{Sha256Digest, strong_sha256};

pub const STUDIO_PROJECT_VERSION: u16 = 1;
pub const STUDIO_ASSET_VERSION: u16 = 1;
pub const STUDIO_EDIT_VERSION: u16 = 1;
pub const STUDIO_JOURNAL_VERSION: u16 = 2;
pub const STUDIO_RENDER_PROTOCOL_VERSION: u16 = 1;
pub const MAX_STUDIO_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_STUDIO_ASSETS: usize = 64;
pub const MAX_STUDIO_EDITS: usize = 1_024;
pub const MAX_STUDIO_RECEIPTS: usize = 200_000;
pub const MAX_STUDIO_VFR_SAMPLES: usize = 2_000_000;
pub const MAX_STUDIO_SIMULATED_TIMESTAMPS: usize = 1_000_000;
pub const MAX_STUDIO_COVERAGE_RANGES: usize = 4_096;
pub const MAX_STUDIO_GAP_INSTRUCTIONS: usize = 32_768;
pub const MAX_STUDIO_QUEUE_BUFFERS: u32 = 512;
pub const MAX_STUDIO_QUEUE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_STUDIO_QUEUE_TIME_NS: u64 = 5_000_000_000;
pub const MAX_STUDIO_PAYLOAD_CHUNK_BYTES: usize = 1024 * 1024;
pub const MAX_STUDIO_CONTROL_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_STUDIO_SOURCE_NAME_BYTES: usize = 240;
pub const MAX_STUDIO_RENDER_SESSIONS: usize = 64;
pub const MAX_STUDIO_RENDER_POLL_WAIT: Duration = Duration::from_secs(30);

macro_rules! opaque_id {
    ($name:ident, $error:ident, $label:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            /// The caller must supply 128 bits from a CSPRNG. All-zero values
            /// are reserved so zero-filled storage cannot become an identity.
            pub fn from_csprng(bytes: [u8; 16]) -> Result<Self, StudioError> {
                if bytes.iter().all(|byte| *byte == 0) {
                    return Err(StudioError::$error);
                }
                Ok(Self(bytes))
            }

            #[allow(dead_code)]
            const fn canonical_bytes(self) -> [u8; 16] {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($label, "(<redacted>)"))
            }
        }
    };
}

opaque_id!(StudioProjectId, InvalidProjectId, "StudioProjectId");
opaque_id!(StudioAssetId, InvalidAssetId, "StudioAssetId");
opaque_id!(StudioOperationId, InvalidOperationId, "StudioOperationId");
opaque_id!(StudioWorkerId, InvalidWorkerId, "StudioWorkerId");
opaque_id!(StudioExportId, InvalidExportId, "StudioExportId");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectCompatibility {
    Supported,
    Migratable { from: u16, to: u16 },
    UnsupportedNewer { found: u16, supported: u16 },
}

#[must_use]
pub const fn inspect_project_version(version: u16) -> ProjectCompatibility {
    match version {
        STUDIO_PROJECT_VERSION => ProjectCompatibility::Supported,
        0 => ProjectCompatibility::Migratable {
            from: 0,
            to: STUDIO_PROJECT_VERSION,
        },
        found => ProjectCompatibility::UnsupportedNewer {
            found,
            supported: STUDIO_PROJECT_VERSION,
        },
    }
}

/// Exact, positive media timebase measured in ticks per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeBase(u32);

impl TimeBase {
    pub fn new(ticks_per_second: u32) -> Result<Self, StudioError> {
        if ticks_per_second == 0 {
            return Err(StudioError::InvalidTimeBase);
        }
        Ok(Self(ticks_per_second))
    }

    #[must_use]
    pub const fn ticks_per_second(self) -> u32 {
        self.0
    }
}

/// An exact non-negative point or duration. Arithmetic uses checked `i128`
/// cross-products and never converts through floating point.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RationalTime {
    ticks: u64,
    time_base: TimeBase,
}

impl RationalTime {
    pub fn new(ticks: u64, time_base: TimeBase) -> Self {
        Self { ticks, time_base }
    }

    pub fn from_nanos(nanos: u64) -> Self {
        Self {
            ticks: nanos,
            time_base: TimeBase(1_000_000_000),
        }
    }

    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.ticks
    }

    #[must_use]
    pub const fn time_base(self) -> TimeBase {
        self.time_base
    }

    pub fn compare(self, other: Self) -> Result<std::cmp::Ordering, StudioError> {
        let left = u128::from(self.ticks)
            .checked_mul(u128::from(other.time_base.0))
            .ok_or(StudioError::TimelineOverflow)?;
        let right = u128::from(other.ticks)
            .checked_mul(u128::from(self.time_base.0))
            .ok_or(StudioError::TimelineOverflow)?;
        Ok(left.cmp(&right))
    }

    pub fn checked_sub(self, other: Self) -> Result<ExactDuration, StudioError> {
        if self.compare(other)? == std::cmp::Ordering::Less {
            return Err(StudioError::TimelineUnderflow);
        }
        let denominator = u128::from(self.time_base.0)
            .checked_mul(u128::from(other.time_base.0))
            .ok_or(StudioError::TimelineOverflow)?;
        let left = u128::from(self.ticks)
            .checked_mul(u128::from(other.time_base.0))
            .ok_or(StudioError::TimelineOverflow)?;
        let right = u128::from(other.ticks)
            .checked_mul(u128::from(self.time_base.0))
            .ok_or(StudioError::TimelineOverflow)?;
        ExactDuration::new(left - right, denominator)
    }
}

impl fmt::Debug for RationalTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}s", self.ticks, self.time_base.0)
    }
}

/// Reduced exact duration in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactDuration {
    numerator: u128,
    denominator: u128,
}

impl ExactDuration {
    pub fn new(numerator: u128, denominator: u128) -> Result<Self, StudioError> {
        if denominator == 0 {
            return Err(StudioError::InvalidTimeBase);
        }
        let divisor = gcd_u128(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self {
            numerator: 0,
            denominator: 1,
        }
    }

    #[must_use]
    pub const fn numerator(self) -> u128 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u128 {
        self.denominator
    }

    pub fn checked_add(self, other: Self) -> Result<Self, StudioError> {
        let common = gcd_u128(self.denominator, other.denominator);
        let left_multiplier = other.denominator / common;
        let right_multiplier = self.denominator / common;
        let left = self
            .numerator
            .checked_mul(left_multiplier)
            .ok_or(StudioError::TimelineOverflow)?;
        let right = other
            .numerator
            .checked_mul(right_multiplier)
            .ok_or(StudioError::TimelineOverflow)?;
        let numerator = left
            .checked_add(right)
            .ok_or(StudioError::TimelineOverflow)?;
        let denominator = self
            .denominator
            .checked_mul(left_multiplier)
            .ok_or(StudioError::TimelineOverflow)?;
        Self::new(numerator, denominator)
    }

    pub fn scaled(self, numerator: u32, denominator: u32) -> Result<Self, StudioError> {
        if numerator == 0 || denominator == 0 {
            return Err(StudioError::InvalidSpeed);
        }
        let mut value_numerator = self.numerator;
        let mut value_denominator = self.denominator;
        let mut scale_numerator = u128::from(numerator);
        let mut scale_denominator = u128::from(denominator);
        let cross = gcd_u128(value_numerator, scale_denominator);
        value_numerator /= cross;
        scale_denominator /= cross;
        let cross = gcd_u128(scale_numerator, value_denominator);
        scale_numerator /= cross;
        value_denominator /= cross;
        Self::new(
            value_numerator
                .checked_mul(scale_numerator)
                .ok_or(StudioError::TimelineOverflow)?,
            value_denominator
                .checked_mul(scale_denominator)
                .ok_or(StudioError::TimelineOverflow)?,
        )
    }

    pub fn floor_ticks(self, time_base: TimeBase) -> Result<u64, StudioError> {
        let ticks = self
            .numerator
            .checked_mul(u128::from(time_base.0))
            .ok_or(StudioError::TimelineOverflow)?
            / self.denominator;
        u64::try_from(ticks).map_err(|_| StudioError::TimelineOverflow)
    }
}

const fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    if left == 0 { 1 } else { left }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrackKind {
    Screen,
    Camera,
    Microphone,
    SystemAudio,
}

impl TrackKind {
    const fn tag(self) -> u8 {
        match self {
            Self::Screen => 1,
            Self::Camera => 2,
            Self::Microphone => 3,
            Self::SystemAudio => 4,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, StudioError> {
        match tag {
            1 => Ok(Self::Screen),
            2 => Ok(Self::Camera),
            3 => Ok(Self::Microphone),
            4 => Ok(Self::SystemAudio),
            _ => Err(StudioError::MalformedDocument),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetChecksum(Sha256Digest);

impl AssetChecksum {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, StudioError> {
        Sha256Digest::from_bytes(bytes)
            .map(Self)
            .map_err(|_| StudioError::InvalidChecksum)
    }

    pub fn from_content(bytes: &[u8]) -> Self {
        Self(strong_sha256(bytes))
    }
}

impl fmt::Debug for AssetChecksum {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AssetChecksum(<redacted>)")
    }
}

/// A relative, canonical storage name. Paths, parent traversal and platform
/// separators are rejected so a decoded project cannot escape its asset root.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StudioSourceName(String);

impl StudioSourceName {
    pub fn new(value: impl Into<String>) -> Result<Self, StudioError> {
        let value = value.into();
        let stem = value.split('.').next().unwrap_or_default();
        let windows_reserved = matches!(stem, "con" | "prn" | "aux" | "nul")
            || stem
                .strip_prefix("com")
                .or_else(|| stem.strip_prefix("lpt"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes().first(), Some(b'1'..=b'9'))
                });
        if value.is_empty()
            || value.len() > MAX_STUDIO_SOURCE_NAME_BYTES
            || value.starts_with('.')
            || value.ends_with('.')
            || value.contains("..")
            || value.contains('/')
            || value.contains('\\')
            || windows_reserved
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
            })
        {
            return Err(StudioError::InvalidSourceName);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for StudioSourceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioSourceName(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCommitState {
    Temporary,
    DurableOriginal,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StudioAsset {
    pub version: u16,
    pub id: StudioAssetId,
    pub track: TrackKind,
    pub source_name: StudioSourceName,
    pub byte_len: u64,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub checksum: AssetChecksum,
    pub commit_state: AssetCommitState,
}

impl fmt::Debug for StudioAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudioAsset")
            .field("version", &self.version)
            .field("id", &self.id)
            .field("track", &self.track)
            .field("source_name", &self.source_name)
            .field("byte_len", &self.byte_len)
            .field("start", &self.start)
            .field("duration", &self.duration)
            .field("checksum", &self.checksum)
            .field("commit_state", &self.commit_state)
            .finish()
    }
}

impl StudioAsset {
    pub fn validate(&self) -> Result<(), StudioError> {
        if self.version != STUDIO_ASSET_VERSION {
            return Err(StudioError::UnsupportedAssetVersion(self.version));
        }
        if self.byte_len == 0 || self.duration.ticks == 0 {
            return Err(StudioError::InvalidAsset);
        }
        self.end()?;
        Ok(())
    }

    pub fn end(&self) -> Result<ExactDuration, StudioError> {
        let start_ticks = self
            .start
            .checked_sub(RationalTime::new(0, self.start.time_base))?;
        let duration = self
            .duration
            .checked_sub(RationalTime::new(0, self.duration.time_base))?;
        start_ticks.checked_add(duration)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudioState {
    Empty,
    Recording,
    Recovering,
    Editing,
    Previewing,
    Exporting,
    Completed,
    Cancelled,
    Failed,
}

impl StudioState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }

    const fn tag(self) -> u8 {
        match self {
            Self::Empty => 1,
            Self::Recording => 2,
            Self::Recovering => 3,
            Self::Editing => 4,
            Self::Previewing => 5,
            Self::Exporting => 6,
            Self::Completed => 7,
            Self::Cancelled => 8,
            Self::Failed => 9,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, StudioError> {
        match tag {
            1 => Ok(Self::Empty),
            2 => Ok(Self::Recording),
            3 => Ok(Self::Recovering),
            4 => Ok(Self::Editing),
            5 => Ok(Self::Previewing),
            6 => Ok(Self::Exporting),
            7 => Ok(Self::Completed),
            8 => Ok(Self::Cancelled),
            9 => Ok(Self::Failed),
            _ => Err(StudioError::MalformedDocument),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPreset {
    ScreenOnly,
    CameraBubble,
    SideBySide,
    CameraFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedRect {
    pub x_millionths: u32,
    pub y_millionths: u32,
    pub width_millionths: u32,
    pub height_millionths: u32,
}

impl NormalizedRect {
    pub fn validate(self) -> Result<Self, StudioError> {
        if self.width_millionths == 0
            || self.height_millionths == 0
            || self
                .x_millionths
                .checked_add(self.width_millionths)
                .is_none_or(|right| right > 1_000_000)
            || self
                .y_millionths
                .checked_add(self.height_millionths)
                .is_none_or(|bottom| bottom > 1_000_000)
        {
            return Err(StudioError::InvalidTransform);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundStyle {
    Transparent,
    SolidRgb { red: u8, green: u8, blue: u8 },
    Blur { radius_milli: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    Trim {
        start: RationalTime,
        end: RationalTime,
    },
    Split {
        at: RationalTime,
    },
    DeleteRange {
        start: RationalTime,
        end: RationalTime,
    },
    Speed {
        start: RationalTime,
        end: RationalTime,
        numerator: u32,
        denominator: u32,
    },
    AudioGain {
        track: TrackKind,
        start: RationalTime,
        end: RationalTime,
        gain_millibels: i32,
        muted: bool,
    },
    Layout {
        start: RationalTime,
        end: RationalTime,
        preset: LayoutPreset,
    },
    CameraTransform {
        start: RationalTime,
        end: RationalTime,
        rect: NormalizedRect,
        corner_radius_milli: u16,
    },
    CursorTransform {
        start: RationalTime,
        end: RationalTime,
        scale_milli: u16,
        hidden: bool,
    },
    Background {
        start: RationalTime,
        end: RationalTime,
        style: BackgroundStyle,
    },
}

impl EditOperation {
    fn range(&self) -> Option<(RationalTime, RationalTime)> {
        match self {
            Self::Trim { start, end }
            | Self::DeleteRange { start, end }
            | Self::Speed { start, end, .. }
            | Self::AudioGain { start, end, .. }
            | Self::Layout { start, end, .. }
            | Self::CameraTransform { start, end, .. }
            | Self::CursorTransform { start, end, .. }
            | Self::Background { start, end, .. } => Some((*start, *end)),
            Self::Split { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSpec {
    pub version: u16,
    pub revision: u64,
    pub operations: Vec<EditOperation>,
}

impl Default for EditSpec {
    fn default() -> Self {
        Self {
            version: STUDIO_EDIT_VERSION,
            revision: 0,
            operations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioProjectManifest {
    pub version: u16,
    pub id: StudioProjectId,
    pub revision: u64,
    pub state: StudioState,
    pub assets: Vec<StudioAsset>,
    pub edits: EditSpec,
}

impl StudioProjectManifest {
    pub fn validate(&self) -> Result<(), StudioError> {
        if self.version != STUDIO_PROJECT_VERSION {
            return Err(StudioError::UnsupportedProjectVersion(self.version));
        }
        if self.assets.len() > MAX_STUDIO_ASSETS || self.edits.operations.len() > MAX_STUDIO_EDITS {
            return Err(StudioError::DocumentTooLarge);
        }
        if self.state != StudioState::Empty && self.revision == 0 {
            return Err(StudioError::InvalidProjectState);
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut screen_ranges = Vec::new();
        let mut has_screen = false;
        for asset in &self.assets {
            asset.validate()?;
            if !ids.insert(asset.id) || !names.insert(asset.source_name.clone()) {
                return Err(StudioError::AssetConflict);
            }
            if asset.commit_state != AssetCommitState::DurableOriginal
                && !matches!(self.state, StudioState::Recording | StudioState::Recovering)
            {
                return Err(StudioError::TemporaryAssetEscapedRecording);
            }
            if asset.track == TrackKind::Screen {
                has_screen = true;
                screen_ranges.push((
                    asset
                        .start
                        .checked_sub(RationalTime::new(0, asset.start.time_base))?,
                    asset.end()?,
                ));
            }
        }
        if self.edits.version != STUDIO_EDIT_VERSION {
            return Err(StudioError::UnsupportedEditVersion(self.edits.version));
        }
        if self.edits.revision > self.revision {
            return Err(StudioError::InvalidProjectState);
        }
        validate_edit_shape(&self.edits)?;
        let requires_renderable_screen = matches!(
            self.state,
            StudioState::Editing
                | StudioState::Previewing
                | StudioState::Exporting
                | StudioState::Completed
        );
        if requires_renderable_screen && (self.assets.is_empty() || !has_screen) {
            return Err(StudioError::NoAssets);
        }
        screen_ranges.sort_by(|left, right| compare_duration(left.0, right.0));
        let source_end = screen_ranges
            .last()
            .map_or(ExactDuration::zero(), |range| range.1);
        if requires_renderable_screen
            && (screen_ranges
                .first()
                .is_none_or(|range| range.0 != ExactDuration::zero())
                || screen_ranges.windows(2).any(|pair| pair[0].1 != pair[1].0))
        {
            return Err(StudioError::InvalidSourceSet);
        }
        if self.state == StudioState::Empty
            && (!self.assets.is_empty() || !self.edits.operations.is_empty())
        {
            return Err(StudioError::InvalidProjectState);
        }
        for operation in &self.edits.operations {
            let endpoints = match operation {
                EditOperation::Split { at } => Some((*at, *at)),
                _ => operation.range(),
            };
            if let Some((start, end)) = endpoints {
                let start = start.checked_sub(RationalTime::new(0, start.time_base))?;
                let end = end.checked_sub(RationalTime::new(0, end.time_base))?;
                if compare_duration(start, source_end) == std::cmp::Ordering::Greater
                    || compare_duration(end, source_end) == std::cmp::Ordering::Greater
                {
                    return Err(StudioError::EditOutsideTimeline);
                }
                if matches!(operation, EditOperation::Split { .. })
                    && (start == ExactDuration::zero() || start == source_end)
                {
                    return Err(StudioError::EditOutsideTimeline);
                }
            }
        }
        if requires_renderable_screen && !edit_retains_output(&self.edits, source_end)? {
            return Err(StudioError::EmptyOutput);
        }
        Ok(())
    }
}

fn edit_retains_output(edits: &EditSpec, source_end: ExactDuration) -> Result<bool, StudioError> {
    let mut active_start = ExactDuration::zero();
    let mut active_end = source_end;
    let mut deleted = Vec::new();
    for operation in &edits.operations {
        match operation {
            EditOperation::Trim { start, end } => {
                active_start = start.checked_sub(RationalTime::new(0, start.time_base))?;
                active_end = end.checked_sub(RationalTime::new(0, end.time_base))?;
            }
            EditOperation::DeleteRange { start, end } => deleted.push((
                start.checked_sub(RationalTime::new(0, start.time_base))?,
                end.checked_sub(RationalTime::new(0, end.time_base))?,
            )),
            _ => {}
        }
    }
    deleted.sort_by(|left, right| compare_duration(left.0, right.0));
    let mut cursor = active_start;
    for (start, end) in deleted {
        if compare_duration(start, cursor) == std::cmp::Ordering::Greater {
            return Ok(true);
        }
        if compare_duration(end, cursor) == std::cmp::Ordering::Greater {
            cursor = end;
        }
        if compare_duration(cursor, active_end) != std::cmp::Ordering::Less {
            return Ok(false);
        }
    }
    Ok(compare_duration(cursor, active_end) == std::cmp::Ordering::Less)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalBoundary {
    Created,
    RecordingGraphPrepared,
    CaptureStarted,
    TempAssetReserved,
    TempAssetDurable,
    AssetCommitRequested,
    AssetCommitted,
    RecordingStopped,
    EditSavePrepared,
    EditSaveCommitted,
    RenderPrepared,
    RenderRunning,
    RenderFinalizing,
    RenderCommitted,
    RenderCancelled,
    FailedRecoverably,
}

impl JournalBoundary {
    const fn tag(self) -> u8 {
        match self {
            Self::Created => 1,
            Self::RecordingGraphPrepared => 2,
            Self::CaptureStarted => 3,
            Self::TempAssetReserved => 4,
            Self::TempAssetDurable => 5,
            Self::AssetCommitRequested => 6,
            Self::AssetCommitted => 7,
            Self::RecordingStopped => 8,
            Self::EditSavePrepared => 9,
            Self::EditSaveCommitted => 10,
            Self::RenderPrepared => 11,
            Self::RenderRunning => 12,
            Self::RenderFinalizing => 13,
            Self::RenderCommitted => 14,
            Self::RenderCancelled => 15,
            Self::FailedRecoverably => 16,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, StudioError> {
        match tag {
            1 => Ok(Self::Created),
            2 => Ok(Self::RecordingGraphPrepared),
            3 => Ok(Self::CaptureStarted),
            4 => Ok(Self::TempAssetReserved),
            5 => Ok(Self::TempAssetDurable),
            6 => Ok(Self::AssetCommitRequested),
            7 => Ok(Self::AssetCommitted),
            8 => Ok(Self::RecordingStopped),
            9 => Ok(Self::EditSavePrepared),
            10 => Ok(Self::EditSaveCommitted),
            11 => Ok(Self::RenderPrepared),
            12 => Ok(Self::RenderRunning),
            13 => Ok(Self::RenderFinalizing),
            14 => Ok(Self::RenderCommitted),
            15 => Ok(Self::RenderCancelled),
            16 => Ok(Self::FailedRecoverably),
            _ => Err(StudioError::MalformedDocument),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptKind {
    GraphPrepared,
    CaptureStarted,
    TempReserved,
    TempDurable,
    AssetCommitRequested,
    AssetCommitted,
    RecordingStopped,
    EditPrepared,
    EditCommitted,
    RenderPrepared,
    RenderStarted,
    RenderFinalizing,
    RenderCommitted,
    PartialDeleted,
    RecoveryApplied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioOperationReceipt {
    pub operation_id: StudioOperationId,
    pub kind: ReceiptKind,
    pub command_digest: Sha256Digest,
    pub outcome_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAssetCommit {
    pub operation_id: StudioOperationId,
    pub asset: StudioAsset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEditSave {
    pub operation_id: StudioOperationId,
    pub expected_project_revision: u64,
    pub edits: EditSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRender {
    pub operation_id: StudioOperationId,
    pub export_id: StudioExportId,
    /// Fence presented to the renderer. It remains immutable even when a
    /// recovery worker later takes ownership of the journal at a newer fence.
    pub fence: u64,
    pub source_set_digest: Sha256Digest,
    pub plan_digest: Sha256Digest,
    pub render_spec_digest: Sha256Digest,
    pub profile: ExportProfile,
    pub output_name: StudioSourceName,
    /// Exact output identity observed from the renderer and durably committed
    /// before a completed output reservation may be released.
    pub terminal_receipt: Option<RenderReceipt>,
}

impl PendingRender {
    pub fn new(
        operation_id: StudioOperationId,
        export_id: StudioExportId,
        fence: u64,
        source_set_digest: Sha256Digest,
        plan_digest: Sha256Digest,
        profile: ExportProfile,
        output_name: StudioSourceName,
    ) -> Result<Self, StudioError> {
        if fence == 0 {
            return Err(StudioError::InvalidRenderTicket);
        }
        let render_spec_digest =
            digest_render_identity(source_set_digest, plan_digest, profile, &output_name)?;
        Ok(Self {
            operation_id,
            export_id,
            fence,
            source_set_digest,
            plan_digest,
            render_spec_digest,
            profile,
            output_name,
            terminal_receipt: None,
        })
    }

    fn validate_terminal_receipt(&self, project_id: StudioProjectId) -> Result<(), StudioError> {
        let Some(receipt) = &self.terminal_receipt else {
            return Ok(());
        };
        if receipt.project_id != project_id
            || receipt.export_id != self.export_id
            || receipt.operation_id != self.operation_id
            || receipt.fence != self.fence
            || receipt.source_set_digest != self.source_set_digest
            || receipt.plan_digest != self.plan_digest
            || receipt.render_spec_digest != self.render_spec_digest
            || receipt.profile.profile != self.profile
            || receipt.output_name != self.output_name
            || receipt.output_bytes == 0
        {
            return Err(StudioError::JournalCorrupt);
        }
        receipt.profile.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioJournalSnapshot {
    pub version: u16,
    pub project_id: StudioProjectId,
    pub revision: u64,
    pub fence: u64,
    pub owner: StudioWorkerId,
    pub boundary: JournalBoundary,
    pub last_operation_id: Option<StudioOperationId>,
    pub pending_asset: Option<PendingAssetCommit>,
    pub pending_edit: Option<PendingEditSave>,
    pub pending_render: Option<PendingRender>,
    pub receipts: BTreeMap<StudioOperationId, StudioOperationReceipt>,
}

impl StudioJournalSnapshot {
    pub fn validate(&self) -> Result<(), StudioError> {
        if self.version != STUDIO_JOURNAL_VERSION {
            return Err(StudioError::UnsupportedJournalVersion(self.version));
        }
        if self.revision == 0 || self.fence == 0 || self.receipts.len() > MAX_STUDIO_RECEIPTS {
            return Err(StudioError::JournalCorrupt);
        }
        for (operation_id, receipt) in &self.receipts {
            if operation_id != &receipt.operation_id {
                return Err(StudioError::JournalCorrupt);
            }
        }
        match (self.boundary, self.last_operation_id) {
            (JournalBoundary::Created, None) => {}
            (JournalBoundary::Created, Some(_)) | (_, None) => {
                return Err(StudioError::JournalCorrupt);
            }
            (boundary, Some(operation_id)) => {
                let receipt = self
                    .receipts
                    .get(&operation_id)
                    .ok_or(StudioError::JournalCorrupt)?;
                if !receipt_matches_boundary(receipt.kind, boundary) {
                    return Err(StudioError::JournalCorrupt);
                }
            }
        }
        match self.boundary {
            JournalBoundary::TempAssetReserved
            | JournalBoundary::TempAssetDurable
            | JournalBoundary::AssetCommitRequested
                if self.pending_asset.as_ref().is_none_or(|pending| {
                    pending.asset.commit_state != AssetCommitState::Temporary
                }) || self.pending_edit.is_some()
                    || self.pending_render.is_some() =>
            {
                return Err(StudioError::JournalCorrupt);
            }
            JournalBoundary::TempAssetReserved
            | JournalBoundary::TempAssetDurable
            | JournalBoundary::AssetCommitRequested => {}
            JournalBoundary::AssetCommitted
                if self.pending_asset.as_ref().is_none_or(|pending| {
                    pending.asset.commit_state != AssetCommitState::DurableOriginal
                }) || self.pending_edit.is_some()
                    || self.pending_render.is_some() =>
            {
                return Err(StudioError::JournalCorrupt);
            }
            JournalBoundary::AssetCommitted => {}
            JournalBoundary::EditSavePrepared | JournalBoundary::EditSaveCommitted
                if self.pending_edit.is_none()
                    || self.pending_asset.is_some()
                    || self.pending_render.is_some() =>
            {
                return Err(StudioError::JournalCorrupt);
            }
            JournalBoundary::EditSavePrepared | JournalBoundary::EditSaveCommitted => {}
            JournalBoundary::RenderPrepared
            | JournalBoundary::RenderRunning
            | JournalBoundary::RenderFinalizing
            | JournalBoundary::RenderCommitted
            | JournalBoundary::RenderCancelled
                if self.pending_render.is_none()
                    || self.pending_asset.is_some()
                    || self.pending_edit.is_some() =>
            {
                return Err(StudioError::JournalCorrupt);
            }
            JournalBoundary::RenderPrepared
            | JournalBoundary::RenderRunning
            | JournalBoundary::RenderFinalizing
            | JournalBoundary::RenderCommitted
            | JournalBoundary::RenderCancelled => {}
            JournalBoundary::FailedRecoverably => {}
            _ if self.pending_asset.is_some()
                || self.pending_edit.is_some()
                || self.pending_render.is_some() =>
            {
                return Err(StudioError::JournalCorrupt);
            }
            _ => {}
        }
        if let Some(pending) = &self.pending_asset {
            pending.asset.validate()?;
            if self.boundary != JournalBoundary::FailedRecoverably
                && Some(pending.operation_id) != self.last_operation_id
            {
                return Err(StudioError::JournalCorrupt);
            }
            if self.boundary == JournalBoundary::FailedRecoverably
                && self
                    .receipts
                    .get(&pending.operation_id)
                    .is_none_or(|receipt| {
                        !matches!(
                            receipt.kind,
                            ReceiptKind::TempReserved
                                | ReceiptKind::TempDurable
                                | ReceiptKind::AssetCommitRequested
                                | ReceiptKind::AssetCommitted
                        )
                    })
            {
                return Err(StudioError::JournalCorrupt);
            }
        }
        if let Some(pending) = &self.pending_edit {
            validate_edit_shape(&pending.edits).map_err(|_| StudioError::JournalCorrupt)?;
            let receipt = self
                .receipts
                .get(&pending.operation_id)
                .ok_or(StudioError::JournalCorrupt)?;
            if receipt.kind != ReceiptKind::EditPrepared || pending.expected_project_revision == 0 {
                return Err(StudioError::JournalCorrupt);
            }
        }
        if let Some(pending) = &self.pending_edit
            && pending
                .expected_project_revision
                .checked_add(1)
                .is_none_or(|next| next != pending.edits.revision)
        {
            return Err(StudioError::JournalCorrupt);
        }
        if let Some(pending) = &self.pending_edit
            && self.boundary == JournalBoundary::EditSavePrepared
            && Some(pending.operation_id) != self.last_operation_id
        {
            return Err(StudioError::JournalCorrupt);
        }
        if let Some(pending) = &self.pending_render {
            let receipt = self
                .receipts
                .get(&pending.operation_id)
                .ok_or(StudioError::JournalCorrupt)?;
            let receipt_matches_render_boundary = receipt.kind == ReceiptKind::RenderPrepared
                || match self.boundary {
                    JournalBoundary::RenderPrepared => true,
                    JournalBoundary::RenderRunning => receipt.kind == ReceiptKind::RenderStarted,
                    JournalBoundary::RenderFinalizing => {
                        receipt.kind == ReceiptKind::RenderFinalizing
                    }
                    JournalBoundary::RenderCommitted => {
                        receipt.kind == ReceiptKind::RenderCommitted
                    }
                    JournalBoundary::RenderCancelled => receipt.kind == ReceiptKind::PartialDeleted,
                    JournalBoundary::FailedRecoverably => matches!(
                        receipt.kind,
                        ReceiptKind::RenderPrepared
                            | ReceiptKind::RenderStarted
                            | ReceiptKind::RenderFinalizing
                            | ReceiptKind::RecoveryApplied
                    ),
                    _ => false,
                };
            if !receipt_matches_render_boundary {
                return Err(StudioError::JournalCorrupt);
            }
            if pending.render_spec_digest
                != digest_render_identity(
                    pending.source_set_digest,
                    pending.plan_digest,
                    pending.profile,
                    &pending.output_name,
                )?
            {
                return Err(StudioError::JournalCorrupt);
            }
            pending.validate_terminal_receipt(self.project_id)?;
            if (self.boundary == JournalBoundary::RenderCommitted)
                != pending.terminal_receipt.is_some()
            {
                return Err(StudioError::JournalCorrupt);
            }
            if self.boundary == JournalBoundary::RenderPrepared
                && Some(pending.operation_id) != self.last_operation_id
            {
                return Err(StudioError::JournalCorrupt);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentKind {
    Project = 1,
    Edit = 2,
    Journal = 3,
    Asset = 4,
    ProjectFence = 5,
    RecordingGraph = 6,
}

/// Canonical bounded binary codec. The envelope is:
/// `FRST` magic, document kind, schema version, payload length, payload, SHA-256.
/// Integers are big-endian, collections are length framed, maps are ordered,
/// strings are UTF-8 and every decoded value is revalidated before return.
#[derive(Debug, Default, Clone, Copy)]
pub struct StudioDocumentCodec;

impl StudioDocumentCodec {
    pub fn encode_asset(asset: &StudioAsset) -> Result<Vec<u8>, StudioError> {
        asset.validate()?;
        let mut writer = CanonicalWriter::new();
        encode_asset(&mut writer, asset)?;
        wrap_document(DocumentKind::Asset, asset.version, writer.finish()?)
    }

    pub fn decode_asset(bytes: &[u8]) -> Result<StudioAsset, StudioError> {
        let (version, payload) = unwrap_document(bytes, DocumentKind::Asset)?;
        if version != STUDIO_ASSET_VERSION {
            return Err(StudioError::UnsupportedAssetVersion(version));
        }
        let mut reader = CanonicalReader::new(payload);
        let asset = decode_asset(&mut reader)?;
        reader.finish()?;
        if asset.version != version {
            return Err(StudioError::MalformedDocument);
        }
        asset.validate()?;
        Ok(asset)
    }

    pub fn encode_project(project: &StudioProjectManifest) -> Result<Vec<u8>, StudioError> {
        project.validate()?;
        let mut writer = CanonicalWriter::new();
        encode_project_payload(&mut writer, project)?;
        wrap_document(DocumentKind::Project, project.version, writer.finish()?)
    }

    pub fn decode_project(bytes: &[u8]) -> Result<StudioProjectManifest, StudioError> {
        let (version, payload) = unwrap_document(bytes, DocumentKind::Project)?;
        match inspect_project_version(version) {
            ProjectCompatibility::Supported => {}
            ProjectCompatibility::Migratable { .. } => {
                return Err(StudioError::LegacyImportRequired);
            }
            ProjectCompatibility::UnsupportedNewer { .. } => {
                return Err(StudioError::UnsupportedProjectVersion(version));
            }
        }
        let mut reader = CanonicalReader::new(payload);
        let project = decode_project_payload(&mut reader)?;
        reader.finish()?;
        if project.version != version {
            return Err(StudioError::MalformedDocument);
        }
        project.validate()?;
        Ok(project)
    }

    pub fn encode_edit(edit: &EditSpec) -> Result<Vec<u8>, StudioError> {
        validate_edit_shape(edit)?;
        let mut writer = CanonicalWriter::new();
        encode_edit(&mut writer, edit)?;
        wrap_document(DocumentKind::Edit, edit.version, writer.finish()?)
    }

    pub fn decode_edit(bytes: &[u8]) -> Result<EditSpec, StudioError> {
        let (version, payload) = unwrap_document(bytes, DocumentKind::Edit)?;
        if version != STUDIO_EDIT_VERSION {
            return Err(StudioError::UnsupportedEditVersion(version));
        }
        let mut reader = CanonicalReader::new(payload);
        let edit = decode_edit(&mut reader)?;
        reader.finish()?;
        validate_edit_shape(&edit)?;
        Ok(edit)
    }

    pub fn encode_journal(journal: &StudioJournalSnapshot) -> Result<Vec<u8>, StudioError> {
        journal.validate()?;
        let mut writer = CanonicalWriter::new();
        encode_journal(&mut writer, journal)?;
        wrap_document(DocumentKind::Journal, journal.version, writer.finish()?)
    }

    pub fn decode_journal(bytes: &[u8]) -> Result<StudioJournalSnapshot, StudioError> {
        let (version, payload) = unwrap_document(bytes, DocumentKind::Journal)?;
        if version != STUDIO_JOURNAL_VERSION {
            return Err(StudioError::UnsupportedJournalVersion(version));
        }
        let mut reader = CanonicalReader::new(payload);
        let journal = decode_journal(&mut reader)?;
        reader.finish()?;
        journal.validate()?;
        Ok(journal)
    }
}

fn wrap_document(
    kind: DocumentKind,
    version: u16,
    payload: Vec<u8>,
) -> Result<Vec<u8>, StudioError> {
    let payload_len = u32::try_from(payload.len()).map_err(|_| StudioError::DocumentTooLarge)?;
    let total = 4_usize
        .checked_add(1)
        .and_then(|value| value.checked_add(2))
        .and_then(|value| value.checked_add(4))
        .and_then(|value| value.checked_add(payload.len()))
        .and_then(|value| value.checked_add(32))
        .ok_or(StudioError::DocumentTooLarge)?;
    if total > MAX_STUDIO_DOCUMENT_BYTES {
        return Err(StudioError::DocumentTooLarge);
    }
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"FRST");
    output.push(kind as u8);
    output.extend_from_slice(&version.to_be_bytes());
    output.extend_from_slice(&payload_len.to_be_bytes());
    output.extend_from_slice(&payload);
    let digest_hex = strong_sha256(&output).to_hex();
    for chunk in digest_hex.as_bytes().chunks_exact(2) {
        let encoded = std::str::from_utf8(chunk).map_err(|_| StudioError::MalformedDocument)?;
        output.push(u8::from_str_radix(encoded, 16).map_err(|_| StudioError::MalformedDocument)?);
    }
    Ok(output)
}

fn unwrap_document(bytes: &[u8], expected_kind: DocumentKind) -> Result<(u16, &[u8]), StudioError> {
    if bytes.len() > MAX_STUDIO_DOCUMENT_BYTES || bytes.len() < 43 {
        return Err(StudioError::MalformedDocument);
    }
    if bytes.get(..4) != Some(b"FRST") || bytes.get(4).copied() != Some(expected_kind as u8) {
        return Err(StudioError::MalformedDocument);
    }
    let version = u16::from_be_bytes([
        *bytes.get(5).ok_or(StudioError::MalformedDocument)?,
        *bytes.get(6).ok_or(StudioError::MalformedDocument)?,
    ]);
    let payload_len = u32::from_be_bytes([
        *bytes.get(7).ok_or(StudioError::MalformedDocument)?,
        *bytes.get(8).ok_or(StudioError::MalformedDocument)?,
        *bytes.get(9).ok_or(StudioError::MalformedDocument)?,
        *bytes.get(10).ok_or(StudioError::MalformedDocument)?,
    ]) as usize;
    let payload_end = 11_usize
        .checked_add(payload_len)
        .ok_or(StudioError::MalformedDocument)?;
    if payload_end.checked_add(32) != Some(bytes.len()) {
        return Err(StudioError::MalformedDocument);
    }
    let expected = strong_sha256(&bytes[..payload_end]).to_hex();
    let mut actual = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in &bytes[payload_end..] {
        actual.push(char::from(HEX[usize::from(byte >> 4)]));
        actual.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    if expected.as_bytes() != actual.as_bytes() {
        return Err(StudioError::CorruptDocument);
    }
    Ok((version, &bytes[11..payload_end]))
}

#[derive(Debug)]
struct CanonicalWriter {
    bytes: Vec<u8>,
}

impl CanonicalWriter {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn finish(self) -> Result<Vec<u8>, StudioError> {
        if self.bytes.len() > MAX_STUDIO_DOCUMENT_BYTES {
            Err(StudioError::DocumentTooLarge)
        } else {
            Ok(self.bytes)
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn id(&mut self, value: [u8; 16]) {
        self.bytes.extend_from_slice(&value);
    }

    fn digest(&mut self, value: Sha256Digest) -> Result<(), StudioError> {
        let hex = value.to_hex();
        for chunk in hex.as_bytes().chunks_exact(2) {
            let encoded = std::str::from_utf8(chunk).map_err(|_| StudioError::MalformedDocument)?;
            self.u8(u8::from_str_radix(encoded, 16).map_err(|_| StudioError::MalformedDocument)?);
        }
        Ok(())
    }

    fn string(&mut self, value: &str) -> Result<(), StudioError> {
        let len = u16::try_from(value.len()).map_err(|_| StudioError::DocumentTooLarge)?;
        self.u16(len);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }
}

#[derive(Debug)]
struct CanonicalReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> CanonicalReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn finish(&self) -> Result<(), StudioError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(StudioError::MalformedDocument)
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], StudioError> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(StudioError::MalformedDocument)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(StudioError::MalformedDocument)?;
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, StudioError> {
        Ok(*self
            .take(1)?
            .first()
            .ok_or(StudioError::MalformedDocument)?)
    }

    fn bool(&mut self) -> Result<bool, StudioError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StudioError::MalformedDocument),
        }
    }

    fn u16(&mut self) -> Result<u16, StudioError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| StudioError::MalformedDocument)?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, StudioError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| StudioError::MalformedDocument)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn i32(&mut self) -> Result<i32, StudioError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| StudioError::MalformedDocument)?;
        Ok(i32::from_be_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, StudioError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| StudioError::MalformedDocument)?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn array_16(&mut self) -> Result<[u8; 16], StudioError> {
        self.take(16)?
            .try_into()
            .map_err(|_| StudioError::MalformedDocument)
    }

    fn digest(&mut self) -> Result<Sha256Digest, StudioError> {
        let bytes: [u8; 32] = self
            .take(32)?
            .try_into()
            .map_err(|_| StudioError::MalformedDocument)?;
        Sha256Digest::from_bytes(bytes).map_err(|_| StudioError::InvalidChecksum)
    }

    fn string(&mut self, maximum: usize) -> Result<String, StudioError> {
        let len = usize::from(self.u16()?);
        if len > maximum {
            return Err(StudioError::DocumentTooLarge);
        }
        let value =
            std::str::from_utf8(self.take(len)?).map_err(|_| StudioError::MalformedDocument)?;
        Ok(value.to_owned())
    }
}

fn encode_time(writer: &mut CanonicalWriter, value: RationalTime) {
    writer.u64(value.ticks);
    writer.u32(value.time_base.0);
}

fn decode_time(reader: &mut CanonicalReader<'_>) -> Result<RationalTime, StudioError> {
    let ticks = reader.u64()?;
    let time_base = TimeBase::new(reader.u32()?)?;
    Ok(RationalTime::new(ticks, time_base))
}

fn encode_asset(writer: &mut CanonicalWriter, asset: &StudioAsset) -> Result<(), StudioError> {
    writer.u16(asset.version);
    writer.id(asset.id.canonical_bytes());
    writer.u8(asset.track.tag());
    writer.string(asset.source_name.as_str())?;
    writer.u64(asset.byte_len);
    encode_time(writer, asset.start);
    encode_time(writer, asset.duration);
    writer.digest(asset.checksum.0)?;
    writer.u8(match asset.commit_state {
        AssetCommitState::Temporary => 1,
        AssetCommitState::DurableOriginal => 2,
    });
    Ok(())
}

fn decode_asset(reader: &mut CanonicalReader<'_>) -> Result<StudioAsset, StudioError> {
    let version = reader.u16()?;
    let id = StudioAssetId::from_csprng(reader.array_16()?)?;
    let track = TrackKind::from_tag(reader.u8()?)?;
    let source_name = StudioSourceName::new(reader.string(MAX_STUDIO_SOURCE_NAME_BYTES)?)?;
    let byte_len = reader.u64()?;
    let start = decode_time(reader)?;
    let duration = decode_time(reader)?;
    let checksum = AssetChecksum(reader.digest()?);
    let commit_state = match reader.u8()? {
        1 => AssetCommitState::Temporary,
        2 => AssetCommitState::DurableOriginal,
        _ => return Err(StudioError::MalformedDocument),
    };
    let asset = StudioAsset {
        version,
        id,
        track,
        source_name,
        byte_len,
        start,
        duration,
        checksum,
        commit_state,
    };
    asset.validate()?;
    Ok(asset)
}

fn encode_project_payload(
    writer: &mut CanonicalWriter,
    project: &StudioProjectManifest,
) -> Result<(), StudioError> {
    writer.u16(project.version);
    writer.id(project.id.canonical_bytes());
    writer.u64(project.revision);
    writer.u8(project.state.tag());
    writer.u16(u16::try_from(project.assets.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for asset in &project.assets {
        encode_asset(writer, asset)?;
    }
    encode_edit(writer, &project.edits)
}

fn decode_project_payload(
    reader: &mut CanonicalReader<'_>,
) -> Result<StudioProjectManifest, StudioError> {
    let version = reader.u16()?;
    let id = StudioProjectId::from_csprng(reader.array_16()?)?;
    let revision = reader.u64()?;
    let state = StudioState::from_tag(reader.u8()?)?;
    let asset_count = usize::from(reader.u16()?);
    if asset_count > MAX_STUDIO_ASSETS {
        return Err(StudioError::DocumentTooLarge);
    }
    let mut assets = Vec::with_capacity(asset_count);
    for _ in 0..asset_count {
        assets.push(decode_asset(reader)?);
    }
    let edits = decode_edit(reader)?;
    Ok(StudioProjectManifest {
        version,
        id,
        revision,
        state,
        assets,
        edits,
    })
}

fn validate_edit_shape(edit: &EditSpec) -> Result<(), StudioError> {
    if edit.version != STUDIO_EDIT_VERSION {
        return Err(StudioError::UnsupportedEditVersion(edit.version));
    }
    if edit.operations.len() > MAX_STUDIO_EDITS {
        return Err(StudioError::DocumentTooLarge);
    }
    let mut trim = None;
    let mut split_points = Vec::new();
    for operation in &edit.operations {
        if let Some((start, end)) = operation.range()
            && start.compare(end)? != std::cmp::Ordering::Less
        {
            return Err(StudioError::EditOutsideTimeline);
        }
        match operation {
            EditOperation::Trim { start, end } => {
                if trim.replace((*start, *end)).is_some() {
                    return Err(StudioError::MultipleTrims);
                }
            }
            EditOperation::Split { at } => split_points.push(*at),
            EditOperation::Speed {
                numerator,
                denominator,
                ..
            } if *numerator == 0
                || *denominator == 0
                || u64::from(*numerator) > u64::from(*denominator) * 4
                || u64::from(*denominator) > u64::from(*numerator) * 4 =>
            {
                return Err(StudioError::InvalidSpeed);
            }
            EditOperation::AudioGain {
                gain_millibels,
                track,
                ..
            } if !matches!(track, TrackKind::Microphone | TrackKind::SystemAudio)
                || !(-9_600..=2_400).contains(gain_millibels) =>
            {
                return Err(StudioError::InvalidGain);
            }
            EditOperation::CameraTransform {
                rect,
                corner_radius_milli,
                ..
            } => {
                rect.validate()?;
                if *corner_radius_milli > 1_000 {
                    return Err(StudioError::InvalidTransform);
                }
            }
            EditOperation::CursorTransform { scale_milli, .. }
                if !(100..=4_000).contains(scale_milli) =>
            {
                return Err(StudioError::InvalidTransform);
            }
            EditOperation::Background {
                style: BackgroundStyle::Blur { radius_milli },
                ..
            } if *radius_milli > 60_000 => return Err(StudioError::InvalidTransform),
            _ => {}
        }
    }
    split_points.sort_by(|left, right| compare_times(*left, *right));
    if split_points
        .windows(2)
        .any(|pair| compare_times(pair[0], pair[1]).is_eq())
    {
        return Err(StudioError::DuplicateEdit);
    }
    if let Some(active) = trim {
        for operation in &edit.operations {
            match operation {
                EditOperation::Trim { .. } => {}
                EditOperation::Split { at }
                    if compare_times(*at, active.0) != std::cmp::Ordering::Greater
                        || compare_times(*at, active.1) != std::cmp::Ordering::Less =>
                {
                    return Err(StudioError::EditOutsideTimeline);
                }
                EditOperation::Split { .. } => {}
                _ if operation
                    .range()
                    .is_some_and(|range| !range_contains(active, range.0, range.1)) =>
                {
                    return Err(StudioError::EditOutsideTimeline);
                }
                _ => {}
            }
        }
    }
    validate_persisted_edit_overlaps(edit)?;
    Ok(())
}

fn validate_persisted_edit_overlaps(edit: &EditSpec) -> Result<(), StudioError> {
    let mut ranges = edit
        .operations
        .iter()
        .filter_map(|operation| Some((operation_category(operation)?, operation.range()?)))
        .collect::<Vec<_>>();
    ranges.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| compare_times(left.1.0, right.1.0))
            .then_with(|| compare_times(left.1.1, right.1.1))
    });
    if ranges
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && ranges_overlap(pair[0].1, pair[1].1))
    {
        return Err(StudioError::OverlappingEdits);
    }
    Ok(())
}

fn encode_edit(writer: &mut CanonicalWriter, edit: &EditSpec) -> Result<(), StudioError> {
    validate_edit_shape(edit)?;
    writer.u16(edit.version);
    writer.u64(edit.revision);
    writer.u32(u32::try_from(edit.operations.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for operation in &edit.operations {
        encode_operation(writer, operation);
    }
    Ok(())
}

fn decode_edit(reader: &mut CanonicalReader<'_>) -> Result<EditSpec, StudioError> {
    let version = reader.u16()?;
    let revision = reader.u64()?;
    let count = usize::try_from(reader.u32()?).map_err(|_| StudioError::DocumentTooLarge)?;
    if count > MAX_STUDIO_EDITS {
        return Err(StudioError::DocumentTooLarge);
    }
    let mut operations = Vec::with_capacity(count);
    for _ in 0..count {
        operations.push(decode_operation(reader)?);
    }
    let edit = EditSpec {
        version,
        revision,
        operations,
    };
    validate_edit_shape(&edit)?;
    Ok(edit)
}

fn encode_operation(writer: &mut CanonicalWriter, operation: &EditOperation) {
    match operation {
        EditOperation::Trim { start, end } => {
            writer.u8(1);
            encode_time(writer, *start);
            encode_time(writer, *end);
        }
        EditOperation::Split { at } => {
            writer.u8(2);
            encode_time(writer, *at);
        }
        EditOperation::DeleteRange { start, end } => {
            writer.u8(3);
            encode_time(writer, *start);
            encode_time(writer, *end);
        }
        EditOperation::Speed {
            start,
            end,
            numerator,
            denominator,
        } => {
            writer.u8(4);
            encode_time(writer, *start);
            encode_time(writer, *end);
            writer.u32(*numerator);
            writer.u32(*denominator);
        }
        EditOperation::AudioGain {
            track,
            start,
            end,
            gain_millibels,
            muted,
        } => {
            writer.u8(5);
            writer.u8(track.tag());
            encode_time(writer, *start);
            encode_time(writer, *end);
            writer.i32(*gain_millibels);
            writer.bool(*muted);
        }
        EditOperation::Layout { start, end, preset } => {
            writer.u8(6);
            encode_time(writer, *start);
            encode_time(writer, *end);
            writer.u8(layout_tag(*preset));
        }
        EditOperation::CameraTransform {
            start,
            end,
            rect,
            corner_radius_milli,
        } => {
            writer.u8(7);
            encode_time(writer, *start);
            encode_time(writer, *end);
            writer.u32(rect.x_millionths);
            writer.u32(rect.y_millionths);
            writer.u32(rect.width_millionths);
            writer.u32(rect.height_millionths);
            writer.u16(*corner_radius_milli);
        }
        EditOperation::CursorTransform {
            start,
            end,
            scale_milli,
            hidden,
        } => {
            writer.u8(8);
            encode_time(writer, *start);
            encode_time(writer, *end);
            writer.u16(*scale_milli);
            writer.bool(*hidden);
        }
        EditOperation::Background { start, end, style } => {
            writer.u8(9);
            encode_time(writer, *start);
            encode_time(writer, *end);
            match style {
                BackgroundStyle::Transparent => writer.u8(1),
                BackgroundStyle::SolidRgb { red, green, blue } => {
                    writer.u8(2);
                    writer.u8(*red);
                    writer.u8(*green);
                    writer.u8(*blue);
                }
                BackgroundStyle::Blur { radius_milli } => {
                    writer.u8(3);
                    writer.u16(*radius_milli);
                }
            }
        }
    }
}

fn decode_operation(reader: &mut CanonicalReader<'_>) -> Result<EditOperation, StudioError> {
    let operation = match reader.u8()? {
        1 => EditOperation::Trim {
            start: decode_time(reader)?,
            end: decode_time(reader)?,
        },
        2 => EditOperation::Split {
            at: decode_time(reader)?,
        },
        3 => EditOperation::DeleteRange {
            start: decode_time(reader)?,
            end: decode_time(reader)?,
        },
        4 => EditOperation::Speed {
            start: decode_time(reader)?,
            end: decode_time(reader)?,
            numerator: reader.u32()?,
            denominator: reader.u32()?,
        },
        5 => EditOperation::AudioGain {
            track: TrackKind::from_tag(reader.u8()?)?,
            start: decode_time(reader)?,
            end: decode_time(reader)?,
            gain_millibels: reader.i32()?,
            muted: reader.bool()?,
        },
        6 => EditOperation::Layout {
            start: decode_time(reader)?,
            end: decode_time(reader)?,
            preset: layout_from_tag(reader.u8()?)?,
        },
        7 => EditOperation::CameraTransform {
            start: decode_time(reader)?,
            end: decode_time(reader)?,
            rect: NormalizedRect {
                x_millionths: reader.u32()?,
                y_millionths: reader.u32()?,
                width_millionths: reader.u32()?,
                height_millionths: reader.u32()?,
            },
            corner_radius_milli: reader.u16()?,
        },
        8 => EditOperation::CursorTransform {
            start: decode_time(reader)?,
            end: decode_time(reader)?,
            scale_milli: reader.u16()?,
            hidden: reader.bool()?,
        },
        9 => {
            let start = decode_time(reader)?;
            let end = decode_time(reader)?;
            let style = match reader.u8()? {
                1 => BackgroundStyle::Transparent,
                2 => BackgroundStyle::SolidRgb {
                    red: reader.u8()?,
                    green: reader.u8()?,
                    blue: reader.u8()?,
                },
                3 => BackgroundStyle::Blur {
                    radius_milli: reader.u16()?,
                },
                _ => return Err(StudioError::MalformedDocument),
            };
            EditOperation::Background { start, end, style }
        }
        _ => return Err(StudioError::MalformedDocument),
    };
    Ok(operation)
}

const fn layout_tag(layout: LayoutPreset) -> u8 {
    match layout {
        LayoutPreset::ScreenOnly => 1,
        LayoutPreset::CameraBubble => 2,
        LayoutPreset::SideBySide => 3,
        LayoutPreset::CameraFull => 4,
    }
}

fn layout_from_tag(tag: u8) -> Result<LayoutPreset, StudioError> {
    match tag {
        1 => Ok(LayoutPreset::ScreenOnly),
        2 => Ok(LayoutPreset::CameraBubble),
        3 => Ok(LayoutPreset::SideBySide),
        4 => Ok(LayoutPreset::CameraFull),
        _ => Err(StudioError::MalformedDocument),
    }
}

fn encode_journal(
    writer: &mut CanonicalWriter,
    journal: &StudioJournalSnapshot,
) -> Result<(), StudioError> {
    writer.u16(journal.version);
    writer.id(journal.project_id.canonical_bytes());
    writer.u64(journal.revision);
    writer.u64(journal.fence);
    writer.id(journal.owner.canonical_bytes());
    writer.u8(journal.boundary.tag());
    writer.bool(journal.last_operation_id.is_some());
    if let Some(operation_id) = journal.last_operation_id {
        writer.id(operation_id.canonical_bytes());
    }
    writer.bool(journal.pending_asset.is_some());
    if let Some(pending) = &journal.pending_asset {
        writer.id(pending.operation_id.canonical_bytes());
        encode_asset(writer, &pending.asset)?;
    }
    writer.bool(journal.pending_edit.is_some());
    if let Some(pending) = &journal.pending_edit {
        writer.id(pending.operation_id.canonical_bytes());
        writer.u64(pending.expected_project_revision);
        encode_edit(writer, &pending.edits)?;
    }
    writer.bool(journal.pending_render.is_some());
    if let Some(pending) = &journal.pending_render {
        writer.id(pending.operation_id.canonical_bytes());
        writer.id(pending.export_id.canonical_bytes());
        writer.u64(pending.fence);
        writer.digest(pending.source_set_digest)?;
        writer.digest(pending.plan_digest)?;
        writer.digest(pending.render_spec_digest)?;
        writer.u8(export_profile_tag(pending.profile));
        writer.string(pending.output_name.as_str())?;
        writer.bool(pending.terminal_receipt.is_some());
        if let Some(receipt) = &pending.terminal_receipt {
            encode_render_receipt(writer, receipt)?;
        }
    }
    writer.u32(u32::try_from(journal.receipts.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for (operation_id, receipt) in &journal.receipts {
        writer.id(operation_id.canonical_bytes());
        writer.u8(receipt_kind_tag(receipt.kind));
        writer.digest(receipt.command_digest)?;
        writer.digest(receipt.outcome_digest)?;
    }
    Ok(())
}

fn decode_journal(reader: &mut CanonicalReader<'_>) -> Result<StudioJournalSnapshot, StudioError> {
    let version = reader.u16()?;
    let project_id = StudioProjectId::from_csprng(reader.array_16()?)?;
    let revision = reader.u64()?;
    let fence = reader.u64()?;
    let owner = StudioWorkerId::from_csprng(reader.array_16()?)?;
    let boundary = JournalBoundary::from_tag(reader.u8()?)?;
    let last_operation_id = if reader.bool()? {
        Some(StudioOperationId::from_csprng(reader.array_16()?)?)
    } else {
        None
    };
    let pending_asset = if reader.bool()? {
        Some(PendingAssetCommit {
            operation_id: StudioOperationId::from_csprng(reader.array_16()?)?,
            asset: decode_asset(reader)?,
        })
    } else {
        None
    };
    let pending_edit = if reader.bool()? {
        Some(PendingEditSave {
            operation_id: StudioOperationId::from_csprng(reader.array_16()?)?,
            expected_project_revision: reader.u64()?,
            edits: decode_edit(reader)?,
        })
    } else {
        None
    };
    let pending_render = if reader.bool()? {
        Some(PendingRender {
            operation_id: StudioOperationId::from_csprng(reader.array_16()?)?,
            export_id: StudioExportId::from_csprng(reader.array_16()?)?,
            fence: reader.u64()?,
            source_set_digest: reader.digest()?,
            plan_digest: reader.digest()?,
            render_spec_digest: reader.digest()?,
            profile: export_profile_from_tag(reader.u8()?)?,
            output_name: StudioSourceName::new(reader.string(MAX_STUDIO_SOURCE_NAME_BYTES)?)?,
            terminal_receipt: if reader.bool()? {
                Some(decode_render_receipt(reader)?)
            } else {
                None
            },
        })
    } else {
        None
    };
    let receipt_count =
        usize::try_from(reader.u32()?).map_err(|_| StudioError::DocumentTooLarge)?;
    if receipt_count > MAX_STUDIO_RECEIPTS {
        return Err(StudioError::DocumentTooLarge);
    }
    let mut receipts = BTreeMap::new();
    for _ in 0..receipt_count {
        let operation_id = StudioOperationId::from_csprng(reader.array_16()?)?;
        let receipt = StudioOperationReceipt {
            operation_id,
            kind: receipt_kind_from_tag(reader.u8()?)?,
            command_digest: reader.digest()?,
            outcome_digest: reader.digest()?,
        };
        if receipts.insert(operation_id, receipt).is_some() {
            return Err(StudioError::JournalCorrupt);
        }
    }
    Ok(StudioJournalSnapshot {
        version,
        project_id,
        revision,
        fence,
        owner,
        boundary,
        last_operation_id,
        pending_asset,
        pending_edit,
        pending_render,
        receipts,
    })
}

fn encode_render_receipt(
    writer: &mut CanonicalWriter,
    receipt: &RenderReceipt,
) -> Result<(), StudioError> {
    writer.id(receipt.project_id.canonical_bytes());
    writer.id(receipt.export_id.canonical_bytes());
    writer.id(receipt.operation_id.canonical_bytes());
    writer.u64(receipt.fence);
    writer.digest(receipt.source_set_digest)?;
    writer.digest(receipt.plan_digest)?;
    writer.digest(receipt.render_spec_digest)?;
    writer.u8(export_profile_tag(receipt.profile.profile));
    writer.string(receipt.output_name.as_str())?;
    writer.digest(receipt.output_checksum.0)?;
    writer.u64(receipt.output_bytes);
    Ok(())
}

fn decode_render_receipt(reader: &mut CanonicalReader<'_>) -> Result<RenderReceipt, StudioError> {
    Ok(RenderReceipt {
        project_id: StudioProjectId::from_csprng(reader.array_16()?)?,
        export_id: StudioExportId::from_csprng(reader.array_16()?)?,
        operation_id: StudioOperationId::from_csprng(reader.array_16()?)?,
        fence: reader.u64()?,
        source_set_digest: reader.digest()?,
        plan_digest: reader.digest()?,
        render_spec_digest: reader.digest()?,
        profile: ExportProfileSpec::approved(export_profile_from_tag(reader.u8()?)?),
        output_name: StudioSourceName::new(reader.string(MAX_STUDIO_SOURCE_NAME_BYTES)?)?,
        output_checksum: AssetChecksum(reader.digest()?),
        output_bytes: reader.u64()?,
    })
}

const fn receipt_kind_tag(kind: ReceiptKind) -> u8 {
    match kind {
        ReceiptKind::GraphPrepared => 1,
        ReceiptKind::CaptureStarted => 2,
        ReceiptKind::TempReserved => 3,
        ReceiptKind::TempDurable => 4,
        ReceiptKind::AssetCommitRequested => 5,
        ReceiptKind::AssetCommitted => 6,
        ReceiptKind::RecordingStopped => 7,
        ReceiptKind::EditPrepared => 8,
        ReceiptKind::EditCommitted => 9,
        ReceiptKind::RenderPrepared => 10,
        ReceiptKind::RenderStarted => 11,
        ReceiptKind::RenderFinalizing => 12,
        ReceiptKind::RenderCommitted => 13,
        ReceiptKind::PartialDeleted => 14,
        ReceiptKind::RecoveryApplied => 15,
    }
}

fn receipt_kind_from_tag(tag: u8) -> Result<ReceiptKind, StudioError> {
    match tag {
        1 => Ok(ReceiptKind::GraphPrepared),
        2 => Ok(ReceiptKind::CaptureStarted),
        3 => Ok(ReceiptKind::TempReserved),
        4 => Ok(ReceiptKind::TempDurable),
        5 => Ok(ReceiptKind::AssetCommitRequested),
        6 => Ok(ReceiptKind::AssetCommitted),
        7 => Ok(ReceiptKind::RecordingStopped),
        8 => Ok(ReceiptKind::EditPrepared),
        9 => Ok(ReceiptKind::EditCommitted),
        10 => Ok(ReceiptKind::RenderPrepared),
        11 => Ok(ReceiptKind::RenderStarted),
        12 => Ok(ReceiptKind::RenderFinalizing),
        13 => Ok(ReceiptKind::RenderCommitted),
        14 => Ok(ReceiptKind::PartialDeleted),
        15 => Ok(ReceiptKind::RecoveryApplied),
        _ => Err(StudioError::MalformedDocument),
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StudioError {
    #[error("Studio project ID is invalid")]
    InvalidProjectId,
    #[error("Studio asset ID is invalid")]
    InvalidAssetId,
    #[error("Studio operation ID is invalid")]
    InvalidOperationId,
    #[error("Studio worker ID is invalid")]
    InvalidWorkerId,
    #[error("Studio export ID is invalid")]
    InvalidExportId,
    #[error("Studio asset checksum is invalid")]
    InvalidChecksum,
    #[error("Studio source name is invalid")]
    InvalidSourceName,
    #[error("Studio timebase is invalid")]
    InvalidTimeBase,
    #[error("Studio asset metadata is invalid")]
    InvalidAsset,
    #[error("Studio asset conflicts with an existing original")]
    AssetConflict,
    #[error("temporary Studio asset escaped the recording journal")]
    TemporaryAssetEscapedRecording,
    #[error("unsupported Studio project version {0}")]
    UnsupportedProjectVersion(u16),
    #[error("unsupported Studio asset version {0}")]
    UnsupportedAssetVersion(u16),
    #[error("unsupported Studio edit version {0}")]
    UnsupportedEditVersion(u16),
    #[error("unsupported Studio journal version {0}")]
    UnsupportedJournalVersion(u16),
    #[error("legacy Studio document must use the non-mutating importer")]
    LegacyImportRequired,
    #[error("Studio document exceeds its bounded codec limit")]
    DocumentTooLarge,
    #[error("Studio document is malformed")]
    MalformedDocument,
    #[error("Studio document checksum does not match")]
    CorruptDocument,
    #[error("Studio durable storage operation failed")]
    StorageIo,
    #[error("Studio durable storage path is unsafe")]
    UnsafeStoragePath,
    #[error("Studio journal is corrupt or overflowed")]
    JournalCorrupt,
    #[error("Studio timeline arithmetic overflowed")]
    TimelineOverflow,
    #[error("Studio timeline arithmetic underflowed")]
    TimelineUnderflow,
    #[error("Studio edit is outside the source timeline")]
    EditOutsideTimeline,
    #[error("Studio speed must be between 0.25x and 4x")]
    InvalidSpeed,
    #[error("Studio gain or audio target is invalid")]
    InvalidGain,
    #[error("Studio visual transform is invalid")]
    InvalidTransform,
    #[error("legacy Cap project is malformed")]
    MalformedLegacyProject,
    #[error("legacy Cap ID assignment does not match the source assets")]
    LegacyIdAssignmentMismatch,
    #[error("legacy Cap source changed during a read-only import")]
    LegacySourceChanged,
    #[error("Studio recording graph is not an isolated four-track graph")]
    InvalidRecordingGraph,
    #[error("Studio graph contains an unbounded media queue")]
    UnboundedMediaQueue,
    #[error("Studio journal revision or fence is stale")]
    StaleJournal,
    #[error("Studio journal commit outcome is ambiguous")]
    AmbiguousJournalCommit,
    #[error("Studio idempotency key was reused for a different command")]
    IdempotencyConflict,
    #[error("invalid Studio journal transition from {from:?} to {to:?}")]
    InvalidJournalTransition {
        from: JournalBoundary,
        to: JournalBoundary,
    },
    #[error("Studio journal receipt kind does not match its durable boundary")]
    InvalidJournalReceipt,
    #[error("Studio journal changed a pending asset, edit, or render identity")]
    JournalPendingIdentityChanged,
    #[error("Studio temporary asset commit ticket is invalid")]
    InvalidAssetCommit,
    #[error("Studio temporary asset commit acknowledgement is ambiguous")]
    AmbiguousAssetCommit,
    #[error("Studio committed asset does not match the exact temporary source")]
    AssetCommitMismatch,
    #[error("Studio edit save ticket is invalid")]
    InvalidEditSave,
    #[error("Studio edit save acknowledgement is ambiguous")]
    AmbiguousEditSave,
    #[error("Studio persisted edit save does not match its exact postcondition")]
    EditSaveMismatch,
    #[error("Studio timeline has no duration")]
    NoTimeline,
    #[error("Studio project has no durable screen original")]
    NoAssets,
    #[error("Studio project state conflicts with its durable contents")]
    InvalidProjectState,
    #[error("Studio source coverage is invalid")]
    InvalidCoverage,
    #[error("Studio source coverage overlaps")]
    OverlappingCoverage,
    #[error("Studio VFR timestamps are invalid or unordered")]
    InvalidVfrSamples,
    #[error("Studio project may contain only one outer trim")]
    MultipleTrims,
    #[error("Studio edit ranges overlap within one effect category")]
    OverlappingEdits,
    #[error("Studio edit contains a duplicate split point")]
    DuplicateEdit,
    #[error("Studio edit would produce an empty output")]
    EmptyOutput,
    #[error("Studio compiled edit plan violates structural invariants")]
    InvalidCompiledPlan,
    #[error("Studio compiled edit plan digest does not match its contents")]
    CorruptCompiledPlan,
    #[error("Studio screen coverage has a gap")]
    UncoveredRequiredVideo,
    #[error("Studio seek is outside the compiled output timeline")]
    SeekOutsideTimeline,
    #[error("Studio frame rate is invalid")]
    InvalidFrameRate,
    #[error("Studio audio format is invalid")]
    InvalidAudioFormat,
    #[error("Studio timestamp simulation exceeded its explicit bound")]
    SimulationLimitExceeded,
    #[error("Studio export profile is invalid")]
    InvalidExportProfile,
    #[error("Studio render graph does not match its canonical plan and profile")]
    InvalidRenderGraph,
    #[error("Studio source set is invalid or contains non-original media")]
    InvalidSourceSet,
    #[error("Studio source set digest does not match its original assets")]
    CorruptSourceSet,
    #[error("Studio source set does not cover the canonical timeline")]
    SourceSetTimelineMismatch,
    #[error("Studio renderer contract is incompatible")]
    IncompatibleRenderer,
    #[error("Studio renderer capabilities changed after preflight")]
    RendererCapabilityChanged,
    #[error("Studio renderer does not support the requested profile")]
    UnsupportedRenderProfile,
    #[error("Studio renderer is missing the required codec license: {0:?}")]
    MissingCodecLicense(CodecLicense),
    #[error("Studio control payload exceeds its explicit bound")]
    PayloadTooLarge,
    #[error("Studio payload chunk is empty or exceeds the pull bound")]
    InvalidPayloadChunk,
    #[error("Studio payload length does not match its declaration")]
    PayloadLengthMismatch,
    #[error("Studio render ticket is invalid")]
    InvalidRenderTicket,
    #[error("Studio render dispatch requires a durable journal reservation")]
    RenderReservationRequired,
    #[error("Studio render exceeded its declared deadline")]
    RenderDeadlineExceeded,
    #[error("Studio renderer event queue bound is invalid")]
    UnboundedRendererEvents,
    #[error("Studio export identity cannot be reused for another command")]
    ExportIdReused,
    #[error("Studio output target is reserved by another unreleased export")]
    OutputTargetBusy,
    #[error("a recovered Studio render must be reconciled before replay or replacement")]
    RecoveredRenderRequiresReconciliation,
    #[error("Studio renderer reached its bounded session limit")]
    RenderConcurrencyLimit,
    #[error("Studio render start acknowledgement is ambiguous")]
    AmbiguousRenderStart,
    #[error("Studio renderer callback has a stale fence, identity, or sequence")]
    StaleRenderCallback,
    #[error("Studio renderer returned more events than requested")]
    RendererEventOverflow,
    #[error("Studio renderer poll or cleanup wait is outside the bounded policy")]
    InvalidRenderPollWait,
    #[error("Studio export is unknown")]
    UnknownExport,
    #[error("Studio render progress is non-monotonic or out of range")]
    NonMonotonicProgress,
    #[error("Studio renderer failure code is not a bounded public identifier")]
    InvalidRenderFailureCode,
    #[error("Studio renderer committed before progress and output were complete")]
    PrematureRenderCommit,
    #[error("Studio committed output does not match the renderer postcondition")]
    RenderPostconditionMismatch,
    #[error("a committed Studio render cannot be cancelled")]
    CommittedRenderCannotBeCancelled,
    #[error("an active Studio render cannot be released")]
    ActiveRenderCannotBeReleased,
    #[error("Studio partial output cleanup was not confirmed")]
    PartialCleanupUnconfirmed,
    #[error("Studio hardware fallback did not preserve the exact plan")]
    InvalidHardwareFallback,
}

// ---------------------------------------------------------------------------
// Pinned Cap .cap-directory compatibility inspection

pub const LEGACY_CAP_SNAPSHOT_VERSION: u16 = 1;
pub const MAX_LEGACY_CAP_PATH_BYTES: usize = 512;
pub const MAX_LEGACY_CAP_SEGMENTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyUnsupportedEffect {
    Zoom,
    Scene,
    Mask,
    Text,
    Caption,
    Keyboard,
    ImportedAudio,
    Clip,
    Annotation,
    ChromaKey,
    DropShadow,
    SpringKeyframes,
    ExternalFont,
    UnknownField(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCompatibilityReport {
    pub source_digest: Sha256Digest,
    pub found_version: u16,
    pub source_asset_count: usize,
    pub supported_effect_count: usize,
    pub unsupported_effects: BTreeSet<LegacyUnsupportedEffect>,
    pub actionable_message: &'static str,
}

impl LegacyCompatibilityReport {
    #[must_use]
    pub fn importable(&self) -> bool {
        self.found_version == LEGACY_CAP_SNAPSHOT_VERSION && self.unsupported_effects.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyIdAssignment {
    pub project_id: StudioProjectId,
    pub asset_ids: Vec<StudioAssetId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyImport {
    pub manifest: StudioProjectManifest,
    pub copy_plan: LegacyCopyPlan,
    pub report: LegacyCompatibilityReport,
    pub source_digest_before: Sha256Digest,
    pub source_digest_after: Sha256Digest,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyCopyPlanEntry {
    pub asset_id: StudioAssetId,
    pub track: TrackKind,
    pub source: LegacyCapFileDescriptor,
    pub destination: StudioSourceName,
}

impl fmt::Debug for LegacyCopyPlanEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCopyPlanEntry")
            .field("asset_id", &self.asset_id)
            .field("track", &self.track)
            .field("source", &self.source)
            .field("destination", &self.destination)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCopyPlan {
    pub project_id: StudioProjectId,
    pub entries: Vec<LegacyCopyPlanEntry>,
}

impl LegacyCopyPlan {
    pub fn validate(&self) -> Result<(), StudioError> {
        if self.entries.is_empty() || self.entries.len() > MAX_STUDIO_ASSETS {
            return Err(StudioError::MalformedLegacyProject);
        }
        let mut assets = BTreeSet::new();
        let mut sources = BTreeSet::new();
        let mut destinations = BTreeSet::new();
        for entry in &self.entries {
            entry.source.validate()?;
            if !assets.insert(entry.asset_id)
                || !sources.insert(entry.source.relative_path.clone())
                || !destinations.insert(entry.destination.clone())
            {
                return Err(StudioError::MalformedLegacyProject);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyImportOutcome {
    Imported(Box<LegacyImport>),
    NeedsUserAction(LegacyCompatibilityReport),
    UnsupportedNewer(LegacyCompatibilityReport),
}

#[derive(Debug, Clone)]
struct LegacyDecoded {
    track: TrackKind,
    segment_index: u32,
    start: RationalTime,
    duration: RationalTime,
    file: LegacyCapFileDescriptor,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegacyCapRelativePath(String);

impl LegacyCapRelativePath {
    pub fn new(value: impl Into<String>) -> Result<Self, StudioError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_LEGACY_CAP_PATH_BYTES
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains('\\')
            || value.split('/').any(|part| {
                part.is_empty()
                    || matches!(part, "." | "..")
                    || !part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            })
        {
            return Err(StudioError::MalformedLegacyProject);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LegacyCapRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyCapRelativePath(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyCapFileDescriptor {
    pub relative_path: LegacyCapRelativePath,
    pub byte_len: u64,
    pub checksum: AssetChecksum,
}

impl fmt::Debug for LegacyCapFileDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCapFileDescriptor")
            .field("relative_path", &self.relative_path)
            .field("byte_len", &self.byte_len)
            .field("checksum", &self.checksum)
            .finish()
    }
}

impl LegacyCapFileDescriptor {
    fn validate(&self) -> Result<(), StudioError> {
        if self.byte_len == 0 {
            return Err(StudioError::MalformedLegacyProject);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCapSegment {
    pub index: u32,
    pub start: RationalTime,
    pub duration: RationalTime,
    pub display: LegacyCapFileDescriptor,
    pub camera: Option<LegacyCapFileDescriptor>,
    pub microphone: Option<LegacyCapFileDescriptor>,
    pub system_audio: Option<LegacyCapFileDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCapProjectSnapshot {
    pub version: u16,
    pub reference_revision: Sha256Digest,
    pub recording_meta: LegacyCapFileDescriptor,
    pub project_config: Option<LegacyCapFileDescriptor>,
    pub segments: Vec<LegacyCapSegment>,
    pub edits: EditSpec,
    pub unsupported_effects: BTreeSet<LegacyUnsupportedEffect>,
    pub source_tree_digest: Sha256Digest,
}

impl LegacyCapProjectSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        recording_meta: LegacyCapFileDescriptor,
        project_config: Option<LegacyCapFileDescriptor>,
        segments: Vec<LegacyCapSegment>,
        edits: EditSpec,
        unsupported_effects: BTreeSet<LegacyUnsupportedEffect>,
    ) -> Result<Self, StudioError> {
        let mut snapshot = Self {
            version,
            reference_revision: pinned_cap_reference_digest(),
            recording_meta,
            project_config,
            segments,
            edits,
            unsupported_effects,
            source_tree_digest: strong_sha256(b"pending legacy Cap tree digest"),
        };
        snapshot.source_tree_digest = digest_legacy_cap_snapshot(&snapshot)?;
        if version == LEGACY_CAP_SNAPSHOT_VERSION {
            snapshot.validate()?;
        }
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), StudioError> {
        if self.version != LEGACY_CAP_SNAPSHOT_VERSION
            || self.reference_revision != pinned_cap_reference_digest()
            || self.segments.is_empty()
            || self.segments.len() > MAX_LEGACY_CAP_SEGMENTS
            || self.recording_meta.relative_path.as_str() != "recording-meta.json"
            || self
                .project_config
                .as_ref()
                .is_some_and(|file| file.relative_path.as_str() != "project-config.json")
        {
            return Err(StudioError::MalformedLegacyProject);
        }
        self.recording_meta.validate()?;
        if let Some(project_config) = &self.project_config {
            project_config.validate()?;
        }
        validate_edit_shape(&self.edits)?;
        let mut cursor = ExactDuration::zero();
        let mut paths = BTreeSet::new();
        for (expected_index, segment) in self.segments.iter().enumerate() {
            if segment.index
                != u32::try_from(expected_index).map_err(|_| StudioError::DocumentTooLarge)?
                || segment.duration.ticks == 0
                || segment
                    .start
                    .checked_sub(RationalTime::new(0, segment.start.time_base))?
                    != cursor
            {
                return Err(StudioError::MalformedLegacyProject);
            }
            for file in [
                Some(&segment.display),
                segment.camera.as_ref(),
                segment.microphone.as_ref(),
                segment.system_audio.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                file.validate()?;
                if !file.relative_path.as_str().starts_with("content/")
                    || !paths.insert(file.relative_path.clone())
                {
                    return Err(StudioError::MalformedLegacyProject);
                }
            }
            cursor = cursor.checked_add(
                segment
                    .duration
                    .checked_sub(RationalTime::new(0, segment.duration.time_base))?,
            )?;
        }
        if self.edits.revision != 1 || digest_legacy_cap_snapshot(self)? != self.source_tree_digest
        {
            return Err(StudioError::MalformedLegacyProject);
        }
        Ok(())
    }
}

pub trait LegacyCapProjectPort {
    fn source_tree_digest(&mut self) -> Result<Sha256Digest, StudioError>;
    fn read_snapshot(&mut self) -> Result<LegacyCapProjectSnapshot, StudioError>;
}

/// Read-only production adapter for a pinned Cap `.cap` directory. JSON is
/// parsed into bounded typed schema views, every referenced media path is
/// traversal-checked and symlink-rejected, and each file is streamed through
/// SHA-256 before an import/copy plan is returned.
pub struct FilesystemLegacyCapProjectPort {
    root: PathBuf,
}

impl fmt::Debug for FilesystemLegacyCapProjectPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemLegacyCapProjectPort")
            .field("root", &"<redacted>")
            .finish()
    }
}

impl FilesystemLegacyCapProjectPort {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StudioError> {
        let root = root.as_ref();
        let metadata =
            fs::symlink_metadata(root).map_err(|_| StudioError::MalformedLegacyProject)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StudioError::MalformedLegacyProject);
        }
        Ok(Self {
            root: root
                .canonicalize()
                .map_err(|_| StudioError::MalformedLegacyProject)?,
        })
    }

    fn descriptor(&self, relative: &str) -> Result<LegacyCapFileDescriptor, StudioError> {
        let relative_path = LegacyCapRelativePath::new(relative)?;
        let path = checked_legacy_path(&self.root, &relative_path)?;
        let mut file = File::open(&path).map_err(|_| StudioError::MalformedLegacyProject)?;
        let metadata = file
            .metadata()
            .map_err(|_| StudioError::MalformedLegacyProject)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(StudioError::MalformedLegacyProject);
        }
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; MAX_STUDIO_PAYLOAD_CHUNK_BYTES];
        let mut observed = 0_u64;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| StudioError::MalformedLegacyProject)?;
            if count == 0 {
                break;
            }
            observed = observed
                .checked_add(u64::try_from(count).map_err(|_| StudioError::DocumentTooLarge)?)
                .ok_or(StudioError::DocumentTooLarge)?;
            hasher.update(&buffer[..count]);
        }
        if observed != metadata.len() {
            return Err(StudioError::LegacySourceChanged);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        Ok(LegacyCapFileDescriptor {
            relative_path,
            byte_len: observed,
            checksum: AssetChecksum::from_bytes(digest)?,
        })
    }

    fn parse_snapshot(&self) -> Result<LegacyCapProjectSnapshot, StudioError> {
        let recording_meta = self.descriptor("recording-meta.json")?;
        let project_config = self.descriptor("project-config.json")?;
        let recording: LegacyRecordingMetaDocument =
            read_legacy_json(&self.root, &recording_meta.relative_path)?;
        let configuration: LegacyProjectConfigDocument =
            read_legacy_json(&self.root, &project_config.relative_path)?;
        let recording_segments = if recording.segments.is_empty() {
            vec![LegacyRecordingSegmentDocument {
                display: recording
                    .display
                    .clone()
                    .ok_or(StudioError::MalformedLegacyProject)?,
                camera: recording.camera.clone(),
                mic: recording.audio.clone(),
                system_audio: None,
                cursor: recording.cursor.clone(),
                keyboard: None,
                unknown: BTreeMap::new(),
            }]
        } else {
            if recording.display.is_some()
                || recording.camera.is_some()
                || recording.audio.is_some()
                || recording.cursor.is_some()
            {
                return Err(StudioError::MalformedLegacyProject);
            }
            recording.segments.clone()
        };
        if recording_segments.len() > MAX_LEGACY_CAP_SEGMENTS
            || recording
                .status
                .as_ref()
                .is_some_and(|status| status.status != "Complete")
            || configuration.timeline.segments.len() != recording_segments.len()
        {
            return Err(StudioError::MalformedLegacyProject);
        }

        let mut unsupported = BTreeSet::new();
        mark_legacy_unknowns("recording", &recording.unknown, &mut unsupported);
        if let Some(status) = &recording.status {
            mark_legacy_unknowns("status", &status.unknown, &mut unsupported);
        }
        if recording
            .cursors
            .as_object()
            .is_some_and(|cursors| !cursors.is_empty())
            || recording.cursor.is_some()
        {
            unsupported.insert(LegacyUnsupportedEffect::UnknownField(
                legacy_unknown_field_marker("recording.cursor"),
            ));
        }
        mark_legacy_unknowns("config", &configuration.unknown, &mut unsupported);
        mark_legacy_unknowns(
            "timeline",
            &configuration.timeline.unknown,
            &mut unsupported,
        );
        if !configuration.timeline.zoom_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::Zoom);
        }
        if !configuration.timeline.scene_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::Scene);
        }
        if !configuration.timeline.mask_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::Mask);
        }
        if !configuration.timeline.text_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::Text);
        }
        if !configuration.timeline.caption_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::Caption);
        }
        if !configuration.timeline.keyboard_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::Keyboard);
        }
        if !configuration.timeline.audio_segments.is_empty() {
            unsupported.insert(LegacyUnsupportedEffect::ImportedAudio);
        }
        if !configuration.clips.as_array().is_none_or(Vec::is_empty) {
            unsupported.insert(LegacyUnsupportedEffect::Clip);
        }
        if !configuration
            .annotations
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            unsupported.insert(LegacyUnsupportedEffect::Annotation);
        }
        mark_nondefault_legacy_audio(&configuration.audio, &mut unsupported)?;
        if legacy_object_has_keys_other_than(&configuration.cursor, &["hide", "size"])? {
            unsupported.insert(LegacyUnsupportedEffect::UnknownField(
                legacy_unknown_field_marker("cursor"),
            ));
        }

        let mut segments = Vec::with_capacity(recording_segments.len());
        let mut cursor = RationalTime::new(0, TimeBase::new(1)?);
        let mut total = ExactDuration::zero();
        let mut timeline_boundaries = Vec::new();
        let mut selected_start = RationalTime::new(0, TimeBase::new(1)?);
        let mut selected_end = RationalTime::new(0, TimeBase::new(1)?);
        for (index, (recorded, timeline)) in recording_segments
            .iter()
            .zip(&configuration.timeline.segments)
            .enumerate()
        {
            if usize::try_from(timeline.recording_segment) != Ok(index) {
                unsupported.insert(LegacyUnsupportedEffect::Clip);
            }
            mark_legacy_unknowns("segment", &recorded.unknown, &mut unsupported);
            mark_legacy_unknowns("timeline-segment", &timeline.unknown, &mut unsupported);
            let start = decimal_number_time(&timeline.start)?;
            let end = decimal_number_time(&timeline.end)?;
            let timescale = decimal_number_time(&timeline.timescale)?;
            if start.compare(end)? != std::cmp::Ordering::Less || timescale.ticks == 0 {
                return Err(StudioError::MalformedLegacyProject);
            }
            if timescale.compare(RationalTime::new(1, TimeBase::new(1)?))?
                != std::cmp::Ordering::Equal
            {
                unsupported.insert(LegacyUnsupportedEffect::Clip);
            }
            if index == 0 {
                selected_start = start;
            } else if start.ticks != 0 {
                unsupported.insert(LegacyUnsupportedEffect::Clip);
            }
            let duration = end;
            let duration_exact =
                duration.checked_sub(RationalTime::new(0, duration.time_base()))?;
            total = total.checked_add(duration_exact)?;
            selected_end = RationalTime::new(
                u64::try_from(total.numerator).map_err(|_| StudioError::TimelineOverflow)?,
                TimeBase::new(
                    u32::try_from(total.denominator).map_err(|_| StudioError::TimelineOverflow)?,
                )?,
            );
            if index + 1 < recording_segments.len() {
                timeline_boundaries.push(selected_end);
            }
            validate_legacy_media_timing(&recorded.display, &mut unsupported)?;
            if let Some(media) = &recorded.camera {
                validate_legacy_media_timing(media, &mut unsupported)?;
            }
            if let Some(media) = &recorded.mic {
                validate_legacy_media_timing(media, &mut unsupported)?;
            }
            if let Some(media) = &recorded.system_audio {
                validate_legacy_media_timing(media, &mut unsupported)?;
            }
            if recorded.cursor.is_some() || recorded.keyboard.is_some() {
                unsupported.insert(LegacyUnsupportedEffect::UnknownField(
                    legacy_unknown_field_marker("segment.auxiliary-events"),
                ));
            }
            segments.push(LegacyCapSegment {
                index: u32::try_from(index).map_err(|_| StudioError::DocumentTooLarge)?,
                start: cursor,
                duration,
                display: self.descriptor(&recorded.display.path)?,
                camera: recorded
                    .camera
                    .as_ref()
                    .map(|media| self.descriptor(&media.path))
                    .transpose()?,
                microphone: recorded
                    .mic
                    .as_ref()
                    .map(|media| self.descriptor(&media.path))
                    .transpose()?,
                system_audio: recorded
                    .system_audio
                    .as_ref()
                    .map(|media| self.descriptor(&media.path))
                    .transpose()?,
            });
            cursor = selected_end;
        }

        let mut operations = Vec::new();
        if selected_start.ticks != 0 {
            operations.push(EditOperation::Trim {
                start: selected_start,
                end: selected_end,
            });
        }
        for boundary in timeline_boundaries {
            if boundary.compare(selected_start)? == std::cmp::Ordering::Greater
                && boundary.compare(selected_end)? == std::cmp::Ordering::Less
            {
                operations.push(EditOperation::Split { at: boundary });
            }
        }
        parse_legacy_supported_visuals(
            &configuration,
            selected_start,
            selected_end,
            &mut operations,
            &mut unsupported,
        )?;
        LegacyCapProjectSnapshot::new(
            LEGACY_CAP_SNAPSHOT_VERSION,
            recording_meta,
            Some(project_config),
            segments,
            EditSpec {
                version: STUDIO_EDIT_VERSION,
                revision: 1,
                operations,
            },
            unsupported,
        )
    }
}

impl LegacyCapProjectPort for FilesystemLegacyCapProjectPort {
    fn source_tree_digest(&mut self) -> Result<Sha256Digest, StudioError> {
        Ok(self.parse_snapshot()?.source_tree_digest)
    }

    fn read_snapshot(&mut self) -> Result<LegacyCapProjectSnapshot, StudioError> {
        self.parse_snapshot()
    }
}

#[derive(Debug, Deserialize)]
struct LegacyRecordingMetaDocument {
    #[serde(rename = "platform", default)]
    _platform: Option<String>,
    #[serde(rename = "pretty_name", default)]
    _pretty_name: Option<String>,
    #[serde(rename = "sharing", default)]
    _sharing: Option<serde_json::Value>,
    #[serde(rename = "upload", default)]
    _upload: Option<serde_json::Value>,
    #[serde(default)]
    display: Option<LegacyMediaDocument>,
    #[serde(default)]
    camera: Option<LegacyMediaDocument>,
    #[serde(default)]
    audio: Option<LegacyMediaDocument>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    segments: Vec<LegacyRecordingSegmentDocument>,
    #[serde(default)]
    cursors: serde_json::Value,
    #[serde(default)]
    status: Option<LegacyRecordingStatusDocument>,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LegacyRecordingStatusDocument {
    status: String,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyRecordingSegmentDocument {
    display: LegacyMediaDocument,
    #[serde(default)]
    camera: Option<LegacyMediaDocument>,
    #[serde(default, alias = "audio")]
    mic: Option<LegacyMediaDocument>,
    #[serde(default)]
    system_audio: Option<LegacyMediaDocument>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    keyboard: Option<String>,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyMediaDocument {
    path: String,
    #[serde(default)]
    fps: Option<serde_json::Number>,
    #[serde(default)]
    start_time: Option<serde_json::Number>,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LegacyProjectConfigDocument {
    timeline: LegacyTimelineDocument,
    #[serde(default)]
    background: serde_json::Value,
    #[serde(default)]
    camera: serde_json::Value,
    #[serde(default)]
    audio: serde_json::Value,
    #[serde(default)]
    cursor: serde_json::Value,
    #[serde(default)]
    clips: serde_json::Value,
    #[serde(default)]
    annotations: serde_json::Value,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LegacyTimelineDocument {
    segments: Vec<LegacyTimelineSegmentDocument>,
    #[serde(rename = "zoomSegments", default)]
    zoom_segments: Vec<serde_json::Value>,
    #[serde(rename = "sceneSegments", default)]
    scene_segments: Vec<serde_json::Value>,
    #[serde(rename = "maskSegments", default)]
    mask_segments: Vec<serde_json::Value>,
    #[serde(rename = "textSegments", default)]
    text_segments: Vec<serde_json::Value>,
    #[serde(rename = "captionSegments", default)]
    caption_segments: Vec<serde_json::Value>,
    #[serde(rename = "keyboardSegments", default)]
    keyboard_segments: Vec<serde_json::Value>,
    #[serde(rename = "audioSegments", default)]
    audio_segments: Vec<serde_json::Value>,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LegacyTimelineSegmentDocument {
    #[serde(rename = "recordingSegment")]
    recording_segment: u32,
    timescale: serde_json::Number,
    start: serde_json::Number,
    end: serde_json::Number,
    #[serde(flatten)]
    unknown: BTreeMap<String, serde_json::Value>,
}

fn checked_legacy_path(
    root: &Path,
    relative: &LegacyCapRelativePath,
) -> Result<PathBuf, StudioError> {
    let mut candidate = root.to_path_buf();
    for component in relative.as_str().split('/') {
        candidate.push(component);
        let metadata =
            fs::symlink_metadata(&candidate).map_err(|_| StudioError::MalformedLegacyProject)?;
        if metadata.file_type().is_symlink() {
            return Err(StudioError::MalformedLegacyProject);
        }
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|_| StudioError::MalformedLegacyProject)?;
    if !canonical.starts_with(root) {
        return Err(StudioError::MalformedLegacyProject);
    }
    Ok(canonical)
}

fn read_legacy_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    relative: &LegacyCapRelativePath,
) -> Result<T, StudioError> {
    let path = checked_legacy_path(root, relative)?;
    let bytes = read_bounded_file(&path, MAX_STUDIO_DOCUMENT_BYTES)?
        .ok_or(StudioError::MalformedLegacyProject)?;
    serde_json::from_slice(&bytes).map_err(|_| StudioError::MalformedLegacyProject)
}

fn decimal_number_time(number: &serde_json::Number) -> Result<RationalTime, StudioError> {
    let encoded = number.to_string();
    if encoded.starts_with('-') || encoded.contains(['e', 'E']) {
        return Err(StudioError::MalformedLegacyProject);
    }
    let (whole, fraction) = encoded.split_once('.').unwrap_or((&encoded, ""));
    if fraction.len() > 9
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(StudioError::MalformedLegacyProject);
    }
    let scale = 10_u32
        .checked_pow(u32::try_from(fraction.len()).map_err(|_| StudioError::TimelineOverflow)?)
        .ok_or(StudioError::TimelineOverflow)?;
    let whole = whole
        .parse::<u64>()
        .map_err(|_| StudioError::TimelineOverflow)?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .map_err(|_| StudioError::TimelineOverflow)?
    };
    let ticks = whole
        .checked_mul(u64::from(scale))
        .and_then(|value| value.checked_add(fractional))
        .ok_or(StudioError::TimelineOverflow)?;
    let divisor = gcd_u128(u128::from(ticks), u128::from(scale));
    let reduced_ticks =
        u64::try_from(u128::from(ticks) / divisor).map_err(|_| StudioError::TimelineOverflow)?;
    let reduced_scale =
        u32::try_from(u128::from(scale) / divisor).map_err(|_| StudioError::TimelineOverflow)?;
    Ok(RationalTime::new(
        reduced_ticks,
        TimeBase::new(reduced_scale)?,
    ))
}

fn validate_legacy_media_timing(
    media: &LegacyMediaDocument,
    unsupported: &mut BTreeSet<LegacyUnsupportedEffect>,
) -> Result<(), StudioError> {
    LegacyCapRelativePath::new(media.path.clone())?;
    mark_legacy_unknowns("media", &media.unknown, unsupported);
    if media
        .fps
        .as_ref()
        .is_some_and(|fps| decimal_number_time(fps).is_err())
    {
        return Err(StudioError::MalformedLegacyProject);
    }
    if let Some(start_time) = &media.start_time
        && decimal_number_time(start_time)?.ticks != 0
    {
        unsupported.insert(LegacyUnsupportedEffect::Clip);
    }
    Ok(())
}

fn legacy_object_has_keys_other_than(
    value: &serde_json::Value,
    allowed: &[&str],
) -> Result<bool, StudioError> {
    if value.is_null() {
        return Ok(false);
    }
    let object = value
        .as_object()
        .ok_or(StudioError::MalformedLegacyProject)?;
    Ok(object.keys().any(|key| !allowed.contains(&key.as_str())))
}

fn mark_nondefault_legacy_audio(
    value: &serde_json::Value,
    unsupported: &mut BTreeSet<LegacyUnsupportedEffect>,
) -> Result<(), StudioError> {
    if value.is_null() {
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or(StudioError::MalformedLegacyProject)?;
    let boolean = |key: &str| -> Result<bool, StudioError> {
        object
            .get(key)
            .map(|value| value.as_bool().ok_or(StudioError::MalformedLegacyProject))
            .transpose()
            .map(Option::unwrap_or_default)
    };
    let nonzero_number = |key: &str| -> Result<bool, StudioError> {
        object
            .get(key)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .map(|number| number != 0.0)
                    .ok_or(StudioError::MalformedLegacyProject)
            })
            .transpose()
            .map(Option::unwrap_or_default)
    };
    let nondefault_stereo = object
        .get("micStereoMode")
        .map(|value| {
            value
                .as_str()
                .map(|mode| mode != "stereo")
                .ok_or(StudioError::MalformedLegacyProject)
        })
        .transpose()?
        .unwrap_or(false);
    if boolean("mute")?
        || boolean("improve")?
        || nonzero_number("micVolumeDb")?
        || nonzero_number("systemVolumeDb")?
        || nondefault_stereo
        || legacy_object_has_keys_other_than(
            value,
            &[
                "mute",
                "improve",
                "micVolumeDb",
                "micStereoMode",
                "systemVolumeDb",
            ],
        )?
    {
        unsupported.insert(LegacyUnsupportedEffect::UnknownField(
            legacy_unknown_field_marker("audio"),
        ));
    }
    Ok(())
}

fn parse_legacy_supported_visuals(
    configuration: &LegacyProjectConfigDocument,
    start: RationalTime,
    end: RationalTime,
    operations: &mut Vec<EditOperation>,
    unsupported: &mut BTreeSet<LegacyUnsupportedEffect>,
) -> Result<(), StudioError> {
    if let Some(source) = configuration
        .background
        .get("source")
        .and_then(serde_json::Value::as_object)
    {
        let kind = source.get("type").and_then(serde_json::Value::as_str);
        let color = source.get("value").and_then(serde_json::Value::as_array);
        if kind == Some("color") && color.is_some_and(|value| value.len() == 3) {
            let color = color.ok_or(StudioError::MalformedLegacyProject)?;
            let component = |index: usize| {
                color
                    .get(index)
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or(StudioError::MalformedLegacyProject)
            };
            operations.push(EditOperation::Background {
                start,
                end,
                style: BackgroundStyle::SolidRgb {
                    red: component(0)?,
                    green: component(1)?,
                    blue: component(2)?,
                },
            });
        } else if !configuration.background.is_null() {
            unsupported.insert(LegacyUnsupportedEffect::UnknownField(
                legacy_unknown_field_marker("background"),
            ));
        }
    }
    if configuration
        .camera
        .get("mirror")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        unsupported.insert(LegacyUnsupportedEffect::UnknownField(
            legacy_unknown_field_marker("camera.mirror"),
        ));
    }
    if configuration
        .camera
        .get("hide")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        operations.push(EditOperation::Layout {
            start,
            end,
            preset: LayoutPreset::ScreenOnly,
        });
    }
    Ok(())
}

fn mark_legacy_unknowns(
    namespace: &str,
    fields: &BTreeMap<String, serde_json::Value>,
    unsupported: &mut BTreeSet<LegacyUnsupportedEffect>,
) {
    for field in fields.keys() {
        unsupported.insert(LegacyUnsupportedEffect::UnknownField(
            legacy_unknown_field_marker(&format!("{namespace}.{field}")),
        ));
    }
}

fn legacy_unknown_field_marker(value: &str) -> u16 {
    let mut hash = 0x811c_u32;
    for byte in value.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let marker = ((hash >> 16) ^ hash) as u16;
    marker.max(1)
}

#[must_use]
pub fn pinned_cap_reference_digest() -> Sha256Digest {
    strong_sha256(b"CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b")
}

/// Imports a normalized, bounded view of Cap's real `.cap` directory schema.
/// The adapter exposes read methods only and the core fingerprints the source
/// tree before and after inspection, so compatibility reporting cannot mutate
/// `recording-meta.json`, `project-config.json`, or media originals.
pub fn import_legacy_cap<P: LegacyCapProjectPort>(
    port: &mut P,
    assignment: &LegacyIdAssignment,
) -> Result<LegacyImportOutcome, StudioError> {
    let before = port.source_tree_digest()?;
    let snapshot = port.read_snapshot()?;
    if snapshot.version > LEGACY_CAP_SNAPSHOT_VERSION {
        let after = port.source_tree_digest()?;
        if before != snapshot.source_tree_digest || before != after {
            return Err(StudioError::LegacySourceChanged);
        }
        return Ok(LegacyImportOutcome::UnsupportedNewer(
            LegacyCompatibilityReport {
                source_digest: before,
                found_version: snapshot.version,
                source_asset_count: 0,
                supported_effect_count: 0,
                unsupported_effects: BTreeSet::new(),
                actionable_message: "open this newer project with a compatible Cap editor",
            },
        ));
    }
    snapshot.validate()?;
    if snapshot.source_tree_digest != before {
        return Err(StudioError::LegacySourceChanged);
    }
    let decoded = legacy_assets(&snapshot);
    let report = LegacyCompatibilityReport {
        source_digest: before,
        found_version: snapshot.version,
        source_asset_count: decoded.len(),
        supported_effect_count: snapshot.edits.operations.len(),
        unsupported_effects: snapshot.unsupported_effects.clone(),
        actionable_message: if snapshot.unsupported_effects.is_empty() {
            "legacy .cap directory can be copied into the Studio v1 format"
        } else {
            "open with the legacy editor or remove the listed unsupported effects"
        },
    };
    if !snapshot.unsupported_effects.is_empty() {
        let after = port.source_tree_digest()?;
        if before != after {
            return Err(StudioError::LegacySourceChanged);
        }
        return Ok(LegacyImportOutcome::NeedsUserAction(report));
    }
    if assignment.asset_ids.len() != decoded.len() {
        return Err(StudioError::LegacyIdAssignmentMismatch);
    }
    let mut assets = Vec::with_capacity(decoded.len());
    let mut copy_entries = Vec::with_capacity(decoded.len());
    for (index, decoded) in decoded.into_iter().enumerate() {
        let id = *assignment
            .asset_ids
            .get(index)
            .ok_or(StudioError::LegacyIdAssignmentMismatch)?;
        let role = match decoded.track {
            TrackKind::Screen => "screen",
            TrackKind::Camera => "camera",
            TrackKind::Microphone => "microphone",
            TrackKind::SystemAudio => "system-audio",
        };
        let destination =
            StudioSourceName::new(format!("segment-{}-{role}.media", decoded.segment_index))?;
        assets.push(StudioAsset {
            version: STUDIO_ASSET_VERSION,
            id,
            track: decoded.track,
            source_name: destination.clone(),
            byte_len: decoded.file.byte_len,
            start: decoded.start,
            duration: decoded.duration,
            checksum: decoded.file.checksum,
            commit_state: AssetCommitState::DurableOriginal,
        });
        copy_entries.push(LegacyCopyPlanEntry {
            asset_id: id,
            track: decoded.track,
            source: decoded.file,
            destination,
        });
    }
    let manifest = StudioProjectManifest {
        version: STUDIO_PROJECT_VERSION,
        id: assignment.project_id,
        revision: 1,
        state: StudioState::Editing,
        assets,
        edits: EditSpec {
            version: STUDIO_EDIT_VERSION,
            revision: 1,
            operations: snapshot.edits.operations.clone(),
        },
    };
    manifest.validate()?;
    let copy_plan = LegacyCopyPlan {
        project_id: assignment.project_id,
        entries: copy_entries,
    };
    copy_plan.validate()?;
    let after = port.source_tree_digest()?;
    if before != after {
        return Err(StudioError::LegacySourceChanged);
    }
    Ok(LegacyImportOutcome::Imported(Box::new(LegacyImport {
        manifest,
        copy_plan,
        report,
        source_digest_before: before,
        source_digest_after: after,
    })))
}

fn legacy_assets(snapshot: &LegacyCapProjectSnapshot) -> Vec<LegacyDecoded> {
    let mut assets = Vec::new();
    for segment in &snapshot.segments {
        for (track, file) in [
            (TrackKind::Screen, Some(&segment.display)),
            (TrackKind::Camera, segment.camera.as_ref()),
            (TrackKind::Microphone, segment.microphone.as_ref()),
            (TrackKind::SystemAudio, segment.system_audio.as_ref()),
        ] {
            if let Some(file) = file {
                assets.push(LegacyDecoded {
                    track,
                    segment_index: segment.index,
                    start: segment.start,
                    duration: segment.duration,
                    file: file.clone(),
                });
            }
        }
    }
    assets
}

fn digest_legacy_cap_snapshot(
    snapshot: &LegacyCapProjectSnapshot,
) -> Result<Sha256Digest, StudioError> {
    let mut writer = CanonicalWriter::new();
    writer.u16(snapshot.version);
    writer.digest(snapshot.reference_revision)?;
    encode_legacy_cap_file(&mut writer, &snapshot.recording_meta)?;
    writer.bool(snapshot.project_config.is_some());
    if let Some(project_config) = &snapshot.project_config {
        encode_legacy_cap_file(&mut writer, project_config)?;
    }
    writer.u16(u16::try_from(snapshot.segments.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for segment in &snapshot.segments {
        writer.u32(segment.index);
        encode_time(&mut writer, segment.start);
        encode_time(&mut writer, segment.duration);
        encode_legacy_cap_file(&mut writer, &segment.display)?;
        for optional in [
            segment.camera.as_ref(),
            segment.microphone.as_ref(),
            segment.system_audio.as_ref(),
        ] {
            writer.bool(optional.is_some());
            if let Some(file) = optional {
                encode_legacy_cap_file(&mut writer, file)?;
            }
        }
    }
    writer.digest(digest_edit_spec(&snapshot.edits)?)?;
    writer.u16(
        u16::try_from(snapshot.unsupported_effects.len())
            .map_err(|_| StudioError::DocumentTooLarge)?,
    );
    for effect in &snapshot.unsupported_effects {
        let (tag, detail) = legacy_unsupported_tag(*effect);
        writer.u16(tag);
        writer.u16(detail);
    }
    Ok(strong_sha256(&writer.finish()?))
}

fn encode_legacy_cap_file(
    writer: &mut CanonicalWriter,
    file: &LegacyCapFileDescriptor,
) -> Result<(), StudioError> {
    writer.string(file.relative_path.as_str())?;
    writer.u64(file.byte_len);
    writer.digest(file.checksum.0)
}

const fn legacy_unsupported_tag(effect: LegacyUnsupportedEffect) -> (u16, u16) {
    match effect {
        LegacyUnsupportedEffect::Zoom => (1, 0),
        LegacyUnsupportedEffect::Scene => (2, 0),
        LegacyUnsupportedEffect::Mask => (3, 0),
        LegacyUnsupportedEffect::Text => (4, 0),
        LegacyUnsupportedEffect::Caption => (5, 0),
        LegacyUnsupportedEffect::Keyboard => (6, 0),
        LegacyUnsupportedEffect::ImportedAudio => (7, 0),
        LegacyUnsupportedEffect::Clip => (8, 0),
        LegacyUnsupportedEffect::Annotation => (9, 0),
        LegacyUnsupportedEffect::ChromaKey => (10, 0),
        LegacyUnsupportedEffect::DropShadow => (11, 0),
        LegacyUnsupportedEffect::SpringKeyframes => (12, 0),
        LegacyUnsupportedEffect::ExternalFont => (13, 0),
        LegacyUnsupportedEffect::UnknownField(value) => (u16::MAX, value),
    }
}

// ---------------------------------------------------------------------------
// Isolated recording graph and durable journal ownership

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureElementFamily {
    NativeScreenBridge,
    NativeCameraBridge,
    NativeMicrophoneBridge,
    NativeSystemAudioBridge,
    MatroskaMux,
    Vp9Encoder,
    OpusEncoder,
    FlacEncoder,
}

impl CaptureElementFamily {
    const fn tag(self) -> u8 {
        match self {
            Self::NativeScreenBridge => 1,
            Self::NativeCameraBridge => 2,
            Self::NativeMicrophoneBridge => 3,
            Self::NativeSystemAudioBridge => 4,
            Self::MatroskaMux => 5,
            Self::Vp9Encoder => 6,
            Self::OpusEncoder => 7,
            Self::FlacEncoder => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedMediaQueue {
    pub max_buffers: u32,
    pub max_bytes: u64,
    pub max_time_ns: u64,
}

impl BoundedMediaQueue {
    pub fn validate(self) -> Result<Self, StudioError> {
        if self.max_buffers == 0
            || self.max_buffers > MAX_STUDIO_QUEUE_BUFFERS
            || self.max_bytes == 0
            || self.max_bytes > MAX_STUDIO_QUEUE_BYTES
            || self.max_time_ns == 0
            || self.max_time_ns > MAX_STUDIO_QUEUE_TIME_NS
        {
            return Err(StudioError::UnboundedMediaQueue);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedTrackBranch {
    pub track: TrackKind,
    pub asset_id: StudioAssetId,
    pub temporary_name: StudioSourceName,
    pub source: CaptureElementFamily,
    pub encoder: CaptureElementFamily,
    pub muxer: CaptureElementFamily,
    pub time_base: TimeBase,
    pub queue: BoundedMediaQueue,
}

impl IsolatedTrackBranch {
    fn validate(&self) -> Result<(), StudioError> {
        self.queue.validate()?;
        let expected_source = match self.track {
            TrackKind::Screen => CaptureElementFamily::NativeScreenBridge,
            TrackKind::Camera => CaptureElementFamily::NativeCameraBridge,
            TrackKind::Microphone => CaptureElementFamily::NativeMicrophoneBridge,
            TrackKind::SystemAudio => CaptureElementFamily::NativeSystemAudioBridge,
        };
        let encoder_valid = match self.track {
            TrackKind::Screen | TrackKind::Camera => {
                self.encoder == CaptureElementFamily::Vp9Encoder
            }
            TrackKind::Microphone | TrackKind::SystemAudio => matches!(
                self.encoder,
                CaptureElementFamily::OpusEncoder | CaptureElementFamily::FlacEncoder
            ),
        };
        let time_base_valid = match self.track {
            TrackKind::Screen | TrackKind::Camera => self.time_base.0 == 90_000,
            TrackKind::Microphone | TrackKind::SystemAudio => self.time_base.0 == 48_000,
        };
        if self.source != expected_source
            || !encoder_valid
            || !time_base_valid
            || self.muxer != CaptureElementFamily::MatroskaMux
        {
            return Err(StudioError::InvalidRecordingGraph);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioRecordingGraphSpec {
    pub project_id: StudioProjectId,
    pub clock_id: StudioOperationId,
    pub branches: Vec<IsolatedTrackBranch>,
}

impl StudioRecordingGraphSpec {
    pub fn new(
        project_id: StudioProjectId,
        clock_id: StudioOperationId,
        branches: Vec<IsolatedTrackBranch>,
    ) -> Result<Self, StudioError> {
        let graph = Self {
            project_id,
            clock_id,
            branches,
        };
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), StudioError> {
        if self.branches.len() != 4 {
            return Err(StudioError::InvalidRecordingGraph);
        }
        let mut tracks = BTreeSet::new();
        let mut assets = BTreeSet::new();
        let mut names = BTreeSet::new();
        for branch in &self.branches {
            branch.validate()?;
            if !tracks.insert(branch.track)
                || !assets.insert(branch.asset_id)
                || !names.insert(branch.temporary_name.clone())
            {
                return Err(StudioError::InvalidRecordingGraph);
            }
        }
        if tracks
            != BTreeSet::from([
                TrackKind::Screen,
                TrackKind::Camera,
                TrackKind::Microphone,
                TrackKind::SystemAudio,
            ])
        {
            return Err(StudioError::InvalidRecordingGraph);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioJournalCasRequest {
    pub project_id: StudioProjectId,
    pub expected_revision: u64,
    pub expected_fence: u64,
    pub next: StudioJournalSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudioPortOutcome<T> {
    Committed(T),
    Conflict(Box<T>),
    AcknowledgementLost,
}

pub trait StudioJournalPort {
    fn load(
        &mut self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioJournalSnapshot>, StudioError>;

    fn create(
        &mut self,
        initial: StudioJournalSnapshot,
    ) -> Result<StudioPortOutcome<StudioJournalSnapshot>, StudioError>;

    fn compare_and_swap(
        &mut self,
        request: StudioJournalCasRequest,
    ) -> Result<StudioPortOutcome<StudioJournalSnapshot>, StudioError>;
}

/// Canonical on-disk Studio journal store. Each compare-and-swap is serialized
/// by a per-project create-new lock, writes a checksummed document to a
/// same-directory temporary file, syncs it, atomically renames it, and syncs
/// the containing directory before acknowledging success.
pub struct FilesystemStudioJournalStore {
    root: PathBuf,
}

impl fmt::Debug for FilesystemStudioJournalStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemStudioJournalStore")
            .field("root", &"<redacted>")
            .finish()
    }
}

impl FilesystemStudioJournalStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, StudioError> {
        let root = prepare_storage_root(root.as_ref())?;
        Ok(Self { root })
    }

    fn journal_path(&self, project_id: StudioProjectId) -> PathBuf {
        self.root
            .join(format!("{}.studio-journal", opaque_id_hex(project_id.0)))
    }

    fn lock_path(&self, project_id: StudioProjectId) -> PathBuf {
        self.root.join(format!(
            "{}.studio-journal.lock",
            opaque_id_hex(project_id.0)
        ))
    }

    fn acquire_lock(
        &self,
        project_id: StudioProjectId,
    ) -> Result<FilesystemStudioLock, StudioError> {
        let path = self.lock_path(project_id);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    StudioError::StaleJournal
                } else {
                    StudioError::StorageIo
                }
            })?;
        file.write_all(b"frame-studio-journal-lock-v1\n")
            .and_then(|()| file.sync_all())
            .map_err(|_| StudioError::StorageIo)?;
        Ok(FilesystemStudioLock { path })
    }

    fn read_snapshot(
        &self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioJournalSnapshot>, StudioError> {
        let path = self.journal_path(project_id);
        let bytes = match read_bounded_file(&path, MAX_STUDIO_DOCUMENT_BYTES)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let snapshot = StudioDocumentCodec::decode_journal(&bytes)?;
        if snapshot.project_id != project_id {
            return Err(StudioError::JournalCorrupt);
        }
        Ok(Some(snapshot))
    }

    fn write_snapshot(&self, snapshot: &StudioJournalSnapshot) -> Result<(), StudioError> {
        let bytes = StudioDocumentCodec::encode_journal(snapshot)?;
        atomic_replace_file(
            &self.journal_path(snapshot.project_id),
            &bytes,
            snapshot.revision,
        )
    }
}

struct FilesystemStudioLock {
    path: PathBuf,
}

impl Drop for FilesystemStudioLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = sync_directory(parent);
        }
    }
}

impl StudioJournalPort for FilesystemStudioJournalStore {
    fn load(
        &mut self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioJournalSnapshot>, StudioError> {
        self.read_snapshot(project_id)
    }

    fn create(
        &mut self,
        initial: StudioJournalSnapshot,
    ) -> Result<StudioPortOutcome<StudioJournalSnapshot>, StudioError> {
        initial.validate()?;
        let _lock = self.acquire_lock(initial.project_id)?;
        if let Some(existing) = self.read_snapshot(initial.project_id)? {
            return Ok(StudioPortOutcome::Conflict(Box::new(existing)));
        }
        self.write_snapshot(&initial)?;
        Ok(StudioPortOutcome::Committed(initial))
    }

    fn compare_and_swap(
        &mut self,
        request: StudioJournalCasRequest,
    ) -> Result<StudioPortOutcome<StudioJournalSnapshot>, StudioError> {
        request.next.validate()?;
        if request.next.project_id != request.project_id {
            return Err(StudioError::JournalCorrupt);
        }
        let _lock = self.acquire_lock(request.project_id)?;
        let current = self
            .read_snapshot(request.project_id)?
            .ok_or(StudioError::JournalCorrupt)?;
        if current.revision != request.expected_revision || current.fence != request.expected_fence
        {
            return Ok(StudioPortOutcome::Conflict(Box::new(current)));
        }
        self.write_snapshot(&request.next)?;
        Ok(StudioPortOutcome::Committed(request.next))
    }
}

fn prepare_storage_root(path: &Path) -> Result<PathBuf, StudioError> {
    fs::create_dir_all(path).map_err(|_| StudioError::StorageIo)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| StudioError::StorageIo)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StudioError::UnsafeStoragePath);
    }
    path.canonicalize().map_err(|_| StudioError::StorageIo)
}

fn opaque_id_hex(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(32);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn read_bounded_file(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>, StudioError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(StudioError::StorageIo),
    };
    let metadata = file.metadata().map_err(|_| StudioError::StorageIo)?;
    if !metadata.is_file()
        || usize::try_from(metadata.len()).map_or(true, |length| length > maximum)
    {
        return Err(StudioError::DocumentTooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| StudioError::StorageIo)?;
    if bytes.len() > maximum {
        return Err(StudioError::DocumentTooLarge);
    }
    Ok(Some(bytes))
}

fn atomic_replace_file(path: &Path, bytes: &[u8], nonce: u64) -> Result<(), StudioError> {
    let parent = path.parent().ok_or(StudioError::UnsafeStoragePath)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StudioError::UnsafeStoragePath)?;
    let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| StudioError::StorageIo)?;
    let result = (|| {
        file.write_all(bytes).map_err(|_| StudioError::StorageIo)?;
        file.sync_all().map_err(|_| StudioError::StorageIo)?;
        fs::rename(&temporary, path).map_err(|_| StudioError::StorageIo)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_directory(path: &Path) -> Result<(), StudioError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| StudioError::StorageIo)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalCommitOutcome {
    Committed,
    ReconciledAfterLostAcknowledgement,
    IdempotentReplay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalAdvanceRequest {
    pub expected_revision: u64,
    pub expected_fence: u64,
    pub operation_id: StudioOperationId,
    pub command_digest: Sha256Digest,
    pub boundary: JournalBoundary,
    pub pending_asset: Option<PendingAssetCommit>,
    pub pending_edit: Option<PendingEditSave>,
    pub pending_render: Option<PendingRender>,
    pub receipt_kind: ReceiptKind,
    pub outcome_digest: Sha256Digest,
}

#[derive(Debug)]
pub struct DurableStudioJournal<P> {
    port: P,
    snapshot: StudioJournalSnapshot,
}

impl<P: StudioJournalPort> DurableStudioJournal<P> {
    /// Opens and validates an existing durable journal. Recovery and render
    /// dispatch authorization must originate from this path (or `create`), not
    /// from a caller-assembled reservation value.
    pub fn open(mut port: P, project_id: StudioProjectId) -> Result<Self, StudioError> {
        let snapshot = port.load(project_id)?.ok_or(StudioError::JournalCorrupt)?;
        snapshot.validate()?;
        if snapshot.project_id != project_id {
            return Err(StudioError::JournalCorrupt);
        }
        Ok(Self { port, snapshot })
    }

    pub fn create(mut port: P, initial: StudioJournalSnapshot) -> Result<Self, StudioError> {
        initial.validate()?;
        let committed = match port.create(initial.clone())? {
            StudioPortOutcome::Committed(committed) => {
                if committed != initial {
                    return Err(StudioError::JournalCorrupt);
                }
                committed
            }
            StudioPortOutcome::Conflict(existing) => {
                if *existing == initial {
                    *existing
                } else {
                    return Err(StudioError::StaleJournal);
                }
            }
            StudioPortOutcome::AcknowledgementLost => {
                let loaded = port
                    .load(initial.project_id)?
                    .ok_or(StudioError::AmbiguousJournalCommit)?;
                if loaded != initial {
                    return Err(StudioError::AmbiguousJournalCommit);
                }
                loaded
            }
        };
        Ok(Self {
            port,
            snapshot: committed,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> &StudioJournalSnapshot {
        &self.snapshot
    }

    pub fn into_port(self) -> P {
        self.port
    }

    pub fn advance(
        &mut self,
        request: JournalAdvanceRequest,
    ) -> Result<JournalCommitOutcome, StudioError> {
        let JournalAdvanceRequest {
            expected_revision,
            expected_fence,
            operation_id,
            command_digest,
            boundary,
            pending_asset,
            pending_edit,
            pending_render,
            receipt_kind,
            outcome_digest,
        } = request;
        if let Some(receipt) = self.snapshot.receipts.get(&operation_id) {
            return if receipt.command_digest == command_digest
                && receipt.kind == receipt_kind
                && receipt.outcome_digest == outcome_digest
            {
                Ok(JournalCommitOutcome::IdempotentReplay)
            } else {
                Err(StudioError::IdempotencyConflict)
            };
        }
        if self.snapshot.revision != expected_revision || self.snapshot.fence != expected_fence {
            return Err(StudioError::StaleJournal);
        }
        if !valid_journal_transition(self.snapshot.boundary, boundary) {
            return Err(StudioError::InvalidJournalTransition {
                from: self.snapshot.boundary,
                to: boundary,
            });
        }
        if !receipt_matches_boundary(receipt_kind, boundary) {
            return Err(StudioError::InvalidJournalReceipt);
        }
        let mut next = self.snapshot.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(StudioError::JournalCorrupt)?;
        next.boundary = boundary;
        next.last_operation_id = Some(operation_id);
        next.pending_asset = pending_asset;
        next.pending_edit = pending_edit;
        next.pending_render = pending_render;
        if next.receipts.len() == MAX_STUDIO_RECEIPTS {
            return Err(StudioError::DocumentTooLarge);
        }
        next.receipts.insert(
            operation_id,
            StudioOperationReceipt {
                operation_id,
                kind: receipt_kind,
                command_digest,
                outcome_digest,
            },
        );
        validate_journal_pending_transition(&self.snapshot, &next)?;
        next.validate()?;
        let request = StudioJournalCasRequest {
            project_id: next.project_id,
            expected_revision,
            expected_fence,
            next: next.clone(),
        };
        let outcome = self.port.compare_and_swap(request)?;
        match outcome {
            StudioPortOutcome::Committed(committed) => {
                if committed != next {
                    return Err(StudioError::JournalCorrupt);
                }
                self.snapshot = committed;
                Ok(JournalCommitOutcome::Committed)
            }
            StudioPortOutcome::Conflict(current) => {
                current.validate()?;
                if current.project_id == next.project_id
                    && current.revision >= next.revision
                    && current.receipts.get(&operation_id) == next.receipts.get(&operation_id)
                {
                    self.snapshot = *current;
                    Ok(JournalCommitOutcome::ReconciledAfterLostAcknowledgement)
                } else {
                    self.snapshot = *current;
                    Err(StudioError::StaleJournal)
                }
            }
            StudioPortOutcome::AcknowledgementLost => {
                let loaded = self
                    .port
                    .load(next.project_id)?
                    .ok_or(StudioError::AmbiguousJournalCommit)?;
                loaded.validate()?;
                if loaded.project_id != next.project_id
                    || loaded.revision < next.revision
                    || loaded.receipts.get(&operation_id) != next.receipts.get(&operation_id)
                {
                    return Err(StudioError::AmbiguousJournalCommit);
                }
                self.snapshot = loaded;
                Ok(JournalCommitOutcome::ReconciledAfterLostAcknowledgement)
            }
        }
    }

    pub fn take_ownership(
        &mut self,
        expected_revision: u64,
        expected_fence: u64,
        new_owner: StudioWorkerId,
    ) -> Result<(), StudioError> {
        if self.snapshot.revision != expected_revision || self.snapshot.fence != expected_fence {
            return Err(StudioError::StaleJournal);
        }
        let mut next = self.snapshot.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(StudioError::JournalCorrupt)?;
        next.fence = next
            .fence
            .checked_add(1)
            .ok_or(StudioError::JournalCorrupt)?;
        next.owner = new_owner;
        match self.port.compare_and_swap(StudioJournalCasRequest {
            project_id: next.project_id,
            expected_revision,
            expected_fence,
            next: next.clone(),
        })? {
            StudioPortOutcome::Committed(committed) if committed == next => {
                self.snapshot = committed;
                Ok(())
            }
            StudioPortOutcome::Conflict(current) => {
                current.validate()?;
                if current.project_id == next.project_id
                    && current.owner == next.owner
                    && current.fence == next.fence
                    && current.revision >= next.revision
                {
                    self.snapshot = *current;
                    return Ok(());
                }
                self.snapshot = *current;
                Err(StudioError::StaleJournal)
            }
            StudioPortOutcome::AcknowledgementLost => {
                let loaded = self
                    .port
                    .load(next.project_id)?
                    .ok_or(StudioError::AmbiguousJournalCommit)?;
                loaded.validate()?;
                if loaded.project_id != next.project_id
                    || loaded.owner != next.owner
                    || loaded.fence != next.fence
                    || loaded.revision < next.revision
                {
                    return Err(StudioError::AmbiguousJournalCommit);
                }
                self.snapshot = loaded;
                Ok(())
            }
            StudioPortOutcome::Committed(_) => Err(StudioError::JournalCorrupt),
        }
    }

    fn advance_render_lifecycle(
        &mut self,
        boundary: JournalBoundary,
        terminal_receipt: Option<RenderReceipt>,
    ) -> Result<(), StudioError> {
        let pending = self
            .snapshot
            .pending_render
            .as_ref()
            .ok_or(StudioError::RenderReservationRequired)?;
        if self.snapshot.boundary == boundary && pending.terminal_receipt == terminal_receipt {
            return Ok(());
        }
        if !valid_journal_transition(self.snapshot.boundary, boundary) {
            return Err(StudioError::InvalidJournalTransition {
                from: self.snapshot.boundary,
                to: boundary,
            });
        }
        let receipt_kind = render_receipt_kind(boundary)?;
        let expected_revision = self.snapshot.revision;
        let expected_fence = self.snapshot.fence;
        let mut next = self.snapshot.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(StudioError::JournalCorrupt)?;
        next.boundary = boundary;
        next.last_operation_id = Some(pending.operation_id);
        let next_pending = next
            .pending_render
            .as_mut()
            .ok_or(StudioError::RenderReservationRequired)?;
        next_pending.terminal_receipt = terminal_receipt;
        let command_digest = digest_render_lifecycle(next_pending, boundary, false)?;
        let outcome_digest = digest_render_lifecycle(next_pending, boundary, true)?;
        next.receipts.insert(
            pending.operation_id,
            StudioOperationReceipt {
                operation_id: pending.operation_id,
                kind: receipt_kind,
                command_digest,
                outcome_digest,
            },
        );
        validate_journal_pending_transition(&self.snapshot, &next)?;
        next.validate()?;
        let committed = match self.port.compare_and_swap(StudioJournalCasRequest {
            project_id: next.project_id,
            expected_revision,
            expected_fence,
            next: next.clone(),
        })? {
            StudioPortOutcome::Committed(committed) if committed == next => committed,
            StudioPortOutcome::Conflict(current) => {
                current.validate()?;
                if current.project_id != next.project_id
                    || current.revision < next.revision
                    || current.boundary != next.boundary
                    || current.pending_render != next.pending_render
                {
                    self.snapshot = *current;
                    return Err(StudioError::StaleJournal);
                }
                *current
            }
            StudioPortOutcome::AcknowledgementLost => {
                let loaded = self
                    .port
                    .load(next.project_id)?
                    .ok_or(StudioError::AmbiguousJournalCommit)?;
                loaded.validate()?;
                if loaded.revision < next.revision
                    || loaded.boundary != next.boundary
                    || loaded.pending_render != next.pending_render
                {
                    return Err(StudioError::AmbiguousJournalCommit);
                }
                loaded
            }
            StudioPortOutcome::Committed(_) => return Err(StudioError::JournalCorrupt),
        };
        self.snapshot = committed;
        Ok(())
    }
}

fn render_receipt_kind(boundary: JournalBoundary) -> Result<ReceiptKind, StudioError> {
    match boundary {
        JournalBoundary::RenderPrepared => Ok(ReceiptKind::RenderPrepared),
        JournalBoundary::RenderRunning => Ok(ReceiptKind::RenderStarted),
        JournalBoundary::RenderFinalizing => Ok(ReceiptKind::RenderFinalizing),
        JournalBoundary::RenderCommitted => Ok(ReceiptKind::RenderCommitted),
        JournalBoundary::RenderCancelled => Ok(ReceiptKind::PartialDeleted),
        JournalBoundary::FailedRecoverably => Ok(ReceiptKind::RecoveryApplied),
        _ => Err(StudioError::InvalidJournalReceipt),
    }
}

fn digest_render_lifecycle(
    pending: &PendingRender,
    boundary: JournalBoundary,
    include_terminal: bool,
) -> Result<Sha256Digest, StudioError> {
    let mut writer = CanonicalWriter::new();
    writer.id(pending.operation_id.canonical_bytes());
    writer.id(pending.export_id.canonical_bytes());
    writer.u64(pending.fence);
    writer.digest(pending.source_set_digest)?;
    writer.digest(pending.plan_digest)?;
    writer.digest(pending.render_spec_digest)?;
    writer.u8(export_profile_tag(pending.profile));
    writer.string(pending.output_name.as_str())?;
    writer.u8(boundary.tag());
    if include_terminal {
        writer.bool(pending.terminal_receipt.is_some());
        if let Some(receipt) = &pending.terminal_receipt {
            encode_render_receipt(&mut writer, receipt)?;
        }
    }
    Ok(strong_sha256(&writer.finish()?))
}

fn same_asset_identity(left: &StudioAsset, right: &StudioAsset) -> bool {
    left.version == right.version
        && left.id == right.id
        && left.track == right.track
        && left.source_name == right.source_name
        && left.byte_len == right.byte_len
        && left.start == right.start
        && left.duration == right.duration
        && left.checksum == right.checksum
}

fn same_render_identity(left: &PendingRender, right: &PendingRender) -> bool {
    left.operation_id == right.operation_id
        && left.export_id == right.export_id
        && left.fence == right.fence
        && left.source_set_digest == right.source_set_digest
        && left.plan_digest == right.plan_digest
        && left.render_spec_digest == right.render_spec_digest
        && left.profile == right.profile
        && left.output_name == right.output_name
}

fn validate_journal_pending_transition(
    current: &StudioJournalSnapshot,
    next: &StudioJournalSnapshot,
) -> Result<(), StudioError> {
    use JournalBoundary::{
        AssetCommitRequested, AssetCommitted, FailedRecoverably, RenderCancelled, RenderCommitted,
        RenderFinalizing, RenderPrepared, RenderRunning, TempAssetDurable, TempAssetReserved,
    };
    match (current.boundary, next.boundary) {
        (TempAssetReserved, TempAssetDurable)
        | (TempAssetDurable, AssetCommitRequested)
        | (AssetCommitRequested, AssetCommitted)
        | (FailedRecoverably, AssetCommitted) => {
            let current_asset = current
                .pending_asset
                .as_ref()
                .ok_or(StudioError::JournalCorrupt)?;
            let next_asset = next
                .pending_asset
                .as_ref()
                .ok_or(StudioError::JournalCorrupt)?;
            if !same_asset_identity(&current_asset.asset, &next_asset.asset) {
                return Err(StudioError::JournalPendingIdentityChanged);
            }
            let expected_state = if next.boundary == AssetCommitted {
                AssetCommitState::DurableOriginal
            } else {
                AssetCommitState::Temporary
            };
            if current_asset.asset.commit_state != AssetCommitState::Temporary
                || next_asset.asset.commit_state != expected_state
            {
                return Err(StudioError::JournalPendingIdentityChanged);
            }
        }
        (RenderPrepared, RenderRunning | RenderCancelled)
        | (RenderRunning, RenderFinalizing | RenderCancelled)
        | (RenderFinalizing, RenderCancelled)
        | (FailedRecoverably, RenderCancelled) => {
            if current.pending_render != next.pending_render {
                return Err(StudioError::JournalPendingIdentityChanged);
            }
        }
        (RenderFinalizing | FailedRecoverably, RenderCommitted) => {
            let current_render = current
                .pending_render
                .as_ref()
                .ok_or(StudioError::JournalCorrupt)?;
            let next_render = next
                .pending_render
                .as_ref()
                .ok_or(StudioError::JournalCorrupt)?;
            if !same_render_identity(current_render, next_render)
                || current_render.terminal_receipt.is_some()
                || next_render.terminal_receipt.is_none()
            {
                return Err(StudioError::JournalPendingIdentityChanged);
            }
        }
        (JournalBoundary::EditSavePrepared, JournalBoundary::EditSaveCommitted)
        | (FailedRecoverably, JournalBoundary::EditSaveCommitted) => {
            if current.pending_edit != next.pending_edit {
                return Err(StudioError::JournalPendingIdentityChanged);
            }
        }
        (FailedRecoverably, JournalBoundary::CaptureStarted) => {
            let durable_asset_or_none = current.pending_asset.as_ref().is_none_or(|pending| {
                pending.asset.commit_state == AssetCommitState::DurableOriginal
            });
            if !durable_asset_or_none
                || current.pending_edit.is_some()
                || current.pending_render.is_some()
                || next.pending_asset.is_some()
                || next.pending_edit.is_some()
                || next.pending_render.is_some()
            {
                return Err(StudioError::JournalPendingIdentityChanged);
            }
        }
        (_, FailedRecoverably)
            if current.pending_asset != next.pending_asset
                || current.pending_edit != next.pending_edit
                || current.pending_render != next.pending_render =>
        {
            return Err(StudioError::JournalPendingIdentityChanged);
        }
        _ => {}
    }
    Ok(())
}

const fn valid_journal_transition(from: JournalBoundary, to: JournalBoundary) -> bool {
    matches!(
        (from, to),
        (
            JournalBoundary::Created,
            JournalBoundary::RecordingGraphPrepared
        ) | (
            JournalBoundary::RecordingGraphPrepared,
            JournalBoundary::CaptureStarted
        ) | (
            JournalBoundary::CaptureStarted,
            JournalBoundary::TempAssetReserved
        ) | (
            JournalBoundary::TempAssetReserved,
            JournalBoundary::TempAssetDurable
        ) | (
            JournalBoundary::TempAssetDurable,
            JournalBoundary::AssetCommitRequested
        ) | (
            JournalBoundary::AssetCommitRequested,
            JournalBoundary::AssetCommitted
        ) | (
            JournalBoundary::AssetCommitted,
            JournalBoundary::CaptureStarted
        ) | (
            JournalBoundary::CaptureStarted,
            JournalBoundary::RecordingStopped
        ) | (
            JournalBoundary::RecordingStopped,
            JournalBoundary::EditSavePrepared
        ) | (
            JournalBoundary::EditSaveCommitted,
            JournalBoundary::EditSavePrepared
        ) | (
            JournalBoundary::EditSavePrepared,
            JournalBoundary::EditSaveCommitted
        ) | (
            JournalBoundary::RecordingStopped,
            JournalBoundary::RenderPrepared
        ) | (
            JournalBoundary::EditSaveCommitted,
            JournalBoundary::RenderPrepared
        ) | (
            JournalBoundary::RenderPrepared,
            JournalBoundary::RenderRunning
        ) | (
            JournalBoundary::RenderRunning,
            JournalBoundary::RenderFinalizing
        ) | (
            JournalBoundary::RenderFinalizing,
            JournalBoundary::RenderCommitted
        ) | (
            JournalBoundary::RenderCommitted | JournalBoundary::RenderCancelled,
            JournalBoundary::EditSavePrepared | JournalBoundary::RenderPrepared
        ) | (
            JournalBoundary::RenderPrepared,
            JournalBoundary::RenderCancelled
        ) | (
            JournalBoundary::RenderRunning,
            JournalBoundary::RenderCancelled
        ) | (
            JournalBoundary::RenderFinalizing,
            JournalBoundary::RenderCancelled
        ) | (_, JournalBoundary::FailedRecoverably)
            | (
                JournalBoundary::FailedRecoverably,
                JournalBoundary::CaptureStarted
            )
            | (
                JournalBoundary::FailedRecoverably,
                JournalBoundary::EditSaveCommitted
            )
            | (
                JournalBoundary::FailedRecoverably,
                JournalBoundary::AssetCommitted
            )
            | (
                JournalBoundary::FailedRecoverably,
                JournalBoundary::RenderCommitted | JournalBoundary::RenderCancelled
            )
    )
}

const fn receipt_matches_boundary(kind: ReceiptKind, boundary: JournalBoundary) -> bool {
    matches!(
        (kind, boundary),
        (
            ReceiptKind::GraphPrepared,
            JournalBoundary::RecordingGraphPrepared
        ) | (ReceiptKind::CaptureStarted, JournalBoundary::CaptureStarted)
            | (
                ReceiptKind::TempReserved,
                JournalBoundary::TempAssetReserved
            )
            | (ReceiptKind::TempDurable, JournalBoundary::TempAssetDurable)
            | (
                ReceiptKind::AssetCommitRequested,
                JournalBoundary::AssetCommitRequested
            )
            | (ReceiptKind::AssetCommitted, JournalBoundary::AssetCommitted)
            | (
                ReceiptKind::RecordingStopped,
                JournalBoundary::RecordingStopped
            )
            | (ReceiptKind::EditPrepared, JournalBoundary::EditSavePrepared)
            | (
                ReceiptKind::EditCommitted,
                JournalBoundary::EditSaveCommitted
            )
            | (ReceiptKind::RenderPrepared, JournalBoundary::RenderPrepared)
            | (ReceiptKind::RenderStarted, JournalBoundary::RenderRunning)
            | (
                ReceiptKind::RenderFinalizing,
                JournalBoundary::RenderFinalizing
            )
            | (
                ReceiptKind::RenderCommitted,
                JournalBoundary::RenderCommitted
            )
            | (
                ReceiptKind::PartialDeleted,
                JournalBoundary::RenderCancelled
            )
            | (
                ReceiptKind::RecoveryApplied,
                JournalBoundary::FailedRecoverably
            )
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudioRecoveryDirective {
    DiscardUnstartedTemporaryFiles,
    ResumeOrSealIsolatedTracks,
    DeleteUncommittedTemporaryAsset,
    ProbeAndCommitExactTemporaryAsset,
    ContinueRecording,
    OpenEditor,
    ReconcileEditSaveByDigest,
    DeletePartialRenderThenOpenEditor,
    VerifyCommittedRenderThenOpenEditor,
    RequireOperatorDecision,
}

#[must_use]
pub const fn recovery_directive(boundary: JournalBoundary) -> StudioRecoveryDirective {
    match boundary {
        JournalBoundary::Created | JournalBoundary::RecordingGraphPrepared => {
            StudioRecoveryDirective::DiscardUnstartedTemporaryFiles
        }
        JournalBoundary::CaptureStarted => StudioRecoveryDirective::ResumeOrSealIsolatedTracks,
        JournalBoundary::TempAssetReserved => {
            StudioRecoveryDirective::DeleteUncommittedTemporaryAsset
        }
        JournalBoundary::TempAssetDurable | JournalBoundary::AssetCommitRequested => {
            StudioRecoveryDirective::ProbeAndCommitExactTemporaryAsset
        }
        JournalBoundary::AssetCommitted => StudioRecoveryDirective::ContinueRecording,
        JournalBoundary::RecordingStopped | JournalBoundary::EditSaveCommitted => {
            StudioRecoveryDirective::OpenEditor
        }
        JournalBoundary::EditSavePrepared => StudioRecoveryDirective::ReconcileEditSaveByDigest,
        JournalBoundary::RenderPrepared
        | JournalBoundary::RenderRunning
        | JournalBoundary::RenderFinalizing
        | JournalBoundary::RenderCancelled => {
            StudioRecoveryDirective::DeletePartialRenderThenOpenEditor
        }
        JournalBoundary::RenderCommitted => {
            StudioRecoveryDirective::VerifyCommittedRenderThenOpenEditor
        }
        JournalBoundary::FailedRecoverably => StudioRecoveryDirective::RequireOperatorDecision,
    }
}

/// Non-cloneable authorization to atomically move one verified temporary asset
/// into the immutable originals namespace.
#[derive(Debug)]
pub struct TempAssetCommitTicket {
    project_id: StudioProjectId,
    operation_id: StudioOperationId,
    expected_fence: u64,
    asset: StudioAsset,
}

impl TempAssetCommitTicket {
    pub fn new(
        project_id: StudioProjectId,
        operation_id: StudioOperationId,
        expected_fence: u64,
        asset: StudioAsset,
    ) -> Result<Self, StudioError> {
        asset.validate()?;
        if expected_fence == 0 || asset.commit_state != AssetCommitState::Temporary {
            return Err(StudioError::InvalidAssetCommit);
        }
        Ok(Self {
            project_id,
            operation_id,
            expected_fence,
            asset,
        })
    }

    #[must_use]
    pub const fn project_id(&self) -> StudioProjectId {
        self.project_id
    }

    #[must_use]
    pub const fn operation_id(&self) -> StudioOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn expected_fence(&self) -> u64 {
        self.expected_fence
    }

    #[must_use]
    pub fn asset(&self) -> &StudioAsset {
        &self.asset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetCommitOutcome {
    Committed(StudioAsset),
    AlreadyCommitted(StudioAsset),
    AcknowledgementLost,
}

pub trait StudioOriginalStorePort {
    fn commit_temporary(
        &mut self,
        ticket: TempAssetCommitTicket,
    ) -> Result<AssetCommitOutcome, StudioError>;

    fn probe_original(
        &mut self,
        project_id: StudioProjectId,
        asset_id: StudioAssetId,
    ) -> Result<Option<StudioAsset>, StudioError>;

    fn delete_temporary(
        &mut self,
        project_id: StudioProjectId,
        asset_id: StudioAssetId,
        expected_checksum: AssetChecksum,
    ) -> Result<(), StudioError>;
}

/// Filesystem-backed immutable original store. Temporary bytes are verified
/// before staging, and commit uses same-filesystem rename plus a canonical
/// asset sidecar. Existing originals are never overwritten.
pub struct FilesystemStudioOriginalStore {
    root: PathBuf,
}

impl fmt::Debug for FilesystemStudioOriginalStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemStudioOriginalStore")
            .field("root", &"<redacted>")
            .finish()
    }
}

impl FilesystemStudioOriginalStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, StudioError> {
        Ok(Self {
            root: prepare_storage_root(root.as_ref())?,
        })
    }

    pub fn stage_temporary_bytes(
        &self,
        project_id: StudioProjectId,
        asset: &StudioAsset,
        bytes: &[u8],
    ) -> Result<(), StudioError> {
        asset.validate()?;
        if asset.commit_state != AssetCommitState::Temporary
            || asset.byte_len != bytes.len() as u64
            || asset.checksum != AssetChecksum::from_content(bytes)
        {
            return Err(StudioError::InvalidAssetCommit);
        }
        let directory = self.project_directory(project_id).join("temporary");
        fs::create_dir_all(&directory).map_err(|_| StudioError::StorageIo)?;
        let path = directory.join(format!("{}.media", opaque_id_hex(asset.id.0)));
        let _lock = acquire_storage_lock(
            &directory.join(format!("{}.stage.lock", opaque_id_hex(asset.id.0))),
        )?;
        if path.exists() {
            verify_asset_file(&path, asset.byte_len, asset.checksum)?;
            return Ok(());
        }
        atomic_replace_file(&path, bytes, asset.byte_len)
    }

    pub fn stage_legacy_copy(
        &self,
        source_root: impl AsRef<Path>,
        project_id: StudioProjectId,
        entry: &LegacyCopyPlanEntry,
        temporary_asset: &StudioAsset,
    ) -> Result<(), StudioError> {
        if entry.asset_id != temporary_asset.id
            || entry.track != temporary_asset.track
            || entry.destination != temporary_asset.source_name
            || entry.source.byte_len != temporary_asset.byte_len
            || entry.source.checksum != temporary_asset.checksum
            || temporary_asset.commit_state != AssetCommitState::Temporary
        {
            return Err(StudioError::InvalidAssetCommit);
        }
        let source_root = FilesystemLegacyCapProjectPort::open(source_root)?.root;
        let source = checked_legacy_path(&source_root, &entry.source.relative_path)?;
        verify_asset_file(&source, entry.source.byte_len, entry.source.checksum)?;
        let directory = self.project_directory(project_id).join("temporary");
        fs::create_dir_all(&directory).map_err(|_| StudioError::StorageIo)?;
        let destination = self.temporary_path(project_id, temporary_asset.id);
        let _lock = acquire_storage_lock(&directory.join(format!(
            "{}.stage.lock",
            opaque_id_hex(temporary_asset.id.0)
        )))?;
        if destination.exists() {
            return verify_asset_file(
                &destination,
                temporary_asset.byte_len,
                temporary_asset.checksum,
            );
        }
        let partial = directory.join(format!(
            ".{}.{}.copy-partial",
            opaque_id_hex(temporary_asset.id.0),
            std::process::id()
        ));
        let mut input = File::open(&source).map_err(|_| StudioError::StorageIo)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|_| StudioError::StorageIo)?;
        let mut hasher = Sha256::new();
        let mut observed = 0_u64;
        let mut buffer = vec![0_u8; MAX_STUDIO_PAYLOAD_CHUNK_BYTES];
        let result = (|| {
            loop {
                let count = input
                    .read(&mut buffer)
                    .map_err(|_| StudioError::StorageIo)?;
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|_| StudioError::StorageIo)?;
                hasher.update(&buffer[..count]);
                observed = observed
                    .checked_add(u64::try_from(count).map_err(|_| StudioError::DocumentTooLarge)?)
                    .ok_or(StudioError::DocumentTooLarge)?;
            }
            let digest: [u8; 32] = hasher.finalize().into();
            if observed != temporary_asset.byte_len
                || AssetChecksum::from_bytes(digest)? != temporary_asset.checksum
            {
                return Err(StudioError::AssetCommitMismatch);
            }
            output.sync_all().map_err(|_| StudioError::StorageIo)?;
            drop(output);
            fs::rename(&partial, &destination).map_err(|_| StudioError::StorageIo)?;
            sync_directory(&directory)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&partial);
        }
        result
    }

    fn project_directory(&self, project_id: StudioProjectId) -> PathBuf {
        self.root.join(opaque_id_hex(project_id.0))
    }

    fn temporary_path(&self, project_id: StudioProjectId, asset_id: StudioAssetId) -> PathBuf {
        self.project_directory(project_id)
            .join("temporary")
            .join(format!("{}.media", opaque_id_hex(asset_id.0)))
    }

    fn original_paths(
        &self,
        project_id: StudioProjectId,
        asset_id: StudioAssetId,
    ) -> (PathBuf, PathBuf) {
        let directory = self.project_directory(project_id).join("originals");
        let stem = opaque_id_hex(asset_id.0);
        (
            directory.join(format!("{stem}.media")),
            directory.join(format!("{stem}.asset")),
        )
    }
}

impl StudioOriginalStorePort for FilesystemStudioOriginalStore {
    fn commit_temporary(
        &mut self,
        ticket: TempAssetCommitTicket,
    ) -> Result<AssetCommitOutcome, StudioError> {
        let expected = ticket.asset.clone();
        let mut durable = expected.clone();
        durable.commit_state = AssetCommitState::DurableOriginal;
        if let Some(existing) = self.probe_original(ticket.project_id, expected.id)? {
            return if existing == durable {
                Ok(AssetCommitOutcome::AlreadyCommitted(existing))
            } else {
                Err(StudioError::AssetConflict)
            };
        }
        let project_directory = self.project_directory(ticket.project_id);
        fs::create_dir_all(project_directory.join("temporary"))
            .and_then(|()| fs::create_dir_all(project_directory.join("originals")))
            .map_err(|_| StudioError::StorageIo)?;
        let lock_path =
            project_directory.join(format!("{}.original.lock", opaque_id_hex(expected.id.0)));
        let _lock = acquire_storage_lock(&lock_path)?;
        if let Some(existing) = self.probe_original(ticket.project_id, expected.id)? {
            return if existing == durable {
                Ok(AssetCommitOutcome::AlreadyCommitted(existing))
            } else {
                Err(StudioError::AssetConflict)
            };
        }
        let temporary = self.temporary_path(ticket.project_id, expected.id);
        let (original, metadata) = self.original_paths(ticket.project_id, expected.id);
        if original.exists() {
            verify_asset_file(&original, expected.byte_len, expected.checksum)?;
            if temporary.exists() {
                verify_asset_file(&temporary, expected.byte_len, expected.checksum)?;
                fs::remove_file(&temporary).map_err(|_| StudioError::StorageIo)?;
            }
        } else {
            verify_asset_file(&temporary, expected.byte_len, expected.checksum)?;
            fs::rename(&temporary, &original).map_err(|_| StudioError::StorageIo)?;
            sync_directory(original.parent().ok_or(StudioError::UnsafeStoragePath)?)?;
        }
        let encoded = StudioDocumentCodec::encode_asset(&durable)?;
        atomic_replace_file(&metadata, &encoded, expected.byte_len)?;
        Ok(AssetCommitOutcome::Committed(durable))
    }

    fn probe_original(
        &mut self,
        project_id: StudioProjectId,
        asset_id: StudioAssetId,
    ) -> Result<Option<StudioAsset>, StudioError> {
        let (media, metadata) = self.original_paths(project_id, asset_id);
        let Some(bytes) = read_bounded_file(&metadata, MAX_STUDIO_DOCUMENT_BYTES)? else {
            if media.exists() {
                return Ok(None);
            }
            return Ok(None);
        };
        let asset = StudioDocumentCodec::decode_asset(&bytes)?;
        if asset.id != asset_id || asset.commit_state != AssetCommitState::DurableOriginal {
            return Err(StudioError::AssetConflict);
        }
        verify_asset_file(&media, asset.byte_len, asset.checksum)?;
        Ok(Some(asset))
    }

    fn delete_temporary(
        &mut self,
        project_id: StudioProjectId,
        asset_id: StudioAssetId,
        expected_checksum: AssetChecksum,
    ) -> Result<(), StudioError> {
        let path = self.temporary_path(project_id, asset_id);
        if !path.exists() {
            return Ok(());
        }
        let metadata = path.metadata().map_err(|_| StudioError::StorageIo)?;
        verify_asset_file(&path, metadata.len(), expected_checksum)?;
        fs::remove_file(&path).map_err(|_| StudioError::StorageIo)?;
        sync_directory(path.parent().ok_or(StudioError::UnsafeStoragePath)?)
    }
}

/// Production sink for the four pre-encoded branches in a validated recording
/// graph. Native capture bridges feed bounded chunks; each track is written,
/// hashed, synced, and sealed into the original store's temporary namespace
/// independently so no mixed/flattened master can replace the originals.
pub struct FilesystemStudioRecordingSession {
    root: PathBuf,
    graph: StudioRecordingGraphSpec,
    tracks: BTreeMap<TrackKind, FilesystemRecordingTrack>,
    maximum_track_bytes: u64,
    finished: bool,
}

struct FilesystemRecordingTrack {
    file: Option<File>,
    partial_path: PathBuf,
    temporary_path: PathBuf,
    bytes: u64,
    hasher: Sha256,
}

impl fmt::Debug for FilesystemStudioRecordingSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemStudioRecordingSession")
            .field("root", &"<redacted>")
            .field("graph", &self.graph)
            .field("track_count", &self.tracks.len())
            .field("maximum_track_bytes", &self.maximum_track_bytes)
            .field("finished", &self.finished)
            .finish()
    }
}

impl FilesystemStudioRecordingSession {
    pub fn begin(
        store: &FilesystemStudioOriginalStore,
        graph: StudioRecordingGraphSpec,
        maximum_track_bytes: u64,
    ) -> Result<Self, StudioError> {
        graph.validate()?;
        if maximum_track_bytes == 0 {
            return Err(StudioError::InvalidRecordingGraph);
        }
        let project = store.project_directory(graph.project_id);
        let partials = project.join("recording-partials");
        let temporary = project.join("temporary");
        fs::create_dir_all(&partials)
            .and_then(|()| fs::create_dir_all(&temporary))
            .map_err(|_| StudioError::StorageIo)?;
        persist_recording_graph(&project, &graph, maximum_track_bytes)?;
        let mut tracks = BTreeMap::new();
        let initialize = (|| {
            for branch in &graph.branches {
                let stem = opaque_id_hex(branch.asset_id.0);
                let partial_path = partials.join(format!("{stem}.recording-partial"));
                let temporary_path = temporary.join(format!("{stem}.media"));
                if partial_path.exists() || temporary_path.exists() {
                    return Err(StudioError::AssetConflict);
                }
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&partial_path)
                    .map_err(|_| StudioError::StorageIo)?;
                tracks.insert(
                    branch.track,
                    FilesystemRecordingTrack {
                        file: Some(file),
                        partial_path,
                        temporary_path,
                        bytes: 0,
                        hasher: Sha256::new(),
                    },
                );
            }
            Ok(())
        })();
        if let Err(error) = initialize {
            for track in tracks.values_mut() {
                track.file.take();
                let _ = fs::remove_file(&track.partial_path);
            }
            return Err(error);
        }
        sync_directory(&partials)?;
        sync_directory(&temporary)?;
        Ok(Self {
            root: store.root.clone(),
            graph,
            tracks,
            maximum_track_bytes,
            finished: false,
        })
    }

    /// Reopens the exact isolated-track sinks after a process or power loss.
    ///
    /// A crash may leave every track as a recording partial, or may occur while
    /// `finish` is sealing tracks and therefore leave a mixture of partial and
    /// already-sealed temporary files. The caller must present the original
    /// graph identity. Existing bytes are re-read through SHA-256 before a
    /// partial is opened for append; an already-sealed track remains immutable.
    /// Missing partials are recreated so a crash during graph preparation can
    /// resume without flattening or substituting another track.
    pub fn recover(
        store: &FilesystemStudioOriginalStore,
        graph: StudioRecordingGraphSpec,
        maximum_track_bytes: u64,
    ) -> Result<Self, StudioError> {
        graph.validate()?;
        if maximum_track_bytes == 0 {
            return Err(StudioError::InvalidRecordingGraph);
        }
        let project = store.project_directory(graph.project_id);
        let partials = project.join("recording-partials");
        let temporary = project.join("temporary");
        fs::create_dir_all(&partials)
            .and_then(|()| fs::create_dir_all(&temporary))
            .map_err(|_| StudioError::StorageIo)?;
        verify_recording_graph(&project, &graph, maximum_track_bytes)?;
        let mut tracks = BTreeMap::new();
        for branch in &graph.branches {
            let stem = opaque_id_hex(branch.asset_id.0);
            let partial_path = partials.join(format!("{stem}.recording-partial"));
            let temporary_path = temporary.join(format!("{stem}.media"));
            let partial_exists = partial_path.exists();
            let temporary_exists = temporary_path.exists();
            if partial_exists && temporary_exists {
                return Err(StudioError::AssetConflict);
            }
            let (file, bytes, hasher) = if temporary_exists {
                let (bytes, hasher) = read_recording_state(&temporary_path, maximum_track_bytes)?;
                (None, bytes, hasher)
            } else {
                if !partial_exists {
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&partial_path)
                        .map_err(|_| StudioError::StorageIo)?;
                }
                let (bytes, hasher) = read_recording_state(&partial_path, maximum_track_bytes)?;
                let file = OpenOptions::new()
                    .append(true)
                    .open(&partial_path)
                    .map_err(|_| StudioError::StorageIo)?;
                (Some(file), bytes, hasher)
            };
            tracks.insert(
                branch.track,
                FilesystemRecordingTrack {
                    file,
                    partial_path,
                    temporary_path,
                    bytes,
                    hasher,
                },
            );
        }
        sync_directory(&partials)?;
        sync_directory(&temporary)?;
        Ok(Self {
            root: store.root.clone(),
            graph,
            tracks,
            maximum_track_bytes,
            finished: false,
        })
    }

    pub fn write_encoded_chunk(
        &mut self,
        track: TrackKind,
        bytes: &[u8],
    ) -> Result<(), StudioError> {
        if self.finished || bytes.is_empty() || bytes.len() > MAX_STUDIO_PAYLOAD_CHUNK_BYTES {
            return Err(StudioError::InvalidPayloadChunk);
        }
        let target = self
            .tracks
            .get_mut(&track)
            .ok_or(StudioError::InvalidRecordingGraph)?;
        let next = target
            .bytes
            .checked_add(u64::try_from(bytes.len()).map_err(|_| StudioError::DocumentTooLarge)?)
            .ok_or(StudioError::DocumentTooLarge)?;
        if next > self.maximum_track_bytes {
            return Err(StudioError::DocumentTooLarge);
        }
        target
            .file
            .as_mut()
            .ok_or(StudioError::InvalidRecordingGraph)?
            .write_all(bytes)
            .map_err(|_| StudioError::StorageIo)?;
        target.hasher.update(bytes);
        target.bytes = next;
        Ok(())
    }

    pub fn finish(
        mut self,
        start: RationalTime,
        duration: RationalTime,
    ) -> Result<Vec<StudioAsset>, StudioError> {
        if duration.ticks == 0 || self.tracks.values().any(|track| track.bytes == 0) {
            return Err(StudioError::InvalidAsset);
        }
        let mut assets = Vec::with_capacity(self.graph.branches.len());
        for branch in &self.graph.branches {
            let track = self
                .tracks
                .get_mut(&branch.track)
                .ok_or(StudioError::InvalidRecordingGraph)?;
            if let Some(file) = track.file.take() {
                file.sync_all().map_err(|_| StudioError::StorageIo)?;
                drop(file);
                fs::rename(&track.partial_path, &track.temporary_path)
                    .map_err(|_| StudioError::StorageIo)?;
            }
            sync_directory(
                track
                    .temporary_path
                    .parent()
                    .ok_or(StudioError::UnsafeStoragePath)?,
            )?;
            let digest: [u8; 32] = track.hasher.clone().finalize().into();
            let checksum = AssetChecksum::from_bytes(digest)?;
            verify_asset_file(&track.temporary_path, track.bytes, checksum)?;
            assets.push(StudioAsset {
                version: STUDIO_ASSET_VERSION,
                id: branch.asset_id,
                track: branch.track,
                source_name: branch.temporary_name.clone(),
                byte_len: track.bytes,
                start,
                duration,
                checksum,
                commit_state: AssetCommitState::Temporary,
            });
        }
        self.finished = true;
        Ok(assets)
    }
}

impl Drop for FilesystemStudioRecordingSession {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        for track in self.tracks.values_mut() {
            if let Some(file) = track.file.take() {
                let _ = file.sync_all();
            }
        }
        // An ordinary process unwind must be as recoverable as a hard crash.
        // Partial and already-sealed temporary tracks are deliberately retained
        // until a journal-authorized recovery or cleanup decision is made.
        let _ = sync_directory(&self.root);
    }
}

fn read_recording_state(
    path: &Path,
    maximum_track_bytes: u64,
) -> Result<(u64, Sha256), StudioError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| StudioError::StorageIo)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > maximum_track_bytes
    {
        return Err(StudioError::AssetConflict);
    }
    let mut file = File::open(path).map_err(|_| StudioError::StorageIo)?;
    let mut hasher = Sha256::new();
    let mut observed = 0_u64;
    let mut buffer = vec![0_u8; MAX_STUDIO_PAYLOAD_CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer).map_err(|_| StudioError::StorageIo)?;
        if count == 0 {
            break;
        }
        observed = observed
            .checked_add(u64::try_from(count).map_err(|_| StudioError::DocumentTooLarge)?)
            .ok_or(StudioError::DocumentTooLarge)?;
        if observed > maximum_track_bytes {
            return Err(StudioError::DocumentTooLarge);
        }
        hasher.update(&buffer[..count]);
    }
    if observed != metadata.len() {
        return Err(StudioError::AssetCommitMismatch);
    }
    Ok((observed, hasher))
}

fn recording_graph_path(project: &Path, clock_id: StudioOperationId) -> PathBuf {
    project.join(format!(
        "{}.studio-recording-graph",
        opaque_id_hex(clock_id.0)
    ))
}

fn encode_recording_graph(
    graph: &StudioRecordingGraphSpec,
    maximum_track_bytes: u64,
) -> Result<Vec<u8>, StudioError> {
    graph.validate()?;
    if maximum_track_bytes == 0 {
        return Err(StudioError::InvalidRecordingGraph);
    }
    let mut branches = graph.branches.iter().collect::<Vec<_>>();
    branches.sort_by_key(|branch| branch.track);
    let mut writer = CanonicalWriter::new();
    writer.id(graph.project_id.canonical_bytes());
    writer.id(graph.clock_id.canonical_bytes());
    writer.u64(maximum_track_bytes);
    writer.u8(u8::try_from(branches.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for branch in branches {
        writer.u8(branch.track.tag());
        writer.id(branch.asset_id.canonical_bytes());
        writer.string(branch.temporary_name.as_str())?;
        writer.u8(branch.source.tag());
        writer.u8(branch.encoder.tag());
        writer.u8(branch.muxer.tag());
        writer.u32(branch.time_base.0);
        writer.u32(branch.queue.max_buffers);
        writer.u64(branch.queue.max_bytes);
        writer.u64(branch.queue.max_time_ns);
    }
    wrap_document(DocumentKind::RecordingGraph, 1, writer.finish()?)
}

fn persist_recording_graph(
    project: &Path,
    graph: &StudioRecordingGraphSpec,
    maximum_track_bytes: u64,
) -> Result<(), StudioError> {
    let path = recording_graph_path(project, graph.clock_id);
    let expected = encode_recording_graph(graph, maximum_track_bytes)?;
    if let Some(existing) = read_bounded_file(&path, MAX_STUDIO_DOCUMENT_BYTES)? {
        return if existing == expected {
            Ok(())
        } else {
            Err(StudioError::JournalCorrupt)
        };
    }
    atomic_replace_file(&path, &expected, maximum_track_bytes)?;
    sync_directory(project)
}

fn verify_recording_graph(
    project: &Path,
    graph: &StudioRecordingGraphSpec,
    maximum_track_bytes: u64,
) -> Result<(), StudioError> {
    let path = recording_graph_path(project, graph.clock_id);
    let existing =
        read_bounded_file(&path, MAX_STUDIO_DOCUMENT_BYTES)?.ok_or(StudioError::JournalCorrupt)?;
    let expected = encode_recording_graph(graph, maximum_track_bytes)?;
    if existing != expected {
        return Err(StudioError::JournalCorrupt);
    }
    // Re-run the envelope verifier so a coincidental byte comparison cannot
    // bypass the declared kind, version, or checksum contract.
    let (version, _) = unwrap_document(&existing, DocumentKind::RecordingGraph)?;
    if version != 1 {
        return Err(StudioError::JournalCorrupt);
    }
    Ok(())
}

pub fn commit_verified_temporary<S: StudioOriginalStorePort>(
    store: &mut S,
    ticket: TempAssetCommitTicket,
) -> Result<StudioAsset, StudioError> {
    let project_id = ticket.project_id;
    let expected = ticket.asset.clone();
    let asset_id = expected.id;
    let mut durable_expected = expected.clone();
    durable_expected.commit_state = AssetCommitState::DurableOriginal;
    let committed = match store.commit_temporary(ticket)? {
        AssetCommitOutcome::Committed(asset) | AssetCommitOutcome::AlreadyCommitted(asset) => asset,
        AssetCommitOutcome::AcknowledgementLost => store
            .probe_original(project_id, asset_id)?
            .ok_or(StudioError::AmbiguousAssetCommit)?,
    };
    if committed != durable_expected {
        return Err(StudioError::AssetCommitMismatch);
    }
    committed.validate()?;
    Ok(committed)
}

/// Non-cloneable authorization for one compare-and-swap edit save. The ticket
/// carries the complete next manifest so an adapter never reconstructs or
/// mutates original asset records while saving editor state.
#[derive(Debug)]
pub struct EditSaveTicket {
    operation_id: StudioOperationId,
    expected_fence: u64,
    expected_project_revision: u64,
    next_project: StudioProjectManifest,
}

impl EditSaveTicket {
    pub fn new(
        current: &StudioProjectManifest,
        operation_id: StudioOperationId,
        expected_fence: u64,
        edits: EditSpec,
    ) -> Result<Self, StudioError> {
        current.validate()?;
        if current.state != StudioState::Editing || expected_fence == 0 {
            return Err(StudioError::InvalidEditSave);
        }
        let next_revision = current
            .revision
            .checked_add(1)
            .ok_or(StudioError::InvalidEditSave)?;
        if edits.revision != next_revision {
            return Err(StudioError::InvalidEditSave);
        }
        let mut next_project = current.clone();
        next_project.revision = next_revision;
        next_project.edits = edits;
        next_project.validate()?;
        Ok(Self {
            operation_id,
            expected_fence,
            expected_project_revision: current.revision,
            next_project,
        })
    }

    #[must_use]
    pub const fn operation_id(&self) -> StudioOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn expected_fence(&self) -> u64 {
        self.expected_fence
    }

    #[must_use]
    pub const fn expected_project_revision(&self) -> u64 {
        self.expected_project_revision
    }

    #[must_use]
    pub fn next_project(&self) -> &StudioProjectManifest {
        &self.next_project
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditSaveOutcome {
    Committed(StudioProjectManifest),
    AlreadyCommitted(StudioProjectManifest),
    AcknowledgementLost,
}

pub trait StudioProjectStorePort {
    fn save_edits(&mut self, ticket: EditSaveTicket) -> Result<EditSaveOutcome, StudioError>;

    fn probe_project(
        &mut self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioProjectManifest>, StudioError>;
}

/// Fenced canonical project-manifest store. The instance is opened for one
/// journal fence; stale edit tickets cannot be persisted by a newer owner.
pub struct FilesystemStudioProjectStore {
    root: PathBuf,
    fence: u64,
}

impl fmt::Debug for FilesystemStudioProjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemStudioProjectStore")
            .field("root", &"<redacted>")
            .field("fence", &self.fence)
            .finish()
    }
}

impl FilesystemStudioProjectStore {
    pub fn new(root: impl AsRef<Path>, fence: u64) -> Result<Self, StudioError> {
        if fence == 0 {
            return Err(StudioError::StaleJournal);
        }
        Ok(Self {
            root: prepare_storage_root(root.as_ref())?,
            fence,
        })
    }

    pub fn create_project(&mut self, project: &StudioProjectManifest) -> Result<(), StudioError> {
        project.validate()?;
        let path = self.project_path(project.id);
        let lock_path = path.with_extension("project.lock");
        let _lock = acquire_storage_lock(&lock_path)?;
        self.claim_fence(project.id)?;
        if let Some(existing) = self.read_project(project.id)? {
            return if existing == *project {
                Ok(())
            } else {
                Err(StudioError::EditSaveMismatch)
            };
        }
        let bytes = StudioDocumentCodec::encode_project(project)?;
        atomic_replace_file(&path, &bytes, project.revision)
    }

    fn project_path(&self, project_id: StudioProjectId) -> PathBuf {
        self.root
            .join(format!("{}.studio-project", opaque_id_hex(project_id.0)))
    }

    fn fence_path(&self, project_id: StudioProjectId) -> PathBuf {
        self.root.join(format!(
            "{}.studio-project-fence",
            opaque_id_hex(project_id.0)
        ))
    }

    fn claim_fence(&self, project_id: StudioProjectId) -> Result<(), StudioError> {
        let path = self.fence_path(project_id);
        if let Some(bytes) = read_bounded_file(&path, MAX_STUDIO_DOCUMENT_BYTES)? {
            let current = decode_project_fence(&bytes, project_id)?;
            if current > self.fence {
                return Err(StudioError::StaleJournal);
            }
            if current == self.fence {
                return Ok(());
            }
        }
        let bytes = encode_project_fence(project_id, self.fence)?;
        atomic_replace_file(&path, &bytes, self.fence)
    }

    fn read_project(
        &self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioProjectManifest>, StudioError> {
        let Some(bytes) =
            read_bounded_file(&self.project_path(project_id), MAX_STUDIO_DOCUMENT_BYTES)?
        else {
            return Ok(None);
        };
        let project = StudioDocumentCodec::decode_project(&bytes)?;
        if project.id != project_id {
            return Err(StudioError::EditSaveMismatch);
        }
        Ok(Some(project))
    }
}

impl StudioProjectStorePort for FilesystemStudioProjectStore {
    fn save_edits(&mut self, ticket: EditSaveTicket) -> Result<EditSaveOutcome, StudioError> {
        if ticket.expected_fence != self.fence {
            return Err(StudioError::StaleJournal);
        }
        let next = ticket.next_project.clone();
        let path = self.project_path(next.id);
        let lock_path = path.with_extension("project.lock");
        let _lock = acquire_storage_lock(&lock_path)?;
        self.claim_fence(next.id)?;
        let current = self
            .read_project(next.id)?
            .ok_or(StudioError::EditSaveMismatch)?;
        if current == next {
            return Ok(EditSaveOutcome::AlreadyCommitted(current));
        }
        if current.revision != ticket.expected_project_revision
            || next.revision != current.revision.saturating_add(1)
            || current.assets != next.assets
        {
            return Err(StudioError::EditSaveMismatch);
        }
        let bytes = StudioDocumentCodec::encode_project(&next)?;
        atomic_replace_file(&path, &bytes, next.revision)?;
        Ok(EditSaveOutcome::Committed(next))
    }

    fn probe_project(
        &mut self,
        project_id: StudioProjectId,
    ) -> Result<Option<StudioProjectManifest>, StudioError> {
        self.read_project(project_id)
    }
}

fn encode_project_fence(project_id: StudioProjectId, fence: u64) -> Result<Vec<u8>, StudioError> {
    if fence == 0 {
        return Err(StudioError::StaleJournal);
    }
    let mut writer = CanonicalWriter::new();
    writer.id(project_id.canonical_bytes());
    writer.u64(fence);
    wrap_document(DocumentKind::ProjectFence, 1, writer.finish()?)
}

fn decode_project_fence(
    bytes: &[u8],
    expected_project: StudioProjectId,
) -> Result<u64, StudioError> {
    let (version, payload) = unwrap_document(bytes, DocumentKind::ProjectFence)?;
    if version != 1 {
        return Err(StudioError::CorruptDocument);
    }
    let mut reader = CanonicalReader::new(payload);
    let project = StudioProjectId::from_csprng(reader.array_16()?)?;
    let fence = reader.u64()?;
    reader.finish()?;
    if project != expected_project || fence == 0 {
        return Err(StudioError::CorruptDocument);
    }
    Ok(fence)
}

fn acquire_storage_lock(path: &Path) -> Result<FilesystemStudioLock, StudioError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| StudioError::StorageIo)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StudioError::StaleJournal
            } else {
                StudioError::StorageIo
            }
        })?;
    file.write_all(b"frame-studio-storage-lock-v1\n")
        .and_then(|()| file.sync_all())
        .map_err(|_| StudioError::StorageIo)?;
    Ok(FilesystemStudioLock {
        path: path.to_path_buf(),
    })
}

fn verify_asset_file(
    path: &Path,
    expected_bytes: u64,
    expected_checksum: AssetChecksum,
) -> Result<(), StudioError> {
    let mut file = File::open(path).map_err(|_| StudioError::InvalidAssetCommit)?;
    let metadata = file
        .metadata()
        .map_err(|_| StudioError::InvalidAssetCommit)?;
    if !metadata.is_file() || metadata.len() != expected_bytes {
        return Err(StudioError::AssetCommitMismatch);
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; MAX_STUDIO_PAYLOAD_CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer).map_err(|_| StudioError::StorageIo)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if AssetChecksum::from_bytes(digest)? != expected_checksum {
        return Err(StudioError::AssetCommitMismatch);
    }
    Ok(())
}

pub fn commit_edit_save<S: StudioProjectStorePort>(
    store: &mut S,
    ticket: EditSaveTicket,
) -> Result<StudioProjectManifest, StudioError> {
    let project_id = ticket.next_project.id;
    let expected = ticket.next_project.clone();
    let committed = match store.save_edits(ticket)? {
        EditSaveOutcome::Committed(project) | EditSaveOutcome::AlreadyCommitted(project) => project,
        EditSaveOutcome::AcknowledgementLost => store
            .probe_project(project_id)?
            .ok_or(StudioError::AmbiguousEditSave)?,
    };
    committed.validate()?;
    if committed != expected {
        return Err(StudioError::EditSaveMismatch);
    }
    Ok(committed)
}

// ---------------------------------------------------------------------------
// Deterministic rational timeline, preview, and seek

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceCoverage {
    pub track: TrackKind,
    pub start: RationalTime,
    pub end: RationalTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineSource {
    pub duration: RationalTime,
    pub coverage: Vec<SourceCoverage>,
    pub vfr_video_pts: BTreeMap<TrackKind, Vec<RationalTime>>,
}

impl TimelineSource {
    pub fn validate(&self) -> Result<(), StudioError> {
        if self.duration.ticks == 0 {
            return Err(StudioError::NoTimeline);
        }
        if self.coverage.len() > MAX_STUDIO_COVERAGE_RANGES {
            return Err(StudioError::DocumentTooLarge);
        }
        let zero = RationalTime::new(0, self.duration.time_base);
        for pair in self.coverage.windows(2) {
            if pair[0].track > pair[1].track
                || (pair[0].track == pair[1].track
                    && compare_times(pair[0].start, pair[1].start) == std::cmp::Ordering::Greater)
            {
                return Err(StudioError::InvalidCoverage);
            }
        }
        let mut previous_by_track: BTreeMap<TrackKind, RationalTime> = BTreeMap::new();
        for range in &self.coverage {
            if compare_times(range.start, zero) == std::cmp::Ordering::Less
                || compare_times(range.start, range.end) != std::cmp::Ordering::Less
                || compare_times(range.end, self.duration) == std::cmp::Ordering::Greater
            {
                return Err(StudioError::InvalidCoverage);
            }
            if let Some(previous_end) = previous_by_track.insert(range.track, range.end)
                && compare_times(range.start, previous_end) == std::cmp::Ordering::Less
            {
                return Err(StudioError::OverlappingCoverage);
            }
        }
        let mut total_samples = 0_usize;
        for (track, points) in &self.vfr_video_pts {
            if !matches!(track, TrackKind::Screen | TrackKind::Camera) {
                return Err(StudioError::InvalidVfrSamples);
            }
            total_samples = total_samples
                .checked_add(points.len())
                .ok_or(StudioError::DocumentTooLarge)?;
            if total_samples > MAX_STUDIO_VFR_SAMPLES {
                return Err(StudioError::DocumentTooLarge);
            }
            let mut previous = None;
            for point in points {
                if compare_times(*point, self.duration) != std::cmp::Ordering::Less
                    || previous.is_some_and(|prior| {
                        compare_times(prior, *point) != std::cmp::Ordering::Less
                    })
                {
                    return Err(StudioError::InvalidVfrSamples);
                }
                previous = Some(*point);
            }
            if !vfr_points_are_covered(&self.coverage, *track, points) {
                return Err(StudioError::InvalidVfrSamples);
            }
        }
        Ok(())
    }
}

fn digest_timeline_source(source: &TimelineSource) -> Result<Sha256Digest, StudioError> {
    source.validate()?;
    let mut writer = CanonicalWriter::new();
    encode_time(&mut writer, source.duration);
    writer.u32(u32::try_from(source.coverage.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for range in &source.coverage {
        writer.u8(range.track.tag());
        encode_time(&mut writer, range.start);
        encode_time(&mut writer, range.end);
    }
    writer.u8(u8::try_from(source.vfr_video_pts.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for (track, points) in &source.vfr_video_pts {
        writer.u8(track.tag());
        writer.u32(u32::try_from(points.len()).map_err(|_| StudioError::DocumentTooLarge)?);
        for point in points {
            encode_time(&mut writer, *point);
        }
    }
    Ok(strong_sha256(&writer.finish()?))
}

fn digest_edit_spec(edit: &EditSpec) -> Result<Sha256Digest, StudioError> {
    validate_edit_shape(edit)?;
    let mut writer = CanonicalWriter::new();
    encode_edit(&mut writer, edit)?;
    Ok(strong_sha256(&writer.finish()?))
}

fn vfr_points_are_covered(
    coverage: &[SourceCoverage],
    track: TrackKind,
    points: &[RationalTime],
) -> bool {
    let ranges = coverage
        .iter()
        .filter(|range| range.track == track)
        .collect::<Vec<_>>();
    let mut range_index = 0_usize;
    for point in points {
        while ranges
            .get(range_index)
            .is_some_and(|range| compare_times(range.end, *point) != std::cmp::Ordering::Greater)
        {
            range_index += 1;
        }
        let Some(range) = ranges.get(range_index) else {
            return false;
        };
        if compare_times(range.start, *point) == std::cmp::Ordering::Greater
            || compare_times(*point, range.end) != std::cmp::Ordering::Less
        {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraStyle {
    pub rect: NormalizedRect,
    pub corner_radius_milli: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorStyle {
    pub scale_milli: u16,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioStyle {
    pub gain_millibels: i32,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositeStyle {
    pub layout: LayoutPreset,
    pub camera: CameraStyle,
    pub cursor: CursorStyle,
    pub background: BackgroundStyle,
    pub microphone: AudioStyle,
    pub system_audio: AudioStyle,
}

impl Default for CompositeStyle {
    fn default() -> Self {
        Self {
            layout: LayoutPreset::CameraBubble,
            camera: CameraStyle {
                rect: NormalizedRect {
                    x_millionths: 740_000,
                    y_millionths: 680_000,
                    width_millionths: 240_000,
                    height_millionths: 300_000,
                },
                corner_radius_milli: 500,
            },
            cursor: CursorStyle {
                scale_milli: 1_000,
                hidden: false,
            },
            background: BackgroundStyle::Transparent,
            microphone: AudioStyle {
                gain_millibels: 0,
                muted: false,
            },
            system_audio: AudioStyle {
                gain_millibels: 0,
                muted: false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTimelineSpan {
    pub source_start: RationalTime,
    pub source_end: RationalTime,
    pub output_start: ExactDuration,
    pub output_end: ExactDuration,
    pub speed_numerator: u32,
    pub speed_denominator: u32,
    pub style: CompositeStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapDisposition {
    InsertSilence,
    HideCamera,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapInstruction {
    pub track: TrackKind,
    pub source_start: RationalTime,
    pub source_end: RationalTime,
    pub disposition: GapDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEditPlan {
    pub version: u16,
    pub edit_revision: u64,
    pub source_duration: RationalTime,
    pub output_duration: ExactDuration,
    pub spans: Vec<CompiledTimelineSpan>,
    pub gaps: Vec<GapInstruction>,
    pub vfr_video_pts: BTreeMap<TrackKind, Vec<RationalTime>>,
    source_topology_digest: Sha256Digest,
    edit_spec_digest: Sha256Digest,
    digest: Sha256Digest,
}

impl CanonicalEditPlan {
    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    #[must_use]
    pub const fn source_topology_digest(&self) -> Sha256Digest {
        self.source_topology_digest
    }

    #[must_use]
    pub const fn edit_spec_digest(&self) -> Sha256Digest {
        self.edit_spec_digest
    }

    pub fn validate(&self) -> Result<(), StudioError> {
        if self.version != STUDIO_EDIT_VERSION
            || self.source_duration.ticks == 0
            || self.spans.is_empty()
            || self.spans.len() > MAX_STUDIO_EDITS.saturating_mul(2).saturating_add(1)
            || self.gaps.len() > MAX_STUDIO_GAP_INSTRUCTIONS
            || self.output_duration.numerator == 0
        {
            return Err(StudioError::InvalidCompiledPlan);
        }
        let mut output_cursor = ExactDuration::zero();
        let mut prior_source_end = None;
        for span in &self.spans {
            if compare_times(span.source_start, span.source_end) != std::cmp::Ordering::Less
                || compare_times(span.source_end, self.source_duration)
                    == std::cmp::Ordering::Greater
                || prior_source_end.is_some_and(|prior| {
                    compare_times(span.source_start, prior) == std::cmp::Ordering::Less
                })
                || span.output_start != output_cursor
                || span.speed_numerator == 0
                || span.speed_denominator == 0
                || u64::from(span.speed_numerator) > u64::from(span.speed_denominator) * 4
                || u64::from(span.speed_denominator) > u64::from(span.speed_numerator) * 4
            {
                return Err(StudioError::InvalidCompiledPlan);
            }
            validate_composite_style(span.style)?;
            let expected_duration = span
                .source_end
                .checked_sub(span.source_start)?
                .scaled(span.speed_denominator, span.speed_numerator)?;
            if duration_sub(span.output_end, span.output_start)? != expected_duration {
                return Err(StudioError::InvalidCompiledPlan);
            }
            output_cursor = span.output_end;
            prior_source_end = Some(span.source_end);
        }
        if output_cursor != self.output_duration {
            return Err(StudioError::InvalidCompiledPlan);
        }
        for gap in &self.gaps {
            let disposition_valid = matches!(
                (gap.track, gap.disposition),
                (
                    TrackKind::Microphone | TrackKind::SystemAudio,
                    GapDisposition::InsertSilence
                ) | (TrackKind::Camera, GapDisposition::HideCamera)
            );
            if !disposition_valid
                || compare_times(gap.source_start, gap.source_end) != std::cmp::Ordering::Less
                || compare_times(gap.source_end, self.source_duration)
                    == std::cmp::Ordering::Greater
            {
                return Err(StudioError::InvalidCompiledPlan);
            }
        }
        let mut total_vfr = 0_usize;
        for (track, points) in &self.vfr_video_pts {
            if !matches!(track, TrackKind::Screen | TrackKind::Camera) {
                return Err(StudioError::InvalidCompiledPlan);
            }
            total_vfr = total_vfr
                .checked_add(points.len())
                .ok_or(StudioError::DocumentTooLarge)?;
            if total_vfr > MAX_STUDIO_VFR_SAMPLES
                || points
                    .windows(2)
                    .any(|pair| compare_times(pair[0], pair[1]) != std::cmp::Ordering::Less)
                || points.iter().any(|point| {
                    compare_times(*point, self.source_duration) != std::cmp::Ordering::Less
                })
            {
                return Err(StudioError::InvalidCompiledPlan);
            }
        }
        let expected_digest = digest_edit_plan(
            self.edit_revision,
            self.source_duration,
            self.output_duration,
            &self.spans,
            &self.gaps,
            &self.vfr_video_pts,
            EditPlanBindings {
                source_topology_digest: self.source_topology_digest,
                edit_spec_digest: self.edit_spec_digest,
            },
        )?;
        if expected_digest != self.digest {
            return Err(StudioError::CorruptCompiledPlan);
        }
        Ok(())
    }

    pub fn seek(&self, output: ExactDuration) -> Result<ExactSourcePosition, StudioError> {
        self.validate()?;
        if compare_duration(output, self.output_duration) != std::cmp::Ordering::Less {
            return Err(StudioError::SeekOutsideTimeline);
        }
        let span = self
            .spans
            .iter()
            .find(|span| {
                compare_duration(output, span.output_start) != std::cmp::Ordering::Less
                    && compare_duration(output, span.output_end) == std::cmp::Ordering::Less
            })
            .ok_or(StudioError::SeekOutsideTimeline)?;
        let offset = duration_sub(output, span.output_start)?
            .scaled(span.speed_numerator, span.speed_denominator)?;
        Ok(ExactSourcePosition {
            span_source_start: span.source_start,
            offset,
        })
    }

    pub fn map_source_point(
        &self,
        source: RationalTime,
    ) -> Result<Option<ExactDuration>, StudioError> {
        self.validate()?;
        self.map_source_point_validated(source)
    }

    fn map_source_point_validated(
        &self,
        source: RationalTime,
    ) -> Result<Option<ExactDuration>, StudioError> {
        for span in &self.spans {
            if compare_times(source, span.source_start) != std::cmp::Ordering::Less
                && compare_times(source, span.source_end) == std::cmp::Ordering::Less
            {
                let offset = source
                    .checked_sub(span.source_start)?
                    .scaled(span.speed_denominator, span.speed_numerator)?;
                return Ok(Some(span.output_start.checked_add(offset)?));
            }
        }
        Ok(None)
    }

    pub fn mapped_vfr_points(&self, track: TrackKind) -> Result<Vec<ExactDuration>, StudioError> {
        self.validate()?;
        let points = self
            .vfr_video_pts
            .get(&track)
            .map_or(&[][..], Vec::as_slice);
        let mut mapped = Vec::with_capacity(points.len());
        let mut span_index = 0_usize;
        for point in points {
            while self.spans.get(span_index).is_some_and(|span| {
                compare_times(span.source_end, *point) != std::cmp::Ordering::Greater
            }) {
                span_index += 1;
            }
            if let Some(span) = self.spans.get(span_index)
                && compare_times(*point, span.source_start) != std::cmp::Ordering::Less
                && compare_times(*point, span.source_end) == std::cmp::Ordering::Less
            {
                let offset = point
                    .checked_sub(span.source_start)?
                    .scaled(span.speed_denominator, span.speed_numerator)?;
                mapped.push(span.output_start.checked_add(offset)?);
            }
        }
        Ok(mapped)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactSourcePosition {
    pub span_source_start: RationalTime,
    pub offset: ExactDuration,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StudioTimelineCompiler;

impl StudioTimelineCompiler {
    pub fn compile(
        source: &TimelineSource,
        edits: &EditSpec,
    ) -> Result<CanonicalEditPlan, StudioError> {
        source.validate()?;
        validate_edit_shape(edits)?;
        let mut trim = None;
        for operation in &edits.operations {
            match operation {
                EditOperation::Trim { start, end } => {
                    if trim.replace((*start, *end)).is_some() {
                        return Err(StudioError::MultipleTrims);
                    }
                }
                EditOperation::Split { at } => {
                    validate_point(*at, source.duration)?;
                }
                _ => {
                    let (start, end) = operation.range().ok_or(StudioError::EditOutsideTimeline)?;
                    validate_source_range(start, end, source.duration)?;
                }
            }
        }
        let active = trim.unwrap_or((
            RationalTime::new(0, source.duration.time_base),
            source.duration,
        ));
        validate_source_range(active.0, active.1, source.duration)?;
        validate_operation_overlaps(edits, active)?;

        let mut boundaries = vec![active.0, active.1];
        for operation in &edits.operations {
            match operation {
                EditOperation::Split { at } => {
                    if compare_times(*at, active.0) != std::cmp::Ordering::Greater
                        || compare_times(*at, active.1) != std::cmp::Ordering::Less
                    {
                        return Err(StudioError::EditOutsideTimeline);
                    }
                    boundaries.push(*at);
                }
                EditOperation::Trim { .. } => {}
                _ => {
                    let (start, end) = operation.range().ok_or(StudioError::EditOutsideTimeline)?;
                    if compare_times(start, active.0) == std::cmp::Ordering::Less
                        || compare_times(end, active.1) == std::cmp::Ordering::Greater
                    {
                        return Err(StudioError::EditOutsideTimeline);
                    }
                    boundaries.push(start);
                    boundaries.push(end);
                }
            }
        }
        boundaries.sort_by(|left, right| compare_times(*left, *right));
        boundaries.dedup_by(|left, right| compare_times(*left, *right).is_eq());

        let mut spans = Vec::new();
        let mut output_cursor = ExactDuration::zero();
        for pair in boundaries.windows(2) {
            let start = pair[0];
            let end = pair[1];
            if operation_covers_delete(&edits.operations, start, end) {
                continue;
            }
            let (speed_numerator, speed_denominator) =
                speed_for(&edits.operations, start, end).unwrap_or((1, 1));
            let source_duration = end.checked_sub(start)?;
            let rendered_duration = source_duration.scaled(speed_denominator, speed_numerator)?;
            if rendered_duration.numerator == 0 {
                return Err(StudioError::EmptyOutput);
            }
            let output_end = output_cursor.checked_add(rendered_duration)?;
            spans.push(CompiledTimelineSpan {
                source_start: start,
                source_end: end,
                output_start: output_cursor,
                output_end,
                speed_numerator,
                speed_denominator,
                style: style_for(&edits.operations, start, end),
            });
            output_cursor = output_end;
        }
        if spans.is_empty() {
            return Err(StudioError::EmptyOutput);
        }

        let mut gaps = Vec::new();
        for span in &spans {
            if !coverage_gaps(
                &source.coverage,
                TrackKind::Screen,
                span.source_start,
                span.source_end,
            )
            .is_empty()
            {
                return Err(StudioError::UncoveredRequiredVideo);
            }
            for track in [TrackKind::Microphone, TrackKind::SystemAudio] {
                for (source_start, source_end) in
                    coverage_gaps(&source.coverage, track, span.source_start, span.source_end)
                {
                    gaps.push(GapInstruction {
                        track,
                        source_start,
                        source_end,
                        disposition: GapDisposition::InsertSilence,
                    });
                }
            }
            for (source_start, source_end) in coverage_gaps(
                &source.coverage,
                TrackKind::Camera,
                span.source_start,
                span.source_end,
            ) {
                gaps.push(GapInstruction {
                    track: TrackKind::Camera,
                    source_start,
                    source_end,
                    disposition: GapDisposition::HideCamera,
                });
            }
        }
        if gaps.len() > MAX_STUDIO_GAP_INSTRUCTIONS {
            return Err(StudioError::DocumentTooLarge);
        }
        let source_topology_digest = digest_timeline_source(source)?;
        let edit_spec_digest = digest_edit_spec(edits)?;
        let digest = digest_edit_plan(
            edits.revision,
            source.duration,
            output_cursor,
            &spans,
            &gaps,
            &source.vfr_video_pts,
            EditPlanBindings {
                source_topology_digest,
                edit_spec_digest,
            },
        )?;
        let plan = CanonicalEditPlan {
            version: STUDIO_EDIT_VERSION,
            edit_revision: edits.revision,
            source_duration: source.duration,
            output_duration: output_cursor,
            spans,
            gaps,
            vfr_video_pts: source.vfr_video_pts.clone(),
            source_topology_digest,
            edit_spec_digest,
            digest,
        };
        plan.validate()?;
        Ok(plan)
    }
}

fn validate_source_range(
    start: RationalTime,
    end: RationalTime,
    source_duration: RationalTime,
) -> Result<(), StudioError> {
    if compare_times(start, end) != std::cmp::Ordering::Less
        || compare_times(end, source_duration) == std::cmp::Ordering::Greater
    {
        Err(StudioError::EditOutsideTimeline)
    } else {
        Ok(())
    }
}

fn validate_point(point: RationalTime, source_duration: RationalTime) -> Result<(), StudioError> {
    let zero = RationalTime::new(0, source_duration.time_base);
    if compare_times(point, zero) != std::cmp::Ordering::Greater
        || compare_times(point, source_duration) != std::cmp::Ordering::Less
    {
        Err(StudioError::EditOutsideTimeline)
    } else {
        Ok(())
    }
}

fn compare_times(left: RationalTime, right: RationalTime) -> std::cmp::Ordering {
    (u128::from(left.ticks) * u128::from(right.time_base.0))
        .cmp(&(u128::from(right.ticks) * u128::from(left.time_base.0)))
}

fn compare_duration(left: ExactDuration, right: ExactDuration) -> std::cmp::Ordering {
    compare_fraction(
        left.numerator,
        left.denominator,
        right.numerator,
        right.denominator,
    )
}

fn compare_fraction(
    mut left_numerator: u128,
    mut left_denominator: u128,
    mut right_numerator: u128,
    mut right_denominator: u128,
) -> std::cmp::Ordering {
    let mut reverse = false;
    loop {
        let left_whole = left_numerator / left_denominator;
        let right_whole = right_numerator / right_denominator;
        let whole_order = left_whole.cmp(&right_whole);
        if !whole_order.is_eq() {
            return if reverse {
                whole_order.reverse()
            } else {
                whole_order
            };
        }
        let left_remainder = left_numerator % left_denominator;
        let right_remainder = right_numerator % right_denominator;
        match (left_remainder == 0, right_remainder == 0) {
            (true, true) => return std::cmp::Ordering::Equal,
            (true, false) => {
                return if reverse {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                };
            }
            (false, true) => {
                return if reverse {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            (false, false) => {
                left_numerator = left_denominator;
                left_denominator = left_remainder;
                right_numerator = right_denominator;
                right_denominator = right_remainder;
                reverse = !reverse;
            }
        }
    }
}

fn duration_sub(left: ExactDuration, right: ExactDuration) -> Result<ExactDuration, StudioError> {
    if compare_duration(left, right) == std::cmp::Ordering::Less {
        return Err(StudioError::TimelineUnderflow);
    }
    let common = gcd_u128(left.denominator, right.denominator);
    let left_multiplier = right.denominator / common;
    let right_multiplier = left.denominator / common;
    let left_scaled = left
        .numerator
        .checked_mul(left_multiplier)
        .ok_or(StudioError::TimelineOverflow)?;
    let right_scaled = right
        .numerator
        .checked_mul(right_multiplier)
        .ok_or(StudioError::TimelineOverflow)?;
    ExactDuration::new(
        left_scaled - right_scaled,
        left.denominator
            .checked_mul(left_multiplier)
            .ok_or(StudioError::TimelineOverflow)?,
    )
}

fn ranges_overlap(left: (RationalTime, RationalTime), right: (RationalTime, RationalTime)) -> bool {
    compare_times(left.0, right.1) == std::cmp::Ordering::Less
        && compare_times(right.0, left.1) == std::cmp::Ordering::Less
}

fn range_contains(
    outer: (RationalTime, RationalTime),
    start: RationalTime,
    end: RationalTime,
) -> bool {
    compare_times(outer.0, start) != std::cmp::Ordering::Greater
        && compare_times(outer.1, end) != std::cmp::Ordering::Less
}

fn operation_category(operation: &EditOperation) -> Option<(u8, Option<TrackKind>)> {
    match operation {
        EditOperation::DeleteRange { .. } | EditOperation::Speed { .. } => Some((1, None)),
        EditOperation::AudioGain { track, .. } => Some((2, Some(*track))),
        EditOperation::Layout { .. } => Some((3, None)),
        EditOperation::CameraTransform { .. } => Some((4, None)),
        EditOperation::CursorTransform { .. } => Some((5, None)),
        EditOperation::Background { .. } => Some((6, None)),
        EditOperation::Trim { .. } | EditOperation::Split { .. } => None,
    }
}

fn validate_operation_overlaps(
    edits: &EditSpec,
    active: (RationalTime, RationalTime),
) -> Result<(), StudioError> {
    for operation in &edits.operations {
        let Some(range) = operation.range() else {
            continue;
        };
        if !range_contains(active, range.0, range.1)
            && !matches!(operation, EditOperation::Trim { .. })
        {
            return Err(StudioError::EditOutsideTimeline);
        }
    }
    Ok(())
}

fn operation_covers_delete(
    operations: &[EditOperation],
    start: RationalTime,
    end: RationalTime,
) -> bool {
    operations.iter().any(|operation| {
        matches!(operation, EditOperation::DeleteRange { .. })
            && operation
                .range()
                .is_some_and(|range| range_contains(range, start, end))
    })
}

fn speed_for(
    operations: &[EditOperation],
    start: RationalTime,
    end: RationalTime,
) -> Option<(u32, u32)> {
    operations.iter().find_map(|operation| {
        if let EditOperation::Speed {
            start: range_start,
            end: range_end,
            numerator,
            denominator,
        } = operation
            && range_contains((*range_start, *range_end), start, end)
        {
            Some((*numerator, *denominator))
        } else {
            None
        }
    })
}

fn style_for(
    operations: &[EditOperation],
    start: RationalTime,
    end: RationalTime,
) -> CompositeStyle {
    let mut style = CompositeStyle::default();
    for operation in operations {
        let applies = operation
            .range()
            .is_some_and(|range| range_contains(range, start, end));
        if !applies {
            continue;
        }
        match operation {
            EditOperation::AudioGain {
                track,
                gain_millibels,
                muted,
                ..
            } => {
                let audio = AudioStyle {
                    gain_millibels: *gain_millibels,
                    muted: *muted,
                };
                match track {
                    TrackKind::Microphone => style.microphone = audio,
                    TrackKind::SystemAudio => style.system_audio = audio,
                    TrackKind::Screen | TrackKind::Camera => {}
                }
            }
            EditOperation::Layout { preset, .. } => style.layout = *preset,
            EditOperation::CameraTransform {
                rect,
                corner_radius_milli,
                ..
            } => {
                style.camera = CameraStyle {
                    rect: *rect,
                    corner_radius_milli: *corner_radius_milli,
                };
            }
            EditOperation::CursorTransform {
                scale_milli,
                hidden,
                ..
            } => {
                style.cursor = CursorStyle {
                    scale_milli: *scale_milli,
                    hidden: *hidden,
                };
            }
            EditOperation::Background {
                style: background, ..
            } => style.background = *background,
            EditOperation::Trim { .. }
            | EditOperation::Split { .. }
            | EditOperation::DeleteRange { .. }
            | EditOperation::Speed { .. } => {}
        }
    }
    style
}

fn coverage_gaps(
    coverage: &[SourceCoverage],
    track: TrackKind,
    start: RationalTime,
    end: RationalTime,
) -> Vec<(RationalTime, RationalTime)> {
    let mut cursor = start;
    let mut gaps = Vec::new();
    for range in coverage.iter().filter(|range| range.track == track) {
        if compare_times(range.end, cursor) != std::cmp::Ordering::Greater {
            continue;
        }
        if compare_times(range.start, cursor) == std::cmp::Ordering::Greater {
            let gap_end = if compare_times(range.start, end) == std::cmp::Ordering::Greater {
                end
            } else {
                range.start
            };
            if compare_times(cursor, gap_end) == std::cmp::Ordering::Less {
                gaps.push((cursor, gap_end));
            }
        }
        if compare_times(range.end, cursor) == std::cmp::Ordering::Greater {
            cursor = range.end;
        }
        if compare_times(cursor, end) != std::cmp::Ordering::Less {
            return gaps;
        }
    }
    if compare_times(cursor, end) == std::cmp::Ordering::Less {
        gaps.push((cursor, end));
    }
    gaps
}

#[derive(Debug, Clone, Copy)]
struct EditPlanBindings {
    source_topology_digest: Sha256Digest,
    edit_spec_digest: Sha256Digest,
}

fn digest_edit_plan(
    edit_revision: u64,
    source_duration: RationalTime,
    output_duration: ExactDuration,
    spans: &[CompiledTimelineSpan],
    gaps: &[GapInstruction],
    vfr: &BTreeMap<TrackKind, Vec<RationalTime>>,
    bindings: EditPlanBindings,
) -> Result<Sha256Digest, StudioError> {
    let mut writer = CanonicalWriter::new();
    writer.u16(STUDIO_EDIT_VERSION);
    writer.u64(edit_revision);
    writer.digest(bindings.source_topology_digest)?;
    writer.digest(bindings.edit_spec_digest)?;
    encode_time(&mut writer, source_duration);
    writer.u128(output_duration.numerator);
    writer.u128(output_duration.denominator);
    writer.u32(u32::try_from(spans.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for span in spans {
        encode_time(&mut writer, span.source_start);
        encode_time(&mut writer, span.source_end);
        writer.u128(span.output_start.numerator);
        writer.u128(span.output_start.denominator);
        writer.u128(span.output_end.numerator);
        writer.u128(span.output_end.denominator);
        writer.u32(span.speed_numerator);
        writer.u32(span.speed_denominator);
        encode_composite_style(&mut writer, span.style);
    }
    writer.u32(u32::try_from(gaps.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for gap in gaps {
        writer.u8(gap.track.tag());
        encode_time(&mut writer, gap.source_start);
        encode_time(&mut writer, gap.source_end);
        writer.u8(match gap.disposition {
            GapDisposition::InsertSilence => 1,
            GapDisposition::HideCamera => 2,
        });
    }
    writer.u8(u8::try_from(vfr.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for (track, points) in vfr {
        writer.u8(track.tag());
        writer.u32(u32::try_from(points.len()).map_err(|_| StudioError::DocumentTooLarge)?);
        for point in points {
            encode_time(&mut writer, *point);
        }
    }
    Ok(strong_sha256(&writer.finish()?))
}

fn encode_composite_style(writer: &mut CanonicalWriter, style: CompositeStyle) {
    writer.u8(layout_tag(style.layout));
    writer.u32(style.camera.rect.x_millionths);
    writer.u32(style.camera.rect.y_millionths);
    writer.u32(style.camera.rect.width_millionths);
    writer.u32(style.camera.rect.height_millionths);
    writer.u16(style.camera.corner_radius_milli);
    writer.u16(style.cursor.scale_milli);
    writer.bool(style.cursor.hidden);
    match style.background {
        BackgroundStyle::Transparent => writer.u8(1),
        BackgroundStyle::SolidRgb { red, green, blue } => {
            writer.u8(2);
            writer.u8(red);
            writer.u8(green);
            writer.u8(blue);
        }
        BackgroundStyle::Blur { radius_milli } => {
            writer.u8(3);
            writer.u16(radius_milli);
        }
    }
    writer.i32(style.microphone.gain_millibels);
    writer.bool(style.microphone.muted);
    writer.i32(style.system_audio.gain_millibels);
    writer.bool(style.system_audio.muted);
}

fn validate_composite_style(style: CompositeStyle) -> Result<(), StudioError> {
    style.camera.rect.validate()?;
    if style.camera.corner_radius_milli > 1_000
        || !(100..=4_000).contains(&style.cursor.scale_milli)
        || !(-9_600..=2_400).contains(&style.microphone.gain_millibels)
        || !(-9_600..=2_400).contains(&style.system_audio.gain_millibels)
        || matches!(
            style.background,
            BackgroundStyle::Blur { radius_milli } if radius_milli > 60_000
        )
    {
        return Err(StudioError::InvalidCompiledPlan);
    }
    Ok(())
}

impl CanonicalWriter {
    fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub fn validate(self) -> Result<Self, StudioError> {
        if self.numerator == 0 || self.denominator == 0 || self.numerator > 240_000 {
            return Err(StudioError::InvalidFrameRate);
        }
        Ok(self)
    }
}

pub fn simulate_cfr_timestamps(
    plan: &CanonicalEditPlan,
    frame_rate: FrameRate,
    maximum_frames: usize,
) -> Result<Vec<ExactDuration>, StudioError> {
    frame_rate.validate()?;
    if maximum_frames > MAX_STUDIO_SIMULATED_TIMESTAMPS {
        return Err(StudioError::SimulationLimitExceeded);
    }
    let mut timestamps = Vec::new();
    for index in 0..maximum_frames {
        let timestamp = ExactDuration::new(
            (index as u128)
                .checked_mul(u128::from(frame_rate.denominator))
                .ok_or(StudioError::TimelineOverflow)?,
            u128::from(frame_rate.numerator),
        )?;
        if compare_duration(timestamp, plan.output_duration) != std::cmp::Ordering::Less {
            return Ok(timestamps);
        }
        timestamps.push(timestamp);
    }
    let next = ExactDuration::new(
        (maximum_frames as u128)
            .checked_mul(u128::from(frame_rate.denominator))
            .ok_or(StudioError::TimelineOverflow)?,
        u128::from(frame_rate.numerator),
    )?;
    if compare_duration(next, plan.output_duration) == std::cmp::Ordering::Less {
        Err(StudioError::SimulationLimitExceeded)
    } else {
        Ok(timestamps)
    }
}

pub fn simulate_audio_block_timestamps(
    plan: &CanonicalEditPlan,
    sample_rate: u32,
    samples_per_block: u32,
    maximum_blocks: usize,
) -> Result<Vec<ExactDuration>, StudioError> {
    if sample_rate == 0 || samples_per_block == 0 {
        return Err(StudioError::InvalidAudioFormat);
    }
    if maximum_blocks > MAX_STUDIO_SIMULATED_TIMESTAMPS {
        return Err(StudioError::SimulationLimitExceeded);
    }
    let mut timestamps = Vec::new();
    for index in 0..maximum_blocks {
        let timestamp = ExactDuration::new(
            (index as u128)
                .checked_mul(u128::from(samples_per_block))
                .ok_or(StudioError::TimelineOverflow)?,
            u128::from(sample_rate),
        )?;
        if compare_duration(timestamp, plan.output_duration) != std::cmp::Ordering::Less {
            return Ok(timestamps);
        }
        timestamps.push(timestamp);
    }
    let next = ExactDuration::new(
        (maximum_blocks as u128)
            .checked_mul(u128::from(samples_per_block))
            .ok_or(StudioError::TimelineOverflow)?,
        u128::from(sample_rate),
    )?;
    if compare_duration(next, plan.output_duration) == std::cmp::Ordering::Less {
        Err(StudioError::SimulationLimitExceeded)
    } else {
        Ok(timestamps)
    }
}

// ---------------------------------------------------------------------------
// Preview and render graph contracts

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaContainer {
    Mp4,
    WebM,
    Matroska,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StudioVideoCodec {
    H264Avc,
    H265Hevc,
    Vp9,
    Av1,
    Ffv1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StudioAudioCodec {
    AacLowComplexity,
    Opus,
    Flac,
    Pcm24,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudioColorSpace {
    Bt709Limited,
    Bt709Full,
    DisplayP3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportProfile {
    /// H.264/AAC MP4 distribution master constrained to the approved hosted
    /// media ingestion contract.
    DistributionMaster,
    /// VP9/Opus WebM native high-quality output.
    NativeHighQualityWebM,
    /// HEVC/AAC MP4 native high-quality output; requires an explicit license.
    NativeHighQualityHevc,
    /// FFV1/FLAC Matroska archival output.
    NativeArchiveLossless,
}

const fn export_profile_tag(profile: ExportProfile) -> u8 {
    match profile {
        ExportProfile::DistributionMaster => 1,
        ExportProfile::NativeHighQualityWebM => 2,
        ExportProfile::NativeHighQualityHevc => 3,
        ExportProfile::NativeArchiveLossless => 4,
    }
}

fn export_profile_from_tag(tag: u8) -> Result<ExportProfile, StudioError> {
    match tag {
        1 => Ok(ExportProfile::DistributionMaster),
        2 => Ok(ExportProfile::NativeHighQualityWebM),
        3 => Ok(ExportProfile::NativeHighQualityHevc),
        4 => Ok(ExportProfile::NativeArchiveLossless),
        _ => Err(StudioError::MalformedDocument),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProfileSpec {
    pub profile: ExportProfile,
    pub container: MediaContainer,
    pub video_codec: StudioVideoCodec,
    pub audio_codec: StudioAudioCodec,
    pub resolution: Resolution,
    pub frame_rate: FrameRate,
    pub color: StudioColorSpace,
    pub audio_sample_rate: u32,
    pub audio_channels: u8,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
    pub hosted_distribution_compatible: bool,
}

impl ExportProfileSpec {
    #[must_use]
    pub const fn approved(profile: ExportProfile) -> Self {
        match profile {
            ExportProfile::DistributionMaster => Self {
                profile,
                container: MediaContainer::Mp4,
                video_codec: StudioVideoCodec::H264Avc,
                audio_codec: StudioAudioCodec::AacLowComplexity,
                resolution: Resolution {
                    width: 1_920,
                    height: 1_080,
                },
                frame_rate: FrameRate {
                    numerator: 30,
                    denominator: 1,
                },
                color: StudioColorSpace::Bt709Limited,
                audio_sample_rate: 48_000,
                audio_channels: 2,
                video_bitrate: 12_000_000,
                audio_bitrate: 256_000,
                hosted_distribution_compatible: true,
            },
            ExportProfile::NativeHighQualityWebM => Self {
                profile,
                container: MediaContainer::WebM,
                video_codec: StudioVideoCodec::Vp9,
                audio_codec: StudioAudioCodec::Opus,
                resolution: Resolution {
                    width: 2_560,
                    height: 1_440,
                },
                frame_rate: FrameRate {
                    numerator: 60,
                    denominator: 1,
                },
                color: StudioColorSpace::Bt709Full,
                audio_sample_rate: 48_000,
                audio_channels: 2,
                video_bitrate: 24_000_000,
                audio_bitrate: 320_000,
                hosted_distribution_compatible: false,
            },
            ExportProfile::NativeHighQualityHevc => Self {
                profile,
                container: MediaContainer::Mp4,
                video_codec: StudioVideoCodec::H265Hevc,
                audio_codec: StudioAudioCodec::AacLowComplexity,
                resolution: Resolution {
                    width: 3_840,
                    height: 2_160,
                },
                frame_rate: FrameRate {
                    numerator: 60,
                    denominator: 1,
                },
                color: StudioColorSpace::DisplayP3,
                audio_sample_rate: 48_000,
                audio_channels: 2,
                video_bitrate: 60_000_000,
                audio_bitrate: 320_000,
                hosted_distribution_compatible: false,
            },
            ExportProfile::NativeArchiveLossless => Self {
                profile,
                container: MediaContainer::Matroska,
                video_codec: StudioVideoCodec::Ffv1,
                audio_codec: StudioAudioCodec::Flac,
                resolution: Resolution {
                    width: 3_840,
                    height: 2_160,
                },
                frame_rate: FrameRate {
                    numerator: 60,
                    denominator: 1,
                },
                color: StudioColorSpace::Bt709Full,
                audio_sample_rate: 48_000,
                audio_channels: 2,
                video_bitrate: 0,
                audio_bitrate: 0,
                hosted_distribution_compatible: false,
            },
        }
    }

    pub fn validate(self) -> Result<Self, StudioError> {
        if self != Self::approved(self.profile) {
            return Err(StudioError::InvalidExportProfile);
        }
        self.frame_rate.validate()?;
        if self.resolution.width == 0
            || self.resolution.height == 0
            || !self.resolution.width.is_multiple_of(2)
            || !self.resolution.height.is_multiple_of(2)
            || self.audio_sample_rate != 48_000
            || !(1..=2).contains(&self.audio_channels)
        {
            return Err(StudioError::InvalidExportProfile);
        }
        if self.hosted_distribution_compatible
            && (self.profile != ExportProfile::DistributionMaster
                || self.container != MediaContainer::Mp4
                || self.video_codec != StudioVideoCodec::H264Avc
                || self.audio_codec != StudioAudioCodec::AacLowComplexity
                || self.color != StudioColorSpace::Bt709Limited)
        {
            return Err(StudioError::InvalidExportProfile);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncoderBackend {
    Hardware,
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodecLicense {
    H264Encode,
    H265Encode,
    AacEncode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderCapabilities {
    pub contract_version: u16,
    pub containers: BTreeSet<MediaContainer>,
    pub hardware_video: BTreeSet<StudioVideoCodec>,
    pub software_video: BTreeSet<StudioVideoCodec>,
    pub audio: BTreeSet<StudioAudioCodec>,
    pub licenses: BTreeSet<CodecLicense>,
    pub maximum_resolution: Resolution,
    pub maximum_frame_rate: FrameRate,
    pub bounded_renderer_queue: bool,
    pub cancellation: bool,
    pub postcondition_probe: bool,
    pub exact_partial_cleanup: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPreflight {
    pub profile: ExportProfileSpec,
    pub selected_backend: EncoderBackend,
    pub software_fallback_available: bool,
}

pub fn preflight_render(
    profile: ExportProfile,
    capabilities: &RenderCapabilities,
) -> Result<RenderPreflight, StudioError> {
    if capabilities.contract_version != STUDIO_RENDER_PROTOCOL_VERSION
        || !capabilities.bounded_renderer_queue
        || !capabilities.cancellation
        || !capabilities.postcondition_probe
        || !capabilities.exact_partial_cleanup
        || capabilities.maximum_resolution.width == 0
        || capabilities.maximum_resolution.height == 0
        || capabilities.maximum_frame_rate.validate().is_err()
    {
        return Err(StudioError::IncompatibleRenderer);
    }
    let profile = ExportProfileSpec::approved(profile).validate()?;
    if !capabilities.containers.contains(&profile.container)
        || !capabilities.audio.contains(&profile.audio_codec)
        || profile.resolution.width > capabilities.maximum_resolution.width
        || profile.resolution.height > capabilities.maximum_resolution.height
        || u64::from(profile.frame_rate.numerator)
            * u64::from(capabilities.maximum_frame_rate.denominator)
            > u64::from(capabilities.maximum_frame_rate.numerator)
                * u64::from(profile.frame_rate.denominator)
    {
        return Err(StudioError::UnsupportedRenderProfile);
    }
    for required in required_licenses(profile) {
        if !capabilities.licenses.contains(&required) {
            return Err(StudioError::MissingCodecLicense(required));
        }
    }
    let software = capabilities.software_video.contains(&profile.video_codec);
    let selected_backend = if capabilities.hardware_video.contains(&profile.video_codec) {
        EncoderBackend::Hardware
    } else if software {
        EncoderBackend::Software
    } else {
        return Err(StudioError::UnsupportedRenderProfile);
    };
    Ok(RenderPreflight {
        profile,
        selected_backend,
        software_fallback_available: software,
    })
}

fn validate_selected_preflight(
    selected: RenderPreflight,
    capabilities: &RenderCapabilities,
) -> Result<(), StudioError> {
    let preferred = preflight_render(selected.profile.profile, capabilities)?;
    if selected.profile != preferred.profile
        || selected.software_fallback_available != preferred.software_fallback_available
    {
        return Err(StudioError::RendererCapabilityChanged);
    }
    match selected.selected_backend {
        EncoderBackend::Hardware if preferred.selected_backend == EncoderBackend::Hardware => {
            Ok(())
        }
        EncoderBackend::Software if preferred.software_fallback_available => Ok(()),
        EncoderBackend::Hardware | EncoderBackend::Software => {
            Err(StudioError::RendererCapabilityChanged)
        }
    }
}

fn required_licenses(profile: ExportProfileSpec) -> Vec<CodecLicense> {
    let mut required = Vec::new();
    match profile.video_codec {
        StudioVideoCodec::H264Avc => required.push(CodecLicense::H264Encode),
        StudioVideoCodec::H265Hevc => required.push(CodecLicense::H265Encode),
        StudioVideoCodec::Vp9 | StudioVideoCodec::Av1 | StudioVideoCodec::Ffv1 => {}
    }
    if profile.audio_codec == StudioAudioCodec::AacLowComplexity {
        required.push(CodecLicense::AacEncode);
    }
    required
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareFailureDisposition {
    RestartFromCleanPartialWithSoftware,
    DeletePartialAndFailSafely,
}

#[must_use]
pub fn hardware_failure_disposition(preflight: RenderPreflight) -> HardwareFailureDisposition {
    if preflight.selected_backend == EncoderBackend::Hardware
        && preflight.software_fallback_available
    {
        HardwareFailureDisposition::RestartFromCleanPartialWithSoftware
    } else {
        HardwareFailureDisposition::DeletePartialAndFailSafely
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderNodeFamily {
    SourceDemux,
    TimelineMapper,
    VideoCompositor,
    AudioMixer,
    ColorConverter,
    HardwareVideoEncoder,
    SoftwareVideoEncoder,
    AudioEncoder,
    Mp4Mux,
    WebMMux,
    MatroskaMux,
    AtomicFileSink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioSourceSet {
    project_id: StudioProjectId,
    project_revision: u64,
    edits: EditSpec,
    timeline: TimelineSource,
    assets: Vec<StudioAsset>,
    digest: Sha256Digest,
}

impl StudioSourceSet {
    pub fn from_project(
        project: &StudioProjectManifest,
        timeline: &TimelineSource,
    ) -> Result<Self, StudioError> {
        project.validate()?;
        validate_assets_match_timeline(&project.assets, timeline)?;
        let digest = digest_source_set(
            project.id,
            project.revision,
            &project.edits,
            timeline,
            &project.assets,
        )?;
        let sources = Self {
            project_id: project.id,
            project_revision: project.revision,
            edits: project.edits.clone(),
            timeline: timeline.clone(),
            assets: project.assets.clone(),
            digest,
        };
        sources.validate()?;
        Ok(sources)
    }

    pub fn validate(&self) -> Result<(), StudioError> {
        if self.project_revision == 0
            || self.assets.is_empty()
            || self.assets.len() > MAX_STUDIO_ASSETS
            || !self
                .assets
                .iter()
                .any(|asset| asset.track == TrackKind::Screen)
        {
            return Err(StudioError::InvalidSourceSet);
        }
        validate_edit_shape(&self.edits)?;
        validate_assets_match_timeline(&self.assets, &self.timeline)?;
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for asset in &self.assets {
            asset.validate()?;
            if asset.commit_state != AssetCommitState::DurableOriginal
                || !ids.insert(asset.id)
                || !names.insert(asset.source_name.clone())
            {
                return Err(StudioError::InvalidSourceSet);
            }
        }
        if digest_source_set(
            self.project_id,
            self.project_revision,
            &self.edits,
            &self.timeline,
            &self.assets,
        )? != self.digest
        {
            return Err(StudioError::CorruptSourceSet);
        }
        Ok(())
    }

    #[must_use]
    pub const fn project_id(&self) -> StudioProjectId {
        self.project_id
    }

    #[must_use]
    pub const fn project_revision(&self) -> u64 {
        self.project_revision
    }

    #[must_use]
    pub fn edits(&self) -> &EditSpec {
        &self.edits
    }

    #[must_use]
    pub fn timeline(&self) -> &TimelineSource {
        &self.timeline
    }

    #[must_use]
    pub fn assets(&self) -> &[StudioAsset] {
        &self.assets
    }

    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }
}

fn digest_source_set(
    project_id: StudioProjectId,
    project_revision: u64,
    edits: &EditSpec,
    timeline: &TimelineSource,
    assets: &[StudioAsset],
) -> Result<Sha256Digest, StudioError> {
    let mut writer = CanonicalWriter::new();
    writer.id(project_id.canonical_bytes());
    writer.u64(project_revision);
    writer.digest(digest_edit_spec(edits)?)?;
    writer.digest(digest_timeline_source(timeline)?)?;
    writer.u16(u16::try_from(assets.len()).map_err(|_| StudioError::DocumentTooLarge)?);
    for asset in assets {
        encode_asset(&mut writer, asset)?;
    }
    Ok(strong_sha256(&writer.finish()?))
}

fn validate_assets_match_timeline(
    assets: &[StudioAsset],
    timeline: &TimelineSource,
) -> Result<(), StudioError> {
    timeline.validate()?;
    let mut expected = Vec::with_capacity(assets.len());
    for asset in assets {
        asset.validate()?;
        expected.push((
            asset.track,
            asset
                .start
                .checked_sub(RationalTime::new(0, asset.start.time_base))?,
            asset.end()?,
        ));
    }
    expected.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| compare_duration(left.1, right.1))
            .then_with(|| compare_duration(left.2, right.2))
    });
    if expected.len() != timeline.coverage.len() {
        return Err(StudioError::SourceSetTimelineMismatch);
    }
    for (expected, actual) in expected.iter().zip(&timeline.coverage) {
        let actual_start = actual
            .start
            .checked_sub(RationalTime::new(0, actual.start.time_base))?;
        let actual_end = actual
            .end
            .checked_sub(RationalTime::new(0, actual.end.time_base))?;
        if expected.0 != actual.track || expected.1 != actual_start || expected.2 != actual_end {
            return Err(StudioError::SourceSetTimelineMismatch);
        }
    }
    let declared_duration = timeline
        .duration
        .checked_sub(RationalTime::new(0, timeline.duration.time_base))?;
    let maximum_end = expected
        .iter()
        .map(|entry| entry.2)
        .max_by(|left, right| compare_duration(*left, *right))
        .ok_or(StudioError::InvalidSourceSet)?;
    if declared_duration != maximum_end {
        return Err(StudioError::SourceSetTimelineMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioRenderGraphSpec {
    pub sources: StudioSourceSet,
    pub plan: CanonicalEditPlan,
    pub preflight: RenderPreflight,
    pub nodes: Vec<RenderNodeFamily>,
    pub queues: Vec<BoundedMediaQueue>,
}

impl StudioRenderGraphSpec {
    pub fn compile(
        sources: StudioSourceSet,
        plan: CanonicalEditPlan,
        preflight: RenderPreflight,
    ) -> Result<Self, StudioError> {
        sources.validate()?;
        plan.validate()?;
        preflight.profile.validate()?;
        let nodes = expected_render_nodes(preflight);
        let queue = BoundedMediaQueue {
            max_buffers: 64,
            max_bytes: 64 * 1024 * 1024,
            max_time_ns: 2_000_000_000,
        }
        .validate()?;
        let graph = Self {
            sources,
            plan,
            preflight,
            nodes,
            queues: vec![queue; 6],
        };
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), StudioError> {
        self.sources.validate()?;
        self.plan.validate()?;
        self.preflight.profile.validate()?;
        if self.nodes != expected_render_nodes(self.preflight)
            || self.queues.len() != 6
            || self.queues.iter().any(|queue| queue.validate().is_err())
        {
            return Err(StudioError::InvalidRenderGraph);
        }
        validate_source_set_duration(&self.sources, &self.plan)?;
        Ok(())
    }

    #[must_use]
    pub const fn edit_plan_digest(&self) -> Sha256Digest {
        self.plan.digest()
    }
}

fn expected_render_nodes(preflight: RenderPreflight) -> Vec<RenderNodeFamily> {
    let video_encoder = match preflight.selected_backend {
        EncoderBackend::Hardware => RenderNodeFamily::HardwareVideoEncoder,
        EncoderBackend::Software => RenderNodeFamily::SoftwareVideoEncoder,
    };
    let muxer = match preflight.profile.container {
        MediaContainer::Mp4 => RenderNodeFamily::Mp4Mux,
        MediaContainer::WebM => RenderNodeFamily::WebMMux,
        MediaContainer::Matroska => RenderNodeFamily::MatroskaMux,
    };
    vec![
        RenderNodeFamily::SourceDemux,
        RenderNodeFamily::TimelineMapper,
        RenderNodeFamily::VideoCompositor,
        RenderNodeFamily::AudioMixer,
        RenderNodeFamily::ColorConverter,
        video_encoder,
        RenderNodeFamily::AudioEncoder,
        muxer,
        RenderNodeFamily::AtomicFileSink,
    ]
}

fn validate_source_set_duration(
    sources: &StudioSourceSet,
    plan: &CanonicalEditPlan,
) -> Result<(), StudioError> {
    if plan.edit_revision != sources.edits.revision
        || plan.edit_spec_digest() != digest_edit_spec(&sources.edits)?
        || plan.source_topology_digest() != digest_timeline_source(&sources.timeline)?
        || plan.source_duration != sources.timeline.duration
    {
        return Err(StudioError::SourceSetTimelineMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioPreviewGraphSpec {
    pub sources: StudioSourceSet,
    pub plan: CanonicalEditPlan,
    pub decoded_frame_queue: BoundedMediaQueue,
    pub audio_queue: BoundedMediaQueue,
}

impl StudioPreviewGraphSpec {
    pub fn compile(sources: StudioSourceSet, plan: CanonicalEditPlan) -> Result<Self, StudioError> {
        sources.validate()?;
        plan.validate()?;
        let graph = Self {
            sources,
            plan,
            decoded_frame_queue: BoundedMediaQueue {
                max_buffers: 8,
                max_bytes: 128 * 1024 * 1024,
                max_time_ns: 500_000_000,
            }
            .validate()?,
            audio_queue: BoundedMediaQueue {
                max_buffers: 32,
                max_bytes: 8 * 1024 * 1024,
                max_time_ns: 500_000_000,
            }
            .validate()?,
        };
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), StudioError> {
        self.sources.validate()?;
        self.plan.validate()?;
        self.decoded_frame_queue.validate()?;
        self.audio_queue.validate()?;
        validate_source_set_duration(&self.sources, &self.plan)?;
        Ok(())
    }

    #[must_use]
    pub const fn edit_plan_digest(&self) -> Sha256Digest {
        self.plan.digest()
    }

    pub fn seek(&self, output: ExactDuration) -> Result<ExactSourcePosition, StudioError> {
        self.validate()?;
        self.plan.seek(output)
    }
}

/// A bounded single-consumer payload used for small renderer control assets
/// (for example a background bitmap). Media originals remain adapter-owned and
/// are referenced by immutable checksum, never buffered through this port.
pub trait StudioOneShotPayload {
    fn declared_len(&self) -> u64;
    fn pull(&mut self, maximum_bytes: usize) -> Result<Option<Vec<u8>>, StudioError>;
    fn cancel(&mut self);
}

pub fn consume_bounded_control_payload<P: StudioOneShotPayload>(
    payload: &mut P,
    expected: AssetChecksum,
    maximum_total_bytes: usize,
) -> Result<Vec<u8>, StudioError> {
    if maximum_total_bytes == 0 || maximum_total_bytes > MAX_STUDIO_CONTROL_PAYLOAD_BYTES {
        payload.cancel();
        return Err(StudioError::PayloadTooLarge);
    }
    let declared =
        usize::try_from(payload.declared_len()).map_err(|_| StudioError::PayloadTooLarge)?;
    if declared == 0 || declared > maximum_total_bytes {
        payload.cancel();
        return Err(StudioError::PayloadTooLarge);
    }
    let mut bytes = Vec::with_capacity(declared);
    loop {
        match payload.pull(MAX_STUDIO_PAYLOAD_CHUNK_BYTES) {
            Ok(Some(chunk)) => {
                if chunk.is_empty() || chunk.len() > MAX_STUDIO_PAYLOAD_CHUNK_BYTES {
                    payload.cancel();
                    return Err(StudioError::InvalidPayloadChunk);
                }
                if bytes
                    .len()
                    .checked_add(chunk.len())
                    .is_none_or(|length| length > declared)
                {
                    payload.cancel();
                    return Err(StudioError::PayloadLengthMismatch);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                payload.cancel();
                return Err(error);
            }
        }
    }
    if bytes.len() != declared {
        payload.cancel();
        return Err(StudioError::PayloadLengthMismatch);
    }
    if AssetChecksum::from_content(&bytes) != expected {
        payload.cancel();
        return Err(StudioError::InvalidChecksum);
    }
    Ok(bytes)
}

#[derive(Debug)]
pub struct StudioRenderTicket {
    project_id: StudioProjectId,
    export_id: StudioExportId,
    operation_id: StudioOperationId,
    expected_fence: u64,
    output_name: StudioSourceName,
    graph: StudioRenderGraphSpec,
    render_spec_digest: Sha256Digest,
    deadline: Duration,
}

impl StudioRenderTicket {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: StudioProjectId,
        export_id: StudioExportId,
        operation_id: StudioOperationId,
        expected_fence: u64,
        output_name: StudioSourceName,
        graph: StudioRenderGraphSpec,
        deadline: Duration,
    ) -> Result<Self, StudioError> {
        if expected_fence == 0
            || deadline.is_zero()
            || deadline > Duration::from_secs(7 * 24 * 60 * 60)
        {
            return Err(StudioError::InvalidRenderTicket);
        }
        graph.validate()?;
        if graph.sources.project_id != project_id {
            return Err(StudioError::SourceSetTimelineMismatch);
        }
        let render_spec_digest = digest_render_spec(&graph, &output_name)?;
        Ok(Self {
            project_id,
            export_id,
            operation_id,
            expected_fence,
            output_name,
            graph,
            render_spec_digest,
            deadline,
        })
    }

    #[must_use]
    pub const fn project_id(&self) -> StudioProjectId {
        self.project_id
    }

    #[must_use]
    pub const fn export_id(&self) -> StudioExportId {
        self.export_id
    }

    #[must_use]
    pub const fn operation_id(&self) -> StudioOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn expected_fence(&self) -> u64 {
        self.expected_fence
    }

    #[must_use]
    pub fn output_name(&self) -> &StudioSourceName {
        &self.output_name
    }

    #[must_use]
    pub fn graph(&self) -> &StudioRenderGraphSpec {
        &self.graph
    }

    #[must_use]
    pub const fn render_spec_digest(&self) -> Sha256Digest {
        self.render_spec_digest
    }

    #[must_use]
    pub const fn deadline(&self) -> Duration {
        self.deadline
    }
}

fn digest_render_spec(
    graph: &StudioRenderGraphSpec,
    output_name: &StudioSourceName,
) -> Result<Sha256Digest, StudioError> {
    graph.validate()?;
    digest_render_identity(
        graph.sources.digest(),
        graph.edit_plan_digest(),
        graph.preflight.profile.profile,
        output_name,
    )
}

fn digest_render_identity(
    source_set_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    profile: ExportProfile,
    output_name: &StudioSourceName,
) -> Result<Sha256Digest, StudioError> {
    let mut writer = CanonicalWriter::new();
    writer.u16(STUDIO_RENDER_PROTOCOL_VERSION);
    writer.digest(source_set_digest)?;
    writer.digest(plan_digest)?;
    writer.u8(export_profile_tag(profile));
    writer.string(output_name.as_str())?;
    Ok(strong_sha256(&writer.finish()?))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPhase {
    Preparing,
    Decoding,
    Compositing,
    Encoding,
    Muxing,
    Finalizing,
}

impl RenderPhase {
    const fn rank(self) -> u8 {
        match self {
            Self::Preparing => 1,
            Self::Decoding => 2,
            Self::Compositing => 3,
            Self::Encoding => 4,
            Self::Muxing => 5,
            Self::Finalizing => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderEventKind {
    Progress {
        phase: RenderPhase,
        basis_points: u16,
    },
    Committed {
        output_checksum: AssetChecksum,
        output_bytes: u64,
    },
    Cancelled,
    Failed {
        safe_code: &'static str,
        hardware_failure: bool,
    },
}

fn valid_safe_render_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderEvent {
    pub project_id: StudioProjectId,
    pub export_id: StudioExportId,
    pub fence: u64,
    pub render_spec_digest: Sha256Digest,
    pub sequence: u64,
    pub kind: RenderEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStartOutcome {
    Accepted,
    AcknowledgementLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderPostcondition {
    Absent,
    Running {
        fence: u64,
        render_spec_digest: Sha256Digest,
    },
    Partial {
        fence: u64,
        render_spec_digest: Sha256Digest,
    },
    Committed {
        fence: u64,
        render_spec_digest: Sha256Digest,
        output_checksum: AssetChecksum,
        output_bytes: u64,
    },
}

pub trait StudioRendererPort {
    fn capabilities(&mut self) -> Result<RenderCapabilities, StudioError>;
    fn start(&mut self, ticket: StudioRenderTicket) -> Result<RenderStartOutcome, StudioError>;
    fn poll(
        &mut self,
        export_id: StudioExportId,
        maximum_events: usize,
        wait: Duration,
    ) -> Result<Vec<RenderEvent>, StudioError>;
    fn probe(&mut self, export_id: StudioExportId) -> Result<RenderPostcondition, StudioError>;
    fn cancel(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        deadline: Duration,
    ) -> Result<(), StudioError>;
    fn cleanup_partial(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        expected_render_spec_digest: Sha256Digest,
        output_name: &StudioSourceName,
    ) -> Result<(), StudioError>;
}

/// Durable local reference renderer. It executes a validated Studio graph by
/// streaming every immutable original into a canonical, checksum-bound export
/// bundle. Inflight and committed sidecars are persisted atomically so probe,
/// cancellation, cleanup, and restart reconciliation do not depend on memory.
/// Native codec adapters can implement the same port while retaining these
/// fencing semantics.
pub struct FilesystemStudioRenderer {
    originals_root: PathBuf,
    outputs_root: PathBuf,
    capabilities: RenderCapabilities,
    events: BTreeMap<StudioExportId, VecDeque<RenderEvent>>,
}

impl fmt::Debug for FilesystemStudioRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemStudioRenderer")
            .field("originals_root", &"<redacted>")
            .field("outputs_root", &"<redacted>")
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

impl FilesystemStudioRenderer {
    pub fn new(
        originals_root: impl AsRef<Path>,
        outputs_root: impl AsRef<Path>,
        capabilities: RenderCapabilities,
    ) -> Result<Self, StudioError> {
        preflight_capability_contract(&capabilities)?;
        Ok(Self {
            originals_root: prepare_storage_root(originals_root.as_ref())?,
            outputs_root: prepare_storage_root(outputs_root.as_ref())?,
            capabilities,
            events: BTreeMap::new(),
        })
    }

    fn inflight_path(&self, export_id: StudioExportId) -> PathBuf {
        self.outputs_root
            .join(format!("{}.render-inflight", opaque_id_hex(export_id.0)))
    }

    fn receipt_path(&self, export_id: StudioExportId) -> PathBuf {
        self.outputs_root
            .join(format!("{}.render-receipt", opaque_id_hex(export_id.0)))
    }

    fn lock_path(&self, export_id: StudioExportId) -> PathBuf {
        self.outputs_root
            .join(format!("{}.render.lock", opaque_id_hex(export_id.0)))
    }

    fn output_path(&self, record: &FilesystemRenderRecord) -> PathBuf {
        self.outputs_root
            .join(opaque_id_hex(record.project_id.0))
            .join(format!(
                "{}-{}",
                opaque_id_hex(record.export_id.0),
                record.output_name.as_str()
            ))
    }

    fn partial_path(&self, record: &FilesystemRenderRecord) -> PathBuf {
        self.outputs_root
            .join(opaque_id_hex(record.project_id.0))
            .join(format!("{}.partial", opaque_id_hex(record.export_id.0)))
    }

    fn original_path(&self, project_id: StudioProjectId, asset_id: StudioAssetId) -> PathBuf {
        self.originals_root
            .join(opaque_id_hex(project_id.0))
            .join("originals")
            .join(format!("{}.media", opaque_id_hex(asset_id.0)))
    }

    fn read_record(&self, path: &Path) -> Result<Option<FilesystemRenderRecord>, StudioError> {
        let Some(bytes) = read_bounded_file(path, MAX_STUDIO_DOCUMENT_BYTES)? else {
            return Ok(None);
        };
        decode_filesystem_render_record(&bytes).map(Some)
    }

    fn read_export_record(
        &self,
        path: &Path,
        export_id: StudioExportId,
        committed: bool,
    ) -> Result<Option<FilesystemRenderRecord>, StudioError> {
        let Some(record) = self.read_record(path)? else {
            return Ok(None);
        };
        if record.export_id != export_id
            || committed != record.output_checksum.is_some()
            || committed != (record.output_bytes > 0)
        {
            return Err(StudioError::JournalCorrupt);
        }
        Ok(Some(record))
    }

    fn write_record(
        &self,
        path: &Path,
        record: &FilesystemRenderRecord,
        committed: bool,
    ) -> Result<(), StudioError> {
        let bytes = encode_filesystem_render_record(record, committed)?;
        atomic_replace_file(path, &bytes, record.output_bytes.max(record.fence))
    }

    fn execute_bundle(
        &self,
        ticket: &StudioRenderTicket,
        record: &mut FilesystemRenderRecord,
    ) -> Result<(), StudioError> {
        let output = self.output_path(record);
        let partial = self.partial_path(record);
        let parent = output.parent().ok_or(StudioError::UnsafeStoragePath)?;
        fs::create_dir_all(parent).map_err(|_| StudioError::StorageIo)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|_| StudioError::OutputTargetBusy)?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        {
            let mut write_hashed = |bytes: &[u8]| -> Result<(), StudioError> {
                file.write_all(bytes).map_err(|_| StudioError::StorageIo)?;
                hasher.update(bytes);
                total = total
                    .checked_add(
                        u64::try_from(bytes.len()).map_err(|_| StudioError::DocumentTooLarge)?,
                    )
                    .ok_or(StudioError::DocumentTooLarge)?;
                Ok(())
            };

            let mut header = CanonicalWriter::new();
            header.string("frame-studio-render-bundle")?;
            header.u16(STUDIO_RENDER_PROTOCOL_VERSION);
            header.id(ticket.project_id.canonical_bytes());
            header.id(ticket.export_id.canonical_bytes());
            header.id(ticket.operation_id.canonical_bytes());
            header.u64(ticket.expected_fence);
            header.digest(ticket.graph.sources.digest())?;
            header.digest(ticket.graph.edit_plan_digest())?;
            header.digest(ticket.render_spec_digest)?;
            header.u8(export_profile_tag(ticket.graph.preflight.profile.profile));
            header.string(ticket.output_name.as_str())?;
            header.u16(
                u16::try_from(ticket.graph.sources.assets().len())
                    .map_err(|_| StudioError::DocumentTooLarge)?,
            );
            let header = header.finish()?;
            write_hashed(
                &u32::try_from(header.len())
                    .map_err(|_| StudioError::DocumentTooLarge)?
                    .to_be_bytes(),
            )?;
            write_hashed(&header)?;
            for asset in ticket.graph.sources.assets() {
                let source = self.original_path(ticket.project_id, asset.id);
                verify_asset_file(&source, asset.byte_len, asset.checksum)?;
                let encoded_asset = StudioDocumentCodec::encode_asset(asset)?;
                write_hashed(
                    &u32::try_from(encoded_asset.len())
                        .map_err(|_| StudioError::DocumentTooLarge)?
                        .to_be_bytes(),
                )?;
                write_hashed(&encoded_asset)?;
                write_hashed(&asset.byte_len.to_be_bytes())?;
                let mut source_file = File::open(&source).map_err(|_| StudioError::StorageIo)?;
                let mut buffer = vec![0_u8; MAX_STUDIO_PAYLOAD_CHUNK_BYTES];
                loop {
                    let count = source_file
                        .read(&mut buffer)
                        .map_err(|_| StudioError::StorageIo)?;
                    if count == 0 {
                        break;
                    }
                    write_hashed(&buffer[..count])?;
                }
            }
        }
        file.sync_all().map_err(|_| StudioError::StorageIo)?;
        drop(file);
        fs::rename(&partial, &output).map_err(|_| StudioError::StorageIo)?;
        sync_directory(parent)?;
        let digest: [u8; 32] = hasher.finalize().into();
        record.output_checksum = Some(AssetChecksum::from_bytes(digest)?);
        record.output_bytes = total;
        Ok(())
    }
}

impl StudioRendererPort for FilesystemStudioRenderer {
    fn capabilities(&mut self) -> Result<RenderCapabilities, StudioError> {
        Ok(self.capabilities.clone())
    }

    fn start(&mut self, ticket: StudioRenderTicket) -> Result<RenderStartOutcome, StudioError> {
        ticket.graph.validate()?;
        validate_selected_preflight(ticket.graph.preflight, &self.capabilities)?;
        let _lock = acquire_storage_lock(&self.lock_path(ticket.export_id))?;
        if let Some(existing) =
            self.read_export_record(&self.receipt_path(ticket.export_id), ticket.export_id, true)?
        {
            return if existing.matches_ticket(&ticket) {
                Ok(RenderStartOutcome::AcknowledgementLost)
            } else {
                Err(StudioError::ExportIdReused)
            };
        }
        if let Some(existing) = self.read_export_record(
            &self.inflight_path(ticket.export_id),
            ticket.export_id,
            false,
        )? {
            return if existing.matches_ticket(&ticket) {
                Ok(RenderStartOutcome::AcknowledgementLost)
            } else {
                Err(StudioError::ExportIdReused)
            };
        }
        let mut record = FilesystemRenderRecord::from_ticket(&ticket);
        self.write_record(&self.inflight_path(ticket.export_id), &record, false)?;
        self.execute_bundle(&ticket, &mut record)?;
        self.write_record(&self.receipt_path(ticket.export_id), &record, true)?;
        fs::remove_file(self.inflight_path(ticket.export_id))
            .map_err(|_| StudioError::StorageIo)?;
        sync_directory(&self.outputs_root)?;
        let output_checksum = record
            .output_checksum
            .ok_or(StudioError::RenderPostconditionMismatch)?;
        self.events.insert(
            ticket.export_id,
            VecDeque::from([
                RenderEvent {
                    project_id: ticket.project_id,
                    export_id: ticket.export_id,
                    fence: ticket.expected_fence,
                    render_spec_digest: ticket.render_spec_digest,
                    sequence: 1,
                    kind: RenderEventKind::Progress {
                        phase: RenderPhase::Finalizing,
                        basis_points: 10_000,
                    },
                },
                RenderEvent {
                    project_id: ticket.project_id,
                    export_id: ticket.export_id,
                    fence: ticket.expected_fence,
                    render_spec_digest: ticket.render_spec_digest,
                    sequence: 2,
                    kind: RenderEventKind::Committed {
                        output_checksum,
                        output_bytes: record.output_bytes,
                    },
                },
            ]),
        );
        Ok(RenderStartOutcome::Accepted)
    }

    fn poll(
        &mut self,
        export_id: StudioExportId,
        maximum_events: usize,
        _wait: Duration,
    ) -> Result<Vec<RenderEvent>, StudioError> {
        let events = self.events.entry(export_id).or_default();
        let count = maximum_events.min(events.len());
        Ok(events.drain(..count).collect())
    }

    fn probe(&mut self, export_id: StudioExportId) -> Result<RenderPostcondition, StudioError> {
        if let Some(record) =
            self.read_export_record(&self.receipt_path(export_id), export_id, true)?
        {
            let checksum = record
                .output_checksum
                .ok_or(StudioError::RenderPostconditionMismatch)?;
            verify_asset_file(&self.output_path(&record), record.output_bytes, checksum)?;
            return Ok(RenderPostcondition::Committed {
                fence: record.fence,
                render_spec_digest: record.render_spec_digest,
                output_checksum: checksum,
                output_bytes: record.output_bytes,
            });
        }
        if let Some(record) =
            self.read_export_record(&self.inflight_path(export_id), export_id, false)?
        {
            return Ok(RenderPostcondition::Partial {
                fence: record.fence,
                render_spec_digest: record.render_spec_digest,
            });
        }
        Ok(RenderPostcondition::Absent)
    }

    fn cancel(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        _deadline: Duration,
    ) -> Result<(), StudioError> {
        match self.probe(export_id)? {
            RenderPostcondition::Committed { .. } => {
                Err(StudioError::CommittedRenderCannotBeCancelled)
            }
            RenderPostcondition::Partial { fence, .. }
            | RenderPostcondition::Running { fence, .. }
                if fence != expected_fence =>
            {
                Err(StudioError::StaleRenderCallback)
            }
            _ => Ok(()),
        }
    }

    fn cleanup_partial(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        expected_render_spec_digest: Sha256Digest,
        output_name: &StudioSourceName,
    ) -> Result<(), StudioError> {
        let _lock = acquire_storage_lock(&self.lock_path(export_id))?;
        if self
            .read_export_record(&self.receipt_path(export_id), export_id, true)?
            .is_some()
        {
            return Err(StudioError::CommittedRenderCannotBeCancelled);
        }
        let Some(record) =
            self.read_export_record(&self.inflight_path(export_id), export_id, false)?
        else {
            return Ok(());
        };
        if record.fence != expected_fence
            || record.render_spec_digest != expected_render_spec_digest
            || record.output_name != *output_name
        {
            return Err(StudioError::StaleRenderCallback);
        }
        for path in [
            self.partial_path(&record),
            self.output_path(&record),
            self.inflight_path(export_id),
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(StudioError::StorageIo),
            }
        }
        sync_directory(&self.outputs_root)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilesystemRenderRecord {
    project_id: StudioProjectId,
    export_id: StudioExportId,
    operation_id: StudioOperationId,
    fence: u64,
    source_set_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    render_spec_digest: Sha256Digest,
    profile: ExportProfile,
    output_name: StudioSourceName,
    output_checksum: Option<AssetChecksum>,
    output_bytes: u64,
}

impl FilesystemRenderRecord {
    fn from_ticket(ticket: &StudioRenderTicket) -> Self {
        Self {
            project_id: ticket.project_id,
            export_id: ticket.export_id,
            operation_id: ticket.operation_id,
            fence: ticket.expected_fence,
            source_set_digest: ticket.graph.sources.digest(),
            plan_digest: ticket.graph.edit_plan_digest(),
            render_spec_digest: ticket.render_spec_digest,
            profile: ticket.graph.preflight.profile.profile,
            output_name: ticket.output_name.clone(),
            output_checksum: None,
            output_bytes: 0,
        }
    }

    fn matches_ticket(&self, ticket: &StudioRenderTicket) -> bool {
        self.project_id == ticket.project_id
            && self.export_id == ticket.export_id
            && self.operation_id == ticket.operation_id
            && self.fence == ticket.expected_fence
            && self.source_set_digest == ticket.graph.sources.digest()
            && self.plan_digest == ticket.graph.edit_plan_digest()
            && self.render_spec_digest == ticket.render_spec_digest
            && self.profile == ticket.graph.preflight.profile.profile
            && self.output_name == ticket.output_name
    }
}

fn preflight_capability_contract(capabilities: &RenderCapabilities) -> Result<(), StudioError> {
    if capabilities.contract_version != STUDIO_RENDER_PROTOCOL_VERSION
        || !capabilities.bounded_renderer_queue
        || !capabilities.cancellation
        || !capabilities.postcondition_probe
        || !capabilities.exact_partial_cleanup
        || capabilities.maximum_resolution.width == 0
        || capabilities.maximum_resolution.height == 0
        || capabilities.maximum_frame_rate.validate().is_err()
    {
        return Err(StudioError::IncompatibleRenderer);
    }
    Ok(())
}

fn encode_filesystem_render_record(
    record: &FilesystemRenderRecord,
    committed: bool,
) -> Result<Vec<u8>, StudioError> {
    if committed != record.output_checksum.is_some() || committed != (record.output_bytes > 0) {
        return Err(StudioError::RenderPostconditionMismatch);
    }
    let mut writer = CanonicalWriter::new();
    writer.string("frame-studio-render-record")?;
    writer.u16(1);
    writer.bool(committed);
    writer.id(record.project_id.canonical_bytes());
    writer.id(record.export_id.canonical_bytes());
    writer.id(record.operation_id.canonical_bytes());
    writer.u64(record.fence);
    writer.digest(record.source_set_digest)?;
    writer.digest(record.plan_digest)?;
    writer.digest(record.render_spec_digest)?;
    writer.u8(export_profile_tag(record.profile));
    writer.string(record.output_name.as_str())?;
    if let Some(checksum) = record.output_checksum {
        writer.digest(checksum.0)?;
    }
    writer.u64(record.output_bytes);
    let payload = writer.finish()?;
    let mut bytes = b"FRRR".to_vec();
    bytes.extend_from_slice(&payload);
    let digest = strong_sha256(&bytes).to_hex();
    bytes.extend_from_slice(digest.as_bytes());
    Ok(bytes)
}

fn decode_filesystem_render_record(bytes: &[u8]) -> Result<FilesystemRenderRecord, StudioError> {
    if bytes.len() < 68 || bytes.get(..4) != Some(b"FRRR") {
        return Err(StudioError::CorruptDocument);
    }
    let payload_end = bytes.len() - 64;
    if strong_sha256(&bytes[..payload_end]).to_hex().as_bytes() != &bytes[payload_end..] {
        return Err(StudioError::CorruptDocument);
    }
    let mut reader = CanonicalReader::new(&bytes[4..payload_end]);
    if reader.string(64)? != "frame-studio-render-record" || reader.u16()? != 1 {
        return Err(StudioError::MalformedDocument);
    }
    let committed = reader.bool()?;
    let record = FilesystemRenderRecord {
        project_id: StudioProjectId::from_csprng(reader.array_16()?)?,
        export_id: StudioExportId::from_csprng(reader.array_16()?)?,
        operation_id: StudioOperationId::from_csprng(reader.array_16()?)?,
        fence: reader.u64()?,
        source_set_digest: reader.digest()?,
        plan_digest: reader.digest()?,
        render_spec_digest: reader.digest()?,
        profile: export_profile_from_tag(reader.u8()?)?,
        output_name: StudioSourceName::new(reader.string(MAX_STUDIO_SOURCE_NAME_BYTES)?)?,
        output_checksum: if committed {
            Some(AssetChecksum(reader.digest()?))
        } else {
            None
        },
        output_bytes: reader.u64()?,
    };
    reader.finish()?;
    if record.fence == 0
        || committed != record.output_checksum.is_some()
        || committed != (record.output_bytes > 0)
    {
        return Err(StudioError::CorruptDocument);
    }
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderReceipt {
    pub project_id: StudioProjectId,
    pub export_id: StudioExportId,
    pub operation_id: StudioOperationId,
    pub fence: u64,
    pub source_set_digest: Sha256Digest,
    pub plan_digest: Sha256Digest,
    pub render_spec_digest: Sha256Digest,
    pub profile: ExportProfileSpec,
    pub output_name: StudioSourceName,
    pub output_checksum: AssetChecksum,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSessionState {
    Running,
    Committed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderProgressSnapshot {
    pub state: RenderSessionState,
    pub phase: Option<RenderPhase>,
    pub basis_points: u16,
    pub last_sequence: u64,
    pub failure_code: Option<&'static str>,
}

#[derive(Debug, Clone)]
struct RenderSession {
    project_id: StudioProjectId,
    operation_id: StudioOperationId,
    fence: u64,
    source_set_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    render_spec_digest: Sha256Digest,
    profile: ExportProfileSpec,
    output_name: StudioSourceName,
    deadline: Duration,
    last_sequence: u64,
    progress_basis_points: u16,
    state: RenderSessionState,
    backend: EncoderBackend,
    hardware_failure: bool,
    current_phase: Option<RenderPhase>,
    failure_code: Option<&'static str>,
    release_safe: bool,
    committed_output: Option<(AssetChecksum, u64)>,
    events: VecDeque<RenderEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderCommandRecord {
    project_id: StudioProjectId,
    export_id: StudioExportId,
    operation_id: StudioOperationId,
    fence: u64,
    source_set_digest: Sha256Digest,
    plan_digest: Sha256Digest,
    render_spec_digest: Sha256Digest,
    profile: ExportProfileSpec,
    backend: EncoderBackend,
    output_name: StudioSourceName,
    deadline: Duration,
    state: RenderSessionState,
}

impl RenderCommandRecord {
    fn matches_ticket(&self, ticket: &StudioRenderTicket) -> bool {
        self.project_id == ticket.project_id
            && self.export_id == ticket.export_id
            && self.operation_id == ticket.operation_id
            && self.fence == ticket.expected_fence
            && self.source_set_digest == ticket.graph.sources.digest()
            && self.plan_digest == ticket.graph.edit_plan_digest()
            && self.render_spec_digest == ticket.render_spec_digest
            && self.profile == ticket.graph.preflight.profile
            && self.backend == ticket.graph.preflight.selected_backend
            && self.output_name == ticket.output_name
            && self.deadline == ticket.deadline
    }
}

fn authorization_matches_ticket(
    reservation: &RenderRecoveryReservation,
    ticket: &StudioRenderTicket,
) -> bool {
    reservation.project_id == ticket.project_id
        && reservation.pending.export_id == ticket.export_id
        && reservation.pending.operation_id == ticket.operation_id
        && reservation.pending.fence == ticket.expected_fence
        && reservation.pending.source_set_digest == ticket.graph.sources.digest()
        && reservation.pending.plan_digest == ticket.graph.edit_plan_digest()
        && reservation.pending.render_spec_digest == ticket.render_spec_digest
        && reservation.pending.profile == ticket.graph.preflight.profile.profile
        && reservation.pending.output_name == ticket.output_name
}

/// Durable output ownership reconstructed from a journal before a restarted
/// coordinator may dispatch any new render for the same project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRecoveryReservation {
    project_id: StudioProjectId,
    journal_fence: u64,
    boundary: JournalBoundary,
    pending: PendingRender,
}

impl RenderRecoveryReservation {
    fn from_journal(journal: &StudioJournalSnapshot) -> Result<Option<Self>, StudioError> {
        journal.validate()?;
        Ok(journal.pending_render.clone().map(|pending| Self {
            project_id: journal.project_id,
            journal_fence: journal.fence,
            boundary: journal.boundary,
            pending,
        }))
    }

    fn validate(&self) -> Result<(), StudioError> {
        if self.journal_fence == 0
            || self.pending.fence == 0
            || self.pending.render_spec_digest
                != digest_render_identity(
                    self.pending.source_set_digest,
                    self.pending.plan_digest,
                    self.pending.profile,
                    &self.pending.output_name,
                )?
        {
            return Err(StudioError::JournalCorrupt);
        }
        Ok(())
    }

    #[must_use]
    pub const fn project_id(&self) -> StudioProjectId {
        self.project_id
    }

    #[must_use]
    pub const fn fence(&self) -> u64 {
        self.pending.fence
    }

    #[must_use]
    pub const fn journal_fence(&self) -> u64 {
        self.journal_fence
    }

    #[must_use]
    pub const fn boundary(&self) -> JournalBoundary {
        self.boundary
    }

    #[must_use]
    pub fn pending(&self) -> &PendingRender {
        &self.pending
    }
}

trait RenderJournalLease {
    fn advance(
        &mut self,
        boundary: JournalBoundary,
        terminal_receipt: Option<RenderReceipt>,
    ) -> Result<(), StudioError>;
}

impl<P: StudioJournalPort> RenderJournalLease for DurableStudioJournal<P> {
    fn advance(
        &mut self,
        boundary: JournalBoundary,
        terminal_receipt: Option<RenderReceipt>,
    ) -> Result<(), StudioError> {
        self.advance_render_lifecycle(boundary, terminal_receipt)
    }
}

/// Non-cloneable proof that an exact render reservation was durably written.
/// It owns the journal handle so terminal output identity can be persisted by
/// compare-and-swap before the coordinator releases ownership.
pub struct RenderJournalAuthorization {
    reservation: RenderRecoveryReservation,
    lease: Box<dyn RenderJournalLease>,
}

impl fmt::Debug for RenderJournalAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderJournalAuthorization")
            .field("project_id", &self.reservation.project_id)
            .field("export_id", &self.reservation.pending.export_id)
            .field("operation_id", &self.reservation.pending.operation_id)
            .field("boundary", &self.reservation.boundary)
            .finish_non_exhaustive()
    }
}

impl RenderJournalAuthorization {
    #[must_use]
    pub fn reservation(&self) -> &RenderRecoveryReservation {
        &self.reservation
    }

    fn advance(
        &mut self,
        boundary: JournalBoundary,
        terminal_receipt: Option<RenderReceipt>,
    ) -> Result<(), StudioError> {
        self.lease.advance(boundary, terminal_receipt.clone())?;
        self.reservation.boundary = boundary;
        self.reservation.pending.terminal_receipt = terminal_receipt;
        Ok(())
    }

    /// Binds this one-use durable authorization to its exact immutable render
    /// ticket. A mismatched ticket never reaches renderer dispatch.
    pub fn bind(self, ticket: StudioRenderTicket) -> Result<AuthorizedRenderDispatch, StudioError> {
        if self.reservation.boundary != JournalBoundary::RenderPrepared
            || !authorization_matches_ticket(&self.reservation, &ticket)
        {
            return Err(StudioError::RenderReservationRequired);
        }
        Ok(AuthorizedRenderDispatch {
            ticket,
            authorization: self,
        })
    }
}

/// One-use render command whose exact identity is already durably reserved.
pub struct AuthorizedRenderDispatch {
    ticket: StudioRenderTicket,
    authorization: RenderJournalAuthorization,
}

impl fmt::Debug for AuthorizedRenderDispatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedRenderDispatch")
            .field("ticket", &self.ticket)
            .field("authorization", &self.authorization)
            .finish()
    }
}

impl std::ops::Deref for AuthorizedRenderDispatch {
    type Target = StudioRenderTicket;

    fn deref(&self) -> &Self::Target {
        &self.ticket
    }
}

impl<P: StudioJournalPort + 'static> DurableStudioJournal<P> {
    /// Consumes a validated durable journal and mints the only authorization
    /// accepted by `StudioRenderCoordinator::start` or restart recovery.
    pub fn into_render_authorization(self) -> Result<RenderJournalAuthorization, StudioError> {
        let reservation = RenderRecoveryReservation::from_journal(self.snapshot())?
            .ok_or(StudioError::RenderReservationRequired)?;
        reservation.validate()?;
        Ok(RenderJournalAuthorization {
            reservation,
            lease: Box::new(self),
        })
    }
}

#[derive(Debug)]
pub struct StudioRenderCoordinator<R> {
    renderer: R,
    sessions: BTreeMap<StudioExportId, RenderSession>,
    recovered: BTreeMap<StudioExportId, RenderJournalAuthorization>,
    active_journals: BTreeMap<StudioExportId, RenderJournalAuthorization>,
    receipts: BTreeMap<StudioOperationId, RenderReceipt>,
    command_history: BTreeMap<StudioOperationId, RenderCommandRecord>,
    maximum_buffered_events: usize,
}

impl<R: StudioRendererPort> StudioRenderCoordinator<R> {
    /// Reconstructs every durable project/output reservation before allowing
    /// any new native render dispatch. Every entry is minted from an opened
    /// durable journal; callers cannot inject a plain reservation value.
    pub fn new(
        renderer: R,
        maximum_buffered_events: usize,
        recovered: Vec<RenderJournalAuthorization>,
    ) -> Result<Self, StudioError> {
        if maximum_buffered_events == 0 || maximum_buffered_events > 1_024 {
            return Err(StudioError::UnboundedRendererEvents);
        }
        if recovered.len() > MAX_STUDIO_RENDER_SESSIONS {
            return Err(StudioError::RenderConcurrencyLimit);
        }
        let mut recovered_by_export = BTreeMap::new();
        let mut operations = BTreeSet::new();
        let mut outputs = BTreeSet::new();
        for authorization in recovered {
            let reservation = &authorization.reservation;
            reservation.validate()?;
            if !operations.insert(reservation.pending.operation_id)
                || !outputs.insert((
                    reservation.project_id,
                    reservation.pending.output_name.clone(),
                ))
                || recovered_by_export
                    .insert(reservation.pending.export_id, authorization)
                    .is_some()
            {
                return Err(StudioError::JournalCorrupt);
            }
        }
        Ok(Self {
            renderer,
            sessions: BTreeMap::new(),
            recovered: recovered_by_export,
            active_journals: BTreeMap::new(),
            receipts: BTreeMap::new(),
            command_history: BTreeMap::new(),
            maximum_buffered_events,
        })
    }

    pub fn into_renderer(self) -> R {
        self.renderer
    }

    pub fn start(
        &mut self,
        dispatch: AuthorizedRenderDispatch,
    ) -> Result<RenderSessionState, StudioError> {
        let AuthorizedRenderDispatch {
            ticket,
            authorization,
        } = dispatch;
        if let Some(receipt) = self.receipts.get(&ticket.operation_id) {
            return if receipt.project_id == ticket.project_id
                && receipt.export_id == ticket.export_id
                && receipt.fence == ticket.expected_fence
                && receipt.source_set_digest == ticket.graph.sources.digest()
                && receipt.plan_digest == ticket.graph.edit_plan_digest()
                && receipt.render_spec_digest == ticket.render_spec_digest
                && receipt.profile == ticket.graph.preflight.profile
                && receipt.output_name == ticket.output_name
            {
                Ok(RenderSessionState::Committed)
            } else {
                Err(StudioError::IdempotencyConflict)
            };
        }
        if let Some(recovered) = self
            .recovered
            .values()
            .find(|recovered| recovered.reservation.pending.operation_id == ticket.operation_id)
        {
            return if authorization_matches_ticket(&recovered.reservation, &ticket) {
                Err(StudioError::RecoveredRenderRequiresReconciliation)
            } else {
                Err(StudioError::IdempotencyConflict)
            };
        }
        if let Some(existing) = self.command_history.get(&ticket.operation_id) {
            return if existing.matches_ticket(&ticket) {
                Ok(existing.state)
            } else {
                Err(StudioError::IdempotencyConflict)
            };
        }
        if self
            .command_history
            .values()
            .any(|command| command.export_id == ticket.export_id)
            || self.recovered.contains_key(&ticket.export_id)
        {
            return Err(StudioError::ExportIdReused);
        }
        if self.sessions.values().any(|session| {
            session.project_id == ticket.project_id && session.output_name == ticket.output_name
        }) || self.recovered.values().any(|authorization| {
            authorization.reservation.project_id == ticket.project_id
                && authorization.reservation.pending.output_name == ticket.output_name
        }) {
            return Err(StudioError::OutputTargetBusy);
        }
        if self
            .sessions
            .len()
            .checked_add(self.recovered.len())
            .is_none_or(|count| count >= MAX_STUDIO_RENDER_SESSIONS)
        {
            return Err(StudioError::RenderConcurrencyLimit);
        }
        let reserved_receipts = self
            .sessions
            .values()
            .filter(|session| session.state == RenderSessionState::Running)
            .count();
        if self
            .receipts
            .len()
            .checked_add(reserved_receipts)
            .and_then(|reserved| reserved.checked_add(self.recovered.len()))
            .is_none_or(|reserved| reserved >= MAX_STUDIO_RECEIPTS)
        {
            return Err(StudioError::DocumentTooLarge);
        }
        if self.command_history.len() == MAX_STUDIO_RECEIPTS {
            return Err(StudioError::DocumentTooLarge);
        }
        let capabilities = self.renderer.capabilities()?;
        validate_selected_preflight(ticket.graph.preflight, &capabilities)?;
        let project_id = ticket.project_id;
        let export_id = ticket.export_id;
        let operation_id = ticket.operation_id;
        let fence = ticket.expected_fence;
        let output_name = ticket.output_name.clone();
        let source_set_digest = ticket.graph.sources.digest();
        let plan_digest = ticket.graph.edit_plan_digest();
        let render_spec_digest = ticket.render_spec_digest;
        let profile = ticket.graph.preflight.profile;
        let backend = ticket.graph.preflight.selected_backend;
        let deadline = ticket.deadline;
        self.command_history.insert(
            operation_id,
            RenderCommandRecord {
                project_id,
                export_id,
                operation_id,
                fence,
                source_set_digest,
                plan_digest,
                render_spec_digest,
                profile,
                backend,
                output_name: output_name.clone(),
                deadline,
                state: RenderSessionState::Failed,
            },
        );
        self.sessions.insert(
            export_id,
            RenderSession {
                project_id,
                operation_id,
                fence,
                source_set_digest,
                plan_digest,
                render_spec_digest,
                profile,
                output_name: output_name.clone(),
                deadline,
                last_sequence: 0,
                progress_basis_points: 0,
                state: RenderSessionState::Running,
                backend,
                hardware_failure: false,
                current_phase: None,
                failure_code: None,
                release_safe: false,
                committed_output: None,
                events: VecDeque::new(),
            },
        );
        if self
            .active_journals
            .insert(export_id, authorization)
            .is_some()
        {
            self.sessions.remove(&export_id);
            self.command_history.remove(&operation_id);
            return Err(StudioError::JournalCorrupt);
        }
        let outcome = match self.renderer.start(ticket) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.mark_session_failed(export_id)?;
                return Err(error);
            }
        };
        if outcome == RenderStartOutcome::Accepted {
            self.persist_render_boundary(export_id, JournalBoundary::RenderRunning, None)?;
            self.command_history
                .get_mut(&operation_id)
                .ok_or(StudioError::JournalCorrupt)?
                .state = RenderSessionState::Running;
            return Ok(RenderSessionState::Running);
        }
        let postcondition = match self.renderer.probe(export_id) {
            Ok(postcondition) => postcondition,
            Err(error) => {
                self.mark_session_failed(export_id)?;
                return Err(error);
            }
        };
        match postcondition {
            RenderPostcondition::Running {
                fence: running_fence,
                render_spec_digest: running_spec,
            } if running_fence == fence && running_spec == render_spec_digest => {
                self.persist_render_boundary(export_id, JournalBoundary::RenderRunning, None)?;
                self.command_history
                    .get_mut(&operation_id)
                    .ok_or(StudioError::JournalCorrupt)?
                    .state = RenderSessionState::Running;
                Ok(RenderSessionState::Running)
            }
            RenderPostcondition::Committed {
                fence: committed_fence,
                render_spec_digest: committed_spec,
                output_checksum,
                output_bytes,
            } if committed_fence == fence
                && committed_spec == render_spec_digest
                && output_bytes > 0 =>
            {
                if self.receipts.len() == MAX_STUDIO_RECEIPTS {
                    return Err(StudioError::DocumentTooLarge);
                }
                let receipt = RenderReceipt {
                    project_id,
                    export_id,
                    operation_id,
                    fence,
                    source_set_digest,
                    plan_digest,
                    render_spec_digest,
                    profile,
                    output_name: output_name.clone(),
                    output_checksum,
                    output_bytes,
                };
                self.persist_render_boundary(export_id, JournalBoundary::RenderRunning, None)?;
                self.persist_render_boundary(export_id, JournalBoundary::RenderFinalizing, None)?;
                self.persist_render_boundary(
                    export_id,
                    JournalBoundary::RenderCommitted,
                    Some(receipt.clone()),
                )?;
                self.receipts.insert(operation_id, receipt);
                let session = self
                    .sessions
                    .get_mut(&export_id)
                    .ok_or(StudioError::UnknownExport)?;
                session.state = RenderSessionState::Committed;
                session.release_safe = true;
                session.progress_basis_points = 10_000;
                session.current_phase = Some(RenderPhase::Finalizing);
                session.committed_output = Some((output_checksum, output_bytes));
                self.command_history
                    .get_mut(&operation_id)
                    .ok_or(StudioError::JournalCorrupt)?
                    .state = RenderSessionState::Committed;
                Ok(RenderSessionState::Committed)
            }
            RenderPostcondition::Running { .. } => {
                self.mark_session_failed(export_id)?;
                Err(StudioError::StaleRenderCallback)
            }
            RenderPostcondition::Partial {
                fence: partial_fence,
                render_spec_digest: partial_spec,
            } if partial_fence == fence && partial_spec == render_spec_digest => {
                if let Err(error) = self.renderer.cleanup_partial(
                    export_id,
                    fence,
                    render_spec_digest,
                    &output_name,
                ) {
                    self.mark_session_failed(export_id)?;
                    return Err(error);
                }
                let cleaned = match self.renderer.probe(export_id) {
                    Ok(postcondition) => postcondition,
                    Err(error) => {
                        self.mark_session_failed(export_id)?;
                        return Err(error);
                    }
                };
                if cleaned != RenderPostcondition::Absent {
                    self.mark_session_failed(export_id)?;
                    return Err(StudioError::PartialCleanupUnconfirmed);
                }
                self.mark_session_cancelled_and_safe(export_id)?;
                Err(StudioError::AmbiguousRenderStart)
            }
            RenderPostcondition::Absent => {
                // A lost start acknowledgement followed by absence is not a
                // cancellation proof: the renderer may publish later. Keep
                // ownership quarantined until an exact fenced cancel, cleanup,
                // and second absence probe all succeed.
                self.mark_session_failed(export_id)?;
                Err(StudioError::AmbiguousRenderStart)
            }
            RenderPostcondition::Partial { .. } | RenderPostcondition::Committed { .. } => {
                self.mark_session_failed(export_id)?;
                Err(StudioError::AmbiguousRenderStart)
            }
        }
    }

    fn mark_session_failed(&mut self, export_id: StudioExportId) -> Result<(), StudioError> {
        self.persist_render_boundary(export_id, JournalBoundary::FailedRecoverably, None)?;
        let operation_id = {
            let session = self
                .sessions
                .get_mut(&export_id)
                .ok_or(StudioError::UnknownExport)?;
            session.state = RenderSessionState::Failed;
            session.release_safe = false;
            session.operation_id
        };
        self.command_history
            .get_mut(&operation_id)
            .ok_or(StudioError::JournalCorrupt)?
            .state = RenderSessionState::Failed;
        Ok(())
    }

    fn mark_session_cancelled_and_safe(
        &mut self,
        export_id: StudioExportId,
    ) -> Result<(), StudioError> {
        self.persist_render_boundary(export_id, JournalBoundary::RenderCancelled, None)?;
        let operation_id = {
            let session = self
                .sessions
                .get_mut(&export_id)
                .ok_or(StudioError::UnknownExport)?;
            session.state = RenderSessionState::Cancelled;
            session.release_safe = true;
            session.operation_id
        };
        self.command_history
            .get_mut(&operation_id)
            .ok_or(StudioError::JournalCorrupt)?
            .state = RenderSessionState::Cancelled;
        Ok(())
    }

    fn persist_render_boundary(
        &mut self,
        export_id: StudioExportId,
        boundary: JournalBoundary,
        terminal_receipt: Option<RenderReceipt>,
    ) -> Result<(), StudioError> {
        self.active_journals
            .get_mut(&export_id)
            .ok_or(StudioError::RenderReservationRequired)?
            .advance(boundary, terminal_receipt)
    }

    pub fn reconcile_recovered_cleanup(
        &mut self,
        export_id: StudioExportId,
        deadline: Duration,
    ) -> Result<RenderSessionState, StudioError> {
        if deadline.is_zero() || deadline > MAX_STUDIO_RENDER_POLL_WAIT {
            return Err(StudioError::InvalidRenderPollWait);
        }
        let reservation = self
            .recovered
            .get(&export_id)
            .map(|authorization| authorization.reservation.clone())
            .ok_or(StudioError::UnknownExport)?;
        let postcondition = self.renderer.probe(export_id)?;
        match postcondition {
            RenderPostcondition::Absent => {}
            RenderPostcondition::Running {
                fence,
                render_spec_digest,
            } if fence == reservation.fence()
                && render_spec_digest == reservation.pending.render_spec_digest => {}
            RenderPostcondition::Partial {
                fence,
                render_spec_digest,
            } if fence == reservation.fence()
                && render_spec_digest == reservation.pending.render_spec_digest => {}
            RenderPostcondition::Committed {
                fence,
                render_spec_digest,
                ..
            } if fence == reservation.fence()
                && render_spec_digest == reservation.pending.render_spec_digest =>
            {
                return Err(StudioError::CommittedRenderCannotBeCancelled);
            }
            RenderPostcondition::Running { .. }
            | RenderPostcondition::Partial { .. }
            | RenderPostcondition::Committed { .. } => {
                return Err(StudioError::StaleRenderCallback);
            }
        }
        // Even an initial absence is only an observation. A renderer whose
        // start acknowledgement was lost may publish after that probe, so
        // recovered ownership is released only after an exact fenced cancel,
        // exact cleanup, and a second absence proof.
        self.renderer
            .cancel(export_id, reservation.fence(), deadline)?;
        self.renderer.cleanup_partial(
            export_id,
            reservation.fence(),
            reservation.pending.render_spec_digest,
            &reservation.pending.output_name,
        )?;
        if self.renderer.probe(export_id)? != RenderPostcondition::Absent {
            return Err(StudioError::PartialCleanupUnconfirmed);
        }
        self.recovered
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?
            .advance(JournalBoundary::RenderCancelled, None)?;
        self.install_recovered_session(export_id, RenderSessionState::Cancelled, None)?;
        Ok(RenderSessionState::Cancelled)
    }

    /// Adopts only an exact terminal receipt bound into the durable journal.
    /// If recovery observes a matching committed renderer postcondition before
    /// the prior process journaled it, this method first CAS-persists the full
    /// receipt and only then installs a releasable session. Caller-supplied
    /// checksums or lengths are intentionally absent.
    pub fn adopt_recovered_commit(
        &mut self,
        export_id: StudioExportId,
    ) -> Result<RenderSessionState, StudioError> {
        let mut reservation = self
            .recovered
            .get(&export_id)
            .map(|authorization| authorization.reservation.clone())
            .ok_or(StudioError::UnknownExport)?;
        let observed = self.renderer.probe(export_id)?;
        let expected = if reservation.boundary == JournalBoundary::RenderCommitted {
            reservation
                .pending
                .terminal_receipt
                .clone()
                .ok_or(StudioError::RenderPostconditionMismatch)?
        } else {
            let RenderPostcondition::Committed {
                fence,
                render_spec_digest,
                output_checksum,
                output_bytes,
            } = observed.clone()
            else {
                return Err(StudioError::RenderPostconditionMismatch);
            };
            if fence != reservation.fence()
                || render_spec_digest != reservation.pending.render_spec_digest
                || output_bytes == 0
                || !matches!(
                    reservation.boundary,
                    JournalBoundary::RenderPrepared
                        | JournalBoundary::RenderRunning
                        | JournalBoundary::RenderFinalizing
                        | JournalBoundary::FailedRecoverably
                )
            {
                return Err(StudioError::RenderPostconditionMismatch);
            }
            let receipt = RenderReceipt {
                project_id: reservation.project_id,
                export_id,
                operation_id: reservation.pending.operation_id,
                fence: reservation.fence(),
                source_set_digest: reservation.pending.source_set_digest,
                plan_digest: reservation.pending.plan_digest,
                render_spec_digest: reservation.pending.render_spec_digest,
                profile: ExportProfileSpec::approved(reservation.pending.profile).validate()?,
                output_name: reservation.pending.output_name.clone(),
                output_checksum,
                output_bytes,
            };
            let authorization = self
                .recovered
                .get_mut(&export_id)
                .ok_or(StudioError::UnknownExport)?;
            if reservation.boundary == JournalBoundary::RenderPrepared {
                authorization.advance(JournalBoundary::RenderRunning, None)?;
                reservation.boundary = JournalBoundary::RenderRunning;
            }
            if reservation.boundary == JournalBoundary::RenderRunning {
                authorization.advance(JournalBoundary::RenderFinalizing, None)?;
            }
            authorization.advance(JournalBoundary::RenderCommitted, Some(receipt.clone()))?;
            receipt
        };
        if observed
            != (RenderPostcondition::Committed {
                fence: reservation.fence(),
                render_spec_digest: reservation.pending.render_spec_digest,
                output_checksum: expected.output_checksum,
                output_bytes: expected.output_bytes,
            })
        {
            return Err(StudioError::RenderPostconditionMismatch);
        }
        if self.receipts.len() == MAX_STUDIO_RECEIPTS {
            return Err(StudioError::DocumentTooLarge);
        }
        self.receipts
            .insert(reservation.pending.operation_id, expected.clone());
        self.install_recovered_session(
            export_id,
            RenderSessionState::Committed,
            Some((expected.output_checksum, expected.output_bytes)),
        )?;
        Ok(RenderSessionState::Committed)
    }

    fn install_recovered_session(
        &mut self,
        export_id: StudioExportId,
        state: RenderSessionState,
        committed_output: Option<(AssetChecksum, u64)>,
    ) -> Result<(), StudioError> {
        let authorization = self
            .recovered
            .remove(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        let reservation = authorization.reservation.clone();
        let profile = ExportProfileSpec::approved(reservation.pending.profile).validate()?;
        if export_id != reservation.pending.export_id {
            return Err(StudioError::JournalCorrupt);
        }
        let operation_id = reservation.pending.operation_id;
        if self.sessions.contains_key(&export_id)
            || self.command_history.contains_key(&operation_id)
            || !matches!(
                state,
                RenderSessionState::Cancelled | RenderSessionState::Committed
            )
        {
            return Err(StudioError::JournalCorrupt);
        }
        self.command_history.insert(
            operation_id,
            RenderCommandRecord {
                project_id: reservation.project_id,
                export_id,
                operation_id,
                fence: reservation.fence(),
                source_set_digest: reservation.pending.source_set_digest,
                plan_digest: reservation.pending.plan_digest,
                render_spec_digest: reservation.pending.render_spec_digest,
                profile,
                backend: EncoderBackend::Software,
                output_name: reservation.pending.output_name.clone(),
                deadline: Duration::ZERO,
                state,
            },
        );
        self.sessions.insert(
            export_id,
            RenderSession {
                project_id: reservation.project_id,
                operation_id,
                fence: reservation.fence(),
                source_set_digest: reservation.pending.source_set_digest,
                plan_digest: reservation.pending.plan_digest,
                render_spec_digest: reservation.pending.render_spec_digest,
                profile,
                output_name: reservation.pending.output_name,
                deadline: Duration::ZERO,
                last_sequence: 0,
                progress_basis_points: if state == RenderSessionState::Committed {
                    10_000
                } else {
                    0
                },
                state,
                backend: EncoderBackend::Software,
                hardware_failure: false,
                current_phase: (state == RenderSessionState::Committed)
                    .then_some(RenderPhase::Finalizing),
                failure_code: None,
                release_safe: true,
                committed_output,
                events: VecDeque::new(),
            },
        );
        if self
            .active_journals
            .insert(export_id, authorization)
            .is_some()
        {
            return Err(StudioError::JournalCorrupt);
        }
        Ok(())
    }

    pub fn poll(
        &mut self,
        export_id: StudioExportId,
        wait: Duration,
    ) -> Result<RenderSessionState, StudioError> {
        if wait > MAX_STUDIO_RENDER_POLL_WAIT {
            return Err(StudioError::InvalidRenderPollWait);
        }
        let (current, release_safe) = self
            .sessions
            .get(&export_id)
            .map(|session| (session.state, session.release_safe))
            .ok_or(StudioError::UnknownExport)?;
        if current != RenderSessionState::Running {
            if matches!(
                current,
                RenderSessionState::Cancelled | RenderSessionState::Failed
            ) && !release_safe
            {
                return Err(StudioError::PartialCleanupUnconfirmed);
            }
            self.reconcile_terminal(export_id, current)?;
            return self
                .sessions
                .get(&export_id)
                .map(|session| session.state)
                .ok_or(StudioError::UnknownExport);
        }
        let events = self
            .renderer
            .poll(export_id, self.maximum_buffered_events, wait)?;
        if events.len() > self.maximum_buffered_events {
            return Err(StudioError::RendererEventOverflow);
        }
        for event in events {
            self.apply_event(export_id, event)?;
            let state = self
                .sessions
                .get(&export_id)
                .map(|session| session.state)
                .ok_or(StudioError::UnknownExport)?;
            if state != RenderSessionState::Running {
                self.reconcile_terminal(export_id, state)?;
            }
        }
        let state = self
            .sessions
            .get(&export_id)
            .map(|session| session.state)
            .ok_or(StudioError::UnknownExport)?;
        self.reconcile_terminal(export_id, state)?;
        self.sessions
            .get(&export_id)
            .map(|session| session.state)
            .ok_or(StudioError::UnknownExport)
    }

    pub fn enforce_deadline(
        &mut self,
        export_id: StudioExportId,
        elapsed: Duration,
        cleanup_deadline: Duration,
    ) -> Result<RenderSessionState, StudioError> {
        let session = self
            .sessions
            .get(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        if session.state != RenderSessionState::Running || elapsed < session.deadline {
            return Ok(session.state);
        }
        self.cancel_and_cleanup(export_id, cleanup_deadline)?;
        Err(StudioError::RenderDeadlineExceeded)
    }

    fn apply_event(
        &mut self,
        export_id: StudioExportId,
        event: RenderEvent,
    ) -> Result<(), StudioError> {
        let session = self
            .sessions
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        let expected_sequence = session
            .last_sequence
            .checked_add(1)
            .ok_or(StudioError::StaleRenderCallback)?;
        if event.export_id != export_id
            || event.project_id != session.project_id
            || event.fence != session.fence
            || event.render_spec_digest != session.render_spec_digest
            || event.sequence != expected_sequence
            || session.state != RenderSessionState::Running
        {
            return Err(StudioError::StaleRenderCallback);
        }
        match &event.kind {
            RenderEventKind::Progress {
                phase,
                basis_points,
            } => {
                if *basis_points > 10_000
                    || *basis_points < session.progress_basis_points
                    || session
                        .current_phase
                        .is_some_and(|current| phase.rank() < current.rank())
                {
                    return Err(StudioError::NonMonotonicProgress);
                }
                session.current_phase = Some(*phase);
                session.progress_basis_points = *basis_points;
            }
            RenderEventKind::Committed {
                output_checksum,
                output_bytes,
            } => {
                if session.progress_basis_points != 10_000 || *output_bytes == 0 {
                    return Err(StudioError::PrematureRenderCommit);
                }
                session.state = RenderSessionState::Committed;
                session.committed_output = Some((*output_checksum, *output_bytes));
            }
            RenderEventKind::Cancelled => session.state = RenderSessionState::Cancelled,
            RenderEventKind::Failed {
                safe_code,
                hardware_failure,
            } => {
                if !valid_safe_render_code(safe_code) {
                    return Err(StudioError::InvalidRenderFailureCode);
                }
                session.state = RenderSessionState::Failed;
                session.hardware_failure = *hardware_failure;
                session.failure_code = Some(*safe_code);
            }
        }
        session.last_sequence = event.sequence;
        if session.events.len() == self.maximum_buffered_events {
            session.events.pop_front();
        }
        session.events.push_back(event);
        let operation_id = session.operation_id;
        let state = session.state;
        self.command_history
            .get_mut(&operation_id)
            .ok_or(StudioError::JournalCorrupt)?
            .state = state;
        Ok(())
    }

    fn reconcile_terminal(
        &mut self,
        export_id: StudioExportId,
        state: RenderSessionState,
    ) -> Result<(), StudioError> {
        match state {
            RenderSessionState::Running => Ok(()),
            RenderSessionState::Committed => {
                let (
                    operation_id,
                    project_id,
                    fence,
                    source_set_digest,
                    plan_digest,
                    render_spec_digest,
                    profile,
                    output_name,
                    committed_output,
                    release_safe,
                ) = {
                    let session = self
                        .sessions
                        .get(&export_id)
                        .ok_or(StudioError::UnknownExport)?;
                    (
                        session.operation_id,
                        session.project_id,
                        session.fence,
                        session.source_set_digest,
                        session.plan_digest,
                        session.render_spec_digest,
                        session.profile,
                        session.output_name.clone(),
                        session.committed_output,
                        session.release_safe,
                    )
                };
                if release_safe && self.receipts.contains_key(&operation_id) {
                    return Ok(());
                }
                let (output_checksum, output_bytes) =
                    committed_output.ok_or(StudioError::PrematureRenderCommit)?;
                let postcondition = self.renderer.probe(export_id)?;
                if postcondition
                    != (RenderPostcondition::Committed {
                        fence,
                        render_spec_digest,
                        output_checksum,
                        output_bytes,
                    })
                {
                    self.mark_session_failed(export_id)?;
                    return Err(StudioError::RenderPostconditionMismatch);
                }
                if self.receipts.len() == MAX_STUDIO_RECEIPTS
                    && !self.receipts.contains_key(&operation_id)
                {
                    return Err(StudioError::DocumentTooLarge);
                }
                let receipt = RenderReceipt {
                    project_id,
                    export_id,
                    operation_id,
                    fence,
                    source_set_digest,
                    plan_digest,
                    render_spec_digest,
                    profile,
                    output_name,
                    output_checksum,
                    output_bytes,
                };
                let boundary = self
                    .active_journals
                    .get(&export_id)
                    .ok_or(StudioError::RenderReservationRequired)?
                    .reservation
                    .boundary;
                if boundary == JournalBoundary::RenderRunning {
                    self.persist_render_boundary(
                        export_id,
                        JournalBoundary::RenderFinalizing,
                        None,
                    )?;
                }
                self.persist_render_boundary(
                    export_id,
                    JournalBoundary::RenderCommitted,
                    Some(receipt.clone()),
                )?;
                self.receipts.insert(operation_id, receipt);
                self.sessions
                    .get_mut(&export_id)
                    .ok_or(StudioError::UnknownExport)?
                    .release_safe = true;
                Ok(())
            }
            RenderSessionState::Cancelled | RenderSessionState::Failed => {
                let session = self
                    .sessions
                    .get(&export_id)
                    .ok_or(StudioError::UnknownExport)?;
                if session.release_safe {
                    return Ok(());
                }
                let fence = session.fence;
                let render_spec_digest = session.render_spec_digest;
                let output_name = session.output_name.clone();
                self.renderer.cleanup_partial(
                    export_id,
                    fence,
                    render_spec_digest,
                    &output_name,
                )?;
                if self.renderer.probe(export_id)? != RenderPostcondition::Absent {
                    return Err(StudioError::PartialCleanupUnconfirmed);
                }
                self.persist_render_boundary(export_id, JournalBoundary::RenderCancelled, None)?;
                self.sessions
                    .get_mut(&export_id)
                    .ok_or(StudioError::UnknownExport)?
                    .release_safe = true;
                Ok(())
            }
        }
    }

    pub fn cancel_and_cleanup(
        &mut self,
        export_id: StudioExportId,
        deadline: Duration,
    ) -> Result<(), StudioError> {
        if deadline.is_zero() || deadline > MAX_STUDIO_RENDER_POLL_WAIT {
            return Err(StudioError::InvalidRenderPollWait);
        }
        let session = self
            .sessions
            .get(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        if session.state == RenderSessionState::Committed {
            return Err(StudioError::CommittedRenderCannotBeCancelled);
        }
        let fence = session.fence;
        let render_spec_digest = session.render_spec_digest;
        let output_name = session.output_name.clone();
        self.renderer.cancel(export_id, fence, deadline)?;
        self.renderer
            .cleanup_partial(export_id, fence, render_spec_digest, &output_name)?;
        if self.renderer.probe(export_id)? != RenderPostcondition::Absent {
            return Err(StudioError::PartialCleanupUnconfirmed);
        }
        self.persist_render_boundary(export_id, JournalBoundary::RenderCancelled, None)?;
        let session = self
            .sessions
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        session.state = RenderSessionState::Cancelled;
        session.release_safe = true;
        self.command_history
            .get_mut(&session.operation_id)
            .ok_or(StudioError::JournalCorrupt)?
            .state = RenderSessionState::Cancelled;
        Ok(())
    }

    pub fn retry_hardware_failure_with_software(
        &mut self,
        failed_export_id: StudioExportId,
        deadline: Duration,
        replacement: AuthorizedRenderDispatch,
    ) -> Result<RenderSessionState, StudioError> {
        let failed = self
            .sessions
            .get(&failed_export_id)
            .cloned()
            .ok_or(StudioError::UnknownExport)?;
        if failed.state != RenderSessionState::Failed
            || !failed.hardware_failure
            || failed.backend != EncoderBackend::Hardware
            || !failed.release_safe
            || replacement.graph.preflight.selected_backend != EncoderBackend::Software
            || replacement.graph.sources.digest() != failed.source_set_digest
            || replacement.graph.edit_plan_digest() != failed.plan_digest
            || replacement.render_spec_digest != failed.render_spec_digest
            || replacement.graph.preflight.profile != failed.profile
            || replacement.project_id != failed.project_id
            || replacement.expected_fence != failed.fence
            || replacement.export_id == failed_export_id
            || replacement.operation_id == failed.operation_id
        {
            return Err(StudioError::InvalidHardwareFallback);
        }
        if deadline.is_zero() || deadline > MAX_STUDIO_RENDER_POLL_WAIT {
            return Err(StudioError::InvalidRenderPollWait);
        }
        let replacement_export_id = replacement.export_id;
        self.sessions.remove(&failed_export_id);
        let result = self.start(replacement);
        if result.is_ok() {
            self.active_journals.remove(&failed_export_id);
        } else if !self.sessions.contains_key(&replacement_export_id) {
            self.sessions.insert(failed_export_id, failed);
        }
        result
    }

    pub fn progress(
        &self,
        export_id: StudioExportId,
    ) -> Result<RenderProgressSnapshot, StudioError> {
        let session = self
            .sessions
            .get(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        Ok(RenderProgressSnapshot {
            state: session.state,
            phase: session.current_phase,
            basis_points: session.progress_basis_points,
            last_sequence: session.last_sequence,
            failure_code: session.failure_code,
        })
    }

    pub fn drain_events(
        &mut self,
        export_id: StudioExportId,
        maximum_events: usize,
    ) -> Result<Vec<RenderEvent>, StudioError> {
        if maximum_events == 0 || maximum_events > self.maximum_buffered_events {
            return Err(StudioError::UnboundedRendererEvents);
        }
        let session = self
            .sessions
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        let count = maximum_events.min(session.events.len());
        Ok(session.events.drain(..count).collect())
    }

    pub fn release_terminal(
        &mut self,
        export_id: StudioExportId,
    ) -> Result<RenderSessionState, StudioError> {
        let session = self
            .sessions
            .get(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        if session.state == RenderSessionState::Running {
            return Err(StudioError::ActiveRenderCannotBeReleased);
        }
        if !session.release_safe {
            return Err(StudioError::PartialCleanupUnconfirmed);
        }
        if session.state == RenderSessionState::Committed
            && self
                .receipts
                .get(&session.operation_id)
                .is_none_or(|receipt| {
                    receipt.project_id != session.project_id
                        || receipt.export_id != export_id
                        || receipt.fence != session.fence
                        || receipt.source_set_digest != session.source_set_digest
                        || receipt.plan_digest != session.plan_digest
                        || receipt.render_spec_digest != session.render_spec_digest
                        || receipt.profile != session.profile
                        || receipt.output_name != session.output_name
                        || session.committed_output
                            != Some((receipt.output_checksum, receipt.output_bytes))
                })
        {
            return Err(StudioError::RenderPostconditionMismatch);
        }
        let state = session.state;
        self.sessions.remove(&export_id);
        self.active_journals.remove(&export_id);
        Ok(state)
    }

    #[must_use]
    pub fn receipt(&self, operation_id: StudioOperationId) -> Option<&RenderReceipt> {
        self.receipts.get(&operation_id)
    }
}
