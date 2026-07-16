use serde::{Deserialize, Serialize};

pub const API_SCHEMA_VERSION: u16 = 1;
pub const MAX_COMMAND_BODY_BYTES: u64 = 32 * 1_024;
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationCode {
    SchemaVersion,
    Identifier,
    Size,
    ContentType,
    ObjectRole,
    ObjectVersion,
    Profile,
}

impl ValidationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaVersion => "invalid_schema_version",
            Self::Identifier => "invalid_identifier",
            Self::Size => "invalid_size",
            Self::ContentType => "invalid_content_type",
            Self::ObjectRole => "invalid_object_role",
            Self::ObjectVersion => "invalid_object_version",
            Self::Profile => "invalid_profile",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadIntentRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub video_id: String,
    pub role: String,
    pub object_version: u32,
    pub expected_bytes: u64,
    pub content_type: String,
}

impl UploadIntentRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        if self.schema_version != API_SCHEMA_VERSION {
            return Err(ValidationCode::SchemaVersion);
        }
        if !valid_uuid(&self.tenant_id) || !valid_uuid(&self.video_id) {
            return Err(ValidationCode::Identifier);
        }
        if !matches!(self.role.as_str(), "source" | "segment" | "import") {
            return Err(ValidationCode::ObjectRole);
        }
        if self.object_version == 0 {
            return Err(ValidationCode::ObjectVersion);
        }
        if self.expected_bytes == 0 || self.expected_bytes > MAX_SAFE_INTEGER {
            return Err(ValidationCode::Size);
        }
        if !valid_content_type(&self.content_type) {
            return Err(ValidationCode::ContentType);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MediaJobRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub video_id: String,
    pub source_version: u32,
    pub profile: String,
}

impl MediaJobRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        if self.schema_version != API_SCHEMA_VERSION {
            return Err(ValidationCode::SchemaVersion);
        }
        if !valid_uuid(&self.tenant_id) || !valid_uuid(&self.video_id) {
            return Err(ValidationCode::Identifier);
        }
        if self.source_version == 0 {
            return Err(ValidationCode::ObjectVersion);
        }
        if !matches!(
            self.profile.as_str(),
            "thumbnail_v1" | "preview_v1" | "spritesheet_v1" | "audio_v1"
        ) {
            return Err(ValidationCode::Profile);
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct DiscoveryResponse {
    pub service: &'static str,
    pub current_version: &'static str,
    pub supported_versions: [&'static str; 1],
    pub capabilities: &'static str,
}

impl Default for DiscoveryResponse {
    fn default() -> Self {
        Self {
            service: "frame-control-plane",
            current_version: "v1",
            supported_versions: ["v1"],
            capabilities: "/api/v1",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CapabilitiesResponse {
    pub schema_version: u16,
    pub api_version: &'static str,
    pub public_share_read: &'static str,
    pub upload_intents: &'static str,
    pub media_jobs: &'static str,
    pub media_executor_selection: &'static str,
    pub native_capture: &'static str,
    pub migration_controls: &'static str,
    pub cors: &'static str,
    pub managed_stream_library: bool,
    pub max_command_body_bytes: u64,
}

impl Default for CapabilitiesResponse {
    fn default() -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            api_version: "v1",
            public_share_read: "read_only",
            upload_intents: "authenticated_d1_r2_single_put",
            media_jobs: "fail_closed_pending_runtime_selection",
            media_executor_selection: "server_controlled",
            native_capture: "external_native_executor",
            migration_controls: "authenticated_read_only",
            cors: "disabled_same_origin_only",
            managed_stream_library: false,
            max_command_body_bytes: MAX_COMMAND_BODY_BYTES,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuthorityResponse {
    pub schema_version: u16,
    pub phase: String,
    pub authority: String,
    pub epoch: u64,
    pub mutations_enabled: bool,
}

#[must_use]
pub fn valid_uuid(value: &str) -> bool {
    if value.len() != 36 || value == "00000000-0000-0000-0000-000000000000" {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
        }
    })
}

#[must_use]
pub fn valid_idempotency_key(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[must_use]
pub fn valid_content_type(value: &str) -> bool {
    if value.len() > 127 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return false;
    }
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'/' | b'+' | b'-' | b'.')
        })
}

#[must_use]
pub fn sanitized_public_title(value: &str) -> String {
    let title = value
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect::<String>();
    let title = title.trim();
    if title.is_empty() {
        "Untitled recording".into()
    } else {
        title.to_owned()
    }
}

#[must_use]
pub fn normalize_cf_ray(value: Option<&str>, fallback_a: u64, fallback_b: u64) -> String {
    if let Some(value) = value.filter(|value| {
        (8..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        format!("r-{value}")
    } else {
        format!("r-{fallback_a:016x}{fallback_b:016x}")
    }
}

#[must_use]
#[cfg(test)]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

#[must_use]
pub fn origin_allowed(origin: &str, expected_host: &str, local: bool) -> bool {
    if local {
        let Some(remainder) = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
        else {
            return false;
        };
        let authority = remainder.split('/').next().unwrap_or_default();
        let host = authority.split(':').next().unwrap_or_default();
        remainder == authority
            && (host == expected_host || matches!(host, "localhost" | "127.0.0.1"))
    } else {
        origin == format!("https://{expected_host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f123";
    const VIDEO: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f124";

    #[test]
    fn identifiers_idempotency_and_content_types_are_bounded() {
        assert!(valid_uuid(TENANT));
        assert!(!valid_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(!valid_uuid("018F47A6-7B1C-7F55-8F39-8F8A8690F123"));
        assert!(valid_idempotency_key("tenant:command-001"));
        assert!(!valid_idempotency_key("short"));
        assert!(valid_content_type("video/mp4"));
        assert!(!valid_content_type("video/mp4;token=secret"));
    }

    #[test]
    fn upload_contract_rejects_unsafe_or_unbounded_fields() {
        let mut request = UploadIntentRequest {
            schema_version: 1,
            tenant_id: TENANT.into(),
            video_id: VIDEO.into(),
            role: "source".into(),
            object_version: 1,
            expected_bytes: 1_024,
            content_type: "video/mp4".into(),
        };
        assert_eq!(request.validate(), Ok(()));
        request.expected_bytes = MAX_SAFE_INTEGER + 1;
        assert_eq!(request.validate(), Err(ValidationCode::Size));
        request.expected_bytes = 1_024;
        request.role = "../../private".into();
        assert_eq!(request.validate(), Err(ValidationCode::ObjectRole));
    }

    #[test]
    fn media_contract_does_not_allow_clients_to_choose_an_executor() {
        let mut request = MediaJobRequest {
            schema_version: 1,
            tenant_id: TENANT.into(),
            video_id: VIDEO.into(),
            source_version: 1,
            profile: "thumbnail_v1".into(),
        };
        assert_eq!(request.validate(), Ok(()));
        request.profile = "cloudflare_media".into();
        assert_eq!(request.validate(), Err(ValidationCode::Profile));
    }

    #[test]
    fn titles_are_bounded_and_control_characters_are_removed() {
        assert_eq!(sanitized_public_title("  Demo\nTitle\0  "), "DemoTitle");
        assert_eq!(sanitized_public_title("\n\0"), "Untitled recording");
        assert_eq!(sanitized_public_title(&"x".repeat(200)).len(), 160);
    }

    #[test]
    fn request_ids_use_only_trusted_ray_shape_or_an_opaque_fallback() {
        assert_eq!(
            normalize_cf_ray(Some("abc12345-SJC"), 1, 2),
            "r-abc12345-SJC"
        );
        assert_eq!(
            normalize_cf_ray(Some("spoofed/request"), 1, 2),
            "r-00000000000000010000000000000002"
        );
    }

    #[test]
    fn secret_comparison_and_origin_policy_fail_closed() {
        assert!(constant_time_eq(b"same-secret", b"same-secret"));
        assert!(!constant_time_eq(b"same-secret", b"other-secret"));
        assert!(origin_allowed(
            "https://frame.engmanager.xyz",
            "frame.engmanager.xyz",
            false
        ));
        assert!(!origin_allowed(
            "https://evil.frame.engmanager.xyz",
            "frame.engmanager.xyz",
            false
        ));
        assert!(origin_allowed("http://localhost:3000", "localhost", true));
    }
}
