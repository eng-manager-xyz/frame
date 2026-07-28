//! Portable contract between the desktop state machine and native capture.
//!
//! Native platform identities never cross this boundary. Backends mint opaque
//! tokens and return only bounded, privacy-safe target metadata. File paths
//! passed to native export have already been scoped by the IPC path policy.

use std::{collections::HashSet, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ipc::{
    CaptureTargetKind, DeviceClass, ExportProfile, RecorderMode, ValidatedPath, valid_opaque_id,
};

pub const CAPTURE_TARGET_CATALOG_VERSION: u16 = 1;
pub const CAPTURE_ARTIFACT_SUMMARY_VERSION: u16 = 1;
pub const STUDIO_PROJECT_CATALOG_VERSION: u16 = 1;
const MAX_CAPTURE_TARGETS: usize = 256;
pub const MAX_STUDIO_PROJECT_CATALOG_ENTRIES: usize = 256;
const MAX_STUDIO_PROJECT_ASSETS: u16 = 64;
const MAX_CAPTURE_DIMENSION: u32 = 65_535;

/// A versioned catalog containing no display titles or platform identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureTargetCatalog {
    pub schema_version: u16,
    pub generation: u64,
    pub targets: Vec<CaptureTargetSummary>,
}

impl CaptureTargetCatalog {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            schema_version: CAPTURE_TARGET_CATALOG_VERSION,
            generation: 0,
            targets: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), NativeDesktopContractError> {
        if self.schema_version != CAPTURE_TARGET_CATALOG_VERSION {
            return Err(NativeDesktopContractError::UnsupportedCatalogVersion);
        }
        if self.generation == 0 && !self.targets.is_empty() {
            return Err(NativeDesktopContractError::InvalidCatalogGeneration);
        }
        if self.targets.len() > MAX_CAPTURE_TARGETS {
            return Err(NativeDesktopContractError::TooManyCaptureTargets);
        }

        let mut tokens = HashSet::with_capacity(self.targets.len());
        let mut ordinals = HashSet::with_capacity(self.targets.len());
        for target in &self.targets {
            target.validate()?;
            if !tokens.insert(target.token.as_str())
                || !ordinals.insert((target_kind_tag(target.kind), target.ordinal))
            {
                return Err(NativeDesktopContractError::DuplicateCaptureTarget);
            }
        }
        Ok(())
    }

    /// Enumeration results must carry a nonzero generation. Generation zero is
    /// reserved for the valid, empty bootstrap catalog.
    pub fn validate_enumeration(&self) -> Result<(), NativeDesktopContractError> {
        self.validate()?;
        if self.generation == 0 {
            return Err(NativeDesktopContractError::InvalidCatalogGeneration);
        }
        Ok(())
    }
}

/// Coarse capture geometry paired with a backend-minted opaque token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureTargetSummary {
    pub token: String,
    pub kind: CaptureTargetKind,
    pub ordinal: u16,
    pub width_pixels: u32,
    pub height_pixels: u32,
    pub scale_numerator: u16,
    pub scale_denominator: u16,
    pub rotation_degrees: u16,
}

impl CaptureTargetSummary {
    pub fn validate(&self) -> Result<(), NativeDesktopContractError> {
        if !valid_opaque_id(&self.token)
            || self.width_pixels == 0
            || self.height_pixels == 0
            || self.width_pixels > MAX_CAPTURE_DIMENSION
            || self.height_pixels > MAX_CAPTURE_DIMENSION
            || self.scale_numerator == 0
            || self.scale_denominator == 0
            || self.scale_numerator > 4_096
            || self.scale_denominator > 4_096
            || !matches!(self.rotation_degrees, 0 | 90 | 180 | 270)
        {
            return Err(NativeDesktopContractError::InvalidCaptureTarget);
        }
        Ok(())
    }
}

impl fmt::Debug for CaptureTargetSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptureTargetSummary")
            .field("token", &"<redacted>")
            .field("kind", &self.kind)
            .field("ordinal", &self.ordinal)
            .field("width_pixels", &self.width_pixels)
            .field("height_pixels", &self.height_pixels)
            .field("scale_numerator", &self.scale_numerator)
            .field("scale_denominator", &self.scale_denominator)
            .field("rotation_degrees", &self.rotation_degrees)
            .finish()
    }
}

/// WebView-safe recording artifact metadata. The source media path remains in
/// Rust; only an optional, path-policy-checked export destination is exposed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureArtifactSummary {
    pub schema_version: u16,
    pub artifact_token: String,
    pub artifact_revision: u64,
    pub duration_ms: u64,
    pub bytes_written: u64,
    pub editable_webm_output_path: Option<String>,
}

impl fmt::Debug for CaptureArtifactSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptureArtifactSummary")
            .field("schema_version", &self.schema_version)
            .field("artifact_token", &"<redacted>")
            .field("artifact_revision", &self.artifact_revision)
            .field("duration_ms", &self.duration_ms)
            .field("bytes_written", &self.bytes_written)
            .field(
                "editable_webm_output_path",
                &self
                    .editable_webm_output_path
                    .as_ref()
                    .map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Coarse project readiness that is safe to render in the WebView.
///
/// Filesystem names, durable project IDs, journal owners, and recovery
/// receipts remain native-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStudioProjectStatus {
    Ready,
    RecoveryRequired,
    AttentionRequired,
}

/// One session-scoped Studio project handle. The token is reminted whenever
/// the native project catalog changes, so stale UI selections fail closed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStudioProjectSummary {
    pub project_token: String,
    pub project_revision: Option<u64>,
    pub asset_count: u16,
    pub status: NativeStudioProjectStatus,
}

impl NativeStudioProjectSummary {
    pub fn validate(&self) -> Result<(), NativeDesktopContractError> {
        if !valid_opaque_id(&self.project_token)
            || self.project_revision == Some(0)
            || self.asset_count > MAX_STUDIO_PROJECT_ASSETS
            || (self.status == NativeStudioProjectStatus::Ready && self.project_revision.is_none())
        {
            return Err(NativeDesktopContractError::InvalidStudioProject);
        }
        Ok(())
    }
}

impl fmt::Debug for NativeStudioProjectSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioProjectSummary")
            .field("project_token", &"<redacted>")
            .field("project_revision", &self.project_revision)
            .field("asset_count", &self.asset_count)
            .field("status", &self.status)
            .finish()
    }
}

/// Bounded native Studio project catalog exposed without filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStudioProjectCatalog {
    pub schema_version: u16,
    pub generation: u64,
    pub projects: Vec<NativeStudioProjectSummary>,
}

impl NativeStudioProjectCatalog {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            schema_version: STUDIO_PROJECT_CATALOG_VERSION,
            generation: 0,
            projects: Vec::new(),
        }
    }

    pub fn validate_enumeration(&self) -> Result<(), NativeDesktopContractError> {
        if self.schema_version != STUDIO_PROJECT_CATALOG_VERSION || self.generation == 0 {
            return Err(NativeDesktopContractError::InvalidStudioProjectCatalog);
        }
        if self.projects.len() > MAX_STUDIO_PROJECT_CATALOG_ENTRIES {
            return Err(NativeDesktopContractError::TooManyStudioProjects);
        }
        let mut tokens = HashSet::with_capacity(self.projects.len());
        for project in &self.projects {
            project.validate()?;
            if !tokens.insert(project.project_token.as_str()) {
                return Err(NativeDesktopContractError::DuplicateStudioProject);
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioProjectOpenRequest {
    pub catalog_generation: u64,
    pub project_token: String,
}

impl fmt::Debug for NativeStudioProjectOpenRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioProjectOpenRequest")
            .field("catalog_generation", &self.catalog_generation)
            .field("project_token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioProjectOpenOutcome {
    pub catalog_generation: u64,
    pub project_token: String,
    pub project_revision: u64,
    pub duration_ms: u64,
}

impl fmt::Debug for NativeStudioProjectOpenOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioProjectOpenOutcome")
            .field("catalog_generation", &self.catalog_generation)
            .field("project_token", &"<redacted>")
            .field("project_revision", &self.project_revision)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

/// A bounded editor mutation retained inside the native Studio authority.
///
/// Millisecond inputs are the already-validated IPC representation. The
/// backend converts them to exact rational time before compiling the draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeStudioEditMutation {
    Trim {
        start_ms: u64,
        end_ms: u64,
    },
    DeleteRange {
        start_ms: u64,
        end_ms: u64,
    },
    Split {
        at_ms: u64,
    },
    Speed {
        start_ms: u64,
        end_ms: u64,
        rate_milli: u16,
    },
    AudioGain {
        start_ms: u64,
        end_ms: u64,
        gain_millibels: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioEditApplyRequest {
    pub base_editor_revision: u64,
    pub mutation: NativeStudioEditMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioEditApplyOutcome {
    pub base_editor_revision: u64,
    pub editor_revision: u64,
}

impl NativeStudioEditApplyOutcome {
    pub fn validate_for(
        self,
        request: &NativeStudioEditApplyRequest,
    ) -> Result<(), NativeDesktopContractError> {
        if self.base_editor_revision != request.base_editor_revision
            || self.editor_revision
                != request
                    .base_editor_revision
                    .checked_add(1)
                    .ok_or(NativeDesktopContractError::InvalidStudioEditor)?
        {
            return Err(NativeDesktopContractError::InvalidStudioEditor);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStudioEditSaveRequest {
    pub expected_editor_revision: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioEditSaveOutcome {
    pub editor_revision: u64,
    pub project_revision: u64,
    pub project_token: String,
    pub catalog: NativeStudioProjectCatalog,
}

impl NativeStudioEditSaveOutcome {
    pub fn validate_for(
        &self,
        request: NativeStudioEditSaveRequest,
        previous_catalog_generation: u64,
    ) -> Result<(), NativeDesktopContractError> {
        self.catalog.validate_enumeration()?;
        let project = self
            .catalog
            .projects
            .iter()
            .find(|project| project.project_token == self.project_token)
            .ok_or(NativeDesktopContractError::InvalidStudioEditor)?;
        if self.editor_revision != request.expected_editor_revision
            || self.project_revision == 0
            || self.catalog.generation <= previous_catalog_generation
            || project.status != NativeStudioProjectStatus::Ready
            || project.project_revision != Some(self.project_revision)
        {
            return Err(NativeDesktopContractError::InvalidStudioEditor);
        }
        Ok(())
    }
}

impl fmt::Debug for NativeStudioEditSaveOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioEditSaveOutcome")
            .field("editor_revision", &self.editor_revision)
            .field("project_revision", &self.project_revision)
            .field("project_token", &"<redacted>")
            .field("catalog", &self.catalog)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioRecoveryRequest {
    pub catalog_generation: u64,
    pub project_token: String,
}

impl fmt::Debug for NativeStudioRecoveryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioRecoveryRequest")
            .field("catalog_generation", &self.catalog_generation)
            .field("project_token", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStudioRecoveryAction {
    ArchiveUnstartedAttempt,
    RecoverRecording,
    ReconcileEditSave,
    OpenEditor,
    RequiresOperatorDecision,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioRecoveryInspection {
    pub catalog_generation: u64,
    pub project_token: String,
    pub status: NativeStudioProjectStatus,
    pub action: NativeStudioRecoveryAction,
}

impl NativeStudioRecoveryInspection {
    pub fn validate_for(
        &self,
        request: &NativeStudioRecoveryRequest,
    ) -> Result<(), NativeDesktopContractError> {
        let valid_action = matches!(
            (self.status, self.action),
            (
                NativeStudioProjectStatus::RecoveryRequired,
                NativeStudioRecoveryAction::ArchiveUnstartedAttempt
                    | NativeStudioRecoveryAction::RecoverRecording
                    | NativeStudioRecoveryAction::ReconcileEditSave
            ) | (
                NativeStudioProjectStatus::Ready,
                NativeStudioRecoveryAction::OpenEditor
            ) | (
                NativeStudioProjectStatus::AttentionRequired,
                NativeStudioRecoveryAction::RequiresOperatorDecision
            )
        );
        if self.catalog_generation != request.catalog_generation
            || self.project_token != request.project_token
            || !valid_action
        {
            return Err(NativeDesktopContractError::InvalidStudioRecovery);
        }
        Ok(())
    }
}

impl fmt::Debug for NativeStudioRecoveryInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioRecoveryInspection")
            .field("catalog_generation", &self.catalog_generation)
            .field("project_token", &"<redacted>")
            .field("status", &self.status)
            .field("action", &self.action)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioRecoveryOutcome {
    pub catalog: NativeStudioProjectCatalog,
    pub recovered_project_token: String,
    pub project_revision: u64,
    pub duration_ms: u64,
}

impl NativeStudioRecoveryOutcome {
    pub fn validate_for(
        &self,
        request: &NativeStudioRecoveryRequest,
    ) -> Result<(), NativeDesktopContractError> {
        self.catalog.validate_enumeration()?;
        let project = self
            .catalog
            .projects
            .iter()
            .find(|project| project.project_token == self.recovered_project_token)
            .ok_or(NativeDesktopContractError::InvalidStudioRecovery)?;
        if self.catalog.generation <= request.catalog_generation
            || self.recovered_project_token == request.project_token
            || self.project_revision == 0
            || self.duration_ms == 0
            || project.status != NativeStudioProjectStatus::Ready
            || project.project_revision != Some(self.project_revision)
        {
            return Err(NativeDesktopContractError::InvalidStudioRecovery);
        }
        Ok(())
    }
}

impl fmt::Debug for NativeStudioRecoveryOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioRecoveryOutcome")
            .field("catalog", &self.catalog)
            .field("recovered_project_token", &"<redacted>")
            .field("project_revision", &self.project_revision)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStudioRecoveryArchiveOutcome {
    pub catalog: NativeStudioProjectCatalog,
}

impl NativeStudioRecoveryArchiveOutcome {
    pub fn validate_for(
        &self,
        request: &NativeStudioRecoveryRequest,
    ) -> Result<(), NativeDesktopContractError> {
        self.catalog.validate_enumeration()?;
        if self.catalog.generation <= request.catalog_generation
            || self
                .catalog
                .projects
                .iter()
                .any(|project| project.project_token == request.project_token)
        {
            return Err(NativeDesktopContractError::InvalidStudioRecovery);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePermissionOutcome {
    Granted,
    Denied,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeTargetSelectionRequest {
    pub catalog_generation: u64,
    pub target: CaptureTargetSummary,
}

impl fmt::Debug for NativeTargetSelectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTargetSelectionRequest")
            .field("catalog_generation", &self.catalog_generation)
            .field("target", &self.target)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeTargetSelectionOutcome {
    pub catalog_generation: u64,
    pub target_token: String,
}

/// A display-relative logical region. Desktop origins remain native-only, and
/// the catalog generation binds the request to the geometry the user saw.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeRegionDefinitionRequest {
    pub catalog_generation: u64,
    pub display: CaptureTargetSummary,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl fmt::Debug for NativeRegionDefinitionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRegionDefinitionRequest")
            .field("catalog_generation", &self.catalog_generation)
            .field("display", &self.display)
            .field("geometry", &"<redacted>")
            .finish()
    }
}

/// The refreshed catalog and exact region selected by the backend. Returning
/// both prevents the WebView from guessing which token belongs to its request.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeRegionDefinitionOutcome {
    pub catalog: CaptureTargetCatalog,
    pub region: CaptureTargetSummary,
}

impl fmt::Debug for NativeRegionDefinitionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRegionDefinitionOutcome")
            .field("catalog", &self.catalog)
            .field("region", &self.region)
            .finish()
    }
}

impl fmt::Debug for NativeTargetSelectionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTargetSelectionOutcome")
            .field("catalog_generation", &self.catalog_generation)
            .field("target_token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeCaptureStartRequest {
    pub catalog_generation: u64,
    pub target: CaptureTargetSummary,
    pub mode: RecorderMode,
    pub frame_rate: u16,
    pub exclude_frame_windows: bool,
    /// Request system audio when the native backend can start it safely.
    /// A backend may report a verified screen-only fallback in its outcome.
    pub system_audio_enabled: bool,
    /// Request the confirmed default microphone through the normalized
    /// native-input bridge. Denial may fall back to the remaining sources.
    pub microphone_enabled: bool,
    /// Request the confirmed default camera as an isolated Studio original.
    /// Visual composition remains owned by the Studio editor.
    pub camera_enabled: bool,
}

impl fmt::Debug for NativeCaptureStartRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeCaptureStartRequest")
            .field("catalog_generation", &self.catalog_generation)
            .field("target", &self.target)
            .field("mode", &self.mode)
            .field("frame_rate", &self.frame_rate)
            .field("exclude_frame_windows", &self.exclude_frame_windows)
            .field("system_audio_enabled", &self.system_audio_enabled)
            .field("microphone_enabled", &self.microphone_enabled)
            .field("camera_enabled", &self.camera_enabled)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeRecordingStartOutcome {
    pub catalog_generation: u64,
    pub target_token: String,
    pub recording_token: String,
    /// True only when the owned recording graph accepted system-audio PCM.
    pub system_audio_included: bool,
    pub microphone_included: bool,
    pub camera_included: bool,
}

impl fmt::Debug for NativeRecordingStartOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRecordingStartOutcome")
            .field("catalog_generation", &self.catalog_generation)
            .field("target_token", &"<redacted>")
            .field("recording_token", &"<redacted>")
            .field("system_audio_included", &self.system_audio_included)
            .field("microphone_included", &self.microphone_included)
            .field("camera_included", &self.camera_included)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeRecordingControlRequest {
    pub recording_token: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeRecordingInputControlRequest {
    pub recording_token: String,
    pub class: DeviceClass,
    pub gain_milli: u16,
    pub muted: bool,
    pub enabled: bool,
}

impl fmt::Debug for NativeRecordingInputControlRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRecordingInputControlRequest")
            .field("recording_token", &"<redacted>")
            .field("class", &self.class)
            .field("gain_milli", &self.gain_milli)
            .field("muted", &self.muted)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl fmt::Debug for NativeRecordingControlRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRecordingControlRequest")
            .field("recording_token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeCaptureArtifact {
    pub recording_token: String,
    pub artifact_token: String,
    pub artifact_revision: u64,
    pub duration_ms: u64,
    pub bytes_written: u64,
    pub media_path: String,
    pub editable_webm_output_path: Option<String>,
    /// Native-only canonical Studio document. The runtime validates and
    /// retains it as Rust authority; it is never serialized into the WebView
    /// artifact summary.
    pub studio_project_path: Option<String>,
}

impl fmt::Debug for NativeCaptureArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeCaptureArtifact")
            .field("recording_token", &"<redacted>")
            .field("artifact_token", &"<redacted>")
            .field("artifact_revision", &self.artifact_revision)
            .field("duration_ms", &self.duration_ms)
            .field("bytes_written", &self.bytes_written)
            .field("media_path", &"<redacted>")
            .field(
                "editable_webm_output_path",
                &self
                    .editable_webm_output_path
                    .as_ref()
                    .map(|_| "<redacted>"),
            )
            .field(
                "studio_project_path",
                &self.studio_project_path.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// A recording ended without a usable artifact.
///
/// This is a terminal backend result rather than a command error: the runtime
/// must consume the recording authority even though it presents a failed
/// recorder state. `teardown_confirmed` is false only when starting another
/// native session would be unsafe without rebuilding the backend.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeRecordingTerminalFailure {
    pub recording_token: String,
    pub error: NativeDesktopBackendError,
    pub teardown_confirmed: bool,
}

impl fmt::Debug for NativeRecordingTerminalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRecordingTerminalFailure")
            .field("recording_token", &"<redacted>")
            .field("error", &self.error)
            .field("teardown_confirmed", &self.teardown_confirmed)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum NativeRecordingStopOutcome {
    Sealed(NativeCaptureArtifact),
    Failed(NativeRecordingTerminalFailure),
}

impl fmt::Debug for NativeRecordingStopOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sealed(artifact) => formatter.debug_tuple("Sealed").field(artifact).finish(),
            Self::Failed(failure) => formatter.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum NativeRecordingCancelOutcome {
    Cancelled { recording_token: String },
    Failed(NativeRecordingTerminalFailure),
}

impl fmt::Debug for NativeRecordingCancelOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled { .. } => formatter
                .debug_struct("Cancelled")
                .field("recording_token", &"<redacted>")
                .finish(),
            Self::Failed(failure) => formatter.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeEditableWebmExportRequest {
    pub artifact_token: String,
    pub artifact_revision: u64,
    pub source_media_path: ValidatedPath,
    pub output_path: ValidatedPath,
}

impl fmt::Debug for NativeEditableWebmExportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeEditableWebmExportRequest")
            .field("artifact_token", &"<redacted>")
            .field("artifact_revision", &self.artifact_revision)
            .field("source_media_path", &self.source_media_path)
            .field("output_path", &self.output_path)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeEditableWebmExportOutcome {
    pub artifact_token: String,
    pub artifact_revision: u64,
    pub bytes_written: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioExportRequest {
    pub project_revision: u64,
    pub output_path: ValidatedPath,
    pub profile: ExportProfile,
}

impl fmt::Debug for NativeStudioExportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioExportRequest")
            .field("project_revision", &self.project_revision)
            .field("output_path", &self.output_path)
            .field("profile", &self.profile)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeStudioExportOutcome {
    pub project_revision: u64,
    pub profile: ExportProfile,
    pub bytes_written: u64,
    pub sha256: String,
}

impl NativeStudioExportOutcome {
    pub fn validate_for(
        &self,
        request: &NativeStudioExportRequest,
    ) -> Result<(), NativeDesktopContractError> {
        if self.project_revision != request.project_revision
            || self.profile != request.profile
            || self.bytes_written == 0
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(NativeDesktopContractError::InvalidStudioExport);
        }
        Ok(())
    }
}

impl fmt::Debug for NativeStudioExportOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeStudioExportOutcome")
            .field("project_revision", &self.project_revision)
            .field("profile", &self.profile)
            .field("bytes_written", &self.bytes_written)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

/// Privacy-safe, raw-media-free telemetry for one active native recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeRecordingMeter {
    pub microphone_basis_points: u16,
    pub system_audio_basis_points: u16,
    pub camera_active: bool,
}

/// Bounded, label-free inventory for native optional inputs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeInputDeviceCounts {
    pub microphones: u16,
    pub system_audio_sources: u16,
    pub cameras: u16,
}

impl fmt::Debug for NativeEditableWebmExportOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeEditableWebmExportOutcome")
            .field("artifact_token", &"<redacted>")
            .field("artifact_revision", &self.artifact_revision)
            .field("bytes_written", &self.bytes_written)
            .finish()
    }
}

/// Injected native capability. Implementations own platform handles, raw IDs,
/// capture sessions, and filesystem opens; the desktop core owns UI truth.
pub trait NativeDesktopBackend {
    fn prepare_capture(&mut self) -> Result<NativePermissionOutcome, NativeDesktopBackendError>;

    fn enumerate_targets(&mut self) -> Result<CaptureTargetCatalog, NativeDesktopBackendError>;

    fn enumerate_input_devices(
        &mut self,
    ) -> Result<NativeInputDeviceCounts, NativeDesktopBackendError> {
        Ok(NativeInputDeviceCounts::default())
    }

    fn select_target(
        &mut self,
        request: &NativeTargetSelectionRequest,
    ) -> Result<NativeTargetSelectionOutcome, NativeDesktopBackendError>;

    fn define_region(
        &mut self,
        request: &NativeRegionDefinitionRequest,
    ) -> Result<NativeRegionDefinitionOutcome, NativeDesktopBackendError>;

    fn start_recording(
        &mut self,
        request: &NativeCaptureStartRequest,
    ) -> Result<NativeRecordingStartOutcome, NativeDesktopBackendError>;

    /// Checks an active recording without stopping it or waiting for it to
    /// finish. Implementations must keep this call bounded and return a
    /// terminal failure only when it belongs to `request.recording_token`.
    /// Backends without an asynchronous worker have no failure to report.
    fn poll_recording_terminal_failure(
        &mut self,
        _request: &NativeRecordingControlRequest,
    ) -> Result<Option<NativeRecordingTerminalFailure>, NativeDesktopBackendError> {
        Ok(None)
    }

    /// Reads coarse recording telemetry without transferring PCM through IPC.
    /// The default keeps portable and non-audio backends silent.
    fn poll_recording_meter(
        &mut self,
        _request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingMeter, NativeDesktopBackendError> {
        Ok(NativeRecordingMeter::default())
    }

    fn pause_recording(
        &mut self,
        _request: &NativeRecordingControlRequest,
    ) -> Result<(), NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn resume_recording(
        &mut self,
        _request: &NativeRecordingControlRequest,
    ) -> Result<(), NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn set_recording_input(
        &mut self,
        _request: &NativeRecordingInputControlRequest,
    ) -> Result<(), NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn stop_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingStopOutcome, NativeDesktopBackendError>;

    fn cancel_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingCancelOutcome, NativeDesktopBackendError>;

    /// Enumerates durable Studio journals and projects through reminted,
    /// session-scoped handles. Portable backends report a valid empty catalog.
    fn scan_studio_projects(
        &mut self,
    ) -> Result<NativeStudioProjectCatalog, NativeDesktopBackendError> {
        Ok(NativeStudioProjectCatalog {
            schema_version: STUDIO_PROJECT_CATALOG_VERSION,
            generation: 1,
            projects: Vec::new(),
        })
    }

    /// Authenticates and opens one ready Studio project selected from the most
    /// recent catalog. Recovery-required entries are never opened through this
    /// path.
    fn open_studio_project(
        &mut self,
        _request: &NativeStudioProjectOpenRequest,
    ) -> Result<NativeStudioProjectOpenOutcome, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn apply_studio_edit(
        &mut self,
        _request: &NativeStudioEditApplyRequest,
    ) -> Result<NativeStudioEditApplyOutcome, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn save_studio_edits(
        &mut self,
        _request: NativeStudioEditSaveRequest,
    ) -> Result<NativeStudioEditSaveOutcome, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn export_studio_project(
        &mut self,
        _request: &NativeStudioExportRequest,
    ) -> Result<NativeStudioExportOutcome, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn inspect_studio_recovery(
        &mut self,
        _request: &NativeStudioRecoveryRequest,
    ) -> Result<NativeStudioRecoveryInspection, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn recover_studio_project(
        &mut self,
        _request: &NativeStudioRecoveryRequest,
    ) -> Result<NativeStudioRecoveryOutcome, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn archive_studio_recovery(
        &mut self,
        _request: &NativeStudioRecoveryRequest,
    ) -> Result<NativeStudioRecoveryArchiveOutcome, NativeDesktopBackendError> {
        Err(NativeDesktopBackendError::Unavailable)
    }

    fn export_editable_webm(
        &mut self,
        request: &NativeEditableWebmExportRequest,
    ) -> Result<NativeEditableWebmExportOutcome, NativeDesktopBackendError>;
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NativeDesktopBackendError {
    #[error("native capture is unavailable")]
    Unavailable,
    #[error("native capture is busy")]
    Busy,
    #[error("native capture permission was denied")]
    PermissionDenied,
    #[error("capture target catalog is stale")]
    StaleCatalog,
    #[error("capture target is unavailable")]
    TargetUnavailable,
    #[error("native operation was cancelled")]
    Cancelled,
    #[error("the requested Studio edit is invalid")]
    InvalidEdit,
    #[error("native filesystem operation failed")]
    Filesystem,
    #[error("native capture failed")]
    Internal,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NativeDesktopContractError {
    #[error("capture target catalog version is unsupported")]
    UnsupportedCatalogVersion,
    #[error("capture target catalog generation is invalid")]
    InvalidCatalogGeneration,
    #[error("capture target catalog exceeds its bound")]
    TooManyCaptureTargets,
    #[error("capture target summary is invalid")]
    InvalidCaptureTarget,
    #[error("capture target catalog contains a duplicate")]
    DuplicateCaptureTarget,
    #[error("Studio project catalog is invalid")]
    InvalidStudioProjectCatalog,
    #[error("Studio project summary is invalid")]
    InvalidStudioProject,
    #[error("Studio project catalog exceeds its bound")]
    TooManyStudioProjects,
    #[error("Studio project catalog contains a duplicate")]
    DuplicateStudioProject,
    #[error("Studio recovery response is invalid")]
    InvalidStudioRecovery,
    #[error("Studio editor response is invalid")]
    InvalidStudioEditor,
    #[error("Studio export response is invalid")]
    InvalidStudioExport,
}

const fn target_kind_tag(kind: CaptureTargetKind) -> u8 {
    match kind {
        CaptureTargetKind::Display => 0,
        CaptureTargetKind::Window => 1,
        CaptureTargetKind::Region => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PathPolicy, PathUse, RootAccess};

    #[test]
    fn catalog_json_has_only_coarse_geometry_and_opaque_identity() {
        let catalog = CaptureTargetCatalog {
            schema_version: CAPTURE_TARGET_CATALOG_VERSION,
            generation: 7,
            targets: vec![CaptureTargetSummary {
                token: "display-token-1".into(),
                kind: CaptureTargetKind::Display,
                ordinal: 1,
                width_pixels: 1_920,
                height_pixels: 1_080,
                scale_numerator: 2,
                scale_denominator: 1,
                rotation_degrees: 0,
            }],
        };
        catalog.validate().expect("valid catalog");
        let debug = format!("{catalog:?}");
        assert!(!debug.contains("display-token-1"));
        let json = serde_json::to_value(&catalog).expect("serialize");
        let target = &json["targets"][0];
        assert!(target.get("token").is_some());
        assert!(target.get("kind").is_some());
        assert!(target.get("ordinal").is_some());
        assert!(target.get("width_pixels").is_some());
        assert!(target.get("height_pixels").is_some());
        assert!(target.get("scale_numerator").is_some());
        assert!(target.get("scale_denominator").is_some());
        assert!(target.get("rotation_degrees").is_some());
        assert!(target.get("title").is_none());
        assert!(target.get("name").is_none());
        assert!(target.get("raw_id").is_none());
        assert_eq!(target.as_object().expect("object").len(), 8);

        let mut injected = json;
        injected["targets"][0]["title"] = serde_json::Value::String("Private title".into());
        assert!(serde_json::from_value::<CaptureTargetCatalog>(injected).is_err());

        let artifact = CaptureArtifactSummary {
            schema_version: CAPTURE_ARTIFACT_SUMMARY_VERSION,
            artifact_token: "artifact-token-1".into(),
            artifact_revision: 1,
            duration_ms: 1_000,
            bytes_written: 4_096,
            editable_webm_output_path: Some("/private/frame/export.webm".into()),
        };
        let debug = format!("{artifact:?}");
        assert!(!debug.contains("artifact-token-1"));
        assert!(!debug.contains("/private/frame/export.webm"));
    }

    #[test]
    fn catalog_rejects_duplicate_or_unbounded_targets() {
        let target = CaptureTargetSummary {
            token: "display-token-1".into(),
            kind: CaptureTargetKind::Display,
            ordinal: 1,
            width_pixels: 1_920,
            height_pixels: 1_080,
            scale_numerator: 1,
            scale_denominator: 1,
            rotation_degrees: 0,
        };
        let duplicate = CaptureTargetCatalog {
            schema_version: CAPTURE_TARGET_CATALOG_VERSION,
            generation: 1,
            targets: vec![target.clone(), target],
        };
        assert_eq!(
            duplicate.validate(),
            Err(NativeDesktopContractError::DuplicateCaptureTarget)
        );
        assert!(CaptureTargetCatalog::empty().validate().is_ok());
        assert_eq!(
            CaptureTargetCatalog::empty().validate_enumeration(),
            Err(NativeDesktopContractError::InvalidCatalogGeneration)
        );
    }

    #[test]
    fn studio_catalog_exposes_only_bounded_opaque_project_facts() {
        let summary = NativeStudioProjectSummary {
            project_token: "studio-project-token-1".into(),
            project_revision: Some(7),
            asset_count: 4,
            status: NativeStudioProjectStatus::Ready,
        };
        let catalog = NativeStudioProjectCatalog {
            schema_version: STUDIO_PROJECT_CATALOG_VERSION,
            generation: 3,
            projects: vec![summary.clone()],
        };
        catalog.validate_enumeration().expect("valid catalog");
        let json = serde_json::to_value(&catalog).expect("catalog JSON");
        let project = &json["projects"][0];
        assert_eq!(project.as_object().expect("project object").len(), 4);
        assert!(project.get("project_token").is_some());
        assert!(project.get("project_revision").is_some());
        assert!(project.get("asset_count").is_some());
        assert!(project.get("status").is_some());
        for forbidden in [
            "path",
            "project_id",
            "journal",
            "owner",
            "receipt",
            "asset_name",
        ] {
            assert!(project.get(forbidden).is_none());
        }
        assert!(!format!("{summary:?}").contains("studio-project-token-1"));

        let duplicate = NativeStudioProjectCatalog {
            projects: vec![summary.clone(), summary],
            ..catalog
        };
        assert_eq!(
            duplicate.validate_enumeration(),
            Err(NativeDesktopContractError::DuplicateStudioProject)
        );
    }

    #[test]
    fn recovery_outcomes_are_generation_fenced_and_redact_tokens() {
        let request = NativeStudioRecoveryRequest {
            catalog_generation: 4,
            project_token: "interrupted-project-token".into(),
        };
        let inspection = NativeStudioRecoveryInspection {
            catalog_generation: 4,
            project_token: "interrupted-project-token".into(),
            status: NativeStudioProjectStatus::RecoveryRequired,
            action: NativeStudioRecoveryAction::RecoverRecording,
        };
        inspection
            .validate_for(&request)
            .expect("valid recovery inspection");
        assert!(!format!("{inspection:?}").contains("interrupted-project-token"));

        let catalog = NativeStudioProjectCatalog {
            schema_version: STUDIO_PROJECT_CATALOG_VERSION,
            generation: 5,
            projects: vec![NativeStudioProjectSummary {
                project_token: "recovered-project-token".into(),
                project_revision: Some(1),
                asset_count: 2,
                status: NativeStudioProjectStatus::Ready,
            }],
        };
        let outcome = NativeStudioRecoveryOutcome {
            catalog: catalog.clone(),
            recovered_project_token: "recovered-project-token".into(),
            project_revision: 1,
            duration_ms: 2_000,
        };
        outcome
            .validate_for(&request)
            .expect("generation-fenced recovery");
        assert!(!format!("{outcome:?}").contains("recovered-project-token"));

        let stale = NativeStudioRecoveryArchiveOutcome {
            catalog: NativeStudioProjectCatalog {
                generation: request.catalog_generation,
                ..catalog
            },
        };
        assert_eq!(
            stale.validate_for(&request),
            Err(NativeDesktopContractError::InvalidStudioRecovery)
        );
    }

    #[test]
    fn editor_outcomes_are_revision_fenced_and_redact_reminted_tokens() {
        let apply = NativeStudioEditApplyRequest {
            base_editor_revision: 7,
            mutation: NativeStudioEditMutation::Split { at_ms: 1_000 },
        };
        NativeStudioEditApplyOutcome {
            base_editor_revision: 7,
            editor_revision: 8,
        }
        .validate_for(&apply)
        .expect("valid editor advance");
        assert_eq!(
            NativeStudioEditApplyOutcome {
                base_editor_revision: 7,
                editor_revision: 9,
            }
            .validate_for(&apply),
            Err(NativeDesktopContractError::InvalidStudioEditor)
        );

        let save = NativeStudioEditSaveRequest {
            expected_editor_revision: 8,
        };
        let outcome = NativeStudioEditSaveOutcome {
            editor_revision: 8,
            project_revision: 4,
            project_token: "studio-project-token-reminted".into(),
            catalog: NativeStudioProjectCatalog {
                schema_version: STUDIO_PROJECT_CATALOG_VERSION,
                generation: 10,
                projects: vec![NativeStudioProjectSummary {
                    project_token: "studio-project-token-reminted".into(),
                    project_revision: Some(4),
                    asset_count: 3,
                    status: NativeStudioProjectStatus::Ready,
                }],
            },
        };
        outcome.validate_for(save, 9).expect("valid durable save");
        assert!(!format!("{outcome:?}").contains("studio-project-token-reminted"));
        assert_eq!(
            outcome.validate_for(save, 10),
            Err(NativeDesktopContractError::InvalidStudioEditor)
        );
    }

    #[test]
    fn studio_export_outcome_is_revision_profile_and_digest_bound() {
        let policy = PathPolicy::empty()
            .allow_root(
                "/private/tmp/frame-native-export-contract",
                RootAccess {
                    read: false,
                    write: true,
                    delete: false,
                },
            )
            .expect("export policy");
        let request = NativeStudioExportRequest {
            project_revision: 9,
            output_path: policy
                .validate(
                    "/private/tmp/frame-native-export-contract/output.mp4",
                    PathUse::ExportWrite,
                )
                .expect("validated export"),
            profile: ExportProfile::DistributionMp4,
        };
        let outcome = NativeStudioExportOutcome {
            project_revision: 9,
            profile: ExportProfile::DistributionMp4,
            bytes_written: 1_024,
            sha256: "ab".repeat(32),
        };
        outcome.validate_for(&request).expect("valid export");
        assert!(!format!("{outcome:?}").contains(&outcome.sha256));

        let uppercase = NativeStudioExportOutcome {
            sha256: "AB".repeat(32),
            ..outcome
        };
        assert_eq!(
            uppercase.validate_for(&request),
            Err(NativeDesktopContractError::InvalidStudioExport)
        );
    }
}
