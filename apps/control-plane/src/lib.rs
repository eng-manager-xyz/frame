pub mod api_workflow_runtime;
pub mod auth_repository;
mod auth_repository_conformance;
mod authenticated_web_runtime;
mod browser_web_runtime;
pub mod business_repository;
pub mod business_repository_conformance;
mod cloudflare_media;
mod commands;
mod compatibility_rate_limit;
mod contracts;
mod control_plane_ui;
pub mod cutover_authority;
pub mod cutover_authority_runtime;
mod instant_finalize_contract;
mod instant_finalize_runtime;
pub mod legacy_analytics_runtime;
mod legacy_analytics_web_runtime;
mod legacy_collaboration_runtime;
mod legacy_collaboration_web_runtime;
pub mod legacy_compatibility_runtime;
mod legacy_core_storage_runtime;
mod legacy_core_storage_web_runtime;
mod legacy_desktop_compatibility_runtime;
mod legacy_desktop_compatibility_web_runtime;
mod legacy_desktop_session_runtime;
mod legacy_desktop_session_web_runtime;
mod legacy_developer_actions_runtime;
mod legacy_developer_api_runtime;
mod legacy_developer_api_web_runtime;
mod legacy_developer_web_runtime;
mod legacy_extension_auth_runtime;
mod legacy_extension_auth_web_runtime;
mod legacy_extension_instant_recordings_runtime;
mod legacy_extension_instant_recordings_web_runtime;
mod legacy_folder_assignment_runtime;
mod legacy_folder_crud_runtime;
mod legacy_folder_crud_web_runtime;
mod legacy_folder_web_runtime;
mod legacy_invite_lifecycle_runtime;
mod legacy_invite_lifecycle_web_runtime;
mod legacy_library_detail_read_runtime;
mod legacy_library_detail_read_web_runtime;
mod legacy_library_id_read_runtime;
mod legacy_library_id_read_web_runtime;
mod legacy_library_placement_runtime;
mod legacy_library_web_runtime;
mod legacy_membership_actions_runtime;
mod legacy_membership_web_runtime;
mod legacy_mobile_bootstrap_caps_runtime;
mod legacy_mobile_bootstrap_caps_web_runtime;
mod legacy_mobile_session_runtime;
mod legacy_mobile_session_web_runtime;
mod legacy_mobile_uploads_runtime;
mod legacy_mobile_uploads_web_runtime;
mod legacy_notification_actions_runtime;
mod legacy_notification_preferences_runtime;
mod legacy_notification_read_runtime;
mod legacy_notification_web_runtime;
mod legacy_org_custom_domain_runtime;
mod legacy_org_custom_domain_web_runtime;
mod legacy_organization_library_runtime;
mod legacy_organization_library_web_runtime;
pub mod legacy_protected_billing_auth_runtime;
pub mod legacy_protected_billing_auth_web_runtime;
pub mod legacy_protected_integrations_runtime;
pub mod legacy_protected_integrations_web_runtime;
pub mod legacy_protected_media_runtime;
pub mod legacy_protected_media_web_runtime;
mod legacy_space_authorization_runtime;
mod legacy_space_authorization_web_runtime;
mod legacy_transcripts_runtime;
mod legacy_transcripts_web_runtime;
mod legacy_upload_storage_runtime;
mod legacy_upload_storage_web_runtime;
mod legacy_user_account_runtime;
mod legacy_user_account_web_runtime;
mod legacy_video_domain_info_runtime;
mod legacy_video_domain_info_web_runtime;
mod legacy_video_lifecycle_runtime;
mod legacy_video_lifecycle_web_runtime;
mod legacy_video_properties_runtime;
mod legacy_video_properties_web_runtime;
mod legacy_web_action_runtime;
mod media_service_runtime;
pub mod organization_repository;
mod organization_repository_conformance;
pub mod public_collaboration_runtime;
pub mod r2_direct_upload;
pub mod r2_multipart;
pub mod r2_storage;
pub mod repository;
mod repository_conformance;
mod routing;
pub mod storage_governance_runtime;
mod worker_auth_runtime;

use std::collections::{HashMap, HashSet};

use commands::{
    ApiKeyRow, COMMAND_TTL_MS, IntegrationRow, MAX_SINGLE_UPLOAD_BYTES, MediaJobResponse,
    MembershipRow, NativeArtifactSourceIdentityV2, NativeJobCandidateRow, NativeJobClaimResponse,
    NativeSandboxEnvelopeV1, SourceObjectRow, StoredCommandRow, UploadIntentResponse,
    UploadStatusResponse, VideoResponse, VideoScopeRow, WorkerOutputDescriptor,
    WorkerOutputResponse, WorkerSourceDescriptor, WorkerSourceRow, digest_credential,
    digest_identifier, media_job_create_digests, native_artifact_digest, parse_sha256,
    profile_kind, request_digest, source_object_key,
};
use compatibility_rate_limit::CompatibilityRateLimitBucketV1;
use contracts::{
    API_SCHEMA_VERSION, AuthorityResponse, CapabilitiesResponse, CreateVideoRequest,
    DirectUploadFinalizeRequest, DiscoveryResponse, MAX_COMMAND_BODY_BYTES, MAX_SAFE_INTEGER,
    MediaJobRequest, UpdatePrivacyRequest, UploadIntentRequest, WorkerClaimRequest,
    WorkerCompleteRequest, WorkerFailRequest, WorkerHeartbeatRequest, WorkerProgressRequest,
    normalize_cf_ray, origin_allowed, sanitized_public_title, valid_content_type,
    valid_idempotency_key, valid_lease_token, valid_uuid,
};
use cutover_authority::{
    ApprovedCutoverTransition, ApprovedReplayControl, CutoverAuthorityFailure,
    CutoverAuthoritySnapshot, CutoverShadowObservation, CutoverSignalKind, ReplayControlAction,
    ShadowClassification,
};
use frame_application::{LegacyCallerV1, RequestSecurityContextV1, StorageGovernanceServiceError};
use frame_client::{
    ApiError, ApiVersion, Capabilities, CaptionTrack, Health, INSTANT_UI_PROGRESS_SCHEMA_VERSION,
    InstantUiErrorCodeV1, InstantUiPhaseV1, InstantUiProgressV1, PlaybackDescriptor,
    PublicShareSummary, RetryAdvice, ServiceStatus, ShareAvailability,
};
use frame_domain::{
    ApiErrorCodeV1, AuthorityFence, ByteSize, ChecksumSha256, ClientCompatibilityPolicyV1,
    ClientReleaseV1, ClientSurfaceV1, ContentType, CorrelationId, CustomDomainName, CutoverDomain,
    CutoverEvidence, CutoverPhase, CutoverScope, DataAuthority, DurationMillis, GovernedObject,
    GovernedObjectId, GovernedObjectRole, GovernedObjectState, MAX_SIGNED_GRANT_LIFETIME_MS,
    MalwareDisposition, MultipartLimitsV1, MultipartPartNumberV1, MultipartUploadId,
    MultipartUploadSpecV1, ObjectVisibility, PublicAnalyticsConsentCommandV1,
    PublicAnalyticsEventCommandV1, PublicCommentCommandV1, PublicTranscriptV1, ScopedObjectKey,
    SignedGrantId, StorageAccessRequest, StorageAccessSurface, StorageActor, StorageHttpMethod,
    StorageMemberRole, StorageOperation, StorageQuotaPolicy, StorageResponsePolicy, TenantId,
    TimestampMillis, UserId, VerifiedCustomDomain, VerifiedRangeResponse,
};
use frame_ports::{
    MultipartObjectStoreV1, ProviderCompleteMultipartRequestV1, ProviderCreateMultipartRequestV1,
    ProviderPutPartRequestV1, StorageFailureKind, StorageGovernanceContextV1,
    StorageGovernanceRepositoryV1, StorageRequestContext,
};
use futures::StreamExt;
use instant_finalize_contract::{InstantFinalizeRequestV1, InstantFinalizeStateV1};
use r2_direct_upload::{
    MAX_DIRECT_UPLOAD_BYTES, R2DirectPutSigner, R2SigningCredentials, private_staging_key,
};
use r2_multipart::{
    AuthenticatedAbortOutcomeV1, D1TrustedMediaProbeV1, R2MultipartObjectStoreV1,
    abort_attempt_lock_until, abort_failure_class, abort_retry_at,
};
use repository::{AggregateRepository, MediaJobRow, UploadRow, VideoMutationRow, WorkerJobRow};
use routing::{
    Deployment, HostPolicy, Route, classify_raw_path, parse_raw_request_target,
    valid_repository_conformance_target, validate_host,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::*;

const PRODUCTION_HOST: &str = "frame.engmanager.xyz";
const NATIVE_LEASE_MS: i64 = 60_000;
const NATIVE_UPLOAD_SETTLE_MS: i64 = 2 * 60 * 60 * 1_000 + 2 * NATIVE_LEASE_MS;
const NATIVE_MAX_OUTPUT_BYTES: u64 = 32 * 1_024 * 1_024;
const NATIVE_MAX_ATTEMPTS: i64 = 3;
const STORAGE_RESERVATION_TTL_MS: i64 = 15 * 60 * 1_000;
const DIRECT_UPLOAD_TTL_SECONDS: u32 = 300;
const DIRECT_STAGING_CLEANUP_GRACE_MS: i64 = 60_000;
const MULTIPART_PART_BYTES: u64 = 16 * 1_024 * 1_024;
const MULTIPART_MAX_BYTES: u64 = 2_000_000_000;
const MULTIPART_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
const NATIVE_STANDARD_MAX_SOURCE_BYTES: u64 = 2_000_000_000;
const NATIVE_HEAVY_MAX_SOURCE_BYTES: u64 = 20_000_000_000;
const METADATA_CUTOVER_DOMAIN: &str = "metadata";

/// Shared, kind-checked ingress for legacy RPC/action protected-media callables.
///
/// Effect-RPC and browser-action decoders use this function after their own
/// authentication boundary. Workflows use the parent-receipt-only ingress
/// below, so this older actor-bearing boundary rejects them.
pub async fn dispatch_legacy_protected_media_callable_v1(
    operation_id: &str,
    database: &D1Database,
    actor_id: &str,
    tenant_id: Option<&str>,
    idempotency_key: &str,
    payload: serde_json::Value,
    now_ms: i64,
) -> std::result::Result<
    legacy_protected_media_runtime::LegacyProtectedMediaStageOutcomeV1,
    legacy_protected_media_runtime::LegacyProtectedMediaFailureV1,
> {
    let profile = frame_application::legacy_protected_media_profile(operation_id)
        .ok_or(legacy_protected_media_runtime::LegacyProtectedMediaFailureV1::Invalid)?;
    match profile.kind {
        frame_application::LegacyProtectedMediaKindV1::Rpc => {
            legacy_protected_media_web_runtime::rpc_response(
                operation_id,
                database,
                actor_id,
                tenant_id,
                idempotency_key,
                payload,
                now_ms,
            )
            .await
        }
        frame_application::LegacyProtectedMediaKindV1::ServerAction => {
            legacy_protected_media_web_runtime::server_action_response(
                operation_id,
                database,
                actor_id,
                tenant_id,
                idempotency_key,
                payload,
                now_ms,
            )
            .await
        }
        frame_application::LegacyProtectedMediaKindV1::Workflow => {
            Err(legacy_protected_media_runtime::LegacyProtectedMediaFailureV1::Invalid)
        }
        frame_application::LegacyProtectedMediaKindV1::Route => {
            Err(legacy_protected_media_runtime::LegacyProtectedMediaFailureV1::Invalid)
        }
    }
}

/// Exact internal ingress for the ten protected-media workflow schedulers.
///
/// The scheduler cannot supply an actor, credential, tenant, or replay key.
/// It names the immutable parent receipt/request pair and Frame reloads the
/// allowlisted edge plus exact authority from the neutral parent registry.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_legacy_protected_media_workflow_v1(
    operation_id: &str,
    database: &D1Database,
    parent_family: &str,
    parent_receipt_id: &str,
    parent_request_digest: &str,
    payload: serde_json::Value,
    now_ms: i64,
) -> std::result::Result<
    legacy_protected_media_runtime::LegacyProtectedMediaStageOutcomeV1,
    legacy_protected_media_runtime::LegacyProtectedMediaFailureV1,
> {
    legacy_protected_media_web_runtime::workflow_response(
        operation_id,
        database,
        parent_family,
        parent_receipt_id,
        parent_request_digest,
        payload,
        now_ms,
    )
    .await
}

/// Exact internal ingress for the two protected Loom workflow schedulers.
///
/// RPC, action, and route operation IDs are rejected even though they belong
/// to the same contract family. A durable provider outbox receipt is returned
/// only as an evidence requirement and is never projected as workflow success.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_legacy_protected_integration_workflow_v1(
    operation_id: &str,
    database: &D1Database,
    parent_family: &str,
    parent_receipt_id: &str,
    parent_request_digest: &str,
    payload: serde_json::Value,
    now_ms: i64,
) -> std::result::Result<
    legacy_protected_integrations_runtime::LegacyProtectedIntegrationStageOutcomeV1,
    legacy_protected_integrations_runtime::LegacyProtectedIntegrationFailureV1,
> {
    if !matches!(
        operation_id,
        "cap-v1-b9fcb0fbd25b2234" | "cap-v1-bd1b9d67380624f7"
    ) {
        return Err(
            legacy_protected_integrations_runtime::LegacyProtectedIntegrationFailureV1::Invalid,
        );
    }
    legacy_protected_integrations_web_runtime::workflow_response(
        operation_id,
        database,
        parent_family,
        parent_receipt_id,
        parent_request_digest,
        payload,
        now_ms,
    )
    .await
}

/// Exact internal ingress for Cap's administrator video-reprocessing
/// workflow. The scheduler supplies only the immutable parent action receipt
/// and its request digest; Frame reloads the actor and video target from that
/// receipt and derives the workflow replay key itself.
pub async fn dispatch_legacy_protected_billing_auth_workflow_v1(
    database: &D1Database,
    parent_receipt_id: &str,
    parent_request_digest: &str,
    now_ms: i64,
) -> std::result::Result<
    legacy_protected_billing_auth_runtime::LegacyProtectedBillingAuthStageOutcomeV1,
    legacy_protected_billing_auth_runtime::LegacyProtectedBillingAuthFailureV1,
> {
    legacy_protected_billing_auth_web_runtime::workflow_response(
        "cap-v1-5a990f470c701cec",
        database,
        parent_receipt_id,
        parent_request_digest,
        now_ms,
    )
    .await
}

#[cfg(test)]
fn is_legacy_protected_billing_auth_workflow(operation_id: &str) -> bool {
    operation_id == "cap-v1-5a990f470c701cec"
}

fn direct_upload_finalize_expired(now_ms: i64, expires_at_ms: i64) -> bool {
    now_ms >= expires_at_ms.saturating_add(DIRECT_STAGING_CLEANUP_GRACE_MS)
}

#[derive(Debug, Clone, Copy)]
struct NativeOutputContract {
    manifest_role: &'static str,
    governed_role: GovernedObjectRole,
    max_bytes: u64,
}

fn native_output_contract(profile: &str, content_type: &str) -> Option<NativeOutputContract> {
    let allowed = match profile {
        "optimized_clip_v1"
        | "distribution_master_v1"
        | "remux_repair_v1"
        | "segment_mux_v1"
        | "composition_v1"
        | "normalize_v1" => content_type == "video/mp4",
        "thumbnail_v1" => matches!(content_type, "image/jpeg" | "image/png"),
        "spritesheet_v1" => content_type == "image/jpeg",
        "audio_extract_v1" => content_type == "audio/mp4",
        "probe_v1" | "audio_presence_v1" | "waveform_v1" => content_type == "application/json",
        "animated_preview_v1" => matches!(content_type, "image/gif" | "video/mp4"),
        "audio_normalize_v1" => {
            matches!(content_type, "audio/mpeg" | "audio/mp4" | "audio/wav")
        }
        _ => false,
    };
    if !allowed {
        return None;
    }
    let (manifest_role, governed_role) = match profile {
        "optimized_clip_v1" | "animated_preview_v1" => ("preview", GovernedObjectRole::Preview),
        "thumbnail_v1" => ("thumbnail", GovernedObjectRole::Thumbnail),
        "spritesheet_v1" => ("spritesheet", GovernedObjectRole::Spritesheet),
        "audio_extract_v1" | "audio_normalize_v1" => ("audio", GovernedObjectRole::Audio),
        "probe_v1" | "audio_presence_v1" | "waveform_v1" => {
            ("manifest", GovernedObjectRole::Manifest)
        }
        "distribution_master_v1"
        | "remux_repair_v1"
        | "segment_mux_v1"
        | "composition_v1"
        | "normalize_v1" => ("export", GovernedObjectRole::Export),
        _ => return None,
    };
    let max_bytes = match profile {
        "probe_v1" | "audio_presence_v1" => 64 * 1_024,
        "waveform_v1" => 4 * 1_024 * 1_024,
        _ => NATIVE_MAX_OUTPUT_BYTES,
    };
    Some(NativeOutputContract {
        manifest_role,
        governed_role,
        // The v1 Worker proxy deliberately imposes a lower bound than the
        // native catalog until multipart direct-to-R2 output is implemented.
        max_bytes,
    })
}

fn native_claim_output(profile: &str, payload_json: &str) -> Option<(String, u64)> {
    let managed_request = if contracts::managed_profile(profile) {
        let request = serde_json::from_str::<MediaJobRequest>(payload_json).ok()?;
        if request.profile != profile || request.validate().is_err() {
            return None;
        }
        Some(request)
    } else {
        None
    };
    let content_type = if let Some(request) = managed_request.as_ref() {
        match request.transform.as_ref()?.format {
            contracts::ManagedMediaFormat::Mp4H264Aac => "video/mp4",
            contracts::ManagedMediaFormat::Jpeg => "image/jpeg",
            contracts::ManagedMediaFormat::Png => "image/png",
            contracts::ManagedMediaFormat::M4aAac => "audio/mp4",
        }
    } else {
        match profile {
            "probe_v1" | "audio_presence_v1" | "waveform_v1" => "application/json",
            "animated_preview_v1"
            | "distribution_master_v1"
            | "remux_repair_v1"
            | "composition_v1"
            | "normalize_v1" => "video/mp4",
            "audio_normalize_v1" => "audio/mp4",
            _ => return None,
        }
    };
    let contract = native_output_contract(profile, content_type)?;
    let requested_max_bytes = managed_request
        .as_ref()
        .and_then(|request| request.transform.as_ref())
        .map_or(contract.max_bytes, |transform| transform.max_output_bytes);
    Some((
        content_type.into(),
        requested_max_bytes.min(contract.max_bytes),
    ))
}

fn native_profile_max_attempts(profile: &str) -> i64 {
    if matches!(profile, "probe_v1" | "audio_presence_v1") {
        2
    } else {
        NATIVE_MAX_ATTEMPTS
    }
}

fn native_execution_failure_class(error_class: &str) -> Option<&'static str> {
    match error_class {
        "input_invalid" => Some("invalid_input"),
        "unsupported_media" => Some("unsupported_format"),
        "pipeline_timeout" => Some("timeout"),
        "pipeline_failure" | "output_invalid" => Some("output_incompatible"),
        "resource_limit" => Some("resource_limit"),
        "cancelled" => Some("cancelled"),
        "transport_failure" => Some("provider_outage"),
        _ => None,
    }
}

fn native_catalog_output_role(profile: &str) -> Option<&'static str> {
    match profile {
        "optimized_clip_v1" => Some("preview"),
        "thumbnail_v1" => Some("thumbnail"),
        "spritesheet_v1" => Some("spritesheet"),
        "audio_extract_v1" => Some("extracted_audio"),
        "probe_v1" | "audio_presence_v1" => Some("probe_manifest"),
        "distribution_master_v1" => Some("distribution_master"),
        "animated_preview_v1" => Some("animated_preview"),
        "audio_normalize_v1" => Some("normalized_audio"),
        "remux_repair_v1" => Some("repaired_media"),
        "segment_mux_v1" => Some("muxed_media"),
        "waveform_v1" => Some("waveform"),
        "composition_v1" => Some("composition"),
        "normalize_v1" => Some("normalized_media"),
        _ => None,
    }
}

fn native_execution_origin(profile: &str) -> Option<&'static str> {
    if contracts::managed_profile(profile) {
        Some("managed_fallback")
    } else if native_catalog_output_role(profile).is_some() {
        Some("native_only")
    } else {
        None
    }
}

fn native_sandbox(profile: &str) -> Option<NativeSandboxEnvelopeV1> {
    let heavy = matches!(
        profile,
        "distribution_master_v1"
            | "remux_repair_v1"
            | "segment_mux_v1"
            | "composition_v1"
            | "normalize_v1"
    );
    native_catalog_output_role(profile)?;
    Some(if heavy {
        NativeSandboxEnvelopeV1 {
            max_source_bytes: NATIVE_HEAVY_MAX_SOURCE_BYTES,
            max_duration_ms: 43_200_000,
            max_width: 7_680,
            max_height: 4_320,
            max_decoded_bytes: 512_000_000_000,
            max_frames: 4_000_000,
            max_tracks: 64,
            max_memory_bytes: 8_589_934_592,
            max_scratch_bytes: 40_000_000_000,
            max_cpu_millis: 7_200_000,
            max_gpu_millis: 7_200_000,
            max_output_bytes: 20_000_000_000,
            max_cost_microunits: 100_000_000,
            network: "denied".into(),
        }
    } else {
        NativeSandboxEnvelopeV1 {
            max_source_bytes: NATIVE_STANDARD_MAX_SOURCE_BYTES,
            max_duration_ms: 14_400_000,
            max_width: 7_680,
            max_height: 4_320,
            max_decoded_bytes: 64_000_000_000,
            max_frames: 1_300_000,
            max_tracks: 32,
            max_memory_bytes: 1_073_741_824,
            max_scratch_bytes: 4_000_000_000,
            max_cpu_millis: 900_000,
            max_gpu_millis: 900_000,
            max_output_bytes: 512_000_000,
            max_cost_microunits: 10_000_000,
            network: "denied".into(),
        }
    })
}
#[derive(Debug, Deserialize)]
struct ReadyRow {
    ready: i32,
}

#[derive(Debug, Deserialize)]
struct NativeExecutionManifestSeed {
    normalized_profile_sha256: String,
    source_checksum_sha256: String,
    attempt: i64,
}

#[derive(Debug, Deserialize)]
struct NativeOutputStagingRow {
    job_id: String,
    attempt: i64,
    organization_id: String,
    video_id: String,
    worker_id: String,
    lease_token_digest: String,
    staging_object_key: String,
    final_object_key: String,
    bytes: i64,
    checksum_sha256: String,
    content_type: String,
    state: String,
    provider_etag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeStagingRecoveryRow {
    job_id: String,
    attempt: i64,
    organization_id: String,
    video_id: String,
    staging_object_key: String,
    final_object_key: String,
    bytes: i64,
    checksum_sha256: String,
    content_type: String,
    state: String,
    updated_at_ms: i64,
    job_state: String,
    job_attempt: i64,
    cancel_requested: i64,
}

#[derive(Debug, Deserialize)]
struct NativeCancellationRecoveryRow {
    job_id: String,
    organization_id: String,
}

#[derive(Debug, Deserialize)]
struct DirectStagingExpiryRow {
    id: String,
    organization_id: String,
    content_type: String,
    state: String,
    direct_staging_key: String,
}

#[derive(Debug, Deserialize)]
struct MultipartProbeCandidateRow {
    organization_id: String,
    upload_id: String,
}

#[derive(Debug, Deserialize)]
struct MultipartAbortCandidateRow {
    organization_id: String,
    upload_id: String,
}

#[derive(Debug, Deserialize)]
struct MultipartAbortAttemptRow {
    intent_kind: String,
    state: String,
    attempt_count: i64,
    next_attempt_at_ms: i64,
    last_failure_class: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MultipartSessionStateRow {
    state: String,
}

#[derive(Debug, Deserialize)]
struct VerifiedMultipartObjectRow {
    provider_version: String,
    provider_etag: String,
    bytes: i64,
    checksum_sha256: String,
    content_type: String,
}

#[derive(Debug, Deserialize)]
struct PresenceFlagRow {
    present: i64,
}

#[derive(Debug, Serialize)]
struct HealthDependencies {
    d1: bool,
    r2: bool,
    media_transformations: bool,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    #[serde(flatten)]
    contract: Health,
    dependencies: HealthDependencies,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicShareRow {
    id: String,
    title: String,
    state: String,
    privacy: String,
    organization_id: Option<String>,
    playback_object_key: Option<String>,
    duration_ms: Option<i64>,
    content_type: Option<String>,
    bytes: Option<i64>,
    checksum_sha256: Option<String>,
    object_version: Option<i64>,
    governed_role: Option<String>,
    governed_visibility: Option<String>,
    governed_state: Option<String>,
    malware_disposition: Option<String>,
    cache_generation: Option<i64>,
    instant_finalize_state: Option<String>,
    instant_finalize_failure_class: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicObject {
    key: String,
    content_type: String,
    bytes: u64,
    checksum: ChecksumSha256,
    governed: GovernedObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateStorageGrantRequest {
    tenant_id: String,
    object_key: String,
    operation: String,
    lifetime_ms: i64,
}

#[derive(Serialize)]
struct CreateStorageGrantResponse {
    schema_version: u16,
    grant_id: String,
    token: String,
    expires_at_ms: i64,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutoverTransitionRequest {
    target: CutoverPhase,
    expected_epoch: u64,
    evidence: CutoverEvidenceRequest,
    reconciliation_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutoverEvidenceRequest {
    shadow_observation_ready: bool,
    reconciliation_clean: bool,
    rollback_rehearsed: bool,
    observation_window_complete: bool,
    reconciliation_digest_present: bool,
    legacy_fenced: bool,
    d1_fenced: bool,
    legacy_caught_up: bool,
    pending_events: u64,
    dead_letter_events: u64,
    shadow_mismatches: u64,
}

impl From<CutoverEvidenceRequest> for CutoverEvidence {
    fn from(value: CutoverEvidenceRequest) -> Self {
        Self {
            shadow_observation_ready: value.shadow_observation_ready,
            reconciliation_clean: value.reconciliation_clean,
            rollback_rehearsed: value.rollback_rehearsed,
            observation_window_complete: value.observation_window_complete,
            reconciliation_digest_present: value.reconciliation_digest_present,
            legacy_fenced: value.legacy_fenced,
            d1_fenced: value.d1_fenced,
            legacy_caught_up: value.legacy_caught_up,
            pending_events: value.pending_events,
            dead_letter_events: value.dead_letter_events,
            shadow_mismatches: value.shadow_mismatches,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutoverReplayControlRequest {
    expected_epoch: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CutoverSignalKindRequest {
    AuthorityContention,
    ReplayWriteFailure,
    ReplayLostAck,
}

impl From<CutoverSignalKindRequest> for CutoverSignalKind {
    fn from(value: CutoverSignalKindRequest) -> Self {
        match value {
            CutoverSignalKindRequest::AuthorityContention => Self::AuthorityContention,
            CutoverSignalKindRequest::ReplayWriteFailure => Self::ReplayWriteFailure,
            CutoverSignalKindRequest::ReplayLostAck => Self::ReplayLostAck,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutoverSignalRequest {
    expected_phase_epoch: u64,
    kind: CutoverSignalKindRequest,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShadowClassificationRequest {
    Match,
    OrderingOnly,
    SemanticMismatch,
    Missing,
    Error,
}

impl From<ShadowClassificationRequest> for ShadowClassification {
    fn from(value: ShadowClassificationRequest) -> Self {
        match value {
            ShadowClassificationRequest::Match => Self::Match,
            ShadowClassificationRequest::OrderingOnly => Self::OrderingOnly,
            ShadowClassificationRequest::SemanticMismatch => Self::SemanticMismatch,
            ShadowClassificationRequest::Missing => Self::Missing,
            ShadowClassificationRequest::Error => Self::Error,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutoverShadowObservationRequest {
    phase_epoch: u64,
    observation_digest: String,
    query_class: String,
    normalization_digest: String,
    legacy_result_digest: String,
    d1_result_digest: String,
    classification: ShadowClassificationRequest,
}

#[derive(Serialize)]
struct CutoverAuthorityResponse {
    schema_version: u16,
    authority: CutoverAuthoritySnapshot,
}

#[derive(Debug, Deserialize)]
struct GovernedContentTypeRow {
    content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestedRange {
    range: worker::Range,
    start: u64,
    length: u64,
}

#[derive(Debug, Deserialize)]
struct AuthorityRow {
    phase: String,
    authority: String,
    epoch: i64,
}

#[derive(Debug, Deserialize)]
struct PlaybackAuthorityRow {
    playback_object_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MutationAuthorityFence {
    /// Local deployments deliberately bypass the cutover table. Production
    /// epochs are always non-negative, so -1 is an unambiguous SQL sentinel.
    sql_epoch: i64,
    scoped: Option<AuthorityFence>,
}

impl MutationAuthorityFence {
    const LOCAL_SQL_EPOCH: i64 = -1;

    const fn local() -> Self {
        Self {
            sql_epoch: Self::LOCAL_SQL_EPOCH,
            scoped: None,
        }
    }

    const fn production(scoped: AuthorityFence) -> Self {
        Self {
            // Legacy inline predicates are bypassed only because the scoped
            // assertion is inserted into the same D1 batch.
            sql_epoch: Self::LOCAL_SQL_EPOCH,
            scoped: Some(scoped),
        }
    }
}

#[derive(Debug)]
struct AuthenticatedActor {
    user_id: String,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequiredAccess {
    Read,
    Write,
    Admin,
    Worker,
}

impl AuthenticatedActor {
    fn allows(&self, required: RequiredAccess) -> bool {
        self.scopes.iter().any(|scope| {
            if required == RequiredAccess::Worker {
                scope == "frame:worker"
            } else {
                scope == "frame:admin"
                    || (scope == "frame:write"
                        && matches!(required, RequiredAccess::Read | RequiredAccess::Write))
                    || (scope == "frame:read" && required == RequiredAccess::Read)
            }
        })
    }
}

enum CommandReplay {
    New,
    Stored { status: u16, json: String },
    Conflict,
}

struct FakePreview<'a> {
    tenant_id: &'a str,
    video_id: &'a str,
    job_id: &'a str,
    output_key: &'a str,
    source_version: u32,
    source: &'a SourceObjectRow,
}

#[derive(Debug)]
struct RuntimeConfig {
    host_policy: HostPolicy,
    media_mode: MediaMode,
    chrome_extension_id: Option<String>,
    cap_hosted: bool,
    videos_default_public: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaMode {
    Remote,
    Fake,
    Native,
}

fn requires_native_claim(mode: MediaMode, profile: &str) -> bool {
    match mode {
        MediaMode::Native => true,
        MediaMode::Remote => !contracts::managed_profile(profile),
        MediaMode::Fake => false,
    }
}

impl RuntimeConfig {
    fn from_env(env: &Env) -> Option<Self> {
        let deployment = env
            .var("FRAME_DEPLOYMENT")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "production".into());
        let deployment = match deployment.as_str() {
            "production" | "staging" => Deployment::Production,
            "local" | "development" | "test" => Deployment::Local,
            _ => return None,
        };
        let default_host = if deployment == Deployment::Local {
            "localhost"
        } else {
            PRODUCTION_HOST
        };
        let public_host = env
            .var("FRAME_PUBLIC_HOST")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| default_host.into());
        let media_mode = env
            .var("FRAME_MEDIA_MODE")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "remote".into());
        let media_mode = match (deployment, media_mode.as_str()) {
            (Deployment::Production, "remote") => MediaMode::Remote,
            (Deployment::Production, "native") => MediaMode::Native,
            (Deployment::Local, "fake") => MediaMode::Fake,
            (Deployment::Local, "remote") => MediaMode::Remote,
            (Deployment::Local, "native") => MediaMode::Native,
            _ => return None,
        };
        let chrome_extension_id = env
            .var("CAP_CHROME_EXTENSION_ID")
            .map(|value| value.to_string())
            .or_else(|_| {
                env.secret("CAP_CHROME_EXTENSION_ID")
                    .map(|value| value.to_string())
            })
            .ok()
            .filter(|value| !value.is_empty() && value.len() <= 255 && value.is_ascii());
        let cap_hosted = match env
            .var("NEXT_PUBLIC_IS_CAP")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "true".into())
            .as_str()
        {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => return None,
        };
        let videos_default_public = match env
            .var("CAP_VIDEOS_DEFAULT_PUBLIC")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "true".into())
            .as_str()
        {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => return None,
        };
        Some(Self {
            host_policy: HostPolicy::new(deployment, public_host)?,
            media_mode,
            chrome_extension_id,
            cap_hosted,
            videos_default_public,
        })
    }

    fn production(&self) -> bool {
        self.host_policy.deployment == Deployment::Production
    }
}

pub(crate) fn direct_upload_signer(env: &Env) -> Option<R2DirectPutSigner> {
    let account_id = env
        .var("FRAME_R2_ACCOUNT_ID")
        .map(|value| value.to_string())
        .or_else(|_| {
            env.secret("FRAME_R2_ACCOUNT_ID")
                .map(|value| value.to_string())
        })
        .ok()?;
    let bucket_name = env.var("FRAME_R2_BUCKET_NAME").ok()?.to_string();
    let access_key_id = env.secret("FRAME_R2_ACCESS_KEY_ID").ok()?.to_string();
    let secret_access_key = env.secret("FRAME_R2_SECRET_ACCESS_KEY").ok()?.to_string();
    let credentials = R2SigningCredentials::parse(access_key_id, secret_access_key).ok()?;
    R2DirectPutSigner::new(account_id, bucket_name, credentials).ok()
}

fn native_worker_enabled(config: &RuntimeConfig) -> bool {
    matches!(config.media_mode, MediaMode::Remote | MediaMode::Native)
}

#[derive(Debug, Clone, Copy)]
struct ApiFailure {
    status: u16,
    code: &'static str,
    message: &'static str,
    retryable: bool,
    allow: Option<&'static str>,
    authenticate: bool,
    retry_after_seconds: Option<u64>,
}

impl ApiFailure {
    const fn new(status: u16, code: &'static str, message: &'static str, retryable: bool) -> Self {
        Self {
            status,
            code,
            message,
            retryable,
            allow: None,
            authenticate: false,
            retry_after_seconds: None,
        }
    }

    const fn with_allow(mut self, allow: &'static str) -> Self {
        self.allow = Some(allow);
        self
    }

    const fn with_authenticate(mut self) -> Self {
        self.authenticate = true;
        self
    }

    const fn with_retry_after_seconds(mut self, seconds: u64) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }
}

#[event(fetch)]
pub async fn main(request: Request, env: Env, context: Context) -> Result<Response> {
    let request_id = request_id(&request);
    let Some(config) = RuntimeConfig::from_env(&env) else {
        return failure_response(
            ApiFailure::new(
                503,
                "service_unavailable",
                "The service is temporarily unavailable.",
                true,
            ),
            &request_id,
            true,
        );
    };

    match dispatch(request, &env, &config, &context, &request_id).await {
        Ok(response) => Ok(response),
        Err(_) => {
            console_error!("control-plane request failed request_id={request_id}");
            failure_response(
                ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
                &request_id,
                config.production(),
            )
        }
    }
}

const AUTH_DELIVERY_CRON: &str = "* * * * *";
const MULTIPART_MAINTENANCE_CRON: &str = "*/2 * * * *";
const INSTANT_FINALIZE_CRON: &str = "1-59/2 * * * *";
const MEDIA_RECOVERY_CRON: &str = "*/5 * * * *";
const RETENTION_MAINTENANCE_CRON: &str = "2-59/5 * * * *";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduledLane {
    AuthDelivery,
    MultipartMaintenance,
    InstantFinalize,
    MediaRecovery,
    RetentionMaintenance,
}

fn scheduled_lane(cron: &str) -> Option<ScheduledLane> {
    match cron {
        AUTH_DELIVERY_CRON => Some(ScheduledLane::AuthDelivery),
        MULTIPART_MAINTENANCE_CRON => Some(ScheduledLane::MultipartMaintenance),
        INSTANT_FINALIZE_CRON => Some(ScheduledLane::InstantFinalize),
        MEDIA_RECOVERY_CRON => Some(ScheduledLane::MediaRecovery),
        RETENTION_MAINTENANCE_CRON => Some(ScheduledLane::RetentionMaintenance),
        _ => None,
    }
}

#[event(scheduled)]
pub async fn scheduled(event: ScheduledEvent, env: Env, context: ScheduleContext) {
    let Some(config) = RuntimeConfig::from_env(&env) else {
        return;
    };
    match scheduled_lane(&event.cron()) {
        Some(ScheduledLane::AuthDelivery) => {
            context.wait_until(worker_auth_runtime::dispatch_delivery_batch(env));
        }
        Some(ScheduledLane::MultipartMaintenance) => {
            context.wait_until(cleanup_expired_multipart(env));
        }
        Some(ScheduledLane::InstantFinalize) => {
            context.wait_until(reconcile_instant_finalize_one(env));
        }
        Some(ScheduledLane::MediaRecovery) => {
            context.wait_until(recover_scheduled_media(env, config.media_mode));
        }
        Some(ScheduledLane::RetentionMaintenance) => {
            context.wait_until(run_retention_maintenance(env));
        }
        None => console_error!("scheduled invocation rejected class=unknown_cron"),
    }
}

async fn recover_scheduled_media(env: Env, mode: MediaMode) {
    // These two bounded one-candidate reconcilers share only the media lane.
    // They can never consume the auth, multipart, or instant-finalize D1
    // allowance because every configured Cron Trigger is a distinct Worker
    // invocation.
    if mode == MediaMode::Remote {
        media_service_runtime::recover_one(env.clone()).await;
    }
    if matches!(mode, MediaMode::Native | MediaMode::Remote) {
        recover_native_staging_one(env).await;
    }
}

async fn run_retention_maintenance(env: Env) {
    prune_public_collaboration(env.clone()).await;
    cleanup_expired_direct_staging(env).await;
}

async fn cleanup_expired_multipart(env: Env) {
    let result = async {
        let config = RuntimeConfig::from_env(&env)
            .ok_or_else(|| Error::RustError("multipart runtime configuration is invalid".into()))?;
        let database = env.d1("DB")?;
        let bucket = env.bucket("RECORDINGS")?;
        let probe = D1TrustedMediaProbeV1::new(&database);
        let store = R2MultipartObjectStoreV1::new(&bucket, &database, &probe)
            .map_err(|_| Error::RustError("multipart adapter configuration is invalid".into()))?;
        if let Err(error) = store.reconcile_completing_one().await
            && !error.retryable()
        {
            console_error!("multipart completion reconciliation failed class=conflict");
        }
        enqueue_pending_multipart_probe(&env, &database).await?;
        reconcile_authenticated_multipart_abort_one(&env, &config, &database, &bucket).await?;
        store
            .cleanup_stale(
                TimestampMillis::new(current_time_ms()?)
                    .map_err(|_| Error::RustError("multipart cleanup clock is invalid".into()))?,
                25,
            )
            .await
            .map_err(|_| Error::RustError("multipart cleanup failed".into()))?;
        Ok::<(), Error>(())
    }
    .await;
    if result.is_err() {
        console_error!("multipart stale-session cleanup failed class=persistence");
    }
}

async fn reconcile_authenticated_multipart_abort_one(
    env: &Env,
    config: &RuntimeConfig,
    database: &D1Database,
    bucket: &Bucket,
) -> Result<()> {
    let now = current_time_ms()?;
    let candidate = database
        .prepare(
            "SELECT upload.organization_id,reconciliation.upload_id \
             FROM r2_multipart_abort_reconciliation_v1 reconciliation \
             JOIN r2_multipart_sessions_v1 session USING(upload_id) \
             JOIN video_uploads upload ON upload.id=reconciliation.upload_id \
             WHERE reconciliation.intent_kind='authenticated_delete' \
               AND reconciliation.state='pending' AND reconciliation.next_attempt_at_ms<=?1 \
               AND session.state IN ('open','completing') \
               AND upload.state IN ('initiated','uploading','finalizing','failed') \
             ORDER BY reconciliation.next_attempt_at_ms,reconciliation.updated_at_ms,\
               reconciliation.upload_id LIMIT 1",
        )
        .bind(&[JsValue::from_f64(now as f64)])?
        .first::<MultipartAbortCandidateRow>(None)
        .await?;
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let Some(authority_fence) =
        mutation_authority_fence(env, config, &candidate.organization_id).await?
    else {
        return Ok(());
    };
    let Some(upload) =
        load_upload(database, &candidate.organization_id, &candidate.upload_id).await?
    else {
        return Ok(());
    };
    let Some(intent) = load_multipart_intent(database, &candidate.upload_id).await? else {
        return Ok(());
    };
    let probe = D1TrustedMediaProbeV1::new(database);
    let store = R2MultipartObjectStoreV1::new(bucket, database, &probe)
        .map_err(|_| Error::RustError("multipart adapter configuration is invalid".into()))?;
    let (context, upload_id, key, _) = multipart_values(&upload, &intent)?;
    let reference = store
        .route_reference(context, upload_id, &key)
        .await
        .map_err(|_| Error::RustError("multipart abort reference is unavailable".into()))?;
    let attempt = match claim_authenticated_multipart_abort(
        database,
        &authority_fence,
        &candidate.organization_id,
        &candidate.upload_id,
        now,
    )
    .await?
    {
        AuthenticatedAbortClaim::Attempt(attempt) => attempt,
        AuthenticatedAbortClaim::AlreadyAborted | AuthenticatedAbortClaim::AlreadyCompleted => {
            return Ok(());
        }
    };
    let outcome = match store
        .reconcile_authenticated_abort_provider(
            context,
            reference,
            attempt,
            TimestampMillis::new(now)
                .map_err(|_| Error::RustError("multipart abort clock is invalid".into()))?,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            retain_authenticated_multipart_abort_failure(
                database,
                &authority_fence,
                &candidate.upload_id,
                attempt,
                error.kind(),
                now,
            )
            .await?;
            return Ok(());
        }
    };
    match outcome {
        AuthenticatedAbortOutcomeV1::Confirmed {
            attempt: provider_attempt,
        }
        | AuthenticatedAbortOutcomeV1::PreservedObject {
            attempt: provider_attempt,
        } if provider_attempt == attempt => {
            finish_authenticated_multipart_abort(
                database,
                &authority_fence,
                &candidate.organization_id,
                &candidate.upload_id,
                attempt,
                outcome,
                now,
            )
            .await
        }
        AuthenticatedAbortOutcomeV1::AlreadyAborted
        | AuthenticatedAbortOutcomeV1::AlreadyCompleted
        | AuthenticatedAbortOutcomeV1::Pending
        | AuthenticatedAbortOutcomeV1::Confirmed { .. }
        | AuthenticatedAbortOutcomeV1::PreservedObject { .. } => Ok(()),
    }
}

async fn enqueue_pending_multipart_probe(env: &Env, database: &D1Database) -> Result<()> {
    let candidate = database
        .prepare(
            "SELECT u.organization_id,u.id AS upload_id \
             FROM r2_multipart_verified_objects_v1 verified \
             JOIN r2_multipart_sessions_v1 s ON s.upload_id=verified.upload_id \
             JOIN video_uploads u ON u.id=s.upload_id \
             LEFT JOIN r2_multipart_completions_v1 complete ON complete.upload_id=s.upload_id \
             WHERE s.state='completing' AND complete.upload_id IS NULL \
             ORDER BY verified.verified_at_ms,u.id LIMIT 1",
        )
        .first::<MultipartProbeCandidateRow>(None)
        .await?;
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let config = RuntimeConfig::from_env(env)
        .ok_or_else(|| Error::RustError("runtime configuration is invalid".into()))?;
    let Some(authority_fence) =
        mutation_authority_fence(env, &config, &candidate.organization_id).await?
    else {
        return Ok(());
    };
    let upload = load_upload(database, &candidate.organization_id, &candidate.upload_id)
        .await?
        .ok_or_else(|| Error::RustError("multipart upload reconciliation is unavailable".into()))?;
    let intent = load_multipart_intent(database, &candidate.upload_id)
        .await?
        .ok_or_else(|| Error::RustError("multipart intent reconciliation is unavailable".into()))?;
    ensure_multipart_probe_job(env, database, &authority_fence, &upload, &intent).await
}

async fn reconcile_instant_finalize_one(env: Env) {
    let result = async {
        let config = RuntimeConfig::from_env(&env)
            .ok_or_else(|| Error::RustError("runtime configuration is invalid".into()))?;
        let database = env.d1("DB")?;
        let now = current_time_ms()?;
        let candidates = instant_finalize_runtime::scan_candidates(&database, now, 8)
            .await
            .map_err(|_| Error::RustError("instant finalize scan failed".into()))?;
        for candidate in candidates {
            // Cursor movement is scheduler bookkeeping, not tenant content.
            // Advancing before authority lookup prevents one disabled tenant
            // from starving the rest of the bounded ring.
            instant_finalize_runtime::advance_cursor(&database, &candidate.session_id, now)
                .await
                .map_err(|_| Error::RustError("instant finalize cursor failed".into()))?;
            let Some(authority_fence) =
                mutation_authority_fence(&env, &config, &candidate.organization_id).await?
            else {
                console_error!(
                    "instant finalize reconciliation skipped class=authority_unavailable"
                );
                continue;
            };
            if let Err(failure) = instant_finalize_runtime::reconcile_session(
                &database,
                &authority_fence,
                &candidate.session_id,
                now,
            )
            .await
                && let Err(record_error) = instant_finalize_runtime::record_reconcile_failure(
                    &database,
                    &authority_fence,
                    &candidate.session_id,
                    failure,
                    now,
                )
                .await
            {
                console_error!(
                    "instant finalize retry retention failed class={}",
                    record_error.safe_code()
                );
            }
        }
        Ok::<(), Error>(())
    }
    .await;
    if result.is_err() {
        console_error!("instant finalize scheduler failed class=persistence");
    }
}

async fn cleanup_expired_direct_staging(env: Env) {
    if cleanup_expired_direct_staging_inner(&env).await.is_err() {
        console_error!("direct upload staging cleanup failed class=persistence");
    }
}

async fn cleanup_expired_direct_staging_inner(env: &Env) -> Result<()> {
    let config = RuntimeConfig::from_env(env)
        .ok_or_else(|| Error::RustError("runtime configuration is invalid".into()))?;
    let database = env.d1("DB")?;
    let now = current_time_ms()?;
    let cleanup_before = now.saturating_sub(DIRECT_STAGING_CLEANUP_GRACE_MS);
    let candidate = database
        .prepare(
            "SELECT u.id, u.organization_id, u.content_type, u.state, u.direct_staging_key \
             FROM video_uploads u LEFT JOIN direct_upload_staging_cleanup_v1 c ON c.upload_id = u.id \
             WHERE u.transfer_mode = 'direct' AND u.direct_expires_at_ms <= ?1 \
               AND u.direct_staging_key IS NOT NULL AND c.upload_id IS NULL \
             ORDER BY u.direct_expires_at_ms, u.id LIMIT 1",
        )
        .bind(&[JsValue::from_f64(cleanup_before as f64)])?
        .first::<DirectStagingExpiryRow>(None)
        .await?;
    let Some(candidate) = candidate else {
        return Ok(());
    };
    if !valid_uuid(&candidate.id)
        || !valid_uuid(&candidate.organization_id)
        || !matches!(
            candidate.state.as_str(),
            "initiated" | "uploading" | "finalizing" | "complete" | "failed" | "aborted"
        )
        || private_staging_key(
            &candidate.organization_id,
            &candidate.id,
            &candidate.content_type,
        )
        .ok()
        .as_deref()
            != Some(candidate.direct_staging_key.as_str())
    {
        return Err(Error::RustError(
            "direct staging cleanup candidate is corrupt".into(),
        ));
    }
    let Some(authority_fence) =
        mutation_authority_fence(env, &config, &candidate.organization_id).await?
    else {
        return Err(Error::RustError(
            "direct staging cleanup authority is unavailable".into(),
        ));
    };
    env.bucket("RECORDINGS")?
        .delete(&candidate.direct_staging_key)
        .await?;
    let event_fingerprint = digest_identifier(
        "direct_upload_event",
        &format!("{}:aborted:cleanup", candidate.id),
    )
    .map_err(|()| Error::RustError("direct cleanup event is invalid".into()))?;
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("direct-upload-cleanup:{}", candidate.id),
            now,
            vec![
                database
                    .prepare(
                        "UPDATE video_uploads SET state = 'aborted', updated_at_ms = ?3, revision = revision + 1, \
                           event_sequence = event_sequence + 1, event_fingerprint = ?4 \
                         WHERE id = ?1 AND organization_id = ?2 AND transfer_mode = 'direct' \
                           AND state IN ('initiated','uploading','finalizing')",
                    )
                    .bind(&[
                        JsValue::from_str(&candidate.id),
                        JsValue::from_str(&candidate.organization_id),
                        JsValue::from_f64(now as f64),
                        JsValue::from_str(&event_fingerprint),
                    ])?,
                database
                    .prepare(
                        "INSERT INTO direct_upload_staging_cleanup_v1(upload_id, cleaned_at_ms) \
                         VALUES (?1, ?2) ON CONFLICT(upload_id) DO NOTHING",
                    )
                    .bind(&[
                        JsValue::from_str(&candidate.id),
                        JsValue::from_f64(now as f64),
                    ])?,
            ],
        )
        .await?,
    )
}

async fn prune_public_collaboration(env: Env) {
    let result = async {
        let database = env.d1("DB")?;
        public_collaboration_runtime::prune_expired(&database, current_time_ms()?).await
    }
    .await;
    if result.is_err() {
        console_error!("public collaboration retention failed class=persistence");
    }
}

async fn recover_native_staging_one(env: Env) {
    if recover_native_staging_one_inner(&env).await.is_err() {
        console_error!("native media staging recovery failed class=persistence");
    }
}

async fn recover_native_staging_one_inner(env: &Env) -> Result<()> {
    let config = RuntimeConfig::from_env(env)
        .ok_or_else(|| Error::RustError("runtime configuration is invalid".into()))?;
    let database = env.d1("DB")?;
    let now = current_time_ms()?;
    let settle_before = now.saturating_sub(NATIVE_UPLOAD_SETTLE_MS);
    let candidate = database
        .prepare(
            "SELECT s.job_id, s.attempt, s.organization_id, s.video_id, \
                    s.staging_object_key, s.final_object_key, s.bytes, s.checksum_sha256, \
                    s.content_type, s.state, s.updated_at_ms, j.state AS job_state, \
                    j.attempt AS job_attempt, j.cancel_requested \
             FROM media_native_output_staging_v1 s JOIN media_jobs j ON j.id = s.job_id \
             WHERE s.state = 'published' OR (s.state IN ('receiving','staged') \
               AND s.updated_at_ms <= ?1 AND (j.cancel_requested = 1 \
                 OR j.attempt > s.attempt OR j.state = 'failed')) \
             ORDER BY CASE s.state WHEN 'published' THEN 0 ELSE 1 END, s.updated_at_ms, s.job_id \
             LIMIT 1",
        )
        .bind(&[JsValue::from_f64(settle_before as f64)])?
        .first::<NativeStagingRecoveryRow>(None)
        .await?;
    let Some(candidate) = candidate else {
        return finalize_native_cancel_without_staging(env, &config, &database, now).await;
    };
    let Some(authority_fence) =
        mutation_authority_fence(env, &config, &candidate.organization_id).await?
    else {
        return Err(Error::RustError(
            "native recovery mutation authority is unavailable".into(),
        ));
    };
    if candidate.bytes <= 0
        || !contracts::valid_sha256(&candidate.checksum_sha256)
        || !(0..=now).contains(&candidate.updated_at_ms)
        || !matches!(
            candidate.job_state.as_str(),
            "queued" | "leased" | "running" | "succeeded" | "failed" | "cancelled"
        )
        || candidate.job_attempt < candidate.attempt
        || (candidate.state == "published"
            && (candidate.job_state != "succeeded" || candidate.job_attempt != candidate.attempt))
        || (candidate.state != "published"
            && candidate.cancel_requested == 0
            && candidate.job_state != "failed"
            && candidate.job_attempt <= candidate.attempt)
        || !valid_private_object_key(
            &candidate.final_object_key,
            &candidate.organization_id,
            &candidate.video_id,
        )
        || candidate.staging_object_key
            != format!(
                "{}.attempt-{}.{}.partial",
                candidate.final_object_key, candidate.attempt, candidate.checksum_sha256
            )
    {
        return mark_native_staging_conflict(&database, &authority_fence, &candidate, now).await;
    }
    let bucket = env.bucket("RECORDINGS")?;
    if candidate.state == "published" {
        let committed = database
            .prepare(
                "SELECT 1 AS ready FROM object_manifests m \
                 WHERE m.object_key = ?1 AND m.organization_id = ?2 AND m.video_id = ?3 \
                   AND m.bytes = ?4 AND m.checksum_sha256 = ?5 AND m.content_type = ?6 \
                   AND m.state = 'available' LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&candidate.final_object_key),
                JsValue::from_str(&candidate.organization_id),
                JsValue::from_str(&candidate.video_id),
                JsValue::from_f64(candidate.bytes as f64),
                JsValue::from_str(&candidate.checksum_sha256),
                JsValue::from_str(&candidate.content_type),
            ])?
            .first::<ReadyRow>(None)
            .await?
            .is_some_and(|row| row.ready == 1);
        if !committed {
            return mark_native_staging_conflict(&database, &authority_fence, &candidate, now)
                .await;
        }
        bucket.delete(&candidate.staging_object_key).await?;
        if !r2_absent_twice(&bucket, &candidate.staging_object_key).await? {
            return Ok(());
        }
        return mark_native_staging_cleaned(&database, &authority_fence, &candidate, now).await;
    }

    bucket.delete(&candidate.staging_object_key).await?;
    if !r2_absent_twice(&bucket, &candidate.staging_object_key).await? {
        return Ok(());
    }
    let published_authority_exists = database
        .prepare(
            "SELECT 1 AS ready WHERE EXISTS (SELECT 1 FROM object_manifests m \
                   WHERE m.object_key = ?1 AND m.organization_id = ?2 AND m.state = 'available') \
                 OR EXISTS (SELECT 1 FROM storage_governed_objects_v1 g \
                   WHERE g.object_key = ?1 AND g.organization_id = ?2 AND g.state = 'active') \
                 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&candidate.final_object_key),
            JsValue::from_str(&candidate.organization_id),
        ])?
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1);
    if published_authority_exists {
        return mark_native_staging_conflict(&database, &authority_fence, &candidate, now).await;
    }
    // Promotion happens before the D1 publication batch. A lease-loss or
    // failed batch can therefore leave an exact but unauthorized final object
    // even when cancellation was not requested. Remove it for every abandoned
    // receiving/staged attempt so the next fenced attempt is not poisoned.
    if let Some(final_object) = bucket.head(&candidate.final_object_key).await? {
        let checksum = parse_sha256(&candidate.checksum_sha256)
            .ok_or_else(|| Error::RustError("native recovery checksum is invalid".into()))?;
        let metadata = final_object.http_metadata();
        let custom = final_object.custom_metadata()?;
        let attempt = candidate.attempt.to_string();
        if final_object.size() != candidate.bytes as u64
            || final_object.checksum().sha256.as_deref() != Some(checksum.as_slice())
            || metadata.content_type.as_deref() != Some(candidate.content_type.as_str())
            || metadata.content_encoding.is_some()
            || custom.get("executor").map(String::as_str) != Some("native-gstreamer-v1")
            || custom.get("job-id").map(String::as_str) != Some(candidate.job_id.as_str())
            || custom.get("attempt").map(String::as_str) != Some(attempt.as_str())
        {
            return mark_native_staging_conflict(&database, &authority_fence, &candidate, now)
                .await;
        }
        bucket.delete(&candidate.final_object_key).await?;
    }
    if !r2_absent_twice(&bucket, &candidate.final_object_key).await? {
        return Ok(());
    }

    let statements = vec![
        database
            .prepare(
                "UPDATE media_native_output_staging_v1 SET state = 'cleaned', updated_at_ms = ?3 \
                 WHERE job_id = ?1 AND attempt = ?2 AND state IN ('receiving','staged')",
            )
            .bind(&[
                JsValue::from_str(&candidate.job_id),
                JsValue::from_f64(candidate.attempt as f64),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE media_jobs SET state = 'cancelled', progress_basis_points = 0, \
                   worker_id = NULL, lease_token_digest = NULL, lease_expires_at_ms = NULL, \
                   updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND cancel_requested = 1 \
                   AND state IN ('queued','leased','running','cancelled') \
                   AND NOT EXISTS (SELECT 1 FROM object_manifests m \
                     WHERE m.object_key = media_jobs.output_object_key AND m.state = 'available')",
            )
            .bind(&[
                JsValue::from_str(&candidate.job_id),
                JsValue::from_str(&candidate.organization_id),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = 'cancelled', failure_class = 'cancelled', \
                   lease_token_digest = NULL, lease_expires_at_ms = NULL, updated_at_ms = ?2 \
                 WHERE job_id = ?1 AND selected_executor = 'native_gstreamer' \
                   AND state NOT IN ('succeeded','failed','cancelled','dead_letter') \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                     AND j.cancel_requested = 1 AND j.state = 'cancelled')",
            )
            .bind(&[
                JsValue::from_str(&candidate.job_id),
                JsValue::from_f64(now as f64),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-recovery:{}:{}", candidate.job_id, candidate.attempt),
            now,
            statements,
        )
        .await?,
    )
}

async fn r2_absent_twice(bucket: &Bucket, key: &str) -> Result<bool> {
    for _ in 0..2 {
        if bucket.head(key).await?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn finalize_native_cancel_without_staging(
    env: &Env,
    config: &RuntimeConfig,
    database: &D1Database,
    now: i64,
) -> Result<()> {
    let candidate = database
        .prepare(
            "SELECT j.id AS job_id, j.organization_id FROM media_jobs j \
             WHERE j.selected_executor = 'native_gstreamer' AND j.cancel_requested = 1 \
               AND j.state IN ('leased','running','cancelled') \
               AND (j.lease_expires_at_ms IS NULL OR j.lease_expires_at_ms <= ?1) \
               AND NOT EXISTS (SELECT 1 FROM media_native_output_staging_v1 s \
                 WHERE s.job_id = j.id AND s.state IN ('receiving','staged','published')) \
               AND NOT EXISTS (SELECT 1 FROM object_manifests m \
                 WHERE m.object_key = j.output_object_key AND m.state = 'available') \
             ORDER BY j.updated_at_ms, j.id LIMIT 1",
        )
        .bind(&[JsValue::from_f64(now as f64)])?
        .first::<NativeCancellationRecoveryRow>(None)
        .await?;
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let Some(authority_fence) =
        mutation_authority_fence(env, config, &candidate.organization_id).await?
    else {
        return Err(Error::RustError(
            "native cancellation mutation authority is unavailable".into(),
        ));
    };
    let statements = vec![
        database
            .prepare(
                "UPDATE media_jobs SET state = 'cancelled', progress_basis_points = 0, \
                   worker_id = NULL, lease_token_digest = NULL, lease_expires_at_ms = NULL, \
                   updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND cancel_requested = 1 \
                   AND state IN ('leased','running','cancelled') \
                   AND NOT EXISTS (SELECT 1 FROM media_native_output_staging_v1 s \
                     WHERE s.job_id = media_jobs.id \
                       AND s.state IN ('receiving','staged','published'))",
            )
            .bind(&[
                JsValue::from_str(&candidate.job_id),
                JsValue::from_str(&candidate.organization_id),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = 'cancelled', failure_class = 'cancelled', \
                   lease_token_digest = NULL, lease_expires_at_ms = NULL, updated_at_ms = ?2 \
                 WHERE job_id = ?1 AND selected_executor = 'native_gstreamer' \
                   AND state NOT IN ('succeeded','failed','cancelled','dead_letter') \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                     AND j.cancel_requested = 1 AND j.state = 'cancelled')",
            )
            .bind(&[
                JsValue::from_str(&candidate.job_id),
                JsValue::from_f64(now as f64),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            database,
            &authority_fence,
            &format!("native-cancel-recovery:{}", candidate.job_id),
            now,
            statements,
        )
        .await?,
    )
}

async fn mark_native_staging_cleaned(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    candidate: &NativeStagingRecoveryRow,
    now: i64,
) -> Result<()> {
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!(
                "native-recovery-clean:{}:{}",
                candidate.job_id, candidate.attempt
            ),
            now,
            vec![database
                .prepare(
                    "UPDATE media_native_output_staging_v1 SET state = 'cleaned', updated_at_ms = ?3 \
                     WHERE job_id = ?1 AND attempt = ?2 AND state = 'published'",
                )
                .bind(&[
                    JsValue::from_str(&candidate.job_id),
                    JsValue::from_f64(candidate.attempt as f64),
                    JsValue::from_f64(now as f64),
                ])?],
        )
        .await?,
    )
}

async fn mark_native_staging_conflict(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    candidate: &NativeStagingRecoveryRow,
    now: i64,
) -> Result<()> {
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!(
                "native-recovery-conflict:{}:{}",
                candidate.job_id, candidate.attempt
            ),
            now,
            vec![database
                .prepare(
                    "UPDATE media_native_output_staging_v1 SET state = 'conflict', updated_at_ms = ?3 \
                     WHERE job_id = ?1 AND attempt = ?2 AND state NOT IN ('cleaned','conflict')",
                )
                .bind(&[
                    JsValue::from_str(&candidate.job_id),
                    JsValue::from_f64(candidate.attempt as f64),
                    JsValue::from_f64(now as f64),
                ])?],
        )
        .await?,
    )?;
    console_error!("native media staging conflict class=immutable_conflict");
    Ok(())
}

async fn dispatch(
    mut request: Request,
    env: &Env,
    config: &RuntimeConfig,
    context: &Context,
    request_id: &str,
) -> Result<Response> {
    let target = match parse_raw_request_target(&request.inner().url()) {
        Ok(target) => target,
        Err(_) => {
            return failure_response(
                ApiFailure::new(
                    400,
                    "invalid_request_target",
                    "The request target is invalid.",
                    false,
                ),
                request_id,
                config.production(),
            );
        }
    };
    let route = classify_raw_path(&target.path);
    // `main` has already normalized the request ID (which may read `cf-ray`),
    // and the raw target has been parsed above. This exact reserved path gets a
    // fixed production 404 before dispatch reads Host or route-specific method,
    // token, content-type, content-length, or body data. No parity with every
    // other unknown route is claimed.
    if local_repository_conformance_hidden(&route, config.production()) {
        return failure_response(not_found_failure(), request_id, true);
    }
    let host = request.headers().get("host")?;
    let primary_host = validate_host(&target, host.as_deref(), &config.host_policy).is_ok();
    let custom_host = if primary_host {
        false
    } else if target.scheme == "https"
        && host.as_deref() == Some(target.authority.as_str())
        && matches!(
            route,
            Route::PublicShare { .. }
                | Route::PublicMedia { .. }
                | Route::PublicCollaborationGrant { .. }
                | Route::PublicComments { .. }
                | Route::PublicTranscript { .. }
                | Route::PublicAnalyticsConsent { .. }
                | Route::PublicAnalyticsEvents { .. }
                | Route::StorageGrantRead { .. }
        )
    {
        storage_governance_runtime::D1StorageGovernanceRepository::new(&env.d1("DB")?)
            .verified_domain(&target.authority)
            .await
            .map_err(|_| Error::RustError("custom domain authority is unavailable".into()))?
            .is_some()
    } else {
        false
    };
    if !primary_host && !custom_host {
        return failure_response(
            ApiFailure::new(
                421,
                "unexpected_host",
                "The request host is not served here.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let canonical_origin = format!("{}://{}", target.scheme, target.authority);
    let response = match route {
        Route::LegacyRoot => method_guard(&request, &[Method::Get], "GET")?.map_or_else(
            || Response::ok("Frame control plane. See /health."),
            |failure| failure_response(failure, request_id, config.production()),
        )?,
        Route::LegacyHealth => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                health_response(env, config).await?
            }
        }
        Route::LegacyMediaServerRoot => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_media_server_root_response(
                    &mut request,
                    env,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::LegacyApiStatus => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_api_status_response(&mut request, env, request_id, config.production())
                    .await?
            }
        }
        Route::LegacyMobileSessionConfig => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_session_config_response(
                    &mut request,
                    env,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::LegacyMobileEmailSessionRequest => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_session_web_runtime::email_request_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileEmailSessionVerify => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_session_web_runtime::email_verify_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileSessionRequest => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_session_web_runtime::session_request_response(
                    &request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileSessionRevoke => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_session_web_runtime::session_revoke_response(
                    &request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileUploadCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_uploads_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_uploads_web_runtime::LegacyMobileUploadsRouteV1::Create,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileUploadComplete { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_uploads_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_uploads_web_runtime::LegacyMobileUploadsRouteV1::Complete {
                        video_id: &video_id,
                    },
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileUploadProgress { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_uploads_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_uploads_web_runtime::LegacyMobileUploadsRouteV1::Progress {
                        video_id: &video_id,
                    },
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileBootstrap => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_bootstrap_caps_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_bootstrap_caps_web_runtime::LegacyMobileBootstrapCapsRouteV1::Bootstrap,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileCaps => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_bootstrap_caps_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_bootstrap_caps_web_runtime::LegacyMobileBootstrapCapsRouteV1::List,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileCap { video_id } => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Delete], "GET, DELETE")?
            {
                failure_response(failure, request_id, config.production())?
            } else {
                let route = if request.method() == Method::Delete {
                    legacy_mobile_bootstrap_caps_web_runtime::LegacyMobileBootstrapCapsRouteV1::Delete {
                        video_id: &video_id,
                    }
                } else {
                    legacy_mobile_bootstrap_caps_web_runtime::LegacyMobileBootstrapCapsRouteV1::Get {
                        video_id: &video_id,
                    }
                };
                legacy_mobile_bootstrap_caps_web_runtime::response(
                    &mut request,
                    env,
                    route,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileCapDownload { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_bootstrap_caps_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_bootstrap_caps_web_runtime::LegacyMobileBootstrapCapsRouteV1::Download {
                        video_id: &video_id,
                    },
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileCapPlayback { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_mobile_bootstrap_caps_web_runtime::response(
                    &mut request,
                    env,
                    legacy_mobile_bootstrap_caps_web_runtime::LegacyMobileBootstrapCapsRouteV1::Playback {
                        video_id: &video_id,
                    },
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMobileFolders => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_folder_crud_web_runtime::mobile_create_response(
                    &mut request,
                    env,
                    request_id,
                )
                .await?
            }
        }
        Route::LegacyMobileCapPassword { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Patch], "PATCH")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_properties_web_runtime::mobile_response(
                    &mut request,
                    env,
                    request_id,
                    video_id,
                    legacy_video_properties_web_runtime::MobileVideoPropertyActionV1::Password,
                )
                .await?
            }
        }
        Route::LegacyMobileCapSharing { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Patch], "PATCH")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_properties_web_runtime::mobile_response(
                    &mut request,
                    env,
                    request_id,
                    video_id,
                    legacy_video_properties_web_runtime::MobileVideoPropertyActionV1::Sharing,
                )
                .await?
            }
        }
        Route::LegacyMobileCapTitle { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Patch], "PATCH")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_properties_web_runtime::mobile_response(
                    &mut request,
                    env,
                    request_id,
                    video_id,
                    legacy_video_properties_web_runtime::MobileVideoPropertyActionV1::Title,
                )
                .await?
            }
        }
        Route::LegacyMobileCapComments { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_collaboration_web_runtime::mobile_create_comment_response(
                    &mut request,
                    env,
                    request_id,
                    video_id,
                )
                .await?
            }
        }
        Route::LegacyMobileCapReactions { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_collaboration_web_runtime::mobile_create_reaction_response(
                    &mut request,
                    env,
                    request_id,
                    video_id,
                )
                .await?
            }
        }
        Route::LegacyMobileComment { comment_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Delete], "DELETE")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_collaboration_web_runtime::mobile_delete_comment_response(
                    &request, env, request_id, comment_id,
                )
                .await?
            }
        }
        Route::LegacyWebCommentDelete => {
            if let Some(failure) = method_guard(&request, &[Method::Delete], "DELETE")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_collaboration_web_runtime::web_delete_comment_response(
                    &request, env, request_id,
                )
                .await?
            }
        }
        Route::LegacyAnalytics => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_analytics_web_runtime::http_response(
                    legacy_analytics_web_runtime::LegacyAnalyticsHttpRouteV1::VideoCount,
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyAnalyticsTrack => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_analytics_web_runtime::http_response(
                    legacy_analytics_web_runtime::LegacyAnalyticsHttpRouteV1::Track,
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyDashboardAnalytics => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_analytics_web_runtime::http_response(
                    legacy_analytics_web_runtime::LegacyAnalyticsHttpRouteV1::Dashboard,
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyVideoMetadata => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_properties_web_runtime::metadata_response(
                    &mut request,
                    env,
                    request_id,
                )
                .await?
            }
        }
        Route::LegacyVideoAnalytics => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_analytics_web_runtime::http_response(
                    legacy_analytics_web_runtime::LegacyAnalyticsHttpRouteV1::VideoHttp,
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyVideoDomainInfo => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_domain_info_web_runtime::response(&request, env).await?
            }
        }
        Route::LegacyVideoDelete => {
            if let Some(failure) = method_guard(&request, &[Method::Delete], "DELETE")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_lifecycle_web_runtime::delete_route_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyVideoOg => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_video_lifecycle_web_runtime::og_response(
                    &request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyRetryTranscription { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_transcripts_web_runtime::retry_response(
                    &request,
                    env,
                    &video_id,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyProtectedMedia => {
            legacy_protected_media_route_dispatch(
                &mut request,
                env,
                &target.path,
                request_id,
                config.production(),
            )
            .await?
        }
        Route::LegacyProtectedIntegration { operation_id } => {
            legacy_protected_integrations_web_runtime::route_response(
                operation_id,
                &mut request,
                env,
                current_time_ms()?,
            )
            .await?
        }
        Route::LegacyProtectedBillingAuth => {
            legacy_protected_billing_auth_route_dispatch(
                &mut request,
                env,
                &target.path,
                request_id,
                config.production(),
            )
            .await?
        }
        Route::LegacyEffectRpc => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Post], "GET, POST")?
            {
                failure_response(failure, request_id, config.production())?
            } else if request.method() == Method::Get {
                legacy_video_lifecycle_web_runtime::effect_rpc_get_response()?
            } else {
                legacy_folder_crud_web_runtime::effect_rpc_response(&mut request, env, request_id)
                    .await?
            }
        }
        Route::LegacyUserName => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_user_account_web_runtime::name_route_response(&mut request, env, request_id)
                    .await?
            }
        }
        Route::LegacyInviteAccept => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_invite_lifecycle_web_runtime::response(
                    &mut request,
                    env,
                    request_id,
                    frame_application::LegacyInviteActionV1::Accept,
                )
                .await?
            }
        }
        Route::LegacyInviteDecline => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_invite_lifecycle_web_runtime::response(
                    &mut request,
                    env,
                    request_id,
                    frame_application::LegacyInviteActionV1::Decline,
                )
                .await?
            }
        }
        Route::LegacyExtensionAuthStart => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_auth_web_runtime::start_response(
                    &request,
                    env,
                    config,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyExtensionAuthApprove => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_auth_web_runtime::approve_response(
                    &mut request,
                    env,
                    config,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyExtensionAuthRevoke => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_auth_web_runtime::revoke_response(
                    &request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyExtensionBootstrap => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_auth_web_runtime::bootstrap_response(
                    &request,
                    env,
                    config,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyExtensionInstantCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_instant_recordings_web_runtime::create_response(
                    &mut request,
                    env,
                    config,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyExtensionInstantProgress => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_instant_recordings_web_runtime::progress_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyExtensionInstantDelete { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Delete], "DELETE")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_extension_instant_recordings_web_runtime::delete_response(
                    &mut request,
                    env,
                    &video_id,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyNotifications => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_notifications_response(&request, env, request_id, config.production())
                    .await?
            }
        }
        Route::LegacyNotificationPreferences => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_notification_preferences_response(
                    &mut request,
                    env,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::LegacyDesktopOrgCustomDomain => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Options], "GET, OPTIONS")?
            {
                let response = failure_response(failure, request_id, config.production())?;
                let request_origin = request.headers().get("origin")?;
                legacy_org_custom_domain_web_runtime::cors_response(
                    response,
                    request_origin.as_deref(),
                    &canonical_origin,
                )?
            } else {
                legacy_desktop_org_custom_domain_response(
                    &request,
                    env,
                    request_id,
                    config.production(),
                    &canonical_origin,
                )
                .await?
            }
        }
        Route::LegacyDesktopOrganizations => {
            legacy_desktop_compatibility_route_response(
                &mut request,
                env,
                request_id,
                config.production(),
                &canonical_origin,
                Method::Get,
                "GET, OPTIONS",
                legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1::Organizations,
            )
            .await?
        }
        Route::LegacyDesktopOrganizationBranding { organization_id } => {
            legacy_desktop_compatibility_route_response(
                &mut request,
                env,
                request_id,
                config.production(),
                &canonical_origin,
                Method::Patch,
                "PATCH, OPTIONS",
                legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1::OrganizationBranding {
                    organization_id: &organization_id,
                },
            )
            .await?
        }
        Route::LegacyDesktopStorageSetActive => {
            legacy_desktop_compatibility_route_response(
                &mut request,
                env,
                request_id,
                config.production(),
                &canonical_origin,
                Method::Post,
                "POST, OPTIONS",
                legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1::StorageSetActive,
            )
            .await?
        }
        Route::LegacyDesktopUserProfile => {
            legacy_desktop_compatibility_route_response(
                &mut request,
                env,
                request_id,
                config.production(),
                &canonical_origin,
                Method::Get,
                "GET, OPTIONS",
                legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1::UserProfile,
            )
            .await?
        }
        Route::LegacyDesktopVideoDelete => {
            legacy_desktop_compatibility_route_response(
                &mut request,
                env,
                request_id,
                config.production(),
                &canonical_origin,
                Method::Delete,
                "DELETE, OPTIONS",
                legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1::VideoDelete,
            )
            .await?
        }
        Route::LegacyDesktopVideoProgress => {
            legacy_desktop_compatibility_route_response(
                &mut request,
                env,
                request_id,
                config.production(),
                &canonical_origin,
                Method::Post,
                "POST, OPTIONS",
                legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1::VideoProgress,
            )
            .await?
        }
        Route::LegacyDesktopSessionRequest => {
            let request_origin = request.headers().get("origin")?;
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Options], "GET, OPTIONS")?
            {
                legacy_org_custom_domain_web_runtime::cors_response(
                    failure_response(failure, request_id, config.production())?,
                    request_origin.as_deref(),
                    &canonical_origin,
                )?
            } else if request.method() == Method::Options {
                legacy_org_custom_domain_web_runtime::preflight_response(
                    &request,
                    &canonical_origin,
                )?
            } else {
                let response = legacy_desktop_session_web_runtime::response(
                    &request,
                    env,
                    config,
                    current_time_ms()?,
                )
                .await?;
                legacy_org_custom_domain_web_runtime::cors_response(
                    response,
                    request_origin.as_deref(),
                    &canonical_origin,
                )?
            }
        }
        Route::LegacyChangelog => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Options], "GET, OPTIONS")?
            {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_changelog_response(
                    &mut request,
                    env,
                    request_id,
                    config.production(),
                    &canonical_origin,
                )
                .await?
            }
        }
        Route::LegacyChangelogStatus => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Options], "GET, OPTIONS")?
            {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_changelog_status_response(&mut request, env, request_id, config.production())
                .await?
            }
        }
        Route::LegacyDownload => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::download_response(&request)?
            }
        }
        Route::LegacyPlaylist => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Head], "GET, HEAD")?
            {
                failure_response(failure, request_id, config.production())?
            } else {
                let head_only = request.method() == Method::Head;
                legacy_core_storage_web_runtime::playlist_response(
                    &request,
                    env,
                    current_time_ms()?,
                    head_only,
                )
                .await?
            }
        }
        Route::LegacyStorageObject => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Head], "GET, HEAD")?
            {
                failure_response(failure, request_id, config.production())?
            } else {
                let head_only = request.method() == Method::Head;
                legacy_core_storage_web_runtime::storage_object_response(
                    &request,
                    env,
                    current_time_ms()?,
                    head_only,
                )
                .await?
            }
        }
        Route::LegacyMultipartAbort => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::multipart_abort_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMultipartComplete => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::multipart_complete_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMultipartInitiate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::multipart_initiate_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyMultipartPresignPart => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::multipart_presign_part_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyRecordingComplete => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::recording_complete_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacySignedUpload => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::signed_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacySignedUploadBatch => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                legacy_core_storage_web_runtime::signed_batch_response(
                    &mut request,
                    env,
                    current_time_ms()?,
                )
                .await?
            }
        }
        Route::LegacyDeveloperStorageCron => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::StorageCron,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperMultipartAbort => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::MultipartAbort,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperMultipartComplete => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::MultipartComplete,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperMultipartInitiate => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::MultipartInitiate,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperMultipartPresign => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::MultipartPresign,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperVideoCreate => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::VideoCreate,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperUsage => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::Usage,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperVideos => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::VideosList,
                    video_id: None,
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperVideo { video_id } => {
            let surface = if request.method() == Method::Delete {
                frame_application::LegacyDeveloperApiSurfaceV1::VideoDelete
            } else {
                frame_application::LegacyDeveloperApiSurfaceV1::VideoGet
            };
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface,
                    video_id: Some(video_id),
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::LegacyDeveloperVideoStatus { video_id } => {
            legacy_developer_api_web_runtime::response(
                legacy_developer_api_web_runtime::LegacyDeveloperApiRouteV1 {
                    surface: frame_application::LegacyDeveloperApiSurfaceV1::VideoStatus,
                    video_id: Some(video_id),
                },
                &mut request,
                env,
            )
            .await?
        }
        Route::ApiHealth => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                public_health_response(env, config).await?
            }
        }
        Route::Discovery => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                Response::from_json(&DiscoveryResponse::default())?
            }
        }
        Route::Capabilities => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let capabilities = CapabilitiesResponse {
                    media_jobs: match config.media_mode {
                        MediaMode::Fake => "authenticated_local_fake_preview",
                        MediaMode::Remote => "durable_hybrid_managed_native_media_v1",
                        MediaMode::Native => "service_authenticated_native_worker",
                    },
                    ..CapabilitiesResponse::default()
                };
                Response::from_json(&capabilities)?
            }
        }
        Route::PublicShare { share_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                public_share_response(env, &share_id, &canonical_origin).await?
            }
        }
        Route::PublicMedia { share_id } => {
            if let Some(failure) = method_guard(
                &request,
                &[Method::Get, Method::Head, Method::Options],
                "GET, HEAD, OPTIONS",
            )? {
                failure_response(failure, request_id, config.production())?
            } else if request.method() == Method::Options {
                storage_preflight_response(
                    env,
                    &request,
                    &canonical_origin,
                    request_id,
                    config.production(),
                )?
            } else {
                public_media_response(
                    env,
                    &request,
                    &share_id,
                    &canonical_origin,
                    &config.host_policy.public_host,
                    request.method() == Method::Head,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::PublicCollaborationGrant { share_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&share_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let outcome = public_collaboration_runtime::issue_grant(
                    &env.d1("DB")?,
                    &share_id,
                    current_time_ms()?,
                    request_id,
                )
                .await?;
                public_collaboration_response(outcome, 201, request_id, config.production())?
            }
        }
        Route::PublicComments { share_id } => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Post], "GET, POST")?
            {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&share_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else if request.method() == Method::Get {
                let outcome =
                    public_collaboration_runtime::list_comments(&env.d1("DB")?, &share_id).await?;
                public_collaboration_response(outcome, 200, request_id, config.production())?
            } else {
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let Some(token) = public_collaboration_token(&request)? else {
                    return failure_response(not_found_failure(), request_id, config.production());
                };
                let body = match request.json::<PublicCommentCommandV1>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if idempotency_header(&request)? != body.idempotency_key {
                    return failure_response(
                        invalid_body_failure("invalid_idempotency_key"),
                        request_id,
                        config.production(),
                    );
                }
                let outcome = public_collaboration_runtime::create_comment(
                    &env.d1("DB")?,
                    &share_id,
                    &token,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?;
                public_collaboration_response(outcome, 201, request_id, config.production())?
            }
        }
        Route::PublicTranscript { share_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&share_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let outcome =
                    public_collaboration_runtime::transcript(&env.d1("DB")?, &share_id).await?;
                public_collaboration_response(outcome, 200, request_id, config.production())?
            }
        }
        Route::PublicAnalyticsConsent { share_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&share_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let Some(token) = public_collaboration_token(&request)? else {
                    return failure_response(not_found_failure(), request_id, config.production());
                };
                let body = match request.json::<PublicAnalyticsConsentCommandV1>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if idempotency_header(&request)? != body.idempotency_key {
                    return failure_response(
                        invalid_body_failure("invalid_idempotency_key"),
                        request_id,
                        config.production(),
                    );
                }
                let outcome = public_collaboration_runtime::set_analytics_consent(
                    &env.d1("DB")?,
                    &share_id,
                    &token,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?;
                public_collaboration_response(outcome, 200, request_id, config.production())?
            }
        }
        Route::PublicAnalyticsEvents { share_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&share_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let Some(token) = public_collaboration_token(&request)? else {
                    return failure_response(not_found_failure(), request_id, config.production());
                };
                let body = match request.json::<PublicAnalyticsEventCommandV1>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if idempotency_header(&request)? != body.idempotency_key {
                    return failure_response(
                        invalid_body_failure("invalid_idempotency_key"),
                        request_id,
                        config.production(),
                    );
                }
                let outcome = public_collaboration_runtime::record_analytics(
                    &env.d1("DB")?,
                    &share_id,
                    &token,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?;
                public_collaboration_response(outcome, 202, request_id, config.production())?
            }
        }
        auth_route @ (Route::BrowserAuthLogin
        | Route::BrowserAuthSignup
        | Route::BrowserAuthRecovery) => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let action = match auth_route {
                    Route::BrowserAuthLogin => worker_auth_runtime::BrowserAuthStart::Login,
                    Route::BrowserAuthSignup => worker_auth_runtime::BrowserAuthStart::Signup,
                    Route::BrowserAuthRecovery => worker_auth_runtime::BrowserAuthStart::Recovery,
                    _ => unreachable!("closed browser auth route"),
                };
                match worker_auth_runtime::start(&mut request, env, action, current_time_ms()?)
                    .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(failure)) => browser_auth_page_failure_response(action, failure)?,
                    Err(_) => {
                        console_error!(
                            "browser authentication start failed request_id={request_id}"
                        );
                        browser_auth_page_failure_response(
                            action,
                            browser_web_runtime::BrowserWebFailure::Unavailable,
                        )?
                    }
                }
            }
        }
        Route::BrowserAuthVerify => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                match worker_auth_runtime::verify(&mut request, env, current_time_ms()?).await {
                    Ok(Ok(response)) => response,
                    Ok(Err(failure)) => browser_auth_verify_failure_response(failure)?,
                    Err(_) => {
                        console_error!(
                            "browser authentication verify failed request_id={request_id}"
                        );
                        browser_auth_verify_failure_response(
                            browser_web_runtime::BrowserWebFailure::Unavailable,
                        )?
                    }
                }
            }
        }
        Route::BrowserAuthLogout => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                match worker_auth_runtime::logout(&request, env, current_time_ms()?).await? {
                    Ok(response) => response,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_logout"),
                        request_id,
                        config.production(),
                    )?,
                }
            }
        }
        Route::AuthenticatedWebWorkspace { surface } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let query = match request.query::<authenticated_web_runtime::WebLoadQuery>() {
                    Ok(query) => query,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_query"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match browser_web_runtime::load(&request, env, &surface, &query, current_time_ms()?)
                    .await?
                {
                    Ok(workspace) => Response::from_json(&workspace)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_query"),
                        request_id,
                        config.production(),
                    )?,
                }
            }
        }
        Route::AuthenticatedWebAction { action } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let body = match browser_web_runtime::decode_action_request(&mut request).await? {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_body"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match browser_web_runtime::mutate(&request, env, &action, &body, current_time_ms()?)
                    .await?
                {
                    Ok(receipt) => {
                        let status = match receipt.effect_state {
                            browser_web_runtime::WebActionEffectState::Applied => 200,
                            browser_web_runtime::WebActionEffectState::PendingProtectedExecution => {
                                202
                            }
                        };
                        Response::from_json(&receipt)?.with_status(status)
                    }
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_body"),
                        request_id,
                        config.production(),
                    )?,
                }
            }
        }
        Route::AuthenticatedWebCompatibilityAction { operation_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if legacy_space_authorization_web_runtime::is_action(&operation_id) {
                let body = match legacy_space_authorization_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_space_authorization_web_runtime::read(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(result) => legacy_space_authorization_response(result)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_library_detail_read_web_runtime::is_action(&operation_id) {
                let body = match legacy_library_detail_read_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_library_detail_read_web_runtime::read(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(result) => legacy_library_detail_read_response(result)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_library_id_read_web_runtime::is_action(&operation_id) {
                let body = match legacy_library_id_read_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_library_id_read_web_runtime::read(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(result) => legacy_library_id_read_response(result)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_folder_web_runtime::is_action(&operation_id) {
                let body = match legacy_folder_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_folder_web_runtime::mutate(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_folder_assignment_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_library_web_runtime::is_action(&operation_id) {
                let body = match legacy_library_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_library_web_runtime::mutate(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_library_placement_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_notification_web_runtime::is_action(&operation_id) {
                let body = match legacy_notification_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_notification_web_runtime::mutate(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_notification_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_developer_web_runtime::is_action(&operation_id) {
                let body = match legacy_developer_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_developer_web_runtime::mutate(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_developer_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_membership_web_runtime::is_action(&operation_id) {
                let body = match legacy_membership_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_membership_web_runtime::mutate(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_membership_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_collaboration_web_runtime::is_action(&operation_id) {
                let body = match legacy_collaboration_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_collaboration_web_runtime::mutate_action(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_collaboration_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_video_properties_web_runtime::is_action(&operation_id) {
                let body = match legacy_video_properties_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                    current_time_ms()?,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                legacy_video_properties_web_runtime::action_response(
                    &request, env, &body, request_id,
                )
                .await?
            } else if legacy_organization_library_web_runtime::is_action(&operation_id) {
                let body = match legacy_organization_library_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                legacy_organization_library_web_runtime::action_response(&request, env, &body)
                    .await?
            } else if legacy_protected_media_web_runtime::is_server_action(&operation_id) {
                let body = match legacy_protected_media_web_runtime::decode_server_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_protected_media_web_runtime::server_action_http_response(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                )
                .await?
                {
                    Ok(response) => response,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_protected_integrations_web_runtime::is_server_action(&operation_id) {
                let body = match legacy_protected_integrations_web_runtime::decode_server_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_protected_integrations_web_runtime::server_action_http_response(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                )
                .await?
                {
                    Ok(response) => response,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_protected_billing_auth_web_runtime::is_server_action(&operation_id) {
                let body = match legacy_protected_billing_auth_web_runtime::decode_server_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_protected_billing_auth_web_runtime::server_action_http_response(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                )
                .await?
                {
                    Ok(response) => response,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_upload_storage_web_runtime::is_action(&operation_id) {
                let body = match legacy_upload_storage_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_upload_storage_web_runtime::action(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                )
                .await?
                {
                    Ok(result) => Response::from_json(&result)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_analytics_web_runtime::is_action(&operation_id) {
                let body = match legacy_analytics_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                legacy_analytics_web_runtime::action_response(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                )
                .await?
            } else if legacy_transcripts_web_runtime::is_action(&operation_id) {
                let body = match legacy_transcripts_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_transcripts_web_runtime::action(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                )
                .await?
                {
                    Ok(result) => Response::from_json(&result)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else if legacy_user_account_web_runtime::is_action(&operation_id) {
                let body = match legacy_user_account_web_runtime::decode_action_request(
                    &mut request,
                    &operation_id,
                )
                .await?
                {
                    Ok(body) => body,
                    Err(failure) => {
                        return failure_response(
                            browser_web_failure(failure, "invalid_compatibility_action"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                match legacy_user_account_web_runtime::mutate_action(
                    &request,
                    env,
                    &body,
                    current_time_ms()?,
                    request_id,
                    config.production(),
                )
                .await?
                {
                    Ok(effect) => legacy_user_account_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            } else {
                let body =
                    match legacy_web_action_runtime::decode_action_request(&mut request).await? {
                        Ok(body) => body,
                        Err(failure) => {
                            return failure_response(
                                browser_web_failure(failure, "invalid_compatibility_action"),
                                request_id,
                                config.production(),
                            );
                        }
                    };
                match legacy_web_action_runtime::mutate(
                    &request,
                    env,
                    &operation_id,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?
                {
                    Ok(effect) => legacy_web_compatibility_action_response(effect)?,
                    Err(failure) => failure_response(
                        browser_web_failure(failure, "invalid_compatibility_action"),
                        request_id,
                        config.production(),
                    )?,
                }
            }
        }
        Route::StorageGrantCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_storage_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CreateStorageGrantRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                storage_grant_create_response(env, config, &request, &actor, body, request_id)
                    .await?
            }
        }
        Route::StorageGrantRevoke { grant_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Delete], "DELETE")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Admin,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                storage_grant_revoke_response(env, config, &request, &actor, &grant_id, request_id)
                    .await?
            }
        }
        Route::StorageGrantRead {
            tenant_id,
            grant_id,
        } => {
            if let Some(failure) = method_guard(
                &request,
                &[Method::Get, Method::Head, Method::Options],
                "GET, HEAD, OPTIONS",
            )? {
                failure_response(failure, request_id, config.production())?
            } else if request.method() == Method::Options {
                storage_preflight_response(
                    env,
                    &request,
                    &canonical_origin,
                    request_id,
                    config.production(),
                )?
            } else {
                storage_grant_read_response(
                    env,
                    &request,
                    &tenant_id,
                    &grant_id,
                    &canonical_origin,
                    &config.host_policy.public_host,
                    request.method() == Method::Head,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::VideoCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CreateVideoRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                video_create_response(env, config, &request, &actor, body, request_id).await?
            }
        }
        Route::VideoPrivacy { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Patch], "PATCH")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&video_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<UpdatePrivacyRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                video_privacy_response(env, config, &request, &actor, &video_id, body, request_id)
                    .await?
            }
        }
        Route::VideoTranscript { video_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&video_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                let Some(tenant_id) =
                    authorized_tenant(&env.d1("DB")?, &request, &actor, RequiredAccess::Write)
                        .await?
                else {
                    return failure_response(not_found_failure(), request_id, config.production());
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<PublicTranscriptV1>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                let outcome = public_collaboration_runtime::publish_transcript(
                    &env.d1("DB")?,
                    &video_id,
                    &tenant_id,
                    &actor.user_id,
                    &body,
                    current_time_ms()?,
                    request_id,
                )
                .await?;
                public_collaboration_response(outcome, 200, request_id, config.production())?
            }
        }
        Route::UploadIntent => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<UploadIntentRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                upload_intent_response(env, config, &request, &actor, body, request_id).await?
            }
        }
        Route::UploadStatus { upload_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Read,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                upload_status_response(
                    env,
                    &request,
                    &actor,
                    &upload_id,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::UploadContent { upload_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                upload_content_response(env, config, &mut request, &actor, &upload_id, request_id)
                    .await?
            }
        }
        Route::UploadFinalize { upload_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<DirectUploadFinalizeRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                direct_upload_finalize_response(
                    env, config, &request, &actor, &upload_id, body, request_id,
                )
                .await?
            }
        }
        Route::UploadMultipart { upload_id } => {
            if let Some(failure) = method_guard(
                &request,
                &[Method::Post, Method::Get, Method::Delete],
                "POST, GET, DELETE",
            )? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                multipart_session_response(env, config, &request, &actor, &upload_id, request_id)
                    .await?
            }
        }
        Route::UploadMultipartPart {
            upload_id,
            part_number,
        } => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                multipart_part_response(
                    env,
                    config,
                    &mut request,
                    &actor,
                    &upload_id,
                    part_number,
                    request_id,
                )
                .await?
            }
        }
        Route::UploadMultipartComplete { upload_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&upload_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                multipart_complete_response(env, config, &request, &actor, &upload_id, request_id)
                    .await?
            }
        }
        Route::InstantFinalize { session_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&session_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<InstantFinalizeRequestV1>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                instant_finalize_response(
                    env,
                    config,
                    &request,
                    &actor,
                    &session_id,
                    body,
                    request_id,
                )
                .await?
            }
        }
        Route::MediaJobCreate => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_json_command_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<MediaJobRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                media_job_create_response(env, config, context, &request, &actor, body, request_id)
                    .await?
            }
        }
        Route::MediaJobStatus { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(
                    invalid_identifier_failure(),
                    request_id,
                    config.production(),
                )?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Read,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                media_job_status_response(
                    env,
                    &request,
                    &actor,
                    &job_id,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::MediaJobCancel { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(
                    invalid_identifier_failure(),
                    request_id,
                    config.production(),
                )?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Write,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_idempotency_header(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                media_job_cancel_response(env, config, &request, &actor, &job_id, request_id)
                    .await?
            }
        }
        Route::WorkerMediaJobClaim => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<WorkerClaimRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                native_job_claim_response(env, config, &request, &actor, body, request_id).await?
            }
        }
        Route::WorkerMediaJobSource { job_id }
        | Route::WorkerMediaJobSourceOrdinal { job_id, ordinal: 0 } => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Head], "GET, HEAD")?
            {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_lease_header(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                native_job_source_response(env, config, &request, &actor, &job_id, 0, request_id)
                    .await?
            }
        }
        Route::WorkerMediaJobSourceOrdinal { job_id, ordinal } => {
            if let Some(failure) =
                method_guard(&request, &[Method::Get, Method::Head], "GET, HEAD")?
            {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_lease_header(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                native_job_source_response(
                    env, config, &request, &actor, &job_id, ordinal, request_id,
                )
                .await?
            }
        }
        Route::WorkerMediaJobOutput { job_id }
        | Route::WorkerMediaJobOutputOrdinal { job_id, ordinal: 0 } => {
            if let Some(failure) = method_guard(&request, &[Method::Put], "PUT")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_output_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                native_job_output_response(env, config, &mut request, &actor, &job_id, request_id)
                    .await?
            }
        }
        Route::WorkerMediaJobOutputOrdinal { .. } => {
            failure_response(not_found_failure(), request_id, config.production())?
        }
        Route::WorkerMediaJobHeartbeat { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<WorkerHeartbeatRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                native_job_heartbeat_response(
                    env, config, &request, &actor, &job_id, body, request_id,
                )
                .await?
            }
        }
        Route::WorkerMediaJobProgress { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<WorkerProgressRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                native_job_progress_response(
                    env, config, &request, &actor, &job_id, body, request_id,
                )
                .await?
            }
        }
        Route::WorkerMediaJobComplete { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<WorkerCompleteRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                native_job_complete_response(
                    env, config, &request, &actor, &job_id, body, request_id,
                )
                .await?
            }
        }
        Route::WorkerMediaJobFail { job_id } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else if !valid_uuid(&job_id) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Worker,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_worker_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<WorkerFailRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                if let Err(code) = body.validate() {
                    return failure_response(
                        invalid_body_failure(code.as_str()),
                        request_id,
                        config.production(),
                    );
                }
                native_job_fail_response(env, config, &request, &actor, &job_id, body, request_id)
                    .await?
            }
        }
        Route::AuthorityStatus => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                if let Err(failure) =
                    authenticated_command_preflight(&request, env, config, RequiredAccess::Admin)
                        .await?
                {
                    return failure_response(failure, request_id, config.production());
                }
                authority_response(env).await?
            }
        }
        Route::CutoverStatus { tenant_id, domain } => {
            if let Some(failure) = method_guard(&request, &[Method::Get], "GET")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Admin,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                cutover_status_response(
                    env,
                    &request,
                    &actor,
                    &tenant_id,
                    &domain,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::CutoverTransition { tenant_id, domain } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Admin,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_storage_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CutoverTransitionRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                cutover_transition_response(
                    env,
                    &request,
                    &actor,
                    &tenant_id,
                    &domain,
                    body,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::CutoverReplayPause { tenant_id, domain }
        | Route::CutoverReplayResume { tenant_id, domain } => {
            let action = if target.path.ends_with("/replay/pause") {
                ReplayControlAction::Pause
            } else {
                ReplayControlAction::Resume
            };
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Admin,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_storage_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CutoverReplayControlRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                cutover_replay_control_response(
                    env,
                    &request,
                    &actor,
                    &tenant_id,
                    &domain,
                    body,
                    action,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::CutoverSignal { tenant_id, domain } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Admin,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_storage_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CutoverSignalRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                cutover_signal_response(
                    env,
                    &request,
                    &actor,
                    &tenant_id,
                    &domain,
                    body,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::CutoverShadowObservation { tenant_id, domain } => {
            if let Some(failure) = method_guard(&request, &[Method::Post], "POST")? {
                failure_response(failure, request_id, config.production())?
            } else {
                let actor = match authenticated_command_preflight(
                    &request,
                    env,
                    config,
                    RequiredAccess::Admin,
                )
                .await?
                {
                    Ok(actor) => actor,
                    Err(failure) => {
                        return failure_response(failure, request_id, config.production());
                    }
                };
                if let Err(failure) = validate_storage_json_headers(&request) {
                    return failure_response(failure, request_id, config.production());
                }
                let body = match request.json::<CutoverShadowObservationRequest>().await {
                    Ok(body) => body,
                    Err(_) => {
                        return failure_response(
                            invalid_body_failure("invalid_json"),
                            request_id,
                            config.production(),
                        );
                    }
                };
                cutover_shadow_observation_response(
                    env,
                    &request,
                    &actor,
                    &tenant_id,
                    &domain,
                    body,
                    request_id,
                    config.production(),
                )
                .await?
            }
        }
        Route::LocalRepositoryConformance => {
            if config.production() || !valid_repository_conformance_target(&target) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                repository_conformance::response(request, env).await?
            }
        }
        Route::LocalAuthRepositoryConformance => {
            if config.production() || !valid_repository_conformance_target(&target) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                auth_repository_conformance::response(request, env).await?
            }
        }
        Route::LocalOrganizationRepositoryConformance => {
            if config.production() || !valid_repository_conformance_target(&target) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                organization_repository_conformance::response(request, env).await?
            }
        }
        Route::LocalR2StorageConformance => {
            if config.production() || !valid_repository_conformance_target(&target) {
                failure_response(not_found_failure(), request_id, config.production())?
            } else {
                r2_storage::local_conformance_response(request, env).await?
            }
        }
        Route::InvalidApiPath => failure_response(
            ApiFailure::new(400, "invalid_api_path", "The API path is invalid.", false),
            request_id,
            config.production(),
        )?,
        Route::UnknownApi => {
            failure_response(not_found_failure(), request_id, config.production())?
        }
        Route::NotApi => failure_response(
            ApiFailure::new(
                404,
                "not_api_route",
                "The requested route is not handled by this service.",
                false,
            ),
            request_id,
            config.production(),
        )?,
    };
    secure_response(response, request_id, config.production())
}

fn local_repository_conformance_hidden(route: &Route, production: bool) -> bool {
    production
        && matches!(
            route,
            Route::LocalRepositoryConformance
                | Route::LocalAuthRepositoryConformance
                | Route::LocalOrganizationRepositoryConformance
                | Route::LocalR2StorageConformance
        )
}

fn method_guard(
    request: &Request,
    accepted: &[Method],
    allow: &'static str,
) -> Result<Option<ApiFailure>> {
    let method = request.method();
    Ok((!accepted.contains(&method)).then(|| {
        ApiFailure::new(
            405,
            "method_not_allowed",
            "The request method is not allowed for this route.",
            false,
        )
        .with_allow(allow)
    }))
}

async fn legacy_protected_media_route_dispatch(
    request: &mut Request,
    env: &Env,
    path: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let method = request.method().to_string();
    if let Some(profile) = frame_application::legacy_protected_media_route_profile(&method, path) {
        return legacy_protected_media_web_runtime::route_response(
            profile.operation_id,
            request,
            env,
            current_time_ms()?,
        )
        .await;
    }

    let get = frame_application::legacy_protected_media_route_profile("GET", path).is_some();
    let head = frame_application::legacy_protected_media_route_profile("HEAD", path).is_some();
    let post = frame_application::legacy_protected_media_route_profile("POST", path).is_some();
    let failure = match (get, head, post) {
        (true, true, false) => method_guard(request, &[Method::Get, Method::Head], "GET, HEAD")?,
        (true, false, false) => method_guard(request, &[Method::Get], "GET")?,
        (false, false, true) => method_guard(request, &[Method::Post], "POST")?,
        _ => None,
    }
    .ok_or_else(|| Error::RustError("protected media route registry is invalid".into()))?;
    failure_response(failure, request_id, production)
}

async fn legacy_protected_billing_auth_route_dispatch(
    request: &mut Request,
    env: &Env,
    path: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let method = request.method().to_string();
    if let Some(operation_id) = legacy_protected_billing_auth_route_operation(&method, path) {
        return legacy_protected_billing_auth_web_runtime::route_response(
            operation_id,
            request,
            env,
            current_time_ms()?,
        )
        .await;
    }

    let (accepted, allow) =
        legacy_protected_billing_auth_allowed_methods(path).ok_or_else(|| {
            Error::RustError("protected billing/auth route registry is invalid".into())
        })?;
    let failure = method_guard(request, accepted, allow)?.ok_or_else(|| {
        Error::RustError("protected billing/auth route registry is invalid".into())
    })?;
    let mut response = failure_response(failure, request_id, production)?;
    if path == "/api/developer/credits/checkout" {
        legacy_protected_billing_auth_web_runtime::add_developer_checkout_cors(
            &mut response,
            request,
            env,
        )?;
    }
    Ok(response)
}

fn legacy_protected_billing_auth_allowed_methods(
    path: &str,
) -> Option<(&'static [Method], &'static str)> {
    const NEXTAUTH: &[Method] = &[Method::Get, Method::Post];
    const DEVELOPER_CHECKOUT: &[Method] = &[Method::Post, Method::Options];
    const GET_ONLY: &[Method] = &[Method::Get];
    const POST_ONLY: &[Method] = &[Method::Post];

    if path.starts_with("/api/auth/") {
        Some((NEXTAUTH, "GET, POST"))
    } else if path == "/api/developer/credits/checkout" {
        Some((DEVELOPER_CHECKOUT, "POST, OPTIONS"))
    } else if path == "/api/settings/billing/usage" {
        Some((GET_ONLY, "GET"))
    } else if matches!(
        path,
        "/api/desktop/subscribe"
            | "/api/settings/billing/guest-checkout"
            | "/api/settings/billing/manage"
            | "/api/settings/billing/subscribe"
            | "/api/webhooks/stripe"
            | "/api/commercial/checkout"
    ) {
        Some((POST_ONLY, "POST"))
    } else {
        None
    }
}

fn legacy_protected_billing_auth_route_operation(method: &str, path: &str) -> Option<&'static str> {
    if path.starts_with("/api/auth/") {
        match method {
            "GET" => return Some("cap-v1-46bda1c18ffba076"),
            "POST" => return Some("cap-v1-82a39c991fae1050"),
            _ => return None,
        }
    }
    match (method, path) {
        ("POST", "/api/desktop/subscribe") => Some("cap-v1-78537fb518df75ec"),
        ("OPTIONS", "/api/developer/credits/checkout") => Some("cap-v1-572763e7b4977abd"),
        ("POST", "/api/developer/credits/checkout") => Some("cap-v1-60b06cc5ab45f187"),
        ("POST", "/api/settings/billing/guest-checkout") => Some("cap-v1-af61fa5c8fc453cf"),
        ("POST", "/api/settings/billing/manage") => Some("cap-v1-e596f65c43ee2a82"),
        ("POST", "/api/settings/billing/subscribe") => Some("cap-v1-96230bf1f2da3d00"),
        ("GET", "/api/settings/billing/usage") => Some("cap-v1-856dfea22b9d979c"),
        ("POST", "/api/webhooks/stripe") => Some("cap-v1-1e5f228815a2a8b7"),
        ("POST", "/api/commercial/checkout") => Some("cap-v1-b2d19e91b05834cf"),
        _ => None,
    }
}

fn legacy_web_compatibility_action_response(
    effect: legacy_web_action_runtime::WebCompatibilityActionEffectV1,
) -> Result<Response> {
    let metadata = legacy_web_compatibility_action_response_metadata(effect);
    let mut response = Response::empty()?.with_status(metadata.status);
    response
        .headers_mut()
        .set("cache-control", metadata.cache_control)?;
    if let Some(set_cookie) = metadata.set_cookie {
        response.headers_mut().set("set-cookie", &set_cookie)?;
    }
    Ok(response)
}

fn legacy_folder_assignment_action_response(
    effect: legacy_folder_web_runtime::WebFolderAssignmentActionEffectV1,
) -> Result<Response> {
    let mut response = match effect {
        legacy_folder_web_runtime::WebFolderAssignmentActionEffectV1::Added {
            added_count,
            message,
        } => Response::from_json(&LegacyFolderAddedResponseV1 {
            success: true,
            message,
            added_count,
        })?,
        legacy_folder_web_runtime::WebFolderAssignmentActionEffectV1::Removed {
            removed_count,
            message,
        } => Response::from_json(&LegacyFolderRemovedResponseV1 {
            success: true,
            message,
            removed_count,
        })?,
        legacy_folder_web_runtime::WebFolderAssignmentActionEffectV1::MoveVoid => {
            Response::empty()?.with_status(204)
        }
    };
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_library_placement_action_response(
    effect: legacy_library_web_runtime::WebLibraryPlacementActionEffectV1,
) -> Result<Response> {
    let mut response = match effect {
        legacy_library_web_runtime::WebLibraryPlacementActionEffectV1::OrganizationAdded {
            message,
        }
        | legacy_library_web_runtime::WebLibraryPlacementActionEffectV1::OrganizationRemoved {
            message,
        }
        | legacy_library_web_runtime::WebLibraryPlacementActionEffectV1::ScopeAdded { message } => {
            Response::from_json(&LegacyLibraryPlacementMessageResponseV1 {
                success: true,
                message,
            })?
        }
        legacy_library_web_runtime::WebLibraryPlacementActionEffectV1::ScopeRemoved {
            message,
            deleted_count,
        } => Response::from_json(&LegacyLibraryPlacementRemovedResponseV1 {
            success: true,
            message,
            deleted_count,
        })?,
    };
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_notification_action_response(
    _effect: legacy_notification_web_runtime::WebNotificationActionVoidV1,
) -> Result<Response> {
    let mut response = Response::empty()?.with_status(204);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_user_account_action_response(
    _effect: legacy_user_account_web_runtime::WebUserAccountActionVoidV1,
) -> Result<Response> {
    let mut response = Response::empty()?.with_status(204);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_developer_action_response(
    effect: legacy_developer_web_runtime::WebDeveloperActionEffectV1,
) -> Result<Response> {
    let mut response = if let Some((app_id, public_key, secret_key)) = effect.app_created() {
        Response::from_json(&LegacyDeveloperAppCreatedResponseV1 {
            app_id,
            public_key,
            secret_key,
        })?
    } else if let Some((public_key, secret_key)) = effect.regenerated_keys() {
        Response::from_json(&LegacyDeveloperKeysResponseV1 {
            public_key,
            secret_key,
        })?
    } else if effect.is_success_object() {
        Response::from_json(&LegacyDeveloperSuccessResponseV1 { success: true })?
    } else {
        return Err(Error::RustError(
            "legacy developer response projection is invalid".into(),
        ));
    };
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_membership_action_response(
    effect: legacy_membership_web_runtime::WebMembershipActionEffectV1,
) -> Result<Response> {
    let mut response = match effect {
        legacy_membership_web_runtime::WebMembershipActionEffectV1::SuccessObject => {
            Response::from_json(&LegacyMembershipSuccessResponseV1 { success: true })?
        }
        legacy_membership_web_runtime::WebMembershipActionEffectV1::SpaceMembersSet { count } => {
            Response::from_json(&LegacyMembershipSetResponseV1 {
                success: true,
                count,
            })?
        }
        legacy_membership_web_runtime::WebMembershipActionEffectV1::SpaceMembersAdded {
            added,
            already_members,
        } => Response::from_json(&LegacyMembershipAddedResponseV1 {
            success: true,
            added,
            already_members,
        })?,
        legacy_membership_web_runtime::WebMembershipActionEffectV1::SpaceMembersRemoved {
            removed_member_ids,
        } => Response::from_json(&LegacyMembershipRemovedResponseV1 {
            success: true,
            removed: removed_member_ids,
        })?,
    };
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_collaboration_action_response(
    effect: legacy_collaboration_web_runtime::WebCollaborationActionEffectV1,
) -> Result<Response> {
    let value = match effect {
        legacy_collaboration_web_runtime::WebCollaborationActionEffectV1::SuccessObject => {
            serde_json::json!({"success": true})
        }
        legacy_collaboration_web_runtime::WebCollaborationActionEffectV1::Comment(comment) => {
            let legacy_collaboration_web_runtime::WebCollaborationCommentEffectV1 {
                id,
                author_id,
                kind,
                content,
                video_id,
                timestamp,
                parent_comment_id,
                created_at,
                updated_at,
                author_name,
                author_image,
                sending,
            } = *comment;
            serde_json::json!({
            "id": id,
            "authorId": author_id,
            "type": kind,
            "content": content,
            "videoId": video_id,
            "timestamp": timestamp,
            "parentCommentId": parent_comment_id,
            "createdAt": created_at,
            "updatedAt": updated_at,
            "authorName": author_name,
            "authorImage": author_image,
            "sending": sending,
            })
        }
    };
    let mut response = Response::from_json(&value)?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_library_id_read_response(
    result: frame_application::LegacyLibraryIdReadResultV1,
) -> Result<Response> {
    let mut response = match result {
        frame_application::LegacyLibraryIdReadResultV1::Success { data } => {
            Response::from_json(&LegacyLibraryIdReadSuccessResponseV1 {
                success: true,
                data,
            })?
        }
        frame_application::LegacyLibraryIdReadResultV1::Failure { error } => {
            Response::from_json(&LegacyLibraryIdReadFailureResponseV1 {
                success: false,
                error,
            })?
        }
    };
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_space_authorization_response(
    result: frame_application::LegacySpaceAuthorizationResultV1,
) -> Result<Response> {
    use frame_application::LegacySpaceAuthorizationResultV1;

    let (mut response, status) = match result {
        LegacySpaceAuthorizationResultV1::GetSpaceAccess { access } => (
            Response::from_json(&access.map(legacy_space_access_wire))?,
            200,
        ),
        LegacySpaceAuthorizationResultV1::RequireSpaceManager { access } => {
            (Response::from_json(&legacy_space_access_wire(access))?, 200)
        }
        LegacySpaceAuthorizationResultV1::Thrown { message } => {
            let status = if message == frame_application::LEGACY_SPACE_NOT_FOUND_MESSAGE {
                404
            } else {
                403
            };
            (
                Response::from_json(&LegacySpaceAuthorizationErrorWireV1 { error: message })?,
                status,
            )
        }
    };
    response = response.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn legacy_space_access_wire(
    access: frame_application::LegacySpaceAccessV1,
) -> LegacySpaceAccessWireV1 {
    LegacySpaceAccessWireV1 {
        space_id: access.space_id,
        organization_id: access.organization_id,
        organization_owner_id: access.organization_owner_id,
        created_by_id: access.created_by_id,
        organization_role: access
            .organization_role
            .map(frame_application::LegacyOrganizationRoleV1::stable_code),
        space_role: access
            .space_role
            .map(frame_application::LegacySpaceRoleV1::stable_code),
        can_manage: access.can_manage,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacySpaceAccessWireV1 {
    space_id: String,
    organization_id: String,
    organization_owner_id: String,
    created_by_id: String,
    organization_role: Option<&'static str>,
    space_role: Option<&'static str>,
    can_manage: bool,
}

#[derive(Serialize)]
struct LegacySpaceAuthorizationErrorWireV1 {
    error: &'static str,
}

fn legacy_library_detail_read_response(
    result: frame_application::LegacyLibraryDetailResultV1,
) -> Result<Response> {
    use frame_application::LegacyLibraryDetailResultV1;

    let mut response = match result {
        LegacyLibraryDetailResultV1::GetUserVideosSuccess { data } => {
            let data = data
                .into_iter()
                .map(|video| {
                    Ok(LegacyUserVideoWireV1 {
                        id: video.id,
                        owner_id: video.owner_id,
                        name: video.name,
                        created_at: legacy_library_detail_iso(video.created_at_ms)?,
                        metadata: video.metadata,
                        is_screenshot: video.is_screenshot,
                        total_comments: video.total_comments,
                        total_reactions: video.total_reactions,
                        owner_name: video.owner_name,
                        folder_name: video.folder_name,
                        folder_color: video.folder_color,
                        has_active_upload: video.has_active_upload,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Response::from_json(&LegacyUserVideosSuccessWireV1 {
                success: true,
                data,
            })?
        }
        LegacyLibraryDetailResultV1::GetUserVideosFailure => {
            Response::from_json(&LegacyUserVideosFailureWireV1 {
                success: false,
                error: "Failed to fetch videos",
            })?
        }
        LegacyLibraryDetailResultV1::SearchDashboardVideos { data } => {
            let data = data
                .into_iter()
                .map(|video| {
                    Ok(LegacyDashboardSearchVideoWireV1 {
                        id: video.id,
                        name: video.name,
                        owner_name: video.owner_name,
                        created_at: legacy_library_detail_iso(video.created_at_ms)?,
                        duration: video.duration_seconds,
                        is_screenshot: video.is_screenshot,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Response::from_json(&data)?
        }
    };
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

#[derive(Serialize)]
struct LegacyUserVideosSuccessWireV1 {
    success: bool,
    data: Vec<LegacyUserVideoWireV1>,
}

#[derive(Serialize)]
struct LegacyUserVideosFailureWireV1 {
    success: bool,
    error: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyUserVideoWireV1 {
    id: String,
    owner_id: String,
    name: String,
    created_at: String,
    metadata: Option<serde_json::Value>,
    is_screenshot: bool,
    total_comments: u64,
    total_reactions: u64,
    owner_name: String,
    folder_name: Option<String>,
    folder_color: Option<String>,
    has_active_upload: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyDashboardSearchVideoWireV1 {
    id: String,
    name: String,
    owner_name: Option<String>,
    created_at: String,
    duration: Option<f64>,
    is_screenshot: bool,
}

fn legacy_library_detail_iso(value: i64) -> Result<String> {
    if !(0..=253_402_300_799_999).contains(&value) {
        return Err(worker::Error::RustError(
            "legacy library detail timestamp is invalid".into(),
        ));
    }
    let seconds = value / 1_000;
    let millis = value % 1_000;
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = legacy_library_detail_civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

fn legacy_library_detail_civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted = days_since_epoch + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[derive(Serialize)]
struct LegacyLibraryIdReadSuccessResponseV1 {
    success: bool,
    data: Vec<String>,
}

#[derive(Serialize)]
struct LegacyLibraryIdReadFailureResponseV1 {
    success: bool,
    error: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyFolderAddedResponseV1 {
    success: bool,
    message: String,
    added_count: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyFolderRemovedResponseV1 {
    success: bool,
    message: String,
    removed_count: u16,
}

#[derive(Serialize)]
struct LegacyLibraryPlacementMessageResponseV1 {
    success: bool,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyLibraryPlacementRemovedResponseV1 {
    success: bool,
    message: String,
    deleted_count: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyDeveloperAppCreatedResponseV1<'a> {
    app_id: &'a str,
    public_key: &'a str,
    secret_key: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyDeveloperKeysResponseV1<'a> {
    public_key: &'a str,
    secret_key: &'a str,
}

#[derive(Serialize)]
struct LegacyDeveloperSuccessResponseV1 {
    success: bool,
}

#[derive(Serialize)]
struct LegacyMembershipSuccessResponseV1 {
    success: bool,
}

#[derive(Serialize)]
struct LegacyMembershipSetResponseV1 {
    success: bool,
    count: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyMembershipAddedResponseV1 {
    success: bool,
    added: Vec<String>,
    already_members: Vec<String>,
}

#[derive(Serialize)]
struct LegacyMembershipRemovedResponseV1 {
    success: bool,
    removed: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct LegacyWebCompatibilityActionResponseMetadataV1 {
    status: u16,
    cache_control: &'static str,
    set_cookie: Option<String>,
}

fn legacy_web_compatibility_action_response_metadata(
    effect: legacy_web_action_runtime::WebCompatibilityActionEffectV1,
) -> LegacyWebCompatibilityActionResponseMetadataV1 {
    let set_cookie = match effect {
        legacy_web_action_runtime::WebCompatibilityActionEffectV1::ActiveOrganizationChanged => {
            None
        }
        legacy_web_action_runtime::WebCompatibilityActionEffectV1::ThemeCookie {
            name,
            value,
            path,
        } => {
            // These values are closed enums/constants produced by the typed
            // action adapter, never request strings. Preserve Next's pinned
            // default response-cookie serialization without adding attributes
            // absent from the source action.
            Some(format!("{name}={value}; Path={path}"))
        }
    };
    LegacyWebCompatibilityActionResponseMetadataV1 {
        status: 204,
        cache_control: "no-store, max-age=0",
        set_cookie,
    }
}

async fn legacy_api_status_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    legacy_static_response(
        request,
        request_id,
        production,
        "GET",
        legacy_compatibility_runtime::LEGACY_STATUS_PATH,
        LegacyCallerV1::Released(ClientReleaseV1 {
            surface: ClientSurfaceV1::Web,
            api_major: 1,
            release: 2,
        }),
        None,
        None,
        env,
        CompatibilityRateLimitBucketV1::ServiceMisc,
        None,
        b"frame:legacy-public-status:v1",
        "invalid_status_request",
    )
    .await
}

async fn legacy_media_server_root_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    legacy_static_response(
        request,
        request_id,
        production,
        "GET",
        legacy_compatibility_runtime::LEGACY_MEDIA_SERVER_ROOT_PATH,
        LegacyCallerV1::InternalWorker,
        None,
        None,
        env,
        CompatibilityRateLimitBucketV1::ServiceMisc,
        None,
        b"frame:legacy-media-server-root:v1",
        "invalid_media_server_root_request",
    )
    .await
}

async fn legacy_mobile_session_config_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    legacy_static_response(
        request,
        request_id,
        production,
        "GET",
        legacy_compatibility_runtime::LEGACY_MOBILE_SESSION_CONFIG_PATH,
        LegacyCallerV1::Released(ClientReleaseV1 {
            surface: ClientSurfaceV1::Mobile,
            api_major: 1,
            release: 2,
        }),
        None,
        None,
        env,
        CompatibilityRateLimitBucketV1::ClientCompatibility,
        Some(env),
        b"frame:legacy-mobile-session-config:v1",
        "invalid_mobile_session_config_request",
    )
    .await
}

async fn legacy_notifications_response(
    request: &Request,
    env: &Env,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
        request,
        env,
        current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(browser_web_runtime::BrowserWebFailure::Unavailable) => {
            return failure_response(
                ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
                request_id,
                production,
            );
        }
        Err(_) => {
            return legacy_notifications_exact_response(
                401,
                legacy_notification_read_runtime::LEGACY_NOTIFICATION_READ_UNAUTHORIZED_BODY
                    .as_bytes()
                    .to_vec(),
                legacy_notification_read_runtime::LEGACY_NOTIFICATION_READ_UNAUTHORIZED_CONTENT_TYPE,
            );
        }
    };
    let database = env.d1("DB")?;
    match compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        &actor_id,
        current_time_ms()?,
    )
    .await?
    {
        frame_application::RateLimitDecisionV1::Allowed => {}
        frame_application::RateLimitDecisionV1::Rejected { .. } => {
            return failure_response(
                ApiFailure::new(
                    429,
                    "rate_limited",
                    "The request rate limit was exceeded.",
                    true,
                )
                .with_retry_after_seconds(compatibility_rate_limit::RETRY_AFTER_SECONDS),
                request_id,
                production,
            );
        }
    }
    match legacy_notification_read_runtime::read_exact_json(&database, &actor_id).await {
        Ok(body) => legacy_notifications_exact_response(
            200,
            body,
            legacy_notification_read_runtime::LEGACY_NOTIFICATION_READ_SUCCESS_CONTENT_TYPE,
        ),
        Err(
            legacy_notification_read_runtime::LegacyNotificationReadErrorV1::InvalidActor
            | legacy_notification_read_runtime::LegacyNotificationReadErrorV1::Unavailable
            | legacy_notification_read_runtime::LegacyNotificationReadErrorV1::Corrupt,
        ) => legacy_notifications_exact_response(
            500,
            legacy_notification_read_runtime::LEGACY_NOTIFICATION_READ_FAILURE_BODY
                .as_bytes()
                .to_vec(),
            legacy_notification_read_runtime::LEGACY_NOTIFICATION_READ_SUCCESS_CONTENT_TYPE,
        ),
    }
}

fn legacy_notifications_exact_response(
    status: u16,
    body: Vec<u8>,
    content_type: &str,
) -> Result<Response> {
    let mut response = Response::from_bytes(body)?.with_status(status);
    response.headers_mut().set("content-type", content_type)?;
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
async fn legacy_desktop_compatibility_route_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
    canonical_origin: &str,
    expected_method: Method,
    allowed_methods: &'static str,
    route: legacy_desktop_compatibility_web_runtime::LegacyDesktopCompatibilityRouteV1<'_>,
) -> Result<Response> {
    let origin = request.headers().get("origin")?;
    if let Some(failure) = method_guard(
        request,
        &[expected_method, Method::Options],
        allowed_methods,
    )? {
        return legacy_org_custom_domain_web_runtime::cors_response(
            failure_response(failure, request_id, production)?,
            origin.as_deref(),
            canonical_origin,
        );
    }
    if request.method() == Method::Options {
        return legacy_org_custom_domain_web_runtime::preflight_response(request, canonical_origin);
    }
    legacy_desktop_compatibility_web_runtime::response(
        request,
        env,
        route,
        current_time_ms()?,
        canonical_origin,
    )
    .await
}

async fn legacy_desktop_org_custom_domain_response(
    request: &Request,
    env: &Env,
    request_id: &str,
    production: bool,
    canonical_origin: &str,
) -> Result<Response> {
    if request.method() == Method::Options {
        return legacy_org_custom_domain_web_runtime::preflight_response(request, canonical_origin);
    }
    let request_origin = request.headers().get("origin")?;
    let actor_id = match legacy_org_custom_domain_web_runtime::authenticate(
        request,
        env,
        current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(
            legacy_org_custom_domain_web_runtime::LegacyDesktopOrgCustomDomainAuthFailureV1::Unauthenticated,
        ) => {
            let response = legacy_desktop_org_custom_domain_exact_response(
                401,
                legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_UNAUTHENTICATED_BODY
                    .as_bytes()
                    .to_vec(),
                legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_UNAUTHENTICATED_CONTENT_TYPE,
            )?;
            return legacy_org_custom_domain_web_runtime::cors_response(
                response,
                request_origin.as_deref(),
                canonical_origin,
            );
        }
        Err(
            legacy_org_custom_domain_web_runtime::LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable,
        ) => {
            let response = failure_response(
                ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
                request_id,
                production,
            )?;
            return legacy_org_custom_domain_web_runtime::cors_response(
                response,
                request_origin.as_deref(),
                canonical_origin,
            );
        }
    };
    let database = env.d1("DB")?;
    if matches!(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::ClientCompatibility,
            &actor_id,
            current_time_ms()?,
        )
        .await?,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    ) {
        let response = failure_response(
            ApiFailure::new(
                429,
                "rate_limited",
                "The request rate limit was exceeded.",
                true,
            )
            .with_retry_after_seconds(compatibility_rate_limit::RETRY_AFTER_SECONDS),
            request_id,
            production,
        )?;
        return legacy_org_custom_domain_web_runtime::cors_response(
            response,
            request_origin.as_deref(),
            canonical_origin,
        );
    }
    let authority =
        legacy_org_custom_domain_runtime::D1LegacyOrganizationCustomDomainAuthorityV1::new(
            &database,
        );
    let (status, body, content_type) = match legacy_org_custom_domain_runtime::LegacyOrganizationCustomDomainAuthorityV1::read_for_actor(
        &authority,
        &actor_id,
    )
    .await
    {
        Ok(value) => match value.exact_json_body() {
            Ok(body) => (
                200,
                body,
                legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_SUCCESS_CONTENT_TYPE,
            ),
            Err(_) => (
                500,
                legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_BODY
                    .as_bytes()
                    .to_vec(),
                legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_CONTENT_TYPE,
            ),
        },
        Err(_) => (
            500,
            legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_BODY
                .as_bytes()
                .to_vec(),
            legacy_org_custom_domain_runtime::LEGACY_ORG_CUSTOM_DOMAIN_FAILURE_CONTENT_TYPE,
        ),
    };
    let response = legacy_desktop_org_custom_domain_exact_response(status, body, content_type)?;
    legacy_org_custom_domain_web_runtime::cors_response(
        response,
        request_origin.as_deref(),
        canonical_origin,
    )
}

fn legacy_desktop_org_custom_domain_exact_response(
    status: u16,
    body: Vec<u8>,
    content_type: &str,
) -> Result<Response> {
    let mut response = Response::from_bytes(body)?.with_status(status);
    response.headers_mut().set("content-type", content_type)?;
    Ok(response)
}

async fn legacy_notification_preferences_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
        request,
        env,
        current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(browser_web_runtime::BrowserWebFailure::Unavailable) => {
            return failure_response(
                ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
                request_id,
                production,
            );
        }
        Err(_) => {
            return legacy_notification_preferences_exact_json_response(
                401,
                legacy_notification_preferences_runtime::LEGACY_NOTIFICATION_PREFERENCES_UNAUTHORIZED_BODY,
            );
        }
    };
    let raw_body = match read_bounded_legacy_body(request, 0).await {
        Ok(body) => body,
        Err(()) => {
            return failure_response(
                invalid_body_failure("invalid_notification_preferences_request"),
                request_id,
                production,
            );
        }
    };
    let raw_query = request.url()?.query().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    for name in [
        "content-length",
        "content-type",
        "idempotency-key",
        "if-match",
        "origin",
    ] {
        if let Some(value) = request.headers().get(name)? {
            headers.push((name.to_owned(), value));
        }
    }
    let compatibility = ClientCompatibilityPolicyV1 {
        api_major: 1,
        current_release: 2,
        previous_release: 1,
        deprecated_after_ms: None,
        retired: false,
    };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        &actor_id,
        current_time_ms()?,
    )
    .await?;
    let transport = legacy_compatibility_runtime::LegacyCompatibilityTransportV1::new_fail_closed(
        &database,
        compatibility,
    )
    .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
    let authenticated = legacy_compatibility_runtime::LegacyAuthenticatedContextV1::principal_only(
        actor_id.clone(),
    )
    .map_err(|_| Error::RustError("legacy session principal is invalid".into()))?;
    let typed_request = match legacy_compatibility_runtime::LegacyHttpTransportRequestV1::new(
        legacy_compatibility_runtime::LegacyHttpTransportRequestPartsV1 {
            method: "GET".into(),
            raw_path: legacy_notification_preferences_runtime::LEGACY_NOTIFICATION_PREFERENCES_PATH
                .into(),
            raw_query,
            raw_body,
            headers,
            caller: LegacyCallerV1::Released(ClientReleaseV1 {
                surface: ClientSurfaceV1::Web,
                api_major: 1,
                release: 2,
            }),
            correlation_id: request_id.into(),
            security: RequestSecurityContextV1 {
                authenticated: true,
                authorized: true,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit,
            },
            scope_digest: ChecksumSha256::digest_bytes(
                format!("frame:legacy-notification-preferences:v1\0{actor_id}").as_bytes(),
            ),
            authenticated: Some(authenticated),
            revision: None,
            fallback_origin: None,
            configured_origin: None,
        },
    ) {
        Ok(request) => request,
        Err(_) => {
            return failure_response(
                invalid_body_failure("invalid_notification_preferences_request"),
                request_id,
                production,
            );
        }
    };
    match transport.dispatch_http_response(typed_request).await {
        Ok(outcome) => {
            let mut response =
                Response::from_bytes(outcome.body().to_vec())?.with_status(outcome.status());
            if let Some(content_type) = outcome.content_type() {
                response.headers_mut().set("content-type", content_type)?;
            }
            Ok(response)
        }
        Err(error) if error.code == ApiErrorCodeV1::Internal => {
            legacy_notification_preferences_exact_json_response(
                500,
                legacy_notification_preferences_runtime::LEGACY_NOTIFICATION_PREFERENCES_FAILURE_BODY,
            )
        }
        Err(error) => {
            let failure = match error.code {
                ApiErrorCodeV1::InvalidRequest => {
                    invalid_body_failure("invalid_notification_preferences_request")
                }
                ApiErrorCodeV1::NotFound => not_found_failure(),
                ApiErrorCodeV1::RateLimited => ApiFailure::new(
                    429,
                    "rate_limited",
                    "The request rate limit was exceeded.",
                    true,
                )
                .with_retry_after_seconds(compatibility_rate_limit::RETRY_AFTER_SECONDS),
                ApiErrorCodeV1::UpgradeRequired => ApiFailure::new(
                    426,
                    "upgrade_required",
                    "A supported client version is required.",
                    false,
                ),
                ApiErrorCodeV1::Unauthenticated
                | ApiErrorCodeV1::Conflict
                | ApiErrorCodeV1::Unsupported
                | ApiErrorCodeV1::TemporarilyUnavailable
                | ApiErrorCodeV1::Indeterminate
                | ApiErrorCodeV1::Internal => ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
            };
            failure_response(failure, request_id, production)
        }
    }
}

fn legacy_notification_preferences_exact_json_response(
    status: u16,
    body: &'static str,
) -> Result<Response> {
    let mut response = Response::from_bytes(body.as_bytes().to_vec())?.with_status(status);
    response.headers_mut().set(
        "content-type",
        legacy_notification_preferences_runtime::LEGACY_NOTIFICATION_PREFERENCES_CONTENT_TYPE,
    )?;
    Ok(response)
}

async fn legacy_changelog_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
    canonical_origin: &str,
) -> Result<Response> {
    let (method, caller, rate_limit_bucket) = match request.method() {
        Method::Get => (
            "GET",
            LegacyCallerV1::Released(ClientReleaseV1 {
                surface: ClientSurfaceV1::Desktop,
                api_major: 1,
                release: 2,
            }),
            CompatibilityRateLimitBucketV1::ClientCompatibility,
        ),
        Method::Options => (
            "OPTIONS",
            LegacyCallerV1::Released(ClientReleaseV1 {
                surface: ClientSurfaceV1::Web,
                api_major: 1,
                release: 2,
            }),
            CompatibilityRateLimitBucketV1::ServiceMisc,
        ),
        _ => {
            return Err(Error::RustError(
                "legacy changelog method guard was bypassed".into(),
            ));
        }
    };
    legacy_static_response(
        request,
        request_id,
        production,
        method,
        legacy_compatibility_runtime::LEGACY_CHANGELOG_FEED_PATH,
        caller,
        Some(canonical_origin.into()),
        Some(canonical_origin.into()),
        env,
        rate_limit_bucket,
        None,
        b"frame:legacy-changelog-feed:v1",
        "invalid_changelog_request",
    )
    .await
}

async fn legacy_changelog_status_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let (method, caller, rate_limit_bucket) = match request.method() {
        Method::Get => (
            "GET",
            LegacyCallerV1::Released(ClientReleaseV1 {
                surface: ClientSurfaceV1::Desktop,
                api_major: 1,
                release: 2,
            }),
            CompatibilityRateLimitBucketV1::ClientCompatibility,
        ),
        Method::Options => (
            "OPTIONS",
            LegacyCallerV1::Released(ClientReleaseV1 {
                surface: ClientSurfaceV1::Web,
                api_major: 1,
                release: 2,
            }),
            CompatibilityRateLimitBucketV1::ServiceMisc,
        ),
        _ => {
            return Err(Error::RustError(
                "legacy changelog method guard was bypassed".into(),
            ));
        }
    };
    legacy_static_response(
        request,
        request_id,
        production,
        method,
        legacy_compatibility_runtime::LEGACY_CHANGELOG_STATUS_PATH,
        caller,
        None,
        None,
        env,
        rate_limit_bucket,
        None,
        b"frame:legacy-changelog-status:v1",
        "invalid_changelog_status_request",
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn legacy_static_response(
    request: &mut Request,
    request_id: &str,
    production: bool,
    method: &'static str,
    raw_path: &'static str,
    caller: LegacyCallerV1,
    fallback_origin: Option<String>,
    configured_origin: Option<String>,
    rate_limit_env: &Env,
    rate_limit_bucket: CompatibilityRateLimitBucketV1,
    worker_env: Option<&Env>,
    scope: &'static [u8],
    invalid_request_reason: &'static str,
) -> Result<Response> {
    let rate_limit = compatibility_rate_limit::admit_edge_request(
        rate_limit_env,
        request,
        rate_limit_bucket,
        current_time_ms()?,
    )
    .await?;
    let raw_body = match read_bounded_legacy_body(request, 0).await {
        Ok(body) => body,
        Err(()) => {
            return failure_response(
                invalid_body_failure(invalid_request_reason),
                request_id,
                production,
            );
        }
    };
    let raw_query = request.url()?.query().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    for name in [
        "content-length",
        "content-type",
        "idempotency-key",
        "if-match",
        "origin",
    ] {
        if let Some(value) = request.headers().get(name)? {
            headers.push((name.to_owned(), value));
        }
    }
    let compatibility = ClientCompatibilityPolicyV1 {
        api_major: 1,
        current_release: 2,
        previous_release: 1,
        deprecated_after_ms: None,
        retired: false,
    };
    let transport = match worker_env {
        Some(env) => legacy_compatibility_runtime::LegacyCompatibilityTransportV1::new_static_from_worker_env(
            compatibility,
            env,
        ),
        None => legacy_compatibility_runtime::LegacyCompatibilityTransportV1::new_static_only(
            compatibility,
        ),
    }
    .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
    let typed_request = match legacy_compatibility_runtime::LegacyHttpTransportRequestV1::new(
        legacy_compatibility_runtime::LegacyHttpTransportRequestPartsV1 {
            method: method.into(),
            raw_path: raw_path.into(),
            raw_query,
            raw_body,
            headers,
            configured_origin,
            caller,
            correlation_id: request_id.into(),
            security: RequestSecurityContextV1 {
                authenticated: false,
                authorized: true,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit,
            },
            scope_digest: ChecksumSha256::digest_bytes(scope),
            authenticated: None,
            revision: None,
            fallback_origin,
        },
    ) {
        Ok(request) => request,
        Err(_) => {
            return failure_response(
                invalid_body_failure(invalid_request_reason),
                request_id,
                production,
            );
        }
    };
    let outcome = transport.dispatch_http_response(typed_request).await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let failure = match error.code {
                ApiErrorCodeV1::InvalidRequest => invalid_body_failure(invalid_request_reason),
                ApiErrorCodeV1::NotFound => not_found_failure(),
                ApiErrorCodeV1::RateLimited => ApiFailure::new(
                    429,
                    "rate_limited",
                    "The request rate limit was exceeded.",
                    true,
                )
                .with_retry_after_seconds(compatibility_rate_limit::RETRY_AFTER_SECONDS),
                ApiErrorCodeV1::UpgradeRequired => ApiFailure::new(
                    426,
                    "upgrade_required",
                    "A supported client version is required.",
                    false,
                ),
                ApiErrorCodeV1::Unauthenticated
                | ApiErrorCodeV1::Conflict
                | ApiErrorCodeV1::Unsupported
                | ApiErrorCodeV1::TemporarilyUnavailable
                | ApiErrorCodeV1::Indeterminate
                | ApiErrorCodeV1::Internal => ApiFailure::new(
                    503,
                    "service_unavailable",
                    "The service is temporarily unavailable.",
                    true,
                ),
            };
            return failure_response(failure, request_id, production);
        }
    };
    let mut response = if outcome.status() == 204 {
        Response::empty()?.with_status(outcome.status())
    } else {
        Response::from_bytes(outcome.body().to_vec())?.with_status(outcome.status())
    };
    if let Some(content_type) = outcome.content_type() {
        response.headers_mut().set("content-type", content_type)?;
    }
    for (name, value) in outcome.headers() {
        response.headers_mut().set(name, value)?;
    }
    Ok(response)
}

pub(crate) async fn read_bounded_legacy_body(
    request: &mut Request,
    max_bytes: usize,
) -> std::result::Result<Vec<u8>, ()> {
    if request.inner().body().is_none() {
        return Ok(Vec::new());
    }
    let mut stream = request.stream().map_err(|_| ())?;
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        let next_length = body.len().checked_add(chunk.len()).ok_or(())?;
        if next_length > max_bytes {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn health_response(env: &Env, config: &RuntimeConfig) -> Result<Response> {
    let (contract, dependencies) = health_snapshot(env, config).await?;
    Response::from_json(&HealthResponse {
        contract,
        dependencies,
    })
}

async fn public_health_response(env: &Env, config: &RuntimeConfig) -> Result<Response> {
    let (contract, _) = health_snapshot(env, config).await?;
    Response::from_json(&contract)
}

async fn health_snapshot(
    env: &Env,
    config: &RuntimeConfig,
) -> Result<(Health, HealthDependencies)> {
    let database = env.d1("DB")?;
    let ready = database
        .prepare("SELECT 1 AS ready")
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1);
    let _recordings = env.bucket("RECORDINGS")?;
    let media_transformations = match config.media_mode {
        MediaMode::Fake | MediaMode::Native => true,
        MediaMode::Remote => cloudflare_media::binding_available(env),
    };

    let status = if ready && media_transformations {
        ServiceStatus::Ok
    } else {
        ServiceStatus::Degraded
    };
    Ok((
        health_contract(status)?,
        HealthDependencies {
            d1: ready,
            r2: true,
            media_transformations,
        },
    ))
}

fn health_contract(status: ServiceStatus) -> Result<Health> {
    let contract = Health {
        api_version: ApiVersion::current(),
        service: "frame".into(),
        status,
        release: env!("CARGO_PKG_VERSION").into(),
        capabilities: Capabilities::from_names(vec![
            "instant_processing_status".into(),
            "public_share_summary".into(),
            "range_playback".into(),
        ])
        .map_err(|_| Error::RustError("public capabilities are invalid".into()))?,
    };
    contract
        .validate()
        .map_err(|_| Error::RustError("health contract is invalid".into()))?;
    Ok(contract)
}

async fn video_create_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: CreateVideoRequest,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("video_create", &body)
        .map_err(|()| Error::RustError("video command could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "video_create",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }

    let video_id = new_id();
    let response = VideoResponse::new(video_id.clone());
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("video response could not be serialized".into()))?;
    let now = current_time_ms()?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "video_id": video_id,
        "state": "pending",
        "privacy": "private",
    })
    .to_string();
    let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO videos(\
                   id, owner_id, title, state, source_object_key, playback_object_key, duration_ms, \
                   created_at_ms, updated_at_ms, organization_id, privacy, metadata_json, revision\
                 ) SELECT ?1, ?2, ?3, 'pending', NULL, NULL, NULL, ?4, ?4, ?5, \
                          'private', '{}', 0 \
                   FROM organization_members m \
                   JOIN organizations o ON o.id = m.organization_id \
                   WHERE m.organization_id = ?5 AND m.user_id = ?2 \
                     AND m.state = 'active' AND m.role IN ('owner', 'admin', 'member') \
                     AND o.status = 'active' \
                     AND (?6 = -1 OR EXISTS (SELECT 1 FROM authority_state a \
                       WHERE a.singleton = 1 AND a.epoch = ?6 AND a.authority = 'd1' \
                         AND a.phase IN ('d1_authoritative', 'finalized')))",
            )
            .bind(&[
                JsValue::from_str(&video_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&body.title),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) SELECT ?1, ?2, 'video_create', ?3, 201, ?4, ?5, ?6 \
                   WHERE EXISTS (SELECT 1 FROM videos v \
                     WHERE v.id = ?7 AND v.organization_id = ?1 AND v.owner_id = ?8 \
                       AND v.deleted_at_ms IS NULL)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
                JsValue::from_str(&video_id),
                JsValue::from_str(&actor.user_id),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) SELECT ?1, ?2, 'video', ?3, 'video.created', ?4, ?5, \
                          'pending', 0, ?6, ?6, 0, ?8, 1, ?9, 0 FROM videos v \
                   WHERE v.id = ?3 AND v.organization_id = ?2 \
                     AND v.owner_id = ?7 AND v.deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&video_id),
                JsValue::from_str(&format!("video-created:{video_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(outbox_payload_checksum.as_str()),
            ])?,
    ];
    if !atomic_batch_applied(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("video-create:{video_id}"),
            now,
            statements,
        )
        .await?,
    )? {
        if authorized_tenant(&database, request, actor, RequiredAccess::Write)
            .await?
            .as_deref()
            != Some(tenant_id.as_str())
        {
            return failure_response(not_found_failure(), request_id, config.production());
        }
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    }
    json_response(&response, 201, None)
}

async fn video_privacy_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    video_id: &str,
    body: UpdatePrivacyRequest,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("video_privacy", &(video_id, &body))
        .map_err(|()| Error::RustError("privacy command could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "video_privacy",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(existing) =
        load_video_mutation(&database, &tenant_id, video_id, &actor.user_id).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !existing.actor_can_update() {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let expected_revision = i64::try_from(body.expected_revision)
        .map_err(|_| Error::RustError("privacy revision is invalid".into()))?;
    if existing.revision != expected_revision {
        return failure_response(revision_conflict_failure(), request_id, config.production());
    }
    let Some(next_revision) = existing
        .revision
        .checked_add(1)
        .filter(|revision| *revision <= i64::try_from(MAX_SAFE_INTEGER).unwrap_or(i64::MAX))
    else {
        return failure_response(revision_conflict_failure(), request_id, config.production());
    };
    if body.privacy == "public"
        && (existing.state != "ready"
            || !video_has_shareable_media(&database, &tenant_id, video_id).await?)
    {
        return failure_response(
            ApiFailure::new(
                409,
                "video_not_shareable",
                "The video is not ready to be shared.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let now = current_time_ms()?;
    let playback = database
        .prepare(
            "SELECT playback_object_key FROM videos \
             WHERE id = ?1 AND organization_id = ?2 AND playback_object_key IS NOT NULL \
               AND deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(video_id), JsValue::from_str(&tenant_id)])?
        .first::<PlaybackAuthorityRow>(None)
        .await?;
    if let Some(playback) = playback {
        let Some(tenant_contract) = storage_tenant(&tenant_id) else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        let governed = governed_object(
            &database,
            tenant_contract,
            &playback.playback_object_key,
            &actor.user_id,
        )
        .await
        .map_err(|()| Error::RustError("storage authority is unavailable".into()))?;
        let Some(governed) = governed else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        let target_visibility = match body.privacy.as_str() {
            "public" => ObjectVisibility::Public,
            "unlisted" => ObjectVisibility::Unlisted,
            "private" | "organization" => ObjectVisibility::Private,
            _ => return failure_response(not_found_failure(), request_id, config.production()),
        };
        if target_visibility != governed.visibility() {
            let role = if existing.owner_id == actor.user_id || existing.actor_role == "owner" {
                StorageMemberRole::Owner
            } else if existing.actor_role == "admin" {
                StorageMemberRole::Admin
            } else {
                StorageMemberRole::Editor
            };
            let Some(storage_actor) = storage_member_actor(tenant_contract, actor, role) else {
                return failure_response(not_found_failure(), request_id, config.production());
            };
            let governance =
                storage_governance_runtime::governance_service(env, &storage_origin(config))
                    .map_err(|_| {
                        Error::RustError("storage governance configuration is invalid".into())
                    })?;
            let storage_now = storage_timestamp(now)
                .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?;
            let repository =
                storage_governance_runtime::D1StorageGovernanceRepository::with_cutover_fence(
                    &database,
                    authority_fence.scoped.clone(),
                    storage_now,
                    format!("storage-privacy:{video_id}"),
                )
                .map_err(|_| Error::RustError("storage mutation fence is invalid".into()))?;
            let provider = storage_governance_runtime::WorkerStorageGovernanceProvider::new(
                env,
                &database,
                storage_origin(config),
            );
            let next_generation = governed
                .cache_generation()
                .checked_add(1)
                .ok_or_else(|| Error::RustError("cache generation overflowed".into()))?;
            if let Err(error) = governance
                .execute_privacy_change(
                    &repository,
                    &provider,
                    storage_context(tenant_contract, &actor.user_id, CorrelationId::new()),
                    storage_actor,
                    &governed,
                    target_visibility,
                    next_generation,
                    storage_now,
                    60_000,
                    storage_now,
                )
                .await
            {
                return storage_policy_error(error, request_id, config.production());
            }
        }
    }
    let mut updated = existing.clone();
    updated.privacy.clone_from(&body.privacy);
    updated.revision = next_revision;
    let response = updated
        .public_response()
        .ok_or_else(|| Error::RustError("privacy response is invalid".into()))?;
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("privacy response could not be serialized".into()))?;
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "video_id": video_id,
        "privacy": body.privacy,
        "revision": response.revision,
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) SELECT ?1, ?2, 'video_privacy', ?3, 200, ?4, ?5, ?6 \
                   FROM videos v \
                   JOIN organizations o ON o.id = v.organization_id AND o.status = 'active' \
                   JOIN organization_members m ON m.organization_id = v.organization_id \
                     AND m.user_id = ?8 AND m.state = 'active' \
                   WHERE v.id = ?7 AND v.organization_id = ?1 \
                     AND v.deleted_at_ms IS NULL AND v.revision = ?9 \
                     AND (m.role IN ('owner', 'admin') OR (m.role = 'member' AND (\
                       v.owner_id = ?8 OR EXISTS (SELECT 1 FROM space_videos sv \
                         JOIN spaces s ON s.id = sv.space_id \
                           AND s.organization_id = v.organization_id AND s.deleted_at_ms IS NULL \
                         JOIN space_members sm ON sm.space_id = s.id \
                         WHERE sv.video_id = v.id AND sm.user_id = ?8 AND sm.role = 'manager')))) \
                     AND (?10 = 'private' OR (v.state = 'ready' AND EXISTS (\
                       SELECT 1 FROM object_manifests om \
                       WHERE om.object_key = v.playback_object_key AND om.video_id = v.id \
                         AND om.organization_id = v.organization_id AND om.role = 'preview' \
                         AND om.object_version > 0 AND om.state = 'available' \
                         AND om.bytes BETWEEN 1 AND 9007199254740991 \
                         AND om.content_type LIKE 'video/%' \
                         AND length(om.checksum_sha256) = 64 \
                         AND lower(om.checksum_sha256) = om.checksum_sha256 \
                         AND om.checksum_sha256 NOT GLOB '*[^0-9a-f]*' \
                         AND om.provider_etag IS NOT NULL AND om.provider_etag <> '' \
                         AND substr(om.object_key, 1, length('tenants/' || v.organization_id || \
                           '/videos/' || v.id || '/derivatives/')) = \
                           'tenants/' || v.organization_id || '/videos/' || v.id || '/derivatives/' \
                         AND instr(om.object_key, '..') = 0 \
                         AND instr(om.object_key, char(92)) = 0 \
                         AND instr(om.object_key, '?') = 0 \
                         AND instr(om.object_key, '#') = 0 \
                         AND instr(om.object_key, '%') = 0))) \
                     AND (?11 = -1 OR EXISTS (SELECT 1 FROM authority_state a \
                       WHERE a.singleton = 1 AND a.epoch = ?11 AND a.authority = 'd1' \
                         AND a.phase IN ('d1_authoritative', 'finalized')))",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
                JsValue::from_str(video_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_f64(expected_revision as f64),
                JsValue::from_str(&body.privacy),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET privacy = ?3, updated_at_ms = ?5, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND revision = ?4 \
                   AND deleted_at_ms IS NULL AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?6 \
                       AND c.command_type = 'video_privacy' AND c.request_digest = ?7 \
                       AND c.response_status = 200 AND c.response_json = ?8)",
            )
            .bind(&[
                JsValue::from_str(video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&body.privacy),
                JsValue::from_f64(expected_revision as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) SELECT ?1, ?2, 'video', ?3, 'video.privacy.changed', ?4, ?5, \
                          'pending', 0, ?6, ?6, 0, ?12, 1, ?13, 0 FROM videos v \
                   JOIN command_idempotency c ON c.organization_id = v.organization_id \
                     AND c.idempotency_key = ?7 AND c.command_type = 'video_privacy' \
                     AND c.request_digest = ?8 AND c.response_json = ?11 \
                   WHERE v.id = ?3 AND v.organization_id = ?2 \
                     AND v.revision = ?9 AND v.privacy = ?10 AND v.deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(video_id),
                JsValue::from_str(&format!("video-privacy:{video_id}:{}", response.revision)),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&body.privacy),
                JsValue::from_str(&response_json),
                JsValue::from_str(event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
    ];
    if !atomic_batch_applied(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("video-privacy:{video_id}:{next_revision}"),
            now,
            statements,
        )
        .await?,
    )? {
        let current_fence = mutation_authority_fence(env, config, &tenant_id).await?;
        if current_fence != Some(authority_fence) {
            return failure_response(mutation_disabled_failure(), request_id, config.production());
        }
        let Some(current) =
            load_video_mutation(&database, &tenant_id, video_id, &actor.user_id).await?
        else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        if !current.actor_can_update() {
            return failure_response(not_found_failure(), request_id, config.production());
        }
        if current.revision != expected_revision {
            return failure_response(revision_conflict_failure(), request_id, config.production());
        }
        if body.privacy == "public"
            && (current.state != "ready"
                || !video_has_shareable_media(&database, &tenant_id, video_id).await?)
        {
            return failure_response(
                ApiFailure::new(
                    409,
                    "video_not_shareable",
                    "The video is not ready to be shared.",
                    true,
                ),
                request_id,
                config.production(),
            );
        }
        return Err(Error::RustError(
            "privacy command made no progress despite valid fences".into(),
        ));
    }
    json_response(&response, 200, response.public_share_path.as_deref())
}

async fn upload_intent_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: UploadIntentRequest,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    if body.role != "source" {
        return failure_response(
            invalid_body_failure("unsupported_object_role"),
            request_id,
            config.production(),
        );
    }
    if !supported_source_content_type(&body.content_type) {
        return failure_response(
            invalid_body_failure("unsupported_media_type"),
            request_id,
            config.production(),
        );
    }
    let multipart_part_count = body
        .expected_bytes
        .div_ceil(MULTIPART_PART_BYTES)
        .try_into()
        .ok()
        .filter(|count: &u16| (1..=10_000).contains(count));
    let upload_size_valid = match body.transfer_mode.as_str() {
        "brokered" => body.expected_bytes <= MAX_SINGLE_UPLOAD_BYTES,
        "direct" => body.expected_bytes <= MAX_DIRECT_UPLOAD_BYTES,
        "multipart" => body.expected_bytes <= MULTIPART_MAX_BYTES && multipart_part_count.is_some(),
        _ => false,
    };
    if !upload_size_valid {
        return failure_response(
            ApiFailure::new(
                413,
                "multipart_required",
                "This upload requires the multipart transport.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("upload_intent", &body)
        .map_err(|()| Error::RustError("upload command could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "upload_intent",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }

    let Some(integration) = active_r2_integration(&database, &tenant_id).await? else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    if (body.transfer_mode == "multipart" && !integration.supports_multipart())
        || (body.transfer_mode != "multipart" && !integration.supports_single_put())
    {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    }

    if !video_is_scoped(&database, &tenant_id, &body.video_id).await? {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let upload_id = new_id();
    let resource_idempotency_key = digest_identifier(
        "upload_resource",
        &format!("{tenant_id}:{idempotency_key}:{upload_id}"),
    )
    .map_err(|()| Error::RustError("upload resource identity is invalid".into()))?;
    let object_key = source_object_key(&tenant_id, &body.video_id, &body.role, body.object_version);
    let now = current_time_ms()?;
    let (response, direct_staging_key, direct_checksum, direct_expires_at_ms) =
        if body.transfer_mode == "direct" {
            let Some(signer) = direct_upload_signer(env) else {
                return failure_response(
                    storage_unavailable_failure(),
                    request_id,
                    config.production(),
                );
            };
            let checksum = body
                .checksum_sha256
                .as_deref()
                .ok_or_else(|| Error::RustError("direct upload checksum is missing".into()))?;
            let staging_key = private_staging_key(&tenant_id, &upload_id, &body.content_type)
                .map_err(|_| Error::RustError("direct staging identity is invalid".into()))?;
            let capability = signer
                .sign_put(
                    &staging_key,
                    &body.content_type,
                    checksum,
                    body.expected_bytes,
                    u64::try_from(now)
                        .map_err(|_| Error::RustError("direct upload clock is invalid".into()))?,
                    DIRECT_UPLOAD_TTL_SECONDS,
                )
                .map_err(|_| Error::RustError("direct upload signing failed closed".into()))?;
            let expires_at_ms = i64::try_from(capability.expires_at_ms)
                .map_err(|_| Error::RustError("direct upload expiry is invalid".into()))?;
            (
                UploadIntentResponse::direct(
                    upload_id.clone(),
                    body.expected_bytes,
                    body.content_type.clone(),
                    capability,
                ),
                Some(staging_key),
                Some(checksum.to_owned()),
                Some(expires_at_ms),
            )
        } else if body.transfer_mode == "multipart" {
            (
                UploadIntentResponse::multipart(
                    upload_id.clone(),
                    body.expected_bytes,
                    body.content_type.clone(),
                    MULTIPART_PART_BYTES,
                    multipart_part_count
                        .ok_or_else(|| Error::RustError("multipart geometry is invalid".into()))?,
                ),
                None,
                None,
                None,
            )
        } else {
            (
                UploadIntentResponse::new(
                    upload_id.clone(),
                    body.expected_bytes,
                    body.content_type.clone(),
                ),
                None,
                None,
                None,
            )
        };
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("upload response could not be serialized".into()))?;
    let command_expires_at_ms =
        direct_expires_at_ms.unwrap_or_else(|| now.saturating_add(COMMAND_TTL_MS));
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "upload_id": upload_id,
        "video_id": body.video_id,
        "role": body.role,
        "object_version": body.object_version,
    })
    .to_string();
    let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();

    let persisted_transfer_mode = if body.transfer_mode == "multipart" {
        "brokered"
    } else {
        body.transfer_mode.as_str()
    };
    let mut statements = vec![
        database
            .prepare(
                "INSERT INTO video_uploads(\
                   id, organization_id, video_id, state, expected_bytes, received_bytes, \
                   idempotency_key, source_object_key, source_version, content_type, \
                   transfer_mode, direct_staging_key, direct_checksum_sha256, direct_expires_at_ms, \
                   created_at_ms, updated_at_ms, revision\
                 ) VALUES (?1, ?2, ?3, 'initiated', ?4, 0, ?5, ?6, ?7, ?8, ?10, ?11, ?12, ?13, ?9, ?9, 0)",
            )
            .bind(&[
                JsValue::from_str(&upload_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&body.video_id),
                JsValue::from_f64(body.expected_bytes as f64),
                JsValue::from_str(&resource_idempotency_key),
                JsValue::from_str(&object_key),
                JsValue::from_f64(f64::from(body.object_version)),
                JsValue::from_str(&body.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(persisted_transfer_mode),
                direct_staging_key
                    .as_deref()
                    .map_or(JsValue::NULL, JsValue::from_str),
                direct_checksum
                    .as_deref()
                    .map_or(JsValue::NULL, JsValue::from_str),
                direct_expires_at_ms
                    .map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64)),
            ])?,
        database
            .prepare(
                "UPDATE videos SET state = 'uploading', updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&body.video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) VALUES (?1, ?2, 'upload_intent', ?3, 201, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(command_expires_at_ms as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) VALUES (?1, ?2, 'video_upload', ?3, 'upload.intent.created', ?4, ?5, \
                           'pending', 0, ?6, ?6, 0, ?7, 1, ?8, 0)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload_id),
                JsValue::from_str(&format!("upload-intent:{upload_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(outbox_payload_checksum.as_str()),
            ])?,
    ];
    if body.transfer_mode == "multipart" {
        let checksum = body
            .checksum_sha256
            .as_deref()
            .ok_or_else(|| Error::RustError("multipart checksum is missing".into()))?;
        let expires_at_ms = now
            .checked_add(MULTIPART_TTL_MS)
            .ok_or_else(|| Error::RustError("multipart expiry overflowed".into()))?;
        statements.push(
            database
                .prepare(
                    "INSERT INTO r2_multipart_intents_v1(\
                     upload_id,integration_id,checksum_sha256,part_size,part_count,expires_at_ms,created_at_ms) \
                     VALUES(?1,?2,?3,?4,?5,?6,?7)",
                )
                .bind(&[
                    JsValue::from_str(&upload_id),
                    JsValue::from_str(&integration.id),
                    JsValue::from_str(checksum),
                    JsValue::from_f64(MULTIPART_PART_BYTES as f64),
                    JsValue::from_f64(f64::from(multipart_part_count.ok_or_else(|| {
                        Error::RustError("multipart geometry is invalid".into())
                    })?)),
                    JsValue::from_f64(expires_at_ms as f64),
                    JsValue::from_f64(now as f64),
                ])?,
        );
    }
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("upload-intent:{upload_id}"),
            now,
            statements,
        )
        .await?,
    )?;
    let location = response
        .upload_path
        .as_deref()
        .or(response.multipart_path.as_deref())
        .unwrap_or(response.status_path.as_str());
    json_response(&response, 201, Some(location))
}

async fn upload_status_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    upload_id: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Read).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(upload) = load_upload(&database, &tenant_id, upload_id).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let status = upload
        .public_status()
        .ok_or_else(|| Error::RustError("upload state is invalid".into()))?;
    json_response(&status, 200, None)
}

#[derive(Debug, Deserialize)]
struct MultipartIntentRow {
    integration_id: String,
    checksum_sha256: String,
    part_size: i64,
    part_count: i64,
    expires_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct MultipartSessionResponse {
    schema_version: u16,
    upload_id: String,
    state: &'static str,
    expires_at_ms: u64,
    part_size: u64,
    part_count: u16,
    parts_path: String,
    complete_path: String,
}

#[derive(Debug, Serialize)]
struct MultipartPartResponse {
    schema_version: u16,
    upload_id: String,
    part_number: u16,
    bytes: u64,
    checksum_sha256: String,
}

#[derive(Debug, Serialize)]
struct MultipartPartsResponse {
    schema_version: u16,
    upload_id: String,
    parts: Vec<MultipartPartResponse>,
}

#[derive(Debug, Serialize)]
struct MultipartCompleteResponse {
    schema_version: u16,
    upload_id: String,
    state: &'static str,
    checksum_sha256: String,
    object_version: String,
    instant_finalize_path_template: String,
}

async fn load_multipart_intent(
    database: &D1Database,
    upload_id: &str,
) -> Result<Option<MultipartIntentRow>> {
    database
        .prepare(
            "SELECT integration_id,checksum_sha256,part_size,part_count,expires_at_ms \
             FROM r2_multipart_intents_v1 WHERE upload_id=?1 LIMIT 1",
        )
        .bind(&[JsValue::from_str(upload_id)])?
        .first::<MultipartIntentRow>(None)
        .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthenticatedAbortClaim {
    Attempt(i64),
    AlreadyAborted,
    AlreadyCompleted,
}

fn multipart_abort_change_assertion(
    database: &D1Database,
    operation_id: &str,
    upload_id: &str,
    assertion_kind: &str,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "INSERT INTO r2_multipart_abort_batch_assertions_v1(\
             operation_id,upload_id,assertion_kind,expected_count,actual_count) \
             VALUES(?1,?2,?3,1,changes())",
        )
        .bind(&[
            JsValue::from_str(operation_id),
            JsValue::from_str(upload_id),
            JsValue::from_str(assertion_kind),
        ])
}

fn multipart_abort_assertion_cleanup(
    database: &D1Database,
    operation_id: &str,
) -> Result<D1PreparedStatement> {
    database
        .prepare("DELETE FROM r2_multipart_abort_batch_assertions_v1 WHERE operation_id=?1")
        .bind(&[JsValue::from_str(operation_id)])
}

async fn claim_authenticated_multipart_abort(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    tenant_id: &str,
    upload_id: &str,
    now_ms: i64,
) -> Result<AuthenticatedAbortClaim> {
    let state = database
        .prepare(
            "SELECT session.state FROM r2_multipart_sessions_v1 session \
             JOIN video_uploads upload ON upload.id=session.upload_id \
             WHERE session.upload_id=?1 AND upload.organization_id=?2 LIMIT 1",
        )
        .bind(&[JsValue::from_str(upload_id), JsValue::from_str(tenant_id)])?
        .first::<MultipartSessionStateRow>(None)
        .await?;
    match state.as_ref().map(|row| row.state.as_str()) {
        Some("aborted" | "expired") => return Ok(AuthenticatedAbortClaim::AlreadyAborted),
        Some("completing" | "complete") => {
            return Ok(AuthenticatedAbortClaim::AlreadyCompleted);
        }
        Some("open") => {}
        _ => {
            return Err(Error::RustError(
                "multipart abort session is unavailable".into(),
            ));
        }
    }

    let operation_id = Uuid::now_v7().to_string();
    let lock_until = abort_attempt_lock_until(now_ms);
    let statements = vec![
        database
            .prepare(
                "INSERT INTO r2_multipart_abort_reconciliation_v1(\
                 upload_id,intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class,\
                 started_at_ms,updated_at_ms,terminal_at_ms) \
                 SELECT ?1,'authenticated_delete','pending',1,?3,NULL,?2,?2,NULL \
                 WHERE EXISTS (SELECT 1 FROM r2_multipart_sessions_v1 session \
                   JOIN video_uploads upload ON upload.id=session.upload_id \
                   WHERE session.upload_id=?1 AND upload.organization_id=?4 \
                     AND session.state = 'open' \
                     AND upload.state IN ('initiated','uploading','finalizing','failed')) \
                 ON CONFLICT(upload_id) DO UPDATE SET \
                   intent_kind='authenticated_delete',attempt_count=attempt_count+1,\
                   next_attempt_at_ms=?3,last_failure_class=NULL,updated_at_ms=?2 \
                 WHERE state='pending' AND (intent_kind='expiry_cleanup' OR next_attempt_at_ms<=?2)",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_f64(now_ms as f64),
                JsValue::from_f64(lock_until as f64),
                JsValue::from_str(tenant_id),
            ])?,
        multipart_abort_change_assertion(database, &operation_id, upload_id, "attempt_claim")?,
        multipart_abort_assertion_cleanup(database, &operation_id)?,
    ];
    require_batch_success(
        execute_mutation_batch(database, authority_fence, &operation_id, now_ms, statements)
            .await?,
    )?;
    let claimed = database
        .prepare(
            "SELECT intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class \
             FROM r2_multipart_abort_reconciliation_v1 WHERE upload_id=?1 LIMIT 1",
        )
        .bind(&[JsValue::from_str(upload_id)])?
        .first::<MultipartAbortAttemptRow>(None)
        .await?;
    let Some(claimed) = claimed.filter(|row| {
        row.intent_kind == "authenticated_delete"
            && row.state == "pending"
            && row.next_attempt_at_ms == lock_until
            && row.last_failure_class.is_none()
    }) else {
        return Err(Error::RustError(
            "multipart abort claim was not retained".into(),
        ));
    };
    Ok(AuthenticatedAbortClaim::Attempt(claimed.attempt_count))
}

async fn retain_authenticated_multipart_abort_failure(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    upload_id: &str,
    attempt: i64,
    failure: StorageFailureKind,
    now_ms: i64,
) -> Result<()> {
    let operation_id = Uuid::now_v7().to_string();
    let next_attempt = abort_retry_at(now_ms, attempt);
    let statements = vec![
        database
            .prepare(
                "UPDATE r2_multipart_abort_reconciliation_v1 SET next_attempt_at_ms=?3,\
                 last_failure_class=?4,updated_at_ms=?5 \
                 WHERE upload_id=?1 AND intent_kind='authenticated_delete' \
                   AND state='pending' AND attempt_count=?2",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_f64(attempt as f64),
                JsValue::from_f64(next_attempt as f64),
                JsValue::from_str(abort_failure_class(failure)),
                JsValue::from_f64(now_ms as f64),
            ])?,
        multipart_abort_change_assertion(database, &operation_id, upload_id, "failure_retained")?,
        multipart_abort_assertion_cleanup(database, &operation_id)?,
    ];
    require_batch_success(
        execute_mutation_batch(database, authority_fence, &operation_id, now_ms, statements)
            .await?,
    )
}

async fn finish_authenticated_multipart_abort(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    tenant_id: &str,
    upload_id: &str,
    attempt: i64,
    outcome: AuthenticatedAbortOutcomeV1,
    now_ms: i64,
) -> Result<()> {
    let (session_state, reconciliation_state) = match outcome {
        AuthenticatedAbortOutcomeV1::Confirmed { .. } => ("aborted", "confirmed"),
        AuthenticatedAbortOutcomeV1::PreservedObject { .. } => ("completing", "preserved_object"),
        _ => {
            return Err(Error::RustError(
                "multipart abort outcome is not terminal".into(),
            ));
        }
    };
    let operation_id = Uuid::now_v7().to_string();
    let fingerprint = digest_identifier(
        "multipart_upload_event",
        &format!("{upload_id}:{session_state}"),
    )
    .map_err(|()| Error::RustError("multipart abort event is invalid".into()))?;
    let mut statements = Vec::with_capacity(9);
    statements.push(
        database
            .prepare(
                "UPDATE r2_multipart_sessions_v1 SET state=?2 \
                 WHERE upload_id=?1 AND state IN ('open','completing')",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_str(session_state),
            ])?,
    );
    statements.push(multipart_abort_change_assertion(
        database,
        &operation_id,
        upload_id,
        "session_transition",
    )?);
    if reconciliation_state == "confirmed" {
        statements.push(
            database
                .prepare(
                    "UPDATE video_uploads SET state='aborted',updated_at_ms=?3,\
                     revision=revision+1,event_sequence=event_sequence+1,event_fingerprint=?4 \
                     WHERE id=?1 AND organization_id=?2 \
                       AND state IN ('initiated','uploading','finalizing','failed')",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_str(tenant_id),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str(&fingerprint),
                ])?,
        );
        statements.push(multipart_abort_change_assertion(
            database,
            &operation_id,
            upload_id,
            "video_upload_transition",
        )?);
    }
    statements.push(
        database
            .prepare(
                "UPDATE r2_multipart_abort_reconciliation_v1 SET state=?3,\
                 next_attempt_at_ms=?4,last_failure_class=NULL,updated_at_ms=?4,terminal_at_ms=?4 \
                 WHERE upload_id=?1 AND intent_kind='authenticated_delete' \
                   AND state='pending' AND attempt_count=?2",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_f64(attempt as f64),
                JsValue::from_str(reconciliation_state),
                JsValue::from_f64(now_ms as f64),
            ])?,
    );
    statements.push(multipart_abort_change_assertion(
        database,
        &operation_id,
        upload_id,
        "reconciliation_transition",
    )?);
    statements.push(
        database
            .prepare(
                "INSERT INTO r2_multipart_abort_terminal_assertions_v1(\
                 upload_id,outcome,asserted_at_ms) VALUES(?1,?2,?3)",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_str(reconciliation_state),
                JsValue::from_f64(now_ms as f64),
            ])?,
    );
    statements.push(multipart_abort_change_assertion(
        database,
        &operation_id,
        upload_id,
        "terminal_assertion",
    )?);
    statements.push(multipart_abort_assertion_cleanup(database, &operation_id)?);
    require_batch_success(
        execute_mutation_batch(database, authority_fence, &operation_id, now_ms, statements)
            .await?,
    )
}

fn multipart_values(
    upload: &UploadRow,
    intent: &MultipartIntentRow,
) -> Result<(
    StorageRequestContext,
    MultipartUploadId,
    ScopedObjectKey,
    MultipartUploadSpecV1,
)> {
    let tenant = TenantId::parse(&upload.organization_id)
        .map_err(|_| Error::RustError("multipart tenant is invalid".into()))?;
    let correlation = CorrelationId::parse(&upload.id)
        .map_err(|_| Error::RustError("multipart correlation is invalid".into()))?;
    let upload_id = MultipartUploadId::parse(&upload.id)
        .map_err(|_| Error::RustError("multipart upload is invalid".into()))?;
    let key = ScopedObjectKey::parse(&upload.source_object_key)
        .map_err(|_| Error::RustError("multipart object key is invalid".into()))?;
    let total = ByteSize::new(
        u64::try_from(upload.expected_bytes)
            .map_err(|_| Error::RustError("multipart size is invalid".into()))?,
    )
    .map_err(|_| Error::RustError("multipart size is invalid".into()))?;
    let part_size = ByteSize::new(
        u64::try_from(intent.part_size)
            .map_err(|_| Error::RustError("multipart part size is invalid".into()))?,
    )
    .map_err(|_| Error::RustError("multipart part size is invalid".into()))?;
    let limits = MultipartLimitsV1::new(
        ByteSize::new(5 * 1_024 * 1_024)
            .map_err(|_| Error::RustError("multipart limits are invalid".into()))?,
        ByteSize::new(100 * 1_024 * 1_024)
            .map_err(|_| Error::RustError("multipart limits are invalid".into()))?,
        10_000,
        ByteSize::new(MULTIPART_MAX_BYTES)
            .map_err(|_| Error::RustError("multipart limits are invalid".into()))?,
        ByteSize::new(100 * 1_024 * 1_024)
            .map_err(|_| Error::RustError("multipart limits are invalid".into()))?,
        DurationMillis::new(MULTIPART_TTL_MS as u64)
            .map_err(|_| Error::RustError("multipart limits are invalid".into()))?,
    )
    .map_err(|_| Error::RustError("multipart limits are invalid".into()))?;
    let spec = MultipartUploadSpecV1::new(
        key.clone(),
        total,
        part_size,
        ChecksumSha256::parse(&intent.checksum_sha256)
            .map_err(|_| Error::RustError("multipart checksum is invalid".into()))?,
        ContentType::parse(&upload.content_type)
            .map_err(|_| Error::RustError("multipart content type is invalid".into()))?,
        limits,
    )
    .map_err(|_| Error::RustError("multipart specification is invalid".into()))?;
    if i64::from(spec.part_count()) != intent.part_count {
        return Err(Error::RustError("multipart part count is invalid".into()));
    }
    Ok((
        StorageRequestContext::new(tenant, correlation),
        upload_id,
        key,
        spec,
    ))
}

async fn multipart_scope(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    upload_id: &str,
    request_id: &str,
) -> Result<std::result::Result<(String, UploadRow, MultipartIntentRow), Response>> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return Ok(Err(failure_response(
            not_found_failure(),
            request_id,
            config.production(),
        )?));
    };
    let Some(upload) = load_upload(&database, &tenant_id, upload_id).await? else {
        return Ok(Err(failure_response(
            not_found_failure(),
            request_id,
            config.production(),
        )?));
    };
    let Some(intent) = load_multipart_intent(&database, upload_id).await? else {
        return Ok(Err(failure_response(
            not_found_failure(),
            request_id,
            config.production(),
        )?));
    };
    let Some(integration) = r2_integration(&database, &tenant_id, &intent.integration_id).await?
    else {
        return Ok(Err(failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        )?));
    };
    if !integration.supports_multipart() {
        return Ok(Err(failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        )?));
    }
    Ok(Ok((tenant_id, upload, intent)))
}

async fn multipart_session_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    upload_id_text: &str,
    request_id: &str,
) -> Result<Response> {
    let (tenant_id, upload, intent) =
        match multipart_scope(env, config, request, actor, upload_id_text, request_id).await? {
            Ok(scope) => scope,
            Err(response) => return Ok(response),
        };
    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    let probe = D1TrustedMediaProbeV1::new(&database);
    let store = R2MultipartObjectStoreV1::new(&bucket, &database, &probe)
        .map_err(|_| Error::RustError("multipart adapter is invalid".into()))?;
    let (context, upload_id, key, spec) = multipart_values(&upload, &intent)?;
    match request.method() {
        Method::Post => {
            let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await?
            else {
                return failure_response(
                    mutation_disabled_failure(),
                    request_id,
                    config.production(),
                );
            };
            let session = match store
                .create_multipart(
                    context,
                    ProviderCreateMultipartRequestV1::new(
                        upload_id,
                        spec.clone(),
                        TimestampMillis::new(intent.expires_at_ms)
                            .map_err(|_| Error::RustError("multipart expiry is invalid".into()))?,
                        context.correlation_id(),
                    ),
                )
                .await
            {
                Ok(session) => session,
                Err(error) => {
                    return multipart_error_response(error, request_id, config.production());
                }
            };
            let fingerprint = digest_identifier(
                "multipart_upload_event",
                &format!("{upload_id_text}:uploading:{}", intent.checksum_sha256),
            )
            .map_err(|()| Error::RustError("multipart event is invalid".into()))?;
            require_batch_success(
                execute_mutation_batch(
                    &database,
                    &authority_fence,
                    &format!("multipart-create:{upload_id_text}"),
                    current_time_ms()?,
                    vec![database
                        .prepare(
                            "UPDATE video_uploads SET state='uploading',updated_at_ms=?3,\
                             revision=revision+1,event_sequence=event_sequence+1,event_fingerprint=?4 \
                             WHERE id=?1 AND organization_id=?2 AND state='initiated'",
                        )
                        .bind(&[
                            JsValue::from_str(upload_id_text),
                            JsValue::from_str(&tenant_id),
                            JsValue::from_f64(current_time_ms()? as f64),
                            JsValue::from_str(&fingerprint),
                        ])?],
                )
                .await?,
            )?;
            json_response(
                &MultipartSessionResponse {
                    schema_version: API_SCHEMA_VERSION,
                    upload_id: upload_id_text.into(),
                    state: "uploading",
                    expires_at_ms: u64::try_from(session.expires_at().get())
                        .map_err(|_| Error::RustError("multipart expiry is invalid".into()))?,
                    part_size: spec.part_size().get(),
                    part_count: spec.part_count(),
                    parts_path: format!(
                        "/api/v1/uploads/{upload_id_text}/multipart/parts/{{part_number}}"
                    ),
                    complete_path: format!("/api/v1/uploads/{upload_id_text}/multipart/complete"),
                },
                201,
                None,
            )
        }
        Method::Get => {
            let reference = match store.route_reference(context, upload_id, &key).await {
                Ok(reference) => reference,
                Err(error) => {
                    return multipart_error_response(error, request_id, config.production());
                }
            };
            let parts = match store.list_parts(context, reference).await {
                Ok(parts) => parts,
                Err(error) => {
                    return multipart_error_response(error, request_id, config.production());
                }
            };
            json_response(
                &MultipartPartsResponse {
                    schema_version: API_SCHEMA_VERSION,
                    upload_id: upload_id_text.into(),
                    parts: parts
                        .parts()
                        .iter()
                        .map(|part| MultipartPartResponse {
                            schema_version: API_SCHEMA_VERSION,
                            upload_id: upload_id_text.into(),
                            part_number: part.part_number().get(),
                            bytes: part.size().get(),
                            checksum_sha256: part.checksum_sha256().as_str().into(),
                        })
                        .collect(),
                },
                200,
                None,
            )
        }
        Method::Delete => {
            let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await?
            else {
                return failure_response(
                    mutation_disabled_failure(),
                    request_id,
                    config.production(),
                );
            };
            let reference = match store.route_reference(context, upload_id, &key).await {
                Ok(reference) => reference,
                Err(error) => {
                    return multipart_error_response(error, request_id, config.production());
                }
            };
            let now = current_time_ms()?;
            let attempt = match claim_authenticated_multipart_abort(
                &database,
                &authority_fence,
                &tenant_id,
                upload_id_text,
                now,
            )
            .await?
            {
                AuthenticatedAbortClaim::Attempt(attempt) => attempt,
                AuthenticatedAbortClaim::AlreadyAborted => {
                    return Ok(Response::empty()?.with_status(204));
                }
                AuthenticatedAbortClaim::AlreadyCompleted => {
                    return failure_response(
                        ApiFailure::new(
                            409,
                            "multipart_already_completed",
                            "The completed multipart upload can no longer be aborted.",
                            false,
                        ),
                        request_id,
                        config.production(),
                    );
                }
            };
            let provider_outcome = match store
                .reconcile_authenticated_abort_provider(
                    context,
                    reference,
                    attempt,
                    TimestampMillis::new(now)
                        .map_err(|_| Error::RustError("multipart abort clock is invalid".into()))?,
                )
                .await
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    retain_authenticated_multipart_abort_failure(
                        &database,
                        &authority_fence,
                        upload_id_text,
                        attempt,
                        error.kind(),
                        now,
                    )
                    .await?;
                    return multipart_error_response(error, request_id, config.production());
                }
            };
            match provider_outcome {
                AuthenticatedAbortOutcomeV1::Confirmed {
                    attempt: provider_attempt,
                }
                | AuthenticatedAbortOutcomeV1::PreservedObject {
                    attempt: provider_attempt,
                } if provider_attempt == attempt => {
                    finish_authenticated_multipart_abort(
                        &database,
                        &authority_fence,
                        &tenant_id,
                        upload_id_text,
                        attempt,
                        provider_outcome,
                        now,
                    )
                    .await?;
                }
                AuthenticatedAbortOutcomeV1::AlreadyAborted => {
                    return Ok(Response::empty()?.with_status(204));
                }
                AuthenticatedAbortOutcomeV1::AlreadyCompleted => {
                    return failure_response(
                        ApiFailure::new(
                            409,
                            "multipart_already_completed",
                            "The completed multipart upload can no longer be aborted.",
                            false,
                        ),
                        request_id,
                        config.production(),
                    );
                }
                AuthenticatedAbortOutcomeV1::Pending
                | AuthenticatedAbortOutcomeV1::Confirmed { .. }
                | AuthenticatedAbortOutcomeV1::PreservedObject { .. } => {
                    return failure_response(
                        storage_unavailable_failure(),
                        request_id,
                        config.production(),
                    );
                }
            }
            if matches!(
                provider_outcome,
                AuthenticatedAbortOutcomeV1::PreservedObject { .. }
            ) {
                return failure_response(
                    ApiFailure::new(
                        409,
                        "multipart_already_completed",
                        "The completed multipart object was preserved and cannot be aborted.",
                        false,
                    ),
                    request_id,
                    config.production(),
                );
            }
            Ok(Response::empty()?.with_status(204))
        }
        _ => failure_response(
            ApiFailure::new(405, "method_not_allowed", "Method not allowed.", false),
            request_id,
            config.production(),
        ),
    }
}

async fn multipart_part_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &mut Request,
    actor: &AuthenticatedActor,
    upload_id_text: &str,
    part_number_value: u16,
    request_id: &str,
) -> Result<Response> {
    let (_tenant_id, upload, intent) =
        match multipart_scope(env, config, request, actor, upload_id_text, request_id).await? {
            Ok(scope) => scope,
            Err(response) => return Ok(response),
        };
    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    let probe = D1TrustedMediaProbeV1::new(&database);
    let store = R2MultipartObjectStoreV1::new(&bucket, &database, &probe)
        .map_err(|_| Error::RustError("multipart adapter is invalid".into()))?;
    let (context, upload_id, key, spec) = multipart_values(&upload, &intent)?;
    let part_number = MultipartPartNumberV1::new(part_number_value)
        .map_err(|_| Error::RustError("multipart part number is invalid".into()))?;
    let expected = spec
        .expected_part_size(part_number)
        .map_err(|_| Error::RustError("multipart part geometry is invalid".into()))?;
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<u64>().ok());
    if content_length != Some(expected.get())
        || request.headers().get("content-type")?.as_deref() != Some("application/octet-stream")
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return failure_response(
            invalid_body_failure("invalid_multipart_part_headers"),
            request_id,
            config.production(),
        );
    }
    let checksum_text = request
        .headers()
        .get("x-content-sha256")?
        .filter(|value| contracts::valid_sha256(value));
    let Some(checksum_text) = checksum_text else {
        return failure_response(
            invalid_body_failure("invalid_content_checksum"),
            request_id,
            config.production(),
        );
    };
    let bytes = request.bytes().await?;
    if bytes.len() as u64 != expected.get()
        || ChecksumSha256::digest_bytes(&bytes).as_str() != checksum_text
    {
        return failure_response(
            invalid_body_failure("content_checksum_mismatch"),
            request_id,
            config.production(),
        );
    }
    let reference = match store.route_reference(context, upload_id, &key).await {
        Ok(reference) => reference,
        Err(error) => {
            return multipart_error_response(error, request_id, config.production());
        }
    };
    let receipt = match store
        .put_part(
            context,
            ProviderPutPartRequestV1::new(
                reference,
                part_number,
                ChecksumSha256::parse(checksum_text)
                    .map_err(|_| Error::RustError("multipart checksum is invalid".into()))?,
                bytes,
            ),
        )
        .await
    {
        Ok(receipt) => receipt,
        Err(error) => return multipart_error_response(error, request_id, config.production()),
    };
    json_response(
        &MultipartPartResponse {
            schema_version: API_SCHEMA_VERSION,
            upload_id: upload_id_text.into(),
            part_number: receipt.part_number().get(),
            bytes: receipt.size().get(),
            checksum_sha256: receipt.checksum_sha256().as_str().into(),
        },
        200,
        None,
    )
}

async fn multipart_complete_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    upload_id_text: &str,
    request_id: &str,
) -> Result<Response> {
    let (tenant_id, upload, intent) =
        match multipart_scope(env, config, request, actor, upload_id_text, request_id).await? {
            Ok(scope) => scope,
            Err(response) => return Ok(response),
        };
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    let probe = D1TrustedMediaProbeV1::new(&database);
    let store = R2MultipartObjectStoreV1::new(&bucket, &database, &probe)
        .map_err(|_| Error::RustError("multipart adapter is invalid".into()))?;
    let (context, upload_id, key, spec) = multipart_values(&upload, &intent)?;
    let reference = match store.route_reference(context, upload_id, &key).await {
        Ok(reference) => reference,
        Err(error) => {
            return multipart_error_response(error, request_id, config.production());
        }
    };
    let parts = match store.list_parts(context, reference.clone()).await {
        Ok(parts) => parts,
        Err(error) => return multipart_error_response(error, request_id, config.production()),
    };
    if parts.parts().len() != usize::from(spec.part_count()) {
        return failure_response(
            ApiFailure::new(
                409,
                "multipart_parts_incomplete",
                "All verified parts are required before completion.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let completion = store
        .complete_multipart(
            context,
            ProviderCompleteMultipartRequestV1::new(
                reference,
                parts.parts().to_vec(),
                spec.total_size(),
                spec.checksum_sha256().clone(),
                spec.content_type().clone(),
            )
            .map_err(|_| Error::RustError("multipart completion is invalid".into()))?,
        )
        .await;
    let completed = match completion {
        Ok(completed) => completed,
        Err(error) if error.kind() == StorageFailureKind::Unavailable => {
            ensure_multipart_probe_job(env, &database, &authority_fence, &upload, &intent).await?;
            return json_response(
                &serde_json::json!({
                    "schema_version": API_SCHEMA_VERSION,
                    "upload_id": upload_id_text,
                    "state": "probe_pending",
                    "retryable": true,
                }),
                202,
                None,
            );
        }
        Err(error) => return multipart_error_response(error, request_id, config.production()),
    };
    let object_version = instant_r2_object_version(
        completed
            .provider_version()
            .expose_for_provider_comparison(),
    );
    json_response(
        &MultipartCompleteResponse {
            schema_version: API_SCHEMA_VERSION,
            upload_id: upload_id_text.into(),
            state: "provider_complete",
            checksum_sha256: completed.checksum_sha256().as_str().into(),
            object_version,
            instant_finalize_path_template: "/api/v1/instant-recordings/{session_id}/finalize"
                .into(),
        },
        200,
        None,
    )
}

fn multipart_error_response(
    error: frame_ports::StorageFailure,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let failure = match error.kind() {
        StorageFailureKind::NotFound | StorageFailureKind::Unauthorized => not_found_failure(),
        StorageFailureKind::PreconditionFailed | StorageFailureKind::Integrity => ApiFailure::new(
            409,
            "multipart_conflict",
            "The multipart operation conflicts with durable provider state.",
            false,
        ),
        StorageFailureKind::QuotaExceeded => ApiFailure::new(
            413,
            "multipart_limit_exceeded",
            "The multipart operation exceeds a configured limit.",
            false,
        ),
        StorageFailureKind::InvalidRequest | StorageFailureKind::UnsupportedCapability => {
            invalid_body_failure("invalid_multipart_request")
        }
        StorageFailureKind::Throttled
        | StorageFailureKind::Timeout
        | StorageFailureKind::Unavailable => storage_unavailable_failure(),
    };
    failure_response(failure, request_id, production)
}

fn instant_r2_object_version(provider_version: &str) -> String {
    let mut digest = sha2::Sha256::new();
    digest.update(b"frame.instant.r2-object-version.v1\0");
    digest.update((provider_version.len() as u32).to_be_bytes());
    digest.update(provider_version.as_bytes());
    instant_hex(&digest.finalize())
}

fn instant_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

async fn ensure_multipart_probe_job(
    env: &Env,
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    upload: &UploadRow,
    intent: &MultipartIntentRow,
) -> Result<()> {
    let verified = database
        .prepare(
            "SELECT provider_version,provider_etag,bytes,checksum_sha256,content_type \
             FROM r2_multipart_verified_objects_v1 WHERE upload_id=?1 LIMIT 1",
        )
        .bind(&[JsValue::from_str(&upload.id)])?
        .first::<VerifiedMultipartObjectRow>(None)
        .await?
        .ok_or_else(|| Error::RustError("multipart verification receipt is unavailable".into()))?;
    if verified.bytes != upload.expected_bytes
        || verified.checksum_sha256 != intent.checksum_sha256
        || verified.content_type != upload.content_type
    {
        return Err(Error::RustError(
            "multipart verification receipt conflicts with the upload".into(),
        ));
    }
    let bucket = env.bucket("RECORDINGS")?;
    let object = bucket
        .head(&upload.source_object_key)
        .await?
        .ok_or_else(|| Error::RustError("multipart completed object is unavailable".into()))?;
    let expected_bytes = u64::try_from(upload.expected_bytes)
        .map_err(|_| Error::RustError("multipart source size is invalid".into()))?;
    let metadata = object.http_metadata();
    let custom = object.custom_metadata()?;
    if object.size() != expected_bytes
        || object.version() != verified.provider_version
        || object.etag() != verified.provider_etag
        || metadata.content_type.as_deref() != Some(upload.content_type.as_str())
        || metadata.content_encoding.is_some()
        || metadata.cache_control.as_deref() != Some("private, no-store")
        || custom.get("frame-sha256").map(String::as_str) != Some(intent.checksum_sha256.as_str())
        || custom.get("frame-correlation-id").map(String::as_str) != Some(upload.id.as_str())
        || custom.get("frame-cache-policy").map(String::as_str) != Some("no_store")
    {
        return Err(Error::RustError(
            "multipart completed object failed probe preflight".into(),
        ));
    }
    let integration = r2_integration(database, &upload.organization_id, &intent.integration_id)
        .await?
        .filter(IntegrationRow::supports_multipart)
        .ok_or_else(|| Error::RustError("multipart integration is unavailable".into()))?;
    let now = current_time_ms()?;
    let job_id = new_id();
    let profile_digest = request_digest("media_profile_v1", &"probe_v1")
        .map_err(|()| Error::RustError("probe profile identity is invalid".into()))?;
    let output_key = format!(
        "tenants/{}/videos/{}/derivatives/probe_v1/{profile_digest}",
        upload.organization_id, upload.video_id
    );
    let payload_json = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "tenant_id": upload.organization_id,
        "video_id": upload.video_id,
        "source_version": upload.source_version,
        "profile": "probe_v1",
    })
    .to_string();
    let job_idempotency = digest_identifier(
        "multipart_probe_job",
        &format!("{}:{}", upload.id, intent.checksum_sha256),
    )
    .map_err(|()| Error::RustError("probe job identity is invalid".into()))?;
    let storage_object_id = new_id();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO object_manifests(object_key,video_id,role,bytes,checksum_sha256,\
                 content_type,created_at_ms,organization_id,object_version,provider_etag,state,updated_at_ms) \
                 VALUES(?1,?2,'source',?3,?4,?5,?6,?7,?8,?9,'available',?6) \
                 ON CONFLICT(object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_str(&upload.video_id),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&intent.checksum_sha256),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&upload.organization_id),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&object.etag()),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(id,organization_id,integration_id,video_id,object_key,role,\
                 object_version,state,bytes,content_type,checksum_sha256,provider_etag,created_at_ms) \
                 VALUES(?1,?2,?3,?4,?5,'source',?6,'available',?7,?8,?9,?10,?11) \
                 ON CONFLICT(integration_id,object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(&upload.organization_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_str(&intent.checksum_sha256),
                JsValue::from_str(&object.etag()),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_governed_objects_v1(organization_id,object_key,role,visibility,state,\
                 malware_disposition,immutable_revision,cache_generation,checksum_sha256,bytes,content_type,\
                 retention_until_ms,created_at_ms,updated_at_ms) \
                 VALUES(?1,?2,'source','private','active','clean',?3,1,?4,?5,?6,NULL,?7,?7) \
                 ON CONFLICT(organization_id,object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&upload.organization_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&intent.checksum_sha256),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET source_object_key=?3,state='processing',updated_at_ms=?4,revision=revision+1 \
                 WHERE id=?1 AND organization_id=?2 AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&upload.organization_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO media_jobs(id,video_id,kind,state,idempotency_key,attempt,payload_json,\
                 created_at_ms,updated_at_ms,organization_id,selected_executor,source_version,\
                 profile_version,output_object_key,cancel_requested,revision) \
                 SELECT ?1,?2,'probe','queued',?3,0,?4,?5,?5,?6,'native_gstreamer',?7,1,?8,0,0 \
                 WHERE NOT EXISTS(SELECT 1 FROM media_jobs j WHERE j.organization_id=?6 \
                   AND j.video_id=?2 AND j.source_version=?7 \
                   AND json_extract(j.payload_json,'$.profile')='probe_v1' \
                   AND j.state IN ('queued','leased','running','succeeded'))",
            )
            .bind(&[
                JsValue::from_str(&job_id),
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&job_idempotency),
                JsValue::from_str(&payload_json),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&upload.organization_id),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&output_key),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!("multipart-probe:{}", upload.id),
            now,
            statements,
        )
        .await?,
    )?;
    let postcondition = database
        .prepare(
            "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM object_manifests m \
               WHERE m.object_key=?1 AND m.organization_id=?2 AND m.video_id=?3 \
                 AND m.object_version=?4 AND m.bytes=?5 AND m.checksum_sha256=?6 \
                 AND m.content_type=?7 AND m.provider_etag=?8 AND m.state='available') \
             AND EXISTS(SELECT 1 FROM media_jobs j WHERE j.organization_id=?2 \
               AND j.video_id=?3 AND j.source_version=?4 \
               AND json_extract(j.payload_json,'$.profile')='probe_v1' \
               AND j.state IN ('queued','leased','running','succeeded'))",
        )
        .bind(&[
            JsValue::from_str(&upload.source_object_key),
            JsValue::from_str(&upload.organization_id),
            JsValue::from_str(&upload.video_id),
            JsValue::from_f64(upload.source_version as f64),
            JsValue::from_f64(expected_bytes as f64),
            JsValue::from_str(&intent.checksum_sha256),
            JsValue::from_str(&upload.content_type),
            JsValue::from_str(&verified.provider_etag),
        ])?
        .first::<PresenceFlagRow>(None)
        .await?;
    if postcondition.is_some_and(|row| row.present == 1) {
        Ok(())
    } else {
        Err(Error::RustError(
            "multipart probe bootstrap postcondition failed".into(),
        ))
    }
}

async fn instant_finalize_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    session_id: &str,
    body: InstantFinalizeRequestV1,
    request_id: &str,
) -> Result<Response> {
    if body.session_id != session_id || body.validate().is_err() {
        return failure_response(
            invalid_body_failure("invalid_instant_finalize"),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let now = current_time_ms()?;
    let retained = match instant_finalize_runtime::retain_request(
        &database,
        &authority_fence,
        &idempotency_key,
        &body,
        now,
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(instant_finalize_runtime::InstantFinalizeFailure::Conflict) => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        Err(_) => {
            return failure_response(
                storage_unavailable_failure(),
                request_id,
                config.production(),
            );
        }
    };
    if retained.state == InstantFinalizeStateV1::Published {
        return json_response(&retained, 200, None);
    }
    match instant_finalize_runtime::reconcile_session(&database, &authority_fence, session_id, now)
        .await
    {
        Ok(receipt) => json_response(&receipt, 200, None),
        Err(instant_finalize_runtime::InstantFinalizeFailure::Pending) => {
            json_response(&retained, 202, None)
        }
        Err(instant_finalize_runtime::InstantFinalizeFailure::Conflict) => failure_response(
            idempotency_conflict_failure(),
            request_id,
            config.production(),
        ),
        Err(instant_finalize_runtime::InstantFinalizeFailure::Persistence) => failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        ),
    }
}

async fn upload_content_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &mut Request,
    actor: &AuthenticatedActor,
    upload_id: &str,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let Some(upload) = load_upload(&database, &tenant_id, upload_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if upload.organization_id != tenant_id || upload.id != upload_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    if upload.transfer_mode != "brokered" {
        return failure_response(
            ApiFailure::new(
                409,
                "direct_upload_requires_finalize",
                "This direct upload must be finalized through its finalize endpoint.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    if database
        .prepare("SELECT 1 AS ready FROM r2_multipart_intents_v1 WHERE upload_id=?1 LIMIT 1")
        .bind(&[JsValue::from_str(upload_id)])?
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1)
    {
        return failure_response(
            ApiFailure::new(
                409,
                "multipart_endpoint_required",
                "This upload must use its multipart endpoint.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    if !matches!(
        upload.state.as_str(),
        "initiated" | "uploading" | "finalizing" | "complete"
    ) {
        return failure_response(
            ApiFailure::new(
                409,
                "upload_not_writable",
                "The upload is not writable in its current state.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let expected_bytes = u64::try_from(upload.expected_bytes)
        .ok()
        .filter(|bytes| *bytes > 0 && *bytes <= MAX_SINGLE_UPLOAD_BYTES)
        .ok_or_else(|| Error::RustError("upload byte contract is invalid".into()))?;
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<u64>().ok());
    if content_length != Some(expected_bytes) {
        return failure_response(
            invalid_body_failure("content_length_mismatch"),
            request_id,
            config.production(),
        );
    }
    if request.headers().get("content-type")?.as_deref() != Some(&upload.content_type) {
        return failure_response(
            invalid_body_failure("content_type_mismatch"),
            request_id,
            config.production(),
        );
    }
    if request
        .headers()
        .get("content-encoding")?
        .is_some_and(|encoding| encoding != "identity")
    {
        return failure_response(
            invalid_body_failure("unsupported_content_encoding"),
            request_id,
            config.production(),
        );
    }
    let checksum_text = request
        .headers()
        .get("x-content-sha256")?
        .filter(|value| value.bytes().all(|byte| !byte.is_ascii_uppercase()));
    let Some((checksum_text, checksum)) = checksum_text
        .as_deref()
        .and_then(|value| parse_sha256(value).map(|checksum| (value.to_owned(), checksum)))
    else {
        return failure_response(
            invalid_body_failure("invalid_content_checksum"),
            request_id,
            config.production(),
        );
    };
    if upload.state == "complete" {
        if upload.checksum_sha256.as_deref() != Some(checksum_text.as_str()) {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        if completed_upload_matches(env, &upload).await? {
            let status = upload
                .public_status()
                .ok_or_else(|| Error::RustError("upload state is invalid".into()))?;
            return json_response(&status, 200, None);
        }
        return failure_response(
            media_unavailable_failure("upload_reconciliation_required"),
            request_id,
            config.production(),
        );
    }
    let integration = active_r2_integration(&database, &tenant_id).await?;
    let Some(integration) = integration else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    if !integration.supports_single_put() {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let now = current_time_ms()?;
    let Some(tenant_contract) = storage_tenant(&tenant_id) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(storage_actor) =
        storage_member_actor(tenant_contract, actor, StorageMemberRole::Editor)
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let governed_upload = GovernedObject::new(
        tenant_contract,
        GovernedObjectId::parse(upload.source_object_key.clone())
            .map_err(|_| Error::RustError("upload object authority is invalid".into()))?,
        GovernedObjectRole::Source,
        ObjectVisibility::Private,
        GovernedObjectState::Active,
        MalwareDisposition::Clean,
        u64::try_from(upload.source_version)
            .map_err(|_| Error::RustError("upload object version is invalid".into()))?,
        1,
        ChecksumSha256::parse(checksum_text.clone())
            .map_err(|_| Error::RustError("upload checksum authority is invalid".into()))?,
        ByteSize::new(expected_bytes)
            .map_err(|_| Error::RustError("upload size authority is invalid".into()))?,
        None,
    )
    .map_err(|_| Error::RustError("upload object authority is invalid".into()))?;
    if governed_object(
        &database,
        tenant_contract,
        governed_upload.object_id().as_str(),
        &actor.user_id,
    )
    .await
    .map_err(|()| Error::RustError("storage authority is unavailable".into()))?
    .is_some_and(|existing| existing != governed_upload)
    {
        return failure_response(
            idempotency_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let governance =
        storage_governance_runtime::governance_service(env, &storage_origin(config))
            .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    let storage_now = storage_timestamp(now)
        .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?;
    if let Err(error) = governance.authorize(
        CorrelationId::new(),
        StorageAccessRequest {
            actor: storage_actor,
            operation: StorageOperation::WriteImmutable,
            surface: StorageAccessSurface::SameOriginApplication,
            object: &governed_upload,
            now: storage_now,
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        },
    ) {
        return storage_policy_error(error, request_id, config.production());
    }
    let storage_correlation = CorrelationId::new();
    let storage_repository =
        storage_governance_runtime::D1StorageGovernanceRepository::with_cutover_fence(
            &database,
            authority_fence.scoped.clone(),
            storage_now,
            format!("storage-upload:{storage_correlation}"),
        )
        .map_err(|_| Error::RustError("storage mutation fence is invalid".into()))?;
    let storage_request_context =
        storage_context(tenant_contract, &actor.user_id, storage_correlation);
    let storage_reservation = match governance
        .reserve_quota(
            &storage_repository,
            storage_request_context.clone(),
            StorageQuotaPolicy::new(
                ByteSize::new(MAX_SAFE_INTEGER)
                    .map_err(|_| Error::RustError("storage quota is invalid".into()))?,
                MAX_SAFE_INTEGER,
            )
            .map_err(|_| Error::RustError("storage quota is invalid".into()))?,
            ByteSize::new(expected_bytes)
                .map_err(|_| Error::RustError("upload size is invalid".into()))?,
            storage_now,
            storage_timestamp(
                now.checked_add(STORAGE_RESERVATION_TTL_MS)
                    .ok_or_else(|| Error::RustError("storage reservation overflowed".into()))?,
            )
            .ok_or_else(|| Error::RustError("storage reservation expiry is invalid".into()))?,
        )
        .await
    {
        Ok(reservation) => reservation,
        Err(_) => {
            return failure_response(
                ApiFailure::new(
                    409,
                    "storage_quota_exceeded",
                    "The storage quota does not allow this upload.",
                    true,
                ),
                request_id,
                config.production(),
            );
        }
    };
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("upload-finalizing:{upload_id}"),
            now,
            vec![database
                .prepare(
                    "UPDATE video_uploads SET state = 'finalizing', updated_at_ms = ?3, revision = revision + 1 \
                     WHERE id = ?1 AND organization_id = ?2 \
                       AND state IN ('initiated', 'uploading', 'finalizing')",
                )
                .bind(&[
                    JsValue::from_str(upload_id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_f64(now as f64),
                ])?],
        )
        .await?,
    )?;

    let bucket = env.bucket("RECORDINGS")?;
    let existing = bucket.head(&upload.source_object_key).await?;
    let object = if let Some(existing) = existing {
        existing
    } else {
        let stream = FixedLengthStream::wrap(request.stream()?, expected_bytes);
        let metadata = HttpMetadata {
            content_type: Some(upload.content_type.clone()),
            content_disposition: Some("attachment".into()),
            cache_control: Some("private, no-store".into()),
            ..HttpMetadata::default()
        };
        bucket
            .put(&upload.source_object_key, stream)
            .http_metadata(metadata)
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
            .ok_or_else(|| Error::RustError("conditional upload was not applied".into()))?
    };
    let metadata = object.http_metadata();
    if object.size() != expected_bytes
        || object.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || metadata.content_type.as_deref() != Some(upload.content_type.as_str())
        || metadata.content_encoding.is_some()
    {
        return failure_response(
            media_unavailable_failure("upload_checksum_mismatch"),
            request_id,
            config.production(),
        );
    }

    let etag = object.etag();
    let storage_object_id = new_id();
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "upload_id": upload.id,
        "video_id": upload.video_id,
        "source_version": upload.source_version,
        "bytes": expected_bytes,
    })
    .to_string();
    let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "UPDATE video_uploads \
                 SET state = 'complete', received_bytes = expected_bytes, checksum_sha256 = ?3, \
                     updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND state = 'finalizing'",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&checksum_text),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO object_manifests(\
                   object_key, video_id, role, bytes, checksum_sha256, content_type, created_at_ms, \
                   organization_id, object_version, provider_etag, state, updated_at_ms\
                 ) VALUES (?1, ?2, 'source', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'available', ?6) \
                 ON CONFLICT(object_key) DO UPDATE SET \
                   bytes = excluded.bytes, checksum_sha256 = excluded.checksum_sha256, \
                   content_type = excluded.content_type, provider_etag = excluded.provider_etag, \
                   state = 'available', updated_at_ms = excluded.updated_at_ms \
                 WHERE object_manifests.video_id = excluded.video_id \
                   AND object_manifests.organization_id = excluded.organization_id \
                   AND object_manifests.role = excluded.role \
                   AND object_manifests.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_str(&upload.video_id),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&checksum_text),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&etag),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(\
                   id, organization_id, integration_id, video_id, object_key, role, object_version, \
                   state, bytes, content_type, checksum_sha256, provider_etag, created_at_ms\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'source', ?6, 'available', ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT(integration_id, object_key) DO UPDATE SET \
                   state = 'available', bytes = excluded.bytes, content_type = excluded.content_type, \
                   checksum_sha256 = excluded.checksum_sha256, provider_etag = excluded.provider_etag \
                 WHERE storage_objects.organization_id = excluded.organization_id \
                   AND storage_objects.video_id = excluded.video_id \
                   AND storage_objects.role = excluded.role \
                   AND storage_objects.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_str(&checksum_text),
                JsValue::from_str(&etag),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_governed_objects_v1(organization_id, object_key, role, visibility, \
                   state, malware_disposition, immutable_revision, cache_generation, checksum_sha256, \
                   bytes, content_type, retention_until_ms, created_at_ms, updated_at_ms) \
                 VALUES (?1, ?2, 'source', 'private', 'active', 'clean', ?3, 1, ?4, ?5, ?6, NULL, ?7, ?7) \
                 ON CONFLICT(organization_id, object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&checksum_text),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET source_object_key = ?3, state = 'processing', \
                    updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) VALUES (?1, ?2, 'video_upload', ?3, 'upload.completed', ?4, ?5, \
                           'pending', 0, ?6, ?6, 0, ?7, 1, ?8, 0)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(upload_id),
                JsValue::from_str(&format!("upload-complete:{upload_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(outbox_payload_checksum.as_str()),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("upload-complete:{upload_id}"),
            now,
            statements,
        )
        .await?,
    )?;
    match storage_repository
        .release_quota_reservation(
            storage_request_context,
            storage_reservation.reservation_id(),
            true,
            storage_now,
        )
        .await
    {
        Ok(
            frame_ports::StorageCasOutcomeV1::Applied | frame_ports::StorageCasOutcomeV1::Replay,
        ) => {}
        Ok(frame_ports::StorageCasOutcomeV1::Conflict) | Err(_) => {
            return Err(Error::RustError(
                "storage quota commit reconciliation is required".into(),
            ));
        }
    }
    let status = UploadStatusResponse {
        schema_version: API_SCHEMA_VERSION,
        upload_id: upload_id.into(),
        state: "complete".into(),
        expected_bytes,
        received_bytes: expected_bytes,
        content_type: upload.content_type,
    };
    json_response(&status, 200, None)
}

async fn direct_upload_finalize_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    upload_id: &str,
    body: DirectUploadFinalizeRequest,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest("direct_upload_finalize", &body)
        .map_err(|()| Error::RustError("direct finalize command could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "direct_upload_finalize",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(upload) = load_upload(&database, &tenant_id, upload_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if upload.transfer_mode != "direct"
        || upload.direct_checksum_sha256.as_deref() != Some(body.checksum_sha256.as_str())
    {
        return failure_response(
            idempotency_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if upload.state == "complete" {
        if upload.checksum_sha256.as_deref() == Some(body.checksum_sha256.as_str())
            && completed_upload_matches(env, &upload).await?
        {
            let status = upload
                .public_status()
                .ok_or_else(|| Error::RustError("direct upload state is invalid".into()))?;
            return json_response(&status, 200, None);
        }
        return failure_response(
            media_unavailable_failure("upload_reconciliation_required"),
            request_id,
            config.production(),
        );
    }
    if !matches!(
        upload.state.as_str(),
        "initiated" | "uploading" | "finalizing"
    ) {
        return failure_response(
            ApiFailure::new(
                409,
                "upload_not_finalizable",
                "The upload cannot be finalized in its current state.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let now = current_time_ms()?;
    let staging_key = upload
        .direct_staging_key
        .as_deref()
        .ok_or_else(|| Error::RustError("direct staging identity is missing".into()))?;
    let expires_at_ms = upload
        .direct_expires_at_ms
        .ok_or_else(|| Error::RustError("direct staging expiry is missing".into()))?;
    let bucket = env.bucket("RECORDINGS")?;
    if direct_upload_finalize_expired(now, expires_at_ms) {
        let event_fingerprint = digest_identifier(
            "direct_upload_event",
            &format!("{upload_id}:aborted:{expires_at_ms}"),
        )
        .map_err(|()| Error::RustError("direct expiry event is invalid".into()))?;
        require_batch_success(
            execute_mutation_batch(
                &database,
                &authority_fence,
                &format!("direct-upload-expired:{upload_id}"),
                now,
                vec![database
                    .prepare(
                        "UPDATE video_uploads SET state = 'aborted', updated_at_ms = ?3, revision = revision + 1, \
                           event_sequence = event_sequence + 1, event_fingerprint = ?4 \
                         WHERE id = ?1 AND organization_id = ?2 AND transfer_mode = 'direct' \
                           AND state IN ('initiated','uploading','finalizing')",
                    )
                    .bind(&[
                        JsValue::from_str(upload_id),
                        JsValue::from_str(&tenant_id),
                        JsValue::from_f64(now as f64),
                        JsValue::from_str(&event_fingerprint),
                    ])?],
            )
            .await?,
        )?;
        let _ = bucket.delete(staging_key).await;
        return failure_response(
            ApiFailure::new(
                409,
                "direct_upload_expired",
                "The direct upload capability expired before finalization.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let expected_bytes = u64::try_from(upload.expected_bytes)
        .ok()
        .filter(|bytes| *bytes > 0 && *bytes <= MAX_DIRECT_UPLOAD_BYTES)
        .ok_or_else(|| Error::RustError("direct upload byte contract is invalid".into()))?;
    let checksum = parse_sha256(&body.checksum_sha256)
        .ok_or_else(|| Error::RustError("direct upload checksum is invalid".into()))?;
    let Some(staged) = bucket.head(staging_key).await? else {
        return failure_response(
            ApiFailure::new(
                409,
                "direct_upload_missing",
                "The direct upload has not reached private staging storage.",
                true,
            ),
            request_id,
            config.production(),
        );
    };
    let staged_http = staged.http_metadata();
    let staged_custom = staged.custom_metadata()?;
    if staged.size() != expected_bytes
        || staged.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || staged_http.content_type.as_deref() != Some(upload.content_type.as_str())
        || staged_http.content_encoding.is_some()
        || staged_custom.get("frame-sha256").map(String::as_str)
            != Some(body.checksum_sha256.as_str())
    {
        return failure_response(
            ApiFailure::new(
                409,
                "direct_upload_verification_failed",
                "The staged object does not match the signed upload contract.",
                false,
            ),
            request_id,
            config.production(),
        );
    }

    let Some(integration) = active_r2_integration(&database, &tenant_id).await? else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    if !integration.supports_single_put() {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let Some(tenant_contract) = storage_tenant(&tenant_id) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(storage_actor) =
        storage_member_actor(tenant_contract, actor, StorageMemberRole::Editor)
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let governed_upload = GovernedObject::new(
        tenant_contract,
        GovernedObjectId::parse(upload.source_object_key.clone())
            .map_err(|_| Error::RustError("direct object authority is invalid".into()))?,
        GovernedObjectRole::Source,
        ObjectVisibility::Private,
        GovernedObjectState::Active,
        MalwareDisposition::Clean,
        u64::try_from(upload.source_version)
            .map_err(|_| Error::RustError("direct object version is invalid".into()))?,
        1,
        ChecksumSha256::parse(body.checksum_sha256.clone())
            .map_err(|_| Error::RustError("direct checksum authority is invalid".into()))?,
        ByteSize::new(expected_bytes)
            .map_err(|_| Error::RustError("direct size authority is invalid".into()))?,
        None,
    )
    .map_err(|_| Error::RustError("direct object authority is invalid".into()))?;
    if governed_object(
        &database,
        tenant_contract,
        governed_upload.object_id().as_str(),
        &actor.user_id,
    )
    .await
    .map_err(|()| Error::RustError("storage authority is unavailable".into()))?
    .is_some_and(|existing| existing != governed_upload)
    {
        return failure_response(
            idempotency_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let governance =
        storage_governance_runtime::governance_service(env, &storage_origin(config))
            .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    let storage_now = storage_timestamp(now)
        .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?;
    if let Err(error) = governance.authorize(
        CorrelationId::new(),
        StorageAccessRequest {
            actor: storage_actor,
            operation: StorageOperation::WriteImmutable,
            surface: StorageAccessSurface::SameOriginApplication,
            object: &governed_upload,
            now: storage_now,
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        },
    ) {
        return storage_policy_error(error, request_id, config.production());
    }
    let storage_correlation = CorrelationId::new();
    let storage_repository =
        storage_governance_runtime::D1StorageGovernanceRepository::with_cutover_fence(
            &database,
            authority_fence.scoped.clone(),
            storage_now,
            format!("storage-direct-upload:{storage_correlation}"),
        )
        .map_err(|_| Error::RustError("storage mutation fence is invalid".into()))?;
    let storage_request_context =
        storage_context(tenant_contract, &actor.user_id, storage_correlation);
    let storage_reservation = match governance
        .reserve_quota(
            &storage_repository,
            storage_request_context.clone(),
            StorageQuotaPolicy::new(
                ByteSize::new(MAX_SAFE_INTEGER)
                    .map_err(|_| Error::RustError("storage quota is invalid".into()))?,
                MAX_SAFE_INTEGER,
            )
            .map_err(|_| Error::RustError("storage quota is invalid".into()))?,
            ByteSize::new(expected_bytes)
                .map_err(|_| Error::RustError("direct upload size is invalid".into()))?,
            storage_now,
            storage_timestamp(
                now.checked_add(STORAGE_RESERVATION_TTL_MS)
                    .ok_or_else(|| Error::RustError("storage reservation overflowed".into()))?,
            )
            .ok_or_else(|| Error::RustError("storage reservation expiry is invalid".into()))?,
        )
        .await
    {
        Ok(reservation) => reservation,
        Err(_) => {
            return failure_response(
                ApiFailure::new(
                    409,
                    "storage_quota_exceeded",
                    "The storage quota does not allow this upload.",
                    true,
                ),
                request_id,
                config.production(),
            );
        }
    };
    let uploading_fingerprint = digest_identifier(
        "direct_upload_event",
        &format!("{upload_id}:uploading:{}", body.checksum_sha256),
    )
    .map_err(|()| Error::RustError("direct uploading event is invalid".into()))?;
    let finalizing_fingerprint = digest_identifier(
        "direct_upload_event",
        &format!("{upload_id}:finalizing:{}", body.checksum_sha256),
    )
    .map_err(|()| Error::RustError("direct finalizing event is invalid".into()))?;
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("direct-upload-finalizing:{upload_id}"),
            now,
            vec![
                database
                    .prepare(
                        "UPDATE video_uploads SET state = 'uploading', updated_at_ms = ?3, revision = revision + 1, \
                           event_sequence = event_sequence + 1, event_fingerprint = ?4 \
                         WHERE id = ?1 AND organization_id = ?2 AND transfer_mode = 'direct' \
                           AND state = 'initiated'",
                    )
                    .bind(&[
                        JsValue::from_str(upload_id),
                        JsValue::from_str(&tenant_id),
                        JsValue::from_f64(now as f64),
                        JsValue::from_str(&uploading_fingerprint),
                    ])?,
                database
                    .prepare(
                        "UPDATE video_uploads SET state = 'finalizing', updated_at_ms = ?3, revision = revision + 1, \
                           event_sequence = event_sequence + 1, event_fingerprint = ?4 \
                         WHERE id = ?1 AND organization_id = ?2 AND transfer_mode = 'direct' \
                           AND state = 'uploading'",
                    )
                    .bind(&[
                        JsValue::from_str(upload_id),
                        JsValue::from_str(&tenant_id),
                        JsValue::from_f64(now as f64),
                        JsValue::from_str(&finalizing_fingerprint),
                    ])?,
            ],
        )
        .await?,
    )?;

    let final_custom = HashMap::from([("frame-sha256".into(), body.checksum_sha256.clone())]);
    let final_object = if let Some(existing) = bucket.head(&upload.source_object_key).await? {
        existing
    } else {
        let staged_object = bucket
            .get(staging_key)
            .execute()
            .await?
            .ok_or_else(|| Error::RustError("direct staging object disappeared".into()))?;
        let staged_body = staged_object
            .body()
            .ok_or_else(|| Error::RustError("direct staging body disappeared".into()))?;
        let stream = FixedLengthStream::wrap(staged_body.stream()?, expected_bytes);
        match bucket
            .put(&upload.source_object_key, stream)
            .http_metadata(HttpMetadata {
                content_type: Some(upload.content_type.clone()),
                content_disposition: Some("attachment".into()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .custom_metadata(final_custom.clone())
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
        {
            Some(created) => created,
            None => bucket
                .head(&upload.source_object_key)
                .await?
                .ok_or_else(|| Error::RustError("direct publication conflicted".into()))?,
        }
    };
    let final_http = final_object.http_metadata();
    if final_object.size() != expected_bytes
        || final_object.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || final_http.content_type.as_deref() != Some(upload.content_type.as_str())
        || final_http.content_encoding.is_some()
        || final_http.cache_control.as_deref() != Some("private, no-store")
        || final_object.custom_metadata()? != final_custom
    {
        return failure_response(
            media_unavailable_failure("upload_checksum_mismatch"),
            request_id,
            config.production(),
        );
    }

    let etag = final_object.etag();
    let storage_object_id = new_id();
    let outbox_id = new_id();
    let status = UploadStatusResponse {
        schema_version: API_SCHEMA_VERSION,
        upload_id: upload_id.into(),
        state: "complete".into(),
        expected_bytes,
        received_bytes: expected_bytes,
        content_type: upload.content_type.clone(),
    };
    let status_json = serde_json::to_string(&status)
        .map_err(|_| Error::RustError("direct finalize response could not be serialized".into()))?;
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "upload_id": upload.id,
        "video_id": upload.video_id,
        "source_version": upload.source_version,
        "bytes": expected_bytes,
        "transfer_mode": "direct",
    })
    .to_string();
    let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let complete_fingerprint = digest_identifier(
        "direct_upload_event",
        &format!("{upload_id}:complete:{}", body.checksum_sha256),
    )
    .map_err(|()| Error::RustError("direct completion event is invalid".into()))?;
    let statements = vec![
        database
            .prepare(
                "UPDATE video_uploads SET state = 'complete', received_bytes = expected_bytes, \
                   checksum_sha256 = ?3, updated_at_ms = ?4, revision = revision + 1, \
                   event_sequence = event_sequence + 1, event_fingerprint = ?5 \
                 WHERE id = ?1 AND organization_id = ?2 AND transfer_mode = 'direct' \
                   AND direct_checksum_sha256 = ?3 AND state = 'finalizing'",
            )
            .bind(&[
                JsValue::from_str(upload_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&body.checksum_sha256),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&complete_fingerprint),
            ])?,
        database
            .prepare(
                "INSERT INTO object_manifests(object_key, video_id, role, bytes, checksum_sha256, \
                   content_type, created_at_ms, organization_id, object_version, provider_etag, state, updated_at_ms) \
                 VALUES (?1, ?2, 'source', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'available', ?6) \
                 ON CONFLICT(object_key) DO UPDATE SET bytes = excluded.bytes, \
                   checksum_sha256 = excluded.checksum_sha256, content_type = excluded.content_type, \
                   provider_etag = excluded.provider_etag, state = 'available', updated_at_ms = excluded.updated_at_ms \
                 WHERE object_manifests.video_id = excluded.video_id \
                   AND object_manifests.organization_id = excluded.organization_id \
                   AND object_manifests.role = excluded.role \
                   AND object_manifests.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_str(&upload.video_id),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&body.checksum_sha256),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&etag),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(id, organization_id, integration_id, video_id, object_key, \
                   role, object_version, state, bytes, content_type, checksum_sha256, provider_etag, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'source', ?6, 'available', ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT(integration_id, object_key) DO UPDATE SET state = 'available', \
                   bytes = excluded.bytes, content_type = excluded.content_type, \
                   checksum_sha256 = excluded.checksum_sha256, provider_etag = excluded.provider_etag \
                 WHERE storage_objects.organization_id = excluded.organization_id \
                   AND storage_objects.video_id = excluded.video_id \
                   AND storage_objects.role = excluded.role \
                   AND storage_objects.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_str(&body.checksum_sha256),
                JsValue::from_str(&etag),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_governed_objects_v1(organization_id, object_key, role, visibility, \
                   state, malware_disposition, immutable_revision, cache_generation, checksum_sha256, \
                   bytes, content_type, retention_until_ms, created_at_ms, updated_at_ms) \
                 VALUES (?1, ?2, 'source', 'private', 'active', 'clean', ?3, 1, ?4, ?5, ?6, NULL, ?7, ?7) \
                 ON CONFLICT(organization_id, object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(upload.source_version as f64),
                JsValue::from_str(&body.checksum_sha256),
                JsValue::from_f64(expected_bytes as f64),
                JsValue::from_str(&upload.content_type),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "UPDATE videos SET source_object_key = ?3, state = 'processing', \
                   updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(&upload.video_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&upload.source_object_key),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(organization_id, idempotency_key, command_type, \
                   request_digest, response_status, response_json, created_at_ms, expires_at_ms) \
                 VALUES (?1, ?2, 'direct_upload_finalize', ?3, 200, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&status_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(now.saturating_add(COMMAND_TTL_MS) as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision) \
                 VALUES (?1, ?2, 'video_upload', ?3, 'upload.completed', ?4, ?5, 'pending', 0, ?6, ?6, \
                         0, ?7, 1, ?8, 0)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(upload_id),
                JsValue::from_str(&format!("upload-complete:{upload_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(outbox_payload_checksum.as_str()),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("direct-upload-complete:{upload_id}"),
            now,
            statements,
        )
        .await?,
    )?;
    match storage_repository
        .release_quota_reservation(
            storage_request_context,
            storage_reservation.reservation_id(),
            true,
            storage_now,
        )
        .await
    {
        Ok(
            frame_ports::StorageCasOutcomeV1::Applied | frame_ports::StorageCasOutcomeV1::Replay,
        ) => {}
        Ok(frame_ports::StorageCasOutcomeV1::Conflict) | Err(_) => {
            return Err(Error::RustError(
                "direct upload quota commit reconciliation is required".into(),
            ));
        }
    }
    let _ = bucket.delete(staging_key).await;
    json_response(&status, 200, None)
}

async fn media_job_create_response(
    env: &Env,
    config: &RuntimeConfig,
    context: &Context,
    request: &Request,
    actor: &AuthenticatedActor,
    mut body: MediaJobRequest,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    if let Err(code) = body.validate() {
        return failure_response(
            invalid_body_failure(code.as_str()),
            request_id,
            config.production(),
        );
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let (digest, serializer_alias_digest) = media_job_create_digests(&body)
        .map_err(|()| Error::RustError("media command could not be digested".into()))?;
    match command_replay_accepting(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "media_job_create",
        &digest,
        serializer_alias_digest.as_deref(),
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    // Replay lookup canonicalizes an omitted or equivalent explicit singleton
    // to the pre-0027 digest and also accepts the explicit rollout alias. Only
    // new work is expanded to the durable, explicit input array below.
    body.source_inputs = body.normalized_source_inputs();
    #[derive(Deserialize)]
    struct MediaInputRolloutRow {
        phase: String,
    }
    let rollout = database
        .prepare("SELECT phase FROM media_job_input_rollout_v1 WHERE singleton=1 LIMIT 1")
        .first::<MediaInputRolloutRow>(None)
        .await?
        .ok_or_else(|| Error::RustError("media input rollout state is unavailable".into()))?;
    if !matches!(rollout.phase.as_str(), "expand" | "enforced") {
        return Err(Error::RustError(
            "media input rollout state is invalid".into(),
        ));
    }
    if rollout.phase != "enforced"
        && (body.source_inputs.len() > 1 || body.profile == "composition_v1")
    {
        return failure_response(
            ApiFailure::new(
                503,
                "media_input_contract_pending",
                "The selected media profile is waiting for a compatible rollout.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let mut resolved_sources = Vec::with_capacity(body.source_inputs.len());
    for input in &body.source_inputs {
        if !video_is_scoped(&database, &tenant_id, &input.video_id).await? {
            return failure_response(not_found_failure(), request_id, config.production());
        }
        let Some(source) =
            load_source_object(&database, &tenant_id, &input.video_id, input.source_version)
                .await?
        else {
            return failure_response(
                ApiFailure::new(
                    409,
                    "source_not_ready",
                    "A source object is not ready for processing.",
                    true,
                ),
                request_id,
                config.production(),
            );
        };
        if !supported_native_source_content_type(&source.content_type)
            || source.bytes <= 0
            || source
                .checksum_sha256
                .as_deref()
                .is_none_or(|checksum| !contracts::valid_sha256(checksum))
        {
            return failure_response(
                invalid_body_failure("unsupported_source_media_type"),
                request_id,
                config.production(),
            );
        }
        resolved_sources.push(source);
    }
    let source = resolved_sources
        .first()
        .ok_or_else(|| Error::RustError("validated media sources are unavailable".into()))?;
    if config.media_mode == MediaMode::Fake && body.profile != "preview_v1" {
        return failure_response(
            ApiFailure::new(
                422,
                "profile_unavailable",
                "The selected media profile is unavailable in this runtime.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    // Remote mode is hybrid: managed profiles use the provider adapter and
    // every other profile falls back to the native worker. Reject a request
    // before persistence when that native path has no executable claim
    // contract; otherwise the job can remain queued forever.
    if requires_native_claim(config.media_mode, &body.profile)
        && native_claim_output(
            &body.profile,
            &serde_json::to_string(&body)
                .map_err(|_| Error::RustError("media request could not be serialized".into()))?,
        )
        .is_none()
    {
        return failure_response(
            ApiFailure::new(
                422,
                "profile_unavailable",
                "The selected media profile is unavailable in this runtime.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let kind = profile_kind(&body.profile)
        .ok_or_else(|| Error::RustError("validated profile is unsupported".into()))?;
    let now = current_time_ms()?;
    let Some(tenant_contract) = storage_tenant(&tenant_id) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let mut source_authorities = Vec::with_capacity(resolved_sources.len());
    for source in &resolved_sources {
        let Some(authority) = governed_object(
            &database,
            tenant_contract,
            &source.object_key,
            &actor.user_id,
        )
        .await
        .map_err(|()| Error::RustError("storage authority is unavailable".into()))?
        else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        source_authorities.push(authority);
    }
    let governance =
        storage_governance_runtime::governance_service(env, &storage_origin(config))
            .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    for source_authority in &source_authorities {
        if let Err(error) = governance.authorize(
            CorrelationId::new(),
            StorageAccessRequest {
                actor: StorageActor::Service {
                    tenant_id: tenant_contract,
                    purpose: frame_domain::StorageServicePurpose::MediaProcessor,
                },
                operation: StorageOperation::Read,
                surface: StorageAccessSurface::MediaTransformation,
                object: source_authority,
                now: storage_timestamp(now)
                    .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?,
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            },
        ) {
            return storage_policy_error(error, request_id, config.production());
        }
    }
    let managed_execution =
        if config.media_mode == MediaMode::Remote && contracts::managed_profile(&body.profile) {
            let Some(source_checksum) = source.checksum_sha256.as_deref() else {
                return failure_response(
                    ApiFailure::new(
                        409,
                        "source_probe_required",
                        "A verified media probe is required before processing.",
                        true,
                    ),
                    request_id,
                    config.production(),
                );
            };
            let Some(probe) = media_service_runtime::load_verified_probe(
                &database,
                &tenant_id,
                &body.video_id,
                body.source_version,
            )
            .await?
            else {
                return failure_response(
                    ApiFailure::new(
                        409,
                        "source_probe_required",
                        "A verified media probe is required before processing.",
                        true,
                    ),
                    request_id,
                    config.production(),
                );
            };
            if probe
                .validate_exact_source(
                    &tenant_id,
                    &body.video_id,
                    body.source_version,
                    &source.object_key,
                    source.bytes,
                    source_checksum,
                    &source.content_type,
                )
                .is_err()
            {
                return failure_response(
                    ApiFailure::new(
                        409,
                        "source_probe_stale",
                        "The verified media probe no longer matches the source.",
                        true,
                    ),
                    request_id,
                    config.production(),
                );
            }
            let seed = match media_service_runtime::ManagedExecutionSeed::for_request(
                &body,
                &probe,
                media_service_runtime::managed_media_enabled(env),
            ) {
                Ok(seed) => seed,
                Err(_) => {
                    return failure_response(
                        invalid_body_failure("invalid_profile"),
                        request_id,
                        config.production(),
                    );
                }
            };
            Some(seed)
        } else {
            None
        };
    let executor = managed_execution
        .as_ref()
        .map_or("native_gstreamer", |seed| seed.selected_executor);
    if executor == "native_gstreamer" {
        let native_sources = body
            .source_inputs
            .iter()
            .zip(&resolved_sources)
            .enumerate()
            .map(|(ordinal, (input, source))| {
                Ok(WorkerSourceRow {
                    ordinal: i64::try_from(ordinal).map_err(|_| ())?,
                    video_id: input.video_id.clone(),
                    source_version: i64::from(input.source_version),
                    object_key: source.object_key.clone(),
                    bytes: source.bytes,
                    checksum_sha256: source.checksum_sha256.clone().ok_or(())?,
                    content_type: source.content_type.clone(),
                })
            })
            .collect::<std::result::Result<Vec<_>, ()>>();
        if !native_sources.is_ok_and(|sources| {
            validated_worker_sources(&tenant_id, &body.profile, sources).is_ok()
        }) {
            return failure_response(
                ApiFailure::new(
                    422,
                    "source_outside_native_sandbox",
                    "The source set exceeds the selected media profile limits.",
                    false,
                ),
                request_id,
                config.production(),
            );
        }
    }
    let normalized_profile_sha256 = if let Some(seed) = managed_execution.as_ref() {
        seed.normalized_profile_sha256.clone()
    } else {
        let artifact_sources = body
            .source_inputs
            .iter()
            .zip(&resolved_sources)
            .zip(&source_authorities)
            .enumerate()
            .map(|(ordinal, ((input, source), authority))| {
                Ok(NativeArtifactSourceIdentityV2 {
                    ordinal: u16::try_from(ordinal)
                        .map_err(|_| Error::RustError("media source ordinal is invalid".into()))?,
                    video_id: input.video_id.clone(),
                    source_version: input.source_version,
                    object_key: source.object_key.clone(),
                    bytes: u64::try_from(source.bytes)
                        .map_err(|_| Error::RustError("media source size is invalid".into()))?,
                    checksum_sha256: source.checksum_sha256.clone().ok_or_else(|| {
                        Error::RustError("media source checksum is unavailable".into())
                    })?,
                    content_type: source.content_type.clone(),
                    authority_digest: authority.audit_digest().as_str().to_owned(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // V2 identity binds the profile payload and every ordered immutable
        // source authority field. Repeated composition identities remain
        // distinct because each occurrence carries its own dense ordinal.
        native_artifact_digest(&body, &artifact_sources)
            .map_err(|()| Error::RustError("media artifact could not be digested".into()))?
    };
    let job_id = new_id();
    let output_key = match storage_governance_runtime::deterministic_derivative_key_for_profile(
        env,
        tenant_contract,
        &body.video_id,
        &body.profile,
        &normalized_profile_sha256,
        &source_authorities[0],
    ) {
        Ok(key) => key,
        Err(_) => {
            return failure_response(
                media_unavailable_failure("managed_media_disabled"),
                request_id,
                config.production(),
            );
        }
    };
    let response = MediaJobResponse::new(job_id.clone(), body.profile.clone(), executor.into());
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("media response could not be serialized".into()))?;
    let mut persisted_body = body.clone();
    if persisted_body.source_inputs.len() == 1 && persisted_body.profile != "composition_v1" {
        // The immutable input table is authoritative. Omitting the redundant
        // singleton field keeps the queued payload parseable by the released
        // N-1 Worker throughout the expand/deploy window.
        persisted_body.source_inputs.clear();
    }
    let payload_json = serde_json::to_string(&persisted_body)
        .map_err(|_| Error::RustError("media request could not be serialized".into()))?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "video_id": body.video_id,
        "profile": body.profile,
        "source_version": body.source_version,
    })
    .to_string();
    let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let scoped_job_idempotency_key = digest_identifier(
        "media_job_resource",
        &format!("{tenant_id}:{idempotency_key}:{job_id}"),
    )
    .map_err(|()| Error::RustError("media job resource identity is invalid".into()))?;
    let profile_version = managed_execution
        .as_ref()
        .map_or(1, |seed| seed.profile_version);
    let mut statements = vec![
        database
            .prepare(
                "INSERT INTO media_jobs(\
                   id, video_id, kind, state, idempotency_key, attempt, payload_json, \
                   created_at_ms, updated_at_ms, organization_id, selected_executor, \
                   source_version, profile_version, output_object_key, cancel_requested, revision, \
                   input_contract_version\
                 ) VALUES (?1, ?2, ?3, 'queued', ?4, 0, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, 0, 0, 1)",
            )
            .bind(&[
                JsValue::from_str(&job_id),
                JsValue::from_str(&body.video_id),
                JsValue::from_str(kind),
                JsValue::from_str(&scoped_job_idempotency_key),
                JsValue::from_str(&payload_json),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(executor),
                JsValue::from_f64(f64::from(body.source_version)),
                JsValue::from_f64(f64::from(profile_version)),
                JsValue::from_str(&output_key),
            ])?,
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) VALUES (?1, ?2, 'media_job_create', ?3, 202, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) VALUES (?1, ?2, 'media_job', ?3, 'media.job.queued', ?4, ?5, \
                           'pending', 0, ?6, ?6, 0, ?7, 1, ?8, 0)",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&job_id),
                JsValue::from_str(&format!("media-job:{job_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(outbox_payload_checksum.as_str()),
            ])?,
    ];
    if let Some(seed) = managed_execution.as_ref() {
        statements.push(
            database
                .prepare(
                    "INSERT INTO media_job_execution_v1(\
                       job_id, organization_id, video_id, source_version, catalog_version, \
                       profile_id, profile_version, normalized_profile_sha256, route_reason, \
                       selected_executor, fallback_executor, state, attempt, lease_epoch, \
                       lease_token_digest, lease_expires_at_ms, staging_object_key, final_object_key, \
                       output_content_type, max_output_bytes, created_at_ms, updated_at_ms\
                     ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9, ?10, 'queued', \
                               0, 0, NULL, NULL, NULL, ?11, ?12, ?13, ?14, ?14)",
                )
                .bind(&[
                    JsValue::from_str(&job_id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&body.video_id),
                    JsValue::from_f64(f64::from(body.source_version)),
                    JsValue::from_str(&body.profile),
                    JsValue::from_f64(f64::from(seed.profile_version)),
                    JsValue::from_str(&seed.normalized_profile_sha256),
                    JsValue::from_str(seed.route_reason),
                    JsValue::from_str(seed.selected_executor),
                    seed.fallback_executor
                        .map(JsValue::from_str)
                        .unwrap_or(JsValue::NULL),
                    JsValue::from_str(&output_key),
                    JsValue::from_str(seed.output_content_type),
                    JsValue::from_f64(seed.max_output_bytes as f64),
                    JsValue::from_f64(now as f64),
                ])?,
        );
    }
    for (ordinal, (input, source)) in body.source_inputs.iter().zip(&resolved_sources).enumerate() {
        let checksum = source
            .checksum_sha256
            .as_deref()
            .ok_or_else(|| Error::RustError("validated media source checksum is absent".into()))?;
        statements.push(
            database
                .prepare(
                    "INSERT INTO media_job_inputs_v1(\
                       job_id,organization_id,ordinal,video_id,source_version,object_key,bytes,\
                       checksum_sha256,content_type,created_at_ms) \
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                )
                .bind(&[
                    JsValue::from_str(&job_id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_f64(ordinal as f64),
                    JsValue::from_str(&input.video_id),
                    JsValue::from_f64(f64::from(input.source_version)),
                    JsValue::from_str(&source.object_key),
                    JsValue::from_f64(source.bytes as f64),
                    JsValue::from_str(checksum),
                    JsValue::from_str(&source.content_type),
                    JsValue::from_f64(now as f64),
                ])?,
        );
    }
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("media-create:{job_id}"),
            now,
            statements,
        )
        .await?,
    )?;

    if executor == "cloudflare_media" {
        context.wait_until(media_service_runtime::process_job(
            env.clone(),
            job_id.clone(),
        ));
    }

    let mut response = response;
    if config.media_mode == MediaMode::Fake && body.profile == "preview_v1" {
        if complete_fake_preview(
            env,
            &database,
            &authority_fence,
            FakePreview {
                tenant_id: &tenant_id,
                video_id: &body.video_id,
                job_id: &job_id,
                output_key: &output_key,
                source_version: body.source_version,
                source,
            },
        )
        .await
        .is_err()
        {
            mark_fake_job_failed(&database, &authority_fence, &tenant_id, &job_id).await?;
        }
        let current = load_media_job(&database, &tenant_id, &job_id)
            .await?
            .ok_or_else(|| Error::RustError("created media job disappeared".into()))?;
        response.state = current.state;
        let response_json = serde_json::to_string(&response)
            .map_err(|_| Error::RustError("media response could not be serialized".into()))?;
        require_batch_success(
            execute_mutation_batch(
                &database,
                &authority_fence,
                &format!("media-fake-response:{job_id}"),
                current_time_ms()?,
                vec![
                    database
                        .prepare(
                            "UPDATE command_idempotency SET response_json = ?4 \
                         WHERE organization_id = ?1 AND idempotency_key = ?2 \
                           AND command_type = 'media_job_create' AND request_digest = ?3",
                        )
                        .bind(&[
                            JsValue::from_str(&tenant_id),
                            JsValue::from_str(&idempotency_key),
                            JsValue::from_str(&digest),
                            JsValue::from_str(&response_json),
                        ])?,
                ],
            )
            .await?,
        )?;
    }
    json_response(&response, 202, Some(&response.status_path))
}

async fn native_job_claim_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: WorkerClaimRequest,
    request_id: &str,
) -> Result<Response> {
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let lease_token = worker_lease_token_header(request)?;
    let lease_digest = digest_credential(&lease_token);
    let idempotency_key = idempotency_header(request)?;
    let digest_value = serde_json::json!({
        "body": body,
        "lease_token_digest": lease_digest,
    });
    let digest = request_digest("native_job_claim", &digest_value)
        .map_err(|()| Error::RustError("worker claim could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_claim",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }

    let now = current_time_ms()?;
    reap_invalid_queued_native_job(&database, &tenant_id, now, &authority_fence).await?;
    reap_exhausted_native_jobs(&database, &tenant_id, now, &authority_fence).await?;
    for _ in 0..2 {
        let candidate = database
            .prepare(
                "SELECT j.id, j.revision, j.attempt, \
                        json_extract(j.payload_json, '$.profile') AS profile, j.payload_json, \
                        m.bytes AS source_bytes, m.checksum_sha256 AS source_checksum_sha256, \
                        m.content_type AS source_content_type \
                 FROM media_jobs j \
                 JOIN media_job_current_inputs_v1 m ON m.job_id = j.id \
                  AND m.organization_id = j.organization_id AND m.ordinal = 0 \
                 WHERE j.organization_id = ?1 AND j.selected_executor = 'native_gstreamer' \
                   AND json_extract(j.payload_json, '$.profile') IN ( \
                     'optimized_clip_v1','thumbnail_v1','spritesheet_v1','audio_extract_v1', \
                     'probe_v1','audio_presence_v1','distribution_master_v1', \
                     'animated_preview_v1','audio_normalize_v1','remux_repair_v1', \
                     'waveform_v1','composition_v1','normalize_v1') \
                   AND j.source_version IS NOT NULL AND j.output_object_key IS NOT NULL \
                   AND j.cancel_requested = 0 AND j.attempt < ?2 \
                   AND j.attempt < CASE json_extract(j.payload_json, '$.profile') \
                     WHEN 'probe_v1' THEN 2 WHEN 'audio_presence_v1' THEN 2 ELSE 3 END \
                   AND (j.state = 'queued' OR (j.state IN ('leased', 'running') \
                     AND j.lease_expires_at_ms IS NOT NULL AND j.lease_expires_at_ms <= ?3)) \
                   AND m.bytes BETWEEN 1 AND CASE json_extract(j.payload_json, '$.profile') \
                     WHEN 'probe_v1' THEN ?4 ELSE ?5 END \
                   AND m.checksum_sha256 IS NOT NULL \
                   AND length(m.checksum_sha256) = 64 AND lower(m.checksum_sha256) = m.checksum_sha256 \
                   AND m.checksum_sha256 NOT GLOB '*[^0-9a-f]*' \
                   AND m.content_type IN (\
                     'video/mp4','video/quicktime','video/webm','video/x-matroska',\
                     'audio/mpeg','audio/mp4','audio/wav','audio/webm','audio/ogg') \
                   AND NOT EXISTS (SELECT 1 FROM media_job_current_inputs_v1 invalid \
                     WHERE invalid.job_id = j.id AND invalid.organization_id = j.organization_id \
                       AND (invalid.source_version <= 0 \
                         OR invalid.bytes NOT BETWEEN 1 AND CASE \
                           json_extract(j.payload_json, '$.profile') \
                           WHEN 'probe_v1' THEN ?4 ELSE ?5 END \
                         OR invalid.checksum_sha256 IS NULL \
                         OR length(invalid.checksum_sha256) != 64 \
                         OR lower(invalid.checksum_sha256) != invalid.checksum_sha256 \
                         OR invalid.checksum_sha256 GLOB '*[^0-9a-f]*' \
                         OR invalid.content_type NOT IN (\
                           'video/mp4','video/quicktime','video/webm','video/x-matroska',\
                           'audio/mpeg','audio/mp4','audio/wav','audio/webm','audio/ogg') \
                         OR substr(invalid.object_key, 1, length('tenants/' || j.organization_id || \
                           '/videos/' || invalid.video_id || '/')) != \
                           'tenants/' || j.organization_id || '/videos/' || invalid.video_id || '/' \
                         OR instr(invalid.object_key, '..') != 0 \
                         OR instr(invalid.object_key, char(92)) != 0 \
                         OR instr(invalid.object_key, '?') != 0 \
                         OR instr(invalid.object_key, '#') != 0 \
                         OR instr(invalid.object_key, '%') != 0)) \
                   AND (SELECT COALESCE(SUM(inputs.bytes), 0) \
                     FROM media_job_current_inputs_v1 inputs \
                     WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) \
                     <= CASE WHEN json_extract(j.payload_json, '$.profile') IN (\
                       'distribution_master_v1','remux_repair_v1',\
                       'composition_v1','normalize_v1') THEN ?6 ELSE ?7 END \
                   AND substr(m.object_key, 1, length('tenants/' || j.organization_id || \
                     '/videos/' || j.video_id || '/')) = \
                     'tenants/' || j.organization_id || '/videos/' || j.video_id || '/' \
                   AND instr(m.object_key, '..') = 0 AND instr(m.object_key, char(92)) = 0 \
                   AND instr(m.object_key, '?') = 0 AND instr(m.object_key, '#') = 0 \
                   AND instr(m.object_key, '%') = 0 \
                   AND substr(j.output_object_key, 1, length('tenants/' || j.organization_id || \
                     '/videos/' || j.video_id || '/derivatives/' || \
                     json_extract(j.payload_json, '$.profile') || '/')) = \
                     'tenants/' || j.organization_id || '/videos/' || j.video_id || \
                     '/derivatives/' || json_extract(j.payload_json, '$.profile') || '/' \
                   AND length(j.output_object_key) = length('tenants/' || j.organization_id || \
                     '/videos/' || j.video_id || '/derivatives/' || \
                     json_extract(j.payload_json, '$.profile') || '/') + 64 \
                   AND lower(substr(j.output_object_key, -64)) = substr(j.output_object_key, -64) \
                   AND substr(j.output_object_key, -64) NOT GLOB '*[^0-9a-f]*' \
                   AND j.input_contract_version = 1 \
                   AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 inputs \
                         WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) = \
                       (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                         WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                   AND (SELECT MIN(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                         WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) = 0 \
                   AND (SELECT MAX(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                         WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) = \
                       (SELECT COUNT(*) - 1 FROM media_job_inputs_v1 bound \
                         WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                   AND CASE json_extract(j.payload_json, '$.profile') \
                     WHEN 'composition_v1' THEN \
                       (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                         WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                         BETWEEN 1 AND 64 \
                     ELSE (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                         WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) = 1 END \
                 ORDER BY j.created_at_ms, j.id LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(NATIVE_MAX_ATTEMPTS as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(MULTIPART_MAX_BYTES as f64),
                JsValue::from_f64(MAX_SINGLE_UPLOAD_BYTES as f64),
                JsValue::from_f64(NATIVE_HEAVY_MAX_SOURCE_BYTES as f64),
                JsValue::from_f64(NATIVE_STANDARD_MAX_SOURCE_BYTES as f64),
            ])?
            .first::<NativeJobCandidateRow>(None)
            .await?;
        let Some(candidate) = candidate else {
            return Ok(Response::empty()?.with_status(204));
        };
        let loaded_sources = load_worker_sources(&database, &tenant_id, &candidate.id).await?;
        let Ok(sources) = validated_worker_sources(&tenant_id, &candidate.profile, loaded_sources)
        else {
            // Current authority changed after candidate selection. Re-query
            // instead of leasing a partial/stale source set or blocking the
            // queue behind it.
            continue;
        };
        if sources.first().is_none_or(|source| {
            source.bytes != candidate.source_bytes
                || source.checksum_sha256 != candidate.source_checksum_sha256
                || source.content_type != candidate.source_content_type
        }) {
            continue;
        }
        let max_attempts = native_profile_max_attempts(&candidate.profile);
        let next_attempt = candidate
            .attempt
            .checked_add(1)
            .filter(|attempt| *attempt <= max_attempts)
            .ok_or_else(|| Error::RustError("worker claim attempt is invalid".into()))?;
        let next_revision = candidate
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= i64::try_from(MAX_SAFE_INTEGER).unwrap_or(i64::MAX))
            .ok_or_else(|| Error::RustError("worker claim revision is invalid".into()))?;
        let lease_expires_at_ms = now
            .checked_add(NATIVE_LEASE_MS)
            .ok_or_else(|| Error::RustError("worker lease expiry overflowed".into()))?;
        let (output_content_type, output_max_bytes) =
            native_claim_output(&candidate.profile, &candidate.payload_json).ok_or_else(|| {
                Error::RustError("worker claim output contract is invalid".into())
            })?;
        let output_role = native_catalog_output_role(&candidate.profile)
            .ok_or_else(|| Error::RustError("worker claim output role is invalid".into()))?;
        let execution_origin = native_execution_origin(&candidate.profile)
            .ok_or_else(|| Error::RustError("worker claim origin is invalid".into()))?;
        let sandbox = native_sandbox(&candidate.profile)
            .ok_or_else(|| Error::RustError("worker claim sandbox is invalid".into()))?;
        let response = NativeJobClaimResponse {
            schema_version: API_SCHEMA_VERSION,
            native_plan_schema_version: 1,
            media_job_catalog_version: 2,
            media_service_catalog_version: 1,
            job_id: candidate.id.clone(),
            state: "leased".into(),
            profile: candidate.profile.clone(),
            execution_origin: execution_origin.into(),
            attempt: u32::try_from(next_attempt)
                .map_err(|_| Error::RustError("worker attempt is invalid".into()))?,
            revision: u64::try_from(next_revision)
                .map_err(|_| Error::RustError("worker revision is invalid".into()))?,
            lease_expires_at_ms: u64::try_from(lease_expires_at_ms)
                .map_err(|_| Error::RustError("worker lease expiry is invalid".into()))?,
            sources: sources
                .iter()
                .map(|source| {
                    Ok(WorkerSourceDescriptor {
                        ordinal: u16::try_from(source.ordinal).map_err(|_| {
                            Error::RustError("worker source ordinal is invalid".into())
                        })?,
                        path: format!(
                            "/api/v1/worker/media-jobs/{}/sources/{}",
                            candidate.id, source.ordinal
                        ),
                        bytes: u64::try_from(source.bytes).map_err(|_| {
                            Error::RustError("worker source size is invalid".into())
                        })?,
                        checksum_sha256: source.checksum_sha256.clone(),
                        content_type: source.content_type.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            outputs: vec![WorkerOutputDescriptor {
                ordinal: 0,
                role: output_role.into(),
                path: format!("/api/v1/worker/media-jobs/{}/outputs/0", candidate.id),
                content_type: output_content_type,
                max_bytes: output_max_bytes,
            }],
            sandbox,
            heartbeat_path: format!("/api/v1/worker/media-jobs/{}/heartbeat", candidate.id),
            progress_path: format!("/api/v1/worker/media-jobs/{}/progress", candidate.id),
            complete_path: format!("/api/v1/worker/media-jobs/{}/complete", candidate.id),
            fail_path: format!("/api/v1/worker/media-jobs/{}/fail", candidate.id),
        };
        let response_json = serde_json::to_string(&response).map_err(|_| {
            Error::RustError("worker claim response could not be serialized".into())
        })?;
        let outbox_id = new_id();
        let outbox_payload = serde_json::json!({
            "schema_version": API_SCHEMA_VERSION,
            "job_id": candidate.id,
            "attempt": next_attempt,
            "state": "leased",
        })
        .to_string();
        let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
        let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
        let reservation_id = new_id();
        let statements = vec![
            worker_command_reservation(
                &database,
                &tenant_id,
                &idempotency_key,
                "native_job_claim",
                &digest,
                &reservation_id,
                now,
            )?,
            database
                .prepare(
                    "UPDATE media_jobs SET state = 'leased', attempt = attempt + 1, \
                       worker_id = ?4, lease_token_digest = ?5, lease_expires_at_ms = ?6, \
                       heartbeat_at_ms = ?3, progress_basis_points = 0, error_code = NULL, \
                       error_class = NULL, updated_at_ms = ?3, revision = revision + 1 \
                     WHERE id = ?1 AND organization_id = ?2 AND revision = ?7 \
                       AND selected_executor = 'native_gstreamer' AND cancel_requested = 0 \
                       AND attempt < ?8 AND (state = 'queued' OR (state IN ('leased', 'running') \
                         AND lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms <= ?3)) \
                       AND input_contract_version = 1 \
                       AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 inputs \
                             WHERE inputs.job_id = media_jobs.id \
                               AND inputs.organization_id = media_jobs.organization_id) = \
                           (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                             WHERE bound.job_id = media_jobs.id \
                               AND bound.organization_id = media_jobs.organization_id) \
                       AND (SELECT MIN(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                             WHERE inputs.job_id = media_jobs.id \
                               AND inputs.organization_id = media_jobs.organization_id) = 0 \
                       AND (SELECT MAX(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                             WHERE inputs.job_id = media_jobs.id \
                               AND inputs.organization_id = media_jobs.organization_id) = \
                           (SELECT COUNT(*) - 1 FROM media_job_inputs_v1 bound \
                             WHERE bound.job_id = media_jobs.id \
                               AND bound.organization_id = media_jobs.organization_id) \
                       AND CASE json_extract(payload_json, '$.profile') \
                         WHEN 'composition_v1' THEN \
                           (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                             WHERE bound.job_id = media_jobs.id \
                               AND bound.organization_id = media_jobs.organization_id) BETWEEN 1 AND 64 \
                         ELSE (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                             WHERE bound.job_id = media_jobs.id \
                               AND bound.organization_id = media_jobs.organization_id) = 1 END \
                       AND (?9 = -1 OR EXISTS (SELECT 1 FROM authority_state a \
                         WHERE a.singleton = 1 AND a.epoch = ?9 AND a.authority = 'd1' \
                           AND a.phase IN ('d1_authoritative', 'finalized'))) \
                       AND EXISTS (SELECT 1 FROM command_idempotency c \
                         WHERE c.organization_id = ?2 AND c.idempotency_key = ?10 \
                           AND c.command_type = 'native_job_claim' AND c.request_digest = ?11 \
                           AND c.reservation_id = ?12 AND c.response_status IS NULL)",
                )
                .bind(&[
                    JsValue::from_str(&candidate.id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_f64(now as f64),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_f64(lease_expires_at_ms as f64),
                    JsValue::from_f64(candidate.revision as f64),
                    JsValue::from_f64(max_attempts as f64),
                    JsValue::from_f64(authority_fence.sql_epoch as f64),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&reservation_id),
                ])?,
            database
                .prepare(
                    "UPDATE media_job_attempts SET finished_at_ms = ?3, outcome = 'lost_lease', \
                       error_class = 'lease_expired' WHERE job_id = ?1 AND attempt = ?2 - 1 \
                       AND outcome IS NULL AND EXISTS (SELECT 1 FROM media_jobs j \
                         WHERE j.id = ?1 AND j.organization_id = ?4 AND j.attempt = ?2 \
                           AND j.worker_id = ?5 AND j.lease_token_digest = ?6) \
                       AND EXISTS (SELECT 1 FROM command_idempotency c \
                         WHERE c.organization_id = ?4 AND c.idempotency_key = ?7 \
                           AND c.command_type = 'native_job_claim' AND c.request_digest = ?8 \
                           AND c.reservation_id = ?9 AND c.response_status IS NULL)",
                )
                .bind(&[
                    JsValue::from_str(&candidate.id),
                    JsValue::from_f64(next_attempt as f64),
                    JsValue::from_f64(now as f64),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&reservation_id),
                ])?,
            database
                .prepare(
                    "INSERT INTO media_job_attempts(job_id, attempt, executor, worker_id, started_at_ms) \
                     SELECT ?1, ?2, 'native_gstreamer', ?3, ?4 FROM media_jobs j \
                     WHERE j.id = ?1 AND j.organization_id = ?5 AND j.state = 'leased' \
                       AND j.attempt = ?2 AND j.worker_id = ?3 AND j.lease_token_digest = ?6 \
                       AND EXISTS (SELECT 1 FROM command_idempotency c \
                         WHERE c.organization_id = ?5 AND c.idempotency_key = ?7 \
                           AND c.command_type = 'native_job_claim' AND c.request_digest = ?8 \
                           AND c.reservation_id = ?9 AND c.response_status IS NULL) \
                     ON CONFLICT(job_id, attempt) DO NOTHING",
                )
                .bind(&[
                    JsValue::from_str(&candidate.id),
                    JsValue::from_f64(next_attempt as f64),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_f64(now as f64),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&reservation_id),
                ])?,
            database
                .prepare(
                    "UPDATE command_idempotency SET response_status = 200, response_json = ?4 \
                     WHERE organization_id = ?1 AND idempotency_key = ?2 \
                       AND command_type = 'native_job_claim' AND request_digest = ?3 \
                       AND reservation_id = ?5 AND response_status IS NULL \
                       AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?6 \
                         AND j.organization_id = ?1 AND j.state = 'leased' AND j.attempt = ?7 \
                         AND j.worker_id = ?8 AND j.lease_token_digest = ?9)",
                )
                .bind(&[
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&response_json),
                    JsValue::from_str(&reservation_id),
                    JsValue::from_str(&candidate.id),
                    JsValue::from_f64(next_attempt as f64),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_str(&lease_digest),
                ])?,
            database
                .prepare(
                    "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                       event_type, deduplication_key, payload_json, state, attempt, available_at_ms, \
                       created_at_ms, event_sequence, event_fingerprint, payload_schema_version, \
                       payload_checksum, revision) \
                     SELECT ?1, ?2, 'media_job', ?3, 'media.job.leased', ?4, ?5, 'pending', 0, ?6, ?6 \
                       , 0, ?13, 1, ?14, 0 \
                     FROM media_jobs j WHERE j.id = ?3 AND j.organization_id = ?2 \
                       AND j.state = 'leased' AND j.attempt = ?7 AND j.worker_id = ?8 \
                       AND j.lease_token_digest = ?9 \
                       AND EXISTS (SELECT 1 FROM command_idempotency c \
                         WHERE c.organization_id = ?2 AND c.idempotency_key = ?10 \
                           AND c.command_type = 'native_job_claim' AND c.request_digest = ?11 \
                           AND c.reservation_id = ?12 AND c.response_status = 200) \
                     ON CONFLICT(deduplication_key) DO NOTHING",
                )
                .bind(&[
                    JsValue::from_str(&outbox_id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&candidate.id),
                    JsValue::from_str(&format!("media-leased:{}:{next_attempt}", candidate.id)),
                    JsValue::from_str(&outbox_payload),
                    JsValue::from_f64(now as f64),
                    JsValue::from_f64(next_attempt as f64),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&reservation_id),
                    JsValue::from_str(outbox_event_fingerprint.as_str()),
                    JsValue::from_str(outbox_payload_checksum.as_str()),
                ])?,
            worker_command_reservation_cleanup(
                &database,
                &tenant_id,
                &idempotency_key,
                &reservation_id,
            )?,
        ];
        require_batch_success(
            execute_mutation_batch(
                &database,
                &authority_fence,
                &format!("native-claim:{}:{next_attempt}", candidate.id),
                now,
                statements,
            )
            .await?,
        )?;
        let claimed = load_worker_job(&database, &tenant_id, &candidate.id).await?;
        if claimed.as_ref().is_some_and(|job| {
            job.state == "leased"
                && job.attempt == next_attempt
                && job.worker_id.as_deref() == Some(actor.user_id.as_str())
                && job.lease_token_digest.as_deref() == Some(lease_digest.as_str())
                && job.lease_expires_at_ms == Some(lease_expires_at_ms)
        }) {
            match command_replay(
                &database,
                &authority_fence,
                &tenant_id,
                &idempotency_key,
                "native_job_claim",
                &digest,
            )
            .await?
            {
                CommandReplay::Stored { status, json } => {
                    return stored_json_response(status, &json);
                }
                CommandReplay::Conflict | CommandReplay::New => {
                    return Err(Error::RustError(
                        "worker claim lost its idempotency reservation".into(),
                    ));
                }
            }
        }
        match command_replay(
            &database,
            &authority_fence,
            &tenant_id,
            &idempotency_key,
            "native_job_claim",
            &digest,
        )
        .await?
        {
            CommandReplay::Stored { status, json } => {
                return stored_json_response(status, &json);
            }
            CommandReplay::Conflict => {
                return failure_response(
                    idempotency_conflict_failure(),
                    request_id,
                    config.production(),
                );
            }
            CommandReplay::New => {}
        }
    }
    failure_response(
        ApiFailure::new(
            409,
            "claim_conflict",
            "The media job was claimed concurrently.",
            true,
        ),
        request_id,
        config.production(),
    )
}

async fn native_job_source_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    ordinal: u16,
    request_id: &str,
) -> Result<Response> {
    let head_only = request.method() == Method::Head;
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let lease_digest = digest_credential(&worker_lease_token_header(request)?);
    let now = current_time_ms()?;
    let Some(job) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !active_worker_lease(&job, actor, &lease_digest, now) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if job.cancel_requested != 0 {
        return failure_response(worker_cancelled_failure(), request_id, config.production());
    }
    let loaded_sources = load_worker_sources(&database, &tenant_id, job_id).await?;
    let sources = match validated_worker_sources(&tenant_id, &job.profile, loaded_sources) {
        Ok(sources) => sources,
        Err(_) => {
            return failure_response(
                ApiFailure::new(
                    409,
                    "source_not_ready",
                    "The current source authority is incomplete.",
                    true,
                ),
                request_id,
                config.production(),
            );
        }
    };
    let Some(source) = sources.get(usize::from(ordinal)) else {
        return failure_response(
            ApiFailure::new(
                409,
                "source_not_ready",
                "The source object is unavailable.",
                true,
            ),
            request_id,
            config.production(),
        );
    };
    if ordinal == 0
        && (source.video_id != job.video_id || source.source_version != job.source_version)
    {
        return Err(Error::RustError(
            "worker primary source authority is inconsistent".into(),
        ));
    }
    let source_bytes = u64::try_from(source.bytes)
        .ok()
        .filter(|bytes| (1..=MAX_SINGLE_UPLOAD_BYTES).contains(bytes))
        .ok_or_else(|| Error::RustError("worker source size is invalid".into()))?;
    let checksum_text = source.checksum_sha256.as_str();
    let checksum = parse_sha256(checksum_text)
        .ok_or_else(|| Error::RustError("worker source checksum is invalid".into()))?;
    if !supported_native_source_content_type(&source.content_type) {
        return failure_response(
            ApiFailure::new(
                409,
                "source_invalid",
                "The source manifest is invalid.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    if !valid_private_object_key(&source.object_key, &tenant_id, &source.video_id) {
        return failure_response(
            ApiFailure::new(
                409,
                "source_invalid",
                "The source manifest is invalid.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let Some(tenant_contract) = storage_tenant(&tenant_id) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(source_authority) = governed_object(
        &database,
        tenant_contract,
        &source.object_key,
        &actor.user_id,
    )
    .await
    .map_err(|()| Error::RustError("storage authority is unavailable".into()))?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if source_authority.object_id().as_str() != source.object_key
        || source_authority.role() != GovernedObjectRole::Source
        || source_authority.state() != GovernedObjectState::Active
        || source_authority.malware() != MalwareDisposition::Clean
        || source_authority.immutable_revision()
            != u64::try_from(source.source_version)
                .map_err(|_| Error::RustError("worker source version is invalid".into()))?
        || source_authority.size().get() != source_bytes
        || source_authority.checksum().as_str() != checksum_text
    {
        return failure_response(
            ApiFailure::new(
                409,
                "source_not_ready",
                "The current source authority changed.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let governance =
        storage_governance_runtime::governance_service(env, &storage_origin(config))
            .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    if let Err(error) = governance.authorize(
        CorrelationId::new(),
        StorageAccessRequest {
            actor: StorageActor::Service {
                tenant_id: tenant_contract,
                purpose: frame_domain::StorageServicePurpose::MediaProcessor,
            },
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::MediaTransformation,
            object: &source_authority,
            now: storage_timestamp(now)
                .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?,
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        },
    ) {
        return storage_policy_error(error, request_id, config.production());
    }
    storage_governance_runtime::managed_media_policy(env)
        .and_then(|policy| {
            policy
                .authorize(tenant_contract, &source_authority)
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        })
        .map_err(|_| Error::RustError("managed media is disabled".into()))?;
    let bucket = env.bucket("RECORDINGS")?;
    let Some(head) = bucket.head(&source.object_key).await? else {
        return failure_response(
            ApiFailure::new(
                409,
                "source_not_ready",
                "The source object is unavailable.",
                true,
            ),
            request_id,
            config.production(),
        );
    };
    let metadata = head.http_metadata();
    if head.size() != source_bytes
        || head.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || metadata.content_type.as_deref() != Some(source.content_type.as_str())
        || metadata.content_encoding.is_some()
    {
        return failure_response(
            ApiFailure::new(
                409,
                "source_invalid",
                "The source object failed verification.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let response = if head_only {
        Response::empty()?
    } else {
        let object = bucket
            .get(&source.object_key)
            .execute()
            .await?
            .filter(|object| {
                object.size() == source_bytes
                    && object.checksum().sha256.as_deref() == Some(checksum.as_slice())
            })
            .ok_or_else(|| Error::RustError("worker source changed during transport".into()))?;
        let body = object
            .body()
            .ok_or_else(|| Error::RustError("worker source body is unavailable".into()))?
            .response_body()?;
        Response::from_body(body)?
    };
    let mut response = response.with_status(200);
    let headers = response.headers_mut();
    headers.set("content-length", &source_bytes.to_string())?;
    headers.set("content-type", &source.content_type)?;
    headers.set("content-disposition", "attachment")?;
    headers.set("x-content-sha256", checksum_text)?;
    Ok(response)
}

async fn native_job_output_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &mut Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    request_id: &str,
) -> Result<Response> {
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let lease_digest = digest_credential(&worker_lease_token_header(request)?);
    let now = current_time_ms()?;
    let Some(job) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !active_worker_lease(&job, actor, &lease_digest, now) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if job.cancel_requested != 0 {
        return failure_response(worker_cancelled_failure(), request_id, config.production());
    }
    if !valid_worker_output_key(&job, &tenant_id) {
        return failure_response(
            ApiFailure::new(
                409,
                "output_invalid",
                "The output manifest is invalid.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let source_version = u32::try_from(job.source_version)
        .map_err(|_| Error::RustError("worker source version is invalid".into()))?;
    let Some(source) =
        load_source_object(&database, &tenant_id, &job.video_id, source_version).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(tenant_contract) = storage_tenant(&tenant_id) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(source_authority) = governed_object(
        &database,
        tenant_contract,
        &source.object_key,
        &actor.user_id,
    )
    .await
    .map_err(|()| Error::RustError("storage authority is unavailable".into()))?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| Error::RustError("validated worker output length is unavailable".into()))?;
    let content_type = request
        .headers()
        .get("content-type")?
        .ok_or_else(|| Error::RustError("validated worker output type is unavailable".into()))?;
    let Some(output_contract) = native_output_contract(&job.profile, &content_type) else {
        return failure_response(
            ApiFailure::new(
                422,
                "profile_unavailable",
                "The media profile output is unavailable.",
                false,
            ),
            request_id,
            config.production(),
        );
    };
    if content_length > output_contract.max_bytes {
        return failure_response(
            invalid_body_failure("invalid_output_manifest"),
            request_id,
            config.production(),
        );
    }
    let checksum_text = request
        .headers()
        .get("x-content-sha256")?
        .filter(|value| contracts::valid_sha256(value))
        .ok_or_else(|| Error::RustError("validated worker checksum is unavailable".into()))?;
    let checksum = parse_sha256(&checksum_text)
        .ok_or_else(|| Error::RustError("validated worker checksum is invalid".into()))?;
    let output_authority = GovernedObject::new(
        tenant_contract,
        GovernedObjectId::parse(job.output_object_key.clone())
            .map_err(|_| Error::RustError("worker output identity is invalid".into()))?,
        output_contract.governed_role,
        ObjectVisibility::Private,
        GovernedObjectState::Active,
        MalwareDisposition::Clean,
        1,
        1,
        ChecksumSha256::parse(checksum_text.clone())
            .map_err(|_| Error::RustError("worker output checksum is invalid".into()))?,
        ByteSize::new(content_length)
            .map_err(|_| Error::RustError("worker output length is invalid".into()))?,
        None,
    )
    .map_err(|_| Error::RustError("worker output authority is invalid".into()))?;
    let media_policy = storage_governance_runtime::managed_media_policy(env)
        .map_err(|_| Error::RustError("managed media is disabled".into()))?;
    let managed_input = media_policy
        .authorize(tenant_contract, &source_authority)
        .map_err(|_| Error::RustError("managed media source is denied".into()))?;
    media_policy
        .authorize_output(
            &managed_input,
            &output_authority,
            &ChecksumSha256::digest_bytes(job.profile.as_bytes()),
        )
        .map_err(|_| Error::RustError("managed media output is denied".into()))?;
    let governance =
        storage_governance_runtime::governance_service(env, &storage_origin(config))
            .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    if let Err(error) = governance.authorize(
        CorrelationId::new(),
        StorageAccessRequest {
            actor: StorageActor::Service {
                tenant_id: tenant_contract,
                purpose: frame_domain::StorageServicePurpose::MediaProcessor,
            },
            operation: StorageOperation::WriteImmutable,
            surface: StorageAccessSurface::MediaTransformation,
            object: &output_authority,
            now: storage_timestamp(now)
                .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?,
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        },
    ) {
        return storage_policy_error(error, request_id, config.production());
    }
    let Some(integration) = active_r2_integration(&database, &tenant_id).await? else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    if !integration.supports_single_put() {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let bucket = env.bucket("RECORDINGS")?;
    let candidate_key = native_output_candidate_key(&job, &tenant_id, &checksum_text)
        .ok_or_else(|| Error::RustError("worker output candidate is invalid".into()))?;
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-output-reserve:{job_id}:{}", job.attempt),
            now,
            vec![database
                .prepare(
                    "INSERT INTO media_native_output_staging_v1(job_id, attempt, organization_id, \
                       video_id, worker_id, lease_token_digest, staging_object_key, final_object_key, \
                       bytes, checksum_sha256, content_type, state, provider_etag, created_at_ms, updated_at_ms) \
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'receiving', NULL, ?12, ?12 \
                     FROM media_jobs j WHERE j.id = ?1 AND j.organization_id = ?3 \
                       AND j.video_id = ?4 AND j.attempt = ?2 AND j.worker_id = ?5 \
                       AND j.lease_token_digest = ?6 AND j.output_object_key = ?8 \
                       AND j.selected_executor = 'native_gstreamer' AND j.cancel_requested = 0 \
                       AND j.state IN ('leased','running') AND j.lease_expires_at_ms > ?12 \
                     ON CONFLICT(job_id, attempt) DO NOTHING",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_f64(job.attempt as f64),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&job.video_id),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_str(&candidate_key),
                    JsValue::from_str(&job.output_object_key),
                    JsValue::from_f64(content_length as f64),
                    JsValue::from_str(&checksum_text),
                    JsValue::from_str(&content_type),
                    JsValue::from_f64(now as f64),
                ])?],
        )
        .await?,
    )?;
    let staged_row = database
        .prepare(
            "SELECT job_id, attempt, organization_id, video_id, worker_id, lease_token_digest, \
                    staging_object_key, final_object_key, bytes, checksum_sha256, content_type, \
                    state, provider_etag FROM media_native_output_staging_v1 \
             WHERE job_id = ?1 AND attempt = ?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_f64(job.attempt as f64),
        ])?
        .first::<NativeOutputStagingRow>(None)
        .await?;
    let Some(staged_row) = staged_row.filter(|row| {
        row.job_id == job_id
            && row.attempt == job.attempt
            && row.organization_id == tenant_id
            && row.video_id == job.video_id
            && row.worker_id == actor.user_id
            && row.lease_token_digest == lease_digest
            && row.staging_object_key == candidate_key
            && row.final_object_key == job.output_object_key
            && row.bytes == content_length as i64
            && row.checksum_sha256 == checksum_text
            && row.content_type == content_type
            && matches!(row.state.as_str(), "receiving" | "staged")
    }) else {
        return failure_response(
            ApiFailure::new(
                409,
                "output_conflict",
                "The immutable output staging reservation conflicts.",
                false,
            ),
            request_id,
            config.production(),
        );
    };
    let staging_metadata = HashMap::from([
        ("executor".into(), "native-gstreamer-v1".into()),
        ("staging-contract".into(), "native-output-v1".into()),
        ("job-id".into(), job_id.into()),
        ("attempt".into(), job.attempt.to_string()),
    ]);
    let object = if let Some(existing) = bucket.head(&candidate_key).await? {
        existing
    } else {
        let stream = FixedLengthStream::wrap(request.stream()?, content_length);
        match bucket
            .put(&candidate_key, stream)
            .http_metadata(HttpMetadata {
                content_type: Some(content_type.clone()),
                content_disposition: Some("inline".into()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .custom_metadata(staging_metadata.clone())
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
        {
            Some(created) => created,
            None => bucket
                .head(&candidate_key)
                .await?
                .ok_or_else(|| Error::RustError("worker output write conflicted".into()))?,
        }
    };
    let metadata = object.http_metadata();
    let custom_metadata = object.custom_metadata()?;
    if object.size() != content_length
        || object.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || metadata.content_type.as_deref() != Some(content_type.as_str())
        || metadata.content_encoding.is_some()
        || metadata.cache_control.as_deref() != Some("private, no-store")
        || custom_metadata != staging_metadata
    {
        return failure_response(
            ApiFailure::new(
                409,
                "output_conflict",
                "The immutable output object does not match this attempt.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let output_etag = object.etag();
    let staged_at = current_time_ms()?;
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-output-stage:{job_id}:{}", job.attempt),
            staged_at,
            vec![
                database
                    .prepare(
                        "UPDATE media_native_output_staging_v1 SET state = 'staged', \
                       provider_etag = ?3, updated_at_ms = ?4 \
                     WHERE job_id = ?1 AND attempt = ?2 AND state IN ('receiving','staged') \
                       AND (provider_etag IS NULL OR provider_etag = ?3) \
                       AND staging_object_key = ?5 AND bytes = ?6 AND checksum_sha256 = ?7 \
                       AND content_type = ?8",
                    )
                    .bind(&[
                        JsValue::from_str(job_id),
                        JsValue::from_f64(job.attempt as f64),
                        JsValue::from_str(&output_etag),
                        JsValue::from_f64(staged_at as f64),
                        JsValue::from_str(&candidate_key),
                        JsValue::from_f64(content_length as f64),
                        JsValue::from_str(&checksum_text),
                        JsValue::from_str(&content_type),
                    ])?,
            ],
        )
        .await?,
    )?;
    let staged = database
        .prepare(
            "SELECT job_id, attempt, organization_id, video_id, worker_id, lease_token_digest, \
                    staging_object_key, final_object_key, bytes, checksum_sha256, content_type, \
                    state, provider_etag FROM media_native_output_staging_v1 \
             WHERE job_id = ?1 AND attempt = ?2 AND state = 'staged' \
               AND provider_etag = ?3 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_f64(job.attempt as f64),
            JsValue::from_str(&output_etag),
        ])?
        .first::<NativeOutputStagingRow>(None)
        .await?;
    if staged.is_none()
        || staged_row
            .provider_etag
            .as_deref()
            .is_some_and(|etag| etag != output_etag)
    {
        return failure_response(
            ApiFailure::new(
                409,
                "output_conflict",
                "The immutable output staging record did not commit.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let response = WorkerOutputResponse {
        schema_version: API_SCHEMA_VERSION,
        job_id: job_id.into(),
        accepted: true,
        bytes: content_length,
        checksum_sha256: checksum_text,
        content_type,
    };
    json_response(&response, 200, None)
}

async fn native_job_heartbeat_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    body: WorkerHeartbeatRequest,
    request_id: &str,
) -> Result<Response> {
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let lease_digest = digest_credential(&worker_lease_token_header(request)?);
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest(
        "native_job_heartbeat",
        &serde_json::json!({
            "job_id": job_id,
            "body": &body,
            "lease_token_digest": lease_digest,
        }),
    )
    .map_err(|()| Error::RustError("worker heartbeat could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_heartbeat",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let now = current_time_ms()?;
    let Some(existing) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !worker_identity_matches(&existing, actor, &lease_digest) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if existing.cancel_requested != 0 || existing.state == "cancelled" {
        let response = existing
            .private_response(false)
            .ok_or_else(|| Error::RustError("worker job response is invalid".into()))?;
        return json_response(&response, 200, None);
    }
    if !active_worker_lease(&existing, actor, &lease_digest, now) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let next_revision = existing
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::RustError("worker heartbeat revision overflowed".into()))?;
    let lease_expires_at_ms = now
        .checked_add(NATIVE_LEASE_MS)
        .ok_or_else(|| Error::RustError("worker heartbeat expiry overflowed".into()))?;
    let mut next = existing.clone();
    next.state = "running".into();
    next.revision = next_revision;
    next.lease_expires_at_ms = Some(lease_expires_at_ms);
    let response = next
        .private_response(false)
        .ok_or_else(|| Error::RustError("worker heartbeat response is invalid".into()))?;
    let response_json = serde_json::to_string(&response).map_err(|_| {
        Error::RustError("worker heartbeat response could not be serialized".into())
    })?;
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "attempt": existing.attempt,
        "state": "running",
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let reservation_id = new_id();
    let statements = vec![
        worker_command_reservation(
            &database,
            &tenant_id,
            &idempotency_key,
            "native_job_heartbeat",
            &digest,
            &reservation_id,
            now,
        )?,
        database
            .prepare(
                "UPDATE media_jobs SET state = 'running', heartbeat_at_ms = ?5, \
                   lease_expires_at_ms = ?6, updated_at_ms = ?5, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND revision = ?3 AND attempt = ?4 \
                   AND state IN ('leased', 'running') AND cancel_requested = 0 \
                   AND worker_id = ?7 AND lease_token_digest = ?8 \
                   AND lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms > ?5 \
                   AND (?9 = -1 OR EXISTS (SELECT 1 FROM authority_state a WHERE a.singleton = 1 \
                     AND a.epoch = ?9 AND a.authority = 'd1' \
                     AND a.phase IN ('d1_authoritative', 'finalized'))) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?10 \
                       AND c.command_type = 'native_job_heartbeat' AND c.request_digest = ?11 \
                       AND c.reservation_id = ?12 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(existing.revision as f64),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(lease_expires_at_ms as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
        database
            .prepare(
                "UPDATE command_idempotency SET response_status = 200, response_json = ?4 \
                 WHERE organization_id = ?1 AND idempotency_key = ?2 \
                   AND command_type = 'native_job_heartbeat' AND request_digest = ?3 \
                   AND reservation_id = ?5 AND response_status IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?6 \
                     AND j.organization_id = ?1 AND j.revision = ?7 \
                     AND j.worker_id = ?8 AND j.lease_token_digest = ?9)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(job_id),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                   event_type, deduplication_key, payload_json, state, attempt, available_at_ms, \
                   created_at_ms, event_sequence, event_fingerprint, payload_schema_version, \
                   payload_checksum, revision) \
                 SELECT ?1, ?2, 'media_job', ?3, 'media.job.heartbeat', ?4, ?5, 'pending', 0, ?6, ?6, \
                        0, ?13, 1, ?14, 0 \
                 FROM media_jobs j WHERE j.id = ?3 AND j.organization_id = ?2 AND j.revision = ?7 \
                   AND j.worker_id = ?8 AND j.lease_token_digest = ?9 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?10 \
                       AND c.command_type = 'native_job_heartbeat' AND c.request_digest = ?11 \
                       AND c.reservation_id = ?12 AND c.response_status = 200) \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-heartbeat:{job_id}:{next_revision}")),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
        worker_command_reservation_cleanup(
            &database,
            &tenant_id,
            &idempotency_key,
            &reservation_id,
        )?,
    ];
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-heartbeat:{job_id}:{next_revision}"),
            now,
            statements,
        )
        .await?,
    )?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_heartbeat",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(current) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if current.revision != next_revision
        || current.lease_expires_at_ms != Some(lease_expires_at_ms)
        || !worker_identity_matches(&current, actor, &lease_digest)
    {
        if current.cancel_requested != 0 {
            let response = current
                .private_response(false)
                .ok_or_else(|| Error::RustError("worker job response is invalid".into()))?;
            return json_response(&response, 200, None);
        }
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    json_response(&response, 200, None)
}

async fn native_job_progress_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    body: WorkerProgressRequest,
    request_id: &str,
) -> Result<Response> {
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let lease_digest = digest_credential(&worker_lease_token_header(request)?);
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest(
        "native_job_progress",
        &serde_json::json!({
            "job_id": job_id,
            "body": &body,
            "lease_token_digest": lease_digest,
        }),
    )
    .map_err(|()| Error::RustError("worker progress could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_progress",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let now = current_time_ms()?;
    let Some(existing) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !worker_identity_matches(&existing, actor, &lease_digest) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if existing.cancel_requested != 0 {
        let response = existing
            .private_response(false)
            .ok_or_else(|| Error::RustError("worker job response is invalid".into()))?;
        return json_response(&response, 200, None);
    }
    if !active_worker_lease(&existing, actor, &lease_digest, now) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let progress = i64::from(body.progress_basis_points);
    if existing
        .progress_basis_points
        .is_some_and(|current| current > progress)
    {
        return failure_response(
            ApiFailure::new(
                409,
                "progress_regression",
                "Media job progress cannot move backwards.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let next_revision = existing
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::RustError("worker progress revision overflowed".into()))?;
    let lease_expires_at_ms = now
        .checked_add(NATIVE_LEASE_MS)
        .ok_or_else(|| Error::RustError("worker progress expiry overflowed".into()))?;
    let mut next = existing.clone();
    next.state = "running".into();
    next.revision = next_revision;
    next.progress_basis_points = Some(progress);
    next.lease_expires_at_ms = Some(lease_expires_at_ms);
    let response = next
        .private_response(false)
        .ok_or_else(|| Error::RustError("worker progress response is invalid".into()))?;
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("worker progress response could not be serialized".into()))?;
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "attempt": existing.attempt,
        "progress_basis_points": progress,
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let reservation_id = new_id();
    let statements = vec![
        worker_command_reservation(
            &database,
            &tenant_id,
            &idempotency_key,
            "native_job_progress",
            &digest,
            &reservation_id,
            now,
        )?,
        database
            .prepare(
                "UPDATE media_jobs SET state = 'running', progress_basis_points = ?5, \
                   heartbeat_at_ms = ?6, lease_expires_at_ms = ?7, updated_at_ms = ?6, \
                   revision = revision + 1 WHERE id = ?1 AND organization_id = ?2 \
                   AND revision = ?3 AND attempt = ?4 AND state IN ('leased', 'running') \
                   AND cancel_requested = 0 AND worker_id = ?8 AND lease_token_digest = ?9 \
                   AND lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms > ?6 \
                   AND (progress_basis_points IS NULL OR progress_basis_points <= ?5) \
                   AND (?10 = -1 OR EXISTS (SELECT 1 FROM authority_state a WHERE a.singleton = 1 \
                     AND a.epoch = ?10 AND a.authority = 'd1' \
                     AND a.phase IN ('d1_authoritative', 'finalized'))) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?11 \
                       AND c.command_type = 'native_job_progress' AND c.request_digest = ?12 \
                       AND c.reservation_id = ?13 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(existing.revision as f64),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_f64(progress as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(lease_expires_at_ms as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
        database
            .prepare(
                "UPDATE command_idempotency SET response_status = 200, response_json = ?4 \
                 WHERE organization_id = ?1 AND idempotency_key = ?2 \
                   AND command_type = 'native_job_progress' AND request_digest = ?3 \
                   AND reservation_id = ?5 AND response_status IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?6 \
                     AND j.organization_id = ?1 AND j.revision = ?7 \
                     AND j.progress_basis_points = ?8 AND j.worker_id = ?9 \
                     AND j.lease_token_digest = ?10)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(job_id),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_f64(progress as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                   event_type, deduplication_key, payload_json, state, attempt, available_at_ms, \
                   created_at_ms, event_sequence, event_fingerprint, payload_schema_version, \
                   payload_checksum, revision) \
                 SELECT ?1, ?2, 'media_job', ?3, 'media.job.progressed', ?4, ?5, 'pending', 0, ?6, ?6, \
                        0, ?14, 1, ?15, 0 \
                 FROM media_jobs j WHERE j.id = ?3 AND j.organization_id = ?2 AND j.revision = ?7 \
                   AND j.progress_basis_points = ?8 AND j.worker_id = ?9 AND j.lease_token_digest = ?10 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?11 \
                       AND c.command_type = 'native_job_progress' AND c.request_digest = ?12 \
                       AND c.reservation_id = ?13 AND c.response_status = 200) \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-progress:{job_id}:{next_revision}")),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_f64(progress as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
        worker_command_reservation_cleanup(
            &database,
            &tenant_id,
            &idempotency_key,
            &reservation_id,
        )?,
    ];
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-progress:{job_id}:{next_revision}"),
            now,
            statements,
        )
        .await?,
    )?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_progress",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(current) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if current.revision != next_revision
        || current.progress_basis_points != Some(progress)
        || !worker_identity_matches(&current, actor, &lease_digest)
    {
        if current.cancel_requested != 0 {
            let response = current
                .private_response(false)
                .ok_or_else(|| Error::RustError("worker job response is invalid".into()))?;
            return json_response(&response, 200, None);
        }
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    json_response(&response, 200, None)
}

async fn native_job_complete_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    body: WorkerCompleteRequest,
    request_id: &str,
) -> Result<Response> {
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let Some(completed) = body.output() else {
        return failure_response(
            invalid_body_failure("invalid_output_manifest"),
            request_id,
            config.production(),
        );
    };
    let lease_digest = digest_credential(&worker_lease_token_header(request)?);
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest(
        "native_job_complete",
        &serde_json::json!({
            "job_id": job_id,
            "body": &body,
            "lease_token_digest": lease_digest,
        }),
    )
    .map_err(|()| Error::RustError("worker completion could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_complete",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let now = current_time_ms()?;
    let Some(existing) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !worker_identity_matches(&existing, actor, &lease_digest) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if existing.state == "succeeded" {
        if worker_manifest_matches(&database, &tenant_id, &existing, &body).await? {
            let response = existing
                .private_response(false)
                .ok_or_else(|| Error::RustError("worker job response is invalid".into()))?;
            return json_response(&response, 200, None);
        }
        return failure_response(
            ApiFailure::new(
                409,
                "output_conflict",
                "The completed output is immutable.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    if existing.cancel_requested != 0 || existing.state == "cancelled" {
        return failure_response(worker_cancelled_failure(), request_id, config.production());
    }
    if matches!(existing.state.as_str(), "failed")
        || !active_worker_lease(&existing, actor, &lease_digest, now)
    {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let Some(output_contract) = native_output_contract(&existing.profile, &completed.content_type)
    else {
        return failure_response(
            ApiFailure::new(
                422,
                "profile_unavailable",
                "The media profile output is unavailable.",
                false,
            ),
            request_id,
            config.production(),
        );
    };
    if completed.bytes > output_contract.max_bytes {
        return failure_response(
            invalid_body_failure("invalid_output_manifest"),
            request_id,
            config.production(),
        );
    }
    if !valid_worker_output_key(&existing, &tenant_id) {
        return failure_response(
            ApiFailure::new(
                409,
                "output_invalid",
                "The output manifest is invalid.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let checksum = parse_sha256(&completed.checksum_sha256)
        .ok_or_else(|| Error::RustError("validated completion checksum is invalid".into()))?;
    let bucket = env.bucket("RECORDINGS")?;
    let candidate_key =
        native_output_candidate_key(&existing, &tenant_id, &completed.checksum_sha256)
            .ok_or_else(|| Error::RustError("worker output candidate is invalid".into()))?;
    let Some(staged_output) = bucket.head(&candidate_key).await? else {
        return failure_response(
            ApiFailure::new(
                409,
                "output_not_ready",
                "The output object is unavailable.",
                true,
            ),
            request_id,
            config.production(),
        );
    };
    let attempt_text = existing.attempt.to_string();
    let metadata = staged_output.http_metadata();
    let staged_custom_metadata = staged_output.custom_metadata()?;
    if staged_output.size() != completed.bytes
        || staged_output.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || metadata.content_type.as_deref() != Some(completed.content_type.as_str())
        || metadata.content_encoding.is_some()
        || metadata.cache_control.as_deref() != Some("private, no-store")
        || staged_custom_metadata.get("executor").map(String::as_str) != Some("native-gstreamer-v1")
        || staged_custom_metadata
            .get("staging-contract")
            .map(String::as_str)
            != Some("native-output-v1")
        || staged_custom_metadata.get("job-id").map(String::as_str) != Some(job_id)
        || staged_custom_metadata.get("attempt").map(String::as_str) != Some(attempt_text.as_str())
    {
        return failure_response(
            ApiFailure::new(
                409,
                "output_invalid",
                "The output object failed verification.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let staging_etag = staged_output.etag();
    let staged_row = database
        .prepare(
            "SELECT job_id, attempt, organization_id, video_id, worker_id, lease_token_digest, \
                    staging_object_key, final_object_key, bytes, checksum_sha256, content_type, \
                    state, provider_etag FROM media_native_output_staging_v1 \
             WHERE job_id = ?1 AND attempt = ?2 AND organization_id = ?3 \
               AND worker_id = ?4 AND lease_token_digest = ?5 AND state = 'staged' \
               AND staging_object_key = ?6 AND final_object_key = ?7 AND bytes = ?8 \
               AND checksum_sha256 = ?9 AND content_type = ?10 AND provider_etag = ?11 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_f64(existing.attempt as f64),
            JsValue::from_str(&tenant_id),
            JsValue::from_str(&actor.user_id),
            JsValue::from_str(&lease_digest),
            JsValue::from_str(&candidate_key),
            JsValue::from_str(&existing.output_object_key),
            JsValue::from_f64(completed.bytes as f64),
            JsValue::from_str(&completed.checksum_sha256),
            JsValue::from_str(&completed.content_type),
            JsValue::from_str(&staging_etag),
        ])?
        .first::<NativeOutputStagingRow>(None)
        .await?;
    if staged_row.is_none() {
        return failure_response(
            ApiFailure::new(
                409,
                "output_not_ready",
                "The output staging record is unavailable.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let native_probe = if existing.profile == "probe_v1" {
        let Some(probe_object) = bucket.get(&candidate_key).execute().await? else {
            return failure_response(
                ApiFailure::new(
                    409,
                    "output_not_ready",
                    "The probe output object is unavailable.",
                    true,
                ),
                request_id,
                config.production(),
            );
        };
        let probe_metadata = probe_object.http_metadata();
        if probe_object.size() != completed.bytes
            || probe_object.checksum().sha256.as_deref() != Some(checksum.as_slice())
            || probe_metadata.content_type.as_deref() != Some("application/json")
            || probe_metadata.content_encoding.is_some()
        {
            return failure_response(
                invalid_body_failure("invalid_probe_manifest"),
                request_id,
                config.production(),
            );
        }
        let probe_bytes = probe_object
            .body()
            .ok_or_else(|| Error::RustError("probe output body is unavailable".into()))?
            .bytes()
            .await?;
        if probe_bytes.len() as u64 != completed.bytes
            || ChecksumSha256::digest_bytes(&probe_bytes).as_str() != completed.checksum_sha256
        {
            return failure_response(
                invalid_body_failure("invalid_probe_manifest"),
                request_id,
                config.production(),
            );
        }
        let source_version = u32::try_from(existing.source_version)
            .map_err(|_| Error::RustError("probe source version is invalid".into()))?;
        let Some(source) =
            load_source_object(&database, &tenant_id, &existing.video_id, source_version).await?
        else {
            return failure_response(not_found_failure(), request_id, config.production());
        };
        let Some(source_checksum) = source.checksum_sha256.as_deref() else {
            return failure_response(
                invalid_body_failure("invalid_probe_source"),
                request_id,
                config.production(),
            );
        };
        if !contracts::valid_sha256(source_checksum) || source.bytes <= 0 {
            return failure_response(
                invalid_body_failure("invalid_probe_source"),
                request_id,
                config.production(),
            );
        }
        let verified =
            match media_service_runtime::verify_native_probe_v1(&probe_bytes, &source.content_type)
            {
                Ok(verified) => verified,
                Err(_) => {
                    return failure_response(
                        invalid_body_failure("invalid_probe_manifest"),
                        request_id,
                        config.production(),
                    );
                }
            };
        Some((verified, source))
    } else {
        None
    };
    let source_version = u32::try_from(existing.source_version)
        .map_err(|_| Error::RustError("worker source version is invalid".into()))?;
    let Some(source_manifest) =
        load_source_object(&database, &tenant_id, &existing.video_id, source_version).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let source_checksum_sha256 = source_manifest
        .checksum_sha256
        .as_deref()
        .filter(|value| contracts::valid_sha256(value))
        .ok_or_else(|| Error::RustError("worker source checksum is invalid".into()))?;
    let Some(integration) = active_r2_integration(&database, &tenant_id).await? else {
        return failure_response(
            storage_unavailable_failure(),
            request_id,
            config.production(),
        );
    };
    let execution_seed = database
        .prepare(
            "SELECT e.normalized_profile_sha256, p.source_checksum_sha256, e.attempt \
             FROM media_job_execution_v1 e JOIN media_source_probes_v1 p \
               ON p.organization_id = e.organization_id AND p.video_id = e.video_id \
              AND p.source_version = e.source_version \
             WHERE e.job_id = ?1 AND e.organization_id = ?2 AND e.video_id = ?3 \
               AND e.profile_id = ?4 AND e.selected_executor = 'native_gstreamer' \
               AND e.final_object_key = ?5 AND p.trust = 'verified_native_probe' \
               AND p.state = 'verified' LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_str(&tenant_id),
            JsValue::from_str(&existing.video_id),
            JsValue::from_str(&existing.profile),
            JsValue::from_str(&existing.output_object_key),
        ])?
        .first::<NativeExecutionManifestSeed>(None)
        .await?;
    if execution_seed
        .as_ref()
        .is_some_and(|seed| seed.attempt > existing.attempt)
    {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let normalized_profile_sha256 = execution_seed
        .as_ref()
        .map_or_else(
            || request_digest("media_profile_v1", &existing.profile),
            |seed| Ok(seed.normalized_profile_sha256.clone()),
        )
        .map_err(|()| Error::RustError("worker profile identity is invalid".into()))?;
    let final_key = existing.output_object_key.clone();
    let final_metadata = HashMap::from([
        ("executor".into(), "native-gstreamer-v1".into()),
        ("source-sha256".into(), source_checksum_sha256.to_owned()),
        ("profile-sha256".into(), normalized_profile_sha256.clone()),
        ("job-id".into(), job_id.into()),
        ("attempt".into(), attempt_text.clone()),
    ]);
    let publish_check_now = current_time_ms()?;
    let current = load_worker_job(&database, &tenant_id, job_id).await?;
    if !current.as_ref().is_some_and(|current| {
        current.revision == existing.revision
            && current.attempt == existing.attempt
            && current.cancel_requested == 0
            && active_worker_lease(current, actor, &lease_digest, publish_check_now)
    }) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let final_output = if let Some(final_output) = bucket.head(&final_key).await? {
        final_output
    } else {
        let staged =
            bucket.get(&candidate_key).execute().await?.ok_or_else(|| {
                Error::RustError("worker output staging object disappeared".into())
            })?;
        let stream = staged
            .body()
            .ok_or_else(|| Error::RustError("worker output staging body is unavailable".into()))?
            .stream()?;
        let stream = FixedLengthStream::wrap(stream, completed.bytes);
        match bucket
            .put(&final_key, stream)
            .http_metadata(HttpMetadata {
                content_type: Some(completed.content_type.clone()),
                content_disposition: Some("inline".into()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .custom_metadata(final_metadata.clone())
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
        {
            Some(created) => created,
            None => bucket
                .head(&final_key)
                .await?
                .ok_or_else(|| Error::RustError("worker output publication conflicted".into()))?,
        }
    };
    let published_metadata = final_output.http_metadata();
    if final_output.size() != completed.bytes
        || final_output.checksum().sha256.as_deref() != Some(checksum.as_slice())
        || published_metadata.content_type.as_deref() != Some(completed.content_type.as_str())
        || published_metadata.content_encoding.is_some()
        || published_metadata.cache_control.as_deref() != Some("private, no-store")
        || final_output.custom_metadata()? != final_metadata
    {
        return failure_response(
            ApiFailure::new(
                409,
                "output_conflict",
                "The immutable published output conflicts.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    let publish_commit_check_now = current_time_ms()?;
    let current = load_worker_job(&database, &tenant_id, job_id).await?;
    if !current.as_ref().is_some_and(|current| {
        current.revision == existing.revision
            && current.attempt == existing.attempt
            && current.cancel_requested == 0
            && active_worker_lease(current, actor, &lease_digest, publish_commit_check_now)
    }) {
        bucket.delete(&candidate_key).await?;
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let output_etag = final_output.etag();
    let next_revision = existing
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::RustError("worker completion revision overflowed".into()))?;
    let mut next = existing.clone();
    next.state = "succeeded".into();
    next.revision = next_revision;
    next.progress_basis_points = Some(10_000);
    next.lease_expires_at_ms = None;
    next.output_object_key.clone_from(&final_key);
    let response = next
        .private_response(false)
        .ok_or_else(|| Error::RustError("worker completion response is invalid".into()))?;
    let response_json = serde_json::to_string(&response).map_err(|_| {
        Error::RustError("worker completion response could not be serialized".into())
    })?;
    let storage_object_id = new_id();
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "video_id": existing.video_id,
        "attempt": existing.attempt,
        "role": output_contract.manifest_role,
        "state": "succeeded",
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let reservation_id = new_id();
    let execution_manifest = execution_seed.as_ref().map(|seed| {
        let manifest_json = serde_json::json!({
            "schema_version": 1,
            "job_id": job_id,
            "executor": "native_gstreamer",
            "source_checksum_sha256": seed.source_checksum_sha256,
            "normalized_profile_sha256": seed.normalized_profile_sha256,
            "object_key": final_key,
            "object_checksum_sha256": completed.checksum_sha256,
            "bytes": completed.bytes,
            "content_type": completed.content_type,
        })
        .to_string();
        let manifest_digest = ChecksumSha256::digest_bytes(manifest_json.as_bytes())
            .as_str()
            .to_owned();
        (manifest_digest, manifest_json)
    });
    let mut statements = vec![
        worker_command_reservation(
            &database,
            &tenant_id,
            &idempotency_key,
            "native_job_complete",
            &digest,
            &reservation_id,
            now,
        )?,
        database
            .prepare(
                "UPDATE media_jobs SET state = 'succeeded', progress_basis_points = 10000, \
                   error_code = NULL, error_class = NULL, lease_expires_at_ms = NULL, \
                   heartbeat_at_ms = ?5, output_object_key = ?8, updated_at_ms = ?5, \
                   revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND revision = ?3 AND attempt = ?4 \
                   AND state IN ('leased', 'running') AND cancel_requested = 0 \
                   AND worker_id = ?6 AND lease_token_digest = ?7 \
                   AND lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms > ?5 \
                   AND NOT EXISTS (SELECT 1 FROM object_manifests m WHERE m.object_key = ?8 \
                     AND (m.organization_id <> ?2 OR m.video_id <> ?9 OR m.role <> ?20 \
                       OR m.object_version <> ?10 OR m.bytes <> ?11 \
                       OR COALESCE(m.checksum_sha256, '') <> ?12 OR m.content_type <> ?13 \
                       OR COALESCE(m.provider_etag, '') <> ?14 OR m.state <> 'available')) \
                   AND NOT EXISTS (SELECT 1 FROM storage_objects s \
                     WHERE s.integration_id = ?15 AND s.object_key = ?8 \
                       AND (s.organization_id <> ?2 OR COALESCE(s.video_id, '') <> ?9 \
                         OR s.role <> ?20 OR s.object_version <> ?10 \
                         OR s.bytes <> ?11 OR COALESCE(s.checksum_sha256, '') <> ?12 \
                         OR s.content_type <> ?13 OR COALESCE(s.provider_etag, '') <> ?14 \
                         OR s.state <> 'available')) \
                   AND NOT EXISTS (SELECT 1 FROM storage_governed_objects_v1 g \
                     WHERE g.organization_id = ?2 AND g.object_key = ?8 \
                       AND (g.role <> ?21 OR g.visibility <> 'private' \
                         OR g.state <> 'active' OR g.malware_disposition <> 'clean' \
                         OR g.immutable_revision <> ?10 OR g.cache_generation <> 1 \
                         OR g.bytes <> ?11 OR g.checksum_sha256 <> ?12 \
                         OR g.content_type <> ?13)) \
                   AND (?16 = -1 OR EXISTS (SELECT 1 FROM authority_state a WHERE a.singleton = 1 \
                     AND a.epoch = ?16 AND a.authority = 'd1' \
                     AND a.phase IN ('d1_authoritative', 'finalized'))) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?17 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?18 \
                       AND c.reservation_id = ?19 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(existing.revision as f64),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_str(&final_key),
                JsValue::from_str(&existing.video_id),
                JsValue::from_f64(existing.source_version as f64),
                JsValue::from_f64(completed.bytes as f64),
                JsValue::from_str(&completed.checksum_sha256),
                JsValue::from_str(&completed.content_type),
                JsValue::from_str(&output_etag),
                JsValue::from_str(&integration.id),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(output_contract.manifest_role),
                JsValue::from_str(output_contract.governed_role.stable_code()),
            ])?,
        database
            .prepare(
                "UPDATE media_job_attempts SET finished_at_ms = ?3, outcome = 'succeeded', \
                   error_class = NULL WHERE job_id = ?1 AND attempt = ?2 AND outcome IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?1 \
                     AND j.organization_id = ?4 AND j.state = 'succeeded' \
                     AND j.worker_id = ?5 AND j.lease_token_digest = ?6 \
                     AND j.revision = ?7 AND j.output_object_key = ?8) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?4 AND c.idempotency_key = ?9 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?10 \
                       AND c.reservation_id = ?11 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&final_key),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
        database
            .prepare(
                "INSERT INTO object_manifests(object_key, video_id, role, bytes, checksum_sha256, \
                   content_type, created_at_ms, organization_id, object_version, provider_etag, state, updated_at_ms) \
                 SELECT ?1, ?2, ?17, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'available', ?6 \
                 FROM media_jobs j WHERE j.id = ?10 AND j.organization_id = ?7 \
                   AND j.state = 'succeeded' AND j.worker_id = ?11 AND j.lease_token_digest = ?12 \
                   AND j.revision = ?13 AND j.output_object_key = ?1 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?7 AND c.idempotency_key = ?14 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?15 \
                       AND c.reservation_id = ?16 AND c.response_status IS NULL) \
                 ON CONFLICT(object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&final_key),
                JsValue::from_str(&existing.video_id),
                JsValue::from_f64(completed.bytes as f64),
                JsValue::from_str(&completed.checksum_sha256),
                JsValue::from_str(&completed.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(existing.source_version as f64),
                JsValue::from_str(&output_etag),
                JsValue::from_str(job_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(output_contract.manifest_role),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(id, organization_id, integration_id, video_id, object_key, \
                   role, object_version, state, bytes, content_type, checksum_sha256, provider_etag, created_at_ms) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?19, ?6, 'available', ?7, ?8, ?9, ?10, ?11 \
                 FROM media_jobs j WHERE j.id = ?12 AND j.organization_id = ?2 \
                   AND j.state = 'succeeded' AND j.worker_id = ?13 AND j.lease_token_digest = ?14 \
                   AND j.revision = ?15 AND j.output_object_key = ?5 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?16 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?17 \
                       AND c.reservation_id = ?18 AND c.response_status IS NULL) \
                 ON CONFLICT(integration_id, object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(&existing.video_id),
                JsValue::from_str(&final_key),
                JsValue::from_f64(existing.source_version as f64),
                JsValue::from_f64(completed.bytes as f64),
                JsValue::from_str(&completed.content_type),
                JsValue::from_str(&completed.checksum_sha256),
                JsValue::from_str(&output_etag),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(output_contract.manifest_role),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_governed_objects_v1(organization_id, object_key, role, visibility, \
                   state, malware_disposition, immutable_revision, cache_generation, checksum_sha256, \
                   bytes, content_type, retention_until_ms, created_at_ms, updated_at_ms) \
                 SELECT ?1, ?2, ?13, 'private', 'active', 'clean', ?3, 1, ?4, ?5, ?6, NULL, ?7, ?7 \
                 FROM media_jobs j WHERE j.id = ?8 AND j.organization_id = ?1 \
                   AND j.state = 'succeeded' AND j.revision = ?9 AND j.output_object_key = ?2 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?1 AND c.idempotency_key = ?10 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?11 \
                       AND c.reservation_id = ?12 AND c.response_status IS NULL) \
                 ON CONFLICT(organization_id, object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&final_key),
                JsValue::from_f64(existing.source_version as f64),
                JsValue::from_str(&completed.checksum_sha256),
                JsValue::from_f64(completed.bytes as f64),
                JsValue::from_str(&completed.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(output_contract.governed_role.stable_code()),
            ])?,
        database
            .prepare(
                "UPDATE command_idempotency SET response_status = 200, response_json = ?4 \
                 WHERE organization_id = ?1 AND idempotency_key = ?2 \
                   AND command_type = 'native_job_complete' AND request_digest = ?3 \
                   AND reservation_id = ?5 AND response_status IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?6 \
                     AND j.organization_id = ?1 AND j.state = 'succeeded' \
                     AND j.revision = ?7 AND j.worker_id = ?8 AND j.lease_token_digest = ?9 \
                     AND j.output_object_key = ?10) \
                   AND (NOT EXISTS (SELECT 1 FROM media_job_execution_v1 e WHERE e.job_id = ?6) \
                     OR EXISTS (SELECT 1 FROM media_job_execution_v1 e WHERE e.job_id = ?6 \
                       AND e.state = 'succeeded' AND e.selected_executor = 'native_gstreamer' \
                       AND e.manifest_digest IS NOT NULL)) \
                   AND EXISTS (SELECT 1 FROM media_native_output_staging_v1 s \
                     WHERE s.job_id = ?6 AND s.attempt = ?11 AND s.state = 'published' \
                       AND s.staging_object_key = ?12 AND s.final_object_key = ?10 \
                       AND s.checksum_sha256 = ?13 AND s.bytes = ?14 \
                       AND s.content_type = ?15)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(job_id),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_str(&final_key),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_str(&candidate_key),
                JsValue::from_str(&completed.checksum_sha256),
                JsValue::from_f64(completed.bytes as f64),
                JsValue::from_str(&completed.content_type),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                   event_type, deduplication_key, payload_json, state, attempt, available_at_ms, \
                   created_at_ms, event_sequence, event_fingerprint, payload_schema_version, \
                   payload_checksum, revision) \
                 SELECT ?1, ?2, 'media_job', ?3, 'media.job.succeeded', ?4, ?5, 'pending', 0, ?6, ?6, \
                        0, ?14, 1, ?15, 0 \
                 FROM media_jobs j WHERE j.id = ?3 AND j.organization_id = ?2 \
                   AND j.state = 'succeeded' AND j.revision = ?7 AND j.worker_id = ?8 \
                   AND j.lease_token_digest = ?9 AND j.output_object_key = ?10 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?11 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?12 \
                       AND c.reservation_id = ?13 AND c.response_status = 200) \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-succeeded:{job_id}")),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_str(&final_key),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
        worker_command_reservation_cleanup(
            &database,
            &tenant_id,
            &idempotency_key,
            &reservation_id,
        )?,
    ];
    statements.insert(
        6,
        database
            .prepare(
                "UPDATE media_native_output_staging_v1 SET state = 'published', updated_at_ms = ?3 \
                 WHERE job_id = ?1 AND attempt = ?2 AND state = 'staged' \
                   AND organization_id = ?4 AND worker_id = ?5 AND lease_token_digest = ?6 \
                   AND staging_object_key = ?7 AND final_object_key = ?8 \
                   AND bytes = ?9 AND checksum_sha256 = ?10 AND content_type = ?11 \
                   AND provider_etag = ?12 AND EXISTS (SELECT 1 FROM object_manifests m \
                     WHERE m.object_key = ?8 AND m.organization_id = ?4 \
                       AND m.video_id = ?13 AND m.bytes = ?9 AND m.checksum_sha256 = ?10 \
                       AND m.content_type = ?11 AND m.provider_etag = ?14 \
                       AND m.state = 'available') \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?4 AND c.idempotency_key = ?15 \
                       AND c.command_type = 'native_job_complete' AND c.request_digest = ?16 \
                       AND c.reservation_id = ?17 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_str(&candidate_key),
                JsValue::from_str(&final_key),
                JsValue::from_f64(completed.bytes as f64),
                JsValue::from_str(&completed.checksum_sha256),
                JsValue::from_str(&completed.content_type),
                JsValue::from_str(&staging_etag),
                JsValue::from_str(&existing.video_id),
                JsValue::from_str(&output_etag),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
    );
    if let (Some(seed), Some((manifest_digest, manifest_json))) =
        (execution_seed.as_ref(), execution_manifest.as_ref())
    {
        let lease_expires_at_ms = existing
            .lease_expires_at_ms
            .ok_or_else(|| Error::RustError("worker completion lease is unavailable".into()))?;
        let execution_statements = vec![
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'publishing', attempt = ?2, \
                       lease_epoch = lease_epoch + 1, lease_token_digest = ?4, \
                       lease_expires_at_ms = ?5, staging_object_key = NULL, \
                       staged_checksum_sha256 = ?6, staged_bytes = ?7, \
                       failure_class = NULL, updated_at_ms = ?8 \
                     WHERE job_id = ?1 AND organization_id = ?3 \
                       AND selected_executor = 'native_gstreamer' \
                       AND state IN ('queued','fallback_queued','leased','transforming','staged','publishing') \
                       AND attempt <= ?2 AND output_content_type = ?9 \
                       AND max_output_bytes >= ?7 AND final_object_key = ?10 \
                       AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = job_id \
                         AND j.organization_id = ?3 AND j.state = 'succeeded' \
                         AND j.revision = ?11 AND j.attempt = ?2 AND j.worker_id = ?12 \
                         AND j.lease_token_digest = ?4 AND j.output_object_key = ?10) \
                       AND EXISTS (SELECT 1 FROM command_idempotency c \
                         WHERE c.organization_id = ?3 AND c.idempotency_key = ?13 \
                           AND c.command_type = 'native_job_complete' \
                           AND c.request_digest = ?14 AND c.reservation_id = ?15 \
                           AND c.response_status IS NULL)",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_f64(existing.attempt as f64),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_f64(lease_expires_at_ms as f64),
                    JsValue::from_str(&completed.checksum_sha256),
                    JsValue::from_f64(completed.bytes as f64),
                    JsValue::from_f64(now as f64),
                    JsValue::from_str(&completed.content_type),
                    JsValue::from_str(&final_key),
                    JsValue::from_f64(next_revision as f64),
                    JsValue::from_str(&actor.user_id),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&reservation_id),
                ])?,
            database
                .prepare(
                    "INSERT INTO media_output_manifests_v1(manifest_digest, job_id, \
                       organization_id, video_id, executor, source_checksum_sha256, \
                       normalized_profile_sha256, object_key, object_checksum_sha256, \
                       bytes, content_type, manifest_json, created_at_ms) \
                     SELECT ?1, ?2, ?3, ?4, 'native_gstreamer', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12 \
                     WHERE EXISTS (SELECT 1 FROM media_job_execution_v1 e \
                       WHERE e.job_id = ?2 AND e.organization_id = ?3 \
                         AND e.state = 'publishing' AND e.selected_executor = 'native_gstreamer' \
                         AND e.attempt = ?13 AND e.lease_token_digest = ?14 \
                         AND e.staged_checksum_sha256 = ?8 AND e.staged_bytes = ?9) \
                       AND EXISTS (SELECT 1 FROM command_idempotency c \
                         WHERE c.organization_id = ?3 AND c.idempotency_key = ?15 \
                           AND c.command_type = 'native_job_complete' \
                           AND c.request_digest = ?16 AND c.reservation_id = ?17 \
                           AND c.response_status IS NULL)",
                )
                .bind(&[
                    JsValue::from_str(manifest_digest),
                    JsValue::from_str(job_id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&existing.video_id),
                    JsValue::from_str(&seed.source_checksum_sha256),
                    JsValue::from_str(&seed.normalized_profile_sha256),
                    JsValue::from_str(&final_key),
                    JsValue::from_str(&completed.checksum_sha256),
                    JsValue::from_f64(completed.bytes as f64),
                    JsValue::from_str(&completed.content_type),
                    JsValue::from_str(manifest_json),
                    JsValue::from_f64(now as f64),
                    JsValue::from_f64(existing.attempt as f64),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&digest),
                    JsValue::from_str(&reservation_id),
                ])?,
            database
                .prepare(
                    "UPDATE media_job_execution_v1 SET state = 'succeeded', \
                       manifest_digest = ?2, lease_token_digest = NULL, \
                       lease_expires_at_ms = NULL, updated_at_ms = ?3 \
                     WHERE job_id = ?1 AND state = 'publishing' \
                       AND selected_executor = 'native_gstreamer' \
                       AND attempt = ?4 AND lease_token_digest = ?5 \
                       AND staged_checksum_sha256 = ?6 AND staged_bytes = ?7 \
                       AND EXISTS (SELECT 1 FROM media_output_manifests_v1 m \
                         WHERE m.job_id = ?1 AND m.manifest_digest = ?2 \
                           AND m.object_key = ?8 AND m.object_checksum_sha256 = ?6 \
                           AND m.bytes = ?7 AND m.content_type = ?9)",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_str(manifest_digest),
                    JsValue::from_f64(now as f64),
                    JsValue::from_f64(existing.attempt as f64),
                    JsValue::from_str(&lease_digest),
                    JsValue::from_str(&completed.checksum_sha256),
                    JsValue::from_f64(completed.bytes as f64),
                    JsValue::from_str(&final_key),
                    JsValue::from_str(&completed.content_type),
                ])?,
        ];
        // The common execution manifest must commit before the command replay
        // and outbox rows can make this native completion externally visible.
        statements.splice(7..7, execution_statements);
    }
    if let Some((probe, source)) = native_probe.as_ref() {
        statements.push(native_probe_insert_statement(
            &database,
            &tenant_id,
            &existing,
            source,
            probe,
            &completed.checksum_sha256,
            now,
            next_revision,
            &actor.user_id,
            &lease_digest,
            &idempotency_key,
            &digest,
        )?);
    }
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-complete:{job_id}:{}", existing.attempt),
            now,
            statements,
        )
        .await?,
    )?;
    let staging_cleanup_confirmed = if bucket.delete(&candidate_key).await.is_ok() {
        let first_absent = bucket.head(&candidate_key).await.ok().flatten().is_none();
        let second_absent = bucket.head(&candidate_key).await.ok().flatten().is_none();
        first_absent && second_absent
    } else {
        false
    };
    if staging_cleanup_confirmed {
        if let Ok(cleanup_now) = current_time_ms()
            && let Ok(statement) = database.prepare(
                "UPDATE media_native_output_staging_v1 SET state = 'cleaned', updated_at_ms = ?3 \
                 WHERE job_id = ?1 AND attempt = ?2 AND state = 'published' \
                   AND staging_object_key = ?4 AND final_object_key = ?5 \
                   AND EXISTS (SELECT 1 FROM object_manifests m WHERE m.object_key = ?5 \
                     AND m.organization_id = ?6 AND m.checksum_sha256 = ?7 \
                     AND m.bytes = ?8 AND m.content_type = ?9 AND m.state = 'available')",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_f64(existing.attempt as f64),
                    JsValue::from_f64(cleanup_now as f64),
                    JsValue::from_str(&candidate_key),
                    JsValue::from_str(&final_key),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_str(&completed.checksum_sha256),
                    JsValue::from_f64(completed.bytes as f64),
                    JsValue::from_str(&completed.content_type),
                ])
        {
            let cleanup = execute_mutation_batch(
                &database,
                &authority_fence,
                &format!("native-output-clean:{job_id}:{}", existing.attempt),
                cleanup_now,
                vec![statement],
            )
            .await;
            let cleanup_succeeded = match cleanup {
                Ok(results) => require_batch_success(results).is_ok(),
                Err(_) => false,
            };
            if !cleanup_succeeded {
                worker::console_warn!(
                    "native media staging cleanup deferred class=cleanup_persistence"
                );
            }
        }
    } else {
        worker::console_warn!("native media staging cleanup deferred class=cleanup_pending");
    }
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_complete",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(current) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if current.state != "succeeded"
        || current.revision != next_revision
        || !worker_identity_matches(&current, actor, &lease_digest)
        || !worker_manifest_matches(&database, &tenant_id, &current, &body).await?
    {
        if current.cancel_requested != 0 || current.state == "cancelled" {
            return failure_response(worker_cancelled_failure(), request_id, config.production());
        }
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if let Some((probe, source)) = native_probe.as_ref()
        && !native_probe_row_matches(
            &database,
            &tenant_id,
            &existing,
            source,
            probe,
            &completed.checksum_sha256,
        )
        .await?
    {
        return failure_response(
            ApiFailure::new(
                409,
                "probe_conflict",
                "The verified probe is immutable and does not match this completion.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    json_response(&response, 200, None)
}

async fn native_job_fail_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    body: WorkerFailRequest,
    request_id: &str,
) -> Result<Response> {
    if !native_worker_enabled(config) {
        return failure_response(
            native_worker_unavailable_failure(),
            request_id,
            config.production(),
        );
    }
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Worker).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if tenant_id != body.tenant_id {
        return failure_response(not_found_failure(), request_id, config.production());
    }
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let lease_digest = digest_credential(&worker_lease_token_header(request)?);
    let idempotency_key = idempotency_header(request)?;
    let digest = request_digest(
        "native_job_fail",
        &serde_json::json!({
            "job_id": job_id,
            "body": &body,
            "lease_token_digest": lease_digest,
        }),
    )
    .map_err(|()| Error::RustError("worker failure could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_fail",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let now = current_time_ms()?;
    let Some(existing) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if !worker_identity_matches(&existing, actor, &lease_digest) {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    if matches!(
        existing.state.as_str(),
        "succeeded" | "failed" | "cancelled"
    ) {
        let response = existing
            .private_response(false)
            .ok_or_else(|| Error::RustError("worker terminal response is invalid".into()))?;
        return json_response(&response, 200, None);
    }
    if existing.cancel_requested == 0 && !active_worker_lease(&existing, actor, &lease_digest, now)
    {
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    let retry_scheduled = body.retryable
        && existing.cancel_requested == 0
        && existing.attempt < native_profile_max_attempts(&existing.profile);
    let target_state = if existing.cancel_requested != 0 || body.error_class == "cancelled" {
        "cancelled"
    } else if retry_scheduled {
        "queued"
    } else {
        "failed"
    };
    let outcome = match target_state {
        "queued" => "retryable_failure",
        "cancelled" => "cancelled",
        _ => "terminal_failure",
    };
    let execution_state = if retry_scheduled {
        "fallback_queued"
    } else {
        target_state
    };
    let execution_failure_class = native_execution_failure_class(&body.error_class)
        .ok_or_else(|| Error::RustError("worker failure class is invalid".into()))?;
    let next_revision = existing
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::RustError("worker failure revision overflowed".into()))?;
    let mut next = existing.clone();
    next.state = target_state.into();
    next.revision = next_revision;
    next.lease_expires_at_ms = None;
    if retry_scheduled {
        next.worker_id = None;
        next.lease_token_digest = None;
    }
    let response = next
        .private_response(retry_scheduled)
        .ok_or_else(|| Error::RustError("worker failure response is invalid".into()))?;
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("worker failure response could not be serialized".into()))?;
    let dead_letter_required = target_state == "failed";
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "attempt": existing.attempt,
        "state": target_state,
        "error_class": body.error_class,
        "retry_scheduled": retry_scheduled,
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let event_type = match target_state {
        "queued" => "media.job.retry_scheduled",
        "cancelled" => "media.job.cancelled",
        _ => "media.job.failed",
    };
    let reservation_id = new_id();
    let mut statements = vec![
        worker_command_reservation(
            &database,
            &tenant_id,
            &idempotency_key,
            "native_job_fail",
            &digest,
            &reservation_id,
            now,
        )?,
        database
            .prepare(
                "UPDATE media_jobs SET state = ?5, error_code = 'native_worker_failure', \
                   error_class = ?6, lease_expires_at_ms = NULL, heartbeat_at_ms = ?7, \
                   worker_id = CASE WHEN ?8 = 1 THEN NULL ELSE worker_id END, \
                   lease_token_digest = CASE WHEN ?8 = 1 THEN NULL ELSE lease_token_digest END, \
                   updated_at_ms = ?7, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND revision = ?3 AND attempt = ?4 \
                   AND state IN ('leased', 'running') AND worker_id = ?9 AND lease_token_digest = ?10 \
                   AND (cancel_requested = 1 OR (lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms > ?7)) \
                   AND (?11 = -1 OR EXISTS (SELECT 1 FROM authority_state a WHERE a.singleton = 1 \
                     AND a.epoch = ?11 AND a.authority = 'd1' \
                     AND a.phase IN ('d1_authoritative', 'finalized'))) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?12 \
                       AND c.command_type = 'native_job_fail' AND c.request_digest = ?13 \
                       AND c.reservation_id = ?14 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(existing.revision as f64),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_str(target_state),
                JsValue::from_str(&body.error_class),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(if retry_scheduled { 1.0 } else { 0.0 }),
                JsValue::from_str(&actor.user_id),
                JsValue::from_str(&lease_digest),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
        database
            .prepare(
                "UPDATE media_job_attempts SET finished_at_ms = ?3, outcome = ?4, error_class = ?5 \
                 WHERE job_id = ?1 AND attempt = ?2 AND outcome IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?1 \
                     AND j.organization_id = ?6 AND j.state = ?7 AND j.revision = ?8) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?6 AND c.idempotency_key = ?9 \
                       AND c.command_type = 'native_job_fail' AND c.request_digest = ?10 \
                       AND c.reservation_id = ?11 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outcome),
                JsValue::from_str(&body.error_class),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(target_state),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
        database
            .prepare(
                "INSERT INTO media_job_dead_letters(job_id, attempt, error_class, diagnostic_code, created_at_ms) \
                 SELECT ?1, ?2, ?3, 'native_worker_exhausted', ?4 FROM media_jobs j \
                 WHERE ?5 = 1 AND j.id = ?1 AND j.organization_id = ?6 AND j.state = 'failed' \
                   AND j.revision = ?7 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?6 AND c.idempotency_key = ?8 \
                       AND c.command_type = 'native_job_fail' AND c.request_digest = ?9 \
                       AND c.reservation_id = ?10 AND c.response_status IS NULL) \
                 ON CONFLICT(job_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_str(&body.error_class),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(if dead_letter_required { 1.0 } else { 0.0 }),
                JsValue::from_str(&tenant_id),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
        database
            .prepare(
                "UPDATE command_idempotency SET response_status = 200, response_json = ?4 \
                 WHERE organization_id = ?1 AND idempotency_key = ?2 \
                   AND command_type = 'native_job_fail' AND request_digest = ?3 \
                   AND reservation_id = ?5 AND response_status IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?6 \
                     AND j.organization_id = ?1 AND j.state = ?7 AND j.revision = ?8) \
                   AND (NOT EXISTS (SELECT 1 FROM media_job_execution_v1 e WHERE e.job_id = ?6) \
                     OR EXISTS (SELECT 1 FROM media_job_execution_v1 e WHERE e.job_id = ?6 \
                       AND e.state = ?9 AND e.selected_executor = 'native_gstreamer' \
                       AND e.failure_class = ?10 AND e.attempt = ?11))",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(job_id),
                JsValue::from_str(target_state),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(execution_state),
                JsValue::from_str(execution_failure_class),
                JsValue::from_f64(existing.attempt as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                   event_type, deduplication_key, payload_json, state, attempt, available_at_ms, \
                   created_at_ms, event_sequence, event_fingerprint, payload_schema_version, \
                   payload_checksum, revision) \
                 SELECT ?1, ?2, 'media_job', ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?7, \
                        0, ?13, 1, ?14, 0 \
                 FROM media_jobs j WHERE j.id = ?3 AND j.organization_id = ?2 \
                   AND j.state = ?8 AND j.revision = ?9 \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?2 AND c.idempotency_key = ?10 \
                       AND c.command_type = 'native_job_fail' AND c.request_digest = ?11 \
                       AND c.reservation_id = ?12 AND c.response_status = 200) \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(event_type),
                JsValue::from_str(&format!("media-{target_state}:{job_id}:{}", existing.attempt)),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(target_state),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
        worker_command_reservation_cleanup(
            &database,
            &tenant_id,
            &idempotency_key,
            &reservation_id,
        )?,
    ];
    statements.insert(
        4,
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = ?2, attempt = ?3, \
                   failure_class = ?4, lease_token_digest = NULL, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?5 \
                 WHERE job_id = ?1 AND selected_executor = 'native_gstreamer' \
                   AND state IN ('queued','fallback_queued','leased','transforming','staged','publishing') \
                   AND attempt <= ?3 AND EXISTS (SELECT 1 FROM media_jobs j \
                     WHERE j.id = job_id AND j.organization_id = ?6 AND j.state = ?7 \
                       AND j.revision = ?8 AND j.attempt = ?3) \
                   AND EXISTS (SELECT 1 FROM command_idempotency c \
                     WHERE c.organization_id = ?6 AND c.idempotency_key = ?9 \
                       AND c.command_type = 'native_job_fail' AND c.request_digest = ?10 \
                       AND c.reservation_id = ?11 AND c.response_status IS NULL)",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(execution_state),
                JsValue::from_f64(existing.attempt as f64),
                JsValue::from_str(execution_failure_class),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(target_state),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&reservation_id),
            ])?,
    );
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("native-fail:{job_id}:{}", existing.attempt),
            now,
            statements,
        )
        .await?,
    )?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "native_job_fail",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(current) = load_worker_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if current.state != target_state || current.revision != next_revision {
        if current.cancel_requested != 0 || current.state == "cancelled" {
            let response = current
                .private_response(false)
                .ok_or_else(|| Error::RustError("worker job response is invalid".into()))?;
            return json_response(&response, 200, None);
        }
        return failure_response(
            worker_lease_conflict_failure(),
            request_id,
            config.production(),
        );
    }
    json_response(&response, 200, None)
}

async fn media_job_status_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Read).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(job) = load_media_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let status = job
        .public_status()
        .ok_or_else(|| Error::RustError("media job state is invalid".into()))?;
    json_response(&status, 200, None)
}

async fn media_job_cancel_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    job_id: &str,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_id) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_id).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let idempotency_key = idempotency_header(request)?;
    let digest = digest_identifier("media_job_cancel", job_id)
        .map_err(|()| Error::RustError("cancel command could not be digested".into()))?;
    match command_replay(
        &database,
        &authority_fence,
        &tenant_id,
        &idempotency_key,
        "media_job_cancel",
        &digest,
    )
    .await?
    {
        CommandReplay::Stored { status, json } => return stored_json_response(status, &json),
        CommandReplay::Conflict => {
            return failure_response(
                idempotency_conflict_failure(),
                request_id,
                config.production(),
            );
        }
        CommandReplay::New => {}
    }
    let Some(existing) = load_media_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let now = current_time_ms()?;
    if matches!(existing.state.as_str(), "succeeded" | "failed") {
        return failure_response(
            ApiFailure::new(
                409,
                "job_terminal",
                "A terminal media job cannot be cancelled.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("media-cancel-request:{job_id}:{digest}"),
            now,
            vec![database
                .prepare(
                    "UPDATE media_jobs SET cancel_requested = 1, \
                       state = CASE WHEN state = 'queued' THEN 'cancelled' ELSE state END, \
                       progress_basis_points = CASE WHEN state = 'queued' THEN 0 ELSE progress_basis_points END, \
                       updated_at_ms = ?3, revision = revision + 1 \
                     WHERE id = ?1 AND organization_id = ?2 \
                       AND state IN ('queued', 'leased', 'running') AND cancel_requested = 0",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_str(&tenant_id),
                    JsValue::from_f64(now as f64),
                ])?],
        )
        .await?,
    )?;
    let Some(job) = load_media_job(&database, &tenant_id, job_id).await? else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    if matches!(job.state.as_str(), "succeeded" | "failed") {
        return failure_response(
            ApiFailure::new(
                409,
                "job_terminal",
                "A terminal media job cannot be cancelled.",
                false,
            ),
            request_id,
            config.production(),
        );
    }
    if job.state != "cancelled" && job.cancel_requested != 1 {
        return failure_response(
            ApiFailure::new(
                409,
                "job_state_conflict",
                "The media job changed while cancellation was requested.",
                true,
            ),
            request_id,
            config.production(),
        );
    }
    let status = job
        .public_status()
        .ok_or_else(|| Error::RustError("media job state is invalid".into()))?;
    let response_json = serde_json::to_string(&status)
        .map_err(|_| Error::RustError("cancel response could not be serialized".into()))?;
    let outbox_id = new_id();
    let outbox_payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "state": status.state,
        "cancel_requested": true,
    })
    .to_string();
    let outbox_payload_checksum = ChecksumSha256::digest_bytes(outbox_payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "INSERT INTO command_idempotency(\
                   organization_id, idempotency_key, command_type, request_digest, \
                   response_status, response_json, created_at_ms, expires_at_ms\
                 ) VALUES (?1, ?2, 'media_job_cancel', ?3, 200, ?4, ?5, ?6)",
            )
            .bind(&[
                JsValue::from_str(&tenant_id),
                JsValue::from_str(&idempotency_key),
                JsValue::from_str(&digest),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now as f64),
                JsValue::from_f64((now + COMMAND_TTL_MS) as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) VALUES (?1, ?2, 'media_job', ?3, 'media.job.cancel_requested', ?4, ?5, \
                           'pending', 0, ?6, ?6, 0, ?7, 1, ?8, 0) \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(&tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-cancel:{job_id}")),
                JsValue::from_str(&outbox_payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(outbox_payload_checksum.as_str()),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            &database,
            &authority_fence,
            &format!("media-cancel:{job_id}:{digest}"),
            now,
            statements,
        )
        .await?,
    )?;
    json_response(&status, 200, None)
}

async fn complete_fake_preview(
    env: &Env,
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    command: FakePreview<'_>,
) -> Result<()> {
    let FakePreview {
        tenant_id,
        video_id,
        job_id,
        output_key,
        source_version,
        source,
    } = command;
    let source_bytes = u64::try_from(source.bytes)
        .ok()
        .filter(|value| *value > 0 && *value <= MAX_SINGLE_UPLOAD_BYTES)
        .ok_or_else(|| Error::RustError("fake source size is invalid".into()))?;
    let checksum_text = source
        .checksum_sha256
        .as_deref()
        .ok_or_else(|| Error::RustError("fake source checksum is missing".into()))?;
    let checksum = parse_sha256(checksum_text)
        .ok_or_else(|| Error::RustError("fake source checksum is invalid".into()))?;
    let tenant_contract = storage_tenant(tenant_id)
        .ok_or_else(|| Error::RustError("fake tenant authority is invalid".into()))?;
    let source_authority = governed_object(
        database,
        tenant_contract,
        &source.object_key,
        "managed-media-fake",
    )
    .await
    .map_err(|()| Error::RustError("storage authority is unavailable".into()))?
    .ok_or_else(|| Error::RustError("fake source authority is unavailable".into()))?;
    let expected_output = storage_governance_runtime::deterministic_derivative_key(
        env,
        tenant_contract,
        video_id,
        "preview_v1",
        &source_authority,
    )
    .map_err(|_| Error::RustError("managed media is disabled".into()))?;
    if expected_output != output_key {
        return Err(Error::RustError(
            "fake derivative identity is invalid".into(),
        ));
    }
    let output_authority = GovernedObject::new(
        tenant_contract,
        GovernedObjectId::parse(output_key)
            .map_err(|_| Error::RustError("fake output identity is invalid".into()))?,
        GovernedObjectRole::Preview,
        ObjectVisibility::Private,
        GovernedObjectState::Active,
        MalwareDisposition::Clean,
        u64::from(source_version),
        1,
        ChecksumSha256::parse(checksum_text)
            .map_err(|_| Error::RustError("fake output checksum is invalid".into()))?,
        ByteSize::new(source_bytes)
            .map_err(|_| Error::RustError("fake output size is invalid".into()))?,
        None,
    )
    .map_err(|_| Error::RustError("fake output authority is invalid".into()))?;
    let policy = storage_governance_runtime::managed_media_policy(env)
        .map_err(|_| Error::RustError("managed media is disabled".into()))?;
    let input = policy
        .authorize(tenant_contract, &source_authority)
        .map_err(|_| Error::RustError("fake source is denied".into()))?;
    policy
        .authorize_output(
            &input,
            &output_authority,
            &ChecksumSha256::digest_bytes(b"preview_v1"),
        )
        .map_err(|_| Error::RustError("fake output is denied".into()))?;
    let now = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?;
    let governance = frame_application::StorageGovernanceService::new(Vec::new())
        .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    for (object, operation) in [
        (&source_authority, StorageOperation::Read),
        (&output_authority, StorageOperation::WriteImmutable),
    ] {
        governance
            .authorize(
                CorrelationId::new(),
                StorageAccessRequest {
                    actor: StorageActor::Service {
                        tenant_id: tenant_contract,
                        purpose: frame_domain::StorageServicePurpose::MediaProcessor,
                    },
                    operation,
                    surface: StorageAccessSurface::MediaTransformation,
                    object,
                    now,
                    grant: None,
                    grant_proof: None,
                    request_domain: None,
                    custom_domain: None,
                },
            )
            .map_err(|_| Error::RustError("fake storage operation is denied".into()))?;
    }
    let integration = active_r2_integration(database, tenant_id)
        .await?
        .ok_or_else(|| Error::RustError("fake R2 integration is unavailable".into()))?;
    let bucket = env.bucket("RECORDINGS")?;
    let output = if let Some(output) = bucket.head(output_key).await? {
        output
    } else {
        let source_object = bucket
            .get(&source.object_key)
            .execute()
            .await?
            .filter(|object| object.size() == source_bytes)
            .ok_or_else(|| Error::RustError("fake source object is unavailable".into()))?;
        if source_object.checksum().sha256.as_deref() != Some(checksum.as_slice()) {
            return Err(Error::RustError(
                "fake source object failed checksum verification".into(),
            ));
        }
        let stream = FixedLengthStream::wrap(
            source_object
                .body()
                .ok_or_else(|| Error::RustError("fake source body is unavailable".into()))?
                .stream()?,
            source_bytes,
        );
        bucket
            .put(output_key, stream)
            .http_metadata(HttpMetadata {
                content_type: Some(source.content_type.clone()),
                content_disposition: Some("inline".into()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .sha256(checksum.to_vec())
            .only_if(Conditional {
                etag_does_not_match: Some("*".into()),
                ..Conditional::default()
            })
            .execute()
            .await?
            .ok_or_else(|| Error::RustError("fake derivative write conflicted".into()))?
    };
    if output.size() != source_bytes
        || output.checksum().sha256.as_deref() != Some(checksum.as_slice())
    {
        return Err(Error::RustError(
            "fake derivative failed checksum verification".into(),
        ));
    }

    let now = current_time_ms()?;
    let storage_object_id = new_id();
    let attempt_id = new_id();
    let completion_lease = digest_identifier("fake_completion_lease", &attempt_id)
        .map_err(|()| Error::RustError("fake completion lease is invalid".into()))?;
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!("media-fake-claim:{job_id}"),
            now,
            vec![database
                .prepare(
                    "UPDATE media_jobs SET state = 'running', worker_id = ?3, lease_token_digest = ?4, \
                       heartbeat_at_ms = ?5, updated_at_ms = ?5, revision = revision + 1 \
                     WHERE id = ?1 AND organization_id = ?2 AND state = 'queued' \
                       AND cancel_requested = 0",
                )
                .bind(&[
                    JsValue::from_str(job_id),
                    JsValue::from_str(tenant_id),
                    JsValue::from_str(&attempt_id),
                    JsValue::from_str(&completion_lease),
                    JsValue::from_f64(now as f64),
                ])?],
        )
        .await?,
    )?;
    let claimed = database
        .prepare(
            "SELECT 1 AS ready FROM media_jobs WHERE id = ?1 AND organization_id = ?2 \
             AND state = 'running' AND worker_id = ?3 AND lease_token_digest = ?4 \
             AND cancel_requested = 0 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(job_id),
            JsValue::from_str(tenant_id),
            JsValue::from_str(&attempt_id),
            JsValue::from_str(&completion_lease),
        ])?
        .first::<ReadyRow>(None)
        .await?;
    if claimed.is_none_or(|row| row.ready != 1) {
        return Err(Error::RustError(
            "fake media job is no longer eligible for completion".into(),
        ));
    }
    let outbox_id = new_id();
    let output_etag = output.etag();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": job_id,
        "video_id": video_id,
        "executor": "local_fake_native_gstreamer",
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "UPDATE media_jobs SET state = 'succeeded', attempt = 1, progress_basis_points = 10000, \
                   error_code = NULL, error_class = NULL, updated_at_ms = ?3, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND state = 'running' \
                   AND cancel_requested = 0 AND lease_token_digest = ?4",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_governed_objects_v1(organization_id, object_key, role, visibility, \
                   state, malware_disposition, immutable_revision, cache_generation, checksum_sha256, \
                   bytes, content_type, retention_until_ms, created_at_ms, updated_at_ms) \
                 SELECT ?1, ?2, 'preview', 'private', 'active', 'clean', ?3, 1, ?4, ?5, ?6, NULL, ?7, ?7 \
                   FROM media_jobs WHERE id = ?8 AND organization_id = ?1 \
                     AND state = 'succeeded' AND lease_token_digest = ?9 \
                 ON CONFLICT(organization_id, object_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(tenant_id),
                JsValue::from_str(output_key),
                JsValue::from_f64(f64::from(source_version)),
                JsValue::from_str(checksum_text),
                JsValue::from_f64(source_bytes as f64),
                JsValue::from_str(&source.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO media_job_attempts(\
                   job_id, attempt, executor, worker_id, started_at_ms, finished_at_ms, outcome\
                 ) SELECT ?1, 1, 'native_gstreamer', ?2, ?3, ?3, 'succeeded' \
                   FROM media_jobs WHERE id = ?1 AND organization_id = ?4 \
                     AND state = 'succeeded' AND lease_token_digest = ?5 \
                 ON CONFLICT(job_id, attempt) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(job_id),
                JsValue::from_str(&attempt_id),
                JsValue::from_f64(now as f64),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO object_manifests(\
                   object_key, video_id, role, bytes, checksum_sha256, content_type, created_at_ms, \
                   organization_id, object_version, provider_etag, state, updated_at_ms\
                 ) SELECT ?1, ?2, 'preview', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'available', ?6 \
                   FROM media_jobs WHERE id = ?10 AND organization_id = ?7 \
                     AND state = 'succeeded' AND lease_token_digest = ?11 \
                 ON CONFLICT(object_key) DO UPDATE SET \
                   bytes = excluded.bytes, checksum_sha256 = excluded.checksum_sha256, \
                   content_type = excluded.content_type, provider_etag = excluded.provider_etag, \
                   state = 'available', updated_at_ms = excluded.updated_at_ms \
                 WHERE object_manifests.video_id = excluded.video_id \
                   AND object_manifests.organization_id = excluded.organization_id \
                   AND object_manifests.role = excluded.role \
                   AND object_manifests.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(output_key),
                JsValue::from_str(video_id),
                JsValue::from_f64(source_bytes as f64),
                JsValue::from_str(checksum_text),
                JsValue::from_str(&source.content_type),
                JsValue::from_f64(now as f64),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(f64::from(source_version)),
                JsValue::from_str(&output_etag),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO storage_objects(\
                   id, organization_id, integration_id, video_id, object_key, role, object_version, \
                   state, bytes, content_type, checksum_sha256, provider_etag, created_at_ms\
                 ) SELECT ?1, ?2, ?3, ?4, ?5, 'preview', ?6, 'available', ?7, ?8, ?9, ?10, ?11 \
                   FROM media_jobs WHERE id = ?12 AND organization_id = ?2 \
                     AND state = 'succeeded' AND lease_token_digest = ?13 \
                 ON CONFLICT(integration_id, object_key) DO UPDATE SET \
                   state = 'available', bytes = excluded.bytes, content_type = excluded.content_type, \
                   checksum_sha256 = excluded.checksum_sha256, provider_etag = excluded.provider_etag \
                 WHERE storage_objects.organization_id = excluded.organization_id \
                   AND storage_objects.video_id = excluded.video_id \
                   AND storage_objects.role = excluded.role \
                   AND storage_objects.object_version = excluded.object_version",
            )
            .bind(&[
                JsValue::from_str(&storage_object_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&integration.id),
                JsValue::from_str(video_id),
                JsValue::from_str(output_key),
                JsValue::from_f64(f64::from(source_version)),
                JsValue::from_f64(source_bytes as f64),
                JsValue::from_str(&source.content_type),
                JsValue::from_str(checksum_text),
                JsValue::from_str(&output_etag),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "UPDATE videos SET playback_object_key = ?3, state = 'ready', \
                    updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?5 \
                     AND j.organization_id = ?2 AND j.state = 'succeeded' \
                     AND j.lease_token_digest = ?6)",
            )
            .bind(&[
                JsValue::from_str(video_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(output_key),
                JsValue::from_f64(now as f64),
                JsValue::from_str(job_id),
                JsValue::from_str(&completion_lease),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(\
                   id, organization_id, aggregate_type, aggregate_id, event_type, \
                   deduplication_key, payload_json, state, attempt, available_at_ms, created_at_ms, \
                   event_sequence, event_fingerprint, payload_schema_version, payload_checksum, revision\
                 ) SELECT ?1, ?2, 'media_job', ?3, 'media.job.succeeded', ?4, ?5, \
                           'pending', 0, ?6, ?6, 0, ?8, 1, ?9, 0 FROM media_jobs \
                   WHERE id = ?3 AND organization_id = ?2 AND state = 'succeeded' \
                     AND lease_token_digest = ?7 \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(job_id),
                JsValue::from_str(&format!("media-succeeded:{job_id}")),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(&completion_lease),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!("media-fake-complete:{job_id}"),
            now,
            statements,
        )
        .await?,
    )?;
    let completed = load_media_job(database, tenant_id, job_id).await?;
    if !completed.is_some_and(|job| job.state == "succeeded" && job.cancel_requested == 0) {
        return Err(Error::RustError(
            "fake media completion lost its state fence".into(),
        ));
    }
    Ok(())
}

async fn mark_fake_job_failed(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    tenant_id: &str,
    job_id: &str,
) -> Result<()> {
    let now = current_time_ms()?;
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!("media-fake-fail:{job_id}"),
            now,
            vec![
                database
                    .prepare(
                        "UPDATE media_jobs SET state = 'failed', attempt = attempt + 1, \
                       error_code = 'executor_failure', error_class = 'fake_executor_failure', \
                       updated_at_ms = ?3, revision = revision + 1 \
                     WHERE id = ?1 AND organization_id = ?2 \
                       AND state IN ('queued', 'running') AND cancel_requested = 0",
                    )
                    .bind(&[
                        JsValue::from_str(job_id),
                        JsValue::from_str(tenant_id),
                        JsValue::from_f64(now as f64),
                    ])?,
            ],
        )
        .await?,
    )
}

async fn command_replay(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    tenant_id: &str,
    key: &str,
    command_type: &str,
    digest: &str,
) -> Result<CommandReplay> {
    command_replay_accepting(
        database,
        authority_fence,
        tenant_id,
        key,
        command_type,
        digest,
        None,
    )
    .await
}

async fn command_replay_accepting(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    tenant_id: &str,
    key: &str,
    command_type: &str,
    canonical_digest: &str,
    equivalent_digest: Option<&str>,
) -> Result<CommandReplay> {
    let row = database
        .prepare(
            "SELECT command_type, request_digest, response_status, response_json, expires_at_ms \
             FROM command_idempotency WHERE organization_id = ?1 AND idempotency_key = ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(key)])?
        .first::<StoredCommandRow>(None)
        .await?;
    let Some(row) = row else {
        return Ok(CommandReplay::New);
    };
    let now = current_time_ms()?;
    if row.expires_at_ms <= now {
        let operation_digest = digest_identifier(
            "command_replay_expiry",
            &format!("{tenant_id}\0{key}\0{}", row.expires_at_ms),
        )
        .map_err(|()| Error::RustError("command expiry identity is invalid".into()))?;
        require_batch_success(
            execute_mutation_batch(
                database,
                authority_fence,
                &format!("command-replay-expire:{operation_digest}"),
                now,
                vec![database
                    .prepare(
                        "DELETE FROM command_idempotency \
                         WHERE organization_id = ?1 AND idempotency_key = ?2 AND expires_at_ms = ?3",
                    )
                    .bind(&[
                        JsValue::from_str(tenant_id),
                        JsValue::from_str(key),
                        JsValue::from_f64(row.expires_at_ms as f64),
                    ])?],
            )
            .await?,
        )?;
        return Ok(CommandReplay::New);
    }
    let digest_matches = row.request_digest == canonical_digest
        || equivalent_digest.is_some_and(|digest| row.request_digest == digest);
    if row.command_type != command_type || !digest_matches {
        return Ok(CommandReplay::Conflict);
    }
    match (row.response_status, row.response_json) {
        (Some(status), Some(json)) if (200..=299).contains(&status) && json.len() <= 64 * 1_024 => {
            Ok(CommandReplay::Stored {
                status: u16::try_from(status)
                    .map_err(|_| Error::RustError("stored command status is invalid".into()))?,
                json,
            })
        }
        _ => Err(Error::RustError(
            "stored command response is incomplete".into(),
        )),
    }
}

async fn video_is_scoped(database: &D1Database, tenant_id: &str, video_id: &str) -> Result<bool> {
    Ok(database
        .prepare(
            "SELECT id FROM videos \
             WHERE id = ?1 AND organization_id = ?2 AND deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(video_id), JsValue::from_str(tenant_id)])?
        .first::<VideoScopeRow>(None)
        .await?
        .is_some_and(|row| row.id == video_id))
}

async fn load_video_mutation(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
    actor_id: &str,
) -> Result<Option<VideoMutationRow>> {
    AggregateRepository::new(database)
        .video_for_mutation(tenant_id, video_id, actor_id)
        .await
        .map_err(repository::RepositoryFailure::into_worker_error)
}

async fn video_has_shareable_media(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
) -> Result<bool> {
    Ok(database
        .prepare(
            "SELECT 1 AS ready FROM videos v \
             JOIN object_manifests m ON m.object_key = v.playback_object_key \
               AND m.video_id = v.id AND m.organization_id = v.organization_id \
             WHERE v.id = ?1 AND v.organization_id = ?2 AND v.state = 'ready' \
               AND v.deleted_at_ms IS NULL AND m.role = 'preview' \
               AND m.object_version > 0 AND m.state = 'available' \
               AND m.bytes BETWEEN 1 AND 9007199254740991 \
               AND m.content_type LIKE 'video/%' \
               AND length(m.checksum_sha256) = 64 \
               AND lower(m.checksum_sha256) = m.checksum_sha256 \
               AND m.checksum_sha256 NOT GLOB '*[^0-9a-f]*' \
               AND m.provider_etag IS NOT NULL AND m.provider_etag <> '' \
               AND substr(m.object_key, 1, length('tenants/' || v.organization_id || \
                 '/videos/' || v.id || '/derivatives/')) = \
                 'tenants/' || v.organization_id || '/videos/' || v.id || '/derivatives/' \
               AND instr(m.object_key, '..') = 0 \
               AND instr(m.object_key, char(92)) = 0 \
               AND instr(m.object_key, '?') = 0 AND instr(m.object_key, '#') = 0 \
               AND instr(m.object_key, '%') = 0 LIMIT 1",
        )
        .bind(&[JsValue::from_str(video_id), JsValue::from_str(tenant_id)])?
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1))
}

async fn load_upload(
    database: &D1Database,
    tenant_id: &str,
    upload_id: &str,
) -> Result<Option<UploadRow>> {
    AggregateRepository::new(database)
        .upload(tenant_id, upload_id)
        .await
        .map_err(repository::RepositoryFailure::into_worker_error)
}

async fn load_source_object(
    database: &D1Database,
    tenant_id: &str,
    video_id: &str,
    source_version: u32,
) -> Result<Option<SourceObjectRow>> {
    database
        .prepare(
            "SELECT object_key, bytes, checksum_sha256, content_type \
             FROM object_manifests \
             WHERE organization_id = ?1 AND video_id = ?2 AND object_version = ?3 \
               AND role IN ('source', 'import') AND state = 'available' \
             ORDER BY CASE role WHEN 'source' THEN 0 ELSE 1 END LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(video_id),
            JsValue::from_f64(f64::from(source_version)),
        ])?
        .first::<SourceObjectRow>(None)
        .await
}

async fn load_worker_sources(
    database: &D1Database,
    tenant_id: &str,
    job_id: &str,
) -> Result<Vec<WorkerSourceRow>> {
    let result = database
        .prepare(
            "SELECT ordinal,video_id,source_version,object_key,bytes,checksum_sha256,content_type \
             FROM media_job_current_inputs_v1 WHERE organization_id=?1 AND job_id=?2 \
             ORDER BY ordinal LIMIT 65",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(job_id)])?
        .all()
        .await?;
    if !result.success() {
        return Err(Error::RustError(
            "worker source authority query failed".into(),
        ));
    }
    result.results::<WorkerSourceRow>()
}

fn validated_worker_sources(
    tenant_id: &str,
    profile: &str,
    rows: Vec<WorkerSourceRow>,
) -> Result<Vec<WorkerSourceRow>> {
    let count_valid = match profile {
        "segment_mux_v1" => (2..=64).contains(&rows.len()),
        "composition_v1" => (1..=64).contains(&rows.len()),
        _ => rows.len() == 1,
    };
    if !count_valid {
        return Err(Error::RustError(
            "worker source cardinality is invalid".into(),
        ));
    }
    let per_source_max = if profile == "probe_v1" {
        MULTIPART_MAX_BYTES
    } else {
        MAX_SINGLE_UPLOAD_BYTES
    };
    let mut total_bytes = 0_u64;
    let mut source_identities = HashSet::with_capacity(rows.len());
    for (ordinal, row) in rows.iter().enumerate() {
        let bytes = u64::try_from(row.bytes)
            .ok()
            .filter(|bytes| (1..=per_source_max).contains(bytes))
            .ok_or_else(|| Error::RustError("worker source size is invalid".into()))?;
        if usize::try_from(row.ordinal).ok() != Some(ordinal)
            || row.source_version <= 0
            || !valid_uuid(&row.video_id)
            || !contracts::valid_sha256(&row.checksum_sha256)
            || !supported_native_source_content_type(&row.content_type)
            || !valid_private_object_key(&row.object_key, tenant_id, &row.video_id)
        {
            return Err(Error::RustError(
                "worker source authority is invalid".into(),
            ));
        }
        if profile != "composition_v1"
            && !source_identities.insert((row.video_id.as_str(), row.source_version))
        {
            return Err(Error::RustError(
                "worker source identity is duplicated".into(),
            ));
        }
        total_bytes = total_bytes
            .checked_add(bytes)
            .ok_or_else(|| Error::RustError("worker source size overflowed".into()))?;
    }
    let max_total = native_sandbox(profile)
        .map(|sandbox| sandbox.max_source_bytes)
        .ok_or_else(|| Error::RustError("worker profile sandbox is invalid".into()))?;
    if total_bytes > max_total {
        return Err(Error::RustError(
            "worker source set exceeds its sandbox".into(),
        ));
    }
    Ok(rows)
}

async fn active_r2_integration(
    database: &D1Database,
    tenant_id: &str,
) -> Result<Option<IntegrationRow>> {
    database
        .prepare(
            "SELECT id, capabilities_json FROM storage_integrations \
             WHERE organization_id = ?1 AND provider = 'r2' AND state = 'active' \
             ORDER BY created_at_ms, id LIMIT 1",
        )
        .bind(&[JsValue::from_str(tenant_id)])?
        .first::<IntegrationRow>(None)
        .await
}

async fn r2_integration(
    database: &D1Database,
    tenant_id: &str,
    integration_id: &str,
) -> Result<Option<IntegrationRow>> {
    database
        .prepare(
            "SELECT id, capabilities_json FROM storage_integrations \
             WHERE id = ?1 AND organization_id = ?2 AND provider = 'r2' AND state = 'active' \
             LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(integration_id),
            JsValue::from_str(tenant_id),
        ])?
        .first::<IntegrationRow>(None)
        .await
}

async fn load_media_job(
    database: &D1Database,
    tenant_id: &str,
    job_id: &str,
) -> Result<Option<MediaJobRow>> {
    AggregateRepository::new(database)
        .media_job(tenant_id, job_id)
        .await
        .map_err(repository::RepositoryFailure::into_worker_error)
}

async fn load_worker_job(
    database: &D1Database,
    tenant_id: &str,
    job_id: &str,
) -> Result<Option<WorkerJobRow>> {
    AggregateRepository::new(database)
        .native_worker_job(tenant_id, job_id)
        .await
        .map_err(repository::RepositoryFailure::into_worker_error)
}

fn worker_identity_matches(
    job: &WorkerJobRow,
    actor: &AuthenticatedActor,
    lease_digest: &str,
) -> bool {
    job.worker_id.as_deref() == Some(actor.user_id.as_str())
        && job.lease_token_digest.as_deref() == Some(lease_digest)
}

fn active_worker_lease(
    job: &WorkerJobRow,
    actor: &AuthenticatedActor,
    lease_digest: &str,
    now: i64,
) -> bool {
    matches!(job.state.as_str(), "leased" | "running")
        && worker_identity_matches(job, actor, lease_digest)
        && job.lease_expires_at_ms.is_some_and(|expiry| expiry > now)
}

async fn worker_manifest_matches(
    database: &D1Database,
    tenant_id: &str,
    job: &WorkerJobRow,
    body: &WorkerCompleteRequest,
) -> Result<bool> {
    let Some(completed) = body.output() else {
        return Ok(false);
    };
    let Some(output_contract) = native_output_contract(&job.profile, &completed.content_type)
    else {
        return Ok(false);
    };
    Ok(database
        .prepare(
            "SELECT 1 AS ready FROM object_manifests WHERE object_key = ?1 \
               AND organization_id = ?2 AND video_id = ?3 AND role = ?8 \
               AND object_version = ?4 AND bytes = ?5 AND checksum_sha256 = ?6 \
               AND content_type = ?7 AND provider_etag IS NOT NULL AND provider_etag <> '' \
               AND state = 'available' LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&job.output_object_key),
            JsValue::from_str(tenant_id),
            JsValue::from_str(&job.video_id),
            JsValue::from_f64(job.source_version as f64),
            JsValue::from_f64(completed.bytes as f64),
            JsValue::from_str(&completed.checksum_sha256),
            JsValue::from_str(&completed.content_type),
            JsValue::from_str(output_contract.manifest_role),
        ])?
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1))
}

async fn native_probe_row_matches(
    database: &D1Database,
    tenant_id: &str,
    job: &WorkerJobRow,
    source: &SourceObjectRow,
    probe: &media_service_runtime::VerifiedNativeProbeV1,
    probe_digest: &str,
) -> Result<bool> {
    let Some(source_checksum) = source.checksum_sha256.as_deref() else {
        return Ok(false);
    };
    Ok(database
        .prepare(
            "SELECT 1 AS ready FROM media_source_probes_v1 \
              WHERE organization_id = ?1 AND video_id = ?2 AND source_version = ?3 \
                AND source_object_key = ?4 AND source_checksum_sha256 = ?5 \
                AND source_bytes = ?6 AND source_content_type = ?7 \
                AND container = ?8 AND video_codec = ?9 AND audio_codec = ?10 \
                AND duration_ms = ?11 AND width = ?12 AND height = ?13 \
                AND frame_rate_numerator = ?14 AND frame_rate_denominator = ?15 \
                AND decoded_bytes_upper_bound = ?16 AND frame_count_upper_bound = ?17 \
                AND track_count = ?18 AND probe_contract_version = 1 \
                AND probe_digest = ?19 AND trust = 'verified_native_probe' \
                AND state = 'verified' LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&job.video_id),
            JsValue::from_f64(job.source_version as f64),
            JsValue::from_str(&source.object_key),
            JsValue::from_str(source_checksum),
            JsValue::from_f64(source.bytes as f64),
            JsValue::from_str(&source.content_type),
            JsValue::from_str(&probe.container),
            JsValue::from_str(&probe.video_codec),
            JsValue::from_str(&probe.audio_codec),
            JsValue::from_f64(probe.duration_ms as f64),
            JsValue::from_f64(f64::from(probe.width)),
            JsValue::from_f64(f64::from(probe.height)),
            JsValue::from_f64(f64::from(probe.frame_rate_numerator)),
            JsValue::from_f64(f64::from(probe.frame_rate_denominator)),
            JsValue::from_f64(probe.decoded_bytes_upper_bound as f64),
            JsValue::from_f64(probe.frame_count_upper_bound as f64),
            JsValue::from_f64(f64::from(probe.track_count)),
            JsValue::from_str(probe_digest),
        ])?
        .first::<ReadyRow>(None)
        .await?
        .is_some_and(|row| row.ready == 1))
}

#[allow(clippy::too_many_arguments)]
fn native_probe_insert_statement(
    database: &D1Database,
    tenant_id: &str,
    job: &WorkerJobRow,
    source: &SourceObjectRow,
    probe: &media_service_runtime::VerifiedNativeProbeV1,
    probe_digest: &str,
    now: i64,
    next_revision: i64,
    worker_id: &str,
    lease_digest: &str,
    idempotency_key: &str,
    request_digest: &str,
) -> Result<D1PreparedStatement> {
    let source_checksum = source
        .checksum_sha256
        .as_deref()
        .ok_or_else(|| Error::RustError("validated probe source checksum is absent".into()))?;
    database
        .prepare(
            "INSERT INTO media_source_probes_v1(organization_id, video_id, source_version, \
               source_object_key, source_checksum_sha256, source_bytes, source_content_type, \
               container, video_codec, audio_codec, duration_ms, width, height, \
               frame_rate_numerator, frame_rate_denominator, decoded_bytes_upper_bound, \
               frame_count_upper_bound, track_count, probe_contract_version, probe_digest, \
               trust, state, verified_at_ms, updated_at_ms) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
                    ?14, ?15, ?16, ?17, ?18, 1, ?19, 'verified_native_probe', \
                    'verified', ?20, ?20 \
               FROM media_jobs j WHERE j.id = ?21 AND j.organization_id = ?1 \
                 AND j.state = 'succeeded' AND j.revision = ?22 \
                 AND j.worker_id = ?23 AND j.lease_token_digest = ?24 \
                 AND EXISTS (SELECT 1 FROM command_idempotency c \
                   WHERE c.organization_id = ?1 AND c.idempotency_key = ?25 \
                     AND c.command_type = 'native_job_complete' \
                     AND c.request_digest = ?26 AND c.response_status = 200) \
             ON CONFLICT(organization_id, video_id, source_version) DO NOTHING",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&job.video_id),
            JsValue::from_f64(job.source_version as f64),
            JsValue::from_str(&source.object_key),
            JsValue::from_str(source_checksum),
            JsValue::from_f64(source.bytes as f64),
            JsValue::from_str(&source.content_type),
            JsValue::from_str(&probe.container),
            JsValue::from_str(&probe.video_codec),
            JsValue::from_str(&probe.audio_codec),
            JsValue::from_f64(probe.duration_ms as f64),
            JsValue::from_f64(f64::from(probe.width)),
            JsValue::from_f64(f64::from(probe.height)),
            JsValue::from_f64(f64::from(probe.frame_rate_numerator)),
            JsValue::from_f64(f64::from(probe.frame_rate_denominator)),
            JsValue::from_f64(probe.decoded_bytes_upper_bound as f64),
            JsValue::from_f64(probe.frame_count_upper_bound as f64),
            JsValue::from_f64(f64::from(probe.track_count)),
            JsValue::from_str(probe_digest),
            JsValue::from_f64(now as f64),
            JsValue::from_str(&job.id),
            JsValue::from_f64(next_revision as f64),
            JsValue::from_str(worker_id),
            JsValue::from_str(lease_digest),
            JsValue::from_str(idempotency_key),
            JsValue::from_str(request_digest),
        ])
}

async fn reap_invalid_queued_native_job(
    database: &D1Database,
    tenant_id: &str,
    now: i64,
    authority_fence: &MutationAuthorityFence,
) -> Result<()> {
    // The current-input view deliberately removes a source as soon as its
    // video, immutable manifest, or storage-governance authority is revoked.
    // Convert that fail-closed disappearance into a durable terminal result;
    // otherwise an attempt-zero job can remain queued but unclaimable forever.
    let invalid = database
        .prepare(
            "SELECT j.id, j.video_id, j.state, j.revision, j.attempt, \
                    json_extract(j.payload_json, '$.profile') AS profile, j.source_version, \
                    j.output_object_key, j.worker_id, j.lease_token_digest, \
                    j.lease_expires_at_ms, j.progress_basis_points, j.cancel_requested \
             FROM media_jobs j \
             WHERE j.organization_id = ?1 AND j.selected_executor = 'native_gstreamer' \
               AND j.state = 'queued' AND NOT (\
                 COALESCE(j.input_contract_version, 0) = 1 \
                 AND (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                       WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                     BETWEEN 1 AND 64 \
                 AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 inputs \
                       WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) = \
                     (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                       WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                 AND (SELECT MIN(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                       WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) = 0 \
                 AND (SELECT MAX(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                       WHERE inputs.job_id = j.id AND inputs.organization_id = j.organization_id) = \
                     (SELECT COUNT(*) - 1 FROM media_job_inputs_v1 bound \
                       WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                 AND CASE json_extract(j.payload_json, '$.profile') \
                   WHEN 'composition_v1' THEN \
                     (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                       WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) \
                       BETWEEN 1 AND 64 \
                   ELSE (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                       WHERE bound.job_id = j.id AND bound.organization_id = j.organization_id) = 1 END) \
             ORDER BY j.updated_at_ms, j.id LIMIT 1",
        )
        .bind(&[JsValue::from_str(tenant_id)])?
        .first::<WorkerJobRow>(None)
        .await?;
    let Some(invalid) = invalid else {
        return Ok(());
    };
    let next_revision = invalid
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::RustError("invalid-input job revision overflowed".into()))?;
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": invalid.id,
        "attempt": invalid.attempt,
        "state": "failed",
        "error_class": "input_authority_missing",
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "UPDATE media_jobs SET state = 'failed', \
                   error_code = 'media_input_authority_missing', \
                   error_class = 'input_authority_missing', lease_expires_at_ms = NULL, \
                   worker_id = NULL, lease_token_digest = NULL, heartbeat_at_ms = NULL, \
                   updated_at_ms = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND organization_id = ?2 AND revision = ?3 \
                   AND selected_executor = 'native_gstreamer' AND state = 'queued' \
                   AND NOT (COALESCE(input_contract_version, 0) = 1 \
                     AND (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                           WHERE bound.job_id = media_jobs.id \
                             AND bound.organization_id = media_jobs.organization_id) BETWEEN 1 AND 64 \
                     AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 inputs \
                           WHERE inputs.job_id = media_jobs.id \
                             AND inputs.organization_id = media_jobs.organization_id) = \
                         (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                           WHERE bound.job_id = media_jobs.id \
                             AND bound.organization_id = media_jobs.organization_id) \
                     AND (SELECT MIN(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                           WHERE inputs.job_id = media_jobs.id \
                             AND inputs.organization_id = media_jobs.organization_id) = 0 \
                     AND (SELECT MAX(inputs.ordinal) FROM media_job_current_inputs_v1 inputs \
                           WHERE inputs.job_id = media_jobs.id \
                             AND inputs.organization_id = media_jobs.organization_id) = \
                         (SELECT COUNT(*) - 1 FROM media_job_inputs_v1 bound \
                           WHERE bound.job_id = media_jobs.id \
                             AND bound.organization_id = media_jobs.organization_id) \
                     AND CASE json_extract(payload_json, '$.profile') \
                       WHEN 'composition_v1' THEN \
                         (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                           WHERE bound.job_id = media_jobs.id \
                             AND bound.organization_id = media_jobs.organization_id) BETWEEN 1 AND 64 \
                       ELSE (SELECT COUNT(*) FROM media_job_inputs_v1 bound \
                           WHERE bound.job_id = media_jobs.id \
                             AND bound.organization_id = media_jobs.organization_id) = 1 END) \
                   AND (?5 = -1 OR EXISTS (SELECT 1 FROM authority_state a \
                     WHERE a.singleton = 1 AND a.epoch = ?5 AND a.authority = 'd1' \
                       AND a.phase IN ('d1_authoritative', 'finalized')))",
            )
            .bind(&[
                JsValue::from_str(&invalid.id),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(invalid.revision as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
            ])?,
        database
            .prepare(
                "UPDATE media_job_execution_v1 SET state = 'failed', \
                   failure_class = 'invalid_input', lease_token_digest = NULL, \
                   lease_expires_at_ms = NULL, updated_at_ms = ?4 \
                 WHERE job_id = ?1 AND organization_id = ?2 \
                   AND selected_executor = 'native_gstreamer' \
                   AND state NOT IN ('succeeded', 'failed', 'cancelled', 'dead_letter') \
                   AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id = ?1 \
                     AND j.organization_id = ?2 AND j.state = 'failed' \
                     AND j.revision = ?3)",
            )
            .bind(&[
                JsValue::from_str(&invalid.id),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_f64(now as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO media_job_dead_letters(\
                   job_id, attempt, error_class, diagnostic_code, created_at_ms) \
                 SELECT ?1, ?3, 'input_authority_missing', \
                        'native_input_authority_revoked', ?4 FROM media_jobs j \
                 WHERE ?3 > 0 AND j.id = ?1 AND j.organization_id = ?2 \
                   AND j.state = 'failed' AND j.revision = ?5 \
                 ON CONFLICT(job_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&invalid.id),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(invalid.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(next_revision as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                   event_type, deduplication_key, payload_json, state, attempt, \
                   available_at_ms, created_at_ms, event_sequence, event_fingerprint, \
                   payload_schema_version, payload_checksum, revision) \
                 SELECT ?1, ?2, 'media_job', ?3, 'media.job.failed', ?4, ?5, \
                        'pending', 0, ?6, ?6, 0, ?7, 1, ?8, 0 FROM media_jobs j \
                 WHERE j.id = ?3 AND j.organization_id = ?2 \
                   AND j.state = 'failed' AND j.revision = ?9 \
                 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&invalid.id),
                JsValue::from_str(&format!(
                    "media-input-invalid:{}:{}",
                    invalid.id, invalid.revision
                )),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
                JsValue::from_f64(next_revision as f64),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!(
                "native-reap-invalid-input:{}:{}",
                invalid.id, invalid.revision
            ),
            now,
            statements,
        )
        .await?,
    )
}

async fn reap_exhausted_native_jobs(
    database: &D1Database,
    tenant_id: &str,
    now: i64,
    authority_fence: &MutationAuthorityFence,
) -> Result<()> {
    let expired = database
        .prepare(
            "SELECT id, video_id, state, revision, attempt, \
                    json_extract(payload_json, '$.profile') AS profile, source_version, \
                    output_object_key, worker_id, lease_token_digest, lease_expires_at_ms, \
                    progress_basis_points, cancel_requested FROM media_jobs \
             WHERE organization_id = ?1 AND selected_executor = 'native_gstreamer' \
               AND state IN ('leased', 'running') AND lease_expires_at_ms IS NOT NULL \
               AND lease_expires_at_ms <= ?2 AND (cancel_requested = 1 OR attempt >= \
                 CASE json_extract(payload_json, '$.profile') \
                   WHEN 'probe_v1' THEN 2 WHEN 'audio_presence_v1' THEN 2 ELSE 3 END) \
             ORDER BY updated_at_ms, id LIMIT 1",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_f64(now as f64)])?
        .first::<WorkerJobRow>(None)
        .await?;
    let Some(expired) = expired else {
        return Ok(());
    };
    let max_attempts = native_profile_max_attempts(&expired.profile);
    let target_state = if expired.cancel_requested != 0 {
        "cancelled"
    } else {
        "failed"
    };
    let outcome = if target_state == "cancelled" {
        "cancelled"
    } else {
        "lost_lease"
    };
    let next_revision = expired
        .revision
        .checked_add(1)
        .ok_or_else(|| Error::RustError("expired job revision overflowed".into()))?;
    let outbox_id = new_id();
    let payload = serde_json::json!({
        "schema_version": API_SCHEMA_VERSION,
        "job_id": expired.id,
        "attempt": expired.attempt,
        "state": target_state,
        "error_class": "lease_expired",
    })
    .to_string();
    let payload_checksum = ChecksumSha256::digest_bytes(payload.as_bytes());
    let outbox_event_fingerprint = frame_domain::business_initial_event_fingerprint();
    let statements = vec![
        database
            .prepare(
                "UPDATE media_jobs SET state = ?5, error_code = 'native_lease_expired', \
                   error_class = 'lease_expired', lease_expires_at_ms = NULL, updated_at_ms = ?4, \
                   revision = revision + 1 WHERE id = ?1 AND organization_id = ?2 \
                   AND revision = ?3 AND state IN ('leased', 'running') \
                   AND lease_expires_at_ms IS NOT NULL AND lease_expires_at_ms <= ?4 \
                   AND (cancel_requested = 1 OR attempt >= ?6) \
                   AND (?7 = -1 OR EXISTS (SELECT 1 FROM authority_state a WHERE a.singleton = 1 \
                     AND a.epoch = ?7 AND a.authority = 'd1' \
                     AND a.phase IN ('d1_authoritative', 'finalized')))",
            )
            .bind(&[
                JsValue::from_str(&expired.id),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(expired.revision as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(target_state),
                JsValue::from_f64(max_attempts as f64),
                JsValue::from_f64(authority_fence.sql_epoch as f64),
            ])?,
        database
            .prepare(
                "UPDATE media_job_attempts SET finished_at_ms = ?3, outcome = ?4, \
                   error_class = 'lease_expired' WHERE job_id = ?1 AND attempt = ?2 \
                   AND outcome IS NULL AND EXISTS (SELECT 1 FROM media_jobs j \
                     WHERE j.id = ?1 AND j.organization_id = ?5 AND j.state = ?6 \
                       AND j.revision = ?7)",
            )
            .bind(&[
                JsValue::from_str(&expired.id),
                JsValue::from_f64(expired.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(outcome),
                JsValue::from_str(tenant_id),
                JsValue::from_str(target_state),
                JsValue::from_f64(next_revision as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO media_job_dead_letters(job_id, attempt, error_class, diagnostic_code, created_at_ms) \
                 SELECT ?1, ?2, 'lease_expired', 'native_worker_lease_exhausted', ?3 FROM media_jobs j \
                 WHERE ?4 = 'failed' AND j.id = ?1 AND j.organization_id = ?5 \
                   AND j.state = 'failed' AND j.revision = ?6 ON CONFLICT(job_id) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&expired.id),
                JsValue::from_f64(expired.attempt as f64),
                JsValue::from_f64(now as f64),
                JsValue::from_str(target_state),
                JsValue::from_str(tenant_id),
                JsValue::from_f64(next_revision as f64),
            ])?,
        database
            .prepare(
                "INSERT INTO outbox_events(id, organization_id, aggregate_type, aggregate_id, \
                   event_type, deduplication_key, payload_json, state, attempt, available_at_ms, \
                   created_at_ms, event_sequence, event_fingerprint, payload_schema_version, \
                   payload_checksum, revision) \
                 SELECT ?1, ?2, 'media_job', ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?7, \
                        0, ?10, 1, ?11, 0 \
                 FROM media_jobs j WHERE j.id = ?3 AND j.organization_id = ?2 \
                   AND j.state = ?8 AND j.revision = ?9 ON CONFLICT(deduplication_key) DO NOTHING",
            )
            .bind(&[
                JsValue::from_str(&outbox_id),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&expired.id),
                JsValue::from_str(if target_state == "cancelled" {
                    "media.job.cancelled"
                } else {
                    "media.job.failed"
                }),
                JsValue::from_str(&format!("media-expired:{}:{}", expired.id, expired.attempt)),
                JsValue::from_str(&payload),
                JsValue::from_f64(now as f64),
                JsValue::from_str(target_state),
                JsValue::from_f64(next_revision as f64),
                JsValue::from_str(outbox_event_fingerprint.as_str()),
                JsValue::from_str(payload_checksum.as_str()),
            ])?,
    ];
    require_batch_success(
        execute_mutation_batch(
            database,
            authority_fence,
            &format!("native-reap:{}:{}", expired.id, expired.attempt),
            now,
            statements,
        )
        .await?,
    )
}

async fn completed_upload_matches(env: &Env, upload: &UploadRow) -> Result<bool> {
    let Some(expected_checksum) = upload.checksum_sha256.as_deref().and_then(parse_sha256) else {
        return Ok(false);
    };
    let Some(object) = env
        .bucket("RECORDINGS")?
        .head(&upload.source_object_key)
        .await?
    else {
        return Ok(false);
    };
    let metadata = object.http_metadata();
    let direct_metadata_matches = if upload.transfer_mode == "direct" {
        object
            .custom_metadata()?
            .get("frame-sha256")
            .map(String::as_str)
            == upload.checksum_sha256.as_deref()
    } else {
        true
    };
    Ok(
        object.size() == u64::try_from(upload.expected_bytes).unwrap_or(u64::MAX)
            && object.checksum().sha256.as_deref() == Some(expected_checksum.as_slice())
            && metadata.content_type.as_deref() == Some(upload.content_type.as_str())
            && metadata.content_encoding.is_none()
            && metadata.cache_control.as_deref() == Some("private, no-store")
            && direct_metadata_matches,
    )
}

async fn mutation_authority_fence(
    env: &Env,
    config: &RuntimeConfig,
    tenant_id: &str,
) -> Result<Option<MutationAuthorityFence>> {
    if !config.production() {
        return Ok(Some(MutationAuthorityFence::local()));
    }
    let Some(tenant_id) = storage_tenant(tenant_id) else {
        return Ok(None);
    };
    let domain = CutoverDomain::parse(METADATA_CUTOVER_DOMAIN)
        .map_err(|_| Error::RustError("cutover domain configuration is invalid".into()))?;
    let scope = CutoverScope::new(tenant_id, domain);
    let now = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("cutover authority clock is invalid".into()))?;
    let database = env.d1("DB")?;
    let runtime = cutover_authority_runtime::CutoverAuthorityRuntime::new(&database);
    let Ok(snapshot) = runtime.status(&scope, now).await else {
        return Ok(None);
    };
    let Ok(fence) = snapshot.authorize_writer(DataAuthority::D1, snapshot.epoch) else {
        return Ok(None);
    };
    Ok(Some(MutationAuthorityFence::production(fence)))
}

async fn execute_mutation_batch(
    database: &D1Database,
    authority_fence: &MutationAuthorityFence,
    operation_id: &str,
    occurred_at_ms: i64,
    statements: Vec<D1PreparedStatement>,
) -> Result<Vec<D1Result>> {
    let Some(scoped) = authority_fence.scoped.as_ref() else {
        return database.batch(statements).await;
    };
    let occurred_at = storage_timestamp(occurred_at_ms)
        .ok_or_else(|| Error::RustError("cutover mutation clock is invalid".into()))?;
    cutover_authority::D1CutoverAuthorityRepository::new(database)
        .execute_fenced_batch_results(operation_id, scoped, occurred_at, statements)
        .await
        .map_err(|error| {
            console_error!("scoped mutation rejected class={}", error.code());
            Error::RustError("scoped mutation authority rejected".into())
        })
}

fn d1_mutation_pair(row: &AuthorityRow) -> bool {
    matches!(
        (row.phase.as_str(), row.authority.as_str()),
        ("d1_authoritative" | "finalized", "d1")
    )
}

fn tenant_header(request: &Request) -> Result<Option<String>> {
    Ok(request
        .headers()
        .get("x-frame-tenant-id")?
        .filter(|value| valid_uuid(value)))
}

fn supported_source_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "video/mp4" | "video/quicktime" | "video/webm" | "video/x-matroska"
    )
}

fn supported_native_source_content_type(content_type: &str) -> bool {
    supported_source_content_type(content_type)
        || matches!(
            content_type,
            "audio/mpeg" | "audio/mp4" | "audio/wav" | "audio/webm" | "audio/ogg"
        )
}

fn valid_private_object_key(key: &str, tenant_id: &str, video_id: &str) -> bool {
    key.starts_with(&format!("tenants/{tenant_id}/videos/{video_id}/"))
        && !key.contains("..")
        && !key.contains(['\\', '?', '#', '%'])
}

fn valid_worker_output_key(job: &WorkerJobRow, tenant_id: &str) -> bool {
    if job.source_version <= 0 {
        return false;
    }
    let prefix = format!(
        "tenants/{tenant_id}/videos/{}/derivatives/{}/",
        job.video_id, job.profile
    );
    job.output_object_key
        .strip_prefix(&prefix)
        .is_some_and(contracts::valid_sha256)
        && valid_private_object_key(&job.output_object_key, tenant_id, &job.video_id)
}

fn native_output_candidate_key(
    job: &WorkerJobRow,
    tenant_id: &str,
    checksum_sha256: &str,
) -> Option<String> {
    if !valid_worker_output_key(job, tenant_id)
        || !(1..=u16::MAX.into()).contains(&job.attempt)
        || !contracts::valid_sha256(checksum_sha256)
    {
        return None;
    }
    Some(format!(
        "{}.attempt-{}.{}.partial",
        job.output_object_key, job.attempt, checksum_sha256
    ))
}

fn idempotency_header(request: &Request) -> Result<String> {
    request
        .headers()
        .get("idempotency-key")?
        .filter(|value| valid_idempotency_key(value))
        .ok_or_else(|| Error::RustError("validated idempotency key is unavailable".into()))
}

fn public_collaboration_token(request: &Request) -> Result<Option<String>> {
    Ok(request
        .headers()
        .get("authorization")?
        .and_then(|value| value.strip_prefix("FrameShare ").map(str::to_owned))
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }))
}

fn worker_lease_token_header(request: &Request) -> Result<String> {
    request
        .headers()
        .get("x-frame-lease-token")?
        .filter(|value| valid_lease_token(value))
        .ok_or_else(|| Error::RustError("validated worker lease token is unavailable".into()))
}

fn worker_command_reservation(
    database: &D1Database,
    tenant_id: &str,
    idempotency_key: &str,
    command_type: &str,
    request_digest: &str,
    reservation_id: &str,
    now: i64,
) -> Result<D1PreparedStatement> {
    let expires_at_ms = now
        .checked_add(COMMAND_TTL_MS)
        .ok_or_else(|| Error::RustError("worker command expiry overflowed".into()))?;
    database
        .prepare(
            "INSERT INTO command_idempotency(organization_id, idempotency_key, command_type, \
               request_digest, response_status, response_json, created_at_ms, expires_at_ms, \
               reservation_id) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6, ?7) \
             ON CONFLICT(organization_id, idempotency_key) DO NOTHING",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(idempotency_key),
            JsValue::from_str(command_type),
            JsValue::from_str(request_digest),
            JsValue::from_f64(now as f64),
            JsValue::from_f64(expires_at_ms as f64),
            JsValue::from_str(reservation_id),
        ])
}

fn worker_command_reservation_cleanup(
    database: &D1Database,
    tenant_id: &str,
    idempotency_key: &str,
    reservation_id: &str,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "DELETE FROM command_idempotency WHERE organization_id = ?1 \
               AND idempotency_key = ?2 AND reservation_id = ?3 \
               AND response_status IS NULL AND response_json IS NULL",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(idempotency_key),
            JsValue::from_str(reservation_id),
        ])
}

fn current_time_ms() -> Result<i64> {
    let now = js_sys::Date::now().floor();
    if !now.is_finite() || !(0.0..=MAX_SAFE_INTEGER as f64).contains(&now) {
        return Err(Error::RustError("runtime clock is invalid".into()));
    }
    Ok(now as i64)
}

fn new_id() -> String {
    Uuid::now_v7().to_string()
}

fn storage_tenant(value: &str) -> Option<TenantId> {
    TenantId::parse(value).ok()
}

fn storage_timestamp(value: i64) -> Option<TimestampMillis> {
    TimestampMillis::new(value).ok()
}

fn storage_context(
    tenant_id: TenantId,
    principal: &str,
    correlation_id: CorrelationId,
) -> StorageGovernanceContextV1 {
    StorageGovernanceContextV1::new(
        tenant_id,
        correlation_id,
        ChecksumSha256::digest_bytes(principal.as_bytes()),
    )
}

fn storage_member_actor(
    tenant_id: TenantId,
    actor: &AuthenticatedActor,
    role: StorageMemberRole,
) -> Option<StorageActor> {
    Some(StorageActor::Member {
        tenant_id,
        user_id: UserId::parse(&actor.user_id).ok()?,
        role,
    })
}

fn storage_origin(config: &RuntimeConfig) -> String {
    let scheme = if config.production() { "https" } else { "http" };
    format!("{scheme}://{}", config.host_policy.public_host)
}

async fn governed_object(
    database: &D1Database,
    tenant_id: TenantId,
    object_key: &str,
    principal: &str,
) -> std::result::Result<Option<GovernedObject>, ()> {
    let object_id = GovernedObjectId::parse(object_key).map_err(|_| ())?;
    storage_governance_runtime::D1StorageGovernanceRepository::new(database)
        .governed_object(
            storage_context(tenant_id, principal, CorrelationId::new()),
            &object_id,
        )
        .await
        .map_err(|_| ())
}

fn storage_policy_error(
    _error: StorageGovernanceServiceError,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    failure_response(not_found_failure(), request_id, production)
}

fn storage_command_error(
    error: StorageGovernanceServiceError,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let failure = match error {
        StorageGovernanceServiceError::StateConflict => ApiFailure::new(
            409,
            "storage_state_conflict",
            "Storage authority changed concurrently.",
            true,
        ),
        StorageGovernanceServiceError::SigningUnavailable
        | StorageGovernanceServiceError::Unavailable
        | StorageGovernanceServiceError::InvalidConfiguration => storage_unavailable_failure(),
        StorageGovernanceServiceError::Denied(_)
        | StorageGovernanceServiceError::InvalidRequest
        | StorageGovernanceServiceError::Contract(_) => not_found_failure(),
    };
    failure_response(failure, request_id, production)
}

fn require_batch_success(results: Vec<D1Result>) -> Result<()> {
    if results.is_empty() || results.iter().any(|result| !result.success()) {
        return Err(Error::RustError("database command batch failed".into()));
    }
    Ok(())
}

fn classify_atomic_changes(changes: &[usize]) -> std::result::Result<bool, ()> {
    if changes.is_empty() {
        return Err(());
    }
    if changes.iter().all(|changes| *changes == 1) {
        return Ok(true);
    }
    if changes.iter().all(|changes| *changes == 0) {
        return Ok(false);
    }
    Err(())
}

fn atomic_batch_applied(results: Vec<D1Result>) -> Result<bool> {
    if results.len() != 3 || results.iter().any(|result| !result.success()) {
        return Err(Error::RustError("atomic database command failed".into()));
    }
    let changes = results
        .iter()
        .map(|result| {
            result
                .meta()?
                .and_then(|meta| meta.changes)
                .ok_or_else(|| Error::RustError("database change metadata is unavailable".into()))
        })
        .collect::<Result<Vec<_>>>()?;
    classify_atomic_changes(&changes)
        .map_err(|()| Error::RustError("atomic database command was partially applied".into()))
}

fn json_response<T: Serialize>(value: &T, status: u16, location: Option<&str>) -> Result<Response> {
    let mut response = Response::from_json(value)?.with_status(status);
    if let Some(location) = location {
        response.headers_mut().set("location", location)?;
    }
    Ok(response)
}

fn public_collaboration_response<T: Serialize>(
    outcome: public_collaboration_runtime::PublicOutcome<T>,
    success_status: u16,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    match outcome {
        Ok(value) => json_response(&value, success_status, None),
        Err(public_collaboration_runtime::PublicCollaborationFailure::Unavailable) => {
            failure_response(not_found_failure(), request_id, production)
        }
        Err(failure) => failure_response(
            ApiFailure::new(
                failure.status(),
                failure.code(),
                "The public collaboration request could not be applied.",
                failure == public_collaboration_runtime::PublicCollaborationFailure::RateLimited,
            ),
            request_id,
            production,
        ),
    }
}

fn stored_json_response(status: u16, json: &str) -> Result<Response> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| Error::RustError("stored command response is invalid".into()))?;
    let location = value
        .get("upload_path")
        .or_else(|| value.get("status_path"))
        .or_else(|| value.get("public_share_path"))
        .and_then(serde_json::Value::as_str);
    json_response(&value, status, location)
}

const fn mutation_disabled_failure() -> ApiFailure {
    ApiFailure::new(
        503,
        "mutation_authority_disabled",
        "Mutations are disabled for the current authority phase.",
        true,
    )
}

const fn idempotency_conflict_failure() -> ApiFailure {
    ApiFailure::new(
        409,
        "idempotency_conflict",
        "The idempotency key was already used for a different command.",
        false,
    )
}

const fn revision_conflict_failure() -> ApiFailure {
    ApiFailure::new(
        409,
        "revision_conflict",
        "The video changed before the privacy update was applied.",
        true,
    )
}

const fn storage_unavailable_failure() -> ApiFailure {
    ApiFailure::new(
        503,
        "storage_unavailable",
        "Storage is temporarily unavailable.",
        true,
    )
}

const fn native_worker_unavailable_failure() -> ApiFailure {
    ApiFailure::new(
        503,
        "native_worker_unavailable",
        "The native media worker protocol is unavailable in this runtime.",
        true,
    )
}

const fn worker_lease_conflict_failure() -> ApiFailure {
    ApiFailure::new(
        409,
        "lease_conflict",
        "The media job lease is unavailable or expired.",
        true,
    )
}

const fn worker_cancelled_failure() -> ApiFailure {
    ApiFailure::new(
        409,
        "cancellation_requested",
        "Cancellation was requested for this media job.",
        false,
    )
}

fn storage_preflight_response(
    env: &Env,
    request: &Request,
    canonical_origin: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let Some(origin) = request.headers().get("origin")? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let requested_method = match request
        .headers()
        .get("access-control-request-method")?
        .as_deref()
    {
        Some("GET") => StorageHttpMethod::Get,
        Some("HEAD") => StorageHttpMethod::Head,
        _ => {
            return failure_response(
                ApiFailure::new(
                    403,
                    "origin_forbidden",
                    "The request origin is not permitted.",
                    false,
                ),
                request_id,
                production,
            );
        }
    };
    let requested_headers = request
        .headers()
        .get("access-control-request-headers")?
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let requested_header_refs = requested_headers
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let allowed_origins =
        storage_governance_runtime::storage_allowed_origins(env, canonical_origin)
            .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    let policy = match StorageResponsePolicy::for_preflight(
        &origin,
        requested_method,
        &requested_header_refs,
        &allowed_origins,
    ) {
        Ok(policy) => policy,
        Err(_) => {
            return failure_response(
                ApiFailure::new(
                    403,
                    "origin_forbidden",
                    "The request origin is not permitted.",
                    false,
                ),
                request_id,
                production,
            );
        }
    };
    let response = secure_response(Response::empty()?.with_status(204), request_id, production)?;
    apply_storage_policy_headers(response, &policy)
}

async fn storage_grant_create_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    body: CreateStorageGrantRequest,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_text) =
        authorized_tenant(&database, request, actor, RequiredAccess::Write).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_text).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    if body.tenant_id != tenant_text
        || !(1..=MAX_SIGNED_GRANT_LIFETIME_MS).contains(&body.lifetime_ms)
    {
        return failure_response(
            invalid_body_failure("invalid_storage_grant"),
            request_id,
            config.production(),
        );
    }
    let Some(tenant_id) = storage_tenant(&tenant_text) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let object_id = match GovernedObjectId::parse(body.object_key) {
        Ok(object_id) => object_id,
        Err(_) => return failure_response(not_found_failure(), request_id, config.production()),
    };
    let operation = match body.operation.as_str() {
        "read" => StorageOperation::Read,
        "read_range" => StorageOperation::ReadRange,
        _ => {
            return failure_response(
                invalid_body_failure("invalid_storage_grant_operation"),
                request_id,
                config.production(),
            );
        }
    };
    let now = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?;
    let expires_at = storage_timestamp(
        now.get()
            .checked_add(body.lifetime_ms)
            .ok_or_else(|| Error::RustError("storage grant expiry is invalid".into()))?,
    )
    .ok_or_else(|| Error::RustError("storage grant expiry is invalid".into()))?;
    let correlation_id = CorrelationId::new();
    let context = storage_context(tenant_id, &actor.user_id, correlation_id);
    let repository = storage_governance_runtime::D1StorageGovernanceRepository::with_cutover_fence(
        &database,
        authority_fence.scoped.clone(),
        now,
        format!("storage-grant-issue:{correlation_id}"),
    )
    .map_err(|_| Error::RustError("storage mutation fence is invalid".into()))?;
    let Some(object) = repository
        .governed_object(context.clone(), &object_id)
        .await
        .map_err(|_| Error::RustError("storage authority is unavailable".into()))?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(storage_actor) = storage_member_actor(tenant_id, actor, StorageMemberRole::Editor)
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let governance = storage_governance_runtime::governance_service(env, &storage_origin(config))
        .map_err(|error| Error::RustError(error.to_string()))?;
    let issued = match governance
        .issue_read_grant(
            &repository,
            &storage_governance_runtime::RuntimeGrantSecretGenerator,
            context,
            storage_actor,
            &object,
            operation,
            now,
            expires_at,
        )
        .await
    {
        Ok(issued) => issued,
        Err(error) => return storage_command_error(error, request_id, config.production()),
    };
    let path = format!(
        "/api/v1/storage/tenants/{tenant_id}/grants/{}",
        issued.grant_id()
    );
    json_response(
        &CreateStorageGrantResponse {
            schema_version: API_SCHEMA_VERSION,
            grant_id: issued.grant_id().to_string(),
            token: issued.opaque_token(),
            expires_at_ms: issued.expires_at().get(),
            path: path.clone(),
        },
        201,
        Some(&path),
    )
}

async fn storage_grant_revoke_response(
    env: &Env,
    config: &RuntimeConfig,
    request: &Request,
    actor: &AuthenticatedActor,
    grant_id: &str,
    request_id: &str,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(tenant_text) =
        authorized_tenant(&database, request, actor, RequiredAccess::Admin).await?
    else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let Some(authority_fence) = mutation_authority_fence(env, config, &tenant_text).await? else {
        return failure_response(mutation_disabled_failure(), request_id, config.production());
    };
    let Some(tenant_id) = storage_tenant(&tenant_text) else {
        return failure_response(not_found_failure(), request_id, config.production());
    };
    let grant_id = match SignedGrantId::parse(grant_id) {
        Ok(grant_id) => grant_id,
        Err(_) => return failure_response(not_found_failure(), request_id, config.production()),
    };
    let now = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?;
    let repository = storage_governance_runtime::D1StorageGovernanceRepository::with_cutover_fence(
        &database,
        authority_fence.scoped.clone(),
        now,
        format!("storage-grant-revoke:{grant_id}"),
    )
    .map_err(|_| Error::RustError("storage mutation fence is invalid".into()))?;
    let governance = storage_governance_runtime::governance_service(env, &storage_origin(config))
        .map_err(|error| Error::RustError(error.to_string()))?;
    if let Err(error) = governance
        .revoke_read_grant(
            &repository,
            storage_context(tenant_id, &actor.user_id, CorrelationId::new()),
            grant_id,
            now,
        )
        .await
    {
        return storage_command_error(error, request_id, config.production());
    }
    Ok(Response::empty()?.with_status(204))
}

#[allow(clippy::too_many_arguments)]
async fn storage_grant_read_response(
    env: &Env,
    request: &Request,
    tenant_text: &str,
    grant_text: &str,
    canonical_origin: &str,
    primary_host: &str,
    head_only: bool,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let tenant_id = match storage_tenant(tenant_text) {
        Some(tenant_id) if tenant_id.to_string() == tenant_text => tenant_id,
        _ => return failure_response(not_found_failure(), request_id, production),
    };
    let grant_id = match SignedGrantId::parse(grant_text) {
        Ok(grant_id) => grant_id,
        Err(_) => return failure_response(not_found_failure(), request_id, production),
    };
    let Some(token) = request
        .headers()
        .get("authorization")?
        .and_then(|value| value.strip_prefix("FrameStorage ").map(str::to_owned))
        .filter(|value| {
            (64..=256).contains(&value.len())
                && value.len().is_multiple_of(2)
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let database = env.d1("DB")?;
    let repository = storage_governance_runtime::D1StorageGovernanceRepository::new(&database);
    let context = storage_context(tenant_id, grant_text, CorrelationId::new());
    let Some(grant) = repository
        .signed_grant(context.clone(), grant_id)
        .await
        .map_err(|_| Error::RustError("storage authority is unavailable".into()))?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(object) = repository
        .governed_object(context.clone(), grant.object_id())
        .await
        .map_err(|_| Error::RustError("storage authority is unavailable".into()))?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(content_type) = database
        .prepare(
            "SELECT content_type FROM storage_governed_objects_v1 \
              WHERE organization_id = ?1 AND object_key = ?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_text),
            JsValue::from_str(object.object_id().as_str()),
        ])?
        .first::<GovernedContentTypeRow>(None)
        .await?
        .map(|row| row.content_type)
        .filter(|value| valid_content_type(value))
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let operation = if request.headers().get("range")?.is_some() {
        StorageOperation::ReadRange
    } else {
        StorageOperation::Read
    };
    let authority = canonical_origin
        .split_once("://")
        .map(|(_, authority)| authority)
        .ok_or_else(|| Error::RustError("storage request origin is invalid".into()))?;
    let custom = authority != primary_host
        && !(authority
            .strip_prefix(primary_host)
            .is_some_and(|suffix| suffix.starts_with(':')));
    let (surface, request_domain) = if custom {
        (StorageAccessSurface::CustomDomain, Some(authority))
    } else {
        (StorageAccessSurface::SignedRoute, None)
    };
    let governance = storage_governance_runtime::governance_service(env, canonical_origin)
        .map_err(|error| Error::RustError(error.to_string()))?;
    let authorized = match governance
        .authorize_persisted_read(
            &repository,
            context,
            &object,
            grant_id,
            &token,
            operation,
            surface,
            request_domain,
            storage_timestamp(current_time_ms()?)
                .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?,
            &content_type,
            request.headers().get("origin")?.as_deref(),
            true,
        )
        .await
    {
        Ok(authorized) => authorized,
        Err(error) => return storage_policy_error(error, request_id, production),
    };
    let expected_checksum = parse_sha256(object.checksum().as_str())
        .ok_or_else(|| Error::RustError("storage checksum is invalid".into()))?;
    let bytes = object.size().get();
    let bucket = env.bucket("RECORDINGS")?;
    let Some(head) = bucket.head(object.object_id().as_str()).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let metadata = head.http_metadata();
    if head.size() != bytes
        || head.checksum().sha256.as_deref() != Some(expected_checksum.as_slice())
        || metadata.content_type.as_deref() != Some(content_type.as_str())
        || metadata.content_encoding.is_some()
    {
        return failure_response(
            media_unavailable_failure("media_unavailable"),
            request_id,
            production,
        );
    }
    let etag = head.http_etag();
    let mut requested_range =
        match parse_range_header(request.headers().get("range")?.as_deref(), bytes) {
            Ok(range) => range,
            Err(()) => return range_not_satisfiable(bytes, request_id, production),
        };
    if requested_range.is_some()
        && request
            .headers()
            .get("if-range")?
            .is_some_and(|candidate| candidate.trim() != etag)
    {
        requested_range = None;
    }
    let response_object = PublicObject {
        key: object.object_id().as_str().to_owned(),
        content_type,
        bytes,
        checksum: object.checksum().clone(),
        governed: object,
    };
    if requested_range.is_none()
        && request
            .headers()
            .get("if-none-match")?
            .is_some_and(|candidate| candidate.trim() == etag)
    {
        let mut response = Response::empty()?.with_status(304);
        response.headers_mut().set("etag", &etag)?;
        let response = secure_response(response, request_id, production)?;
        return apply_storage_headers(response, &authorized);
    }
    if head_only {
        return media_response(
            Response::empty()?,
            &response_object,
            &etag,
            requested_range.as_ref(),
            &authorized,
            request_id,
            production,
        );
    }
    let fetched = match requested_range.as_ref() {
        Some(range) => {
            bucket
                .get(&response_object.key)
                .range(range.range.clone())
                .execute()
                .await?
        }
        None => bucket.get(&response_object.key).execute().await?,
    };
    let Some(fetched) = fetched.filter(|value| {
        value.size() == bytes
            && value.checksum().sha256.as_deref() == Some(expected_checksum.as_slice())
            && value.http_etag() == etag
    }) else {
        return failure_response(
            media_unavailable_failure("media_changed"),
            request_id,
            production,
        );
    };
    let body = fetched
        .body()
        .ok_or_else(|| Error::RustError("R2 returned no storage body".into()))?
        .response_body()?;
    media_response(
        Response::from_body(body)?,
        &response_object,
        &etag,
        requested_range.as_ref(),
        &authorized,
        request_id,
        production,
    )
}

async fn public_share_response(
    env: &Env,
    share_id: &str,
    canonical_origin: &str,
) -> Result<Response> {
    let summary = if valid_uuid(share_id) {
        public_share_row(env, share_id)
            .await?
            .as_ref()
            .map_or_else(unavailable_share, |row| {
                public_summary(row, canonical_origin)
            })
    } else {
        unavailable_share()
    };
    Response::from_json(&summary)
}

async fn public_share_row(env: &Env, share_id: &str) -> Result<Option<PublicShareRow>> {
    env.d1("DB")?
        .prepare(
            "SELECT v.id, v.title, v.state, v.privacy, v.organization_id, \
                    v.playback_object_key, v.duration_ms, g.content_type, g.bytes, \
                    g.checksum_sha256, g.immutable_revision AS object_version, \
                    g.role AS governed_role, g.visibility AS governed_visibility, \
                    g.state AS governed_state, g.malware_disposition, g.cache_generation, \
                    (SELECT f.state FROM instant_finalize_requests_v1 f \
                      WHERE f.video_id=v.id AND f.organization_id=v.organization_id \
                      ORDER BY f.updated_at_ms DESC,f.session_id DESC LIMIT 1) \
                      AS instant_finalize_state, \
                    (SELECT f.last_failure_class FROM instant_finalize_requests_v1 f \
                      WHERE f.video_id=v.id AND f.organization_id=v.organization_id \
                      ORDER BY f.updated_at_ms DESC,f.session_id DESC LIMIT 1) \
                      AS instant_finalize_failure_class \
             FROM videos v \
             LEFT JOIN object_manifests om \
               ON om.object_key = v.playback_object_key AND om.state = 'available' \
             LEFT JOIN storage_governed_objects_v1 g \
               ON g.organization_id = v.organization_id AND g.object_key = v.playback_object_key \
              AND g.checksum_sha256 = om.checksum_sha256 AND g.bytes = om.bytes \
             WHERE v.id = ?1 AND v.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(share_id)])?
        .first::<PublicShareRow>(None)
        .await
}

fn unavailable_share() -> PublicShareSummary {
    PublicShareSummary {
        api_version: ApiVersion::current(),
        availability: ShareAvailability::Unavailable,
        title: None,
        description: None,
        canonical_url: None,
        duration_ms: None,
        playback: None,
        processing_status: None,
    }
}

fn public_summary(row: &PublicShareRow, canonical_origin: &str) -> PublicShareSummary {
    let canonical_url = format!("{canonical_origin}/s/{}", row.id);
    if !matches!(row.privacy.as_str(), "public" | "unlisted") {
        return unavailable_share();
    }
    if row.state == "processing" {
        let processing_status = match (
            row.instant_finalize_state.as_deref(),
            row.instant_finalize_failure_class.as_deref(),
        ) {
            (None, None) => None,
            (Some("pending"), None) => Some(InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: false,
                error: None,
            }),
            (
                Some("pending"),
                Some("dependency_pending" | "authority_unavailable" | "persistence"),
            ) => Some(InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: true,
                error: Some(InstantUiErrorCodeV1::FinalizeDelayed),
            }),
            _ => return unavailable_share(),
        };
        return PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Processing,
            title: None,
            description: None,
            canonical_url: Some(canonical_url),
            duration_ms: None,
            playback: None,
            processing_status,
        };
    }
    let Some(object) = validated_public_object(row) else {
        return unavailable_share();
    };
    let duration_ms = match row.duration_ms {
        Some(value) if (0..=86_400_000).contains(&value) => u64::try_from(value).ok(),
        None => None,
        Some(_) => return unavailable_share(),
    };
    PublicShareSummary {
        api_version: ApiVersion::current(),
        availability: ShareAvailability::Public,
        title: Some(sanitized_public_title(&row.title)),
        description: None,
        canonical_url: Some(canonical_url),
        duration_ms,
        playback: Some(PlaybackDescriptor {
            path: format!("/api/v1/public/shares/{}/media", row.id),
            content_type: object.content_type,
            supports_range: true,
            captions: Vec::<CaptionTrack>::new(),
        }),
        processing_status: None,
    }
}

fn validated_public_object(row: &PublicShareRow) -> Option<PublicObject> {
    if row.state != "ready" || row.privacy != "public" {
        return None;
    }
    let organization_id = row.organization_id.as_deref().filter(|id| valid_uuid(id))?;
    let tenant_id = storage_tenant(organization_id)?;
    let key = row.playback_object_key.as_deref()?;
    let expected_prefix = format!("tenants/{organization_id}/videos/{}/", row.id);
    if !key.starts_with(&expected_prefix)
        || !key.contains("/derivatives/")
        || key.contains("..")
        || key.contains(['\\', '?', '#', '%'])
    {
        return None;
    }
    let content_type = row
        .content_type
        .as_deref()
        .filter(|value| valid_content_type(value) && value.starts_with("video/"))?;
    let bytes = u64::try_from(row.bytes?).ok()?;
    if bytes == 0 || bytes > MAX_SAFE_INTEGER {
        return None;
    }
    let checksum = ChecksumSha256::parse(row.checksum_sha256.clone()?).ok()?;
    let role = match row.governed_role.as_deref()? {
        "preview" => GovernedObjectRole::Preview,
        "thumbnail" => GovernedObjectRole::Thumbnail,
        "spritesheet" => GovernedObjectRole::Spritesheet,
        "audio" => GovernedObjectRole::Audio,
        _ => return None,
    };
    if row.governed_visibility.as_deref() != Some("public")
        || row.governed_state.as_deref() != Some("active")
        || row.malware_disposition.as_deref() != Some("clean")
    {
        return None;
    }
    let governed = GovernedObject::new(
        tenant_id,
        GovernedObjectId::parse(key).ok()?,
        role,
        ObjectVisibility::Public,
        GovernedObjectState::Active,
        MalwareDisposition::Clean,
        u64::try_from(row.object_version?).ok()?,
        u64::try_from(row.cache_generation?).ok()?,
        checksum.clone(),
        ByteSize::new(bytes).ok()?,
        None,
    )
    .ok()?;
    Some(PublicObject {
        key: key.to_owned(),
        content_type: content_type.to_owned(),
        bytes,
        checksum,
        governed,
    })
}

#[allow(clippy::too_many_arguments)]
async fn public_media_response(
    env: &Env,
    request: &Request,
    share_id: &str,
    canonical_origin: &str,
    primary_host: &str,
    head_only: bool,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    if !valid_uuid(share_id) {
        return failure_response(not_found_failure(), request_id, production);
    }
    let Some(row) = public_share_row(env, share_id).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let Some(public) = validated_public_object(&row) else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let request_domain = canonical_origin
        .split_once("://")
        .and_then(|(_, authority)| CustomDomainName::parse(authority).ok());
    let Some(request_domain) = request_domain else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let domain_binding = if request_domain.as_str() == primary_host {
        VerifiedCustomDomain::new(public.governed.tenant_id(), request_domain.clone(), 1, true)
            .map_err(|_| Error::RustError("public domain binding is invalid".into()))?
    } else {
        let database = env.d1("DB")?;
        let binding = storage_governance_runtime::D1StorageGovernanceRepository::new(&database)
            .verified_domain(request_domain.as_str())
            .await
            .map_err(|_| Error::RustError("custom domain authority is unavailable".into()))?;
        let Some(binding) = binding.filter(|binding| {
            binding.active() && binding.tenant_id() == public.governed.tenant_id()
        }) else {
            return failure_response(not_found_failure(), request_id, production);
        };
        binding
    };
    let governance = storage_governance_runtime::governance_service(env, canonical_origin)
        .map_err(|_| Error::RustError("storage governance configuration is invalid".into()))?;
    let operation = if request.headers().get("range")?.is_some() {
        StorageOperation::ReadRange
    } else {
        StorageOperation::Read
    };
    let authorized = match governance.authorize_read(
        CorrelationId::new(),
        StorageAccessRequest {
            actor: StorageActor::Anonymous,
            operation,
            surface: StorageAccessSurface::CustomDomain,
            object: &public.governed,
            now: storage_timestamp(current_time_ms()?)
                .ok_or_else(|| Error::RustError("storage clock is invalid".into()))?,
            grant: None,
            grant_proof: None,
            request_domain: Some(&request_domain),
            custom_domain: Some(&domain_binding),
        },
        &public.content_type,
        request.headers().get("origin")?.as_deref(),
        false,
    ) {
        Ok(authorized) => authorized,
        Err(error) => return storage_policy_error(error, request_id, production),
    };
    let expected_checksum = parse_sha256(public.checksum.as_str())
        .ok_or_else(|| Error::RustError("public checksum is invalid".into()))?;
    let bucket = env.bucket("RECORDINGS")?;
    let Some(head) = bucket.head(&public.key).await? else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let metadata = head.http_metadata();
    if head.size() != public.bytes
        || head.checksum().sha256.as_deref() != Some(expected_checksum.as_slice())
        || metadata.content_type.as_deref() != Some(public.content_type.as_str())
        || metadata.content_encoding.is_some()
    {
        return failure_response(
            media_unavailable_failure("media_unavailable"),
            request_id,
            production,
        );
    }
    let mut requested_range =
        match parse_range_header(request.headers().get("range")?.as_deref(), public.bytes) {
            Ok(range) => range,
            Err(()) => return range_not_satisfiable(public.bytes, request_id, production),
        };
    let etag = head.http_etag();
    if requested_range.is_some()
        && request
            .headers()
            .get("if-range")?
            .is_some_and(|candidate| candidate.trim() != etag)
    {
        requested_range = None;
    }
    if requested_range.is_none()
        && request
            .headers()
            .get("if-none-match")?
            .is_some_and(|candidate| candidate.trim() == etag)
    {
        let mut response = Response::empty()?.with_status(304);
        response.headers_mut().set("etag", &etag)?;
        let response = secure_response(response, request_id, production)?;
        return apply_storage_headers(response, &authorized);
    }

    if head_only {
        return media_response(
            Response::empty()?,
            &public,
            &etag,
            requested_range.as_ref(),
            &authorized,
            request_id,
            production,
        );
    }
    let object = match requested_range.as_ref() {
        Some(range) => {
            bucket
                .get(&public.key)
                .range(range.range.clone())
                .execute()
                .await?
        }
        None => bucket.get(&public.key).execute().await?,
    };
    let Some(object) = object.filter(|object| {
        object.size() == public.bytes
            && object.checksum().sha256.as_deref() == Some(expected_checksum.as_slice())
    }) else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let object_etag = object.http_etag();
    if object_etag != etag {
        return failure_response(
            media_unavailable_failure("media_changed"),
            request_id,
            production,
        );
    }
    let body = object
        .body()
        .ok_or_else(|| Error::RustError("R2 returned no media body".into()))?
        .response_body()?;
    media_response(
        Response::from_body(body)?,
        &public,
        &etag,
        requested_range.as_ref(),
        &authorized,
        request_id,
        production,
    )
}

const fn media_unavailable_failure(code: &'static str) -> ApiFailure {
    ApiFailure::new(503, code, "The media is temporarily unavailable.", true)
}

fn media_response(
    mut response: Response,
    public: &PublicObject,
    etag: &str,
    range: Option<&RequestedRange>,
    authorized: &frame_application::AuthorizedObjectRead,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let content_length = range.map_or(public.bytes, |range| range.length);
    VerifiedRangeResponse::new(
        if range.is_some() { 206 } else { 200 },
        content_length,
        range.map(|range| (range.start, range.start + range.length)),
        public.bytes,
    )
    .map_err(|_| Error::RustError("provider range response is invalid".into()))?;
    if let Some(range) = range {
        response = response.with_status(206);
        response.headers_mut().set(
            "content-range",
            &format!(
                "bytes {}-{}/{}",
                range.start,
                range.start + range.length - 1,
                public.bytes
            ),
        )?;
    }
    let headers = response.headers_mut();
    headers.set("accept-ranges", "bytes")?;
    headers.set("content-length", &content_length.to_string())?;
    headers.set("content-type", &public.content_type)?;
    headers.set("content-disposition", "inline")?;
    headers.set("etag", etag)?;
    let response = secure_response(response, request_id, production)?;
    apply_storage_headers(response, authorized)
}

fn apply_storage_headers(
    mut response: Response,
    authorized: &frame_application::AuthorizedObjectRead,
) -> Result<Response> {
    for (name, value) in authorized.headers() {
        response.headers_mut().set(name, value)?;
    }
    Ok(response)
}

fn apply_storage_policy_headers(
    mut response: Response,
    policy: &StorageResponsePolicy,
) -> Result<Response> {
    for (name, value) in policy.headers() {
        response.headers_mut().set(name, value)?;
    }
    Ok(response)
}

fn range_not_satisfiable(bytes: u64, request_id: &str, production: bool) -> Result<Response> {
    let mut response = failure_response(
        ApiFailure::new(
            416,
            "range_not_satisfiable",
            "The requested byte range is not satisfiable.",
            false,
        ),
        request_id,
        production,
    )?;
    response
        .headers_mut()
        .set("content-range", &format!("bytes */{bytes}"))?;
    Ok(response)
}

fn parse_range_header(
    value: Option<&str>,
    size: u64,
) -> std::result::Result<Option<RequestedRange>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let range = value.strip_prefix("bytes=").ok_or(())?;
    if range.contains(',') || range.bytes().any(|byte| byte.is_ascii_whitespace()) || size == 0 {
        return Err(());
    }
    let (start, end) = range.split_once('-').ok_or(())?;
    if start.is_empty() {
        let requested = end.parse::<u64>().map_err(|_| ())?;
        if requested == 0 {
            return Err(());
        }
        let length = requested.min(size);
        return Ok(Some(RequestedRange {
            range: worker::Range::Suffix { suffix: length },
            start: size - length,
            length,
        }));
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= size || start > MAX_SAFE_INTEGER {
        return Err(());
    }
    let requested_end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(size - 1)
    };
    if requested_end < start || requested_end > MAX_SAFE_INTEGER {
        return Err(());
    }
    let length = requested_end - start + 1;
    let range = if end.is_empty() {
        worker::Range::OffsetToEnd { offset: start }
    } else {
        worker::Range::OffsetWithLength {
            offset: start,
            length,
        }
    };
    Ok(Some(RequestedRange {
        range,
        start,
        length,
    }))
}

async fn authority_response(env: &Env) -> Result<Response> {
    let row = env
        .d1("DB")?
        .prepare("SELECT phase, authority, epoch FROM authority_state WHERE singleton = 1")
        .first::<AuthorityRow>(None)
        .await?
        .ok_or_else(|| Error::RustError("authority state is unavailable".into()))?;
    if !matches!(
        row.phase.as_str(),
        "legacy_authoritative"
            | "shadow_read"
            | "dual_write"
            | "d1_authoritative"
            | "rolled_back"
            | "finalized"
    ) || !matches!(row.authority.as_str(), "legacy" | "dual_write" | "d1")
        || !(0..=i64::try_from(MAX_SAFE_INTEGER).expect("safe integer fits i64"))
            .contains(&row.epoch)
    {
        return Err(Error::RustError("authority state is invalid".into()));
    }
    // This Worker has no legacy adapter. Dual-write is therefore deliberately
    // fail-closed until both authorities and durable outcome reconciliation exist.
    let mutations_enabled = d1_mutation_pair(&row);
    Response::from_json(&AuthorityResponse {
        schema_version: API_SCHEMA_VERSION,
        phase: row.phase,
        authority: row.authority,
        epoch: u64::try_from(row.epoch)
            .map_err(|_| Error::RustError("authority epoch is invalid".into()))?,
        mutations_enabled,
    })
}

async fn authorized_cutover_scope(
    database: &D1Database,
    request: &Request,
    actor: &AuthenticatedActor,
    tenant_text: &str,
    domain_text: &str,
) -> Result<Option<CutoverScope>> {
    let Some(authorized_tenant) =
        authorized_tenant(database, request, actor, RequiredAccess::Admin).await?
    else {
        return Ok(None);
    };
    if authorized_tenant != tenant_text {
        return Ok(None);
    }
    let Some(tenant_id) = storage_tenant(tenant_text) else {
        return Ok(None);
    };
    let Ok(domain) = CutoverDomain::parse(domain_text) else {
        return Ok(None);
    };
    Ok(Some(CutoverScope::new(tenant_id, domain)))
}

fn cutover_failure(error: CutoverAuthorityFailure) -> ApiFailure {
    match error {
        CutoverAuthorityFailure::InvalidRequest => ApiFailure::new(
            422,
            "cutover_invalid_request",
            "The cutover control request is invalid.",
            false,
        ),
        CutoverAuthorityFailure::NotFound => not_found_failure(),
        CutoverAuthorityFailure::StaleAuthority => ApiFailure::new(
            409,
            "cutover_authority_stale",
            "The cutover authority changed before the control was applied.",
            true,
        ),
        CutoverAuthorityFailure::MutationRejected => ApiFailure::new(
            409,
            "cutover_mutation_rejected",
            "The cutover control was rejected by its live state fence.",
            true,
        ),
        CutoverAuthorityFailure::Unavailable | CutoverAuthorityFailure::Corrupt => ApiFailure::new(
            503,
            "cutover_unavailable",
            "Cutover authority is temporarily unavailable.",
            true,
        ),
    }
}

fn cutover_snapshot_response(snapshot: CutoverAuthoritySnapshot) -> Result<Response> {
    Response::from_json(&CutoverAuthorityResponse {
        schema_version: API_SCHEMA_VERSION,
        authority: snapshot,
    })
}

async fn cutover_status_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    tenant_text: &str,
    domain_text: &str,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(scope) =
        authorized_cutover_scope(&database, request, actor, tenant_text, domain_text).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let now = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("cutover authority clock is invalid".into()))?;
    match cutover_authority_runtime::CutoverAuthorityRuntime::new(&database)
        .status(&scope, now)
        .await
    {
        Ok(snapshot) => cutover_snapshot_response(snapshot),
        Err(error) => failure_response(cutover_failure(error), request_id, production),
    }
}

#[allow(clippy::too_many_arguments)]
async fn cutover_transition_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    tenant_text: &str,
    domain_text: &str,
    body: CutoverTransitionRequest,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(scope) =
        authorized_cutover_scope(&database, request, actor, tenant_text, domain_text).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let occurred_at = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("cutover authority clock is invalid".into()))?;
    let operator_digest = digest_identifier("cutover_operator", &actor.user_id)
        .map_err(|()| Error::RustError("cutover operator identity is invalid".into()))?;
    let command = ApprovedCutoverTransition {
        scope,
        target: body.target,
        expected_epoch: body.expected_epoch,
        operator_digest,
        evidence: body.evidence.into(),
        reconciliation_digest: body.reconciliation_digest,
        occurred_at,
    };
    match cutover_authority_runtime::CutoverAuthorityRuntime::new(&database)
        .transition(&command)
        .await
    {
        Ok(snapshot) => cutover_snapshot_response(snapshot),
        Err(error) => failure_response(cutover_failure(error), request_id, production),
    }
}

#[allow(clippy::too_many_arguments)]
async fn cutover_replay_control_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    tenant_text: &str,
    domain_text: &str,
    body: CutoverReplayControlRequest,
    action: ReplayControlAction,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(scope) =
        authorized_cutover_scope(&database, request, actor, tenant_text, domain_text).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let occurred_at = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("cutover authority clock is invalid".into()))?;
    let operator_digest = digest_identifier("cutover_operator", &actor.user_id)
        .map_err(|()| Error::RustError("cutover operator identity is invalid".into()))?;
    let command = ApprovedReplayControl {
        scope,
        action,
        expected_epoch: body.expected_epoch,
        operator_digest,
        occurred_at,
    };
    match cutover_authority_runtime::CutoverAuthorityRuntime::new(&database)
        .replay_control(&command)
        .await
    {
        Ok(snapshot) => cutover_snapshot_response(snapshot),
        Err(error) => failure_response(cutover_failure(error), request_id, production),
    }
}

#[allow(clippy::too_many_arguments)]
async fn cutover_signal_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    tenant_text: &str,
    domain_text: &str,
    body: CutoverSignalRequest,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(scope) =
        authorized_cutover_scope(&database, request, actor, tenant_text, domain_text).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let occurred_at = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("cutover authority clock is invalid".into()))?;
    match cutover_authority_runtime::CutoverAuthorityRuntime::new(&database)
        .record_signal(
            &scope,
            body.expected_phase_epoch,
            body.kind.into(),
            occurred_at,
        )
        .await
    {
        Ok(()) => Ok(Response::empty()?.with_status(204)),
        Err(error) => failure_response(cutover_failure(error), request_id, production),
    }
}

#[allow(clippy::too_many_arguments)]
async fn cutover_shadow_observation_response(
    env: &Env,
    request: &Request,
    actor: &AuthenticatedActor,
    tenant_text: &str,
    domain_text: &str,
    body: CutoverShadowObservationRequest,
    request_id: &str,
    production: bool,
) -> Result<Response> {
    let database = env.d1("DB")?;
    let Some(scope) =
        authorized_cutover_scope(&database, request, actor, tenant_text, domain_text).await?
    else {
        return failure_response(not_found_failure(), request_id, production);
    };
    let observed_at = storage_timestamp(current_time_ms()?)
        .ok_or_else(|| Error::RustError("cutover authority clock is invalid".into()))?;
    let observation = CutoverShadowObservation {
        scope,
        phase_epoch: body.phase_epoch,
        observation_digest: body.observation_digest,
        query_class: body.query_class,
        normalization_digest: body.normalization_digest,
        legacy_result_digest: body.legacy_result_digest,
        d1_result_digest: body.d1_result_digest,
        classification: body.classification.into(),
        observed_at,
    };
    match cutover_authority_runtime::CutoverAuthorityRuntime::new(&database)
        .record_shadow_observation(&observation)
        .await
    {
        Ok(()) => Ok(Response::empty()?.with_status(204)),
        Err(error) => failure_response(cutover_failure(error), request_id, production),
    }
}

async fn authenticated_command_preflight(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    required: RequiredAccess,
) -> Result<std::result::Result<AuthenticatedActor, ApiFailure>> {
    if request.headers().get("cookie").ok().flatten().is_some() {
        return Ok(Err(ApiFailure::new(
            401,
            "unsupported_authentication",
            "This endpoint requires explicit bearer authentication.",
            false,
        )
        .with_authenticate()));
    }
    if request
        .headers()
        .get("origin")
        .ok()
        .flatten()
        .is_some_and(|origin| {
            !origin_allowed(
                &origin,
                &config.host_policy.public_host,
                config.host_policy.deployment == Deployment::Local,
            )
        })
    {
        return Ok(Err(ApiFailure::new(
            403,
            "origin_forbidden",
            "The request origin is not permitted.",
            false,
        )));
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .ok()
        .flatten()
        .is_some_and(|fetch_site| !matches!(fetch_site.as_str(), "same-origin" | "none"))
    {
        return Ok(Err(ApiFailure::new(
            403,
            "origin_forbidden",
            "The request origin is not permitted.",
            false,
        )));
    }

    let Some(authorization) = request
        .headers()
        .get("authorization")
        .map_err(|_| Error::RustError("authorization header is unavailable".into()))?
    else {
        return Ok(Err(unauthenticated_failure()));
    };
    let Some(token) = authorization.strip_prefix("Bearer ").filter(|token| {
        (32..=512).contains(&token.len())
            && token
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
    }) else {
        return Ok(Err(unauthenticated_failure()));
    };
    let now = current_time_ms()?;
    let digest = digest_credential(token);
    let Some(row) = env
        .d1("DB")?
        .prepare(
            "SELECT k.user_id, k.scopes_json FROM auth_api_keys k \
             JOIN users u ON u.id = k.user_id \
             WHERE k.key_digest = ?1 AND k.revoked_at_ms IS NULL \
               AND (k.expires_at_ms IS NULL OR k.expires_at_ms > ?2) \
               AND u.status = 'active' AND u.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(&digest), JsValue::from_f64(now as f64)])?
        .first::<ApiKeyRow>(None)
        .await?
    else {
        return Ok(Err(unauthenticated_failure()));
    };
    if !valid_uuid(&row.user_id) {
        return Err(Error::RustError("authenticated actor is invalid".into()));
    }
    let scopes = serde_json::from_str::<Vec<String>>(&row.scopes_json)
        .map_err(|_| Error::RustError("API key scopes are invalid".into()))?;
    if scopes.is_empty()
        || scopes.len() > 16
        || scopes
            .iter()
            .any(|scope| scope.len() > 64 || !scope.is_ascii())
    {
        return Err(Error::RustError("API key scopes are invalid".into()));
    }
    let actor = AuthenticatedActor {
        user_id: row.user_id,
        scopes,
    };
    if !actor.allows(required) {
        return Ok(Err(ApiFailure::new(
            403,
            "insufficient_scope",
            "The credential does not permit this operation.",
            false,
        )));
    }
    Ok(Ok(actor))
}

async fn authorized_tenant(
    database: &D1Database,
    request: &Request,
    actor: &AuthenticatedActor,
    required: RequiredAccess,
) -> Result<Option<String>> {
    if !actor.allows(required) {
        return Ok(None);
    }
    let Some(tenant_id) = tenant_header(request)? else {
        return Ok(None);
    };
    let Some(membership) = database
        .prepare(
            "SELECT m.role FROM organization_members m \
             JOIN organizations o ON o.id = m.organization_id \
             WHERE m.organization_id = ?1 AND m.user_id = ?2 \
               AND m.state = 'active' AND o.status = 'active' LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&tenant_id),
            JsValue::from_str(&actor.user_id),
        ])?
        .first::<MembershipRow>(None)
        .await?
    else {
        return Ok(None);
    };
    let permitted = match required {
        RequiredAccess::Read => matches!(
            membership.role.as_str(),
            "owner" | "admin" | "member" | "viewer"
        ),
        RequiredAccess::Write => matches!(membership.role.as_str(), "owner" | "admin" | "member"),
        RequiredAccess::Admin => matches!(membership.role.as_str(), "owner" | "admin"),
        RequiredAccess::Worker => matches!(
            membership.role.as_str(),
            "owner" | "admin" | "member" | "viewer"
        ),
    };
    Ok(permitted.then_some(tenant_id))
}

fn validate_storage_json_headers(request: &Request) -> std::result::Result<(), ApiFailure> {
    let content_type = request
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("unsupported_content_type"))?;
    if !matches!(
        content_type.as_str(),
        "application/json" | "application/json; charset=utf-8"
    ) {
        return Err(invalid_body_failure("unsupported_content_type"));
    }
    if request
        .headers()
        .get("content-encoding")
        .ok()
        .flatten()
        .is_some_and(|encoding| encoding != "identity")
    {
        return Err(invalid_body_failure("unsupported_content_encoding"));
    }
    let content_length = request
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=MAX_COMMAND_BODY_BYTES).contains(value));
    if content_length.is_none() {
        return Err(invalid_body_failure("invalid_content_length"));
    }
    Ok(())
}

fn validate_json_command_headers(request: &Request) -> std::result::Result<(), ApiFailure> {
    validate_idempotency_header(request)?;
    let content_type = request
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("unsupported_content_type"))?;
    if !matches!(
        content_type.as_str(),
        "application/json" | "application/json; charset=utf-8"
    ) {
        return Err(invalid_body_failure("unsupported_content_type"));
    }
    if request
        .headers()
        .get("content-encoding")
        .ok()
        .flatten()
        .is_some_and(|encoding| encoding != "identity")
    {
        return Err(invalid_body_failure("unsupported_content_encoding"));
    }
    let content_length = request
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .ok_or_else(|| {
            ApiFailure::new(
                411,
                "content_length_required",
                "A bounded content length is required.",
                false,
            )
        })?
        .parse::<u64>()
        .map_err(|_| invalid_body_failure("invalid_content_length"))?;
    if content_length == 0 || content_length > MAX_COMMAND_BODY_BYTES {
        return Err(ApiFailure::new(
            413,
            "payload_too_large",
            "The request body exceeds the allowed size.",
            false,
        ));
    }
    Ok(())
}

fn validate_idempotency_header(request: &Request) -> std::result::Result<(), ApiFailure> {
    let key = request
        .headers()
        .get("idempotency-key")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("missing_idempotency_key"))?;
    if !valid_idempotency_key(&key) {
        return Err(invalid_body_failure("invalid_idempotency_key"));
    }
    Ok(())
}

fn validate_worker_lease_header(request: &Request) -> std::result::Result<(), ApiFailure> {
    let token = request
        .headers()
        .get("x-frame-lease-token")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("missing_lease_token"))?;
    if !valid_lease_token(&token) {
        return Err(invalid_body_failure(
            contracts::ValidationCode::LeaseToken.as_str(),
        ));
    }
    Ok(())
}

fn validate_worker_json_headers(request: &Request) -> std::result::Result<(), ApiFailure> {
    validate_json_command_headers(request)?;
    validate_worker_lease_header(request)
}

fn validate_worker_output_headers(request: &Request) -> std::result::Result<(), ApiFailure> {
    validate_idempotency_header(request)?;
    validate_worker_lease_header(request)?;
    let content_type = request
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("unsupported_content_type"))?;
    if content_type != "image/png" {
        return Err(invalid_body_failure("unsupported_content_type"));
    }
    if request
        .headers()
        .get("content-encoding")
        .ok()
        .flatten()
        .is_some_and(|encoding| encoding != "identity")
    {
        return Err(invalid_body_failure("unsupported_content_encoding"));
    }
    let content_length = request
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .ok_or_else(|| {
            ApiFailure::new(
                411,
                "content_length_required",
                "A bounded content length is required.",
                false,
            )
        })?
        .parse::<u64>()
        .map_err(|_| invalid_body_failure("invalid_content_length"))?;
    if content_length == 0 || content_length > NATIVE_MAX_OUTPUT_BYTES {
        return Err(ApiFailure::new(
            413,
            "payload_too_large",
            "The output exceeds the allowed size.",
            false,
        ));
    }
    let checksum = request
        .headers()
        .get("x-content-sha256")
        .ok()
        .flatten()
        .ok_or_else(|| invalid_body_failure("missing_checksum"))?;
    if !contracts::valid_sha256(&checksum) {
        return Err(invalid_body_failure("invalid_checksum"));
    }
    Ok(())
}

fn invalid_body_failure(code: &'static str) -> ApiFailure {
    ApiFailure::new(
        code_status(code),
        code,
        "The request body is invalid.",
        false,
    )
}

fn code_status(code: &str) -> u16 {
    match code {
        "invalid_schema_version" => 422,
        _ => 400,
    }
}

const fn invalid_identifier_failure() -> ApiFailure {
    ApiFailure::new(
        404,
        "not_found",
        "The requested resource was not found.",
        false,
    )
}

const fn not_found_failure() -> ApiFailure {
    ApiFailure::new(
        404,
        "not_found",
        "The requested resource was not found.",
        false,
    )
}

const fn browser_web_failure(
    failure: browser_web_runtime::BrowserWebFailure,
    invalid_code: &'static str,
) -> ApiFailure {
    match failure {
        browser_web_runtime::BrowserWebFailure::Unauthenticated => ApiFailure::new(
            401,
            "unauthenticated",
            "Valid browser authentication is required.",
            false,
        ),
        browser_web_runtime::BrowserWebFailure::Forbidden => {
            ApiFailure::new(403, "forbidden", "The browser request was rejected.", false)
        }
        browser_web_runtime::BrowserWebFailure::Invalid => {
            ApiFailure::new(400, invalid_code, "The browser request is invalid.", false)
        }
        browser_web_runtime::BrowserWebFailure::Conflict => ApiFailure::new(
            409,
            "conflict",
            "The workspace changed before the request completed.",
            false,
        ),
        browser_web_runtime::BrowserWebFailure::RateLimited => {
            ApiFailure::new(429, "rate_limited", "The request rate is too high.", true)
                .with_retry_after_seconds(compatibility_rate_limit::RETRY_AFTER_SECONDS)
        }
        browser_web_runtime::BrowserWebFailure::NotFound => not_found_failure(),
        browser_web_runtime::BrowserWebFailure::Unavailable => ApiFailure::new(
            503,
            "service_unavailable",
            "The service is temporarily unavailable.",
            true,
        ),
    }
}

fn browser_auth_page_failure_response(
    action: worker_auth_runtime::BrowserAuthStart,
    failure: browser_web_runtime::BrowserWebFailure,
) -> Result<Response> {
    browser_auth_failure_redirect(browser_auth_page_failure_location(action, failure))
}

const fn browser_auth_page_failure_location(
    action: worker_auth_runtime::BrowserAuthStart,
    failure: browser_web_runtime::BrowserWebFailure,
) -> &'static str {
    match (action, failure) {
        (
            worker_auth_runtime::BrowserAuthStart::Login,
            browser_web_runtime::BrowserWebFailure::Invalid,
        ) => "/login?auth_error=invalid",
        (worker_auth_runtime::BrowserAuthStart::Login, _) => "/login?auth_error=failed",
        (
            worker_auth_runtime::BrowserAuthStart::Signup,
            browser_web_runtime::BrowserWebFailure::Invalid,
        ) => "/signup?auth_error=invalid",
        (worker_auth_runtime::BrowserAuthStart::Signup, _) => "/signup?auth_error=failed",
        (
            worker_auth_runtime::BrowserAuthStart::Recovery,
            browser_web_runtime::BrowserWebFailure::Invalid,
        ) => "/recovery?auth_error=invalid",
        (worker_auth_runtime::BrowserAuthStart::Recovery, _) => "/recovery?auth_error=failed",
    }
}

fn browser_auth_verify_failure_response(
    failure: browser_web_runtime::BrowserWebFailure,
) -> Result<Response> {
    browser_auth_failure_redirect(browser_auth_verify_failure_location(failure))
}

const fn browser_auth_verify_failure_location(
    failure: browser_web_runtime::BrowserWebFailure,
) -> &'static str {
    match failure {
        browser_web_runtime::BrowserWebFailure::Invalid => "/verify?auth_error=invalid",
        _ => "/verify?auth_error=failed",
    }
}

fn browser_auth_failure_redirect(location: &'static str) -> Result<Response> {
    let mut response = Response::empty()?.with_status(303);
    response.headers_mut().set("location", location)?;
    Ok(response)
}

const fn unauthenticated_failure() -> ApiFailure {
    ApiFailure::new(
        401,
        "unauthenticated",
        "Valid authentication is required.",
        false,
    )
    .with_authenticate()
}

fn failure_response(failure: ApiFailure, request_id: &str, production: bool) -> Result<Response> {
    let mut response = Response::from_json(&ApiError {
        code: failure.code.into(),
        message: failure.message.into(),
        request_id: Some(request_id.into()),
        retry: if failure.retryable {
            RetryAdvice::Later
        } else {
            RetryAdvice::Never
        },
    })?
    .with_status(failure.status);
    if let Some(allow) = failure.allow {
        response.headers_mut().set("allow", allow)?;
    }
    if failure.authenticate {
        response
            .headers_mut()
            .set("www-authenticate", "Bearer realm=\"frame\"")?;
    }
    if let Some(seconds) = failure.retry_after_seconds {
        response
            .headers_mut()
            .set("retry-after", &seconds.to_string())?;
    }
    secure_response(response, request_id, production)
}

fn secure_response(mut response: Response, request_id: &str, production: bool) -> Result<Response> {
    let headers = response.headers_mut();
    headers.set("cache-control", "no-store, max-age=0")?;
    headers.set("pragma", "no-cache")?;
    headers.set("expires", "0")?;
    headers.set("vary", "Origin")?;
    headers.set("x-request-id", request_id)?;
    headers.set("x-content-type-options", "nosniff")?;
    headers.set("x-frame-options", "DENY")?;
    headers.set("referrer-policy", "no-referrer")?;
    headers.set("cross-origin-resource-policy", "same-origin")?;
    headers.set("x-robots-tag", "noindex, nofollow, noarchive")?;
    headers.set(
        "permissions-policy",
        "camera=(), microphone=(), display-capture=(), geolocation=()",
    )?;
    headers.set(
        "content-security-policy",
        "default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
    )?;
    if production {
        headers.set(
            "strict-transport-security",
            "max-age=31536000; includeSubDomains",
        )?;
    }
    Ok(response)
}

fn request_id(request: &Request) -> String {
    let ray = request.headers().get("cf-ray").ok().flatten();
    normalize_cf_ray(
        ray.as_deref(),
        js_sys::Date::now().to_bits(),
        js_sys::Math::random().to_bits(),
    )
}

#[cfg(test)]
mod tests {
    use frame_client::FrameOrigin;

    use super::*;
    use crate::contracts::ValidationCode;

    #[test]
    fn protected_billing_auth_method_paths_resolve_exact_operation_ids() {
        let routes = [
            ("GET", "/api/auth/session", "cap-v1-46bda1c18ffba076"),
            (
                "POST",
                "/api/auth/callback/google",
                "cap-v1-82a39c991fae1050",
            ),
            ("POST", "/api/desktop/subscribe", "cap-v1-78537fb518df75ec"),
            (
                "OPTIONS",
                "/api/developer/credits/checkout",
                "cap-v1-572763e7b4977abd",
            ),
            (
                "POST",
                "/api/developer/credits/checkout",
                "cap-v1-60b06cc5ab45f187",
            ),
            (
                "POST",
                "/api/settings/billing/guest-checkout",
                "cap-v1-af61fa5c8fc453cf",
            ),
            (
                "POST",
                "/api/settings/billing/manage",
                "cap-v1-e596f65c43ee2a82",
            ),
            (
                "POST",
                "/api/settings/billing/subscribe",
                "cap-v1-96230bf1f2da3d00",
            ),
            (
                "GET",
                "/api/settings/billing/usage",
                "cap-v1-856dfea22b9d979c",
            ),
            ("POST", "/api/webhooks/stripe", "cap-v1-1e5f228815a2a8b7"),
            (
                "POST",
                "/api/commercial/checkout",
                "cap-v1-b2d19e91b05834cf",
            ),
        ];
        for (method, path, operation_id) in routes {
            assert_eq!(
                legacy_protected_billing_auth_route_operation(method, path),
                Some(operation_id),
                "{method} {path}"
            );
        }
        for (method, path) in [
            ("DELETE", "/api/auth/session"),
            ("GET", "/api/desktop/subscribe"),
            ("GET", "/api/developer/credits/checkout"),
            ("POST", "/api/settings/billing/usage"),
            ("GET", "/api/webhooks/stripe"),
        ] {
            assert_eq!(
                legacy_protected_billing_auth_route_operation(method, path),
                None,
                "{method} {path}"
            );
        }
        for (path, allow) in [
            ("/api/auth/session", "GET, POST"),
            ("/api/developer/credits/checkout", "POST, OPTIONS"),
            ("/api/settings/billing/usage", "GET"),
            ("/api/settings/billing/manage", "POST"),
            ("/api/commercial/checkout", "POST"),
        ] {
            assert_eq!(
                legacy_protected_billing_auth_allowed_methods(path).map(|(_, actual)| actual),
                Some(allow),
                "{path}"
            );
        }
        assert!(legacy_protected_billing_auth_allowed_methods("/api/auth").is_none());
        assert!(is_legacy_protected_billing_auth_workflow(
            "cap-v1-5a990f470c701cec"
        ));
        for operation_id in [
            "cap-v1-14ea978608dcf07e",
            "cap-v1-90a6eb69c3fd7b4b",
            "cap-v1-b9fcb0fbd25b2234",
        ] {
            assert!(
                !is_legacy_protected_billing_auth_workflow(operation_id),
                "{operation_id}"
            );
        }
    }

    #[test]
    fn compatibility_action_void_responses_are_no_store_and_cookie_exact() {
        let theme = legacy_web_compatibility_action_response_metadata(
            legacy_web_action_runtime::WebCompatibilityActionEffectV1::ThemeCookie {
                name: "theme",
                value: "dark",
                path: "/",
            },
        );
        assert_eq!(theme.status, 204);
        assert_eq!(theme.cache_control, "no-store, max-age=0");
        assert_eq!(theme.set_cookie, Some("theme=dark; Path=/".into()));

        let organization = legacy_web_compatibility_action_response_metadata(
            legacy_web_action_runtime::WebCompatibilityActionEffectV1::ActiveOrganizationChanged,
        );
        assert_eq!(organization.status, 204);
        assert_eq!(organization.cache_control, "no-store, max-age=0");
        assert_eq!(organization.set_cookie, None);
    }

    #[test]
    fn folder_assignment_response_objects_preserve_the_source_field_order() {
        assert_eq!(
            serde_json::to_string(&LegacyFolderAddedResponseV1 {
                success: true,
                message: "2 videos added to folder".into(),
                added_count: 2,
            })
            .expect("add response"),
            r#"{"success":true,"message":"2 videos added to folder","addedCount":2}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyFolderRemovedResponseV1 {
                success: true,
                message: "1 video removed from folder".into(),
                removed_count: 1,
            })
            .expect("remove response"),
            r#"{"success":true,"message":"1 video removed from folder","removedCount":1}"#
        );
    }

    #[test]
    fn membership_response_objects_preserve_exact_source_shapes_and_cap_ids() {
        assert_eq!(
            serde_json::to_string(&LegacyMembershipSuccessResponseV1 { success: true })
                .expect("single-remove response"),
            r#"{"success":true}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyMembershipAddedResponseV1 {
                success: true,
                added: vec!["3123456789abcde".into()],
                already_members: vec!["4123456789abcde".into()],
            })
            .expect("add-members response"),
            r#"{"success":true,"added":["3123456789abcde"],"alreadyMembers":["4123456789abcde"]}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyMembershipRemovedResponseV1 {
                success: true,
                removed: vec!["5123456789abcde".into(), "5123456789abcde".into()],
            })
            .expect("batch-remove response"),
            r#"{"success":true,"removed":["5123456789abcde","5123456789abcde"]}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyMembershipRemovedResponseV1 {
                success: true,
                removed: Vec::new(),
            })
            .expect("batch no-match response"),
            r#"{"success":true,"removed":[]}"#
        );
    }

    #[test]
    fn library_placement_response_objects_preserve_source_shapes_and_field_order() {
        assert_eq!(
            serde_json::to_string(&LegacyLibraryPlacementMessageResponseV1 {
                success: true,
                message: "2 videos are now in organization root".into(),
            })
            .expect("message response"),
            r#"{"success":true,"message":"2 videos are now in organization root"}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyLibraryPlacementRemovedResponseV1 {
                success: true,
                message: "Removed 2 video(s) from space and folders".into(),
                deleted_count: 2,
            })
            .expect("remove response"),
            r#"{"success":true,"message":"Removed 2 video(s) from space and folders","deletedCount":2}"#
        );
    }

    #[test]
    fn developer_response_objects_preserve_source_shapes_and_field_order() {
        assert_eq!(
            serde_json::to_string(&LegacyDeveloperAppCreatedResponseV1 {
                app_id: "0123456789abcde",
                public_key: "cpk_0123456789abcdefghjkmnpqrstvw",
                secret_key: "csk_0123456789abcdefghjkmnpqrstvw",
            })
            .expect("create response"),
            r#"{"appId":"0123456789abcde","publicKey":"cpk_0123456789abcdefghjkmnpqrstvw","secretKey":"csk_0123456789abcdefghjkmnpqrstvw"}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyDeveloperKeysResponseV1 {
                public_key: "cpk_0123456789abcdefghjkmnpqrstvw",
                secret_key: "csk_0123456789abcdefghjkmnpqrstvw",
            })
            .expect("key response"),
            r#"{"publicKey":"cpk_0123456789abcdefghjkmnpqrstvw","secretKey":"csk_0123456789abcdefghjkmnpqrstvw"}"#
        );
        assert_eq!(
            serde_json::to_string(&LegacyDeveloperSuccessResponseV1 { success: true })
                .expect("success response"),
            r#"{"success":true}"#
        );
    }

    #[test]
    fn scheduled_work_is_partitioned_into_distinct_invocations() {
        assert_eq!(
            scheduled_lane(AUTH_DELIVERY_CRON),
            Some(ScheduledLane::AuthDelivery)
        );
        assert_eq!(
            scheduled_lane(MULTIPART_MAINTENANCE_CRON),
            Some(ScheduledLane::MultipartMaintenance)
        );
        assert_eq!(
            scheduled_lane(INSTANT_FINALIZE_CRON),
            Some(ScheduledLane::InstantFinalize)
        );
        assert_eq!(
            scheduled_lane(MEDIA_RECOVERY_CRON),
            Some(ScheduledLane::MediaRecovery)
        );
        assert_eq!(
            scheduled_lane(RETENTION_MAINTENANCE_CRON),
            Some(ScheduledLane::RetentionMaintenance)
        );
        assert_eq!(scheduled_lane("0 0 * * *"), None);
    }

    #[test]
    fn failures_have_stable_status_and_do_not_carry_internal_details() {
        let failure = unauthenticated_failure();
        assert_eq!(failure.status, 401);
        assert_eq!(failure.code, "unauthenticated");
        assert_eq!(failure.message, "Valid authentication is required.");
        assert!(failure.authenticate);
        assert!(!failure.retryable);
    }

    #[test]
    fn validation_codes_map_to_stable_public_statuses() {
        for code in [
            ValidationCode::Identifier,
            ValidationCode::Size,
            ValidationCode::ContentType,
            ValidationCode::ObjectRole,
            ValidationCode::ObjectVersion,
            ValidationCode::Profile,
            ValidationCode::Title,
            ValidationCode::Privacy,
            ValidationCode::Revision,
            ValidationCode::LeaseToken,
            ValidationCode::Checksum,
            ValidationCode::Progress,
            ValidationCode::FailureClass,
        ] {
            let failure = invalid_body_failure(code.as_str());
            assert_eq!(failure.status, 400);
            assert_eq!(failure.message, "The request body is invalid.");
        }
        assert_eq!(
            invalid_body_failure(ValidationCode::SchemaVersion.as_str()).status,
            422
        );
    }

    #[test]
    fn worker_scope_is_explicit_and_not_implied_by_global_admin() {
        let worker = AuthenticatedActor {
            user_id: "018f47a6-7b1c-7f55-8f39-8f8a86900101".into(),
            scopes: vec!["frame:worker".into()],
        };
        assert!(worker.allows(RequiredAccess::Worker));
        assert!(!worker.allows(RequiredAccess::Read));

        let admin = AuthenticatedActor {
            user_id: "018f47a6-7b1c-7f55-8f39-8f8a86900101".into(),
            scopes: vec!["frame:admin".into()],
        };
        assert!(!admin.allows(RequiredAccess::Worker));
        assert!(admin.allows(RequiredAccess::Admin));
    }

    #[test]
    fn capability_discovery_describes_persisted_mutation_transports() {
        let capabilities = CapabilitiesResponse::default();
        assert_eq!(
            capabilities.upload_intents,
            "authenticated_d1_r2_single_put_and_multipart"
        );
        assert_eq!(
            capabilities.upload_transfer_modes,
            ["brokered", "direct", "multipart"]
        );
        assert_eq!(
            capabilities.direct_upload_finalize,
            "/api/v1/uploads/{upload_id}/finalize"
        );
        assert_eq!(
            capabilities.multipart_upload,
            "/api/v1/uploads/{upload_id}/multipart"
        );
        assert_eq!(
            capabilities.instant_finalize,
            "/api/v1/instant-recordings/{session_id}/finalize"
        );
        assert_eq!(
            capabilities.media_jobs,
            "fail_closed_pending_runtime_selection"
        );
        assert_eq!(capabilities.media_executor_selection, "server_controlled");
        assert!(!capabilities.managed_stream_library);
    }

    #[test]
    fn authority_pairs_fail_closed_without_a_legacy_dual_writer() {
        let row = |phase: &str, authority: &str| AuthorityRow {
            phase: phase.into(),
            authority: authority.into(),
            epoch: 4,
        };
        assert!(d1_mutation_pair(&row("d1_authoritative", "d1")));
        assert!(d1_mutation_pair(&row("finalized", "d1")));
        assert!(!d1_mutation_pair(&row("dual_write", "dual_write")));
        assert!(!d1_mutation_pair(&row("dual_write", "d1")));
        assert!(!d1_mutation_pair(&row("finalized", "dual_write")));
    }

    #[test]
    fn atomic_command_batches_accept_only_all_or_nothing_effects() {
        assert_eq!(classify_atomic_changes(&[1, 1, 1]), Ok(true));
        assert_eq!(classify_atomic_changes(&[0, 0, 0]), Ok(false));
        assert_eq!(classify_atomic_changes(&[1, 0, 1]), Err(()));
        assert_eq!(classify_atomic_changes(&[]), Err(()));
    }

    #[test]
    fn direct_finalize_uses_the_same_post_expiry_grace_as_staging_cleanup() {
        let expiry = 1_000_000_i64;
        assert!(!direct_upload_finalize_expired(expiry, expiry));
        assert!(!direct_upload_finalize_expired(
            expiry + DIRECT_STAGING_CLEANUP_GRACE_MS - 1,
            expiry
        ));
        assert!(direct_upload_finalize_expired(
            expiry + DIRECT_STAGING_CLEANUP_GRACE_MS,
            expiry
        ));
        assert!(direct_upload_finalize_expired(i64::MAX, i64::MAX));
    }

    #[test]
    fn native_dispatch_keeps_unimplemented_segment_mux_unclaimable() {
        assert!(native_claim_output("segment_mux_v1", "{}").is_none());
        assert!(requires_native_claim(MediaMode::Remote, "segment_mux_v1"));
        assert!(requires_native_claim(MediaMode::Remote, "transcription_v1"));
        assert!(!requires_native_claim(MediaMode::Remote, "thumbnail_v1"));
        assert!(requires_native_claim(MediaMode::Native, "thumbnail_v1"));
        assert!(!requires_native_claim(MediaMode::Fake, "segment_mux_v1"));
        assert_eq!(
            native_claim_output("probe_v1", "{}"),
            Some(("application/json".into(), 64 * 1_024))
        );
    }

    #[test]
    fn worker_source_sets_are_dense_scoped_and_profile_bounded() {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a86900001";
        let first_video = "018f47a6-7b1c-7f55-8f39-8f8a86900002";
        let second_video = "018f47a6-7b1c-7f55-8f39-8f8a86900003";
        let source = |ordinal, video: &str| WorkerSourceRow {
            ordinal,
            video_id: video.into(),
            source_version: 1,
            object_key: format!("tenants/{tenant}/videos/{video}/source/v1/payload"),
            bytes: 1_024,
            checksum_sha256: "a".repeat(64),
            content_type: "video/mp4".into(),
        };
        assert!(
            validated_worker_sources(
                tenant,
                "segment_mux_v1",
                vec![source(0, first_video), source(1, second_video)]
            )
            .is_ok()
        );
        assert!(
            validated_worker_sources(
                tenant,
                "segment_mux_v1",
                vec![source(0, first_video), source(2, second_video)]
            )
            .is_err()
        );
        assert!(
            validated_worker_sources(
                tenant,
                "segment_mux_v1",
                vec![source(0, first_video), source(1, first_video)]
            )
            .is_err()
        );
        assert!(
            validated_worker_sources(
                tenant,
                "composition_v1",
                vec![source(0, first_video), source(1, first_video)]
            )
            .is_ok()
        );
        assert!(
            validated_worker_sources(
                tenant,
                "probe_v1",
                vec![source(0, first_video), source(1, second_video)]
            )
            .is_err()
        );
        let mut cross_tenant = source(0, first_video);
        cross_tenant.object_key =
            format!("tenants/{tenant}/videos/{second_video}/source/v1/payload");
        assert!(validated_worker_sources(tenant, "probe_v1", vec![cross_tenant]).is_err());

        let mut oversized_secondary = source(1, second_video);
        oversized_secondary.bytes =
            i64::try_from(MAX_SINGLE_UPLOAD_BYTES.saturating_add(1)).unwrap_or(i64::MAX);
        assert!(
            validated_worker_sources(
                tenant,
                "composition_v1",
                vec![source(0, first_video), oversized_secondary]
            )
            .is_err()
        );

        let mut multipart_probe = source(0, first_video);
        multipart_probe.bytes =
            i64::try_from(MAX_SINGLE_UPLOAD_BYTES.saturating_add(1)).unwrap_or(i64::MAX);
        assert!(
            validated_worker_sources(tenant, "probe_v1", vec![multipart_probe.clone()]).is_ok()
        );
        multipart_probe.bytes =
            i64::try_from(MULTIPART_MAX_BYTES.saturating_add(1)).unwrap_or(i64::MAX);
        assert!(validated_worker_sources(tenant, "probe_v1", vec![multipart_probe]).is_err());
    }

    #[test]
    fn native_attempt_and_failure_policy_is_profile_specific_and_closed() {
        assert_eq!(native_profile_max_attempts("probe_v1"), 2);
        assert_eq!(native_profile_max_attempts("audio_presence_v1"), 2);
        assert_eq!(native_profile_max_attempts("thumbnail_v1"), 3);
        assert_eq!(
            native_execution_failure_class("pipeline_timeout"),
            Some("timeout")
        );
        assert_eq!(
            native_execution_failure_class("cancelled"),
            Some("cancelled")
        );
        assert_eq!(native_execution_failure_class("unknown"), None);
    }

    #[test]
    fn production_hides_reserved_repository_route_before_route_specific_processing() {
        assert!(local_repository_conformance_hidden(
            &Route::LocalRepositoryConformance,
            true
        ));
        assert!(local_repository_conformance_hidden(
            &Route::LocalOrganizationRepositoryConformance,
            true
        ));
        assert!(local_repository_conformance_hidden(
            &Route::LocalR2StorageConformance,
            true
        ));
        assert!(!local_repository_conformance_hidden(
            &Route::LocalRepositoryConformance,
            false
        ));
        assert!(!local_repository_conformance_hidden(
            &Route::ApiHealth,
            true
        ));
    }

    #[test]
    fn credential_scopes_and_media_types_are_explicit() {
        let read = AuthenticatedActor {
            user_id: "018f47a6-7b1c-7f55-8f39-8f8a86900101".into(),
            scopes: vec!["frame:read".into()],
        };
        assert!(read.allows(RequiredAccess::Read));
        assert!(!read.allows(RequiredAccess::Write));
        assert!(!read.allows(RequiredAccess::Admin));
        assert!(supported_source_content_type("video/webm"));
        assert!(supported_source_content_type("video/mp4"));
        assert!(!supported_source_content_type("text/html"));
        assert!(!supported_source_content_type("application/octet-stream"));
    }

    fn public_row() -> PublicShareRow {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a8690f123";
        let video = "018f47a6-7b1c-7f55-8f39-8f8a8690f124";
        PublicShareRow {
            id: video.into(),
            title: "Synthetic public recording".into(),
            state: "ready".into(),
            privacy: "public".into(),
            organization_id: Some(tenant.into()),
            playback_object_key: Some(format!(
                "tenants/{tenant}/videos/{video}/derivatives/playback/{}",
                "a".repeat(64)
            )),
            duration_ms: Some(42_000),
            content_type: Some("video/mp4".into()),
            bytes: Some(1_024),
            checksum_sha256: Some("b".repeat(64)),
            object_version: Some(1),
            governed_role: Some("preview".into()),
            governed_visibility: Some("public".into()),
            governed_state: Some("active".into()),
            malware_disposition: Some("clean".into()),
            cache_generation: Some(1),
            instant_finalize_state: None,
            instant_finalize_failure_class: None,
        }
    }

    #[test]
    fn worker_health_and_share_are_consumable_by_frame_client() {
        let health = health_contract(ServiceStatus::Ok).expect("health contract");
        let encoded = serde_json::to_vec(&health).expect("encode health");
        let decoded: Health = serde_json::from_slice(&encoded).expect("client health");
        decoded.validate().expect("valid client health");
        let fields = serde_json::to_value(&health)
            .expect("health value")
            .as_object()
            .expect("health object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            fields,
            [
                "api_version",
                "capabilities",
                "release",
                "service",
                "status",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );

        let summary = public_summary(&public_row(), "https://frame.engmanager.xyz");
        let encoded = serde_json::to_vec(&summary).expect("encode share");
        let decoded: PublicShareSummary = serde_json::from_slice(&encoded).expect("client share");
        decoded
            .validate(&FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("origin"))
            .expect("valid client share");
        assert_eq!(decoded.availability, ShareAvailability::Public);
        assert!(decoded.processing_status.is_none());
    }

    #[test]
    fn public_processing_status_uses_only_retained_indeterminate_finalize_truth() {
        let origin = FrameOrigin::parse_https("https://frame.engmanager.xyz").expect("origin");
        let mut processing = public_row();
        processing.state = "processing".into();
        processing.playback_object_key = None;
        processing.duration_ms = None;
        processing.content_type = None;
        processing.bytes = None;
        processing.checksum_sha256 = None;
        processing.object_version = None;
        processing.governed_role = None;
        processing.governed_visibility = None;
        processing.governed_state = None;
        processing.malware_disposition = None;
        processing.cache_generation = None;

        let without_finalize = public_summary(&processing, "https://frame.engmanager.xyz");
        without_finalize
            .validate(&origin)
            .expect("legacy processing summary");
        assert!(without_finalize.processing_status.is_none());

        processing.instant_finalize_state = Some("pending".into());
        let pending = public_summary(&processing, "https://frame.engmanager.xyz");
        pending.validate(&origin).expect("pending summary");
        assert_eq!(
            pending.processing_status,
            Some(InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: false,
                error: None,
            })
        );

        processing.instant_finalize_failure_class = Some("persistence".into());
        let delayed = public_summary(&processing, "https://frame.engmanager.xyz");
        delayed.validate(&origin).expect("delayed summary");
        assert_eq!(
            delayed.processing_status,
            Some(InstantUiProgressV1 {
                schema_version: INSTANT_UI_PROGRESS_SCHEMA_VERSION,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: true,
                error: Some(InstantUiErrorCodeV1::FinalizeDelayed),
            })
        );
        let encoded = serde_json::to_string(&delayed).expect("delayed JSON");
        assert!(!encoded.contains("progress_basis_points\":0"));
        for forbidden in [
            "object_key",
            "provider",
            "request_sha256",
            "tenant_id",
            "upload_id",
        ] {
            assert!(!encoded.contains(forbidden));
        }

        for (state, failure) in [
            ("dead_letter", Some("persistence")),
            ("published", None),
            ("pending", Some("conflict")),
        ] {
            processing.instant_finalize_state = Some(state.into());
            processing.instant_finalize_failure_class = failure.map(str::to_owned);
            assert_eq!(
                serde_json::to_vec(&public_summary(&processing, "https://frame.engmanager.xyz"))
                    .expect("terminal summary"),
                serde_json::to_vec(&unavailable_share()).expect("unavailable")
            );
        }
    }

    #[test]
    fn non_public_and_invalid_object_rows_are_indistinguishable() {
        let mut private = public_row();
        private.privacy = "private".into();
        let mut malformed = public_row();
        malformed.playback_object_key = Some("tenants/other/private.mp4".into());
        let unavailable = serde_json::to_vec(&unavailable_share()).expect("unavailable");
        assert_eq!(
            serde_json::to_vec(&public_summary(&private, "https://frame.engmanager.xyz"))
                .expect("private"),
            unavailable
        );
        assert_eq!(
            serde_json::to_vec(&public_summary(&malformed, "https://frame.engmanager.xyz"))
                .expect("malformed"),
            unavailable
        );
    }

    #[test]
    fn range_parser_accepts_one_bounded_range_and_rejects_ambiguity() {
        let prefix = parse_range_header(Some("bytes=0-9"), 100)
            .expect("range")
            .expect("present");
        assert_eq!((prefix.start, prefix.length), (0, 10));
        assert!(matches!(
            prefix.range,
            worker::Range::OffsetWithLength {
                offset: 0,
                length: 10
            }
        ));
        let tail = parse_range_header(Some("bytes=-12"), 100)
            .expect("suffix")
            .expect("present");
        assert_eq!((tail.start, tail.length), (88, 12));
        let open = parse_range_header(Some("bytes=90-"), 100)
            .expect("open")
            .expect("present");
        assert_eq!((open.start, open.length), (90, 10));
        for invalid in ["bytes=100-", "bytes=9-2", "bytes=0-1,4-5", "bytes=-0"] {
            assert!(parse_range_header(Some(invalid), 100).is_err(), "{invalid}");
        }
    }

    #[test]
    fn browser_auth_failures_redirect_to_closed_render_states_without_bearer_challenge() {
        assert_eq!(
            browser_auth_page_failure_location(
                worker_auth_runtime::BrowserAuthStart::Login,
                browser_web_runtime::BrowserWebFailure::Invalid,
            ),
            "/login?auth_error=invalid"
        );
        assert_eq!(
            browser_auth_page_failure_location(
                worker_auth_runtime::BrowserAuthStart::Recovery,
                browser_web_runtime::BrowserWebFailure::Unavailable,
            ),
            "/recovery?auth_error=failed"
        );
        assert_eq!(
            browser_auth_verify_failure_location(
                browser_web_runtime::BrowserWebFailure::Unauthenticated,
            ),
            "/verify?auth_error=failed"
        );

        let cookie_failure = browser_web_failure(
            browser_web_runtime::BrowserWebFailure::Unauthenticated,
            "invalid_browser_request",
        );
        assert_eq!(cookie_failure.status, 401);
        assert!(!cookie_failure.authenticate);
        let target_denial = browser_web_failure(
            browser_web_runtime::BrowserWebFailure::NotFound,
            "invalid_compatibility_action",
        );
        assert_eq!(target_denial.status, 404);
        assert_eq!(target_denial.code, "not_found");
    }
}
