//! Source-pinned compatibility semantics for Cap's ten retained video-property mutations.
//!
//! These operations look similar at the storage layer but intentionally keep
//! different observable rules. Mobile titles and passwords ECMAScript-trim;
//! browser titles and passwords do not. Metadata is replaced with any truthy
//! JSON value, while title/date edits spread the existing runtime value before
//! adding their key. Password verification is anonymous and considers the
//! video's password before joined space passwords. Every distinction is bound
//! into the atomic request fingerprint.

use std::{fmt, num::NonZeroU32, str::FromStr};

use async_trait::async_trait;
use frame_domain::{IdempotencyKey, OrganizationOperationId, UserId, VideoId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_VIDEO_PROPERTIES_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_MOBILE_VIDEO_PASSWORD_OPERATION_ID: &str = "cap-v1-2cfe7fc40a6f5a78";
pub const LEGACY_MOBILE_VIDEO_SHARING_OPERATION_ID: &str = "cap-v1-5fdf332d1448aedc";
pub const LEGACY_MOBILE_VIDEO_TITLE_OPERATION_ID: &str = "cap-v1-b2db0e7ec51f7898";
pub const LEGACY_VIDEO_METADATA_OPERATION_ID: &str = "cap-v1-5b36dac105856ede";
pub const LEGACY_EDIT_VIDEO_DATE_OPERATION_ID: &str = "cap-v1-96c52e9330f9a131";
pub const LEGACY_EDIT_VIDEO_TITLE_OPERATION_ID: &str = "cap-v1-6e9f3d370f1ce239";
pub const LEGACY_REMOVE_VIDEO_PASSWORD_OPERATION_ID: &str = "cap-v1-ab11637faa2de45e";
pub const LEGACY_SET_VIDEO_PASSWORD_OPERATION_ID: &str = "cap-v1-455e6a1b82e647d9";
pub const LEGACY_VERIFY_VIDEO_PASSWORD_OPERATION_ID: &str = "cap-v1-0a2c44d7a626a1fe";
pub const LEGACY_UPDATE_VIDEO_SETTINGS_OPERATION_ID: &str = "cap-v1-49dba3fbc7c4a74c";

pub const LEGACY_MOBILE_VIDEO_PASSWORD_IDENTITY: &str = "/api/mobile/caps/:id/password";
pub const LEGACY_MOBILE_VIDEO_SHARING_IDENTITY: &str = "/api/mobile/caps/:id/sharing";
pub const LEGACY_MOBILE_VIDEO_TITLE_IDENTITY: &str = "/api/mobile/caps/:id/title";
pub const LEGACY_VIDEO_METADATA_IDENTITY: &str = "/api/video/metadata";
pub const LEGACY_EDIT_VIDEO_DATE_IDENTITY: &str =
    "action://apps/web/actions/videos/edit-date.ts#editDate";
pub const LEGACY_EDIT_VIDEO_TITLE_IDENTITY: &str =
    "action://apps/web/actions/videos/edit-title.ts#editTitle";
pub const LEGACY_REMOVE_VIDEO_PASSWORD_IDENTITY: &str =
    "action://apps/web/actions/videos/password.ts#removeVideoPassword";
pub const LEGACY_SET_VIDEO_PASSWORD_IDENTITY: &str =
    "action://apps/web/actions/videos/password.ts#setVideoPassword";
pub const LEGACY_VERIFY_VIDEO_PASSWORD_IDENTITY: &str =
    "action://apps/web/actions/videos/password.ts#verifyVideoPassword";
pub const LEGACY_UPDATE_VIDEO_SETTINGS_IDENTITY: &str =
    "action://apps/web/actions/videos/settings.ts#updateVideoSettings";

pub const LEGACY_VIDEO_PROPERTIES_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_VIDEO_PROPERTIES_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_VIDEO_PROPERTIES_PROVIDER_GATES: &[&str] = &["provider_execution"];
pub const LEGACY_VIDEO_DATE_PROTECTED_GATES: &[&str] = &["human_approval"];
pub const LEGACY_VIDEO_TITLE_MAX_CHARACTERS: usize = 255;
pub const LEGACY_PASSWORD_PBKDF2_ITERATIONS: u32 = 100_000;
pub const LEGACY_PASSWORD_SALT_BYTES: usize = 16;
pub const LEGACY_PASSWORD_DERIVED_BYTES: usize = 32;
pub const LEGACY_PASSWORD_WIRE_BYTES: usize = 48;
pub const LEGACY_PASSWORD_BASE64_LENGTH: usize = 64;
pub const LEGACY_PASSWORD_COOKIE_MAX_HASHES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoPropertySourceRoleV1 {
    Transport,
    Contract,
    Authentication,
    Schema,
    Repository,
    Service,
    Crypto,
    Cookie,
    Action,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVideoPropertySourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyVideoPropertySourceRoleV1,
}

const MOBILE_ROUTE: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "apps/web/app/api/mobile/[...route]/route.ts",
    symbol: "mobile video-property handler",
    sha256: "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
    role: LegacyVideoPropertySourceRoleV1::Transport,
};
const MOBILE_CONTRACT: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "packages/web-domain/src/Mobile.ts",
    symbol: "MobileCapSummary+mobile video-property inputs",
    sha256: "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
    role: LegacyVideoPropertySourceRoleV1::Contract,
};
const DATABASE_SCHEMA: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "videos+spaces+spaceVideos+comments+videoUploads",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyVideoPropertySourceRoleV1::Schema,
};
const SESSION: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "packages/database/auth/session.ts",
    symbol: "getCurrentUser",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyVideoPropertySourceRoleV1::Authentication,
};
const PASSWORD_CRYPTO: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "packages/database/crypto.ts",
    symbol: "hashPassword+verifyPassword+encrypt+decrypt",
    sha256: "d547c7ba0f984d1e625d807e4a1e64cfb400ed2fcc796cf9f6e43713805efb6f",
    role: LegacyVideoPropertySourceRoleV1::Crypto,
};
const VIDEOS_SERVICE: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "packages/web-backend/src/Videos/index.ts",
    symbol: "Videos.getThumbnailURL+getAnalytics",
    sha256: "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
    role: LegacyVideoPropertySourceRoleV1::Service,
};

pub const LEGACY_MOBILE_VIDEO_PASSWORD_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        symbol: "mobile handler:updateCapPassword",
        ..MOBILE_ROUTE
    },
    LegacyVideoPropertySourcePinV1 {
        symbol: "updateCapPassword",
        ..MOBILE_CONTRACT
    },
    DATABASE_SCHEMA,
    PASSWORD_CRYPTO,
    VIDEOS_SERVICE,
];
pub const LEGACY_MOBILE_VIDEO_SHARING_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        symbol: "mobile handler:updateCapSharing",
        ..MOBILE_ROUTE
    },
    LegacyVideoPropertySourcePinV1 {
        symbol: "updateCapSharing",
        ..MOBILE_CONTRACT
    },
    DATABASE_SCHEMA,
    VIDEOS_SERVICE,
];
pub const LEGACY_MOBILE_VIDEO_TITLE_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        symbol: "mobile handler:updateCapTitle",
        ..MOBILE_ROUTE
    },
    LegacyVideoPropertySourcePinV1 {
        symbol: "updateCapTitle",
        ..MOBILE_CONTRACT
    },
    DATABASE_SCHEMA,
    VIDEOS_SERVICE,
];
pub const LEGACY_VIDEO_METADATA_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        path: "apps/web/app/api/video/metadata/route.ts",
        symbol: "PUT",
        sha256: "cbd25bc1150aa53dea5f5b8a120c899c36b151134af307f89992108e81b17812",
        role: LegacyVideoPropertySourceRoleV1::Transport,
    },
    SESSION,
    DATABASE_SCHEMA,
];
pub const LEGACY_EDIT_VIDEO_DATE_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        path: "apps/web/actions/videos/edit-date.ts",
        symbol: "editDate",
        sha256: "f54229f4aed3648988529a310a0cb831c530aac54177766312abe029d8326d78",
        role: LegacyVideoPropertySourceRoleV1::Action,
    },
    SESSION,
    DATABASE_SCHEMA,
    LegacyVideoPropertySourcePinV1 {
        path: "packages/database/types/metadata.ts",
        symbol: "VideoMetadata.customCreatedAt",
        sha256: "bedb7af94afc4332bbcc2e86ed195641d670d9937ef3ddae6ba62186b6dfbcee",
        role: LegacyVideoPropertySourceRoleV1::Contract,
    },
];
pub const LEGACY_EDIT_VIDEO_TITLE_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        path: "apps/web/actions/videos/edit-title.ts",
        symbol: "editTitle",
        sha256: "7991386a504054dbafa40b46ff46a1f0fa11791b36adf503623fc17a55a6ecf8",
        role: LegacyVideoPropertySourceRoleV1::Action,
    },
    SESSION,
    DATABASE_SCHEMA,
    LegacyVideoPropertySourcePinV1 {
        path: "packages/database/types/metadata.ts",
        symbol: "VideoMetadata.titleManuallyEdited",
        sha256: "bedb7af94afc4332bbcc2e86ed195641d670d9937ef3ddae6ba62186b6dfbcee",
        role: LegacyVideoPropertySourceRoleV1::Contract,
    },
];
const PASSWORD_ACTION: LegacyVideoPropertySourcePinV1 = LegacyVideoPropertySourcePinV1 {
    path: "apps/web/actions/videos/password.ts",
    symbol: "video password actions",
    sha256: "13a240f004a307bba1e0b66b4341036dcab941aa1037ae16dfdcbfcbd485b119",
    role: LegacyVideoPropertySourceRoleV1::Action,
};
pub const LEGACY_REMOVE_VIDEO_PASSWORD_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        symbol: "removeVideoPassword",
        ..PASSWORD_ACTION
    },
    SESSION,
    DATABASE_SCHEMA,
];
pub const LEGACY_SET_VIDEO_PASSWORD_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        symbol: "setVideoPassword",
        ..PASSWORD_ACTION
    },
    SESSION,
    DATABASE_SCHEMA,
    PASSWORD_CRYPTO,
];
pub const LEGACY_VERIFY_VIDEO_PASSWORD_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        symbol: "verifyVideoPassword",
        ..PASSWORD_ACTION
    },
    DATABASE_SCHEMA,
    PASSWORD_CRYPTO,
    LegacyVideoPropertySourcePinV1 {
        path: "packages/web-backend/src/Videos/EffectiveVideoRules.ts",
        symbol: "collectPasswordHashes",
        sha256: "e9b26784e4a1ed5782f9a5cfab52231de629b2f0a3d1b5f40d577b3c798cd015",
        role: LegacyVideoPropertySourceRoleV1::Service,
    },
    LegacyVideoPropertySourcePinV1 {
        path: "apps/web/lib/password-cookie.ts",
        symbol: "setVerifiedPasswordCookie+MAX_VERIFIED_HASHES",
        sha256: "3af65d04b06ca336b5e6659806380e4552d1d5514abfbc7f7d771c7cb75260e7",
        role: LegacyVideoPropertySourceRoleV1::Cookie,
    },
];
pub const LEGACY_UPDATE_VIDEO_SETTINGS_SOURCES: &[LegacyVideoPropertySourcePinV1] = &[
    LegacyVideoPropertySourcePinV1 {
        path: "apps/web/actions/videos/settings.ts",
        symbol: "updateVideoSettings",
        sha256: "c6dcfca09bcde824b071c56432124ccec9fa2c690b528afd131284d69c0bf78c",
        role: LegacyVideoPropertySourceRoleV1::Action,
    },
    SESSION,
    DATABASE_SCHEMA,
    LegacyVideoPropertySourcePinV1 {
        path: "apps/web/lib/playback-speed.ts",
        symbol: "normalizePlaybackSpeed+PLAYBACK_SPEEDS",
        sha256: "ac57f7543696c735d6d60def2b62482c34a472161c0558c0cd03d97c2a3b5ced",
        role: LegacyVideoPropertySourceRoleV1::Contract,
    },
];

// Canonical SHA-256 over each report row's sorted path/symbol/hash source array.
pub const LEGACY_MOBILE_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256: &str =
    "24e4585815c8a22ce021d49c460f5c27cc1c038431a9345e4f2c3bcf08d5175a";
pub const LEGACY_MOBILE_VIDEO_SHARING_SOURCE_MANIFEST_SHA256: &str =
    "07fd9bbfc4674bdd69b0d983dd5e13635ff229b8f6c15dff7a7d27e92a33c46a";
pub const LEGACY_MOBILE_VIDEO_TITLE_SOURCE_MANIFEST_SHA256: &str =
    "048f7393a88ed0317ab1ecd4f3c493848c23493b11ae25932802d2870cd1c292";
pub const LEGACY_VIDEO_METADATA_SOURCE_MANIFEST_SHA256: &str =
    "01a4d4f059a7d9c28d4c0580f0dccb19e3752808a4bf29899a6edd14e78873a7";
pub const LEGACY_EDIT_VIDEO_DATE_SOURCE_MANIFEST_SHA256: &str =
    "13d31ba77997d6b33ef14c1dfac6c6cc31e192368ebba11c9f66e6b31f9d5ce1";
pub const LEGACY_EDIT_VIDEO_TITLE_SOURCE_MANIFEST_SHA256: &str =
    "537d4de6afbbd653fceedcc90864d1e46d4083a58db693c5b2d954d04cb879b1";
pub const LEGACY_REMOVE_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256: &str =
    "334eecf86628602d944d8af6f4125f25379510a28259b396cc48c92febf7afdd";
pub const LEGACY_SET_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256: &str =
    "3a96a2b46b7c5f7516a88225b56a8b3513d03c90980d1154fad7c65bf8dcbe2d";
pub const LEGACY_VERIFY_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256: &str =
    "f08afa095fb760b933f6299fdcf414327925b7b3e9957e2924e9827fd3e87945";
pub const LEGACY_UPDATE_VIDEO_SETTINGS_SOURCE_MANIFEST_SHA256: &str =
    "95c1c1f551e17bceaec86e945b6f8b92a780d372de89974e57f00c6b3950fffa";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegacyVideoPropertiesSurfaceV1 {
    MobilePassword,
    MobileSharing,
    MobileTitle,
    MetadataPut,
    EditDate,
    EditTitle,
    RemovePassword,
    SetPassword,
    VerifyPassword,
    UpdateSettings,
}

impl LegacyVideoPropertiesSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::MobilePassword => LEGACY_MOBILE_VIDEO_PASSWORD_OPERATION_ID,
            Self::MobileSharing => LEGACY_MOBILE_VIDEO_SHARING_OPERATION_ID,
            Self::MobileTitle => LEGACY_MOBILE_VIDEO_TITLE_OPERATION_ID,
            Self::MetadataPut => LEGACY_VIDEO_METADATA_OPERATION_ID,
            Self::EditDate => LEGACY_EDIT_VIDEO_DATE_OPERATION_ID,
            Self::EditTitle => LEGACY_EDIT_VIDEO_TITLE_OPERATION_ID,
            Self::RemovePassword => LEGACY_REMOVE_VIDEO_PASSWORD_OPERATION_ID,
            Self::SetPassword => LEGACY_SET_VIDEO_PASSWORD_OPERATION_ID,
            Self::VerifyPassword => LEGACY_VERIFY_VIDEO_PASSWORD_OPERATION_ID,
            Self::UpdateSettings => LEGACY_UPDATE_VIDEO_SETTINGS_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::MobilePassword => "mobile_password",
            Self::MobileSharing => "mobile_sharing",
            Self::MobileTitle => "mobile_title",
            Self::MetadataPut => "metadata_put",
            Self::EditDate => "edit_date",
            Self::EditTitle => "edit_title",
            Self::RemovePassword => "remove_password",
            Self::SetPassword => "set_password",
            Self::VerifyPassword => "verify_password",
            Self::UpdateSettings => "update_settings",
        }
    }

    #[must_use]
    pub const fn is_mobile(self) -> bool {
        matches!(
            self,
            Self::MobilePassword | Self::MobileSharing | Self::MobileTitle
        )
    }

    #[must_use]
    pub const fn is_password_result(self) -> bool {
        matches!(
            self,
            Self::RemovePassword | Self::SetPassword | Self::VerifyPassword
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoPropertyValidationV1 {
    MobileNullablePasswordEcmaTrimClearBlank,
    BooleanSharing,
    MobileTitleEcmaTrimRejectBlank,
    TruthyMetadataReplacement,
    SafeIsoDateSubsetFutureRejected,
    BrowserTitleRejectEmptyPreserveWhitespace,
    VideoIdOnly,
    BrowserPasswordStringNoTrimEmptyHashesAndFails,
    AnonymousPasswordString,
    RuntimeSettingsReplacementPreserveUnknownNormalizeSpeed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoPropertyAuthorityV1 {
    OwnerSessionOrMobileApiKey,
    OwnerSession,
    AnonymousVideoAndJoinedSpaces,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoPropertySuccessV1 {
    FullMobileCapSummaryProviderFallbacks,
    JsonTrue,
    SuccessObject,
    PasswordActionObject,
    VerificationObjectAndEncryptedCookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVideoPropertyProfileV1 {
    pub surface: LegacyVideoPropertiesSurfaceV1,
    pub identity: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub source_manifest_sha256: &'static str,
    pub sources: &'static [LegacyVideoPropertySourcePinV1],
    pub validation: LegacyVideoPropertyValidationV1,
    pub authority: LegacyVideoPropertyAuthorityV1,
    pub success: LegacyVideoPropertySuccessV1,
    pub rate_limit_bucket: &'static str,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

macro_rules! profile {
    ($surface:ident,$identity:expr,$kind:expr,$method:expr,$manifest:expr,$sources:expr,
     $validation:ident,$authority:ident,$success:ident,$bucket:expr,$gates:expr,$promoted:expr) => {
        LegacyVideoPropertyProfileV1 {
            surface: LegacyVideoPropertiesSurfaceV1::$surface,
            identity: $identity,
            kind: $kind,
            method: $method,
            source_manifest_sha256: $manifest,
            sources: $sources,
            validation: LegacyVideoPropertyValidationV1::$validation,
            authority: LegacyVideoPropertyAuthorityV1::$authority,
            success: LegacyVideoPropertySuccessV1::$success,
            rate_limit_bucket: $bucket,
            protected_gates: $gates,
            production_promoted: $promoted,
        }
    };
}

pub const LEGACY_VIDEO_PROPERTY_PROFILES: &[LegacyVideoPropertyProfileV1] = &[
    profile!(
        MobilePassword,
        LEGACY_MOBILE_VIDEO_PASSWORD_IDENTITY,
        "route",
        "PATCH",
        LEGACY_MOBILE_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256,
        LEGACY_MOBILE_VIDEO_PASSWORD_SOURCES,
        MobileNullablePasswordEcmaTrimClearBlank,
        OwnerSessionOrMobileApiKey,
        FullMobileCapSummaryProviderFallbacks,
        "share_playback.v1",
        LEGACY_VIDEO_PROPERTIES_PROVIDER_GATES,
        false
    ),
    profile!(
        MobileSharing,
        LEGACY_MOBILE_VIDEO_SHARING_IDENTITY,
        "route",
        "PATCH",
        LEGACY_MOBILE_VIDEO_SHARING_SOURCE_MANIFEST_SHA256,
        LEGACY_MOBILE_VIDEO_SHARING_SOURCES,
        BooleanSharing,
        OwnerSessionOrMobileApiKey,
        FullMobileCapSummaryProviderFallbacks,
        "client_compatibility.v1",
        LEGACY_VIDEO_PROPERTIES_PROVIDER_GATES,
        false
    ),
    profile!(
        MobileTitle,
        LEGACY_MOBILE_VIDEO_TITLE_IDENTITY,
        "route",
        "PATCH",
        LEGACY_MOBILE_VIDEO_TITLE_SOURCE_MANIFEST_SHA256,
        LEGACY_MOBILE_VIDEO_TITLE_SOURCES,
        MobileTitleEcmaTrimRejectBlank,
        OwnerSessionOrMobileApiKey,
        FullMobileCapSummaryProviderFallbacks,
        "client_compatibility.v1",
        LEGACY_VIDEO_PROPERTIES_PROVIDER_GATES,
        false
    ),
    profile!(
        MetadataPut,
        LEGACY_VIDEO_METADATA_IDENTITY,
        "route",
        "PUT",
        LEGACY_VIDEO_METADATA_SOURCE_MANIFEST_SHA256,
        LEGACY_VIDEO_METADATA_SOURCES,
        TruthyMetadataReplacement,
        OwnerSession,
        JsonTrue,
        "video_media.v1",
        LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        EditDate,
        LEGACY_EDIT_VIDEO_DATE_IDENTITY,
        "server_action",
        "ACTION",
        LEGACY_EDIT_VIDEO_DATE_SOURCE_MANIFEST_SHA256,
        LEGACY_EDIT_VIDEO_DATE_SOURCES,
        SafeIsoDateSubsetFutureRejected,
        OwnerSession,
        SuccessObject,
        "video_media.v1",
        LEGACY_VIDEO_DATE_PROTECTED_GATES,
        false
    ),
    profile!(
        EditTitle,
        LEGACY_EDIT_VIDEO_TITLE_IDENTITY,
        "server_action",
        "ACTION",
        LEGACY_EDIT_VIDEO_TITLE_SOURCE_MANIFEST_SHA256,
        LEGACY_EDIT_VIDEO_TITLE_SOURCES,
        BrowserTitleRejectEmptyPreserveWhitespace,
        OwnerSession,
        SuccessObject,
        "video_media.v1",
        LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        RemovePassword,
        LEGACY_REMOVE_VIDEO_PASSWORD_IDENTITY,
        "server_action",
        "ACTION",
        LEGACY_REMOVE_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256,
        LEGACY_REMOVE_VIDEO_PASSWORD_SOURCES,
        VideoIdOnly,
        OwnerSession,
        PasswordActionObject,
        "share_playback.v1",
        LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        SetPassword,
        LEGACY_SET_VIDEO_PASSWORD_IDENTITY,
        "server_action",
        "ACTION",
        LEGACY_SET_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256,
        LEGACY_SET_VIDEO_PASSWORD_SOURCES,
        BrowserPasswordStringNoTrimEmptyHashesAndFails,
        OwnerSession,
        PasswordActionObject,
        "share_playback.v1",
        LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        VerifyPassword,
        LEGACY_VERIFY_VIDEO_PASSWORD_IDENTITY,
        "server_action",
        "ACTION",
        LEGACY_VERIFY_VIDEO_PASSWORD_SOURCE_MANIFEST_SHA256,
        LEGACY_VERIFY_VIDEO_PASSWORD_SOURCES,
        AnonymousPasswordString,
        AnonymousVideoAndJoinedSpaces,
        VerificationObjectAndEncryptedCookie,
        "share_playback.v1",
        LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        UpdateSettings,
        LEGACY_UPDATE_VIDEO_SETTINGS_IDENTITY,
        "server_action",
        "ACTION",
        LEGACY_UPDATE_VIDEO_SETTINGS_SOURCE_MANIFEST_SHA256,
        LEGACY_UPDATE_VIDEO_SETTINGS_SOURCES,
        RuntimeSettingsReplacementPreserveUnknownNormalizeSpeed,
        OwnerSession,
        SuccessObject,
        "video_media.v1",
        LEGACY_VIDEO_PROPERTIES_NO_PROTECTED_GATES,
        true
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoPropertiesCredentialV1 {
    Session,
    ApiKey,
    Anonymous,
}

#[derive(Clone, PartialEq)]
pub enum LegacyVideoPropertiesInputV1 {
    MobilePassword {
        legacy_video_id: String,
        password: Option<String>,
    },
    MobileSharing {
        legacy_video_id: String,
        public: bool,
    },
    MobileTitle {
        legacy_video_id: String,
        title: String,
    },
    MetadataPut {
        legacy_video_id: String,
        metadata: Value,
    },
    EditDate {
        legacy_video_id: String,
        date: String,
        now_ms: i64,
    },
    EditTitle {
        legacy_video_id: String,
        title: String,
    },
    RemovePassword {
        legacy_video_id: String,
    },
    SetPassword {
        legacy_video_id: String,
        password: String,
    },
    VerifyPassword {
        legacy_video_id: String,
        password: String,
    },
    UpdateSettings {
        legacy_video_id: String,
        settings: Value,
    },
}

impl fmt::Debug for LegacyVideoPropertiesInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyVideoPropertiesInputV1")
            .field("surface", &self.surface())
            .field("payload", &"<redacted>")
            .finish()
    }
}

impl LegacyVideoPropertiesInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyVideoPropertiesSurfaceV1 {
        match self {
            Self::MobilePassword { .. } => LegacyVideoPropertiesSurfaceV1::MobilePassword,
            Self::MobileSharing { .. } => LegacyVideoPropertiesSurfaceV1::MobileSharing,
            Self::MobileTitle { .. } => LegacyVideoPropertiesSurfaceV1::MobileTitle,
            Self::MetadataPut { .. } => LegacyVideoPropertiesSurfaceV1::MetadataPut,
            Self::EditDate { .. } => LegacyVideoPropertiesSurfaceV1::EditDate,
            Self::EditTitle { .. } => LegacyVideoPropertiesSurfaceV1::EditTitle,
            Self::RemovePassword { .. } => LegacyVideoPropertiesSurfaceV1::RemovePassword,
            Self::SetPassword { .. } => LegacyVideoPropertiesSurfaceV1::SetPassword,
            Self::VerifyPassword { .. } => LegacyVideoPropertiesSurfaceV1::VerifyPassword,
            Self::UpdateSettings { .. } => LegacyVideoPropertiesSurfaceV1::UpdateSettings,
        }
    }

    #[must_use]
    pub fn legacy_video_id(&self) -> &str {
        match self {
            Self::MobilePassword {
                legacy_video_id, ..
            }
            | Self::MobileSharing {
                legacy_video_id, ..
            }
            | Self::MobileTitle {
                legacy_video_id, ..
            }
            | Self::MetadataPut {
                legacy_video_id, ..
            }
            | Self::EditDate {
                legacy_video_id, ..
            }
            | Self::EditTitle {
                legacy_video_id, ..
            }
            | Self::RemovePassword { legacy_video_id }
            | Self::SetPassword {
                legacy_video_id, ..
            }
            | Self::VerifyPassword {
                legacy_video_id, ..
            }
            | Self::UpdateSettings {
                legacy_video_id, ..
            } => legacy_video_id,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyVideoPropertiesRequestV1 {
    pub credential: LegacyVideoPropertiesCredentialV1,
    pub actor_id: Option<UserId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyVideoPropertiesInputV1,
}

impl fmt::Debug for LegacyVideoPropertiesRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyVideoPropertiesRequestV1")
            .field("credential", &self.credential)
            .field("actor_id", &self.actor_id)
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("input", &self.input)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileUploadProgressV1 {
    pub uploaded: f64,
    pub total: f64,
    pub phase: String,
    pub processing_progress: f64,
    pub processing_message: Option<String>,
    pub processing_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileCapSummaryV1 {
    pub id: String,
    pub share_url: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub owner_name: String,
    pub duration_seconds: Option<f64>,
    pub thumbnail_url: Option<String>,
    pub folder_id: Option<String>,
    pub public: bool,
    pub protected: bool,
    pub view_count: f64,
    pub comment_count: f64,
    pub reaction_count: f64,
    pub upload: Option<LegacyMobileUploadProgressV1>,
}

#[derive(Clone, PartialEq)]
pub enum LegacyVideoPropertyMutationV1 {
    MobilePassword { password: Option<String> },
    MobileSharing { public: bool },
    MobileTitle { title: String },
    MetadataReplace { metadata: Value },
    MetadataCustomDate { custom_created_at: String },
    BrowserTitle { title: String },
    RemovePassword,
    SetPassword { password: String },
    VerifyPassword { password: String },
    SettingsReplace { settings: Value },
}

impl fmt::Debug for LegacyVideoPropertyMutationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MobilePassword { .. } => "MobilePassword([redacted])",
            Self::MobileSharing { .. } => "MobileSharing",
            Self::MobileTitle { .. } => "MobileTitle([redacted])",
            Self::MetadataReplace { .. } => "MetadataReplace([redacted])",
            Self::MetadataCustomDate { .. } => "MetadataCustomDate([redacted])",
            Self::BrowserTitle { .. } => "BrowserTitle([redacted])",
            Self::RemovePassword => "RemovePassword",
            Self::SetPassword { .. } => "SetPassword([redacted])",
            Self::VerifyPassword { .. } => "VerifyPassword([redacted])",
            Self::SettingsReplace { .. } => "SettingsReplace([redacted])",
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyVideoPropertiesCommandV1 {
    operation_id: OrganizationOperationId,
    idempotency_key: IdempotencyKey,
    request_digest: String,
    surface: LegacyVideoPropertiesSurfaceV1,
    actor_id: Option<UserId>,
    legacy_video_id: String,
    video_id: VideoId,
    mutation: LegacyVideoPropertyMutationV1,
}

impl fmt::Debug for LegacyVideoPropertiesCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyVideoPropertiesCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.surface)
            .field("actor_id", &self.actor_id)
            .field("video_id", &self.video_id)
            .field("idempotency_key", &"<redacted>")
            .field("request_digest", &"<redacted>")
            .field("mutation", &self.mutation)
            .finish()
    }
}

impl LegacyVideoPropertiesCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> OrganizationOperationId {
        self.operation_id
    }
    #[must_use]
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }
    #[must_use]
    pub fn idempotency_key_digest_hex(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-legacy-video-property-key-v1\0");
        digest.update(self.idempotency_key.expose().as_bytes());
        lower_hex(&digest.finalize())
    }
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }
    #[must_use]
    pub const fn surface(&self) -> LegacyVideoPropertiesSurfaceV1 {
        self.surface
    }
    #[must_use]
    pub const fn actor_id(&self) -> Option<UserId> {
        self.actor_id
    }
    #[must_use]
    pub fn legacy_video_id(&self) -> &str {
        &self.legacy_video_id
    }
    #[must_use]
    pub const fn video_id(&self) -> VideoId {
        self.video_id
    }
    #[must_use]
    pub fn mutation(&self) -> &LegacyVideoPropertyMutationV1 {
        &self.mutation
    }
}

#[derive(Clone, PartialEq)]
pub enum LegacyVideoPropertiesAtomicResultV1 {
    MobileSummary(Box<LegacyMobileCapSummaryV1>),
    JsonTrue,
    SuccessObject,
    PasswordSet,
    PasswordRemoved,
    PasswordVerified { matched_hash: String },
    PasswordRejected,
}

impl fmt::Debug for LegacyVideoPropertiesAtomicResultV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MobileSummary(summary) => formatter
                .debug_tuple("MobileSummary")
                .field(summary)
                .finish(),
            Self::JsonTrue => formatter.write_str("JsonTrue"),
            Self::SuccessObject => formatter.write_str("SuccessObject"),
            Self::PasswordSet => formatter.write_str("PasswordSet"),
            Self::PasswordRemoved => formatter.write_str("PasswordRemoved"),
            Self::PasswordVerified { .. } => formatter.write_str("PasswordVerified([redacted])"),
            Self::PasswordRejected => formatter.write_str("PasswordRejected"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyVideoPropertiesAtomicOutcomeV1 {
    pub result: LegacyVideoPropertiesAtomicResultV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoPropertiesAtomicErrorV1 {
    TargetMissing,
    AccessDenied,
    StaleAuthority,
    IdempotencyConflict,
    Database,
    Crypto,
    Unavailable,
    Corrupt,
}

#[async_trait(?Send)]
pub trait LegacyVideoPropertiesAtomicPortV1 {
    async fn execute(
        &self,
        command: LegacyVideoPropertiesCommandV1,
    ) -> Result<LegacyVideoPropertiesAtomicOutcomeV1, LegacyVideoPropertiesAtomicErrorV1>;
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyVideoPropertiesErrorV1 {
    #[error("authentication required")]
    Unauthorized,
    #[error("invalid input")]
    InvalidInput,
    #[error("video not found")]
    NotFound,
    #[error("access denied")]
    AccessDenied,
    #[error("request conflicts with current state")]
    Conflict,
    #[error("date is outside the locally exact JavaScript subset")]
    UnsupportedDate,
    #[error("database failure")]
    Database,
    #[error("password operation failed")]
    PasswordFailure,
    #[error("service unavailable")]
    Unavailable,
    #[error("internal failure")]
    Internal,
}

impl fmt::Debug for LegacyVideoPropertiesErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

pub struct LegacyVideoPropertiesAdapterV1<'port, Port> {
    port: &'port Port,
}

impl<'port, Port> LegacyVideoPropertiesAdapterV1<'port, Port>
where
    Port: LegacyVideoPropertiesAtomicPortV1,
{
    #[must_use]
    pub const fn new(port: &'port Port) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        request: LegacyVideoPropertiesRequestV1,
    ) -> Result<LegacyVideoPropertiesAtomicOutcomeV1, LegacyVideoPropertiesErrorV1> {
        let command = prepare(request)?;
        self.port.execute(command).await.map_err(map_atomic_error)
    }
}

fn prepare(
    request: LegacyVideoPropertiesRequestV1,
) -> Result<LegacyVideoPropertiesCommandV1, LegacyVideoPropertiesErrorV1> {
    let surface = request.input.surface();
    let actor_id = if surface == LegacyVideoPropertiesSurfaceV1::VerifyPassword {
        if request.credential != LegacyVideoPropertiesCredentialV1::Anonymous
            || request.actor_id.is_some()
        {
            return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
        }
        None
    } else {
        if request.credential == LegacyVideoPropertiesCredentialV1::Anonymous {
            return Err(LegacyVideoPropertiesErrorV1::Unauthorized);
        }
        Some(
            request
                .actor_id
                .ok_or(LegacyVideoPropertiesErrorV1::Unauthorized)?,
        )
    };
    let legacy_video_id = request.input.legacy_video_id().to_owned();
    if legacy_video_id.is_empty() {
        return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
    }
    let video_id = mapped_video_id(&legacy_video_id)?;
    let mutation = normalize_input(request.input)?;
    let operation_id = OrganizationOperationId::new();
    let idempotency_key = request
        .idempotency_key
        .map_or_else(
            || IdempotencyKey::parse(format!("video-property-auto:{operation_id}")),
            IdempotencyKey::parse,
        )
        .map_err(|_| LegacyVideoPropertiesErrorV1::InvalidInput)?;
    let request_digest = request_fingerprint(surface, actor_id, &legacy_video_id, &mutation)?;
    Ok(LegacyVideoPropertiesCommandV1 {
        operation_id,
        idempotency_key,
        request_digest,
        surface,
        actor_id,
        legacy_video_id,
        video_id,
        mutation,
    })
}

fn normalize_input(
    input: LegacyVideoPropertiesInputV1,
) -> Result<LegacyVideoPropertyMutationV1, LegacyVideoPropertiesErrorV1> {
    match input {
        LegacyVideoPropertiesInputV1::MobilePassword { password, .. } => {
            Ok(LegacyVideoPropertyMutationV1::MobilePassword {
                password: password
                    .map(|value| trim_ecmascript(&value).to_owned())
                    .filter(|value| !value.is_empty()),
            })
        }
        LegacyVideoPropertiesInputV1::MobileSharing { public, .. } => {
            Ok(LegacyVideoPropertyMutationV1::MobileSharing { public })
        }
        LegacyVideoPropertiesInputV1::MobileTitle { title, .. } => {
            let title = trim_ecmascript(&title).to_owned();
            if title.is_empty() {
                return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
            }
            validate_title_bound(&title)?;
            Ok(LegacyVideoPropertyMutationV1::MobileTitle { title })
        }
        LegacyVideoPropertiesInputV1::MetadataPut { metadata, .. } => {
            if !javascript_truthy_json(&metadata) {
                return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
            }
            Ok(LegacyVideoPropertyMutationV1::MetadataReplace { metadata })
        }
        LegacyVideoPropertiesInputV1::EditDate { date, now_ms, .. } => {
            let parsed = parse_safe_iso_millis(&date)
                .ok_or(LegacyVideoPropertiesErrorV1::UnsupportedDate)?;
            if parsed > now_ms {
                return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
            }
            Ok(LegacyVideoPropertyMutationV1::MetadataCustomDate {
                custom_created_at: date,
            })
        }
        LegacyVideoPropertiesInputV1::EditTitle { title, .. } => {
            if title.is_empty() {
                return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
            }
            validate_title_bound(&title)?;
            Ok(LegacyVideoPropertyMutationV1::BrowserTitle { title })
        }
        LegacyVideoPropertiesInputV1::RemovePassword { .. } => {
            Ok(LegacyVideoPropertyMutationV1::RemovePassword)
        }
        LegacyVideoPropertiesInputV1::SetPassword { password, .. } => {
            Ok(LegacyVideoPropertyMutationV1::SetPassword { password })
        }
        LegacyVideoPropertiesInputV1::VerifyPassword { password, .. } => {
            Ok(LegacyVideoPropertyMutationV1::VerifyPassword { password })
        }
        LegacyVideoPropertiesInputV1::UpdateSettings { settings, .. } => {
            if !javascript_truthy_json(&settings) {
                return Err(LegacyVideoPropertiesErrorV1::InvalidInput);
            }
            Ok(LegacyVideoPropertyMutationV1::SettingsReplace {
                settings: normalize_settings(settings),
            })
        }
    }
}

fn validate_title_bound(title: &str) -> Result<(), LegacyVideoPropertiesErrorV1> {
    if title.chars().count() > LEGACY_VIDEO_TITLE_MAX_CHARACTERS {
        return Err(LegacyVideoPropertiesErrorV1::Database);
    }
    Ok(())
}

#[must_use]
pub fn normalize_playback_speed(value: &Value) -> f64 {
    const SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.2, 1.5, 1.75, 2.0];
    let Some(value) = value
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return 1.2;
    };
    let mut closest = SPEEDS[0];
    let mut smallest_delta = (value - closest).abs();
    for speed in SPEEDS {
        let delta = (value - speed).abs();
        if delta + f64::EPSILON < smallest_delta {
            smallest_delta = delta;
            closest = speed;
        }
    }
    closest
}

fn normalize_settings(mut settings: Value) -> Value {
    if let Value::Object(object) = &mut settings
        && let Some(value) = object.get("defaultPlaybackSpeed")
    {
        let normalized = normalize_playback_speed(value);
        if let Some(number) = Number::from_f64(normalized) {
            object.insert("defaultPlaybackSpeed".into(), Value::Number(number));
        }
    }
    settings
}

#[must_use]
pub fn javascript_object_spread(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        Value::Array(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value))
            .collect(),
        Value::String(value) => value
            .chars()
            .enumerate()
            .map(|(index, value)| (index.to_string(), Value::String(value.to_string())))
            .collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => Map::new(),
    }
}

fn javascript_truthy_json(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn request_fingerprint(
    surface: LegacyVideoPropertiesSurfaceV1,
    actor_id: Option<UserId>,
    legacy_video_id: &str,
    mutation: &LegacyVideoPropertyMutationV1,
) -> Result<String, LegacyVideoPropertiesErrorV1> {
    let payload = match mutation {
        LegacyVideoPropertyMutationV1::MobilePassword { password } => format!(
            "password:{}",
            password
                .as_deref()
                .map(password_digest)
                .unwrap_or_else(|| "clear".into())
        ),
        LegacyVideoPropertyMutationV1::MobileSharing { public } => format!("public:{public}"),
        LegacyVideoPropertyMutationV1::MobileTitle { title }
        | LegacyVideoPropertyMutationV1::BrowserTitle { title } => format!("title:{title}"),
        LegacyVideoPropertyMutationV1::MetadataReplace { metadata } => format!(
            "metadata:{}",
            serde_json::to_string(metadata)
                .map_err(|_| LegacyVideoPropertiesErrorV1::InvalidInput)?
        ),
        LegacyVideoPropertyMutationV1::MetadataCustomDate { custom_created_at } => {
            format!("date:{custom_created_at}")
        }
        LegacyVideoPropertyMutationV1::RemovePassword => "remove-password".into(),
        LegacyVideoPropertyMutationV1::SetPassword { password }
        | LegacyVideoPropertyMutationV1::VerifyPassword { password } => {
            format!("password:{}", password_digest(password))
        }
        LegacyVideoPropertyMutationV1::SettingsReplace { settings } => format!(
            "settings:{}",
            serde_json::to_string(settings)
                .map_err(|_| LegacyVideoPropertiesErrorV1::InvalidInput)?
        ),
    };
    let mut digest = Sha256::new();
    for value in [
        surface.stable_code(),
        actor_id
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
            .unwrap_or("anonymous"),
        legacy_video_id,
        &payload,
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    Ok(lower_hex(&digest.finalize()))
}

fn password_digest(password: &str) -> String {
    lower_hex(&Sha256::digest(password.as_bytes()))
}

fn mapped_video_id(value: &str) -> Result<VideoId, LegacyVideoPropertiesErrorV1> {
    if let Ok(value) = VideoId::from_str(value) {
        return Ok(value);
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
    let encoded = format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes(
            bytes[0..4]
                .try_into()
                .map_err(|_| LegacyVideoPropertiesErrorV1::Internal)?
        ),
        u16::from_be_bytes(
            bytes[4..6]
                .try_into()
                .map_err(|_| LegacyVideoPropertiesErrorV1::Internal)?
        ),
        u16::from_be_bytes(
            bytes[6..8]
                .try_into()
                .map_err(|_| LegacyVideoPropertiesErrorV1::Internal)?
        ),
        u16::from_be_bytes(
            bytes[8..10]
                .try_into()
                .map_err(|_| LegacyVideoPropertiesErrorV1::Internal)?
        ),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    );
    VideoId::from_str(&encoded).map_err(|_| LegacyVideoPropertiesErrorV1::Internal)
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

fn parse_safe_iso_millis(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
    {
        return None;
    }
    let number = |start: usize, end: usize| -> Option<i64> {
        bytes
            .get(start..end)?
            .iter()
            .try_fold(0_i64, |value, byte| {
                byte.is_ascii_digit()
                    .then_some(value * 10 + i64::from(byte - b'0'))
            })
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    let millis = number(20, 23)?;
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
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
        .checked_add(hour * 3_600_000 + minute * 60_000 + second * 1_000 + millis)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn map_atomic_error(error: LegacyVideoPropertiesAtomicErrorV1) -> LegacyVideoPropertiesErrorV1 {
    match error {
        LegacyVideoPropertiesAtomicErrorV1::TargetMissing => LegacyVideoPropertiesErrorV1::NotFound,
        LegacyVideoPropertiesAtomicErrorV1::AccessDenied => {
            LegacyVideoPropertiesErrorV1::AccessDenied
        }
        LegacyVideoPropertiesAtomicErrorV1::StaleAuthority
        | LegacyVideoPropertiesAtomicErrorV1::IdempotencyConflict => {
            LegacyVideoPropertiesErrorV1::Conflict
        }
        LegacyVideoPropertiesAtomicErrorV1::Database => LegacyVideoPropertiesErrorV1::Database,
        LegacyVideoPropertiesAtomicErrorV1::Crypto => LegacyVideoPropertiesErrorV1::PasswordFailure,
        LegacyVideoPropertiesAtomicErrorV1::Unavailable => {
            LegacyVideoPropertiesErrorV1::Unavailable
        }
        LegacyVideoPropertiesAtomicErrorV1::Corrupt => LegacyVideoPropertiesErrorV1::Internal,
    }
}

#[must_use]
pub const fn password_iterations() -> NonZeroU32 {
    match NonZeroU32::new(LEGACY_PASSWORD_PBKDF2_ITERATIONS) {
        Some(value) => value,
        None => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakePort {
        commands: Mutex<Vec<LegacyVideoPropertiesCommandV1>>,
    }

    #[async_trait(?Send)]
    impl LegacyVideoPropertiesAtomicPortV1 for FakePort {
        async fn execute(
            &self,
            command: LegacyVideoPropertiesCommandV1,
        ) -> Result<LegacyVideoPropertiesAtomicOutcomeV1, LegacyVideoPropertiesAtomicErrorV1>
        {
            self.commands
                .lock()
                .expect("commands")
                .push(command.clone());
            let result = match command.surface() {
                LegacyVideoPropertiesSurfaceV1::MobilePassword
                | LegacyVideoPropertiesSurfaceV1::MobileSharing
                | LegacyVideoPropertiesSurfaceV1::MobileTitle => {
                    LegacyVideoPropertiesAtomicResultV1::MobileSummary(Box::new(
                        LegacyMobileCapSummaryV1 {
                            id: command.legacy_video_id().into(),
                            share_url: format!("https://cap.so/s/{}", command.legacy_video_id()),
                            title: "Video".into(),
                            created_at: "2026-01-01T00:00:00.000Z".into(),
                            updated_at: "2026-01-01T00:00:00.000Z".into(),
                            owner_name: String::new(),
                            duration_seconds: None,
                            thumbnail_url: None,
                            folder_id: None,
                            public: true,
                            protected: false,
                            view_count: 0.0,
                            comment_count: 0.0,
                            reaction_count: 0.0,
                            upload: None,
                        },
                    ))
                }
                LegacyVideoPropertiesSurfaceV1::MetadataPut => {
                    LegacyVideoPropertiesAtomicResultV1::JsonTrue
                }
                LegacyVideoPropertiesSurfaceV1::SetPassword => {
                    LegacyVideoPropertiesAtomicResultV1::PasswordSet
                }
                LegacyVideoPropertiesSurfaceV1::RemovePassword => {
                    LegacyVideoPropertiesAtomicResultV1::PasswordRemoved
                }
                LegacyVideoPropertiesSurfaceV1::VerifyPassword => {
                    LegacyVideoPropertiesAtomicResultV1::PasswordRejected
                }
                _ => LegacyVideoPropertiesAtomicResultV1::SuccessObject,
            };
            Ok(LegacyVideoPropertiesAtomicOutcomeV1 {
                result,
                replayed: false,
            })
        }
    }

    fn request(input: LegacyVideoPropertiesInputV1) -> LegacyVideoPropertiesRequestV1 {
        let anonymous = input.surface() == LegacyVideoPropertiesSurfaceV1::VerifyPassword;
        LegacyVideoPropertiesRequestV1 {
            credential: if anonymous {
                LegacyVideoPropertiesCredentialV1::Anonymous
            } else {
                LegacyVideoPropertiesCredentialV1::Session
            },
            actor_id: (!anonymous).then(UserId::new),
            idempotency_key: None,
            input,
        }
    }

    #[tokio::test]
    async fn mobile_title_and_password_use_ecmascript_trim_without_title_metadata_flag() {
        let port = FakePort::default();
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::MobileTitle {
                legacy_video_id: "video".into(),
                title: "\u{FEFF}  Roadmap  \u{00A0}".into(),
            }))
            .await
            .expect("title");
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::MobilePassword {
                legacy_video_id: "video".into(),
                password: Some(" \u{FEFF} ".into()),
            }))
            .await
            .expect("password clear");
        let commands = port.commands.lock().expect("commands");
        assert!(
            matches!(commands[0].mutation(), LegacyVideoPropertyMutationV1::MobileTitle { title } if title == "Roadmap")
        );
        assert!(matches!(
            commands[1].mutation(),
            LegacyVideoPropertyMutationV1::MobilePassword { password: None }
        ));
        assert!(format!("{:?}", commands[1]).contains("[redacted]"));
    }

    #[tokio::test]
    async fn browser_title_and_password_preserve_whitespace_and_empty_hash_failure_reaches_port() {
        let port = FakePort::default();
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::EditTitle {
                legacy_video_id: "video".into(),
                title: "   ".into(),
            }))
            .await
            .expect("whitespace title");
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::SetPassword {
                legacy_video_id: "video".into(),
                password: "".into(),
            }))
            .await
            .expect("hash failure belongs to runtime");
        let commands = port.commands.lock().expect("commands");
        assert!(
            matches!(commands[0].mutation(), LegacyVideoPropertyMutationV1::BrowserTitle { title } if title == "   ")
        );
        assert!(
            matches!(commands[1].mutation(), LegacyVideoPropertyMutationV1::SetPassword { password } if password.is_empty())
        );
    }

    #[tokio::test]
    async fn metadata_truthiness_and_settings_replacement_preserve_unknown_runtime_keys() {
        let port = FakePort::default();
        assert_eq!(
            LegacyVideoPropertiesAdapterV1::new(&port)
                .execute(request(LegacyVideoPropertiesInputV1::MetadataPut {
                    legacy_video_id: "video".into(),
                    metadata: Value::Null
                }))
                .await,
            Err(LegacyVideoPropertiesErrorV1::InvalidInput)
        );
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::MetadataPut {
                legacy_video_id: "video".into(),
                metadata: Value::Array(vec![]),
            }))
            .await
            .expect("empty array is JavaScript truthy");
        LegacyVideoPropertiesAdapterV1::new(&port).execute(request(LegacyVideoPropertiesInputV1::UpdateSettings { legacy_video_id: "video".into(), settings: serde_json::json!({"unknown": {"kept": true}, "defaultPlaybackSpeed": 1.1}) })).await.expect("settings");
        let commands = port.commands.lock().expect("commands");
        assert!(
            matches!(commands[1].mutation(), LegacyVideoPropertyMutationV1::SettingsReplace { settings } if settings["unknown"]["kept"] == true && settings["defaultPlaybackSpeed"] == 1.0)
        );
        assert_eq!(normalize_playback_speed(&serde_json::json!(1.1)), 1.0);
        assert_eq!(normalize_playback_speed(&serde_json::json!(1.35)), 1.2);
        assert_eq!(normalize_playback_speed(&Value::Null), 1.2);
    }

    #[tokio::test]
    async fn edit_date_accepts_canonical_ui_subset_rejects_future_and_marks_human_gate() {
        let port = FakePort::default();
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::EditDate {
                legacy_video_id: "video".into(),
                date: "2025-01-02T03:04:05.006Z".into(),
                now_ms: 1_800_000_000_000,
            }))
            .await
            .expect("canonical date");
        assert_eq!(
            LegacyVideoPropertiesAdapterV1::new(&port)
                .execute(request(LegacyVideoPropertiesInputV1::EditDate {
                    legacy_video_id: "video".into(),
                    date: "January 2, 2025".into(),
                    now_ms: 1_800_000_000_000
                }))
                .await,
            Err(LegacyVideoPropertiesErrorV1::UnsupportedDate)
        );
        assert_eq!(LEGACY_VIDEO_DATE_PROTECTED_GATES, &["human_approval"]);
    }

    #[tokio::test]
    async fn verify_is_the_only_anonymous_operation_and_raw_branded_ids_reach_the_port() {
        let port = FakePort::default();
        LegacyVideoPropertiesAdapterV1::new(&port)
            .execute(request(LegacyVideoPropertiesInputV1::VerifyPassword {
                legacy_video_id: "not-a-cap-nanoid".into(),
                password: " ".into(),
            }))
            .await
            .expect("anonymous verify");
        let command = port.commands.lock().expect("commands")[0].clone();
        assert_eq!(command.legacy_video_id(), "not-a-cap-nanoid");
        assert_eq!(command.video_id().to_string().len(), 36);
        let mut invalid = request(LegacyVideoPropertiesInputV1::MobileSharing {
            legacy_video_id: "video".into(),
            public: false,
        });
        invalid.credential = LegacyVideoPropertiesCredentialV1::Anonymous;
        invalid.actor_id = None;
        assert_eq!(
            LegacyVideoPropertiesAdapterV1::new(&port)
                .execute(invalid)
                .await,
            Err(LegacyVideoPropertiesErrorV1::Unauthorized)
        );
    }

    #[test]
    fn javascript_spread_matches_object_array_and_scalar_runtime_behavior() {
        assert_eq!(
            javascript_object_spread(serde_json::json!({"x": 1}))["x"],
            1
        );
        assert_eq!(
            javascript_object_spread(serde_json::json!(["a", "b"]))["1"],
            "b"
        );
        assert_eq!(javascript_object_spread(serde_json::json!("ab"))["0"], "a");
        assert!(javascript_object_spread(serde_json::json!(42)).is_empty());
    }

    #[test]
    fn profiles_pin_ten_distinct_contracts_and_password_parameters() {
        assert_eq!(LEGACY_VIDEO_PROPERTY_PROFILES.len(), 10);
        assert_eq!(LEGACY_PASSWORD_PBKDF2_ITERATIONS, 100_000);
        assert_eq!(
            LEGACY_PASSWORD_SALT_BYTES + LEGACY_PASSWORD_DERIVED_BYTES,
            LEGACY_PASSWORD_WIRE_BYTES
        );
        assert_eq!(LEGACY_PASSWORD_BASE64_LENGTH, 64);
        assert_eq!(LEGACY_PASSWORD_COOKIE_MAX_HASHES, 10);
        assert_eq!(password_iterations().get(), 100_000);
    }
}
