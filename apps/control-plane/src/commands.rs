use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::contracts::API_SCHEMA_VERSION;

pub const MAX_SINGLE_UPLOAD_BYTES: u64 = 100 * 1_024 * 1_024;
pub const COMMAND_TTL_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadIntentResponse {
    pub schema_version: u16,
    pub upload_id: String,
    pub state: String,
    pub upload_path: String,
    pub expected_bytes: u64,
    pub content_type: String,
    pub checksum_header: &'static str,
}

impl UploadIntentResponse {
    pub fn new(upload_id: String, expected_bytes: u64, content_type: String) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            upload_path: format!("/api/v1/uploads/{upload_id}/content"),
            upload_id,
            state: "initiated".into(),
            expected_bytes,
            content_type,
            checksum_header: "x-content-sha256",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadStatusResponse {
    pub schema_version: u16,
    pub upload_id: String,
    pub state: String,
    pub expected_bytes: u64,
    pub received_bytes: u64,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaJobResponse {
    pub schema_version: u16,
    pub job_id: String,
    pub state: String,
    pub profile: String,
    pub executor: String,
    pub status_path: String,
}

impl MediaJobResponse {
    pub fn new(job_id: String, profile: String, executor: String) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            status_path: format!("/api/v1/media-jobs/{job_id}"),
            job_id,
            state: "queued".into(),
            profile,
            executor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaJobStatusResponse {
    pub schema_version: u16,
    pub job_id: String,
    pub state: String,
    pub profile: String,
    pub executor: Option<String>,
    pub progress_basis_points: Option<u16>,
    pub attempt: u32,
    pub cancel_requested: bool,
    pub error_class: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoResponse {
    pub schema_version: u16,
    pub video_id: String,
    pub state: String,
    pub privacy: String,
    pub revision: u64,
    pub upload_intents_path: String,
    pub public_share_path: Option<String>,
}

impl VideoResponse {
    pub fn new(video_id: String) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            upload_intents_path: "/api/v1/uploads/intents".into(),
            video_id,
            state: "pending".into(),
            privacy: "private".into(),
            revision: 0,
            public_share_path: None,
        }
    }
}

#[derive(Deserialize)]
pub struct StoredCommandRow {
    pub command_type: String,
    pub request_digest: String,
    pub response_status: Option<i32>,
    pub response_json: Option<String>,
    pub expires_at_ms: i64,
}

#[derive(Deserialize)]
pub struct ApiKeyRow {
    pub user_id: String,
    pub scopes_json: String,
}

#[derive(Deserialize)]
pub struct MembershipRow {
    pub role: String,
}

#[derive(Deserialize)]
pub struct VideoMutationRow {
    pub id: String,
    pub owner_id: String,
    pub state: String,
    pub privacy: String,
    pub revision: i64,
    pub actor_role: String,
    pub actor_manages_space: i64,
}

impl VideoMutationRow {
    pub fn actor_can_update(&self, actor_id: &str) -> bool {
        matches!(self.actor_role.as_str(), "owner" | "admin")
            || (self.actor_role == "member"
                && (self.owner_id == actor_id || self.actor_manages_space == 1))
    }

    pub fn public_response(&self) -> Option<VideoResponse> {
        let revision = u64::try_from(self.revision).ok()?;
        Some(VideoResponse {
            schema_version: API_SCHEMA_VERSION,
            video_id: self.id.clone(),
            state: self.state.clone(),
            privacy: self.privacy.clone(),
            revision,
            upload_intents_path: "/api/v1/uploads/intents".into(),
            public_share_path: (self.privacy == "public")
                .then(|| format!("/api/v1/public/shares/{}", self.id)),
        })
    }
}

#[derive(Deserialize)]
pub struct VideoScopeRow {
    pub id: String,
}

#[derive(Deserialize)]
pub struct UploadRow {
    pub id: String,
    pub organization_id: String,
    pub video_id: String,
    pub state: String,
    pub expected_bytes: i64,
    pub received_bytes: i64,
    pub source_object_key: String,
    pub source_version: i64,
    pub content_type: String,
    pub checksum_sha256: Option<String>,
}

impl UploadRow {
    pub fn public_status(&self) -> Option<UploadStatusResponse> {
        Some(UploadStatusResponse {
            schema_version: API_SCHEMA_VERSION,
            upload_id: self.id.clone(),
            state: self.state.clone(),
            expected_bytes: u64::try_from(self.expected_bytes).ok()?,
            received_bytes: u64::try_from(self.received_bytes).ok()?,
            content_type: self.content_type.clone(),
        })
    }
}

#[derive(Deserialize)]
pub struct SourceObjectRow {
    pub object_key: String,
    pub bytes: i64,
    pub checksum_sha256: Option<String>,
    pub content_type: String,
}

#[derive(Deserialize)]
pub struct IntegrationRow {
    pub id: String,
    pub capabilities_json: String,
}

impl IntegrationRow {
    pub fn supports_single_put(&self) -> bool {
        serde_json::from_str::<serde_json::Value>(&self.capabilities_json)
            .ok()
            .and_then(|value| value.get("conditional_put").and_then(|flag| flag.as_bool()))
            == Some(true)
    }
}

#[derive(Deserialize)]
pub struct MediaJobRow {
    pub id: String,
    pub state: String,
    pub profile: String,
    pub selected_executor: Option<String>,
    pub progress_basis_points: Option<i64>,
    pub attempt: i64,
    pub cancel_requested: i64,
    pub error_class: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl MediaJobRow {
    pub fn public_status(&self) -> Option<MediaJobStatusResponse> {
        let progress_basis_points = self
            .progress_basis_points
            .map(u16::try_from)
            .transpose()
            .ok()?;
        Some(MediaJobStatusResponse {
            schema_version: API_SCHEMA_VERSION,
            job_id: self.id.clone(),
            state: self.state.clone(),
            profile: self.profile.clone(),
            executor: self.selected_executor.clone(),
            progress_basis_points,
            attempt: u32::try_from(self.attempt).ok()?,
            cancel_requested: match self.cancel_requested {
                0 => false,
                1 => true,
                _ => return None,
            },
            error_class: self.error_class.clone(),
            created_at_ms: u64::try_from(self.created_at_ms).ok()?,
            updated_at_ms: u64::try_from(self.updated_at_ms).ok()?,
        })
    }
}

pub fn request_digest<T: Serialize>(command_type: &str, value: &T) -> Result<String, ()> {
    if command_type.is_empty() || command_type.len() > 64 || !command_type.is_ascii() {
        return Err(());
    }
    let serialized = serde_json::to_vec(value).map_err(|_| ())?;
    let mut hasher = Sha256::new();
    hasher.update(command_type.as_bytes());
    hasher.update([0]);
    hasher.update(serialized);
    Ok(hex(&hasher.finalize()))
}

pub fn digest_identifier(command_type: &str, identifier: &str) -> Result<String, ()> {
    request_digest(command_type, &identifier)
}

pub fn digest_credential(credential: &str) -> String {
    hex(&Sha256::digest(credential.as_bytes()))
}

pub fn source_object_key(
    tenant_id: &str,
    video_id: &str,
    role: &str,
    object_version: u32,
) -> String {
    format!("tenants/{tenant_id}/videos/{video_id}/{role}/v{object_version}/payload")
}

pub fn derivative_object_key(
    tenant_id: &str,
    video_id: &str,
    profile: &str,
    source_version: u32,
) -> String {
    format!("tenants/{tenant_id}/videos/{video_id}/derivatives/{profile}/v{source_version}/output")
}

pub fn profile_kind(profile: &str) -> Option<&'static str> {
    match profile {
        "thumbnail_v1" => Some("frame"),
        "preview_v1" => Some("optimized_video"),
        "spritesheet_v1" => Some("spritesheet"),
        "audio_v1" => Some("audio"),
        _ => None,
    }
}

pub fn parse_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (nibble(chunk[0])? << 4) | nibble(chunk[1])?;
    }
    Some(output)
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_are_stable_and_domain_separated() {
        let payload = serde_json::json!({"video_id": "video", "version": 1});
        let first = request_digest("upload_intent", &payload).expect("digest");
        assert_eq!(first.len(), 64);
        assert_eq!(
            first,
            request_digest("upload_intent", &payload).expect("repeat")
        );
        assert_ne!(
            first,
            request_digest("media_job", &payload).expect("domain")
        );
    }

    #[test]
    fn checksum_parser_is_strict_lowercase_hex() {
        let digest = "0123456789abcdef".repeat(4);
        let bytes = parse_sha256(&digest).expect("checksum");
        assert_eq!(hex(&bytes), digest);
        assert!(parse_sha256(&"A".repeat(64)).is_none());
        assert!(parse_sha256("short").is_none());
    }

    #[test]
    fn public_responses_never_expose_storage_keys() {
        let response = UploadIntentResponse::new(
            "018f47a6-7b1c-7f55-8f39-8f8a86900111".into(),
            10,
            "video/webm".into(),
        );
        let json = serde_json::to_string(&response).expect("json");
        assert!(!json.contains("object_key"));
        assert!(!json.contains("tenants/"));
        assert!(json.contains("x-content-sha256"));
    }

    #[test]
    fn video_update_policy_requires_ownership_or_space_management_for_members() {
        let actor = "018f47a6-7b1c-7f55-8f39-8f8a86900101";
        let mut row = VideoMutationRow {
            id: "018f47a6-7b1c-7f55-8f39-8f8a86900104".into(),
            owner_id: "018f47a6-7b1c-7f55-8f39-8f8a86900109".into(),
            state: "ready".into(),
            privacy: "private".into(),
            revision: 3,
            actor_role: "member".into(),
            actor_manages_space: 0,
        };
        assert!(!row.actor_can_update(actor));
        row.owner_id = actor.into();
        assert!(row.actor_can_update(actor));
        row.owner_id = "018f47a6-7b1c-7f55-8f39-8f8a86900109".into();
        row.actor_manages_space = 1;
        assert!(row.actor_can_update(actor));
        row.actor_role = "viewer".into();
        assert!(!row.actor_can_update(actor));
        row.actor_role = "admin".into();
        assert!(row.actor_can_update(actor));
    }

    #[test]
    fn object_keys_are_deterministic_and_tenant_scoped() {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
        let video = "018f47a6-7b1c-7f55-8f39-8f8a86900104";
        let source = source_object_key(tenant, video, "source", 2);
        let output = derivative_object_key(tenant, video, "preview_v1", 2);
        assert!(source.starts_with(&format!("tenants/{tenant}/videos/{video}/")));
        assert!(output.contains("/derivatives/preview_v1/"));
        assert_ne!(source, output);
    }

    #[test]
    fn storage_capability_requires_an_explicit_boolean() {
        let supported = IntegrationRow {
            id: "integration".into(),
            capabilities_json: r#"{"conditional_put":true}"#.into(),
        };
        assert!(supported.supports_single_put());
        for capabilities_json in [
            r#"{"conditional_put":false}"#,
            r#"{"conditional_put":"true"}"#,
            "{}",
            "not-json",
        ] {
            let unsupported = IntegrationRow {
                id: "integration".into(),
                capabilities_json: capabilities_json.into(),
            };
            assert!(!unsupported.supports_single_put());
        }
    }
}
