//! Source-pinned contracts for Cap's core download, playback, and upload routes.
//!
//! The compatibility surface deliberately projects Cap's actor/video object
//! keys onto Frame's tenant-scoped R2 authority. Deprecated `fileKey` inputs
//! never select an actor: the authenticated actor is always substituted before
//! an object key is admitted. Multipart provider handles remain server-side,
//! while direct capabilities bind one normalized key, method, and content type.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LEGACY_CORE_STORAGE_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";

pub const LEGACY_DOWNLOAD_OPERATION_ID: &str = "cap-v1-d9d8d275d476c8be";
pub const LEGACY_PLAYLIST_GET_OPERATION_ID: &str = "cap-v1-ebb74af6ba0b5848";
pub const LEGACY_PLAYLIST_HEAD_OPERATION_ID: &str = "cap-v1-428610929ae5bb26";
pub const LEGACY_STORAGE_OBJECT_GET_OPERATION_ID: &str = "cap-v1-b5388e4ddf2d7f17";
pub const LEGACY_STORAGE_OBJECT_HEAD_OPERATION_ID: &str = "cap-v1-09e9e5a5c86b98c1";
pub const LEGACY_MULTIPART_ABORT_OPERATION_ID: &str = "cap-v1-f191ed86271608e3";
pub const LEGACY_MULTIPART_COMPLETE_OPERATION_ID: &str = "cap-v1-efc19423a62b7976";
pub const LEGACY_MULTIPART_INITIATE_OPERATION_ID: &str = "cap-v1-f47512c6177fa691";
pub const LEGACY_MULTIPART_PRESIGN_PART_OPERATION_ID: &str = "cap-v1-7b584d9338e8bf31";
pub const LEGACY_RECORDING_COMPLETE_OPERATION_ID: &str = "cap-v1-f9deb8104204a30d";
pub const LEGACY_SIGNED_UPLOAD_OPERATION_ID: &str = "cap-v1-7f87205cb7d39ee6";
pub const LEGACY_SIGNED_UPLOAD_BATCH_OPERATION_ID: &str = "cap-v1-c64cec46e4b828da";

pub const LEGACY_DOWNLOAD_PATH: &str = "/api/download";
pub const LEGACY_PLAYLIST_PATH: &str = "/api/playlist";
pub const LEGACY_STORAGE_OBJECT_PATH: &str = "/api/storage/object";
pub const LEGACY_MULTIPART_ABORT_PATH: &str = "/api/upload/multipart/abort";
pub const LEGACY_MULTIPART_COMPLETE_PATH: &str = "/api/upload/multipart/complete";
pub const LEGACY_MULTIPART_INITIATE_PATH: &str = "/api/upload/multipart/initiate";
pub const LEGACY_MULTIPART_PRESIGN_PART_PATH: &str = "/api/upload/multipart/presign-part";
pub const LEGACY_RECORDING_COMPLETE_PATH: &str = "/api/upload/recording-complete";
pub const LEGACY_SIGNED_UPLOAD_PATH: &str = "/api/upload/signed";
pub const LEGACY_SIGNED_UPLOAD_BATCH_PATH: &str = "/api/upload/signed/batch";

pub const LEGACY_CORE_STORAGE_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
pub const LEGACY_CORE_STORAGE_MAX_ID_BYTES: usize = 255 * 4;
pub const LEGACY_CORE_STORAGE_MAX_SUBPATH_BYTES: usize = 768;
pub const LEGACY_CORE_STORAGE_MAX_PARTS: usize = 10_000;
pub const LEGACY_CORE_STORAGE_MAX_BATCH: usize = 50;
pub const LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS: u32 = 3_600;
pub const LEGACY_CORE_STORAGE_MULTIPART_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
pub const LEGACY_CORE_STORAGE_FREE_PLAN_COMPLETION_SECONDS: f64 = 5.0 * 60.0 + 30.0;
pub const LEGACY_CORE_STORAGE_PROVIDER_GATES: &[&str] = &["provider_execution"];
pub const LEGACY_CORE_STORAGE_SOURCE_MANIFEST_SHA256: &str =
    "c673b6b50de8c2cb1ac7d94bb5dda52f13fcafb1ba7e0ab3a683e33f6d0e49f3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCoreStorageSourceRoleV1 {
    Handler,
    Contract,
    Authentication,
    Authorization,
    Persistence,
    Storage,
    Playlist,
    ProviderEffect,
    Client,
    Environment,
    DependencyLock,
}

impl LegacyCoreStorageSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Contract => "contract",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Persistence => "persistence",
            Self::Storage => "storage",
            Self::Playlist => "playlist",
            Self::ProviderEffect => "provider_effect",
            Self::Client => "client",
            Self::Environment => "environment",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCoreStorageSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyCoreStorageSourceRoleV1,
}

const fn source(
    path: &'static str,
    symbol: &'static str,
    sha256: &'static str,
    role: LegacyCoreStorageSourceRoleV1,
) -> LegacyCoreStorageSourcePinV1 {
    LegacyCoreStorageSourcePinV1 {
        path,
        symbol,
        sha256,
        role,
    }
}

pub const LEGACY_CORE_STORAGE_SOURCES: &[LegacyCoreStorageSourcePinV1] = &[
    source(
        "apps/web/app/api/download/route.ts",
        "GET platform redirect",
        "56b53c55eead824ad59651ac1ec27cc893c60b6440ff72af288033fcdcab380b",
        LegacyCoreStorageSourceRoleV1::Handler,
    ),
    source(
        "apps/web/app/api/playlist/route.ts",
        "GET+HEAD getVideoSrc and playlist generation",
        "f9e19e262054235aef305ed4e1052729035ee212fcdad5126b517a84e30c9d29",
        LegacyCoreStorageSourceRoleV1::Handler,
    ),
    source(
        "apps/web/app/api/storage/object/route.ts",
        "GET+HEAD optional-auth range proxy",
        "85b77b12ca6553b592428bec9e3c9bba1a966d7c5b5f2f440edc861e7d45d775",
        LegacyCoreStorageSourceRoleV1::Handler,
    ),
    source(
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "initiate+presign-part+complete+abort",
        "97644564903178d153d1232d9767086e5c77c8cb4cb96fedc483755260e29934",
        LegacyCoreStorageSourceRoleV1::Handler,
    ),
    source(
        "apps/web/app/api/upload/[...route]/multipart-utils.ts",
        "getMultipartFileKey+getSubpath+isRawRecorderUpload",
        "3b5811250f1bfe55a3142af27aa8b3772244248993f51a9daa04b62508251993",
        LegacyCoreStorageSourceRoleV1::Contract,
    ),
    source(
        "apps/web/app/api/upload/[...route]/signed.ts",
        "single+batch upload target handlers",
        "c1b02757cc95ae84e8beb2249e00e3495061ddf8b6d9db1e7e10712e7fbb11c3",
        LegacyCoreStorageSourceRoleV1::Handler,
    ),
    source(
        "apps/web/app/api/upload/[...route]/recording-complete.ts",
        "recording completion queue handler",
        "cf62e89c55960c9ff016233c4c3e463ce46111680c2a18f317743f393bd2a46a",
        LegacyCoreStorageSourceRoleV1::Handler,
    ),
    source(
        "apps/web/app/api/upload/utils.ts",
        "parseVideoIdOrFileKey",
        "4914978b1fd72ccfa5d39f09f718d9521ebc8e602527cc39fc240f65fff49fc9",
        LegacyCoreStorageSourceRoleV1::Contract,
    ),
    source(
        "apps/web/app/api/utils.ts",
        "withAuth+API key/session precedence",
        "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
        LegacyCoreStorageSourceRoleV1::Authentication,
    ),
    source(
        "packages/web-backend/src/Auth.ts",
        "provideOptionalAuth+HttpAuthMiddlewareLive",
        "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
        LegacyCoreStorageSourceRoleV1::Authentication,
    ),
    source(
        "packages/web-backend/src/Storage/index.ts",
        "getAccessForVideo+upload targets+signed object proxy",
        "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
        LegacyCoreStorageSourceRoleV1::Storage,
    ),
    source(
        "packages/web-backend/src/Storage/SignedObject.ts",
        "createStorageObjectToken+verifyStorageObjectToken",
        "0313426fe91c1a9822c7fdd00bfd5a21fecc86ab77880d24bf8d461a2daee34a",
        LegacyCoreStorageSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        "range+signing+multipart provider behavior",
        "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
        LegacyCoreStorageSourceRoleV1::Storage,
    ),
    source(
        "packages/web-backend/src/Videos/index.ts",
        "getByIdForViewing+storage selection",
        "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
        LegacyCoreStorageSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-backend/src/Videos/VideosPolicy.ts",
        "canView+isOwner",
        "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
        LegacyCoreStorageSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-domain/src/Video.ts",
        "Video sources+SegmentManifest+key derivation",
        "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
        LegacyCoreStorageSourceRoleV1::Contract,
    ),
    source(
        "packages/database/schema.ts",
        "videos+videoUploads+storageIntegrations+organizations",
        "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        LegacyCoreStorageSourceRoleV1::Persistence,
    ),
    source(
        "packages/env/server.ts",
        "storage+media-server+NEXTAUTH_SECRET environment",
        "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
        LegacyCoreStorageSourceRoleV1::Environment,
    ),
    source(
        "apps/web/utils/helpers.ts",
        "CACHE_CONTROL_HEADERS",
        "29981cf466c2a5a8e9ab947c6174a945010ba2ee353e2b4fcc4fcbc4509dfe6a",
        LegacyCoreStorageSourceRoleV1::Contract,
    ),
    source(
        "apps/web/utils/video/ffmpeg/helpers.ts",
        "generateM3U8Playlist+generateMasterPlaylist",
        "5d415f303e8bb65c710569493ed4006cb496a886452884f5cde55bc8f98fb87a",
        LegacyCoreStorageSourceRoleV1::Playlist,
    ),
    source(
        "apps/web/lib/desktop-segments-finalization.ts",
        "queueDesktopSegmentsFinalization",
        "298c57c6c73cd368abc725acff12c472e953e6856075258552cb066174dd60dd",
        LegacyCoreStorageSourceRoleV1::ProviderEffect,
    ),
    source(
        "apps/web/workflows/finalize-desktop-recording.ts",
        "finalizeDesktopRecordingWorkflow media provider",
        "8363e6455411dc9660d2bc72964d37fb2bf9f4616c418570b0322b6a45573cb6",
        LegacyCoreStorageSourceRoleV1::ProviderEffect,
    ),
    source(
        "apps/web/lib/video-processing.ts",
        "startVideoProcessingWorkflow",
        "56d755ad564725c2912a48bce70e2410b991e2bb94889aba021ad4f1ecad32a0",
        LegacyCoreStorageSourceRoleV1::ProviderEffect,
    ),
    source(
        "packages/recorder-core/src/instant-mp4-uploader.ts",
        "released multipart lifecycle client",
        "5be774de9fe4f715129f8b0c9fe4d1d8797dedf0f042f38806c3c0e582dc3978",
        LegacyCoreStorageSourceRoleV1::Client,
    ),
    source(
        "apps/desktop/src-tauri/src/api.rs",
        "released signed+multipart+recording-complete client",
        "d029c4cc7eba0be97f03bba8da2f3ab02277ce65161de69aca4ad77b3474a48e",
        LegacyCoreStorageSourceRoleV1::Client,
    ),
    source(
        "apps/cli/src/upload.rs",
        "released signed upload client",
        "2d05f536b8eca123ae26330884b720518dd1d92b10d872a3f24196932e3ab7a2",
        LegacyCoreStorageSourceRoleV1::Client,
    ),
    source(
        "pnpm-lock.yaml",
        "Effect+Drizzle+AWS SDK dependency resolutions",
        "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        LegacyCoreStorageSourceRoleV1::DependencyLock,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCoreStorageProfileV1 {
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub auth: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency_retry: &'static str,
    pub failure: &'static str,
    pub protected_gates: &'static [&'static str],
}

const NO_GATES: &[&str] = &[];

macro_rules! profile {
    ($operation_id:expr, $method:expr, $path:expr, $auth:expr, $success:expr,
     $validation:expr, $authorization:expr, $idempotency_retry:expr, $failure:expr,
     $protected_gates:expr $(,)?) => {
        LegacyCoreStorageProfileV1 {
            operation_id: $operation_id,
            method: $method,
            path: $path,
            auth: $auth,
            success: $success,
            validation: $validation,
            authorization: $authorization,
            idempotency_retry: $idempotency_retry,
            failure: $failure,
            protected_gates: $protected_gates,
        }
    };
}

pub const LEGACY_CORE_STORAGE_PROFILES: &[LegacyCoreStorageProfileV1] = &[
    profile!(
        LEGACY_DOWNLOAD_OPERATION_ID,
        "GET",
        LEGACY_DOWNLOAD_PATH,
        "optional_session_or_share_capability",
        "temporary_redirect_to_platform_download",
        "bounded_user_agent_and_client_platform_headers",
        "public_route_without_tenant_selection",
        "safe_get_retries_are_equivalent",
        "307_only_after_valid_origin_projection",
        NO_GATES,
    ),
    profile!(
        LEGACY_PLAYLIST_GET_OPERATION_ID,
        "GET",
        LEGACY_PLAYLIST_PATH,
        "optional_session_or_share_capability",
        "playlist_text_or_exact_r2_object_redirect",
        "exact_video_type_and_bounded_query",
        "public_video_or_session_authorized_tenant_video",
        "safe_get_retries_refresh_only_bounded_capabilities",
        "400_schema_401_policy_403_password_404_video_or_object_500_storage",
        NO_GATES,
    ),
    profile!(
        LEGACY_PLAYLIST_HEAD_OPERATION_ID,
        "HEAD",
        LEGACY_PLAYLIST_PATH,
        "optional_session_or_share_capability",
        "get_equivalent_status_and_headers_without_body",
        "same_query_contract_as_get",
        "same_authority_as_get",
        "safe_head_retries_are_equivalent",
        "get_head_status_header_parity",
        NO_GATES,
    ),
    profile!(
        LEGACY_STORAGE_OBJECT_GET_OPERATION_ID,
        "GET",
        LEGACY_STORAGE_OBJECT_PATH,
        "session",
        "r2_body_or_single_range_response",
        "video_id_key_token_and_single_range",
        "exact_token_binding_or_session_video_policy_and_key_prefix",
        "safe_get_retries_are_equivalent",
        "400_missing_404_non_disclosure_416_range_502_storage",
        NO_GATES,
    ),
    profile!(
        LEGACY_STORAGE_OBJECT_HEAD_OPERATION_ID,
        "HEAD",
        LEGACY_STORAGE_OBJECT_PATH,
        "session",
        "get_equivalent_status_and_headers_without_body",
        "same_query_and_range_contract_as_get",
        "same_authority_as_get",
        "safe_head_retries_are_equivalent",
        "get_head_status_header_parity",
        NO_GATES,
    ),
    profile!(
        LEGACY_MULTIPART_ABORT_OPERATION_ID,
        "POST",
        LEGACY_MULTIPART_ABORT_PATH,
        "session",
        "success_file_key_and_upload_id_after_r2_abort",
        "bounded_json_exact_target_and_upload_id",
        "session_actor_owned_video_active_r2_integration_and_bound_session",
        "required_key_replays_converge_to_terminal_abort",
        "400_schema_401_session_404_non_disclosure_409_conflict_500_storage",
        NO_GATES,
    ),
    profile!(
        LEGACY_MULTIPART_COMPLETE_OPERATION_ID,
        "POST",
        LEGACY_MULTIPART_COMPLETE_PATH,
        "session",
        "durable_completion_claim_then_provider_receipt",
        "ordered_unique_parts_and_finite_optional_media_metadata",
        "session_actor_owned_video_and_bound_r2_session",
        "required_key_replays_same_claim_or_conflict",
        "provider_execution_gate_fails_closed_after_durable_orchestration",
        LEGACY_CORE_STORAGE_PROVIDER_GATES,
    ),
    profile!(
        LEGACY_MULTIPART_INITIATE_OPERATION_ID,
        "POST",
        LEGACY_MULTIPART_INITIATE_PATH,
        "session",
        "opaque_legacy_upload_id_and_r2_provider",
        "bounded_content_type_and_normalized_target",
        "session_actor_owned_video_active_r2_integration",
        "required_key_replays_same_upload_or_conflict",
        "400_schema_401_session_404_non_disclosure_409_conflict_500_storage",
        NO_GATES,
    ),
    profile!(
        LEGACY_MULTIPART_PRESIGN_PART_OPERATION_ID,
        "POST",
        LEGACY_MULTIPART_PRESIGN_PART_PATH,
        "session",
        "exact_r2_upload_part_capability",
        "bounded_upload_id_part_number_optional_md5_and_target",
        "session_actor_owned_video_and_bound_open_session",
        "required_key_replays_same_capability_binding_or_conflict",
        "400_schema_401_session_404_non_disclosure_409_conflict_500_signing",
        NO_GATES,
    ),
    profile!(
        LEGACY_RECORDING_COMPLETE_OPERATION_ID,
        "POST",
        LEGACY_RECORDING_COMPLETE_PATH,
        "session",
        "durable_finalize_workflow_intent",
        "bounded_exact_video_id_json",
        "session_actor_owned_desktop_segments_video",
        "required_key_replays_same_workflow_intent_or_conflict",
        "provider_execution_gate_fails_closed_after_durable_orchestration",
        LEGACY_CORE_STORAGE_PROVIDER_GATES,
    ),
    profile!(
        LEGACY_SIGNED_UPLOAD_OPERATION_ID,
        "POST",
        LEGACY_SIGNED_UPLOAD_PATH,
        "session",
        "presigned_post_or_put_shape_with_exact_metadata",
        "bounded_method_target_and_finite_optional_media_metadata",
        "session_actor_owned_video_active_r2_integration",
        "required_key_replays_same_object_capability_binding_or_conflict",
        "400_schema_401_session_404_non_disclosure_409_conflict_500_signing",
        NO_GATES,
    ),
    profile!(
        LEGACY_SIGNED_UPLOAD_BATCH_OPERATION_ID,
        "POST",
        LEGACY_SIGNED_UPLOAD_BATCH_PATH,
        "session",
        "one_to_fifty_subpath_upload_targets_and_url_aliases",
        "bounded_unique_normalized_subpaths",
        "session_actor_owned_video_active_r2_integration",
        "required_key_replays_same_batch_binding_or_conflict",
        "400_schema_401_session_404_non_disclosure_409_conflict_500_signing",
        NO_GATES,
    ),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LegacyNumberInputV1 {
    Number(f64),
    String(String),
}

impl LegacyNumberInputV1 {
    #[must_use]
    pub fn finite(&self) -> Option<f64> {
        match self {
            Self::Number(value) => value.is_finite().then_some(*value),
            Self::String(value) if value.len() <= 128 && !value.chars().any(char::is_control) => {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|parsed| parsed.is_finite())
            }
            Self::String(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyStorageTargetV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
}

impl LegacyStorageTargetV1 {
    #[must_use]
    pub fn normalized(
        &self,
        actor_id: &str,
        default_subpath: &str,
    ) -> Option<LegacyObjectTargetV1> {
        if !valid_id(actor_id) {
            return None;
        }
        let (video_id, subpath) = match (&self.video_id, &self.subpath, &self.file_key) {
            (Some(video_id), subpath, None) => (
                video_id.as_str(),
                subpath.as_deref().unwrap_or(default_subpath),
            ),
            (None, None, Some(file_key)) => {
                let mut parts = file_key.split('/');
                let _untrusted_actor = parts.next()?;
                let video_id = parts.next()?;
                let subpath = parts.collect::<Vec<_>>().join("/");
                if subpath.is_empty() {
                    return None;
                }
                return LegacyObjectTargetV1::new(actor_id, video_id, &subpath);
            }
            _ => return None,
        };
        LegacyObjectTargetV1::new(actor_id, video_id, subpath)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyObjectTargetV1 {
    pub video_id: String,
    pub subpath: String,
    pub object_key: String,
}

impl LegacyObjectTargetV1 {
    fn new(actor_id: &str, video_id: &str, subpath: &str) -> Option<Self> {
        if !valid_id(video_id) || !valid_subpath(subpath) {
            return None;
        }
        Some(Self {
            video_id: video_id.into(),
            subpath: subpath.into(),
            object_key: format!("{actor_id}/{video_id}/{subpath}"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMultipartInitiateInputV1 {
    pub content_type: String,
    #[serde(flatten)]
    pub target: LegacyStorageTargetV1,
}

impl LegacyMultipartInitiateInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_content_type_or_empty(&self.content_type)
    }

    #[must_use]
    pub fn content_type(&self) -> &str {
        if self.content_type.is_empty() {
            "video/mp4"
        } else {
            &self.content_type
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMultipartPresignPartInputV1 {
    pub upload_id: String,
    pub part_number: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5_sum: Option<String>,
    #[serde(flatten)]
    pub target: LegacyStorageTargetV1,
}

impl LegacyMultipartPresignPartInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_upload_id(&self.upload_id)
            && self.part_number > 0
            && usize::from(self.part_number) <= LEGACY_CORE_STORAGE_MAX_PARTS
            && self.md5_sum.as_deref().is_none_or(valid_md5)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMultipartAbortInputV1 {
    pub upload_id: String,
    #[serde(flatten)]
    pub target: LegacyStorageTargetV1,
}

impl LegacyMultipartAbortInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_upload_id(&self.upload_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMultipartPartInputV1 {
    pub part_number: u16,
    pub etag: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMultipartCompleteInputV1 {
    pub upload_id: String,
    pub parts: Vec<LegacyMultipartPartInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_in_secs: Option<LegacyNumberInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<LegacyNumberInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<LegacyNumberInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<LegacyNumberInputV1>,
    #[serde(flatten)]
    pub target: LegacyStorageTargetV1,
}

impl LegacyMultipartCompleteInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_upload_id(&self.upload_id)
            && !self.parts.is_empty()
            && self.parts.len() <= LEGACY_CORE_STORAGE_MAX_PARTS
            && self.parts.iter().enumerate().all(|(index, part)| {
                usize::from(part.part_number) == index + 1
                    && part.size > 0
                    && part.size <= 9_007_199_254_740_991
                    && valid_etag(&part.etag)
            })
            && [
                self.duration_in_secs.as_ref(),
                self.width.as_ref(),
                self.height.as_ref(),
                self.fps.as_ref(),
            ]
            .into_iter()
            .all(|value| value.is_none_or(|value| value.finite().is_some()))
    }

    #[must_use]
    pub fn total_size(&self) -> Option<u64> {
        self.parts
            .iter()
            .try_fold(0_u64, |sum, part| sum.checked_add(part.size))
            .filter(|sum| *sum <= 9_007_199_254_740_991)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LegacySignedMethodV1 {
    #[default]
    Post,
    Put,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacySignedUploadInputV1 {
    #[serde(default)]
    pub method: LegacySignedMethodV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_in_secs: Option<LegacyNumberInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<LegacyNumberInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<LegacyNumberInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<LegacyNumberInputV1>,
    #[serde(flatten)]
    pub target: LegacyStorageTargetV1,
}

impl LegacySignedUploadInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        [
            self.duration_in_secs.as_ref(),
            self.width.as_ref(),
            self.height.as_ref(),
            self.fps.as_ref(),
        ]
        .into_iter()
        .all(|value| value.is_none_or(|value| value.finite().is_some()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacySignedUploadBatchInputV1 {
    pub video_id: String,
    pub subpaths: Vec<String>,
}

impl LegacySignedUploadBatchInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.video_id)
            && !self.subpaths.is_empty()
            && self.subpaths.len() <= LEGACY_CORE_STORAGE_MAX_BATCH
            && self.subpaths.iter().all(|subpath| valid_subpath(subpath))
            && {
                let mut sorted = self.subpaths.iter().collect::<Vec<_>>();
                sorted.sort_unstable();
                sorted.dedup();
                sorted.len() == self.subpaths.len()
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyRecordingCompleteInputV1 {
    pub video_id: String,
}

impl LegacyRecordingCompleteInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.video_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyPlaylistVideoTypeV1 {
    Video,
    Audio,
    Master,
    Mp4,
    RawPreview,
    SegmentsMaster,
    SegmentsVideo,
    SegmentsAudio,
}

impl LegacyPlaylistVideoTypeV1 {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "master" => Some(Self::Master),
            "mp4" => Some(Self::Mp4),
            "raw-preview" => Some(Self::RawPreview),
            "segments-master" => Some(Self::SegmentsMaster),
            "segments-video" => Some(Self::SegmentsVideo),
            "segments-audio" => Some(Self::SegmentsAudio),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPlaylistQueryV1 {
    pub video_id: String,
    pub video_type: LegacyPlaylistVideoTypeV1,
    pub require_complete: bool,
    pub thumbnail: bool,
    pub file_type: Option<String>,
}

impl LegacyPlaylistQueryV1 {
    #[must_use]
    pub fn parse(
        video_id: &str,
        video_type: &str,
        require_complete: Option<&str>,
        thumbnail: Option<&str>,
        file_type: Option<&str>,
    ) -> Option<Self> {
        if !valid_id(video_id)
            || require_complete.is_some_and(|value| !valid_query_value(value))
            || thumbnail.is_some_and(|value| !valid_query_value(value))
            || file_type.is_some_and(|value| !valid_query_value(value))
        {
            return None;
        }
        Some(Self {
            video_id: video_id.into(),
            video_type: LegacyPlaylistVideoTypeV1::parse(video_type)?,
            require_complete: matches!(require_complete, Some("1" | "true")),
            thumbnail: thumbnail.is_some(),
            file_type: file_type.map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyStorageObjectQueryV1 {
    pub video_id: String,
    pub key: String,
    pub token: Option<String>,
}

impl LegacyStorageObjectQueryV1 {
    #[must_use]
    pub fn parse(video_id: &str, key: &str, token: Option<&str>) -> Option<Self> {
        if !valid_id(video_id)
            || !valid_object_key(key)
            || token.is_some_and(|value| {
                value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
            })
        {
            return None;
        }
        Some(Self {
            video_id: video_id.into(),
            key: key.into(),
            token: token.map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDownloadPlatformV1 {
    AppleSilicon,
    AppleIntel,
    Windows,
    LinuxDeb,
}

impl LegacyDownloadPlatformV1 {
    #[must_use]
    pub const fn path_segment(self) -> &'static str {
        match self {
            Self::AppleSilicon => "apple-silicon",
            Self::AppleIntel => "apple-intel",
            Self::Windows => "windows",
            Self::LinuxDeb => "linux-deb",
        }
    }
}

#[must_use]
pub fn legacy_download_platform(
    user_agent: &str,
    client_platform: &str,
) -> LegacyDownloadPlatformV1 {
    let user_agent = user_agent.to_ascii_lowercase();
    let client_platform = client_platform.replace('"', "").to_ascii_lowercase();
    if client_platform.contains("windows") || user_agent.contains("windows") {
        LegacyDownloadPlatformV1::Windows
    } else if client_platform.contains("macos") || user_agent.contains("mac") {
        if ["intel", "x86_64", "amd64"]
            .iter()
            .any(|needle| user_agent.contains(needle))
        {
            LegacyDownloadPlatformV1::AppleIntel
        } else {
            LegacyDownloadPlatformV1::AppleSilicon
        }
    } else if client_platform.contains("linux")
        || (user_agent.contains("linux") && !user_agent.contains("android"))
    {
        LegacyDownloadPlatformV1::LinuxDeb
    } else {
        LegacyDownloadPlatformV1::AppleSilicon
    }
}

#[must_use]
pub fn legacy_content_type_for_subpath(subpath: &str) -> &'static str {
    if subpath.ends_with(".json") {
        "application/json"
    } else if subpath.ends_with(".mp4") || subpath.ends_with(".m4s") {
        "video/mp4"
    } else if subpath.ends_with(".jpg") || subpath.ends_with(".jpeg") {
        "image/jpeg"
    } else if subpath.ends_with(".png") {
        "image/png"
    } else if subpath.ends_with(".aac") {
        "audio/aac"
    } else if subpath.ends_with(".webm") {
        "audio/webm"
    } else if subpath.ends_with(".mp3") {
        "audio/mpeg"
    } else if subpath.ends_with(".m3u8") {
        "application/x-mpegURL"
    } else {
        "application/octet-stream"
    }
}

#[must_use]
pub fn legacy_core_storage_source_manifest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-core-storage-source-manifest-v1\0");
    for source in LEGACY_CORE_STORAGE_SOURCES {
        digest.update(source.path.as_bytes());
        digest.update([0]);
        digest.update(source.sha256.as_bytes());
        digest.update([0]);
        digest.update(source.role.stable_code().as_bytes());
        digest.update(b"\n");
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LEGACY_CORE_STORAGE_MAX_ID_BYTES
        && !value.chars().any(char::is_control)
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
        && !value.contains(['?', '#', '%'])
}

fn valid_subpath(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LEGACY_CORE_STORAGE_MAX_SUBPATH_BYTES
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && !value.ends_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.contains(['?', '#', '%'])
        && !value.chars().any(char::is_control)
}

fn valid_object_key(value: &str) -> bool {
    value.len() <= LEGACY_CORE_STORAGE_MAX_ID_BYTES + LEGACY_CORE_STORAGE_MAX_SUBPATH_BYTES + 2
        && value.split('/').count() >= 3
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.contains(['?', '#', '%'])
        && !value.chars().any(char::is_control)
}

fn valid_content_type_or_empty(value: &str) -> bool {
    value.is_empty()
        || (value.len() <= 127
            && value.contains('/')
            && !value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace()))
}

fn valid_upload_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.chars().any(char::is_control)
        && !value.contains(['?', '#'])
}

fn valid_md5(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
        })
}

fn valid_etag(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn valid_query_value(value: &str) -> bool {
    value.len() <= 1024 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_five_axis_closures_are_complete() {
        assert_eq!(LEGACY_CORE_STORAGE_SOURCES.len(), 27);
        assert_eq!(LEGACY_CORE_STORAGE_PROFILES.len(), 12);
        assert_eq!(
            legacy_core_storage_source_manifest(),
            LEGACY_CORE_STORAGE_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            LEGACY_CORE_STORAGE_PROFILES
                .iter()
                .filter(|profile| !profile.protected_gates.is_empty())
                .map(|profile| profile.operation_id)
                .collect::<Vec<_>>(),
            [
                LEGACY_MULTIPART_COMPLETE_OPERATION_ID,
                LEGACY_RECORDING_COMPLETE_OPERATION_ID
            ]
        );
    }

    #[test]
    fn deprecated_file_keys_never_select_an_actor_or_escape_the_video_prefix() {
        let target = LegacyStorageTargetV1 {
            video_id: None,
            subpath: None,
            file_key: Some("attacker/video/segments/video/segment_001.m4s".into()),
        }
        .normalized("trusted-actor", "result.mp4")
        .expect("target");
        assert_eq!(target.video_id, "video");
        assert_eq!(
            target.object_key,
            "trusted-actor/video/segments/video/segment_001.m4s"
        );
        for escaped in ["../secret", "/secret", "segment%2fsecret", "a\\b"] {
            assert!(
                LegacyStorageTargetV1 {
                    video_id: Some("video".into()),
                    subpath: Some(escaped.into()),
                    file_key: None,
                }
                .normalized("trusted-actor", "result.mp4")
                .is_none(),
                "{escaped}"
            );
        }
    }

    #[test]
    fn multipart_completion_requires_ordered_unique_bounded_parts() {
        let mut input = LegacyMultipartCompleteInputV1 {
            upload_id: "opaque-upload".into(),
            parts: vec![
                LegacyMultipartPartInputV1 {
                    part_number: 1,
                    etag: "etag-1".into(),
                    size: 5,
                },
                LegacyMultipartPartInputV1 {
                    part_number: 2,
                    etag: "etag-2".into(),
                    size: 7,
                },
            ],
            duration_in_secs: Some(LegacyNumberInputV1::String("12.5".into())),
            width: None,
            height: None,
            fps: None,
            target: LegacyStorageTargetV1 {
                video_id: Some("video".into()),
                subpath: None,
                file_key: None,
            },
        };
        assert!(input.valid());
        assert_eq!(input.total_size(), Some(12));
        input.parts[1].part_number = 1;
        assert!(!input.valid());
    }

    #[test]
    fn platform_and_content_type_projection_match_released_clients() {
        assert_eq!(
            legacy_download_platform("Mozilla Windows", ""),
            LegacyDownloadPlatformV1::Windows
        );
        assert_eq!(
            legacy_download_platform("Macintosh Intel", "macOS"),
            LegacyDownloadPlatformV1::AppleIntel
        );
        assert_eq!(
            legacy_download_platform("Linux x86_64", ""),
            LegacyDownloadPlatformV1::LinuxDeb
        );
        assert_eq!(legacy_content_type_for_subpath("a.m4s"), "video/mp4");
        assert_eq!(
            legacy_content_type_for_subpath("manifest.json"),
            "application/json"
        );
    }
}
