use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::contracts::{API_SCHEMA_VERSION, MediaJobRequest, valid_sha256};
use crate::r2_direct_upload::DirectPutCapabilityV1;

pub const MAX_SINGLE_UPLOAD_BYTES: u64 = 100 * 1_024 * 1_024;
pub const COMMAND_TTL_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UploadIntentResponse {
    pub schema_version: u16,
    pub upload_id: String,
    pub state: String,
    pub status_path: String,
    pub transfer_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalize_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_put: Option<DirectPutCapabilityV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multipart_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_count: Option<u16>,
    pub expected_bytes: u64,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum_header: Option<&'static str>,
}

impl UploadIntentResponse {
    pub fn new(upload_id: String, expected_bytes: u64, content_type: String) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            status_path: format!("/api/v1/uploads/{upload_id}"),
            upload_path: Some(format!("/api/v1/uploads/{upload_id}/content")),
            finalize_path: None,
            direct_put: None,
            multipart_path: None,
            part_size: None,
            part_count: None,
            transfer_mode: "brokered".into(),
            upload_id,
            state: "initiated".into(),
            expected_bytes,
            content_type,
            checksum_header: Some("x-content-sha256"),
        }
    }

    pub fn direct(
        upload_id: String,
        expected_bytes: u64,
        content_type: String,
        direct_put: DirectPutCapabilityV1,
    ) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            status_path: format!("/api/v1/uploads/{upload_id}"),
            upload_path: None,
            finalize_path: Some(format!("/api/v1/uploads/{upload_id}/finalize")),
            direct_put: Some(direct_put),
            multipart_path: None,
            part_size: None,
            part_count: None,
            transfer_mode: "direct".into(),
            upload_id,
            state: "initiated".into(),
            expected_bytes,
            content_type,
            checksum_header: None,
        }
    }

    pub fn multipart(
        upload_id: String,
        expected_bytes: u64,
        content_type: String,
        part_size: u64,
        part_count: u16,
    ) -> Self {
        Self {
            schema_version: API_SCHEMA_VERSION,
            status_path: format!("/api/v1/uploads/{upload_id}"),
            upload_path: None,
            finalize_path: None,
            direct_put: None,
            multipart_path: Some(format!("/api/v1/uploads/{upload_id}/multipart")),
            part_size: Some(part_size),
            part_count: Some(part_count),
            transfer_mode: "multipart".into(),
            upload_id,
            state: "initiated".into(),
            expected_bytes,
            content_type,
            checksum_header: Some("x-content-sha256"),
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
pub struct WorkerSourceDescriptor {
    pub ordinal: u16,
    pub path: String,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerOutputDescriptor {
    pub ordinal: u16,
    pub role: String,
    pub path: String,
    pub content_type: String,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeJobClaimResponse {
    pub schema_version: u16,
    pub native_plan_schema_version: u16,
    pub media_job_catalog_version: u16,
    pub media_service_catalog_version: u16,
    pub job_id: String,
    pub state: String,
    pub profile: String,
    pub execution_origin: String,
    pub attempt: u32,
    pub revision: u64,
    pub lease_expires_at_ms: u64,
    pub sources: Vec<WorkerSourceDescriptor>,
    pub outputs: Vec<WorkerOutputDescriptor>,
    pub sandbox: NativeSandboxEnvelopeV1,
    pub heartbeat_path: String,
    pub progress_path: String,
    pub complete_path: String,
    pub fail_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeSandboxEnvelopeV1 {
    pub max_source_bytes: u64,
    pub max_duration_ms: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_decoded_bytes: u64,
    pub max_frames: u64,
    pub max_tracks: u16,
    pub max_memory_bytes: u64,
    pub max_scratch_bytes: u64,
    pub max_cpu_millis: u64,
    pub max_gpu_millis: u64,
    pub max_output_bytes: u64,
    pub max_cost_microunits: u64,
    pub network: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerJobResponse {
    pub schema_version: u16,
    pub job_id: String,
    pub state: String,
    pub attempt: u32,
    pub revision: u64,
    pub progress_basis_points: Option<u16>,
    pub cancel_requested: bool,
    pub lease_expires_at_ms: Option<u64>,
    pub retry_scheduled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerOutputResponse {
    pub schema_version: u16,
    pub job_id: String,
    pub accepted: bool,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
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
pub struct VideoScopeRow {
    pub id: String,
}

#[derive(Deserialize)]
pub struct SourceObjectRow {
    pub object_key: String,
    pub bytes: i64,
    pub checksum_sha256: Option<String>,
    pub content_type: String,
}

#[derive(Clone, Deserialize)]
pub struct WorkerSourceRow {
    pub ordinal: i64,
    pub video_id: String,
    pub source_version: i64,
    pub object_key: String,
    pub bytes: i64,
    pub checksum_sha256: String,
    pub content_type: String,
}

/// Ordered, immutable input identity used only for deterministic artifact
/// naming. Every occurrence is retained, including repeated composition
/// sources, because the ordinal is part of the semantic input identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeArtifactSourceIdentityV2 {
    pub ordinal: u16,
    pub video_id: String,
    pub source_version: u32,
    pub object_key: String,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub content_type: String,
    pub authority_digest: String,
}

#[derive(Clone, Deserialize)]
pub struct NativeJobCandidateRow {
    pub id: String,
    pub revision: i64,
    pub attempt: i64,
    pub profile: String,
    pub payload_json: String,
    pub source_bytes: i64,
    pub source_checksum_sha256: String,
    pub source_content_type: String,
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

    pub fn supports_multipart(&self) -> bool {
        serde_json::from_str::<serde_json::Value>(&self.capabilities_json)
            .ok()
            .and_then(|value| value.get("multipart").and_then(|flag| flag.as_bool()))
            == Some(true)
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

/// Returns the canonical media-create digest and the only serializer-version
/// alias that is allowed to replay it. An omitted source list and an explicit
/// singleton equal to the top-level source describe the same command. New
/// receipts keep the pre-0027 omitted form as their canonical identity, while
/// the explicit digest remains accepted for receipts written during a mixed
/// serializer rollout.
pub fn media_job_create_digests(request: &MediaJobRequest) -> Result<(String, Option<String>), ()> {
    let singleton_is_primary = request.source_inputs.is_empty()
        || matches!(
            request.source_inputs.as_slice(),
            [source]
                if source.video_id == request.video_id
                    && source.source_version == request.source_version
        );
    if !singleton_is_primary {
        return Ok((request_digest("media_job_create", request)?, None));
    }

    let mut omitted = request.clone();
    omitted.source_inputs.clear();
    let canonical = request_digest("media_job_create", &omitted)?;

    let mut explicit = omitted;
    explicit.source_inputs = explicit.normalized_source_inputs();
    let explicit = request_digest("media_job_create", &explicit)?;
    let alias = (explicit != canonical).then_some(explicit);
    Ok((canonical, alias))
}

pub fn native_artifact_digest(
    request: &MediaJobRequest,
    sources: &[NativeArtifactSourceIdentityV2],
) -> Result<String, ()> {
    let inputs = request.normalized_source_inputs();
    if sources.len() != inputs.len()
        || sources
            .iter()
            .zip(inputs)
            .enumerate()
            .any(|(ordinal, (source, input))| {
                usize::from(source.ordinal) != ordinal
                    || source.video_id != input.video_id
                    || source.source_version != input.source_version
                    || source.object_key.is_empty()
                    || source.bytes == 0
                    || !valid_sha256(&source.checksum_sha256)
                    || source.content_type.is_empty()
                    || !valid_sha256(&source.authority_digest)
            })
    {
        return Err(());
    }

    #[derive(Serialize)]
    struct ArtifactIdentityV2<'a> {
        schema_version: u16,
        request: &'a MediaJobRequest,
        sources: &'a [NativeArtifactSourceIdentityV2],
    }

    request_digest(
        "native_media_artifact_v2",
        &ArtifactIdentityV2 {
            schema_version: 2,
            request,
            sources,
        },
    )
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

pub fn profile_kind(profile: &str) -> Option<&'static str> {
    match profile {
        "thumbnail_v1" => Some("frame"),
        "optimized_clip_v1" | "preview_v1" => Some("optimized_clip"),
        "spritesheet_v1" => Some("spritesheet"),
        "audio_extract_v1" | "audio_v1" => Some("audio_extract"),
        "probe_v1" => Some("probe"),
        "audio_presence_v1" => Some("audio_presence"),
        "distribution_master_v1" => Some("distribution_master"),
        "animated_preview_v1" => Some("animated_preview"),
        "audio_normalize_v1" => Some("audio_normalize"),
        "remux_repair_v1" => Some("remux_repair"),
        "segment_mux_v1" => Some("segment_mux"),
        "waveform_v1" => Some("waveform"),
        "composition_v1" => Some("composition"),
        "normalize_v1" => Some("normalize"),
        "transcription_v1" => Some("transcription"),
        "ai_cleanup_v1" => Some("ai_cleanup"),
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
    fn singleton_media_request_replays_across_omitted_and_explicit_serializers() {
        #[derive(Serialize)]
        struct LegacyMediaJobRequest<'a> {
            schema_version: u16,
            tenant_id: &'a str,
            video_id: &'a str,
            source_version: u32,
            profile: &'a str,
        }

        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a86900101";
        let video = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
        let mut request = MediaJobRequest {
            schema_version: API_SCHEMA_VERSION,
            tenant_id: tenant.into(),
            video_id: video.into(),
            source_version: 3,
            source_inputs: Vec::new(),
            profile: "probe_v1".into(),
            transform: None,
            composition: None,
        };
        let legacy = LegacyMediaJobRequest {
            schema_version: API_SCHEMA_VERSION,
            tenant_id: tenant,
            video_id: video,
            source_version: 3,
            profile: "probe_v1",
        };
        let legacy_digest = request_digest("media_job_create", &legacy).expect("legacy digest");
        let (omitted, omitted_alias) =
            media_job_create_digests(&request).expect("omitted digest set");
        assert_eq!(omitted, legacy_digest);

        request.source_inputs = request.normalized_source_inputs();
        let explicit_wire_digest =
            request_digest("media_job_create", &request).expect("explicit wire digest");
        let (explicit, explicit_alias) =
            media_job_create_digests(&request).expect("explicit digest set");
        assert_eq!(explicit, legacy_digest);
        assert_ne!(explicit_wire_digest, legacy_digest);
        assert_eq!(
            omitted_alias.as_deref(),
            Some(explicit_wire_digest.as_str())
        );
        assert_eq!(
            explicit_alias.as_deref(),
            Some(explicit_wire_digest.as_str())
        );

        request
            .source_inputs
            .push(crate::contracts::MediaJobSourceInputV1 {
                video_id: "018f47a6-7b1c-7f55-8f39-8f8a86900103".into(),
                source_version: 4,
            });
        let (multi_source, alias) =
            media_job_create_digests(&request).expect("multi-source digest set");
        assert_eq!(
            multi_source,
            request_digest("media_job_create", &request).expect("multi-source wire digest")
        );
        assert!(alias.is_none());
    }

    #[test]
    fn artifact_digest_binds_every_ordered_source_authority_field() {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a86900101";
        let first_video = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
        let second_video = "018f47a6-7b1c-7f55-8f39-8f8a86900103";
        let request = MediaJobRequest {
            schema_version: API_SCHEMA_VERSION,
            tenant_id: tenant.into(),
            video_id: first_video.into(),
            source_version: 1,
            source_inputs: vec![
                crate::contracts::MediaJobSourceInputV1 {
                    video_id: first_video.into(),
                    source_version: 1,
                },
                crate::contracts::MediaJobSourceInputV1 {
                    video_id: second_video.into(),
                    source_version: 2,
                },
            ],
            profile: "segment_mux_v1".into(),
            transform: None,
            composition: None,
        };
        let identity =
            |ordinal: u16, video_id: &str, version: u32, byte: u8| NativeArtifactSourceIdentityV2 {
                ordinal,
                video_id: video_id.into(),
                source_version: version,
                object_key: format!("tenants/{tenant}/videos/{video_id}/source/v{version}/payload"),
                bytes: 1_024,
                checksum_sha256: format!("{byte:02x}").repeat(32),
                content_type: "video/webm".into(),
                authority_digest: format!("{:02x}", byte.saturating_add(1)).repeat(32),
            };
        let sources = vec![
            identity(0, first_video, 1, 0xab_u8),
            identity(1, second_video, 2, 0xcd_u8),
        ];
        let digest = native_artifact_digest(&request, &sources).expect("artifact digest");
        assert_eq!(
            digest,
            native_artifact_digest(&request, &sources).expect("stable artifact digest")
        );

        for mutate in 0..5 {
            let mut changed = sources.clone();
            match mutate {
                0 => changed[1].object_key.push_str("-other"),
                1 => changed[1].bytes += 1,
                2 => changed[1].checksum_sha256 = "ef".repeat(32),
                3 => changed[1].content_type = "video/mp4".into(),
                _ => changed[1].authority_digest = "12".repeat(32),
            }
            assert_ne!(
                digest,
                native_artifact_digest(&request, &changed).expect("changed artifact digest")
            );
        }

        let mut sparse = sources;
        sparse[1].ordinal = 2;
        assert!(native_artifact_digest(&request, &sparse).is_err());
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

        let direct = UploadIntentResponse::direct(
            "018f47a6-7b1c-7f55-8f39-8f8a86900112".into(),
            10,
            "video/webm".into(),
            DirectPutCapabilityV1 {
                url: "https://example.invalid/private?X-Amz-Signature=secret".into(),
                method: "PUT",
                required_headers: vec![("content-length".into(), "10".into())],
                expires_at_ms: 100,
            },
        );
        let debug = format!("{direct:?}");
        assert!(!debug.contains("X-Amz-Signature"));
        let json = serde_json::to_string(&direct).expect("direct json");
        assert!(json.contains("/finalize"));
        assert!(!json.contains("tenants/"));

        let claim = NativeJobClaimResponse {
            schema_version: API_SCHEMA_VERSION,
            native_plan_schema_version: 1,
            media_job_catalog_version: 2,
            media_service_catalog_version: 1,
            job_id: "018f47a6-7b1c-7f55-8f39-8f8a86900112".into(),
            state: "leased".into(),
            profile: "thumbnail_v1".into(),
            execution_origin: "managed_fallback".into(),
            attempt: 1,
            revision: 1,
            lease_expires_at_ms: 100,
            sources: vec![WorkerSourceDescriptor {
                ordinal: 0,
                path: "/api/v1/worker/media-jobs/018f47a6-7b1c-7f55-8f39-8f8a86900112/source"
                    .into(),
                bytes: 10,
                checksum_sha256: "a".repeat(64),
                content_type: "video/webm".into(),
            }],
            outputs: vec![WorkerOutputDescriptor {
                ordinal: 0,
                role: "thumbnail".into(),
                path: "/api/v1/worker/media-jobs/018f47a6-7b1c-7f55-8f39-8f8a86900112/output"
                    .into(),
                content_type: "image/png".into(),
                max_bytes: 1_024,
            }],
            sandbox: NativeSandboxEnvelopeV1 {
                max_source_bytes: 10,
                max_duration_ms: 1_000,
                max_width: 640,
                max_height: 360,
                max_decoded_bytes: 1_000,
                max_frames: 30,
                max_tracks: 2,
                max_memory_bytes: 1_000,
                max_scratch_bytes: 1_000,
                max_cpu_millis: 1_000,
                max_gpu_millis: 1_000,
                max_output_bytes: 1_024,
                max_cost_microunits: 1_000,
                network: "denied".into(),
            },
            heartbeat_path:
                "/api/v1/worker/media-jobs/018f47a6-7b1c-7f55-8f39-8f8a86900112/heartbeat".into(),
            progress_path:
                "/api/v1/worker/media-jobs/018f47a6-7b1c-7f55-8f39-8f8a86900112/progress".into(),
            complete_path:
                "/api/v1/worker/media-jobs/018f47a6-7b1c-7f55-8f39-8f8a86900112/complete".into(),
            fail_path: "/api/v1/worker/media-jobs/018f47a6-7b1c-7f55-8f39-8f8a86900112/fail".into(),
        };
        let json = serde_json::to_string(&claim).expect("claim json");
        assert!(!json.contains("object_key"));
        assert!(!json.contains("tenants/"));
        assert!(!json.contains("lease_token"));
        assert!(!json.contains("worker_id"));
    }

    #[test]
    fn source_object_keys_are_deterministic_and_tenant_scoped() {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
        let video = "018f47a6-7b1c-7f55-8f39-8f8a86900104";
        let source = source_object_key(tenant, video, "source", 2);
        assert!(source.starts_with(&format!("tenants/{tenant}/videos/{video}/")));
        assert_eq!(source, source_object_key(tenant, video, "source", 2));
    }

    #[test]
    fn storage_capability_requires_an_explicit_boolean() {
        let supported = IntegrationRow {
            id: "integration".into(),
            capabilities_json: r#"{"conditional_put":true}"#.into(),
        };
        assert!(supported.supports_single_put());
        assert!(!supported.supports_multipart());
        let multipart = IntegrationRow {
            id: "integration".into(),
            capabilities_json: r#"{"conditional_put":true,"multipart":true}"#.into(),
        };
        assert!(multipart.supports_single_put());
        assert!(multipart.supports_multipart());
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
