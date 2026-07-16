use std::{collections::BTreeMap, fmt};

use thiserror::Error;

pub const STUDIO_PROJECT_VERSION: u16 = 1;
pub const STUDIO_EDIT_VERSION: u16 = 1;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Screen,
    Camera,
    Microphone,
    SystemAudio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioAsset {
    pub id: String,
    pub track: TrackKind,
    pub start_ns: u64,
    pub duration_ns: u64,
    pub checksum_sha256: String,
}

impl StudioAsset {
    pub fn validate(self) -> Result<Self, StudioError> {
        if !valid_identifier(&self.id)
            || self.duration_ns == 0
            || self.start_ns.checked_add(self.duration_ns).is_none()
        {
            return Err(StudioError::InvalidAsset);
        }
        if !valid_checksum(&self.checksum_sha256) {
            return Err(StudioError::InvalidChecksum);
        }
        Ok(self)
    }

    #[must_use]
    pub const fn end_ns(&self) -> u64 {
        self.start_ns.saturating_add(self.duration_ns)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPreset {
    ScreenOnly,
    CameraBubble,
    SideBySide,
    CameraFull,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    Trim {
        start_ns: u64,
        end_ns: u64,
    },
    Split {
        at_ns: u64,
    },
    DeleteRange {
        start_ns: u64,
        end_ns: u64,
    },
    Speed {
        start_ns: u64,
        end_ns: u64,
        rate_milli: u16,
    },
    AudioGain {
        track: TrackKind,
        start_ns: u64,
        end_ns: u64,
        gain_millibels: i32,
    },
    Layout {
        start_ns: u64,
        end_ns: u64,
        preset: LayoutPreset,
    },
}

impl EditOperation {
    fn range(&self) -> Option<(u64, u64)> {
        match self {
            Self::Trim { start_ns, end_ns }
            | Self::DeleteRange { start_ns, end_ns }
            | Self::Speed {
                start_ns, end_ns, ..
            }
            | Self::AudioGain {
                start_ns, end_ns, ..
            }
            | Self::Layout {
                start_ns, end_ns, ..
            } => Some((*start_ns, *end_ns)),
            Self::Split { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSpec {
    pub version: u16,
    pub operations: Vec<EditOperation>,
}

impl Default for EditSpec {
    fn default() -> Self {
        Self {
            version: STUDIO_EDIT_VERSION,
            operations: Vec::new(),
        }
    }
}

impl EditSpec {
    pub fn compile(&self, source_duration_ns: u64) -> Result<CompiledEdit, StudioError> {
        if self.version != STUDIO_EDIT_VERSION {
            return Err(StudioError::UnsupportedEditVersion(self.version));
        }
        if source_duration_ns == 0 {
            return Err(StudioError::NoTimeline);
        }

        let trims: Vec<_> = self
            .operations
            .iter()
            .filter_map(|operation| match operation {
                EditOperation::Trim { start_ns, end_ns } => Some((*start_ns, *end_ns)),
                _ => None,
            })
            .collect();
        if trims.len() > 1 {
            return Err(StudioError::MultipleTrims);
        }
        let active = trims.first().copied().unwrap_or((0, source_duration_ns));
        validate_range(active.0, active.1, source_duration_ns)?;

        let mut duration_changing_ranges = Vec::new();
        let mut output_duration = active.1 - active.0;
        for operation in &self.operations {
            match operation {
                EditOperation::Trim { .. } => {}
                EditOperation::Split { at_ns } => {
                    if *at_ns <= active.0 || *at_ns >= active.1 {
                        return Err(StudioError::EditOutsideTimeline);
                    }
                }
                EditOperation::Speed {
                    start_ns,
                    end_ns,
                    rate_milli,
                } => {
                    validate_active_range(*start_ns, *end_ns, active)?;
                    if !(250..=4_000).contains(rate_milli) {
                        return Err(StudioError::InvalidSpeed);
                    }
                    ensure_non_overlapping(&duration_changing_ranges, (*start_ns, *end_ns))?;
                    duration_changing_ranges.push((*start_ns, *end_ns));
                    let source = end_ns - start_ns;
                    let rendered = source
                        .checked_mul(1_000)
                        .ok_or(StudioError::TimelineOverflow)?
                        / u64::from(*rate_milli);
                    output_duration = output_duration
                        .checked_sub(source)
                        .and_then(|duration| duration.checked_add(rendered))
                        .ok_or(StudioError::TimelineOverflow)?;
                }
                EditOperation::DeleteRange { start_ns, end_ns } => {
                    validate_active_range(*start_ns, *end_ns, active)?;
                    ensure_non_overlapping(&duration_changing_ranges, (*start_ns, *end_ns))?;
                    duration_changing_ranges.push((*start_ns, *end_ns));
                    output_duration = output_duration
                        .checked_sub(end_ns - start_ns)
                        .ok_or(StudioError::TimelineOverflow)?;
                }
                EditOperation::AudioGain {
                    start_ns,
                    end_ns,
                    gain_millibels,
                    ..
                } => {
                    validate_active_range(*start_ns, *end_ns, active)?;
                    if !(-9_600..=2_400).contains(gain_millibels) {
                        return Err(StudioError::InvalidGain);
                    }
                }
                EditOperation::Layout {
                    start_ns, end_ns, ..
                } => validate_active_range(*start_ns, *end_ns, active)?,
            }
            if let Some((start, end)) = operation.range() {
                validate_range(start, end, source_duration_ns)?;
            }
        }
        if output_duration == 0 {
            return Err(StudioError::EmptyOutput);
        }

        Ok(CompiledEdit {
            version: self.version,
            active_start_ns: active.0,
            active_end_ns: active.1,
            output_duration_ns: output_duration,
            operations: self.operations.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledEdit {
    pub version: u16,
    pub active_start_ns: u64,
    pub active_end_ns: u64,
    pub output_duration_ns: u64,
    pub operations: Vec<EditOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportProfile {
    Preview,
    DistributionH264AacMp4,
    EditableVp8OpusWebm,
    ArchiveLossless,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPlan {
    pub project_version: u16,
    pub project_revision: u64,
    pub profile: ExportProfile,
    pub source_assets: Vec<StudioAsset>,
    pub edit: CompiledEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupDirective {
    DeletePartialOutput,
}

#[derive(Clone)]
pub struct StudioProject {
    state: StudioState,
    revision: u64,
    recover_to: Option<StudioState>,
    assets: BTreeMap<String, StudioAsset>,
    edits: EditSpec,
    export_progress_basis_points: u16,
    completed_exports: u32,
}

impl fmt::Debug for StudioProject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudioProject")
            .field("state", &self.state)
            .field("revision", &self.revision)
            .field("asset_count", &self.assets.len())
            .field("edit_count", &self.edits.operations.len())
            .field(
                "export_progress_basis_points",
                &self.export_progress_basis_points,
            )
            .field("completed_exports", &self.completed_exports)
            .finish()
    }
}

impl Default for StudioProject {
    fn default() -> Self {
        Self::new()
    }
}

impl StudioProject {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: StudioState::Empty,
            revision: 0,
            recover_to: None,
            assets: BTreeMap::new(),
            edits: EditSpec::default(),
            export_progress_basis_points: 0,
            completed_exports: 0,
        }
    }

    #[must_use]
    pub const fn state(&self) -> StudioState {
        self.state
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn completed_exports(&self) -> u32 {
        self.completed_exports
    }

    #[must_use]
    pub fn edit_spec(&self) -> &EditSpec {
        &self.edits
    }

    pub fn begin_recording(&mut self) -> Result<(), StudioError> {
        self.require_state(&[StudioState::Empty])?;
        self.state = StudioState::Recording;
        self.bump_revision()
    }

    pub fn add_asset(
        &mut self,
        expected_revision: u64,
        asset: StudioAsset,
    ) -> Result<bool, StudioError> {
        self.expect_revision(expected_revision)?;
        self.require_state(&[StudioState::Recording, StudioState::Recovering])?;
        let asset = asset.validate()?;
        if let Some(existing) = self.assets.get(&asset.id) {
            return if existing == &asset {
                Ok(false)
            } else {
                Err(StudioError::AssetConflict)
            };
        }
        self.assets.insert(asset.id.clone(), asset);
        self.bump_revision()?;
        Ok(true)
    }

    pub fn finish_recording(&mut self, expected_revision: u64) -> Result<(), StudioError> {
        self.expect_revision(expected_revision)?;
        self.require_state(&[StudioState::Recording])?;
        self.validate_asset_set()?;
        self.state = StudioState::Editing;
        self.bump_revision()
    }

    pub fn process_crashed(&mut self) -> Result<(), StudioError> {
        self.require_state(&[
            StudioState::Recording,
            StudioState::Editing,
            StudioState::Previewing,
            StudioState::Exporting,
        ])?;
        self.recover_to = Some(match self.state {
            StudioState::Previewing | StudioState::Exporting => StudioState::Editing,
            other => other,
        });
        self.state = StudioState::Recovering;
        self.bump_revision()
    }

    pub fn resume_after_crash(
        &mut self,
        expected_revision: u64,
    ) -> Result<StudioState, StudioError> {
        self.expect_revision(expected_revision)?;
        self.require_state(&[StudioState::Recovering])?;
        let state = self.recover_to.take().ok_or(StudioError::JournalCorrupt)?;
        // A crash immediately after recording starts is recoverable even before
        // the first asset checkpoint exists. All later states require assets.
        if state != StudioState::Recording {
            self.validate_asset_set()?;
        }
        self.state = state;
        self.bump_revision()?;
        Ok(state)
    }

    pub fn replace_edits(
        &mut self,
        expected_revision: u64,
        edits: EditSpec,
    ) -> Result<bool, StudioError> {
        self.expect_revision(expected_revision)?;
        self.require_state(&[StudioState::Editing])?;
        edits.compile(self.source_duration_ns()?)?;
        if self.edits == edits {
            return Ok(false);
        }
        self.edits = edits;
        self.bump_revision()?;
        Ok(true)
    }

    pub fn begin_preview(&mut self, expected_revision: u64) -> Result<RenderPlan, StudioError> {
        self.begin_render(
            expected_revision,
            ExportProfile::Preview,
            StudioState::Previewing,
        )
    }

    pub fn end_preview(&mut self) -> Result<(), StudioError> {
        self.require_state(&[StudioState::Previewing])?;
        self.state = StudioState::Editing;
        self.bump_revision()
    }

    pub fn begin_export(
        &mut self,
        expected_revision: u64,
        profile: ExportProfile,
    ) -> Result<RenderPlan, StudioError> {
        if profile == ExportProfile::Preview {
            return Err(StudioError::InvalidExportProfile);
        }
        self.export_progress_basis_points = 0;
        self.begin_render(expected_revision, profile, StudioState::Exporting)
    }

    pub fn update_export_progress(&mut self, basis_points: u16) -> Result<bool, StudioError> {
        self.require_state(&[StudioState::Exporting])?;
        if basis_points > 10_000 || basis_points < self.export_progress_basis_points {
            return Err(StudioError::NonMonotonicProgress);
        }
        if basis_points == self.export_progress_basis_points {
            return Ok(false);
        }
        self.export_progress_basis_points = basis_points;
        Ok(true)
    }

    pub fn complete_export(&mut self) -> Result<(), StudioError> {
        self.require_state(&[StudioState::Exporting])?;
        if self.export_progress_basis_points != 10_000 {
            return Err(StudioError::ExportIncomplete);
        }
        self.completed_exports = self
            .completed_exports
            .checked_add(1)
            .ok_or(StudioError::JournalCorrupt)?;
        self.export_progress_basis_points = 0;
        self.state = StudioState::Editing;
        self.bump_revision()
    }

    pub fn cancel_export(&mut self) -> Result<CleanupDirective, StudioError> {
        self.require_state(&[StudioState::Exporting])?;
        self.export_progress_basis_points = 0;
        self.state = StudioState::Editing;
        self.bump_revision()?;
        Ok(CleanupDirective::DeletePartialOutput)
    }

    pub fn complete_project(&mut self, expected_revision: u64) -> Result<(), StudioError> {
        self.expect_revision(expected_revision)?;
        self.require_state(&[StudioState::Editing])?;
        self.state = StudioState::Completed;
        self.bump_revision()
    }

    pub fn fail(&mut self) -> Result<(), StudioError> {
        if self.state.is_terminal() {
            return Err(StudioError::InvalidState(self.state));
        }
        self.state = StudioState::Failed;
        self.bump_revision()
    }

    pub fn cancel_project(&mut self) -> Result<bool, StudioError> {
        if self.state == StudioState::Cancelled {
            return Ok(false);
        }
        if self.state.is_terminal() {
            return Err(StudioError::InvalidState(self.state));
        }
        self.state = StudioState::Cancelled;
        self.bump_revision()?;
        Ok(true)
    }

    fn begin_render(
        &mut self,
        expected_revision: u64,
        profile: ExportProfile,
        target_state: StudioState,
    ) -> Result<RenderPlan, StudioError> {
        self.expect_revision(expected_revision)?;
        self.require_state(&[StudioState::Editing])?;
        let edit = self.edits.compile(self.source_duration_ns()?)?;
        let plan = RenderPlan {
            project_version: STUDIO_PROJECT_VERSION,
            project_revision: self.revision,
            profile,
            source_assets: self.assets.values().cloned().collect(),
            edit,
        };
        self.state = target_state;
        self.bump_revision()?;
        Ok(plan)
    }

    fn source_duration_ns(&self) -> Result<u64, StudioError> {
        self.assets
            .values()
            .map(StudioAsset::end_ns)
            .max()
            .filter(|duration| *duration > 0)
            .ok_or(StudioError::NoTimeline)
    }

    fn validate_asset_set(&self) -> Result<(), StudioError> {
        if self.assets.is_empty() {
            return Err(StudioError::NoAssets);
        }
        if self.source_duration_ns()? == 0 {
            return Err(StudioError::NoTimeline);
        }
        Ok(())
    }

    fn require_state(&self, allowed: &[StudioState]) -> Result<(), StudioError> {
        if allowed.contains(&self.state) {
            Ok(())
        } else {
            Err(StudioError::InvalidState(self.state))
        }
    }

    fn expect_revision(&self, expected: u64) -> Result<(), StudioError> {
        if self.revision == expected {
            Ok(())
        } else {
            Err(StudioError::StaleRevision {
                expected,
                actual: self.revision,
            })
        }
    }

    fn bump_revision(&mut self) -> Result<(), StudioError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(StudioError::JournalCorrupt)?;
        Ok(())
    }
}

fn validate_range(start: u64, end: u64, source_duration: u64) -> Result<(), StudioError> {
    if start >= end || end > source_duration {
        Err(StudioError::EditOutsideTimeline)
    } else {
        Ok(())
    }
}

fn validate_active_range(start: u64, end: u64, active: (u64, u64)) -> Result<(), StudioError> {
    if start < active.0 || start >= end || end > active.1 {
        Err(StudioError::EditOutsideTimeline)
    } else {
        Ok(())
    }
}

fn ensure_non_overlapping(
    existing: &[(u64, u64)],
    candidate: (u64, u64),
) -> Result<(), StudioError> {
    if existing
        .iter()
        .any(|range| candidate.0 < range.1 && range.0 < candidate.1)
    {
        Err(StudioError::OverlappingDurationEdits)
    } else {
        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn valid_checksum(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StudioError {
    #[error("Studio operation is invalid while project is {0:?}")]
    InvalidState(StudioState),
    #[error("project revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("Studio asset metadata is invalid")]
    InvalidAsset,
    #[error("Studio asset checksum must be a SHA-256 hex digest")]
    InvalidChecksum,
    #[error("Studio asset ID conflicts with an existing asset")]
    AssetConflict,
    #[error("Studio project has no assets")]
    NoAssets,
    #[error("Studio project has no renderable timeline")]
    NoTimeline,
    #[error("Studio project journal is corrupt or overflowed")]
    JournalCorrupt,
    #[error("unsupported edit version {0}")]
    UnsupportedEditVersion(u16),
    #[error("only one trim operation is permitted")]
    MultipleTrims,
    #[error("edit is outside the active source timeline")]
    EditOutsideTimeline,
    #[error("speed must be between 0.25x and 4x")]
    InvalidSpeed,
    #[error("audio gain is outside the safe range")]
    InvalidGain,
    #[error("duration-changing edits cannot overlap")]
    OverlappingDurationEdits,
    #[error("edit would overflow the timeline")]
    TimelineOverflow,
    #[error("edit would produce an empty output")]
    EmptyOutput,
    #[error("preview profile cannot be used for export")]
    InvalidExportProfile,
    #[error("export progress must be monotonic and at most 10000 basis points")]
    NonMonotonicProgress,
    #[error("export cannot complete before reaching 10000 basis points")]
    ExportIncomplete,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, track: TrackKind) -> StudioAsset {
        StudioAsset {
            id: id.into(),
            track,
            start_ns: 0,
            duration_ns: 10_000,
            checksum_sha256: "a".repeat(64),
        }
    }

    fn editable_project() -> StudioProject {
        let mut project = StudioProject::new();
        project.begin_recording().expect("begin");
        project
            .add_asset(project.revision(), asset("screen", TrackKind::Screen))
            .expect("asset");
        project
            .add_asset(project.revision(), asset("mic", TrackKind::Microphone))
            .expect("asset");
        project
            .finish_recording(project.revision())
            .expect("finish");
        project
    }

    #[test]
    fn preview_and_export_share_one_compiled_edit() {
        let mut project = editable_project();
        let edits = EditSpec {
            version: STUDIO_EDIT_VERSION,
            operations: vec![
                EditOperation::Trim {
                    start_ns: 1_000,
                    end_ns: 9_000,
                },
                EditOperation::Speed {
                    start_ns: 2_000,
                    end_ns: 4_000,
                    rate_milli: 2_000,
                },
            ],
        };
        project
            .replace_edits(project.revision(), edits)
            .expect("edits");
        let preview = project.begin_preview(project.revision()).expect("preview");
        project.end_preview().expect("end preview");
        let export = project
            .begin_export(project.revision(), ExportProfile::DistributionH264AacMp4)
            .expect("export");
        assert_eq!(preview.edit, export.edit);
        assert_eq!(preview.edit.output_duration_ns, 7_000);
    }

    #[test]
    fn stale_edit_save_cannot_overwrite_newer_project() {
        let mut project = editable_project();
        let stale = project.revision() - 1;
        assert!(matches!(
            project.replace_edits(stale, EditSpec::default()),
            Err(StudioError::StaleRevision { .. })
        ));
    }

    #[test]
    fn crash_during_export_returns_to_editing() {
        let mut project = editable_project();
        project
            .begin_export(project.revision(), ExportProfile::EditableVp8OpusWebm)
            .expect("export");
        project.process_crashed().expect("crash");
        let revision = project.revision();
        assert_eq!(
            project.resume_after_crash(revision).expect("recover"),
            StudioState::Editing
        );
    }

    #[test]
    fn crash_before_first_asset_can_resume_recording() {
        let mut project = StudioProject::new();
        project.begin_recording().expect("begin");
        project.process_crashed().expect("crash");
        assert_eq!(
            project
                .resume_after_crash(project.revision())
                .expect("recover"),
            StudioState::Recording
        );
    }

    #[test]
    fn completed_export_keeps_the_project_editable() {
        let mut project = editable_project();
        project
            .begin_export(project.revision(), ExportProfile::EditableVp8OpusWebm)
            .expect("export");
        project.update_export_progress(10_000).expect("progress");
        project.complete_export().expect("complete");
        assert_eq!(project.state(), StudioState::Editing);
        assert_eq!(project.completed_exports(), 1);
        project
            .begin_export(project.revision(), ExportProfile::ArchiveLossless)
            .expect("second export");
    }

    #[test]
    fn cancelled_export_requires_partial_cleanup() {
        let mut project = editable_project();
        project
            .begin_export(project.revision(), ExportProfile::ArchiveLossless)
            .expect("export");
        project.update_export_progress(500).expect("progress");
        assert_eq!(
            project.cancel_export().expect("cancel"),
            CleanupDirective::DeletePartialOutput
        );
        assert_eq!(project.state(), StudioState::Editing);
    }
}
