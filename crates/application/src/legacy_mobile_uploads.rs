//! Source-pinned contract for Cap's released mobile upload lifecycle.
//!
//! The released client creates a fresh upload without an idempotency header,
//! streams bytes to the returned PUT target, periodically reports byte counts,
//! and finally asks the server to start video processing. Frame retains that
//! wire contract while tightening object keys to the exact minted R2 key. The
//! processing start remains a protected provider gate: durable intent is not
//! represented as successful workflow submission.

use std::{collections::BTreeMap, fmt::Write as _};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{LegacyMobileCapSummaryV1, legacy_mobile_trim};

pub const LEGACY_MOBILE_UPLOADS_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_MOBILE_UPLOAD_CREATE_OPERATION_ID: &str = "cap-v1-b0116dd82b010477";
pub const LEGACY_MOBILE_UPLOAD_COMPLETE_OPERATION_ID: &str = "cap-v1-b43b6ede64a73798";
pub const LEGACY_MOBILE_UPLOAD_PROGRESS_OPERATION_ID: &str = "cap-v1-62469fe03e030052";

pub const LEGACY_MOBILE_UPLOAD_CREATE_PATH: &str = "/api/mobile/uploads";
pub const LEGACY_MOBILE_UPLOAD_COMPLETE_PATH: &str = "/api/mobile/uploads/:id/complete";
pub const LEGACY_MOBILE_UPLOAD_PROGRESS_PATH: &str = "/api/mobile/uploads/:id/progress";
pub const LEGACY_MOBILE_UPLOADS_AUTH: &str = "session_or_api_key";
pub const LEGACY_MOBILE_UPLOADS_POLICY: &str = "upload_storage.v1";
pub const LEGACY_MOBILE_UPLOADS_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
pub const LEGACY_MOBILE_UPLOADS_UPLOAD_TTL_SECONDS: u32 = 1_800;
pub const LEGACY_MOBILE_UPLOADS_PROVIDER_GATES: &[&str] = &["provider_execution"];
pub const LEGACY_MOBILE_UPLOADS_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const LEGACY_MOBILE_UPLOAD_CREATE_SOURCE_MANIFEST_SHA256: &str =
    "de3bf22950e46dfdbe9bc54bc020888fa5f11398353b2c0890883ebfd7ee869c";
pub const LEGACY_MOBILE_UPLOAD_PROGRESS_SOURCE_MANIFEST_SHA256: &str =
    "29dd22432c3807153944b701e72c0aee34481ecd8ed1bf9c46577c88ca9699da";
pub const LEGACY_MOBILE_UPLOAD_COMPLETE_SOURCE_MANIFEST_SHA256: &str =
    "08cb691aed6602219569e8c37977fe21b02b3f66dadf561d166916d8e84dfa04";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileUploadsSourceRoleV1 {
    Contract,
    Handler,
    Authentication,
    Authorization,
    Persistence,
    Service,
    Storage,
    ProviderEffect,
    Mount,
    Client,
    Environment,
    DependencyLock,
}

impl LegacyMobileUploadsSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Handler => "handler",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Persistence => "persistence",
            Self::Service => "service",
            Self::Storage => "storage",
            Self::ProviderEffect => "provider_effect",
            Self::Mount => "mount",
            Self::Client => "client",
            Self::Environment => "environment",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMobileUploadsSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyMobileUploadsSourceRoleV1,
}

const fn source(
    path: &'static str,
    symbol: &'static str,
    sha256: &'static str,
    role: LegacyMobileUploadsSourceRoleV1,
) -> LegacyMobileUploadsSourcePinV1 {
    LegacyMobileUploadsSourcePinV1 {
        path,
        symbol,
        sha256,
        role,
    }
}

const MOBILE_DOMAIN: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-domain/src/Mobile.ts",
    "MobileUploadCreateInput+MobileUploadProgressInput+MobileUploadCompleteInput+endpoints",
    "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
    LegacyMobileUploadsSourceRoleV1::Contract,
);
const STORAGE_DOMAIN: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-domain/src/Storage.ts",
    "UploadTarget",
    "a6ce2c9b6c70c7bd1a0f0539291ef2b7ce64093e46ab26646e432e10772bc75d",
    LegacyMobileUploadsSourceRoleV1::Contract,
);
const MOBILE_HANDLER: LegacyMobileUploadsSourcePinV1 = source(
    "apps/web/app/api/mobile/[...route]/route.ts",
    "createUpload+updateUploadProgress+completeUpload",
    "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
    LegacyMobileUploadsSourceRoleV1::Handler,
);
const AUTH_BACKEND: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/Auth.ts",
    "HttpAuthMiddlewareLive+CurrentUser",
    "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    LegacyMobileUploadsSourceRoleV1::Authentication,
);
const AUTH_DOMAIN: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-domain/src/Authentication.ts",
    "HttpAuthMiddleware+CurrentUser",
    "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    LegacyMobileUploadsSourceRoleV1::Authentication,
);
const DATABASE_SCHEMA: LegacyMobileUploadsSourcePinV1 = source(
    "packages/database/schema.ts",
    "users+organizations+organizationMembers+folders+videos+videoUploads+storageIntegrations",
    "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    LegacyMobileUploadsSourceRoleV1::Persistence,
);
const DATABASE_SERVICE: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/Database.ts",
    "Database use+transaction",
    "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
    LegacyMobileUploadsSourceRoleV1::Persistence,
);
const MOBILE_CLIENT: LegacyMobileUploadsSourcePinV1 = source(
    "apps/mobile/src/api/mobile.ts",
    "released createUpload+updateUploadProgress+completeUpload callers and upload target transport",
    "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08",
    LegacyMobileUploadsSourceRoleV1::Client,
);
const MOBILE_ORCHESTRATOR: LegacyMobileUploadsSourcePinV1 = source(
    "apps/mobile/src/uploads/runMobileUpload.ts",
    "released create-stream-progress-complete orchestration",
    "b67c264da096baa103df5beb7e26ff95f7002c85305c97f1ff327a63be6cf253",
    LegacyMobileUploadsSourceRoleV1::Client,
);
const HTTP_MOUNT: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-domain/src/Http/Api.ts",
    "ApiContract /api/mobile mount",
    "33f37588b210fe8be6584a51cd786b1347f5f7733fb2ac6b9111002e61437a25",
    LegacyMobileUploadsSourceRoleV1::Mount,
);
const VIDEOS_SERVICE: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/Videos/index.ts",
    "updateUploadProgress+getUploadProgress",
    "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
    LegacyMobileUploadsSourceRoleV1::Service,
);
const VIDEOS_REPO: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/Videos/VideosRepo.ts",
    "VideosRepo.create",
    "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
    LegacyMobileUploadsSourceRoleV1::Persistence,
);
const VIDEOS_POLICY: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/Videos/VideosPolicy.ts",
    "VideosPolicy.isOwner",
    "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
    LegacyMobileUploadsSourceRoleV1::Authorization,
);
const STORAGE_SERVICE: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/Storage/index.ts",
    "getWritableAccessForUser+createUploadTarget",
    "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
    LegacyMobileUploadsSourceRoleV1::Storage,
);
const S3_ACCESS: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
    "getPresignedPutUrl",
    "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
    LegacyMobileUploadsSourceRoleV1::Storage,
);
const S3_SELECTION: LegacyMobileUploadsSourcePinV1 = source(
    "packages/web-backend/src/S3Buckets/index.ts",
    "organization+user writable bucket selection",
    "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
    LegacyMobileUploadsSourceRoleV1::Storage,
);
const VIDEO_PROCESSING: LegacyMobileUploadsSourcePinV1 = source(
    "apps/web/lib/video-processing.ts",
    "transitionVideoToProcessing+startVideoProcessingWorkflow",
    "56d755ad564725c2912a48bce70e2410b991e2bb94889aba021ad4f1ecad32a0",
    LegacyMobileUploadsSourceRoleV1::ProviderEffect,
);
const PROCESS_WORKFLOW: LegacyMobileUploadsSourcePinV1 = source(
    "apps/web/workflows/process-video.ts",
    "processVideoWorkflow",
    "972696993e47609932fedb6973f75b2c26dafdca0363b07e061d57c777d4095d",
    LegacyMobileUploadsSourceRoleV1::ProviderEffect,
);
const SERVER_ENV: LegacyMobileUploadsSourcePinV1 = source(
    "packages/env/server.ts",
    "WEB_URL+CAP_VIDEOS_DEFAULT_PUBLIC+R2-compatible storage environment",
    "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
    LegacyMobileUploadsSourceRoleV1::Environment,
);
const LOCKFILE: LegacyMobileUploadsSourcePinV1 = source(
    "pnpm-lock.yaml",
    "Effect+Drizzle+AWS SDK+workflow dependency resolutions",
    "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    LegacyMobileUploadsSourceRoleV1::DependencyLock,
);

pub const LEGACY_MOBILE_UPLOAD_CREATE_SOURCES: &[LegacyMobileUploadsSourcePinV1] = &[
    MOBILE_DOMAIN,
    STORAGE_DOMAIN,
    MOBILE_HANDLER,
    AUTH_BACKEND,
    AUTH_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    MOBILE_CLIENT,
    MOBILE_ORCHESTRATOR,
    HTTP_MOUNT,
    VIDEOS_REPO,
    STORAGE_SERVICE,
    S3_ACCESS,
    S3_SELECTION,
    SERVER_ENV,
    LOCKFILE,
];

pub const LEGACY_MOBILE_UPLOAD_PROGRESS_SOURCES: &[LegacyMobileUploadsSourcePinV1] = &[
    MOBILE_DOMAIN,
    MOBILE_HANDLER,
    AUTH_BACKEND,
    AUTH_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    MOBILE_CLIENT,
    MOBILE_ORCHESTRATOR,
    HTTP_MOUNT,
    VIDEOS_SERVICE,
    VIDEOS_POLICY,
    LOCKFILE,
];

pub const LEGACY_MOBILE_UPLOAD_COMPLETE_SOURCES: &[LegacyMobileUploadsSourcePinV1] = &[
    MOBILE_DOMAIN,
    MOBILE_HANDLER,
    AUTH_BACKEND,
    AUTH_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    MOBILE_CLIENT,
    MOBILE_ORCHESTRATOR,
    HTTP_MOUNT,
    VIDEO_PROCESSING,
    PROCESS_WORKFLOW,
    LOCKFILE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileUploadOperationV1 {
    Create,
    Complete,
    Progress,
}

impl LegacyMobileUploadOperationV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::Create => LEGACY_MOBILE_UPLOAD_CREATE_OPERATION_ID,
            Self::Complete => LEGACY_MOBILE_UPLOAD_COMPLETE_OPERATION_ID,
            Self::Progress => LEGACY_MOBILE_UPLOAD_PROGRESS_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn method(self) -> &'static str {
        "POST"
    }

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Create => LEGACY_MOBILE_UPLOAD_CREATE_PATH,
            Self::Complete => LEGACY_MOBILE_UPLOAD_COMPLETE_PATH,
            Self::Progress => LEGACY_MOBILE_UPLOAD_PROGRESS_PATH,
        }
    }

    #[must_use]
    pub const fn sources(self) -> &'static [LegacyMobileUploadsSourcePinV1] {
        match self {
            Self::Create => LEGACY_MOBILE_UPLOAD_CREATE_SOURCES,
            Self::Complete => LEGACY_MOBILE_UPLOAD_COMPLETE_SOURCES,
            Self::Progress => LEGACY_MOBILE_UPLOAD_PROGRESS_SOURCES,
        }
    }

    #[must_use]
    pub const fn protected_gates(self) -> &'static [&'static str] {
        match self {
            Self::Complete => LEGACY_MOBILE_UPLOADS_PROVIDER_GATES,
            Self::Create | Self::Progress => LEGACY_MOBILE_UPLOADS_NO_PROTECTED_GATES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMobileUploadProfileV1 {
    pub operation: LegacyMobileUploadOperationV1,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency_retry: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_MOBILE_UPLOAD_PROFILES: &[LegacyMobileUploadProfileV1] = &[
    LegacyMobileUploadProfileV1 {
        operation: LegacyMobileUploadOperationV1::Create,
        success: "fresh_cap_summary_and_exact_r2_put_capability",
        validation: "bounded_json_video_content_type_finite_media_numbers_and_optional_safe_byte_length",
        authorization: "api_key_precedence_else_session_active_or_requested_accessible_organization_and_actor_owned_personal_folder",
        idempotency_retry: "released_client_sends_no_key_each_accepted_request_mints_a_fresh_video",
        failure: "400_schema_401_actor_403_organization_404_folder_500_storage_or_persistence",
    },
    LegacyMobileUploadProfileV1 {
        operation: LegacyMobileUploadOperationV1::Complete,
        success: "success_true_only_after_processing_provider_submission_is_admitted",
        validation: "exact_minted_raw_key_optional_safe_byte_length_and_observed_nonempty_r2_object",
        authorization: "api_key_precedence_else_session_and_actor_owned_video_non_disclosure",
        idempotency_retry: "one_durable_provider_intent_per_video_retries_never_duplicate_or_fabricate_submission",
        failure: "400_schema_401_actor_404_video_or_object_409_length_503_provider_execution_500_storage_or_persistence",
    },
    LegacyMobileUploadProfileV1 {
        operation: LegacyMobileUploadOperationV1::Progress,
        success: "success_true_with_uploaded_and_total_truncated_nonnegative_and_uploaded_clamped_to_total",
        validation: "finite_json_numbers_tightened_to_javascript_safe_integer_range",
        authorization: "api_key_precedence_else_session_and_actor_owned_video_non_disclosure",
        idempotency_retry: "server_timestamp_latest_accepted_request_wins_and_equal_retries_converge",
        failure: "400_schema_401_actor_404_video_500_persistence",
    },
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMobileUploadCreateInputV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    pub file_name: String,
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_length: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
}

impl LegacyMobileUploadCreateInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_wire_value(&self.file_name, 1_024)
            && valid_content_type(&self.content_type)
            && self.organization_id.as_deref().is_none_or(valid_cap_id)
            && self.folder_id.as_deref().is_none_or(valid_cap_id)
            && self.content_length.is_none_or(valid_safe_number)
            && [self.duration_seconds, self.width, self.height, self.fps]
                .into_iter()
                .all(|value| value.is_none_or(f64::is_finite))
    }

    #[must_use]
    pub fn normalized_content_length(&self) -> Option<u64> {
        self.content_length.and_then(safe_integer)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMobileUploadProgressInputV1 {
    pub uploaded: f64,
    pub total: f64,
}

impl LegacyMobileUploadProgressInputV1 {
    #[must_use]
    pub fn normalized(&self) -> Option<(u64, u64)> {
        let uploaded = truncated_nonnegative_safe(self.uploaded)?;
        let total = truncated_nonnegative_safe(self.total)?;
        Some((uploaded.min(total), total))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyMobileUploadCompleteInputV1 {
    pub raw_file_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_length: Option<f64>,
}

impl LegacyMobileUploadCompleteInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_object_key(&self.raw_file_key) && self.content_length.is_none_or(valid_safe_number)
    }

    #[must_use]
    pub fn normalized_content_length(&self) -> Option<u64> {
        self.content_length.and_then(safe_integer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileUploadPutTargetV1 {
    #[serde(rename = "type")]
    pub target_type: &'static str,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileUploadCreateResponseV1 {
    pub id: String,
    pub share_url: String,
    pub raw_file_key: String,
    pub upload: LegacyMobileUploadPutTargetV1,
    pub cap: LegacyMobileCapSummaryV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LegacyMobileUploadSuccessV1 {
    pub success: bool,
}

#[must_use]
pub fn legacy_mobile_upload_title(file_name: &str) -> String {
    let last_slash = file_name.rfind('/');
    let extension = file_name
        .rfind('.')
        .filter(|dot| last_slash.is_none_or(|slash| *dot > slash) && *dot + 1 < file_name.len());
    let without_extension = extension.map_or(file_name, |dot| &file_name[..dot]);
    let title = legacy_mobile_trim(without_extension);
    if title.is_empty() {
        "Mobile Upload".into()
    } else {
        title.into()
    }
}

#[must_use]
pub fn legacy_mobile_upload_extension(file_name: &str, content_type: &str) -> String {
    if let Some(extension) = file_name.rsplit('.').next().filter(|extension| {
        !extension.is_empty()
            && extension.len() < file_name.len()
            && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }) {
        return extension.to_ascii_lowercase();
    }
    if content_type.contains("quicktime") {
        "mov".into()
    } else if content_type.contains("webm") {
        "webm".into()
    } else if content_type.contains("matroska") {
        "mkv".into()
    } else if content_type.contains("x-msvideo") {
        "avi".into()
    } else if content_type.contains("x-m4v") {
        "m4v".into()
    } else {
        "mp4".into()
    }
}

#[must_use]
pub fn legacy_mobile_upload_raw_key(
    legacy_actor_id: &str,
    legacy_video_id: &str,
    extension: &str,
) -> Option<String> {
    if !valid_cap_id(legacy_actor_id)
        || !valid_cap_id(legacy_video_id)
        || extension.is_empty()
        || extension.len() > 64
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(format!(
        "{legacy_actor_id}/{legacy_video_id}/raw-upload.{extension}"
    ))
}

#[must_use]
pub fn legacy_mobile_upload_source_manifest(operation: LegacyMobileUploadOperationV1) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-mobile-uploads-source-manifest-v1\0");
    digest.update(operation.operation_id().as_bytes());
    digest.update([0]);
    for source in operation.sources() {
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

fn valid_cap_id(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

fn valid_wire_value(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn valid_content_type(value: &str) -> bool {
    value.starts_with("video/")
        && value.len() <= 127
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn valid_safe_number(value: f64) -> bool {
    value.is_finite()
        && value >= 0.0
        && value <= LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER as f64
        && value.fract() == 0.0
}

fn safe_integer(value: f64) -> Option<u64> {
    valid_safe_number(value).then_some(value as u64)
}

fn truncated_nonnegative_safe(value: f64) -> Option<u64> {
    if !value.is_finite() || value.abs() > LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER as f64 {
        return None;
    }
    Some(value.trunc().max(0.0) as u64)
}

fn valid_object_key(value: &str) -> bool {
    value.len() >= 33
        && value.len() <= 2_048
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.contains(['?', '#', '%'])
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_closure_and_protected_gate_are_frozen() {
        assert_eq!(LEGACY_MOBILE_UPLOAD_CREATE_SOURCES.len(), 16);
        assert_eq!(LEGACY_MOBILE_UPLOAD_PROGRESS_SOURCES.len(), 12);
        assert_eq!(LEGACY_MOBILE_UPLOAD_COMPLETE_SOURCES.len(), 12);
        assert_eq!(
            legacy_mobile_upload_source_manifest(LegacyMobileUploadOperationV1::Create),
            LEGACY_MOBILE_UPLOAD_CREATE_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            legacy_mobile_upload_source_manifest(LegacyMobileUploadOperationV1::Progress),
            LEGACY_MOBILE_UPLOAD_PROGRESS_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            legacy_mobile_upload_source_manifest(LegacyMobileUploadOperationV1::Complete),
            LEGACY_MOBILE_UPLOAD_COMPLETE_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            LegacyMobileUploadOperationV1::Complete.protected_gates(),
            &["provider_execution"]
        );
    }

    #[test]
    fn filename_helpers_match_the_mobile_handler_and_keys_cannot_escape() {
        assert_eq!(
            legacy_mobile_upload_title(" holiday clip.MOV "),
            "holiday clip"
        );
        assert_eq!(legacy_mobile_upload_title(".mp4"), "Mobile Upload");
        assert_eq!(
            legacy_mobile_upload_extension("clip.MOV", "video/quicktime"),
            "mov"
        );
        assert_eq!(
            legacy_mobile_upload_extension("clip", "video/x-matroska"),
            "mkv"
        );
        assert_eq!(
            legacy_mobile_upload_raw_key("0123456789abcde", "0123456789abcdf", "mp4").as_deref(),
            Some("0123456789abcde/0123456789abcdf/raw-upload.mp4")
        );
        assert!(
            legacy_mobile_upload_raw_key("0123456789abcde", "0123456789abcdf", "../mp4").is_none()
        );
    }

    #[test]
    fn progress_applies_javascript_truncation_nonnegative_and_total_clamp() {
        assert_eq!(
            LegacyMobileUploadProgressInputV1 {
                uploaded: 11.9,
                total: 10.8,
            }
            .normalized(),
            Some((10, 10))
        );
        assert_eq!(
            LegacyMobileUploadProgressInputV1 {
                uploaded: -4.9,
                total: 10.2,
            }
            .normalized(),
            Some((0, 10))
        );
        assert!(
            LegacyMobileUploadProgressInputV1 {
                uploaded: f64::INFINITY,
                total: 10.0,
            }
            .normalized()
            .is_none()
        );
    }
}
