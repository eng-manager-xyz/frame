use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{ClientError, ClientErrorCode, FrameOrigin};

pub const CONTRACT_MAJOR: u16 = 1;
pub const INSTANT_UI_PROGRESS_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiVersion {
    pub major: u16,
}

impl ApiVersion {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            major: CONTRACT_MAJOR,
        }
    }

    pub fn negotiate(self) -> Result<(), ClientError> {
        if self.major == CONTRACT_MAJOR {
            Ok(())
        } else {
            Err(ClientError::new(ClientErrorCode::IncompatibleVersion))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capabilities(Vec<String>);

impl Capabilities {
    pub fn from_names(names: Vec<String>) -> Result<Self, ClientError> {
        let capabilities = Self(names);
        capabilities.validate()?;
        Ok(capabilities)
    }

    #[must_use]
    pub fn supports(&self, capability: &str) -> bool {
        self.0.iter().any(|candidate| candidate == capability)
    }

    pub fn validate(&self) -> Result<(), ClientError> {
        if self.0.len() > 64
            || self.0.iter().any(|capability| {
                capability.is_empty()
                    || capability.len() > 64
                    || !capability.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'.' | b'_' | b'-')
                    })
            })
        {
            return Err(ClientError::new(ClientErrorCode::InvalidContract));
        }
        Ok(())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Ok,
    Degraded,
    Maintenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub api_version: ApiVersion,
    pub service: String,
    pub status: ServiceStatus,
    pub release: String,
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl Health {
    pub fn validate(&self) -> Result<(), ClientError> {
        self.api_version.negotiate()?;
        self.capabilities.validate()?;
        if self.service != "frame"
            || self.release.is_empty()
            || self.release.len() > 64
            || !self
                .release
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ClientError::new(ClientErrorCode::InvalidContract));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareAvailability {
    Public,
    Processing,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstantUiPhaseV1 {
    Recording,
    LocallyRecoverable,
    Uploading,
    Finalizing,
    ShareReady,
    Cancelled,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstantUiErrorCodeV1 {
    LocalStorageFull,
    LocalStorageUnavailable,
    NetworkOffline,
    UploadDelayed,
    UploadExpired,
    FinalizeDelayed,
    RecordingRecoveryRequired,
    RecordingCancelled,
    RecordingFailed,
}

impl InstantUiErrorCodeV1 {
    const fn retryable(self) -> bool {
        matches!(
            self,
            Self::NetworkOffline
                | Self::UploadDelayed
                | Self::UploadExpired
                | Self::FinalizeDelayed
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstantUiProgressV1 {
    pub schema_version: u16,
    pub phase: InstantUiPhaseV1,
    pub progress_basis_points: Option<u16>,
    pub retrying: bool,
    pub error: Option<InstantUiErrorCodeV1>,
}

impl InstantUiProgressV1 {
    pub fn new(
        phase: InstantUiPhaseV1,
        progress_basis_points: Option<u16>,
        retrying: bool,
        error: Option<InstantUiErrorCodeV1>,
    ) -> Result<Self, ClientError> {
        let progress = Self {
            schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
            phase,
            progress_basis_points,
            retrying,
            error,
        };
        progress.validate()?;
        Ok(progress)
    }

    pub fn validate(&self) -> Result<(), ClientError> {
        if self.schema_version != INSTANT_UI_PROGRESS_SCHEMA_VERSION
            || self
                .progress_basis_points
                .is_some_and(|basis_points| basis_points > 10_000)
            || self.retrying != self.error.is_some_and(InstantUiErrorCodeV1::retryable)
            || !self.phase_accepts_error()
        {
            return Err(ClientError::new(ClientErrorCode::InvalidContract));
        }

        match self.phase {
            InstantUiPhaseV1::ShareReady
                if self.progress_basis_points == Some(10_000)
                    && !self.retrying
                    && self.error.is_none() =>
            {
                Ok(())
            }
            InstantUiPhaseV1::Cancelled
                if self.progress_basis_points.is_none()
                    && !self.retrying
                    && self.error == Some(InstantUiErrorCodeV1::RecordingCancelled) =>
            {
                Ok(())
            }
            InstantUiPhaseV1::RecoveryRequired
                if self.progress_basis_points.is_none()
                    && !self.retrying
                    && matches!(
                        self.error,
                        Some(
                            InstantUiErrorCodeV1::LocalStorageFull
                                | InstantUiErrorCodeV1::LocalStorageUnavailable
                                | InstantUiErrorCodeV1::RecordingRecoveryRequired
                                | InstantUiErrorCodeV1::RecordingFailed
                        )
                    ) =>
            {
                Ok(())
            }
            InstantUiPhaseV1::Recording
            | InstantUiPhaseV1::LocallyRecoverable
            | InstantUiPhaseV1::Uploading
            | InstantUiPhaseV1::Finalizing => Ok(()),
            InstantUiPhaseV1::ShareReady
            | InstantUiPhaseV1::Cancelled
            | InstantUiPhaseV1::RecoveryRequired => {
                Err(ClientError::new(ClientErrorCode::InvalidContract))
            }
        }
    }

    pub fn validate_for_public_share(&self) -> Result<(), ClientError> {
        self.validate()?;
        let safe = matches!(
            (self.phase, self.retrying, self.error),
            (InstantUiPhaseV1::Uploading, false, None)
                | (InstantUiPhaseV1::Finalizing, false, None)
                | (
                    InstantUiPhaseV1::Uploading,
                    true,
                    Some(InstantUiErrorCodeV1::UploadDelayed)
                )
                | (
                    InstantUiPhaseV1::Finalizing,
                    true,
                    Some(InstantUiErrorCodeV1::FinalizeDelayed)
                )
        );
        if safe {
            Ok(())
        } else {
            Err(ClientError::new(ClientErrorCode::PrivacyViolation))
        }
    }

    fn phase_accepts_error(&self) -> bool {
        match (self.phase, self.error) {
            (_, None) => !matches!(
                self.phase,
                InstantUiPhaseV1::Cancelled | InstantUiPhaseV1::RecoveryRequired
            ),
            (InstantUiPhaseV1::Recording, Some(InstantUiErrorCodeV1::UploadDelayed))
            | (
                InstantUiPhaseV1::LocallyRecoverable,
                Some(InstantUiErrorCodeV1::NetworkOffline | InstantUiErrorCodeV1::UploadDelayed),
            )
            | (
                InstantUiPhaseV1::Uploading,
                Some(
                    InstantUiErrorCodeV1::NetworkOffline
                    | InstantUiErrorCodeV1::UploadDelayed
                    | InstantUiErrorCodeV1::UploadExpired,
                ),
            )
            | (InstantUiPhaseV1::Finalizing, Some(InstantUiErrorCodeV1::FinalizeDelayed))
            | (InstantUiPhaseV1::Cancelled, Some(InstantUiErrorCodeV1::RecordingCancelled))
            | (
                InstantUiPhaseV1::RecoveryRequired,
                Some(
                    InstantUiErrorCodeV1::LocalStorageFull
                    | InstantUiErrorCodeV1::LocalStorageUnavailable
                    | InstantUiErrorCodeV1::RecordingRecoveryRequired
                    | InstantUiErrorCodeV1::RecordingFailed,
                ),
            ) => true,
            _ => false,
        }
    }
}

impl fmt::Debug for InstantUiProgressV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstantUiProgressV1")
            .field("schema_version", &self.schema_version)
            .field("public_status", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackDescriptor {
    pub path: String,
    pub content_type: String,
    pub supports_range: bool,
    #[serde(default)]
    pub captions: Vec<CaptionTrack>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub path: String,
    pub language: String,
    pub label: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicShareSummary {
    pub api_version: ApiVersion,
    pub availability: ShareAvailability,
    pub title: Option<String>,
    pub description: Option<String>,
    pub canonical_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub playback: Option<PlaybackDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_status: Option<InstantUiProgressV1>,
}

impl PublicShareSummary {
    pub fn validate(&self, origin: &FrameOrigin) -> Result<(), ClientError> {
        self.api_version.negotiate()?;
        match self.availability {
            ShareAvailability::Public => {
                let title = self
                    .title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty() && title.len() <= 200)
                    .ok_or_else(|| ClientError::new(ClientErrorCode::InvalidContract))?;
                if title.chars().any(char::is_control)
                    || self.description.as_ref().is_some_and(|value| {
                        value.len() > 2_000 || value.chars().any(char::is_control)
                    })
                    || self
                        .duration_ms
                        .is_some_and(|duration| duration > 24 * 60 * 60 * 1_000)
                    || self.processing_status.is_some()
                {
                    return Err(ClientError::new(ClientErrorCode::InvalidContract));
                }
                validate_canonical(self.canonical_url.as_deref(), origin)?;
                validate_playback(
                    self.playback
                        .as_ref()
                        .ok_or_else(|| ClientError::new(ClientErrorCode::InvalidContract))?,
                )?;
            }
            ShareAvailability::Processing => {
                validate_canonical(self.canonical_url.as_deref(), origin)?;
                if self.title.is_some()
                    || self.description.is_some()
                    || self.duration_ms.is_some()
                    || self.playback.is_some()
                {
                    return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
                }
                if let Some(status) = self.processing_status.as_ref() {
                    status.validate_for_public_share()?;
                }
            }
            ShareAvailability::Unavailable => {
                if self.title.is_some()
                    || self.description.is_some()
                    || self.canonical_url.is_some()
                    || self.duration_ms.is_some()
                    || self.playback.is_some()
                    || self.processing_status.is_some()
                {
                    return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for PublicShareSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicShareSummary")
            .field("availability", &self.availability)
            .field("public_fields", &"<redacted>")
            .finish()
    }
}

impl fmt::Debug for PlaybackDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaybackDescriptor")
            .field("path", &"<redacted-public-path>")
            .field("content_type", &self.content_type)
            .field("supports_range", &self.supports_range)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for CaptionTrack {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptionTrack")
            .field("path", &"<redacted-public-path>")
            .field("language", &self.language)
            .field("label", &self.label)
            .field("default", &self.default)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryAdvice {
    Never,
    Later,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    pub retry: RetryAdvice,
}

impl fmt::Debug for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiError")
            .field("code", &self.code)
            .field("message", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("retry", &self.retry)
            .finish()
    }
}

impl ApiError {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.code.is_empty()
            || self.code.len() > 64
            || !self
                .code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || self.message.is_empty()
            || self.message.len() > 256
            || self.message.chars().any(char::is_control)
            || self.request_id.as_ref().is_some_and(|request_id| {
                request_id.is_empty()
                    || request_id.len() > 128
                    || !request_id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
            })
        {
            return Err(ClientError::new(ClientErrorCode::InvalidContract));
        }
        Ok(())
    }
}

fn validate_canonical(value: Option<&str>, origin: &FrameOrigin) -> Result<(), ClientError> {
    let value = value.ok_or_else(|| ClientError::new(ClientErrorCode::InvalidContract))?;
    let url = Url::parse(value).map_err(|_| ClientError::new(ClientErrorCode::InvalidContract))?;
    let public_id = url.path().strip_prefix("/s/").filter(|identifier| {
        !identifier.is_empty()
            && identifier.len() <= 128
            && !identifier.contains('/')
            && identifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    });
    if !origin.is_same_origin_url(value)
        || public_id.is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
    }
    Ok(())
}

fn validate_playback(playback: &PlaybackDescriptor) -> Result<(), ClientError> {
    if !approved_playback_path(&playback.path)
        || !approved_media_type(&playback.content_type)
        || playback.captions.len() > 32
    {
        return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
    }
    for caption in &playback.captions {
        if !approved_caption_path(&caption.path)
            || caption.language.is_empty()
            || caption.language.len() > 35
            || !caption
                .language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || caption.label.is_empty()
            || caption.label.len() > 80
            || caption.label.chars().any(char::is_control)
        {
            return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
        }
    }
    Ok(())
}

fn approved_playback_path(path: &str) -> bool {
    approved_public_path_suffix(path).is_some_and(
        |segments| matches!(segments.as_slice(), [identifier, "media"] if safe_segment(identifier)),
    )
}

fn approved_caption_path(path: &str) -> bool {
    approved_public_path_suffix(path).is_some_and(|segments| {
        matches!(segments.as_slice(), [identifier, "captions", track]
            if safe_segment(identifier) && safe_segment(track))
    })
}

fn approved_public_path_suffix(path: &str) -> Option<Vec<&str>> {
    if !path.is_ascii()
        || path.contains(['?', '#', '\\', '%'])
        || path.contains("..")
        || path.chars().any(char::is_control)
    {
        return None;
    }
    path.strip_prefix("/api/v1/public/shares/")
        .map(|suffix| suffix.split('/').collect())
}

fn safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 128
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn approved_media_type(value: &str) -> bool {
    value == "application/vnd.apple.mpegurl"
        || value.strip_prefix("video/").is_some_and(|subtype| {
            !subtype.is_empty()
                && subtype.len() <= 64
                && subtype
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEALTH: &str = include_str!("../../../fixtures/frame-api/v1/health.ok.json");
    const HEALTH_ADDITIVE: &str =
        include_str!("../../../fixtures/frame-api/v1/health.additive.json");
    const SHARE_PUBLIC: &str = include_str!("../../../fixtures/frame-api/v1/share.public.json");
    const SHARE_PROCESSING: &str =
        include_str!("../../../fixtures/frame-api/v1/share.processing.json");
    const SHARE_UNAVAILABLE: &str =
        include_str!("../../../fixtures/frame-api/v1/share.unavailable.json");
    const SHARE_PRIVATE: &str = include_str!("../../../fixtures/frame-api/v1/share.private.json");
    const SHARE_DELETED: &str = include_str!("../../../fixtures/frame-api/v1/share.deleted.json");
    const SHARE_FAILED: &str = include_str!("../../../fixtures/frame-api/v1/share.failed.json");
    const ERROR: &str = include_str!("../../../fixtures/frame-api/v1/error.json");
    const COMPATIBILITY_CASES: &str =
        include_str!("../../../fixtures/cross-repo-preview/v1/compatibility-cases.json");

    fn origin() -> FrameOrigin {
        FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("canonical origin")
    }

    #[test]
    fn health_tolerates_additive_fields_and_unknown_capabilities() {
        for fixture in [HEALTH, HEALTH_ADDITIVE] {
            let health: Health = serde_json::from_str(fixture).expect("health fixture");
            health.validate().expect("valid health contract");
        }
        let additive: Health = serde_json::from_str(HEALTH_ADDITIVE).expect("additive health");
        assert!(additive.capabilities.supports("future_safe_capability"));
        assert!(Capabilities::from_names(vec!["public_share_summary".into()]).is_ok());
        assert!(Capabilities::from_names(vec!["NOT_SAFE".into()]).is_err());
    }

    #[test]
    fn incompatible_major_fails_closed() {
        let mut health: Health = serde_json::from_str(HEALTH).expect("health fixture");
        health.api_version.major = CONTRACT_MAJOR + 1;
        let error = health.validate().expect_err("major must be rejected");
        assert_eq!(error.code(), ClientErrorCode::IncompatibleVersion);
    }

    #[test]
    fn seeded_producer_changes_match_the_current_consumer_contract() {
        let matrix: serde_json::Value =
            serde_json::from_str(COMPATIBILITY_CASES).expect("compatibility matrix");
        assert_eq!(matrix["schema_version"], 1);
        let cases = matrix["cases"].as_array().expect("case array");
        let expected_ids = [
            "additive_unknown_health_field",
            "breaking_required_field_removal",
            "breaking_major_version_change",
            "breaking_release_type_change",
            "breaking_status_semantic_change",
            "breaking_public_media_path_change",
        ];
        assert_eq!(cases.len(), expected_ids.len());

        for (case, expected_id) in cases.iter().zip(expected_ids) {
            assert_eq!(case["id"], expected_id);
            let resource = case["resource"].as_str().expect("resource");
            let source = match resource {
                "health" => HEALTH,
                "public_share" => SHARE_PUBLIC,
                other => panic!("unknown compatibility resource {other}"),
            };
            let mut payload: serde_json::Value =
                serde_json::from_str(source).expect("base fixture");
            apply_seeded_mutation(&mut payload, case);

            let accepted = match resource {
                "health" => serde_json::from_value::<Health>(payload)
                    .ok()
                    .is_some_and(|value| value.validate().is_ok()),
                "public_share" => serde_json::from_value::<PublicShareSummary>(payload)
                    .ok()
                    .is_some_and(|value| value.validate(&origin()).is_ok()),
                _ => unreachable!(),
            };
            let expected_accept = case["expected_current_consumer"] == "accept";
            assert_eq!(
                accepted, expected_accept,
                "current consumer result drifted for {expected_id}"
            );
            assert_eq!(
                case["expected_last_released_consumer"], case["expected_current_consumer"],
                "v1 compatibility expectations must agree across consumer generations"
            );
            assert_eq!(
                case["classification"],
                if expected_accept {
                    "compatible"
                } else {
                    "breaking"
                }
            );
        }
    }

    fn apply_seeded_mutation(payload: &mut serde_json::Value, case: &serde_json::Value) {
        let pointer = case["pointer"].as_str().expect("mutation pointer");
        let (parent_pointer, key) = pointer.rsplit_once('/').expect("object pointer");
        let parent = payload
            .pointer_mut(parent_pointer)
            .and_then(serde_json::Value::as_object_mut)
            .expect("mutation parent");
        match case["operation"].as_str().expect("mutation operation") {
            "add" | "replace" => {
                parent.insert(key.to_owned(), case["value"].clone());
            }
            "remove" => {
                assert!(parent.remove(key).is_some(), "mutation target missing");
            }
            other => panic!("unknown compatibility mutation {other}"),
        }
    }

    #[test]
    fn public_and_processing_fixtures_validate() {
        let public: PublicShareSummary =
            serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        public.validate(&origin()).expect("public contract");
        let processing: PublicShareSummary =
            serde_json::from_str(SHARE_PROCESSING).expect("processing fixture");
        processing.validate(&origin()).expect("processing contract");
        assert_eq!(
            processing.processing_status,
            Some(InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: false,
                error: None,
            })
        );
    }

    #[test]
    fn instant_ui_progress_enforces_closed_phase_error_and_terminal_states() {
        let valid = [
            InstantUiProgressV1::new(InstantUiPhaseV1::Recording, Some(10_000), false, None),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::LocallyRecoverable,
                Some(2_500),
                true,
                Some(InstantUiErrorCodeV1::NetworkOffline),
            ),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::Uploading,
                Some(10_000),
                true,
                Some(InstantUiErrorCodeV1::UploadDelayed),
            ),
            InstantUiProgressV1::new(InstantUiPhaseV1::Finalizing, Some(10_000), false, None),
            InstantUiProgressV1::new(InstantUiPhaseV1::ShareReady, Some(10_000), false, None),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::Cancelled,
                None,
                false,
                Some(InstantUiErrorCodeV1::RecordingCancelled),
            ),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::RecoveryRequired,
                None,
                false,
                Some(InstantUiErrorCodeV1::RecordingRecoveryRequired),
            ),
        ];
        assert!(valid.into_iter().all(|result| result.is_ok()));

        let invalid = [
            InstantUiProgressV1 {
                schema_version: 2,
                phase: InstantUiPhaseV1::Uploading,
                progress_basis_points: None,
                retrying: false,
                error: None,
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Uploading,
                progress_basis_points: Some(10_001),
                retrying: false,
                error: None,
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Uploading,
                progress_basis_points: None,
                retrying: true,
                error: None,
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Uploading,
                progress_basis_points: None,
                retrying: false,
                error: Some(InstantUiErrorCodeV1::UploadDelayed),
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: true,
                error: Some(InstantUiErrorCodeV1::UploadDelayed),
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::ShareReady,
                progress_basis_points: None,
                retrying: false,
                error: None,
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::ShareReady,
                progress_basis_points: Some(10_000),
                retrying: true,
                error: Some(InstantUiErrorCodeV1::FinalizeDelayed),
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Cancelled,
                progress_basis_points: Some(10_000),
                retrying: false,
                error: Some(InstantUiErrorCodeV1::RecordingCancelled),
            },
            InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::RecoveryRequired,
                progress_basis_points: None,
                retrying: false,
                error: None,
            },
        ];
        assert!(invalid.into_iter().all(|status| status.validate().is_err()));
    }

    #[test]
    fn public_processing_projection_is_coarse_additive_and_debug_redacted() {
        let valid = [
            InstantUiProgressV1::new(InstantUiPhaseV1::Uploading, None, false, None)
                .expect("uploading"),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::Uploading,
                Some(7_500),
                true,
                Some(InstantUiErrorCodeV1::UploadDelayed),
            )
            .expect("delayed upload"),
            InstantUiProgressV1::new(InstantUiPhaseV1::Finalizing, Some(10_000), false, None)
                .expect("finalizing"),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::Finalizing,
                None,
                true,
                Some(InstantUiErrorCodeV1::FinalizeDelayed),
            )
            .expect("delayed finalize"),
        ];
        assert!(
            valid
                .iter()
                .all(|status| status.validate_for_public_share().is_ok())
        );

        for status in [
            InstantUiProgressV1::new(InstantUiPhaseV1::Recording, None, false, None)
                .expect("recording"),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::LocallyRecoverable,
                None,
                true,
                Some(InstantUiErrorCodeV1::NetworkOffline),
            )
            .expect("offline"),
            InstantUiProgressV1::new(
                InstantUiPhaseV1::RecoveryRequired,
                None,
                false,
                Some(InstantUiErrorCodeV1::LocalStorageFull),
            )
            .expect("storage"),
        ] {
            assert_eq!(
                status
                    .validate_for_public_share()
                    .expect_err("private phase or error must fail")
                    .code(),
                ClientErrorCode::PrivacyViolation
            );
        }

        let debug = format!("{:?}", valid[1]);
        for private in ["Uploading", "7500", "UploadDelayed"] {
            assert!(!debug.contains(private));
        }

        let mut additive: serde_json::Value =
            serde_json::from_str(SHARE_PROCESSING).expect("processing fixture");
        additive["processing_status"]["future_safe_hint"] = serde_json::json!("ignored");
        let additive: PublicShareSummary =
            serde_json::from_value(additive).expect("additive nested field");
        additive.validate(&origin()).expect("additive contract");
    }

    #[test]
    fn processing_status_cannot_cross_public_or_unavailable_privacy_boundaries() {
        let processing: PublicShareSummary =
            serde_json::from_str(SHARE_PROCESSING).expect("processing fixture");
        let status = processing.processing_status.expect("processing status");

        let mut public: PublicShareSummary =
            serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        public.processing_status = Some(status);
        assert_eq!(
            public
                .validate(&origin())
                .expect_err("ready share cannot carry processing state")
                .code(),
            ClientErrorCode::InvalidContract
        );

        let mut unavailable: PublicShareSummary =
            serde_json::from_str(SHARE_UNAVAILABLE).expect("unavailable fixture");
        unavailable.processing_status = Some(status);
        assert_eq!(
            unavailable
                .validate(&origin())
                .expect_err("unavailable share cannot carry processing state")
                .code(),
            ClientErrorCode::PrivacyViolation
        );

        let mut legacy: serde_json::Value =
            serde_json::from_str(SHARE_PROCESSING).expect("processing fixture");
        legacy
            .as_object_mut()
            .expect("processing object")
            .remove("processing_status");
        let legacy: PublicShareSummary =
            serde_json::from_value(legacy).expect("legacy processing body");
        legacy
            .validate(&origin())
            .expect("missing additive status remains compatible");
    }

    #[test]
    fn every_non_public_state_has_identical_wire_data() {
        assert_eq!(SHARE_UNAVAILABLE, SHARE_PRIVATE);
        assert_eq!(SHARE_UNAVAILABLE, SHARE_DELETED);
        assert_eq!(SHARE_UNAVAILABLE, SHARE_FAILED);
        let unavailable: PublicShareSummary =
            serde_json::from_str(SHARE_UNAVAILABLE).expect("unavailable fixture");
        unavailable
            .validate(&origin())
            .expect("unavailable contract");
        assert!(unavailable.title.is_none());
        assert!(unavailable.playback.is_none());
    }

    #[test]
    fn secret_bearing_public_fields_fail_validation_and_debug_is_redacted() {
        let mut public: PublicShareSummary =
            serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        public.playback.as_mut().expect("playback").path =
            "/api/v1/public/shares/demo/object?X-Amz-Signature=secret".into();
        let error = public
            .validate(&origin())
            .expect_err("signed path must fail");
        assert_eq!(error.code(), ClientErrorCode::PrivacyViolation);
        assert!(!format!("{public:?}").contains("secret"));

        let api_error: ApiError = serde_json::from_str(ERROR).expect("error fixture");
        api_error.validate().expect("valid public error");
        assert!(!format!("{api_error:?}").contains(&api_error.message));
    }

    #[test]
    fn canonical_share_and_error_fields_reject_path_or_log_confusion() {
        let mut public: PublicShareSummary =
            serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        for canonical in [
            "https://frame.engmanager.xyz/s/demo/extra",
            "https://frame.engmanager.xyz/s/%2e%2e/private",
            "https://frame.engmanager.xyz/s/demo?token=secret",
        ] {
            public.canonical_url = Some(canonical.into());
            assert!(public.validate(&origin()).is_err(), "accepted {canonical}");
        }

        let mut api_error: ApiError = serde_json::from_str(ERROR).expect("error fixture");
        api_error.message = "safe\nforged-log-line".into();
        assert!(api_error.validate().is_err());
        api_error.message = "Safe public message.".into();
        api_error.request_id = Some("../../private".into());
        assert!(api_error.validate().is_err());
    }

    #[test]
    fn playback_and_caption_paths_are_exact_public_capabilities() {
        for path in [
            "/api/v1/public/shares/demo/media/extra",
            "/api/v1/public/shares/demo/private",
            "/api/v1/public/shares/demo/captions/en",
            "/api/v1/public/shares/demo/object-media",
        ] {
            let mut public: PublicShareSummary =
                serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
            public.playback.as_mut().expect("playback").path = path.into();
            assert!(public.validate(&origin()).is_err(), "accepted {path}");
        }

        let mut public: PublicShareSummary =
            serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        let playback = public.playback.as_mut().expect("playback");
        playback.content_type = "video/mp4\nset-cookie: secret".into();
        assert!(public.validate(&origin()).is_err());

        public = serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        let caption = public
            .playback
            .as_mut()
            .expect("playback")
            .captions
            .first_mut()
            .expect("caption");
        caption.path = "/api/v1/public/shares/demo/captions/en/secret".into();
        assert!(public.validate(&origin()).is_err());
    }

    #[test]
    fn unavailable_contract_rejects_seeded_private_metadata() {
        let mut unavailable: PublicShareSummary =
            serde_json::from_str(SHARE_UNAVAILABLE).expect("unavailable fixture");
        unavailable.title = Some("Private quarterly review".into());
        let error = unavailable
            .validate(&origin())
            .expect_err("private title must fail");
        assert_eq!(error.code(), ClientErrorCode::PrivacyViolation);
    }
}
