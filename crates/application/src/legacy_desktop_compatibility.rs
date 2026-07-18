//! Source-pinned semantics for Cap's retained desktop compatibility routes.
//!
//! The released desktop sends a 36-character bearer API key or a browser
//! session token and does not send an idempotency header. Mutations therefore
//! accept an optional key: a supplied key is durable and replayable, while an
//! absent key receives a one-execution server key so source retry behaviour is
//! not silently changed. Organization, owner, and video identifiers remain
//! source NanoIDs at the boundary and are resolved by the D1 port.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{IdempotencyKey, OrganizationOperationId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_DESKTOP_COMPATIBILITY_CAP_COMMIT: &str =
    "6ba69561ac86b8efdb17616d6727f9638015546b";

pub const LEGACY_DESKTOP_ORGANIZATIONS_OPERATION_ID: &str = "cap-v1-ab49cf36a3f243ac";
pub const LEGACY_DESKTOP_ORGANIZATION_BRANDING_OPERATION_ID: &str = "cap-v1-cdfdf7db0f5cb243";
pub const LEGACY_DESKTOP_STORAGE_SET_ACTIVE_OPERATION_ID: &str = "cap-v1-a77171e54b2ba955";
pub const LEGACY_DESKTOP_USER_PROFILE_OPERATION_ID: &str = "cap-v1-7508c5a7da637a0b";
pub const LEGACY_DESKTOP_VIDEO_DELETE_OPERATION_ID: &str = "cap-v1-acc98d2d5e8ff345";
pub const LEGACY_DESKTOP_VIDEO_PROGRESS_OPERATION_ID: &str = "cap-v1-117b0cb801816693";

pub const LEGACY_DESKTOP_ORGANIZATIONS_PATH: &str = "/api/desktop/organizations";
pub const LEGACY_DESKTOP_ORGANIZATION_BRANDING_PATH: &str =
    "/api/desktop/organizations/:organizationId/branding";
pub const LEGACY_DESKTOP_STORAGE_SET_ACTIVE_PATH: &str = "/api/desktop/storage/set-active";
pub const LEGACY_DESKTOP_USER_PROFILE_PATH: &str = "/api/desktop/user/profile";
pub const LEGACY_DESKTOP_VIDEO_DELETE_PATH: &str = "/api/desktop/video/delete";
pub const LEGACY_DESKTOP_VIDEO_PROGRESS_PATH: &str = "/api/desktop/video/progress";

pub const LEGACY_DESKTOP_JSON_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_DESKTOP_MUTATION_MAX_BODY_BYTES: usize = 256 * 1_024;
// A one-MiB binary logo expands to 1,398,104 base64 bytes before the JSON
// envelope. Keep a narrow deterministic ceiling above the source maximum.
pub const LEGACY_DESKTOP_BRANDING_MAX_BODY_BYTES: usize = 1_500_000;
pub const LEGACY_DESKTOP_LOGO_MAX_BYTES: usize = 1_024 * 1_024;
pub const LEGACY_DESKTOP_NO_PROTECTED_GATES: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDesktopSourceRoleV1 {
    Handler,
    Mount,
    Authentication,
    Contract,
    Schema,
    Service,
    Client,
    ClientTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDesktopSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyDesktopSourceRoleV1,
}

const DESKTOP_MOUNT: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/web/app/api/desktop/[...route]/route.ts",
    symbol: "desktop basePath mount+methods+CORS",
    sha256: "34854ff6fc0839838165990bea1c9ebee86770b1648ec832bbbb786720c9db41",
    role: LegacyDesktopSourceRoleV1::Mount,
};
const DESKTOP_AUTH_CORS: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/web/app/api/utils.ts",
    symbol: "getAuth+withAuth+corsMiddleware",
    sha256: "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
    role: LegacyDesktopSourceRoleV1::Authentication,
};
const DATABASE_SESSION: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "packages/database/auth/session.ts",
    symbol: "getCurrentUser",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyDesktopSourceRoleV1::Authentication,
};
const DATABASE_SCHEMA: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users+organizations+organizationMembers+storageIntegrations+videos+videoUploads+importedVideos",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyDesktopSourceRoleV1::Schema,
};
const DESKTOP_CONTRACT: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "packages/web-api-contract/src/desktop.ts",
    symbol: "desktop protected contract",
    sha256: "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    role: LegacyDesktopSourceRoleV1::Contract,
};
const DESKTOP_WEB_CLIENT: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/desktop/src/utils/web-api.ts",
    symbol: "apiClient+protectedHeaders+server routing",
    sha256: "d3655b985a21a54d97b9974b17536aebab490929850baffaa5186d7a5632b45a",
    role: LegacyDesktopSourceRoleV1::ClientTransport,
};
const DESKTOP_RUST_CLIENT: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/desktop/src-tauri/src/web_api.rs",
    symbol: "authed_api_request+Authorization bearer",
    sha256: "33abf0a3ffbe2912d2fcf251eb4892713ed85b48ccf66822d916b43ce935316c",
    role: LegacyDesktopSourceRoleV1::ClientTransport,
};
const ROOT_HANDLER: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/web/app/api/desktop/[...route]/root.ts",
    symbol: "desktop root handlers",
    sha256: "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
    role: LegacyDesktopSourceRoleV1::Handler,
};
const ORGANIZATION_BRANDING: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/web/app/api/desktop/[...route]/organization-branding.ts",
    symbol: "organization branding normalization+logo validation+projection",
    sha256: "6383ce7f6d8bc2600c19947e693ab8b7f74ab36baf220a7967f3f9127009f002",
    role: LegacyDesktopSourceRoleV1::Service,
};
const ORGANIZATION_ROLES: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/web/lib/permissions/roles.ts",
    symbol: "normalizeOrganizationRole+getEffectiveOrganizationRole",
    sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    role: LegacyDesktopSourceRoleV1::Service,
};

pub const LEGACY_DESKTOP_ORGANIZATIONS_SOURCES: &[LegacyDesktopSourcePinV1] = &[
    LegacyDesktopSourcePinV1 {
        symbol: "GET /organizations",
        ..ROOT_HANDLER
    },
    ORGANIZATION_BRANDING,
    ORGANIZATION_ROLES,
    LegacyDesktopSourcePinV1 {
        symbol: "GET /desktop/organizations",
        ..DESKTOP_CONTRACT
    },
    DESKTOP_MOUNT,
    DESKTOP_AUTH_CORS,
    DATABASE_SESSION,
    DATABASE_SCHEMA,
    LegacyDesktopSourcePinV1 {
        path: "apps/desktop/src-tauri/src/api.rs",
        symbol: "fetch_organizations+Organization",
        sha256: "d029c4cc7eba0be97f03bba8da2f3ab02277ce65161de69aca4ad77b3474a48e",
        role: LegacyDesktopSourceRoleV1::Client,
    },
    DESKTOP_RUST_CLIENT,
];

pub const LEGACY_DESKTOP_ORGANIZATION_BRANDING_SOURCES: &[LegacyDesktopSourcePinV1] = &[
    LegacyDesktopSourcePinV1 {
        symbol: "PATCH /organizations/:organizationId/branding",
        ..ROOT_HANDLER
    },
    ORGANIZATION_BRANDING,
    ORGANIZATION_ROLES,
    LegacyDesktopSourcePinV1 {
        path: "packages/web-backend/src/ImageUploads/index.ts",
        symbol: "ImageUploads.applyUpdate+resolveImageUrl",
        sha256: "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
        role: LegacyDesktopSourceRoleV1::Service,
    },
    LegacyDesktopSourcePinV1 {
        path: "packages/web-domain/src/ImageUpload.ts",
        symbol: "ImageUpdatePayload+extractFileKey",
        sha256: "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
        role: LegacyDesktopSourceRoleV1::Contract,
    },
    LegacyDesktopSourcePinV1 {
        symbol: "PATCH /desktop/organizations/:organizationId/branding",
        ..DESKTOP_CONTRACT
    },
    DESKTOP_MOUNT,
    DESKTOP_AUTH_CORS,
    DATABASE_SESSION,
    DATABASE_SCHEMA,
    LegacyDesktopSourcePinV1 {
        path: "apps/desktop/src/utils/organization-branding.ts",
        symbol: "updateOrganizationBranding+encodeFileAsBase64",
        sha256: "659410130f8ff54a4fa7adb3e1d60e8515fc78b44b118bfa74f07537445ab07e",
        role: LegacyDesktopSourceRoleV1::Client,
    },
    DESKTOP_WEB_CLIENT,
];

pub const LEGACY_DESKTOP_STORAGE_SET_ACTIVE_SOURCES: &[LegacyDesktopSourcePinV1] = &[
    LegacyDesktopSourcePinV1 {
        path: "apps/web/app/api/desktop/[...route]/storage.ts",
        symbol: "POST /set-active",
        sha256: "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886",
        role: LegacyDesktopSourceRoleV1::Handler,
    },
    LegacyDesktopSourcePinV1 {
        symbol: "POST /desktop/storage/set-active",
        ..DESKTOP_CONTRACT
    },
    DESKTOP_MOUNT,
    DESKTOP_AUTH_CORS,
    DATABASE_SESSION,
    DATABASE_SCHEMA,
    LegacyDesktopSourcePinV1 {
        path: "apps/desktop/src/routes/(window-chrome)/settings/integrations/google-drive-config.tsx",
        symbol: "setActive storage mutation",
        sha256: "84b7e589f65ff6a121ad1fca963134526c75d21aea5f978c8cd50ce23b935c33",
        role: LegacyDesktopSourceRoleV1::Client,
    },
    DESKTOP_WEB_CLIENT,
];

pub const LEGACY_DESKTOP_USER_PROFILE_SOURCES: &[LegacyDesktopSourcePinV1] = &[
    LegacyDesktopSourcePinV1 {
        symbol: "GET /user/profile",
        ..ROOT_HANDLER
    },
    LegacyDesktopSourcePinV1 {
        path: "packages/web-api-contract-effect/src/index.ts",
        symbol: "getUserProfile",
        sha256: "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
        role: LegacyDesktopSourceRoleV1::Contract,
    },
    LegacyDesktopSourcePinV1 {
        symbol: "GET /desktop/user/profile",
        ..DESKTOP_CONTRACT
    },
    DESKTOP_MOUNT,
    DESKTOP_AUTH_CORS,
    DATABASE_SESSION,
    DATABASE_SCHEMA,
    LegacyDesktopSourcePinV1 {
        path: "apps/desktop/src/routes/(window-chrome)/settings.tsx",
        symbol: "getUserProfile query",
        sha256: "4ee20069fdd0ef077e5e89e5c7bcb8f353b30302c7ab40725c44dc28db7880ae",
        role: LegacyDesktopSourceRoleV1::Client,
    },
    DESKTOP_WEB_CLIENT,
];

const VIDEO_HANDLER: LegacyDesktopSourcePinV1 = LegacyDesktopSourcePinV1 {
    path: "apps/web/app/api/desktop/[...route]/video.ts",
    symbol: "desktop video handlers",
    sha256: "03e50223fb6968dafdbaa8a8c8cb537c46be27a0c88b9c92e004afa95f7c013d",
    role: LegacyDesktopSourceRoleV1::Handler,
};

pub const LEGACY_DESKTOP_VIDEO_DELETE_SOURCES: &[LegacyDesktopSourcePinV1] = &[
    LegacyDesktopSourcePinV1 {
        symbol: "DELETE /delete",
        ..VIDEO_HANDLER
    },
    LegacyDesktopSourcePinV1 {
        path: "packages/web-backend/src/Storage/index.ts",
        symbol: "Storage.getAccessForVideo",
        sha256: "3ea22f76907104e26df8f48bdcac87a5dc2d3d60497dfc409110eb0fa8446b4c",
        role: LegacyDesktopSourceRoleV1::Service,
    },
    LegacyDesktopSourcePinV1 {
        symbol: "DELETE /desktop/video/delete",
        ..DESKTOP_CONTRACT
    },
    DESKTOP_MOUNT,
    DESKTOP_AUTH_CORS,
    DATABASE_SESSION,
    DATABASE_SCHEMA,
    LegacyDesktopSourcePinV1 {
        path: "apps/desktop/src-tauri/src/recording.rs",
        symbol: "delete_remote_instant_video",
        sha256: "15e3dd2b7278e829d9f4dd0b0e5b49bc6b6cc5e21a46a1a36466ad3da319f313",
        role: LegacyDesktopSourceRoleV1::Client,
    },
    DESKTOP_RUST_CLIENT,
];

pub const LEGACY_DESKTOP_VIDEO_PROGRESS_SOURCES: &[LegacyDesktopSourcePinV1] = &[
    LegacyDesktopSourcePinV1 {
        symbol: "POST /progress",
        ..VIDEO_HANDLER
    },
    DESKTOP_MOUNT,
    DESKTOP_AUTH_CORS,
    DATABASE_SESSION,
    DATABASE_SCHEMA,
    LegacyDesktopSourcePinV1 {
        path: "apps/desktop/src-tauri/src/api.rs",
        symbol: "desktop_video_progress",
        sha256: "d029c4cc7eba0be97f03bba8da2f3ab02277ce65161de69aca4ad77b3474a48e",
        role: LegacyDesktopSourceRoleV1::Client,
    },
    DESKTOP_RUST_CLIENT,
];

// Canonical SHA-256 values of each operation's ordered source-pin JSON. These
// are generated and verified by the parity checker.
pub const LEGACY_DESKTOP_ORGANIZATIONS_SOURCE_MANIFEST_SHA256: &str =
    "57c38458f638ec3715c6ea08845e9e7ed34fa594244deca966d71734ce83d8e3";
pub const LEGACY_DESKTOP_ORGANIZATION_BRANDING_SOURCE_MANIFEST_SHA256: &str =
    "644d6f74e135bcab7978f4921563b9454b1213345acce07049f360aec3333492";
pub const LEGACY_DESKTOP_STORAGE_SET_ACTIVE_SOURCE_MANIFEST_SHA256: &str =
    "12fd70ac75df2f65fa8daaeaa1f27b0e622e973745b02b199531251dd0bf5ae8";
pub const LEGACY_DESKTOP_USER_PROFILE_SOURCE_MANIFEST_SHA256: &str =
    "d3b9e425a3c603aa946511e3418ff607fc0654fe0bdbfa08a8a1942713437cdd";
pub const LEGACY_DESKTOP_VIDEO_DELETE_SOURCE_MANIFEST_SHA256: &str =
    "d79e69c91b9fddae0fb55620cde75649e2ab85ad18d1cc16756f5a241d071822";
pub const LEGACY_DESKTOP_VIDEO_PROGRESS_SOURCE_MANIFEST_SHA256: &str =
    "80fab5666369f7d185f9c4aec1c7a16973a33e9948005078ca1e6b6b2018a135";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDesktopCompatibilitySurfaceV1 {
    Organizations,
    OrganizationBranding,
    StorageSetActive,
    UserProfile,
    VideoDelete,
    VideoProgress,
}

impl LegacyDesktopCompatibilitySurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::Organizations => LEGACY_DESKTOP_ORGANIZATIONS_OPERATION_ID,
            Self::OrganizationBranding => LEGACY_DESKTOP_ORGANIZATION_BRANDING_OPERATION_ID,
            Self::StorageSetActive => LEGACY_DESKTOP_STORAGE_SET_ACTIVE_OPERATION_ID,
            Self::UserProfile => LEGACY_DESKTOP_USER_PROFILE_OPERATION_ID,
            Self::VideoDelete => LEGACY_DESKTOP_VIDEO_DELETE_OPERATION_ID,
            Self::VideoProgress => LEGACY_DESKTOP_VIDEO_PROGRESS_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::Organizations => LEGACY_DESKTOP_ORGANIZATIONS_PATH,
            Self::OrganizationBranding => LEGACY_DESKTOP_ORGANIZATION_BRANDING_PATH,
            Self::StorageSetActive => LEGACY_DESKTOP_STORAGE_SET_ACTIVE_PATH,
            Self::UserProfile => LEGACY_DESKTOP_USER_PROFILE_PATH,
            Self::VideoDelete => LEGACY_DESKTOP_VIDEO_DELETE_PATH,
            Self::VideoProgress => LEGACY_DESKTOP_VIDEO_PROGRESS_PATH,
        }
    }

    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::Organizations | Self::UserProfile => "GET",
            Self::OrganizationBranding => "PATCH",
            Self::StorageSetActive | Self::VideoProgress => "POST",
            Self::VideoDelete => "DELETE",
        }
    }

    #[must_use]
    pub const fn rate_limit_bucket(self) -> &'static str {
        match self {
            Self::Organizations | Self::OrganizationBranding => "organization_library.v1",
            Self::StorageSetActive => "upload_storage.v1",
            Self::UserProfile => "client_compatibility.v1",
            Self::VideoDelete | Self::VideoProgress => "video_media.v1",
        }
    }

    #[must_use]
    pub const fn mutating(self) -> bool {
        !matches!(self, Self::Organizations | Self::UserProfile)
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Organizations => "organizations",
            Self::OrganizationBranding => "organization_branding",
            Self::StorageSetActive => "storage_set_active",
            Self::UserProfile => "user_profile",
            Self::VideoDelete => "video_delete",
            Self::VideoProgress => "video_progress",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDesktopCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegacyDesktopOrganizationRoleV1 {
    Owner,
    Admin,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LegacyDesktopBrandColorsV1 {
    pub primary: Option<String>,
    pub secondary: Option<String>,
    pub accent: Option<String>,
    pub background: Option<String>,
}

impl LegacyDesktopBrandColorsV1 {
    pub fn normalize(self) -> Result<Self, LegacyDesktopCompatibilityErrorV1> {
        Ok(Self {
            primary: normalize_color(self.primary)?,
            secondary: normalize_color(self.secondary)?,
            accent: normalize_color(self.accent)?,
            background: normalize_color(self.background)?,
        })
    }
}

fn normalize_color(
    value: Option<String>,
) -> Result<Option<String>, LegacyDesktopCompatibilityErrorV1> {
    let Some(value) = value else { return Ok(None) };
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    Ok(Some(value.to_ascii_uppercase()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyDesktopLogoContentTypeV1 {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    Webp,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/avif")]
    Avif,
}

impl LegacyDesktopLogoContentTypeV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Avif => "image/avif",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum LegacyDesktopLogoWireV1 {
    #[default]
    Keep,
    Remove,
    Upload {
        #[serde(rename = "contentType")]
        content_type: LegacyDesktopLogoContentTypeV1,
        data: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyDesktopLogoUpdateV1 {
    Keep,
    Remove,
    Upload {
        content_type: LegacyDesktopLogoContentTypeV1,
        data: Vec<u8>,
        data_url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LegacyDesktopBrandingPatchWireV1 {
    pub brand_colors: LegacyDesktopBrandColorsV1,
    #[serde(default)]
    pub logo: LegacyDesktopLogoWireV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyDesktopBrandingPatchV1 {
    pub brand_colors: LegacyDesktopBrandColorsV1,
    pub logo: LegacyDesktopLogoUpdateV1,
}

impl LegacyDesktopBrandingPatchWireV1 {
    pub fn normalize(
        self,
    ) -> Result<LegacyDesktopBrandingPatchV1, LegacyDesktopCompatibilityErrorV1> {
        let brand_colors = self.brand_colors.normalize()?;
        let logo = match self.logo {
            LegacyDesktopLogoWireV1::Keep => LegacyDesktopLogoUpdateV1::Keep,
            LegacyDesktopLogoWireV1::Remove => LegacyDesktopLogoUpdateV1::Remove,
            LegacyDesktopLogoWireV1::Upload { content_type, data } => {
                let decoded = decode_base64_logo(&data)?;
                if decoded.is_empty() {
                    return Err(LegacyDesktopCompatibilityErrorV1::LogoEmpty);
                }
                if decoded.len() > LEGACY_DESKTOP_LOGO_MAX_BYTES {
                    return Err(LegacyDesktopCompatibilityErrorV1::LogoTooLarge);
                }
                if !valid_logo_magic(content_type, &decoded) {
                    return Err(LegacyDesktopCompatibilityErrorV1::LogoTypeInvalid);
                }
                let data_url = format!("data:{};base64,{data}", content_type.as_str());
                LegacyDesktopLogoUpdateV1::Upload {
                    content_type,
                    data: decoded,
                    data_url,
                }
            }
        };
        Ok(LegacyDesktopBrandingPatchV1 { brand_colors, logo })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyDesktopStorageProviderV1 {
    #[serde(rename = "s3")]
    S3,
    #[serde(rename = "googleDrive")]
    GoogleDrive,
}

impl LegacyDesktopStorageProviderV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::GoogleDrive => "googleDrive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LegacyDesktopVideoProgressWireV1 {
    pub video_id: String,
    pub uploaded: f64,
    pub total: f64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyDesktopVideoProgressV1 {
    pub legacy_video_id: String,
    pub uploaded: f64,
    pub total: f64,
    pub updated_at_ms: i64,
}

impl LegacyDesktopVideoProgressWireV1 {
    pub fn normalize(
        self,
    ) -> Result<LegacyDesktopVideoProgressV1, LegacyDesktopCompatibilityErrorV1> {
        if self.video_id.is_empty()
            || self.video_id.len() > 2_048
            || !self.uploaded.is_finite()
            || !self.total.is_finite()
        {
            return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
        }
        let updated_at_ms = parse_rfc3339_millis(&self.updated_at)
            .ok_or(LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
        Ok(LegacyDesktopVideoProgressV1 {
            legacy_video_id: self.video_id,
            uploaded: self.uploaded.min(self.total),
            total: self.total,
            updated_at_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyDesktopCompatibilityInputV1 {
    Organizations,
    OrganizationBranding {
        legacy_organization_id: String,
        patch: LegacyDesktopBrandingPatchV1,
    },
    StorageSetActive {
        provider: LegacyDesktopStorageProviderV1,
    },
    UserProfile,
    VideoDelete {
        legacy_video_id: String,
    },
    VideoProgress(LegacyDesktopVideoProgressV1),
}

impl LegacyDesktopCompatibilityInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyDesktopCompatibilitySurfaceV1 {
        match self {
            Self::Organizations => LegacyDesktopCompatibilitySurfaceV1::Organizations,
            Self::OrganizationBranding { .. } => {
                LegacyDesktopCompatibilitySurfaceV1::OrganizationBranding
            }
            Self::StorageSetActive { .. } => LegacyDesktopCompatibilitySurfaceV1::StorageSetActive,
            Self::UserProfile => LegacyDesktopCompatibilitySurfaceV1::UserProfile,
            Self::VideoDelete { .. } => LegacyDesktopCompatibilitySurfaceV1::VideoDelete,
            Self::VideoProgress(_) => LegacyDesktopCompatibilitySurfaceV1::VideoProgress,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyDesktopCompatibilityRequestV1 {
    pub actor_id: String,
    pub credential: LegacyDesktopCredentialV1,
    pub input: LegacyDesktopCompatibilityInputV1,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct LegacyDesktopCompatibilityCommandV1 {
    operation_id: Option<OrganizationOperationId>,
    surface: LegacyDesktopCompatibilitySurfaceV1,
    actor_id: String,
    credential: LegacyDesktopCredentialV1,
    input: LegacyDesktopCompatibilityInputV1,
    idempotency_key: Option<IdempotencyKey>,
    request_digest: String,
}

impl fmt::Debug for LegacyDesktopCompatibilityCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDesktopCompatibilityCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.surface)
            .field("actor_id", &self.actor_id)
            .field("credential", &self.credential)
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("request_digest", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl LegacyDesktopCompatibilityCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> Option<OrganizationOperationId> {
        self.operation_id
    }

    #[must_use]
    pub const fn surface(&self) -> LegacyDesktopCompatibilitySurfaceV1 {
        self.surface
    }

    #[must_use]
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    #[must_use]
    pub const fn credential(&self) -> LegacyDesktopCredentialV1 {
        self.credential
    }

    #[must_use]
    pub fn input(&self) -> &LegacyDesktopCompatibilityInputV1 {
        &self.input
    }

    #[must_use]
    pub fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    #[must_use]
    pub fn idempotency_key_digest_hex(&self) -> Option<String> {
        self.idempotency_key.as_ref().map(|key| {
            digest_fields(
                b"frame.legacy-desktop-compatibility.idempotency.v1\0",
                &[key.expose()],
            )
        })
    }

    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDesktopOrganizationV1 {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub role: LegacyDesktopOrganizationRoleV1,
    pub can_edit_brand: bool,
    pub icon_url: Option<String>,
    pub brand_colors: LegacyDesktopBrandColorsV1,
}

impl LegacyDesktopOrganizationV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && !self.owner_id.is_empty()
            && self.can_edit_brand
                == matches!(
                    self.role,
                    LegacyDesktopOrganizationRoleV1::Owner | LegacyDesktopOrganizationRoleV1::Admin
                )
            && self.clone().brand_colors.normalize().is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDesktopUserProfileV1 {
    pub name: Option<String>,
    pub email: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyDesktopCompatibilityResultV1 {
    Organizations(Vec<LegacyDesktopOrganizationV1>),
    Organization(LegacyDesktopOrganizationV1),
    StorageSuccess,
    UserProfile(LegacyDesktopUserProfileV1),
    JsonTrue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyDesktopCompatibilityOutcomeV1 {
    pub result: LegacyDesktopCompatibilityResultV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDesktopCompatibilityPortErrorV1 {
    NotFound,
    BrandingForbidden,
    StorageNotConnected,
    IdempotencyConflict,
    Unavailable,
    Provider,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyDesktopCompatibilityErrorV1 {
    #[error("invalid desktop compatibility input")]
    InvalidInput,
    #[error("desktop user is not authenticated")]
    Unauthorized,
    #[error("desktop target was not found")]
    NotFound,
    #[error("desktop branding requires an organization owner or administrator")]
    BrandingForbidden,
    #[error("Google Drive is not connected")]
    StorageNotConnected,
    #[error("Invalid logo data")]
    LogoDataInvalid,
    #[error("Logo file is empty")]
    LogoEmpty,
    #[error("Logo file must be less than 1MB")]
    LogoTooLarge,
    #[error("Logo file type is invalid")]
    LogoTypeInvalid,
    #[error("desktop idempotency key conflicts with a prior request")]
    Conflict,
    #[error("desktop compatibility service is unavailable")]
    Unavailable,
    #[error("desktop compatibility provider effect failed")]
    Provider,
    #[error("desktop compatibility state is corrupt")]
    Internal,
}

#[async_trait(?Send)]
pub trait LegacyDesktopCompatibilityPortV1 {
    async fn execute(
        &self,
        command: LegacyDesktopCompatibilityCommandV1,
    ) -> Result<LegacyDesktopCompatibilityOutcomeV1, LegacyDesktopCompatibilityPortErrorV1>;
}

pub struct LegacyDesktopCompatibilityAdapterV1<'port, Port> {
    port: &'port Port,
}

impl<'port, Port> LegacyDesktopCompatibilityAdapterV1<'port, Port>
where
    Port: LegacyDesktopCompatibilityPortV1,
{
    #[must_use]
    pub const fn new(port: &'port Port) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        request: LegacyDesktopCompatibilityRequestV1,
    ) -> Result<LegacyDesktopCompatibilityOutcomeV1, LegacyDesktopCompatibilityErrorV1> {
        let command = prepare(request)?;
        self.port.execute(command).await.map_err(map_port_error)
    }
}

fn prepare(
    request: LegacyDesktopCompatibilityRequestV1,
) -> Result<LegacyDesktopCompatibilityCommandV1, LegacyDesktopCompatibilityErrorV1> {
    if !valid_boundary_id(&request.actor_id) {
        return Err(LegacyDesktopCompatibilityErrorV1::Unauthorized);
    }
    let surface = request.input.surface();
    validate_input(&request.input)?;
    if !surface.mutating() && request.idempotency_key.is_some() {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    let operation_id = surface.mutating().then(OrganizationOperationId::new);
    let idempotency_key = if surface.mutating() {
        Some(
            request
                .idempotency_key
                .map_or_else(
                    || {
                        IdempotencyKey::parse(format!(
                            "desktop-auto:{}",
                            operation_id.expect("mutations have operation IDs")
                        ))
                    },
                    IdempotencyKey::parse,
                )
                .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?,
        )
    } else {
        None
    };
    let request_digest = request_fingerprint(surface, &request.actor_id, &request.input)?;
    Ok(LegacyDesktopCompatibilityCommandV1 {
        operation_id,
        surface,
        actor_id: request.actor_id,
        credential: request.credential,
        input: request.input,
        idempotency_key,
        request_digest,
    })
}

fn validate_input(
    input: &LegacyDesktopCompatibilityInputV1,
) -> Result<(), LegacyDesktopCompatibilityErrorV1> {
    let valid = match input {
        LegacyDesktopCompatibilityInputV1::Organizations
        | LegacyDesktopCompatibilityInputV1::UserProfile
        | LegacyDesktopCompatibilityInputV1::StorageSetActive { .. } => true,
        LegacyDesktopCompatibilityInputV1::OrganizationBranding {
            legacy_organization_id,
            patch,
        } => {
            valid_source_id(legacy_organization_id)
                && patch.clone().brand_colors.normalize().is_ok()
                && match &patch.logo {
                    LegacyDesktopLogoUpdateV1::Keep | LegacyDesktopLogoUpdateV1::Remove => true,
                    LegacyDesktopLogoUpdateV1::Upload {
                        content_type,
                        data,
                        data_url,
                    } => {
                        !data.is_empty()
                            && data.len() <= LEGACY_DESKTOP_LOGO_MAX_BYTES
                            && valid_logo_magic(*content_type, data)
                            && data_url
                                .starts_with(&format!("data:{};base64,", content_type.as_str()))
                    }
                }
        }
        LegacyDesktopCompatibilityInputV1::VideoDelete { legacy_video_id } => {
            valid_source_id(legacy_video_id)
        }
        LegacyDesktopCompatibilityInputV1::VideoProgress(progress) => {
            valid_source_id(&progress.legacy_video_id)
                && progress.uploaded.is_finite()
                && progress.total.is_finite()
                && (0..=9_007_199_254_740_991).contains(&progress.updated_at_ms)
        }
    };
    valid
        .then_some(())
        .ok_or(LegacyDesktopCompatibilityErrorV1::InvalidInput)
}

fn request_fingerprint(
    surface: LegacyDesktopCompatibilitySurfaceV1,
    actor_id: &str,
    input: &LegacyDesktopCompatibilityInputV1,
) -> Result<String, LegacyDesktopCompatibilityErrorV1> {
    let payload = match input {
        LegacyDesktopCompatibilityInputV1::Organizations
        | LegacyDesktopCompatibilityInputV1::UserProfile => Value::Null,
        LegacyDesktopCompatibilityInputV1::OrganizationBranding {
            legacy_organization_id,
            patch,
        } => serde_json::json!({
            "organizationId": legacy_organization_id,
            "brandColors": patch.brand_colors,
            "logo": match &patch.logo {
                LegacyDesktopLogoUpdateV1::Keep => serde_json::json!({"action":"keep"}),
                LegacyDesktopLogoUpdateV1::Remove => serde_json::json!({"action":"remove"}),
                LegacyDesktopLogoUpdateV1::Upload { content_type, data_url, .. } => serde_json::json!({
                    "action":"upload", "contentType":content_type.as_str(), "dataUrl":data_url
                }),
            },
        }),
        LegacyDesktopCompatibilityInputV1::StorageSetActive { provider } => {
            serde_json::json!({"provider":provider.as_str()})
        }
        LegacyDesktopCompatibilityInputV1::VideoDelete { legacy_video_id } => {
            serde_json::json!({"videoId":legacy_video_id})
        }
        LegacyDesktopCompatibilityInputV1::VideoProgress(progress) => serde_json::json!({
            "videoId":progress.legacy_video_id,
            "uploaded":progress.uploaded,
            "total":progress.total,
            "updatedAtMs":progress.updated_at_ms,
        }),
    };
    let encoded =
        serde_json::to_string(&payload).map_err(|_| LegacyDesktopCompatibilityErrorV1::Internal)?;
    Ok(digest_fields(
        b"frame.legacy-desktop-compatibility.request.v1\0",
        &[surface.operation_id(), actor_id, &encoded],
    ))
}

#[must_use]
pub fn organization_brand_colors_from_metadata(metadata: &Value) -> LegacyDesktopBrandColorsV1 {
    let color = |name: &str| {
        metadata
            .as_object()
            .and_then(|value| value.get("branding"))
            .and_then(Value::as_object)
            .and_then(|value| value.get("colors"))
            .and_then(Value::as_object)
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .and_then(|value| normalize_color(Some(value)).ok().flatten())
    };
    LegacyDesktopBrandColorsV1 {
        primary: color("primary"),
        secondary: color("secondary"),
        accent: color("accent"),
        background: color("background"),
    }
}

#[must_use]
pub fn merge_organization_branding_metadata(
    metadata: Value,
    colors: &LegacyDesktopBrandColorsV1,
) -> Value {
    let mut metadata = match metadata {
        Value::Object(value) => value,
        _ => Map::new(),
    };
    let mut branding = match metadata.remove("branding") {
        Some(Value::Object(value)) => value,
        _ => Map::new(),
    };
    branding.insert(
        "colors".into(),
        serde_json::to_value(colors).expect("brand colors always serialize"),
    );
    metadata.insert("branding".into(), Value::Object(branding));
    Value::Object(metadata)
}

#[must_use]
pub fn desktop_profile_name(first: Option<&str>, last: Option<&str>) -> Option<String> {
    let joined = [first, last]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = trim_ecmascript(&joined);
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

#[must_use]
pub fn mapped_legacy_video_uuid(value: &str) -> Option<String> {
    if uuid_shape(value) {
        return Some(value.to_ascii_lowercase());
    }
    if !valid_source_id(value) {
        return None;
    }
    let payload = Sha256::digest(
        [
            b"frame-cap-video-id-to-uuid-v1\0".as_slice(),
            value.as_bytes(),
        ]
        .concat(),
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&payload[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes(bytes[0..4].try_into().ok()?),
        u16::from_be_bytes(bytes[4..6].try_into().ok()?),
        u16::from_be_bytes(bytes[6..8].try_into().ok()?),
        u16::from_be_bytes(bytes[8..10].try_into().ok()?),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    ))
}

fn decode_base64_logo(value: &str) -> Result<Vec<u8>, LegacyDesktopCompatibilityErrorV1> {
    if value.is_empty()
        || !value.len().is_multiple_of(4)
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')))
        || value[..value.len().saturating_sub(2)].contains('=')
    {
        return Err(LegacyDesktopCompatibilityErrorV1::LogoDataInvalid);
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = index + 1 == value.len() / 4;
        let c_pad = chunk[2] == b'=';
        let d_pad = chunk[3] == b'=';
        if (c_pad && !d_pad) || (d_pad && !last) {
            return Err(LegacyDesktopCompatibilityErrorV1::LogoDataInvalid);
        }
        let a = base64_digit(chunk[0])?;
        let b = base64_digit(chunk[1])?;
        let c = if c_pad { 0 } else { base64_digit(chunk[2])? };
        let d = if d_pad { 0 } else { base64_digit(chunk[3])? };
        output.push((a << 2) | (b >> 4));
        if !c_pad {
            output.push((b << 4) | (c >> 2));
        }
        if !d_pad {
            output.push((c << 6) | d);
        }
    }
    Ok(output)
}

fn base64_digit(value: u8) -> Result<u8, LegacyDesktopCompatibilityErrorV1> {
    match value {
        b'A'..=b'Z' => Ok(value - b'A'),
        b'a'..=b'z' => Ok(value - b'a' + 26),
        b'0'..=b'9' => Ok(value - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(LegacyDesktopCompatibilityErrorV1::LogoDataInvalid),
    }
}

fn valid_logo_magic(content_type: LegacyDesktopLogoContentTypeV1, data: &[u8]) -> bool {
    match content_type {
        LegacyDesktopLogoContentTypeV1::Png => data.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]),
        LegacyDesktopLogoContentTypeV1::Jpeg => data.starts_with(&[255, 216, 255]),
        LegacyDesktopLogoContentTypeV1::Webp => {
            data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP"
        }
        LegacyDesktopLogoContentTypeV1::Gif => {
            data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")
        }
        LegacyDesktopLogoContentTypeV1::Avif => {
            data.len() >= 12 && &data[4..8] == b"ftyp" && matches!(&data[8..12], b"avif" | b"avis")
        }
    }
}

fn parse_rfc3339_millis(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't' | b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let digits = |start: usize, end: usize| -> Option<i64> {
        bytes.get(start..end)?.iter().try_fold(0_i64, |sum, byte| {
            byte.is_ascii_digit()
                .then_some(sum * 10 + i64::from(byte - b'0'))
        })
    };
    let year = digits(0, 4)?;
    let month = digits(5, 7)?;
    let day = digits(8, 10)?;
    let hour = digits(11, 13)?;
    let minute = digits(14, 16)?;
    let second = digits(17, 19)?;
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut cursor = 19;
    let mut millis = 0_i64;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == start {
            return None;
        }
        let fraction = bytes.get(start..cursor)?;
        for (index, byte) in fraction.iter().take(3).enumerate() {
            millis += i64::from(byte - b'0') * [100_i64, 10, 1][index];
        }
    }
    let offset_minutes = match bytes.get(cursor) {
        Some(b'Z' | b'z') if cursor + 1 == bytes.len() => 0_i64,
        Some(sign @ (b'+' | b'-')) if cursor + 6 == bytes.len() => {
            if bytes.get(cursor + 3) != Some(&b':') {
                return None;
            }
            let hours = digits(cursor + 1, cursor + 3)?;
            let minutes = digits(cursor + 4, cursor + 6)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let offset = hours * 60 + minutes;
            if *sign == b'+' { offset } else { -offset }
        }
        _ => return None,
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > days_in_month[usize::try_from(month - 1).ok()?] {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    days.checked_mul(86_400_000)?
        .checked_add(hour * 3_600_000 + minute * 60_000 + second * 1_000 + millis)?
        .checked_sub(offset_minutes * 60_000)
        .filter(|value| (0..=9_007_199_254_740_991).contains(value))
}

fn trim_ecmascript(value: &str) -> &str {
    value.trim_matches(is_ecmascript_whitespace)
}

const fn is_ecmascript_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            | '\u{2001}'
            | '\u{2002}'
            | '\u{2003}'
            | '\u{2004}'
            | '\u{2005}'
            | '\u{2006}'
            | '\u{2007}'
            | '\u{2008}'
            | '\u{2009}'
            | '\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

fn valid_boundary_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_source_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2_048
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn uuid_shape(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn digest_fields(domain: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    lower_hex(&digest.finalize())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn map_port_error(
    error: LegacyDesktopCompatibilityPortErrorV1,
) -> LegacyDesktopCompatibilityErrorV1 {
    match error {
        LegacyDesktopCompatibilityPortErrorV1::NotFound => {
            LegacyDesktopCompatibilityErrorV1::NotFound
        }
        LegacyDesktopCompatibilityPortErrorV1::BrandingForbidden => {
            LegacyDesktopCompatibilityErrorV1::BrandingForbidden
        }
        LegacyDesktopCompatibilityPortErrorV1::StorageNotConnected => {
            LegacyDesktopCompatibilityErrorV1::StorageNotConnected
        }
        LegacyDesktopCompatibilityPortErrorV1::IdempotencyConflict => {
            LegacyDesktopCompatibilityErrorV1::Conflict
        }
        LegacyDesktopCompatibilityPortErrorV1::Unavailable => {
            LegacyDesktopCompatibilityErrorV1::Unavailable
        }
        LegacyDesktopCompatibilityPortErrorV1::Provider => {
            LegacyDesktopCompatibilityErrorV1::Provider
        }
        LegacyDesktopCompatibilityPortErrorV1::Corrupt => {
            LegacyDesktopCompatibilityErrorV1::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingPort {
        commands: Mutex<Vec<LegacyDesktopCompatibilityCommandV1>>,
    }

    #[async_trait(?Send)]
    impl LegacyDesktopCompatibilityPortV1 for RecordingPort {
        async fn execute(
            &self,
            command: LegacyDesktopCompatibilityCommandV1,
        ) -> Result<LegacyDesktopCompatibilityOutcomeV1, LegacyDesktopCompatibilityPortErrorV1>
        {
            let result = match command.surface() {
                LegacyDesktopCompatibilitySurfaceV1::Organizations => {
                    LegacyDesktopCompatibilityResultV1::Organizations(Vec::new())
                }
                LegacyDesktopCompatibilitySurfaceV1::UserProfile => {
                    LegacyDesktopCompatibilityResultV1::UserProfile(LegacyDesktopUserProfileV1 {
                        name: None,
                        email: None,
                        image_url: None,
                    })
                }
                LegacyDesktopCompatibilitySurfaceV1::OrganizationBranding => {
                    LegacyDesktopCompatibilityResultV1::Organization(LegacyDesktopOrganizationV1 {
                        id: "0123456789abcde".into(),
                        name: "Engineering".into(),
                        owner_id: "0123456789abcdf".into(),
                        role: LegacyDesktopOrganizationRoleV1::Owner,
                        can_edit_brand: true,
                        icon_url: None,
                        brand_colors: LegacyDesktopBrandColorsV1::default(),
                    })
                }
                LegacyDesktopCompatibilitySurfaceV1::StorageSetActive => {
                    LegacyDesktopCompatibilityResultV1::StorageSuccess
                }
                LegacyDesktopCompatibilitySurfaceV1::VideoDelete
                | LegacyDesktopCompatibilitySurfaceV1::VideoProgress => {
                    LegacyDesktopCompatibilityResultV1::JsonTrue
                }
            };
            self.commands.lock().expect("commands").push(command);
            Ok(LegacyDesktopCompatibilityOutcomeV1 {
                result,
                replayed: false,
            })
        }
    }

    fn actor() -> String {
        "00000000-0000-7000-8000-000000000001".into()
    }

    #[tokio::test]
    async fn reads_forbid_idempotency_and_mutations_generate_or_bind_a_key() {
        let port = RecordingPort::default();
        let adapter = LegacyDesktopCompatibilityAdapterV1::new(&port);
        adapter
            .execute(LegacyDesktopCompatibilityRequestV1 {
                actor_id: actor(),
                credential: LegacyDesktopCredentialV1::ApiKey,
                input: LegacyDesktopCompatibilityInputV1::Organizations,
                idempotency_key: None,
            })
            .await
            .expect("read");
        assert_eq!(
            port.commands.lock().expect("commands")[0].idempotency_key(),
            None
        );
        assert_eq!(
            adapter
                .execute(LegacyDesktopCompatibilityRequestV1 {
                    actor_id: actor(),
                    credential: LegacyDesktopCredentialV1::Session,
                    input: LegacyDesktopCompatibilityInputV1::UserProfile,
                    idempotency_key: Some("forbidden-read-key".into()),
                })
                .await,
            Err(LegacyDesktopCompatibilityErrorV1::InvalidInput)
        );
        adapter
            .execute(LegacyDesktopCompatibilityRequestV1 {
                actor_id: actor(),
                credential: LegacyDesktopCredentialV1::ApiKey,
                input: LegacyDesktopCompatibilityInputV1::StorageSetActive {
                    provider: LegacyDesktopStorageProviderV1::S3,
                },
                idempotency_key: None,
            })
            .await
            .expect("mutation");
        adapter
            .execute(LegacyDesktopCompatibilityRequestV1 {
                actor_id: actor(),
                credential: LegacyDesktopCredentialV1::ApiKey,
                input: LegacyDesktopCompatibilityInputV1::StorageSetActive {
                    provider: LegacyDesktopStorageProviderV1::GoogleDrive,
                },
                idempotency_key: Some("desktop-storage-key-1".into()),
            })
            .await
            .expect("keyed mutation");
        let commands = port.commands.lock().expect("commands");
        assert!(commands[1].idempotency_key().is_some());
        assert_eq!(
            commands[2].idempotency_key().expect("key").expose(),
            "desktop-storage-key-1"
        );
    }

    #[test]
    fn branding_normalizes_colors_preserves_metadata_and_validates_magic() {
        let wire: LegacyDesktopBrandingPatchWireV1 = serde_json::from_value(serde_json::json!({
            "brandColors": {
                "primary":"#aabbcc", "secondary":null, "accent":"#123456", "background":null
            },
            "logo": {"action":"upload", "contentType":"image/png", "data":"iVBORw0KGgo="}
        }))
        .expect("wire");
        let patch = wire.normalize().expect("patch");
        assert_eq!(patch.brand_colors.primary.as_deref(), Some("#AABBCC"));
        assert!(matches!(
            patch.logo,
            LegacyDesktopLogoUpdateV1::Upload { .. }
        ));
        let merged = merge_organization_branding_metadata(
            serde_json::json!({"keep":1,"branding":{"keepToo":2}}),
            &patch.brand_colors,
        );
        assert_eq!(merged["keep"], 1);
        assert_eq!(merged["branding"]["keepToo"], 2);
        assert_eq!(merged["branding"]["colors"]["primary"], "#AABBCC");
        let invalid = LegacyDesktopBrandingPatchWireV1 {
            brand_colors: LegacyDesktopBrandColorsV1::default(),
            logo: LegacyDesktopLogoWireV1::Upload {
                content_type: LegacyDesktopLogoContentTypeV1::Jpeg,
                data: "iVBORw0KGgo=".into(),
            },
        };
        assert_eq!(
            invalid.normalize(),
            Err(LegacyDesktopCompatibilityErrorV1::LogoTypeInvalid)
        );
    }

    #[test]
    fn progress_accepts_the_released_chrono_rfc3339_shape_and_clamps_uploaded() {
        let progress = LegacyDesktopVideoProgressWireV1 {
            video_id: "0123456789abcde".into(),
            uploaded: 200.0,
            total: 100.0,
            updated_at: "2026-07-17T12:34:56.123456+00:00".into(),
        }
        .normalize()
        .expect("progress");
        assert_eq!(progress.uploaded, 100.0);
        assert_eq!(progress.updated_at_ms, 1_784_291_696_123);
    }

    #[test]
    fn profile_join_uses_ecmascript_truthiness_and_whitespace() {
        assert_eq!(
            desktop_profile_name(Some("Ada"), Some("Lovelace")),
            Some("Ada Lovelace".into())
        );
        assert_eq!(desktop_profile_name(Some("\u{FEFF}"), None), None);
        assert_eq!(
            desktop_profile_name(Some(""), Some("Hopper")),
            Some("Hopper".into())
        );
    }

    #[test]
    fn video_mapping_matches_the_existing_cap_uuidv8_mapping() {
        let mapped = mapped_legacy_video_uuid("0123456789abcde").expect("mapped");
        assert_eq!(mapped.len(), 36);
        assert_eq!(mapped.as_bytes()[14], b'8');
        assert_eq!(
            mapped,
            mapped_legacy_video_uuid("0123456789abcde").expect("stable")
        );
    }
}
