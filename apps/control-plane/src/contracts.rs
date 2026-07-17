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
    Title,
    Privacy,
    Revision,
    LeaseToken,
    Checksum,
    TransferMode,
    Progress,
    FailureClass,
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
            Self::Title => "invalid_title",
            Self::Privacy => "invalid_privacy",
            Self::Revision => "invalid_revision",
            Self::LeaseToken => "invalid_lease_token",
            Self::Checksum => "invalid_checksum",
            Self::TransferMode => "invalid_transfer_mode",
            Self::Progress => "invalid_progress",
            Self::FailureClass => "invalid_failure_class",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateVideoRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub title: String,
}

impl CreateVideoRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        if self.schema_version != API_SCHEMA_VERSION {
            return Err(ValidationCode::SchemaVersion);
        }
        if !valid_uuid(&self.tenant_id) {
            return Err(ValidationCode::Identifier);
        }
        if self.title.trim().is_empty()
            || self.title.trim() != self.title
            || self.title.chars().count() > 160
            || self.title.chars().any(char::is_control)
        {
            return Err(ValidationCode::Title);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePrivacyRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub privacy: String,
    pub expected_revision: u64,
}

impl UpdatePrivacyRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        if self.schema_version != API_SCHEMA_VERSION {
            return Err(ValidationCode::SchemaVersion);
        }
        if !valid_uuid(&self.tenant_id) {
            return Err(ValidationCode::Identifier);
        }
        if !matches!(self.privacy.as_str(), "private" | "public") {
            return Err(ValidationCode::Privacy);
        }
        if self.expected_revision > MAX_SAFE_INTEGER {
            return Err(ValidationCode::Revision);
        }
        Ok(())
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
    #[serde(
        default = "default_upload_transfer_mode",
        skip_serializing_if = "upload_transfer_is_brokered"
    )]
    pub transfer_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
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
        match self.transfer_mode.as_str() {
            "brokered" if self.checksum_sha256.is_none() => {}
            "direct" if self.checksum_sha256.as_deref().is_some_and(valid_sha256) => {}
            "multipart" if self.checksum_sha256.as_deref().is_some_and(valid_sha256) => {}
            "direct" => return Err(ValidationCode::Checksum),
            "multipart" => return Err(ValidationCode::Checksum),
            "brokered" => return Err(ValidationCode::Checksum),
            _ => return Err(ValidationCode::TransferMode),
        }
        Ok(())
    }
}

fn default_upload_transfer_mode() -> String {
    "brokered".into()
}

fn upload_transfer_is_brokered(value: &String) -> bool {
    value == "brokered"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectUploadFinalizeRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub checksum_sha256: String,
}

impl DirectUploadFinalizeRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        if self.schema_version != API_SCHEMA_VERSION {
            return Err(ValidationCode::SchemaVersion);
        }
        if !valid_uuid(&self.tenant_id) {
            return Err(ValidationCode::Identifier);
        }
        if !valid_sha256(&self.checksum_sha256) {
            return Err(ValidationCode::Checksum);
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<MediaTransformRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedMediaMode {
    Video,
    Frame,
    Spritesheet,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaResizeFit {
    Contain,
    Cover,
    ScaleDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedMediaFormat {
    Mp4H264Aac,
    Jpeg,
    Png,
    M4aAac,
}

/// A normalized derivative profile. Clients describe the requested output;
/// they never select an executor, source URL, output key, or provider option.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MediaTransformRequest {
    pub schema_version: u16,
    pub profile_version: u16,
    pub mode: ManagedMediaMode,
    pub start_ms: u64,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: MediaResizeFit,
    pub image_count: Option<u16>,
    pub include_audio: bool,
    pub format: ManagedMediaFormat,
    pub max_output_bytes: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerClaimRequest {
    pub schema_version: u16,
    pub tenant_id: String,
}

impl WorkerClaimRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        validate_worker_tenant(self.schema_version, &self.tenant_id)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerHeartbeatRequest {
    pub schema_version: u16,
    pub tenant_id: String,
}

impl WorkerHeartbeatRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        validate_worker_tenant(self.schema_version, &self.tenant_id)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProgressRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub progress_basis_points: u16,
}

impl WorkerProgressRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        validate_worker_tenant(self.schema_version, &self.tenant_id)?;
        if self.progress_basis_points >= 10_000 {
            return Err(ValidationCode::Progress);
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCompleteRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub outputs: Vec<WorkerCompletedOutput>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCompletedOutput {
    pub ordinal: u16,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
}

impl WorkerCompleteRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        validate_worker_tenant(self.schema_version, &self.tenant_id)?;
        let [output] = self.outputs.as_slice() else {
            return Err(ValidationCode::Profile);
        };
        if output.ordinal != 0 || output.bytes == 0 || output.bytes > MAX_SAFE_INTEGER {
            return Err(ValidationCode::Size);
        }
        if !valid_sha256(&output.checksum_sha256) {
            return Err(ValidationCode::Checksum);
        }
        if !valid_content_type(&output.content_type) {
            return Err(ValidationCode::ContentType);
        }
        Ok(())
    }

    #[must_use]
    pub fn output(&self) -> Option<&WorkerCompletedOutput> {
        let [output] = self.outputs.as_slice() else {
            return None;
        };
        (output.ordinal == 0).then_some(output)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerFailRequest {
    pub schema_version: u16,
    pub tenant_id: String,
    pub error_class: String,
    pub retryable: bool,
}

impl WorkerFailRequest {
    pub fn validate(&self) -> Result<(), ValidationCode> {
        validate_worker_tenant(self.schema_version, &self.tenant_id)?;
        if !matches!(
            self.error_class.as_str(),
            "input_invalid"
                | "unsupported_media"
                | "pipeline_timeout"
                | "pipeline_failure"
                | "resource_limit"
                | "output_invalid"
                | "cancelled"
                | "transport_failure"
        ) {
            return Err(ValidationCode::FailureClass);
        }
        Ok(())
    }
}

fn validate_worker_tenant(schema_version: u16, tenant_id: &str) -> Result<(), ValidationCode> {
    if schema_version != API_SCHEMA_VERSION {
        return Err(ValidationCode::SchemaVersion);
    }
    if !valid_uuid(tenant_id) {
        return Err(ValidationCode::Identifier);
    }
    Ok(())
}

#[must_use]
pub fn valid_lease_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[must_use]
pub fn valid_sha256(value: &str) -> bool {
    valid_lease_token(value)
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
        if !media_profile_supported(&self.profile) {
            return Err(ValidationCode::Profile);
        }
        match (managed_profile(&self.profile), self.transform.as_ref()) {
            (true, Some(transform)) => transform.validate_for(&self.profile)?,
            (true, None) => return Err(ValidationCode::Profile),
            (false, Some(_)) => return Err(ValidationCode::Profile),
            (false, None) => {}
        }
        Ok(())
    }
}

impl MediaTransformRequest {
    pub fn validate_for(&self, profile: &str) -> Result<(), ValidationCode> {
        if self.schema_version != 1
            || self.profile_version == 0
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_SAFE_INTEGER
            || self.start_ms > MAX_SAFE_INTEGER
            || self
                .duration_ms
                .is_some_and(|value| value == 0 || value > MAX_SAFE_INTEGER)
            || self.width.is_some() != self.height.is_some()
            || self.width.is_some_and(|value| value == 0)
            || self.height.is_some_and(|value| value == 0)
        {
            return Err(ValidationCode::Profile);
        }
        let shape_matches = match profile {
            "optimized_clip_v1" => {
                self.mode == ManagedMediaMode::Video
                    && self.format == ManagedMediaFormat::Mp4H264Aac
                    && self.duration_ms.is_some()
                    && self.image_count.is_none()
            }
            "thumbnail_v1" => {
                self.mode == ManagedMediaMode::Frame
                    && matches!(
                        self.format,
                        ManagedMediaFormat::Jpeg | ManagedMediaFormat::Png
                    )
                    && self.duration_ms.is_none()
                    && self.image_count.is_none()
                    && !self.include_audio
            }
            "spritesheet_v1" => {
                self.mode == ManagedMediaMode::Spritesheet
                    && self.format == ManagedMediaFormat::Jpeg
                    && self.duration_ms.is_some()
                    && self.image_count.is_some_and(|count| count > 0)
                    && !self.include_audio
            }
            "audio_extract_v1" => {
                self.mode == ManagedMediaMode::Audio
                    && self.format == ManagedMediaFormat::M4aAac
                    && self.duration_ms.is_some()
                    && self.width.is_none()
                    && self.image_count.is_none()
                    && !self.include_audio
            }
            _ => false,
        };
        if !shape_matches {
            return Err(ValidationCode::Profile);
        }
        Ok(())
    }
}

#[must_use]
pub fn managed_profile(profile: &str) -> bool {
    matches!(
        profile,
        "optimized_clip_v1" | "thumbnail_v1" | "spritesheet_v1" | "audio_extract_v1"
    )
}

#[must_use]
pub fn media_profile_supported(profile: &str) -> bool {
    matches!(
        profile,
        "optimized_clip_v1"
            | "thumbnail_v1"
            | "spritesheet_v1"
            | "audio_extract_v1"
            | "probe_v1"
            | "audio_presence_v1"
            | "distribution_master_v1"
            | "animated_preview_v1"
            | "audio_normalize_v1"
            | "remux_repair_v1"
            | "segment_mux_v1"
            | "waveform_v1"
            | "composition_v1"
            | "normalize_v1"
            | "transcription_v1"
            | "ai_cleanup_v1"
            // Compatibility aliases are local-fake only and are rejected by
            // the production router before persistence.
            | "preview_v1"
            | "audio_v1"
    )
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
    pub video_lifecycle: &'static str,
    pub upload_intents: &'static str,
    pub upload_transfer_modes: [&'static str; 3],
    pub direct_upload_finalize: &'static str,
    pub multipart_upload: &'static str,
    pub instant_finalize: &'static str,
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
            video_lifecycle: "authenticated_d1_revision_fenced",
            upload_intents: "authenticated_d1_r2_single_put_and_multipart",
            upload_transfer_modes: ["brokered", "direct", "multipart"],
            direct_upload_finalize: "/api/v1/uploads/{upload_id}/finalize",
            multipart_upload: "/api/v1/uploads/{upload_id}/multipart",
            instant_finalize: "/api/v1/instant-recordings/{session_id}/finalize",
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

    const WORKER_TENANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900102";

    #[test]
    fn native_worker_contracts_are_bounded_and_fail_closed() {
        assert!(valid_lease_token(&"a".repeat(64)));
        assert!(!valid_lease_token(&"A".repeat(64)));
        assert!(!valid_lease_token("short"));

        assert!(
            WorkerClaimRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: WORKER_TENANT.into(),
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            WorkerProgressRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: WORKER_TENANT.into(),
                progress_basis_points: 10_000,
            }
            .validate(),
            Err(ValidationCode::Progress)
        );
        assert_eq!(
            WorkerCompleteRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: WORKER_TENANT.into(),
                outputs: vec![WorkerCompletedOutput {
                    ordinal: 0,
                    bytes: 1,
                    checksum_sha256: "A".repeat(64),
                    content_type: "image/png".into(),
                }],
            }
            .validate(),
            Err(ValidationCode::Checksum)
        );
        assert_eq!(
            WorkerFailRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: WORKER_TENANT.into(),
                error_class: "internal stack trace".into(),
                retryable: false,
            }
            .validate(),
            Err(ValidationCode::FailureClass)
        );
        assert!(
            WorkerFailRequest {
                schema_version: API_SCHEMA_VERSION,
                tenant_id: WORKER_TENANT.into(),
                error_class: "pipeline_failure".into(),
                retryable: true,
            }
            .validate()
            .is_ok()
        );
    }

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
            transfer_mode: "brokered".into(),
            checksum_sha256: None,
        };
        assert_eq!(request.validate(), Ok(()));
        let brokered_json = serde_json::to_value(&request).expect("brokered json");
        assert!(brokered_json.get("transfer_mode").is_none());
        assert!(brokered_json.get("checksum_sha256").is_none());
        request.expected_bytes = MAX_SAFE_INTEGER + 1;
        assert_eq!(request.validate(), Err(ValidationCode::Size));
        request.expected_bytes = 1_024;
        request.role = "../../private".into();
        assert_eq!(request.validate(), Err(ValidationCode::ObjectRole));

        request.role = "source".into();
        request.transfer_mode = "direct".into();
        assert_eq!(request.validate(), Err(ValidationCode::Checksum));
        request.checksum_sha256 = Some("ab".repeat(32));
        assert_eq!(request.validate(), Ok(()));
        request.transfer_mode = "multipart".into();
        assert_eq!(request.validate(), Ok(()));
        request.transfer_mode = "brokered".into();
        assert_eq!(request.validate(), Err(ValidationCode::Checksum));
    }

    #[test]
    fn direct_finalize_contract_is_tenant_and_checksum_bound() {
        let mut request = DirectUploadFinalizeRequest {
            schema_version: API_SCHEMA_VERSION,
            tenant_id: TENANT.into(),
            checksum_sha256: "ab".repeat(32),
        };
        assert_eq!(request.validate(), Ok(()));
        request.checksum_sha256 = "AB".repeat(32);
        assert_eq!(request.validate(), Err(ValidationCode::Checksum));
    }

    #[test]
    fn media_contract_does_not_allow_clients_to_choose_an_executor() {
        let mut request = MediaJobRequest {
            schema_version: 1,
            tenant_id: TENANT.into(),
            video_id: VIDEO.into(),
            source_version: 1,
            profile: "thumbnail_v1".into(),
            transform: Some(MediaTransformRequest {
                schema_version: 1,
                profile_version: 1,
                mode: ManagedMediaMode::Frame,
                start_ms: 0,
                duration_ms: None,
                width: Some(640),
                height: Some(360),
                fit: MediaResizeFit::Contain,
                image_count: None,
                include_audio: false,
                format: ManagedMediaFormat::Jpeg,
                max_output_bytes: 8_000_000,
            }),
        };
        assert_eq!(request.validate(), Ok(()));
        request.profile = "cloudflare_media".into();
        assert_eq!(request.validate(), Err(ValidationCode::Profile));
    }

    #[test]
    fn managed_transform_shapes_are_exact_and_executor_free() {
        let mut transform = MediaTransformRequest {
            schema_version: 1,
            profile_version: 1,
            mode: ManagedMediaMode::Video,
            start_ms: 0,
            duration_ms: Some(5_000),
            width: Some(640),
            height: Some(360),
            fit: MediaResizeFit::Cover,
            image_count: None,
            include_audio: true,
            format: ManagedMediaFormat::Mp4H264Aac,
            max_output_bytes: 16_000_000,
        };
        assert_eq!(transform.validate_for("optimized_clip_v1"), Ok(()));
        transform.image_count = Some(1);
        assert_eq!(
            transform.validate_for("optimized_clip_v1"),
            Err(ValidationCode::Profile)
        );
        let json = serde_json::to_string(&transform).expect("serialize");
        assert!(!json.contains("executor"));
        assert!(!json.contains("source"));
        assert!(!json.contains("object_key"));
    }

    #[test]
    fn video_lifecycle_contracts_are_bounded_and_revision_fenced() {
        let mut create = CreateVideoRequest {
            schema_version: 1,
            tenant_id: TENANT.into(),
            title: "Synthetic recording".into(),
        };
        assert_eq!(create.validate(), Ok(()));
        create.title = " padded ".into();
        assert_eq!(create.validate(), Err(ValidationCode::Title));

        let mut privacy = UpdatePrivacyRequest {
            schema_version: 1,
            tenant_id: TENANT.into(),
            privacy: "public".into(),
            expected_revision: 7,
        };
        assert_eq!(privacy.validate(), Ok(()));
        privacy.privacy = "unlisted".into();
        assert_eq!(privacy.validate(), Err(ValidationCode::Privacy));
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
