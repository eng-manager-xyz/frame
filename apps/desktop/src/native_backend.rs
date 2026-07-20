//! Portable contract between the desktop state machine and native capture.
//!
//! Native platform identities never cross this boundary. Backends mint opaque
//! tokens and return only bounded, privacy-safe target metadata. File paths
//! passed to native export have already been scoped by the IPC path policy.

use std::{collections::HashSet, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ipc::{CaptureTargetKind, ValidatedPath, valid_opaque_id};

pub const CAPTURE_TARGET_CATALOG_VERSION: u16 = 1;
pub const CAPTURE_ARTIFACT_SUMMARY_VERSION: u16 = 1;
const MAX_CAPTURE_TARGETS: usize = 256;
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
    pub frame_rate: u16,
    pub exclude_frame_windows: bool,
    /// Request system audio when the native backend can start it safely.
    /// A backend may report a verified screen-only fallback in its outcome.
    pub system_audio_enabled: bool,
}

impl fmt::Debug for NativeCaptureStartRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeCaptureStartRequest")
            .field("catalog_generation", &self.catalog_generation)
            .field("target", &self.target)
            .field("frame_rate", &self.frame_rate)
            .field("exclude_frame_windows", &self.exclude_frame_windows)
            .field("system_audio_enabled", &self.system_audio_enabled)
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
}

impl fmt::Debug for NativeRecordingStartOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRecordingStartOutcome")
            .field("catalog_generation", &self.catalog_generation)
            .field("target_token", &"<redacted>")
            .field("recording_token", &"<redacted>")
            .field("system_audio_included", &self.system_audio_included)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeRecordingControlRequest {
    pub recording_token: String,
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

/// Privacy-safe, raw-media-free telemetry for one active native recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeRecordingMeter {
    pub system_audio_basis_points: u16,
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

    fn stop_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingStopOutcome, NativeDesktopBackendError>;

    fn cancel_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingCancelOutcome, NativeDesktopBackendError>;

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
}
