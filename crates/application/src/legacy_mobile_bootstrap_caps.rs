//! Source-pinned contracts for Cap's six mobile bootstrap and cap-read routes.
//!
//! The family is deliberately owner-scoped. Cap's list is also scoped to the
//! actor's active organization, while detail, download, playback, and delete
//! identify an owned video directly. Private R2 keys never cross the wire;
//! read capabilities sign only GET so clients may send ordinary Range
//! headers without widening the object or method authority.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{LegacyMobileCapSummaryV1, LegacyMobileUploadProgressV1};

pub const LEGACY_MOBILE_BOOTSTRAP_CAPS_CAP_COMMIT: &str =
    "6ba69561ac86b8efdb17616d6727f9638015546b";

pub const LEGACY_MOBILE_BOOTSTRAP_OPERATION_ID: &str = "cap-v1-32a24fe16a4c4a4f";
pub const LEGACY_MOBILE_CAPS_LIST_OPERATION_ID: &str = "cap-v1-951ad1523ae9dff4";
pub const LEGACY_MOBILE_CAP_DELETE_OPERATION_ID: &str = "cap-v1-6b8a689bf00a9187";
pub const LEGACY_MOBILE_CAP_GET_OPERATION_ID: &str = "cap-v1-7f0ed5caf3eaf97c";
pub const LEGACY_MOBILE_CAP_DOWNLOAD_OPERATION_ID: &str = "cap-v1-95fe41c72ce5ca9f";
pub const LEGACY_MOBILE_CAP_PLAYBACK_OPERATION_ID: &str = "cap-v1-bde34617e42a8834";

pub const LEGACY_MOBILE_BOOTSTRAP_PATH: &str = "/api/mobile/bootstrap";
pub const LEGACY_MOBILE_CAPS_PATH: &str = "/api/mobile/caps";
pub const LEGACY_MOBILE_CAP_PATH: &str = "/api/mobile/caps/:id";
pub const LEGACY_MOBILE_CAP_DOWNLOAD_PATH: &str = "/api/mobile/caps/:id/download";
pub const LEGACY_MOBILE_CAP_PLAYBACK_PATH: &str = "/api/mobile/caps/:id/playback";

pub const LEGACY_MOBILE_BOOTSTRAP_CAPS_POLICY: &str = "client_compatibility.v1";
pub const LEGACY_MOBILE_BOOTSTRAP_CAPS_AUTH: &str = "session_or_api_key";
pub const LEGACY_MOBILE_BOOTSTRAP_CAPS_MAX_BODY_BYTES: usize = 0;
pub const LEGACY_MOBILE_BOOTSTRAP_CAPS_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_MOBILE_CAPS_DEFAULT_PAGE: u32 = 1;
pub const LEGACY_MOBILE_CAPS_MAX_PAGE: u32 = 10_000;
pub const LEGACY_MOBILE_CAPS_DEFAULT_LIMIT: u32 = 20;
pub const LEGACY_MOBILE_CAPS_MAX_LIMIT: u32 = 50;
pub const LEGACY_MOBILE_R2_GET_TTL_SECONDS: u32 = 3_600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileBootstrapUserV1 {
    pub id: String,
    pub name: Option<String>,
    pub email: String,
    pub image_url: Option<String>,
    pub active_organization_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileBootstrapOrganizationV1 {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileBootstrapFolderV1 {
    pub id: String,
    pub name: String,
    pub color: String,
    pub parent_id: Option<String>,
    pub video_count: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileBootstrapResponseV1 {
    pub user: LegacyMobileBootstrapUserV1,
    pub organizations: Vec<LegacyMobileBootstrapOrganizationV1>,
    pub active_organization_id: Option<String>,
    pub root_folders: Vec<LegacyMobileBootstrapFolderV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileCapsListResponseV1 {
    pub folders: Vec<LegacyMobileBootstrapFolderV1>,
    pub caps: Vec<LegacyMobileCapSummaryV1>,
    pub page: f64,
    pub limit: f64,
    pub total: f64,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileCapCommentAuthorV1 {
    pub id: String,
    pub name: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileCapCommentV1 {
    pub id: String,
    pub video_id: String,
    #[serde(rename = "type")]
    pub comment_type: String,
    pub content: String,
    pub timestamp: Option<f64>,
    pub parent_comment_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub author: LegacyMobileCapCommentAuthorV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileCapDetailV1 {
    pub cap: LegacyMobileCapSummaryV1,
    pub summary: Option<String>,
    pub chapters: Vec<LegacyMobileChapterV1>,
    pub transcription_status: Option<String>,
    pub comments: Vec<LegacyMobileCapCommentV1>,
    pub share_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobilePlaybackResponseV1 {
    pub kind: String,
    pub url: String,
    pub transcript_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileDownloadResponseV1 {
    pub file_name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMobileSuccessResponseV1 {
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyMobileCapProjectionV1 {
    pub legacy_video_id: String,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub owner_name: String,
    pub duration_seconds: Option<f64>,
    pub legacy_folder_id: Option<String>,
    pub public: bool,
    pub protected: bool,
    pub view_count: f64,
    pub comment_count: f64,
    pub reaction_count: f64,
    pub upload: Option<LegacyMobileUploadProgressV1>,
}

impl LegacyMobileCapProjectionV1 {
    pub fn into_summary(
        self,
        web_url: &str,
        thumbnail_url: Option<String>,
    ) -> Option<LegacyMobileCapSummaryV1> {
        let created_at = legacy_mobile_iso_from_millis(self.created_at_ms)?;
        let updated_at = legacy_mobile_iso_from_millis(self.updated_at_ms)?;
        Some(LegacyMobileCapSummaryV1 {
            share_url: format!(
                "{}/s/{}",
                web_url.trim_end_matches('/'),
                self.legacy_video_id
            ),
            id: self.legacy_video_id,
            title: self.title,
            created_at,
            updated_at,
            owner_name: self.owner_name,
            duration_seconds: self.duration_seconds,
            thumbnail_url,
            folder_id: self.legacy_folder_id,
            public: self.public,
            protected: self.protected,
            view_count: self.view_count,
            comment_count: self.comment_count,
            reaction_count: self.reaction_count,
            upload: self.upload,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileBootstrapCapsSourceRoleV1 {
    Declaration,
    Handler,
    Authentication,
    DatabaseSchema,
    ReleasedClient,
    VideoAuthority,
    StorageAuthority,
    ImageAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMobileBootstrapCapsSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyMobileBootstrapCapsSourceRoleV1,
}

const DECLARATION_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-domain/src/Mobile.ts",
        symbol: "mobile bootstrap/caps declarations",
        sha256: "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
        role: LegacyMobileBootstrapCapsSourceRoleV1::Declaration,
    };
const HANDLER_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 = LegacyMobileBootstrapCapsSourcePinV1 {
    path: "apps/web/app/api/mobile/[...route]/route.ts",
    symbol: "mobile bootstrap/caps handlers",
    sha256: "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
    role: LegacyMobileBootstrapCapsSourceRoleV1::Handler,
};
const AUTH_BACKEND_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-backend/src/Auth.ts",
        symbol: "HttpAuthMiddlewareLive+CurrentUser",
        sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
        role: LegacyMobileBootstrapCapsSourceRoleV1::Authentication,
    };
const AUTH_DOMAIN_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-domain/src/Authentication.ts",
        symbol: "HttpAuthMiddleware+CurrentUser",
        sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
        role: LegacyMobileBootstrapCapsSourceRoleV1::Authentication,
    };
const SCHEMA_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 = LegacyMobileBootstrapCapsSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users+organizations+organizationMembers+folders+videos+comments+videoUploads+authApiKeys",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyMobileBootstrapCapsSourceRoleV1::DatabaseSchema,
};
const CLIENT_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 = LegacyMobileBootstrapCapsSourcePinV1 {
    path: "apps/mobile/src/api/mobile.ts",
    symbol: "bootstrap+listCaps+getCap+deleteCap+getDownload+getPlayback released callers",
    sha256: "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08",
    role: LegacyMobileBootstrapCapsSourceRoleV1::ReleasedClient,
};
const VIDEOS_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 = LegacyMobileBootstrapCapsSourcePinV1 {
    path: "packages/web-backend/src/Videos/index.ts",
    symbol: "delete+getDownloadInfo+getThumbnailURL+getAnalytics+getAnalyticsBulk",
    sha256: "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
    role: LegacyMobileBootstrapCapsSourceRoleV1::VideoAuthority,
};
const VIDEO_REPO_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-backend/src/Videos/VideosRepo.ts",
        symbol: "getById+delete",
        sha256: "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
        role: LegacyMobileBootstrapCapsSourceRoleV1::VideoAuthority,
    };
const VIDEO_DOMAIN_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        symbol: "Video.getSource+Mp4Source+M3U8Source+SegmentsSource",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
        role: LegacyMobileBootstrapCapsSourceRoleV1::VideoAuthority,
    };
const STORAGE_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 = LegacyMobileBootstrapCapsSourcePinV1 {
    path: "packages/web-backend/src/Storage/index.ts",
    symbol: "getAccessForVideo+S3 list/head/sign/delete",
    sha256: "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
    role: LegacyMobileBootstrapCapsSourceRoleV1::StorageAuthority,
};
const S3_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 = LegacyMobileBootstrapCapsSourcePinV1 {
    path: "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
    symbol: "DEFAULT_PRESIGNED_GET_EXPIRES_SECONDS+listObjects+headObject+deleteObjects",
    sha256: "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
    role: LegacyMobileBootstrapCapsSourceRoleV1::StorageAuthority,
};
const IMAGE_UPLOADS_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-backend/src/ImageUploads/index.ts",
        symbol: "resolveImageUrl",
        sha256: "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
        role: LegacyMobileBootstrapCapsSourceRoleV1::ImageAuthority,
    };
const IMAGE_DOMAIN_SOURCE: LegacyMobileBootstrapCapsSourcePinV1 =
    LegacyMobileBootstrapCapsSourcePinV1 {
        path: "packages/web-domain/src/ImageUpload.ts",
        symbol: "extractFileKey",
        sha256: "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
        role: LegacyMobileBootstrapCapsSourceRoleV1::ImageAuthority,
    };

macro_rules! handler {
    ($symbol:literal) => {
        LegacyMobileBootstrapCapsSourcePinV1 {
            symbol: $symbol,
            ..HANDLER_SOURCE
        }
    };
}
macro_rules! declaration {
    ($symbol:literal) => {
        LegacyMobileBootstrapCapsSourcePinV1 {
            symbol: $symbol,
            ..DECLARATION_SOURCE
        }
    };
}
macro_rules! client {
    ($symbol:literal) => {
        LegacyMobileBootstrapCapsSourcePinV1 {
            symbol: $symbol,
            ..CLIENT_SOURCE
        }
    };
}

pub const LEGACY_MOBILE_BOOTSTRAP_SOURCES: &[LegacyMobileBootstrapCapsSourcePinV1] = &[
    handler!("mobile handler:bootstrap"),
    declaration!("bootstrap"),
    AUTH_BACKEND_SOURCE,
    AUTH_DOMAIN_SOURCE,
    SCHEMA_SOURCE,
    client!("bootstrap released caller"),
    IMAGE_UPLOADS_SOURCE,
    IMAGE_DOMAIN_SOURCE,
    S3_SOURCE,
];
pub const LEGACY_MOBILE_CAPS_LIST_SOURCES: &[LegacyMobileBootstrapCapsSourcePinV1] = &[
    handler!("mobile handler:listCaps"),
    declaration!("listCaps"),
    AUTH_BACKEND_SOURCE,
    AUTH_DOMAIN_SOURCE,
    SCHEMA_SOURCE,
    client!("listCaps released caller"),
    VIDEOS_SOURCE,
    VIDEO_DOMAIN_SOURCE,
    STORAGE_SOURCE,
    S3_SOURCE,
];
pub const LEGACY_MOBILE_CAP_DELETE_SOURCES: &[LegacyMobileBootstrapCapsSourcePinV1] = &[
    handler!("mobile handler:deleteCap"),
    declaration!("deleteCap"),
    AUTH_BACKEND_SOURCE,
    AUTH_DOMAIN_SOURCE,
    SCHEMA_SOURCE,
    client!("deleteCap released caller"),
    VIDEOS_SOURCE,
    VIDEO_REPO_SOURCE,
    VIDEO_DOMAIN_SOURCE,
    STORAGE_SOURCE,
    S3_SOURCE,
];
pub const LEGACY_MOBILE_CAP_GET_SOURCES: &[LegacyMobileBootstrapCapsSourcePinV1] = &[
    handler!("mobile handler:getCap"),
    declaration!("getCap"),
    AUTH_BACKEND_SOURCE,
    AUTH_DOMAIN_SOURCE,
    SCHEMA_SOURCE,
    client!("getCap released caller"),
    VIDEOS_SOURCE,
    VIDEO_DOMAIN_SOURCE,
    STORAGE_SOURCE,
    S3_SOURCE,
    IMAGE_UPLOADS_SOURCE,
    IMAGE_DOMAIN_SOURCE,
];
pub const LEGACY_MOBILE_CAP_DOWNLOAD_SOURCES: &[LegacyMobileBootstrapCapsSourcePinV1] = &[
    handler!("mobile handler:getDownload"),
    declaration!("getDownload"),
    AUTH_BACKEND_SOURCE,
    AUTH_DOMAIN_SOURCE,
    SCHEMA_SOURCE,
    client!("getDownload released caller"),
    VIDEOS_SOURCE,
    VIDEO_REPO_SOURCE,
    VIDEO_DOMAIN_SOURCE,
    STORAGE_SOURCE,
    S3_SOURCE,
];
pub const LEGACY_MOBILE_CAP_PLAYBACK_SOURCES: &[LegacyMobileBootstrapCapsSourcePinV1] = &[
    handler!("mobile handler:getPlayback"),
    declaration!("getPlayback"),
    AUTH_BACKEND_SOURCE,
    AUTH_DOMAIN_SOURCE,
    SCHEMA_SOURCE,
    client!("getPlayback released caller"),
    VIDEOS_SOURCE,
    VIDEO_REPO_SOURCE,
    VIDEO_DOMAIN_SOURCE,
    STORAGE_SOURCE,
    S3_SOURCE,
];

// Generated from canonical JSON of each sorted source array by the parity checker.
pub const LEGACY_MOBILE_BOOTSTRAP_SOURCE_MANIFEST_SHA256: &str =
    "7cd6cb37f1e7fc7f903e5b3dc1454512d3ed7981395e6dcd6ad1d6abd83f1304";
pub const LEGACY_MOBILE_CAPS_LIST_SOURCE_MANIFEST_SHA256: &str =
    "cec1ea58faee8e794a05497980a205d171b3cd27ebf6bd2179046fa99a4d9385";
pub const LEGACY_MOBILE_CAP_DELETE_SOURCE_MANIFEST_SHA256: &str =
    "9424a3a90d6bffcdb63224dcc16a8c64dee9e24592f6fd21f16fd40ee404b8b0";
pub const LEGACY_MOBILE_CAP_GET_SOURCE_MANIFEST_SHA256: &str =
    "3a73b50674e15aad9cdb6e56132b49d20496c3dde788c417b5a6c0273e33b3ff";
pub const LEGACY_MOBILE_CAP_DOWNLOAD_SOURCE_MANIFEST_SHA256: &str =
    "7ba403bb34ea76809133854c15517897d517936197587bcf2944df44c11606a5";
pub const LEGACY_MOBILE_CAP_PLAYBACK_SOURCE_MANIFEST_SHA256: &str =
    "ec1b3f4874f4306a6a7cb824038d4a6815f5bd813af70ee190fbe7f5ea871d8d";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileBootstrapCapsOperationV1 {
    Bootstrap,
    List,
    Delete,
    Get,
    Download,
    Playback,
}

impl LegacyMobileBootstrapCapsOperationV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::Bootstrap => LEGACY_MOBILE_BOOTSTRAP_OPERATION_ID,
            Self::List => LEGACY_MOBILE_CAPS_LIST_OPERATION_ID,
            Self::Delete => LEGACY_MOBILE_CAP_DELETE_OPERATION_ID,
            Self::Get => LEGACY_MOBILE_CAP_GET_OPERATION_ID,
            Self::Download => LEGACY_MOBILE_CAP_DOWNLOAD_OPERATION_ID,
            Self::Playback => LEGACY_MOBILE_CAP_PLAYBACK_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn method(self) -> &'static str {
        if matches!(self, Self::Delete) {
            "DELETE"
        } else {
            "GET"
        }
    }

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Bootstrap => LEGACY_MOBILE_BOOTSTRAP_PATH,
            Self::List => LEGACY_MOBILE_CAPS_PATH,
            Self::Delete | Self::Get => LEGACY_MOBILE_CAP_PATH,
            Self::Download => LEGACY_MOBILE_CAP_DOWNLOAD_PATH,
            Self::Playback => LEGACY_MOBILE_CAP_PLAYBACK_PATH,
        }
    }

    #[must_use]
    pub const fn sources(self) -> &'static [LegacyMobileBootstrapCapsSourcePinV1] {
        match self {
            Self::Bootstrap => LEGACY_MOBILE_BOOTSTRAP_SOURCES,
            Self::List => LEGACY_MOBILE_CAPS_LIST_SOURCES,
            Self::Delete => LEGACY_MOBILE_CAP_DELETE_SOURCES,
            Self::Get => LEGACY_MOBILE_CAP_GET_SOURCES,
            Self::Download => LEGACY_MOBILE_CAP_DOWNLOAD_SOURCES,
            Self::Playback => LEGACY_MOBILE_CAP_PLAYBACK_SOURCES,
        }
    }
}

/// Mirrors `Number`, finite/positive validation, `Math.trunc`, and the Cap cap.
#[must_use]
pub fn legacy_mobile_positive_integer(value: Option<&str>, fallback: u32, maximum: u32) -> u32 {
    let Some(value) = value else { return fallback };
    let value = crate::legacy_mobile_trim(value);
    let parsed = js_number(value);
    if !parsed.is_finite() || parsed < 1.0 {
        return fallback;
    }
    parsed.trunc().min(f64::from(maximum)) as u32
}

fn js_number(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).map_or(f64::NAN, |value| value as f64);
    }
    if let Some(binary) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        return u64::from_str_radix(binary, 2).map_or(f64::NAN, |value| value as f64);
    }
    if let Some(octal) = value
        .strip_prefix("0o")
        .or_else(|| value.strip_prefix("0O"))
    {
        return u64::from_str_radix(octal, 8).map_or(f64::NAN, |value| value as f64);
    }
    match value {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => value.parse::<f64>().unwrap_or(f64::NAN),
    }
}

/// JavaScript `Date#toISOString` for the non-negative D1 timestamp domain.
#[must_use]
pub fn legacy_mobile_iso_from_millis(value: i64) -> Option<String> {
    const DAY_MS: i64 = 86_400_000;
    if !(0..=253_402_300_799_999).contains(&value) {
        return None;
    }
    let days = value / DAY_MS;
    let day_ms = value % DAY_MS;
    let (year, month, day) = civil_from_days(days);
    let hour = day_ms / 3_600_000;
    let minute = day_ms % 3_600_000 / 60_000;
    let second = day_ms % 60_000 / 1_000;
    let millis = day_ms % 1_000;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyMobileChapterV1 {
    pub title: String,
    pub start: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyMobileMetadataProjectionV1 {
    pub summary: Option<String>,
    pub chapters: Vec<LegacyMobileChapterV1>,
}

#[must_use]
pub fn legacy_mobile_metadata_projection(
    metadata: Option<&Value>,
) -> LegacyMobileMetadataProjectionV1 {
    let object = metadata.and_then(Value::as_object);
    let summary = object
        .and_then(|value| value.get("summary"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let chapters = object
        .and_then(|value| value.get("chapters"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|chapter| {
            let chapter = chapter.as_object()?;
            Some(LegacyMobileChapterV1 {
                title: chapter.get("title")?.as_str()?.to_owned(),
                start: chapter.get("start")?.as_f64()?,
            })
        })
        .collect();
    LegacyMobileMetadataProjectionV1 { summary, chapters }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyMobileVideoSourceV1 {
    #[serde(rename = "MediaConvert")]
    MediaConvert,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "desktopMP4")]
    DesktopMp4,
    #[serde(rename = "desktopSegments")]
    DesktopSegments,
    #[serde(rename = "webMP4")]
    WebMp4,
}

impl LegacyMobileVideoSourceV1 {
    #[must_use]
    pub const fn parse(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"MediaConvert" => Some(Self::MediaConvert),
            b"local" => Some(Self::Local),
            b"desktopMP4" => Some(Self::DesktopMp4),
            b"desktopSegments" => Some(Self::DesktopSegments),
            b"webMP4" => Some(Self::WebMp4),
            _ => None,
        }
    }

    #[must_use]
    pub fn playback_object_key(self, prefix: &str) -> Option<String> {
        let suffix = match self {
            Self::MediaConvert => "output/video_recording_000.m3u8",
            Self::Local => "combined-source/stream.m3u8",
            Self::DesktopMp4 | Self::WebMp4 => "result.mp4",
            Self::DesktopSegments => return None,
        };
        Some(format!("{prefix}{suffix}"))
    }

    #[must_use]
    pub const fn playback_kind(self) -> &'static str {
        match self {
            Self::DesktopMp4 | Self::WebMp4 => "mp4",
            Self::MediaConvert | Self::Local | Self::DesktopSegments => "hls",
        }
    }

    #[must_use]
    pub const fn download_is_mp4(self) -> bool {
        matches!(self, Self::DesktopMp4 | Self::WebMp4)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMobileStorageObjectV1 {
    pub key: String,
    pub last_modified_ms: Option<i64>,
}

const SCREENSHOT_SUFFIXES: [&str; 5] = [
    "screenshot/screen-capture.png",
    "screenshot/screen-capture.jpg",
    "screenshot/screen-capture.jpeg",
    "screen-capture.jpg",
    "screen-capture.jpeg",
];

/// Reproduces Cap's newest-first screenshot candidate selection and suffix tie-break.
#[must_use]
pub fn legacy_mobile_screenshot_key(objects: &[LegacyMobileStorageObjectV1]) -> Option<String> {
    let mut candidates = objects
        .iter()
        .filter_map(|object| {
            let suffix = SCREENSHOT_SUFFIXES
                .iter()
                .position(|suffix| object.key.ends_with(suffix))?;
            Some((object, suffix))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(left, left_suffix), (right, right_suffix)| {
        let time = match (left.last_modified_ms, right.last_modified_ms) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(left), Some(right)) => right.cmp(&left),
        };
        time.then_with(|| left_suffix.cmp(right_suffix))
    });
    candidates.first().map(|(object, _)| object.key.clone())
}

#[must_use]
pub fn legacy_mobile_file_extension(key: &str) -> Option<String> {
    let file_name = key.rsplit('/').next().unwrap_or_default();
    let extension = file_name.rsplit('.').next()?.to_ascii_lowercase();
    (!extension.is_empty() && extension != file_name.to_ascii_lowercase()).then_some(extension)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyMobileImageLocationV1 {
    ExternalUrl(String),
    PrivateObjectKey(String),
}

/// Mirrors `ImageUpload.extractFileKey` for Frame's path-style R2 deployment.
#[must_use]
pub fn legacy_mobile_image_location(value: &str) -> LegacyMobileImageLocationV1 {
    match Url::parse(value) {
        Ok(url) if url.origin().ascii_serialization() == "https://lh3.googleusercontent.com" => {
            LegacyMobileImageLocationV1::ExternalUrl(value.to_owned())
        }
        Ok(url) => {
            let path = url.path().trim_start_matches('/');
            let key = path.split_once('/').map_or("", |(_, key)| key);
            if key.trim().is_empty() {
                LegacyMobileImageLocationV1::ExternalUrl(value.to_owned())
            } else {
                LegacyMobileImageLocationV1::PrivateObjectKey(key.to_owned())
            }
        }
        Err(_) => LegacyMobileImageLocationV1::PrivateObjectKey(value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pagination_matches_javascript_number_and_truncation() {
        assert_eq!(legacy_mobile_positive_integer(None, 20, 50), 20);
        assert_eq!(legacy_mobile_positive_integer(Some(""), 20, 50), 20);
        assert_eq!(legacy_mobile_positive_integer(Some(" 2.9 "), 20, 50), 2);
        assert_eq!(legacy_mobile_positive_integer(Some("0x10"), 20, 50), 16);
        assert_eq!(legacy_mobile_positive_integer(Some("1e3"), 20, 50), 50);
        assert_eq!(legacy_mobile_positive_integer(Some("Infinity"), 20, 50), 20);
        assert_eq!(
            legacy_mobile_iso_from_millis(1_735_787_045_006).as_deref(),
            Some("2025-01-02T03:04:05.006Z")
        );
    }

    #[test]
    fn metadata_filters_like_record_and_flat_map_helpers() {
        let value = json!({
            "summary": " ",
            "chapters": [
                {"title":"Intro","start":0},
                {"title":1,"start":2},
                null,
                {"title":"End","start":2.5,"ignored":true}
            ]
        });
        let projection = legacy_mobile_metadata_projection(Some(&value));
        assert_eq!(projection.summary.as_deref(), Some(" "));
        assert_eq!(projection.chapters.len(), 2);
        assert_eq!(projection.chapters[1].start, 2.5);
        assert!(
            legacy_mobile_metadata_projection(Some(&json!([])))
                .chapters
                .is_empty()
        );
    }

    #[test]
    fn source_keys_and_download_eligibility_are_exact() {
        assert_eq!(
            LegacyMobileVideoSourceV1::MediaConvert.playback_object_key("owner/video/"),
            Some("owner/video/output/video_recording_000.m3u8".into())
        );
        assert_eq!(
            LegacyMobileVideoSourceV1::DesktopSegments.playback_object_key("owner/video/"),
            None
        );
        assert!(LegacyMobileVideoSourceV1::WebMp4.download_is_mp4());
        assert!(!LegacyMobileVideoSourceV1::Local.download_is_mp4());
    }

    #[test]
    fn screenshot_selection_prefers_newest_then_source_suffix_order() {
        let objects = vec![
            LegacyMobileStorageObjectV1 {
                key: "o/v/screen-capture.jpeg".into(),
                last_modified_ms: Some(20),
            },
            LegacyMobileStorageObjectV1 {
                key: "o/v/screenshot/screen-capture.png".into(),
                last_modified_ms: Some(20),
            },
            LegacyMobileStorageObjectV1 {
                key: "o/v/screenshot/screen-capture.jpg".into(),
                last_modified_ms: Some(10),
            },
        ];
        assert_eq!(
            legacy_mobile_screenshot_key(&objects).as_deref(),
            Some("o/v/screenshot/screen-capture.png")
        );
    }

    #[test]
    fn image_location_preserves_google_and_extracts_other_paths() {
        assert!(matches!(
            legacy_mobile_image_location("https://lh3.googleusercontent.com/avatar"),
            LegacyMobileImageLocationV1::ExternalUrl(_)
        ));
        assert_eq!(
            legacy_mobile_image_location("https://r2.example/frame-recordings/a/b.png"),
            LegacyMobileImageLocationV1::PrivateObjectKey("a/b.png".into())
        );
        assert!(matches!(
            legacy_mobile_image_location("https://bucket.example/key-only.png"),
            LegacyMobileImageLocationV1::ExternalUrl(_)
        ));
        assert_eq!(
            legacy_mobile_image_location("owner/avatar.png"),
            LegacyMobileImageLocationV1::PrivateObjectKey("owner/avatar.png".into())
        );
        assert_eq!(
            legacy_mobile_file_extension("a/b.CAP.MP4").as_deref(),
            Some("mp4")
        );
    }
}
