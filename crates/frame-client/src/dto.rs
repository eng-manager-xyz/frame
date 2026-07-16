use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{ClientError, ClientErrorCode, FrameOrigin};

pub const CONTRACT_MAJOR: u16 = 1;

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
                    || self
                        .description
                        .as_ref()
                        .is_some_and(|value| value.len() > 2_000)
                    || self
                        .duration_ms
                        .is_some_and(|duration| duration > 24 * 60 * 60 * 1_000)
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
            }
            ShareAvailability::Unavailable => {
                if self.title.is_some()
                    || self.description.is_some()
                    || self.canonical_url.is_some()
                    || self.duration_ms.is_some()
                    || self.playback.is_some()
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

fn validate_canonical(value: Option<&str>, origin: &FrameOrigin) -> Result<(), ClientError> {
    let value = value.ok_or_else(|| ClientError::new(ClientErrorCode::InvalidContract))?;
    let url = Url::parse(value).map_err(|_| ClientError::new(ClientErrorCode::InvalidContract))?;
    if !origin.is_same_origin_url(value)
        || !url.path().starts_with("/s/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
    }
    Ok(())
}

fn validate_playback(playback: &PlaybackDescriptor) -> Result<(), ClientError> {
    if !approved_public_path(&playback.path)
        || !(playback.content_type.starts_with("video/")
            || playback.content_type == "application/vnd.apple.mpegurl")
        || playback.captions.len() > 32
    {
        return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
    }
    for caption in &playback.captions {
        if !approved_public_path(&caption.path)
            || caption.language.is_empty()
            || caption.language.len() > 35
            || caption.label.is_empty()
            || caption.label.len() > 80
        {
            return Err(ClientError::new(ClientErrorCode::PrivacyViolation));
        }
    }
    Ok(())
}

fn approved_public_path(path: &str) -> bool {
    path.is_ascii()
        && path.starts_with("/api/v1/public/shares/")
        && !path.contains(['?', '#', '\\', '%'])
        && !path.contains("..")
        && !path.to_ascii_lowercase().contains("object")
        && !path.to_ascii_lowercase().contains("x-amz")
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
    }

    #[test]
    fn incompatible_major_fails_closed() {
        let mut health: Health = serde_json::from_str(HEALTH).expect("health fixture");
        health.api_version.major = CONTRACT_MAJOR + 1;
        let error = health.validate().expect_err("major must be rejected");
        assert_eq!(error.code(), ClientErrorCode::IncompatibleVersion);
    }

    #[test]
    fn public_and_processing_fixtures_validate() {
        let public: PublicShareSummary =
            serde_json::from_str(SHARE_PUBLIC).expect("public fixture");
        public.validate(&origin()).expect("public contract");
        let processing: PublicShareSummary =
            serde_json::from_str(SHARE_PROCESSING).expect("processing fixture");
        processing.validate(&origin()).expect("processing contract");
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
        assert!(!format!("{api_error:?}").contains(&api_error.message));
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
