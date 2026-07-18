//! Source-pinned contract for Cap's Chrome-extension instant recordings.
//!
//! The three carriers share Cap's API-key-or-session middleware. Creation is
//! deliberately non-idempotent: each accepted request mints a new Cap NanoID
//! wire alias and a collision-safe native video. Progress is timestamp fenced
//! and Frame additionally refuses byte regressions. Delete is owner-scoped and
//! retains the immutable wire alias while converging an R2 prefix cleanup.

use std::{collections::BTreeMap, fmt::Write as _};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

pub const LEGACY_EXTENSION_INSTANT_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_EXTENSION_INSTANT_CREATE_OPERATION_ID: &str = "cap-v1-00422c50f4d39053";
pub const LEGACY_EXTENSION_INSTANT_DELETE_OPERATION_ID: &str = "cap-v1-8fd4741d6e52465e";
pub const LEGACY_EXTENSION_INSTANT_PROGRESS_OPERATION_ID: &str = "cap-v1-82dec55d0fbea3db";
pub const LEGACY_EXTENSION_INSTANT_CREATE_PATH: &str = "/api/extension/instant-recordings";
pub const LEGACY_EXTENSION_INSTANT_PROGRESS_PATH: &str =
    "/api/extension/instant-recordings/progress";
pub const LEGACY_EXTENSION_INSTANT_DELETE_PATH: &str = "/api/extension/instant-recordings/:videoId";
pub const LEGACY_EXTENSION_INSTANT_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_EXTENSION_INSTANT_MAX_VALUE_BYTES: usize = 4 * 1024;
pub const LEGACY_EXTENSION_INSTANT_WIRE_ID_BYTES: usize = 15;
pub const LEGACY_EXTENSION_INSTANT_UPLOAD_TTL_SECONDS: u32 = 900;
pub const LEGACY_EXTENSION_INSTANT_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_EXTENSION_INSTANT_SOURCE_MANIFEST_SHA256: &str =
    "9a36f7c2832868b8ae4c1b8de1a7e825b15210323243becec599b39dabe208a7";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyExtensionInstantSourceRoleV1 {
    Contract,
    Handler,
    Service,
    Authorization,
    Persistence,
    Storage,
    Mount,
    Environment,
    Client,
    DependencyLock,
}

impl LegacyExtensionInstantSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Handler => "handler",
            Self::Service => "service",
            Self::Authorization => "authorization",
            Self::Persistence => "persistence",
            Self::Storage => "storage",
            Self::Mount => "mount",
            Self::Environment => "environment",
            Self::Client => "client",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyExtensionInstantSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyExtensionInstantSourceRoleV1,
}

pub const LEGACY_EXTENSION_INSTANT_SOURCES: &[LegacyExtensionInstantSourcePinV1] = &[
    source(
        "packages/web-domain/src/Extension.ts",
        "ExtensionHttpApi+ExtensionApiPaths+ExtensionUploadProgressUpdateInput",
        "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        LegacyExtensionInstantSourceRoleV1::Contract,
    ),
    source(
        "packages/web-domain/src/Video.ts",
        "InstantRecordingCreateInput+InstantRecordingCreateSuccess+VideoNotFoundError",
        "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
        LegacyExtensionInstantSourceRoleV1::Contract,
    ),
    source(
        "packages/web-domain/src/Storage.ts",
        "UploadTarget",
        "a6ce2c9b6c70c7bd1a0f0539291ef2b7ce64093e46ab26646e432e10772bc75d",
        LegacyExtensionInstantSourceRoleV1::Contract,
    ),
    source(
        "packages/web-domain/src/Policy.ts",
        "PolicyDeniedError+withPolicy",
        "0621949aa1f994836d0d168b39dc3aada3ad0478052b712de564b105c94ebe5c",
        LegacyExtensionInstantSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-domain/src/Authentication.ts",
        "CurrentUser+HttpAuthMiddleware",
        "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
        LegacyExtensionInstantSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-domain/src/Http/Api.ts",
        "ApiContract /api/extension mount",
        "33f37588b210fe8be6584a51cd786b1347f5f7733fb2ac6b9111002e61437a25",
        LegacyExtensionInstantSourceRoleV1::Mount,
    ),
    source(
        "packages/web-backend/src/Extension/Http.ts",
        "createInstantRecording+updateInstantRecordingProgress+deleteInstantRecording",
        "8bcaeadc626ec4b0bd43ca6f6e2bba643c7386f16f033b7f5d3c103e2173c602",
        LegacyExtensionInstantSourceRoleV1::Handler,
    ),
    source(
        "packages/web-backend/src/Videos/index.ts",
        "Videos.createInstantRecording+updateUploadProgress+delete",
        "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
        LegacyExtensionInstantSourceRoleV1::Service,
    ),
    source(
        "packages/web-backend/src/Videos/VideosRepo.ts",
        "VideosRepo.create+getById+delete",
        "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
        LegacyExtensionInstantSourceRoleV1::Persistence,
    ),
    source(
        "packages/web-backend/src/Videos/VideosPolicy.ts",
        "VideosPolicy.isOwner",
        "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
        LegacyExtensionInstantSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-backend/src/Storage/index.ts",
        "getWritableAccessForUser+createUploadTarget",
        "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
        LegacyExtensionInstantSourceRoleV1::Storage,
    ),
    source(
        "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        "getPresignedPostUrl+deleteObjects",
        "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
        LegacyExtensionInstantSourceRoleV1::Storage,
    ),
    source(
        "packages/web-backend/src/S3Buckets/index.ts",
        "organization+user bucket selection",
        "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
        LegacyExtensionInstantSourceRoleV1::Storage,
    ),
    source(
        "packages/web-backend/src/Auth.ts",
        "HttpAuthMiddlewareLive",
        "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
        LegacyExtensionInstantSourceRoleV1::Authorization,
    ),
    source(
        "packages/web-backend/src/Database.ts",
        "Database",
        "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
        LegacyExtensionInstantSourceRoleV1::Persistence,
    ),
    source(
        "packages/web-backend/src/Http/Live.ts",
        "ExtensionHttpLive",
        "fa73f7797f44f11271e0e59fe14817144733ea06fb954c30e8f2f4720fa7216c",
        LegacyExtensionInstantSourceRoleV1::Mount,
    ),
    source(
        "packages/database/schema.ts",
        "videos+videoUploads+organizations+folders+authApiKeys",
        "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        LegacyExtensionInstantSourceRoleV1::Persistence,
    ),
    source(
        "packages/database/helpers.ts",
        "nanoId",
        "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        LegacyExtensionInstantSourceRoleV1::Persistence,
    ),
    source(
        "packages/env/server.ts",
        "WEB_URL+CAP_VIDEOS_DEFAULT_PUBLIC+R2-compatible storage environment",
        "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
        LegacyExtensionInstantSourceRoleV1::Environment,
    ),
    source(
        "packages/env/build.ts",
        "NEXT_PUBLIC_IS_CAP",
        "454bc82ebd9ca83bae656336b67287d13bc351d357c2143444d226d84f2707bd",
        LegacyExtensionInstantSourceRoleV1::Environment,
    ),
    source(
        "apps/chrome-extension/src/shared/api.ts",
        "createInstantRecording+updateUploadProgress+deleteInstantRecording",
        "7439a031accac54fcd727c8b643a40f1fca885fbaa15d769c8a6c1e99bf28df7",
        LegacyExtensionInstantSourceRoleV1::Client,
    ),
    source(
        "apps/chrome-extension/src/shared/types.ts",
        "InstantRecordingCreation",
        "fdd5da209e33f6a28158b4a33e52e147fb03de44c8aa6cb39b6d9cc20b52ead1",
        LegacyExtensionInstantSourceRoleV1::Client,
    ),
    source(
        "apps/chrome-extension/src/offscreen/recorder.ts",
        "instant create+progress+failure delete lifecycle",
        "03c2128a66fc6ff2adfb5116907787fde065e2c4887aa1a8242180f1a8546ce4",
        LegacyExtensionInstantSourceRoleV1::Client,
    ),
    source(
        "apps/web/utils/upload-target.ts",
        "UploadTarget transport aliases",
        "4677b454e1766367c56d0d8d348628b200f41cde59e064db25699a9e4e2038a3",
        LegacyExtensionInstantSourceRoleV1::Client,
    ),
    source(
        "pnpm-lock.yaml",
        "Effect+Drizzle+AWS SDK+nanoid dependency resolutions",
        "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        LegacyExtensionInstantSourceRoleV1::DependencyLock,
    ),
];

const fn source(
    path: &'static str,
    symbol: &'static str,
    sha256: &'static str,
    role: LegacyExtensionInstantSourceRoleV1,
) -> LegacyExtensionInstantSourcePinV1 {
    LegacyExtensionInstantSourcePinV1 {
        path,
        symbol,
        sha256,
        role,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyExtensionInstantProfileV1 {
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency_retry: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_EXTENSION_INSTANT_PROFILES: &[LegacyExtensionInstantProfileV1] = &[
    LegacyExtensionInstantProfileV1 {
        operation_id: LEGACY_EXTENSION_INSTANT_CREATE_OPERATION_ID,
        method: "POST",
        path: LEGACY_EXTENSION_INSTANT_CREATE_PATH,
        success: "video_alias_share_url_and_presigned_r2_upload_target",
        validation: "bounded_json_finite_optional_media_metadata_and_live_folder",
        authorization: "api_key_precedence_else_session_actor_active_organization_and_r2_tenant",
        idempotency_retry: "no_client_key_each_success_creates_a_new_recording",
        failure: "400_schema_401_actor_404_tenant_403_policy_500_storage_or_persistence",
    },
    LegacyExtensionInstantProfileV1 {
        operation_id: LEGACY_EXTENSION_INSTANT_PROGRESS_OPERATION_ID,
        method: "POST",
        path: LEGACY_EXTENSION_INSTANT_PROGRESS_PATH,
        success: "success_true_with_uploaded_clamped_to_total",
        validation: "nonnegative_safe_integers_and_parseable_updated_at",
        authorization: "api_key_precedence_else_session_and_actor_owned_video_alias",
        idempotency_retry: "stale_or_byte_regressing_updates_are_noops_equal_retries_converge",
        failure: "400_schema_401_actor_404_video_500_persistence",
    },
    LegacyExtensionInstantProfileV1 {
        operation_id: LEGACY_EXTENSION_INSTANT_DELETE_OPERATION_ID,
        method: "DELETE",
        path: LEGACY_EXTENSION_INSTANT_DELETE_PATH,
        success: "success_true_after_durable_tombstone_and_r2_prefix_cleanup",
        validation: "exact_fifteen_character_cap_video_alias",
        authorization: "api_key_precedence_else_session_and_actor_owned_tenant_storage_prefix",
        idempotency_retry: "pending_cleanup_retries_converge_completed_tombstone_is_not_found",
        failure: "401_actor_404_video_500_storage_or_persistence",
    },
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyExtensionInstantCreateInputV1 {
    pub org_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_upload_progress: Option<bool>,
}

impl LegacyExtensionInstantCreateInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_value(&self.org_id)
            && self.folder_id.as_deref().is_none_or(valid_value)
            && self.duration_seconds.is_none_or(f64::is_finite)
            && self.width.is_none_or(f64::is_finite)
            && self.height.is_none_or(f64::is_finite)
            && self.resolution.as_deref().is_none_or(valid_optional_value)
            && self.video_codec.as_deref().is_none_or(valid_optional_value)
            && self.audio_codec.as_deref().is_none_or(valid_optional_value)
    }

    #[must_use]
    pub fn supports_progress(&self) -> bool {
        self.supports_upload_progress.unwrap_or(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyExtensionInstantProgressInputV1 {
    pub video_id: String,
    pub uploaded: u64,
    pub total: u64,
    pub updated_at: String,
}

impl LegacyExtensionInstantProgressInputV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        legacy_extension_instant_valid_wire_id(&self.video_id)
            && self.uploaded <= 9_007_199_254_740_991
            && self.total <= 9_007_199_254_740_991
            && !self.updated_at.is_empty()
            && self.updated_at.len() <= 64
            && self.updated_at.is_ascii()
    }

    #[must_use]
    pub fn clamped_uploaded(&self) -> u64 {
        self.uploaded.min(self.total)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyExtensionInstantCreateSuccessV1 {
    pub id: String,
    #[serde(rename = "shareUrl")]
    pub share_url: String,
    pub upload: LegacyExtensionInstantPutTargetV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyExtensionInstantPutTargetV1 {
    #[serde(rename = "type")]
    pub target_type: &'static str,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[must_use]
pub fn legacy_extension_instant_valid_wire_id(value: &str) -> bool {
    value.len() == LEGACY_EXTENSION_INSTANT_WIRE_ID_BYTES
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

#[must_use]
pub fn legacy_extension_instant_object_key(actor_id: &str, wire_video_id: &str) -> Option<String> {
    valid_value(actor_id)
        .then_some(())
        .filter(|()| legacy_extension_instant_valid_wire_id(wire_video_id))
        .map(|()| format!("{actor_id}/{wire_video_id}/result.mp4"))
}

#[must_use]
pub fn legacy_extension_instant_title(day: u8, month: u8, year: i32) -> Option<String> {
    let month = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(usize::from(month.checked_sub(1)?))?;
    (day > 0 && day <= 31 && (1970..=9999).contains(&year))
        .then(|| format!("Cap Recording - {day} {month} {year}"))
}

#[must_use]
pub fn legacy_extension_instant_share_url(
    web_origin: &Url,
    custom_domain: Option<&str>,
    domain_verified: bool,
    wire_video_id: &str,
) -> Option<String> {
    if !legacy_extension_instant_valid_wire_id(wire_video_id) {
        return None;
    }
    let canonical = web_origin
        .join(&format!("/s/{wire_video_id}"))
        .ok()?
        .to_string();
    let Some(custom_domain) = custom_domain.filter(|_| domain_verified) else {
        return Some(canonical);
    };
    if custom_domain.is_empty()
        || custom_domain.len() > 255
        || custom_domain.chars().any(char::is_control)
    {
        return Some(canonical);
    }
    let domain = if custom_domain.starts_with("http://") || custom_domain.starts_with("https://") {
        custom_domain.to_owned()
    } else {
        format!("https://{custom_domain}")
    };
    Some(format!("{domain}/s/{wire_video_id}"))
}

#[must_use]
pub fn legacy_extension_instant_upload_headers(
    actor_id: &str,
    input: &LegacyExtensionInstantCreateInputV1,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Content-Type".into(), "video/mp4".into()),
        ("x-amz-meta-userid".into(), actor_id.into()),
        (
            "x-amz-meta-duration".into(),
            input
                .duration_seconds
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        (
            "x-amz-meta-resolution".into(),
            input.resolution.clone().unwrap_or_default(),
        ),
        (
            "x-amz-meta-videocodec".into(),
            input.video_codec.clone().unwrap_or_default(),
        ),
        (
            "x-amz-meta-audiocodec".into(),
            input.audio_codec.clone().unwrap_or_default(),
        ),
    ])
}

#[must_use]
pub fn legacy_extension_instant_source_manifest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-extension-instant-source-manifest-v1\0");
    for source in LEGACY_EXTENSION_INSTANT_SOURCES {
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

fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LEGACY_EXTENSION_INSTANT_MAX_VALUE_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_optional_value(value: &str) -> bool {
    value.len() <= LEGACY_EXTENSION_INSTANT_MAX_VALUE_BYTES && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> LegacyExtensionInstantCreateInputV1 {
        LegacyExtensionInstantCreateInputV1 {
            org_id: "org".into(),
            folder_id: None,
            duration_seconds: Some(12.5),
            resolution: Some("1920x1080".into()),
            width: Some(1920.0),
            height: Some(1080.0),
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
            supports_upload_progress: Some(true),
        }
    }

    #[test]
    fn source_and_five_axis_closures_are_complete_and_provider_free() {
        assert_eq!(LEGACY_EXTENSION_INSTANT_SOURCES.len(), 25);
        assert_eq!(LEGACY_EXTENSION_INSTANT_PROFILES.len(), 3);
        assert_eq!(LEGACY_EXTENSION_INSTANT_NO_PROTECTED_GATES, &[] as &[&str]);
        assert_eq!(
            legacy_extension_instant_source_manifest(),
            LEGACY_EXTENSION_INSTANT_SOURCE_MANIFEST_SHA256
        );
        assert!(LEGACY_EXTENSION_INSTANT_PROFILES.iter().all(|profile| {
            !profile.success.is_empty()
                && !profile.validation.is_empty()
                && !profile.authorization.is_empty()
                && !profile.idempotency_retry.is_empty()
                && !profile.failure.is_empty()
        }));
    }

    #[test]
    fn create_schema_defaults_progress_and_preserves_exact_metadata_aliases() {
        let input = input();
        assert!(input.valid());
        assert!(input.supports_progress());
        let headers = legacy_extension_instant_upload_headers("actor", &input);
        assert_eq!(
            headers.get("Content-Type").map(String::as_str),
            Some("video/mp4")
        );
        assert_eq!(
            headers.get("x-amz-meta-duration").map(String::as_str),
            Some("12.5")
        );
        assert!(
            !LegacyExtensionInstantCreateInputV1 {
                width: Some(f64::INFINITY),
                ..input
            }
            .valid()
        );
    }

    #[test]
    fn progress_clamps_uploaded_and_rejects_non_cap_wire_aliases() {
        let progress = LegacyExtensionInstantProgressInputV1 {
            video_id: "0123456789abcde".into(),
            uploaded: 12,
            total: 10,
            updated_at: "2026-07-17T12:34:56.789Z".into(),
        };
        assert!(progress.valid());
        assert_eq!(progress.clamped_uploaded(), 10);
        assert!(!legacy_extension_instant_valid_wire_id("018f47a6-7b1c"));
    }

    #[test]
    fn title_share_and_storage_prefixes_are_deterministic() {
        let wire = "0123456789abcde";
        assert_eq!(
            legacy_extension_instant_title(17, 7, 2026).as_deref(),
            Some("Cap Recording - 17 July 2026")
        );
        let origin = Url::parse("https://frame.engmanager.xyz").expect("origin");
        assert_eq!(
            legacy_extension_instant_share_url(
                &origin,
                Some("recordings.engmanager.xyz"),
                true,
                wire,
            )
            .as_deref(),
            Some("https://recordings.engmanager.xyz/s/0123456789abcde")
        );
        assert_eq!(
            legacy_extension_instant_object_key("actor", wire).as_deref(),
            Some("actor/0123456789abcde/result.mp4")
        );
    }
}
