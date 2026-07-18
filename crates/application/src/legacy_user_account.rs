//! Source-pinned contracts for Cap's user, onboarding, account, and devtool writes.
//!
//! These eight identities look similar at the UI but have materially different
//! observable contracts.  The name route parses before authentication and does
//! not trim.  Effect RPC preserves its success `Exit` and ignores `UserUpdate.id`.
//! Account settings distinguish omitted fields from explicit null/empty values,
//! while logout revokes every credential family atomically.  Devtools reject a
//! non-development environment before consulting authentication.
//!
//! Image updates retain Cap's provider boundary. D1 can prove every local branch,
//! including the core organization-onboarding transaction, but exact user-image
//! success requires an R2-capable port. Organization icons are best-effort and
//! therefore never turn a committed onboarding step into a failed RPC response.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    CAP_NANOID_ALPHABET, CAP_NANOID_LENGTH, IdempotencyKey, LegacyCapNanoId, OrganizationId,
    OrganizationOperationId, SessionId, SessionMutationGrantId, UserId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;

pub const LEGACY_USER_ACCOUNT_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_USER_NAME_OPERATION_ID: &str = "cap-v1-fdc3d5d49bb5ad6d";
pub const LEGACY_USER_ONBOARDING_OPERATION_ID: &str = "cap-v1-c7827a1de563f856";
pub const LEGACY_USER_UPDATE_OPERATION_ID: &str = "cap-v1-295a3eb4ba9ffe6f";
pub const LEGACY_PATCH_ACCOUNT_OPERATION_ID: &str = "cap-v1-fdf4d6473b7f6608";
pub const LEGACY_SIGN_OUT_ALL_OPERATION_ID: &str = "cap-v1-c067d69850110640";
pub const LEGACY_DEMOTE_FROM_PRO_OPERATION_ID: &str = "cap-v1-3d28eb7593bd4b1e";
pub const LEGACY_PROMOTE_TO_PRO_OPERATION_ID: &str = "cap-v1-e0040a01322ea19e";
pub const LEGACY_RESTART_ONBOARDING_OPERATION_ID: &str = "cap-v1-859bad07650343aa";

pub const LEGACY_USER_NAME_IDENTITY: &str = "/api/settings/user/name";
pub const LEGACY_USER_ONBOARDING_IDENTITY: &str = "/api/erpc#UserCompleteOnboardingStep";
pub const LEGACY_USER_UPDATE_IDENTITY: &str = "/api/erpc#UserUpdate";
pub const LEGACY_PATCH_ACCOUNT_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/settings/account/server.ts#patchAccountSettings";
pub const LEGACY_SIGN_OUT_ALL_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/settings/account/server.ts#signOutAllDevices";
pub const LEGACY_DEMOTE_FROM_PRO_IDENTITY: &str =
    "action://apps/web/app/Layout/devtoolsServer.ts#demoteFromPro";
pub const LEGACY_PROMOTE_TO_PRO_IDENTITY: &str =
    "action://apps/web/app/Layout/devtoolsServer.ts#promoteToPro";
pub const LEGACY_RESTART_ONBOARDING_IDENTITY: &str =
    "action://apps/web/app/Layout/devtoolsServer.ts#restartOnboarding";

pub const LEGACY_USER_ACCOUNT_POLICY: &str = "current_user.v1";
pub const LEGACY_USER_ACCOUNT_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_USER_ACCOUNT_TEXT_MAX_CHARACTERS: usize = 255;
pub const LEGACY_ACCOUNT_REVALIDATION_PATH: &str = "/dashboard/settings/account";
pub const LEGACY_EFFECT_RPC_EXIT_ID: &str = "Exit";
pub const LEGACY_EFFECT_RPC_SUCCESS_TAG: &str = "Success";
pub const LEGACY_EFFECT_RPC_EXIT_ID_KEY: &str = "_id";
pub const LEGACY_EFFECT_RPC_EXIT_TAG_KEY: &str = "_tag";
pub const LEGACY_ORGANIZATION_ICON_CONTENT_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/webp", "image/svg+xml"];
pub const LEGACY_USER_ACCOUNT_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_USER_ACCOUNT_IMAGE_PROTECTED_GATES: &[&str] = &["provider_execution"];
pub const LEGACY_USER_ACCOUNT_DEVTOOL_PROTECTED_GATES: &[&str] = &["human_approval"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyUserAccountSourceRoleV1 {
    Transport,
    Action,
    Contract,
    Authentication,
    Service,
    Repository,
    Schema,
    Provider,
    Identifier,
    Dependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyUserAccountSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyUserAccountSourceRoleV1,
}

const SOURCE_SCHEMA: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users+organizations+organizationMembers+sessions+authApiKeys",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyUserAccountSourceRoleV1::Schema,
};
const SOURCE_DATABASE: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/database/index.ts",
    symbol: "db",
    sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    role: LegacyUserAccountSourceRoleV1::Repository,
};
const SOURCE_SESSION: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/database/auth/session.ts",
    symbol: "getCurrentUser",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyUserAccountSourceRoleV1::Authentication,
};
const SOURCE_RPC_TRANSPORT: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "apps/web/app/api/erpc/route.ts",
    symbol: "Effect RPC JSON transport",
    sha256: "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
    role: LegacyUserAccountSourceRoleV1::Transport,
};
const SOURCE_RPC_ROOT: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-backend/src/Rpcs.ts",
    symbol: "RpcsLive+RpcAuthMiddlewareLive",
    sha256: "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    role: LegacyUserAccountSourceRoleV1::Transport,
};
const SOURCE_EFFECT_AUTH: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-backend/src/Auth.ts",
    symbol: "getCurrentUser+makeCurrentUser",
    sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    role: LegacyUserAccountSourceRoleV1::Authentication,
};
const SOURCE_EFFECT_DATABASE: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-backend/src/Database.ts",
    symbol: "Database",
    sha256: "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
    role: LegacyUserAccountSourceRoleV1::Repository,
};
const SOURCE_AUTH_CONTRACT: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-domain/src/Authentication.ts",
    symbol: "CurrentUser+RpcAuthMiddleware",
    sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    role: LegacyUserAccountSourceRoleV1::Contract,
};
const SOURCE_USER_CONTRACT: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-domain/src/User.ts",
    symbol: "UserUpdate+OnboardingStepPayload+OnboardingStepResult+UserRpcs",
    sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    role: LegacyUserAccountSourceRoleV1::Contract,
};
const SOURCE_ERROR_CONTRACT: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-domain/src/Errors.ts",
    symbol: "InternalError",
    sha256: "80493b611030104b495601652e2a87589ec9e293605ab92f4016ad38c9c67260",
    role: LegacyUserAccountSourceRoleV1::Contract,
};
const SOURCE_USER_RPCS: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-backend/src/Users/UsersRpcs.ts",
    symbol: "UsersRpcsLive",
    sha256: "7446ba17a317affa70bba61d9de7c4dad19df1999cfc362c57ce874063b4bc9b",
    role: LegacyUserAccountSourceRoleV1::Service,
};
const SOURCE_IMAGE_CONTRACT: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-domain/src/ImageUpload.ts",
    symbol: "ImageUpdatePayload+extractFileKey",
    sha256: "23b81310fbe78dad7ac94d0985518e1f3ad86926df282646ca38fd5bd547f47a",
    role: LegacyUserAccountSourceRoleV1::Contract,
};
const SOURCE_IMAGE_SERVICE: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-backend/src/ImageUploads/index.ts",
    symbol: "ImageUploads.applyUpdate",
    sha256: "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
    role: LegacyUserAccountSourceRoleV1::Provider,
};
const SOURCE_S3_SERVICE: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "packages/web-backend/src/S3Buckets/index.ts",
    symbol: "S3Buckets.getBucketAccess",
    sha256: "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
    role: LegacyUserAccountSourceRoleV1::Provider,
};
const SOURCE_LOCK: LegacyUserAccountSourcePinV1 = LegacyUserAccountSourcePinV1 {
    path: "pnpm-lock.yaml",
    symbol: "@effect/rpc@0.71.2+effect@3.21.4",
    sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    role: LegacyUserAccountSourceRoleV1::Dependency,
};

pub const LEGACY_USER_NAME_SOURCES: &[LegacyUserAccountSourcePinV1] = &[
    LegacyUserAccountSourcePinV1 {
        path: "apps/web/app/api/settings/user/name/route.ts",
        symbol: "POST",
        sha256: "0185e704e578084d1b1ab63b012a26cda5f0e64af098ba1ccf39cb33dadeefd6",
        role: LegacyUserAccountSourceRoleV1::Transport,
    },
    SOURCE_DATABASE,
    SOURCE_SESSION,
    SOURCE_SCHEMA,
];

pub const LEGACY_USER_ONBOARDING_SOURCES: &[LegacyUserAccountSourcePinV1] = &[
    SOURCE_RPC_TRANSPORT,
    SOURCE_RPC_ROOT,
    SOURCE_EFFECT_AUTH,
    SOURCE_EFFECT_DATABASE,
    SOURCE_AUTH_CONTRACT,
    SOURCE_USER_CONTRACT,
    SOURCE_ERROR_CONTRACT,
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        symbol: "OrganisationId",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
        role: LegacyUserAccountSourceRoleV1::Contract,
    },
    SOURCE_USER_RPCS,
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-backend/src/Users/UsersOnboarding.ts",
        symbol: "UsersOnboarding",
        sha256: "fb64431395e35b1ecc2901a8a1541922e98700d6e5f17b8dd907fcc4dc94dc82",
        role: LegacyUserAccountSourceRoleV1::Service,
    },
    SOURCE_IMAGE_CONTRACT,
    SOURCE_IMAGE_SERVICE,
    SOURCE_S3_SERVICE,
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-backend/src/S3Buckets/S3BucketsRepo.ts",
        symbol: "S3BucketsRepo",
        sha256: "efd7081204c829384bdc13d04295364d99c3c0cb6400821df06a993c55caffba",
        role: LegacyUserAccountSourceRoleV1::Provider,
    },
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        symbol: "S3BucketAccess",
        sha256: "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
        role: LegacyUserAccountSourceRoleV1::Provider,
    },
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-backend/src/S3Buckets/S3BucketClientProvider.ts",
        symbol: "S3BucketClientProvider",
        sha256: "d715478d0b5a9981315259e0dd9ddf03273a075a01a9ea685facd6d0ab75242a",
        role: LegacyUserAccountSourceRoleV1::Provider,
    },
    SOURCE_SCHEMA,
    LegacyUserAccountSourcePinV1 {
        path: "packages/database/helpers.ts",
        symbol: "nanoId",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyUserAccountSourceRoleV1::Identifier,
    },
    SOURCE_LOCK,
];

pub const LEGACY_USER_UPDATE_SOURCES: &[LegacyUserAccountSourcePinV1] = &[
    SOURCE_RPC_TRANSPORT,
    SOURCE_RPC_ROOT,
    SOURCE_EFFECT_AUTH,
    SOURCE_EFFECT_DATABASE,
    SOURCE_AUTH_CONTRACT,
    SOURCE_USER_CONTRACT,
    SOURCE_ERROR_CONTRACT,
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-domain/src/Policy.ts",
        symbol: "PolicyDeniedError",
        sha256: "0621949aa1f994836d0d168b39dc3aada3ad0478052b712de564b105c94ebe5c",
        role: LegacyUserAccountSourceRoleV1::Contract,
    },
    SOURCE_USER_RPCS,
    LegacyUserAccountSourcePinV1 {
        path: "packages/web-backend/src/Users/index.ts",
        symbol: "Users.update",
        sha256: "6e992f04942fca3647e7b9673d7fff087fad59e905d08edd1b8ef39579a808a4",
        role: LegacyUserAccountSourceRoleV1::Service,
    },
    SOURCE_IMAGE_CONTRACT,
    SOURCE_IMAGE_SERVICE,
    SOURCE_S3_SERVICE,
    SOURCE_SCHEMA,
    SOURCE_LOCK,
];

pub const LEGACY_ACCOUNT_ACTION_SOURCES: &[LegacyUserAccountSourcePinV1] = &[
    LegacyUserAccountSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/settings/account/server.ts",
        symbol: "patchAccountSettings+signOutAllDevices",
        sha256: "87980903a1f08bc10d826529aee964db5fdc8832cdf58af8aac4aec8d63e3d7c",
        role: LegacyUserAccountSourceRoleV1::Action,
    },
    SOURCE_DATABASE,
    SOURCE_SESSION,
    SOURCE_SCHEMA,
];

pub const LEGACY_DEVTOOL_ACTION_SOURCES: &[LegacyUserAccountSourcePinV1] = &[
    LegacyUserAccountSourcePinV1 {
        path: "apps/web/app/Layout/devtoolsServer.ts",
        symbol: "promoteToPro+demoteFromPro+restartOnboarding",
        sha256: "04b103a4435195608fbe7e6476b5b486ea114530da073d4db553351a76d18343",
        role: LegacyUserAccountSourceRoleV1::Action,
    },
    SOURCE_DATABASE,
    SOURCE_SESSION,
    SOURCE_SCHEMA,
];

// Canonical SHA-256 of the sorted `{path,symbol,sha256}` source rows.
pub const LEGACY_USER_NAME_SOURCE_MANIFEST_SHA256: &str =
    "8bb2b5b6398a8d197418c621a5de8fdded75dc397fdb79e5d655df839861a197";
pub const LEGACY_USER_ONBOARDING_SOURCE_MANIFEST_SHA256: &str =
    "1005767c78e0243c7a9909fb9ff75f03104f317d9c4176cda437fe51c87a8cdb";
pub const LEGACY_USER_UPDATE_SOURCE_MANIFEST_SHA256: &str =
    "1600e83d8be61c1613fba8181414bcb6e3546edd0f4454da8aec39536cc20dfc";
pub const LEGACY_ACCOUNT_ACTION_SOURCE_MANIFEST_SHA256: &str =
    "448bdcde83147aefc1c713367f96fe17d8566b3798d3d85e056e71e2a7159554";
pub const LEGACY_DEVTOOL_ACTION_SOURCE_MANIFEST_SHA256: &str =
    "0dd6130474181804819d88cca837f652e7bcd633f2530fe6161b71ec27b796af";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyUserAccountSurfaceV1 {
    NameRoute,
    CompleteOnboardingStep,
    UserUpdate,
    PatchAccountSettings,
    SignOutAllDevices,
    DemoteFromPro,
    PromoteToPro,
    RestartOnboarding,
}

impl LegacyUserAccountSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::NameRoute => LEGACY_USER_NAME_OPERATION_ID,
            Self::CompleteOnboardingStep => LEGACY_USER_ONBOARDING_OPERATION_ID,
            Self::UserUpdate => LEGACY_USER_UPDATE_OPERATION_ID,
            Self::PatchAccountSettings => LEGACY_PATCH_ACCOUNT_OPERATION_ID,
            Self::SignOutAllDevices => LEGACY_SIGN_OUT_ALL_OPERATION_ID,
            Self::DemoteFromPro => LEGACY_DEMOTE_FROM_PRO_OPERATION_ID,
            Self::PromoteToPro => LEGACY_PROMOTE_TO_PRO_OPERATION_ID,
            Self::RestartOnboarding => LEGACY_RESTART_ONBOARDING_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::NameRoute => "name_route",
            Self::CompleteOnboardingStep => "complete_onboarding_step",
            Self::UserUpdate => "user_update",
            Self::PatchAccountSettings => "patch_account_settings",
            Self::SignOutAllDevices => "sign_out_all_devices",
            Self::DemoteFromPro => "demote_from_pro",
            Self::PromoteToPro => "promote_to_pro",
            Self::RestartOnboarding => "restart_onboarding",
        }
    }

    #[must_use]
    pub const fn is_devtool(self) -> bool {
        matches!(
            self,
            Self::DemoteFromPro | Self::PromoteToPro | Self::RestartOnboarding
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyUserAccountObservedSuccessV1 {
    JsonTrue,
    EffectRpcOnboardingExit,
    EffectRpcVoidExit,
    ServerActionVoid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyUserAccountProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub sources: &'static [LegacyUserAccountSourcePinV1],
    pub source_manifest_sha256: &'static str,
    pub authentication: &'static str,
    pub policy: &'static str,
    pub observed_success: LegacyUserAccountObservedSuccessV1,
    pub parses_body_before_authentication: bool,
    pub environment_guard_before_authentication: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

macro_rules! profile {
    ($surface:ident, $kind:literal, $method:literal, $identity:expr, $sources:expr,
     $manifest:expr, $success:ident, $parse_first:expr, $env_first:expr, $gates:expr,
     $promoted:expr) => {
        LegacyUserAccountProfileV1 {
            operation_id: LegacyUserAccountSurfaceV1::$surface.operation_id(),
            kind: $kind,
            method: $method,
            legacy_identity: $identity,
            pinned_commit: LEGACY_USER_ACCOUNT_CAP_COMMIT,
            sources: $sources,
            source_manifest_sha256: $manifest,
            authentication: "session",
            policy: LEGACY_USER_ACCOUNT_POLICY,
            observed_success: LegacyUserAccountObservedSuccessV1::$success,
            parses_body_before_authentication: $parse_first,
            environment_guard_before_authentication: $env_first,
            protected_gates: $gates,
            production_promoted: $promoted,
        }
    };
}

pub const LEGACY_USER_ACCOUNT_PROFILES: &[LegacyUserAccountProfileV1] = &[
    profile!(
        NameRoute,
        "route",
        "POST",
        LEGACY_USER_NAME_IDENTITY,
        LEGACY_USER_NAME_SOURCES,
        LEGACY_USER_NAME_SOURCE_MANIFEST_SHA256,
        JsonTrue,
        true,
        false,
        LEGACY_USER_ACCOUNT_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        CompleteOnboardingStep,
        "rpc",
        "RPC",
        LEGACY_USER_ONBOARDING_IDENTITY,
        LEGACY_USER_ONBOARDING_SOURCES,
        LEGACY_USER_ONBOARDING_SOURCE_MANIFEST_SHA256,
        EffectRpcOnboardingExit,
        false,
        false,
        LEGACY_USER_ACCOUNT_IMAGE_PROTECTED_GATES,
        false
    ),
    profile!(
        UserUpdate,
        "rpc",
        "RPC",
        LEGACY_USER_UPDATE_IDENTITY,
        LEGACY_USER_UPDATE_SOURCES,
        LEGACY_USER_UPDATE_SOURCE_MANIFEST_SHA256,
        EffectRpcVoidExit,
        false,
        false,
        LEGACY_USER_ACCOUNT_IMAGE_PROTECTED_GATES,
        false
    ),
    profile!(
        PatchAccountSettings,
        "server_action",
        "ACTION",
        LEGACY_PATCH_ACCOUNT_IDENTITY,
        LEGACY_ACCOUNT_ACTION_SOURCES,
        LEGACY_ACCOUNT_ACTION_SOURCE_MANIFEST_SHA256,
        ServerActionVoid,
        false,
        false,
        LEGACY_USER_ACCOUNT_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        SignOutAllDevices,
        "server_action",
        "ACTION",
        LEGACY_SIGN_OUT_ALL_IDENTITY,
        LEGACY_ACCOUNT_ACTION_SOURCES,
        LEGACY_ACCOUNT_ACTION_SOURCE_MANIFEST_SHA256,
        ServerActionVoid,
        false,
        false,
        LEGACY_USER_ACCOUNT_NO_PROTECTED_GATES,
        true
    ),
    profile!(
        DemoteFromPro,
        "server_action",
        "ACTION",
        LEGACY_DEMOTE_FROM_PRO_IDENTITY,
        LEGACY_DEVTOOL_ACTION_SOURCES,
        LEGACY_DEVTOOL_ACTION_SOURCE_MANIFEST_SHA256,
        ServerActionVoid,
        false,
        true,
        LEGACY_USER_ACCOUNT_DEVTOOL_PROTECTED_GATES,
        false
    ),
    profile!(
        PromoteToPro,
        "server_action",
        "ACTION",
        LEGACY_PROMOTE_TO_PRO_IDENTITY,
        LEGACY_DEVTOOL_ACTION_SOURCES,
        LEGACY_DEVTOOL_ACTION_SOURCE_MANIFEST_SHA256,
        ServerActionVoid,
        false,
        true,
        LEGACY_USER_ACCOUNT_DEVTOOL_PROTECTED_GATES,
        false
    ),
    profile!(
        RestartOnboarding,
        "server_action",
        "ACTION",
        LEGACY_RESTART_ONBOARDING_IDENTITY,
        LEGACY_DEVTOOL_ACTION_SOURCES,
        LEGACY_DEVTOOL_ACTION_SOURCE_MANIFEST_SHA256,
        ServerActionVoid,
        false,
        true,
        LEGACY_USER_ACCOUNT_DEVTOOL_PROTECTED_GATES,
        false
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyUserAccountEnvironmentV1 {
    Development,
    Production,
}

/// JavaScript field presence is observable: `Absent` is filtered by Drizzle,
/// `Null` is persisted, and `Value("")` remains an empty string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyNullableTextPatchV1 {
    Absent,
    Null,
    Value(String),
}

impl LegacyNullableTextPatchV1 {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyImageBytesV1 {
    pub data: Vec<u8>,
    pub content_type: String,
    pub file_name: String,
}

impl fmt::Debug for LegacyImageBytesV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyImageBytesV1")
            .field("bytes", &self.data.len())
            .field("content_type", &"<redacted>")
            .field("file_name", &"<redacted>")
            .finish()
    }
}

impl LegacyImageBytesV1 {
    /// Cap uses `fileName.split(".").pop() || "jpg"`, independently of MIME.
    #[must_use]
    pub fn source_file_extension(&self) -> &str {
        self.file_name
            .rsplit('.')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or("jpg")
    }

    #[must_use]
    pub fn organization_icon_content_type_allowed(&self) -> bool {
        LEGACY_ORGANIZATION_ICON_CONTENT_TYPES.contains(&self.content_type.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyOptionalImageUpdateV1 {
    Absent,
    None,
    Some(LegacyImageBytesV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyOnboardingStepInputV1 {
    Welcome {
        first_name: String,
        last_name: Option<String>,
    },
    OrganizationSetup {
        organization_name: String,
        organization_icon: Option<LegacyImageBytesV1>,
    },
    CustomDomain,
    InviteTeam,
    SkipToDashboard,
}

impl LegacyOnboardingStepInputV1 {
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::Welcome { .. } => "welcome",
            Self::OrganizationSetup { .. } => "organizationSetup",
            Self::CustomDomain => "customDomain",
            Self::InviteTeam => "inviteTeam",
            Self::SkipToDashboard => "skipToDashboard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyUserAccountInputV1 {
    NameRoute {
        first_name: LegacyNullableTextPatchV1,
        last_name: LegacyNullableTextPatchV1,
    },
    CompleteOnboardingStep {
        step: LegacyOnboardingStepInputV1,
        active_organization_legacy_id: Option<String>,
        default_organization_legacy_id: Option<String>,
    },
    UserUpdate {
        payload_id: String,
        image: LegacyOptionalImageUpdateV1,
    },
    PatchAccountSettings {
        first_name: LegacyNullableTextPatchV1,
        last_name: LegacyNullableTextPatchV1,
        default_organization_legacy_id: Option<String>,
    },
    SignOutAllDevices,
    DemoteFromPro,
    PromoteToPro,
    RestartOnboarding,
}

impl LegacyUserAccountInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyUserAccountSurfaceV1 {
        match self {
            Self::NameRoute { .. } => LegacyUserAccountSurfaceV1::NameRoute,
            Self::CompleteOnboardingStep { .. } => {
                LegacyUserAccountSurfaceV1::CompleteOnboardingStep
            }
            Self::UserUpdate { .. } => LegacyUserAccountSurfaceV1::UserUpdate,
            Self::PatchAccountSettings { .. } => LegacyUserAccountSurfaceV1::PatchAccountSettings,
            Self::SignOutAllDevices => LegacyUserAccountSurfaceV1::SignOutAllDevices,
            Self::DemoteFromPro => LegacyUserAccountSurfaceV1::DemoteFromPro,
            Self::PromoteToPro => LegacyUserAccountSurfaceV1::PromoteToPro,
            Self::RestartOnboarding => LegacyUserAccountSurfaceV1::RestartOnboarding,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyUserAccountRequestV1 {
    pub environment: LegacyUserAccountEnvironmentV1,
    pub actor_id: Option<UserId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyUserAccountInputV1,
}

impl fmt::Debug for LegacyUserAccountRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyUserAccountRequestV1")
            .field("environment", &self.environment)
            .field("actor", &self.actor_id.map(|_| "<redacted>"))
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("surface", &self.input.surface())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyOrganizationIdHintV1 {
    pub legacy_id: LegacyCapNanoId,
    pub organization_id: OrganizationId,
}

/// One-use browser authority carried only by the authenticated compatibility
/// action ingress. HTTP and Effect-RPC carriers never manufacture this fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyUserAccountBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyUserAccountBrowserFenceV1 {
    #[must_use]
    pub fn from_validated_proof(proof: &ValidatedBrowserMutationProof) -> Self {
        Self {
            mutation_grant_id: proof.mutation_grant_id(),
            session_id: proof.session_id(),
            actor_id: proof.user_id(),
        }
    }

    #[must_use]
    pub const fn mutation_grant_id(self) -> SessionMutationGrantId {
        self.mutation_grant_id
    }

    #[must_use]
    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn actor_id(self) -> UserId {
        self.actor_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyUserAccountMutationV1 {
    NameRoute {
        first_name: LegacyNullableTextPatchV1,
        last_name: LegacyNullableTextPatchV1,
    },
    Welcome {
        first_name: String,
        last_name: String,
    },
    OrganizationSetup {
        organization_name: String,
        fallback_legacy_id: LegacyCapNanoId,
        fallback_organization_id: OrganizationId,
        active_hint: Option<LegacyOrganizationIdHintV1>,
        default_hint: Option<LegacyOrganizationIdHintV1>,
        organization_icon: Option<LegacyImageBytesV1>,
    },
    CustomDomain,
    InviteTeam,
    SkipToDashboard {
        fallback_legacy_id: LegacyCapNanoId,
        fallback_organization_id: OrganizationId,
        active_hint: Option<LegacyOrganizationIdHintV1>,
    },
    UserImageAbsent,
    UserImageClear,
    UserImageSome(LegacyImageBytesV1),
    PatchAccountSettings {
        first_name: LegacyNullableTextPatchV1,
        last_name: LegacyNullableTextPatchV1,
        default_organization_id: Option<OrganizationId>,
    },
    SignOutAllDevices,
    DemoteFromPro,
    PromoteToPro,
    RestartOnboarding,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyUserAccountCommandV1 {
    operation_id: OrganizationOperationId,
    surface: LegacyUserAccountSurfaceV1,
    actor_id: UserId,
    idempotency_key: IdempotencyKey,
    request_digest: [u8; 32],
    mutation: LegacyUserAccountMutationV1,
    browser_fence: Option<LegacyUserAccountBrowserFenceV1>,
}

impl LegacyUserAccountCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> OrganizationOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn surface(&self) -> LegacyUserAccountSurfaceV1 {
        self.surface
    }

    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    #[must_use]
    pub const fn mutation(&self) -> &LegacyUserAccountMutationV1 {
        &self.mutation
    }

    #[must_use]
    pub const fn browser_fence(&self) -> Option<LegacyUserAccountBrowserFenceV1> {
        self.browser_fence
    }

    #[must_use]
    pub fn request_digest_hex(&self) -> String {
        hex(&self.request_digest)
    }

    #[must_use]
    pub fn idempotency_key_digest_hex(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-legacy-user-account-key-v1\0");
        digest.update(self.idempotency_key.expose().as_bytes());
        hex(&digest.finalize())
    }
}

impl fmt::Debug for LegacyUserAccountCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyUserAccountCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.surface)
            .field("actor", &"<redacted>")
            .field("request_digest", &"<redacted>")
            .field("mutation", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyUserAccountProviderEffectV1 {
    NotRequested,
    Applied,
    BestEffortFailed,
    BestEffortProtectedGate,
}

impl LegacyUserAccountProviderEffectV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::Applied => "applied",
            Self::BestEffortFailed => "best_effort_failed",
            Self::BestEffortProtectedGate => "best_effort_protected_gate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyUserAccountMutationResultV1 {
    JsonTrue,
    Onboarding { step: LegacyOnboardingStepResultV1 },
    RpcVoid,
    ServerActionVoid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyOnboardingStepResultV1 {
    Welcome,
    OrganizationSetup {
        legacy_organization_id: LegacyCapNanoId,
    },
    CustomDomain,
    InviteTeam,
    SkipToDashboard,
}

impl LegacyOnboardingStepResultV1 {
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::OrganizationSetup { .. } => "organizationSetup",
            Self::CustomDomain => "customDomain",
            Self::InviteTeam => "inviteTeam",
            Self::SkipToDashboard => "skipToDashboard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyUserAccountAtomicOutcomeV1 {
    pub result: LegacyUserAccountMutationResultV1,
    pub provider_effect: LegacyUserAccountProviderEffectV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyEffectRpcSuccessEnvelopeV1<T> {
    pub id_key: &'static str,
    pub tag_key: &'static str,
    pub id: &'static str,
    pub tag: &'static str,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyEffectRpcVoidV1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyUserAccountSuccessV1 {
    JsonTrue {
        status: u16,
        body: bool,
        replayed: bool,
    },
    OnboardingExit {
        envelope: LegacyEffectRpcSuccessEnvelopeV1<LegacyOnboardingStepResultV1>,
        provider_effect: LegacyUserAccountProviderEffectV1,
        replayed: bool,
    },
    RpcVoidExit {
        envelope: LegacyEffectRpcSuccessEnvelopeV1<LegacyEffectRpcVoidV1>,
        replayed: bool,
    },
    ServerActionVoid {
        revalidate_path: Option<&'static str>,
        replayed: bool,
    },
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyUserAccountAtomicErrorV1 {
    #[error("current user authority is stale")]
    StaleAuthority,
    #[error("account action is forbidden")]
    Forbidden,
    #[error("the requested organization is unavailable")]
    OrganizationMissing,
    #[error("the legacy organization identifier projection is unavailable")]
    ProjectionUnavailable,
    #[error("an exact image effect requires the protected provider")]
    ProviderRequired,
    #[error("idempotency key was reused with another request")]
    IdempotencyConflict,
    #[error("account mutation conflicts with current state")]
    Conflict,
    #[error("account authority is unavailable")]
    Unavailable,
    #[error("account authority returned corrupt state")]
    Corrupt,
}

#[async_trait]
pub trait LegacyUserAccountAtomicPortV1: Send + Sync {
    async fn execute(
        &self,
        command: LegacyUserAccountCommandV1,
    ) -> Result<LegacyUserAccountAtomicOutcomeV1, LegacyUserAccountAtomicErrorV1>;
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyUserAccountErrorV1 {
    #[error("authentication is required")]
    Unauthorized,
    #[error("account input is invalid")]
    InvalidInput,
    #[error("account persistence constraint failed")]
    Database,
    #[error("the update contains no values")]
    EmptyPatch,
    #[error("account action is forbidden")]
    Forbidden,
    #[error("this action is development-only")]
    DevelopmentOnly,
    #[error("an exact image effect requires protected provider execution")]
    ProviderRequired,
    #[error("account request conflicts with current state")]
    Conflict,
    #[error("account authority is unavailable")]
    Unavailable,
    #[error("account action failed")]
    Internal,
}

pub struct LegacyUserAccountAdapterV1<'port, Port> {
    port: &'port Port,
}

impl<'port, Port> LegacyUserAccountAdapterV1<'port, Port>
where
    Port: LegacyUserAccountAtomicPortV1,
{
    #[must_use]
    pub const fn new(port: &'port Port) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        request: LegacyUserAccountRequestV1,
    ) -> Result<LegacyUserAccountSuccessV1, LegacyUserAccountErrorV1> {
        self.execute_with_browser_fence(request, None).await
    }

    /// Execute a server action with the same one-use browser proof that passed
    /// cookie, Origin, Fetch Metadata, CSRF, session, and generation checks.
    pub async fn execute_web_action(
        &self,
        request: LegacyUserAccountRequestV1,
        proof: &ValidatedBrowserMutationProof,
    ) -> Result<LegacyUserAccountSuccessV1, LegacyUserAccountErrorV1> {
        if !matches!(
            request.input.surface(),
            LegacyUserAccountSurfaceV1::PatchAccountSettings
                | LegacyUserAccountSurfaceV1::SignOutAllDevices
                | LegacyUserAccountSurfaceV1::DemoteFromPro
                | LegacyUserAccountSurfaceV1::PromoteToPro
                | LegacyUserAccountSurfaceV1::RestartOnboarding
        ) || request.actor_id != Some(proof.user_id())
        {
            return Err(LegacyUserAccountErrorV1::Unauthorized);
        }
        self.execute_with_browser_fence(
            request,
            Some(LegacyUserAccountBrowserFenceV1::from_validated_proof(proof)),
        )
        .await
    }

    async fn execute_with_browser_fence(
        &self,
        request: LegacyUserAccountRequestV1,
        browser_fence: Option<LegacyUserAccountBrowserFenceV1>,
    ) -> Result<LegacyUserAccountSuccessV1, LegacyUserAccountErrorV1> {
        let command = prepare(request, browser_fence)?;
        let surface = command.surface();
        let outcome = self.port.execute(command).await.map_err(map_atomic_error)?;
        project_success(surface, outcome)
    }
}

fn prepare(
    request: LegacyUserAccountRequestV1,
    browser_fence: Option<LegacyUserAccountBrowserFenceV1>,
) -> Result<LegacyUserAccountCommandV1, LegacyUserAccountErrorV1> {
    let surface = request.input.surface();
    // This ordering is source-observable for all three devtools.
    if surface.is_devtool() && request.environment != LegacyUserAccountEnvironmentV1::Development {
        return Err(LegacyUserAccountErrorV1::DevelopmentOnly);
    }
    let actor_id = request
        .actor_id
        .ok_or(LegacyUserAccountErrorV1::Unauthorized)?;
    let operation_id = OrganizationOperationId::new();
    let idempotency_key = match request.idempotency_key {
        Some(value) => {
            IdempotencyKey::parse(value).map_err(|_| LegacyUserAccountErrorV1::InvalidInput)?
        }
        None => IdempotencyKey::parse(format!("user-account-auto:{operation_id}"))
            .map_err(|_| LegacyUserAccountErrorV1::Internal)?,
    };
    let mutation = normalize(request.input, actor_id, &idempotency_key)?;
    let request_digest = command_digest(surface, actor_id, &mutation);
    Ok(LegacyUserAccountCommandV1 {
        operation_id,
        surface,
        actor_id,
        idempotency_key,
        request_digest,
        mutation,
        browser_fence,
    })
}

fn normalize(
    input: LegacyUserAccountInputV1,
    actor_id: UserId,
    idempotency_key: &IdempotencyKey,
) -> Result<LegacyUserAccountMutationV1, LegacyUserAccountErrorV1> {
    match input {
        LegacyUserAccountInputV1::NameRoute {
            first_name,
            last_name,
        } => {
            validate_patch(&first_name)?;
            validate_patch(&last_name)?;
            if first_name.is_absent() && last_name.is_absent() {
                return Err(LegacyUserAccountErrorV1::EmptyPatch);
            }
            Ok(LegacyUserAccountMutationV1::NameRoute {
                first_name,
                last_name,
            })
        }
        LegacyUserAccountInputV1::CompleteOnboardingStep {
            step,
            active_organization_legacy_id,
            default_organization_legacy_id,
        } => match step {
            LegacyOnboardingStepInputV1::Welcome {
                first_name,
                last_name,
            } => {
                let first_name = trim_ecmascript(&first_name).to_owned();
                let last_name = last_name
                    .as_deref()
                    .map(trim_ecmascript)
                    .unwrap_or("")
                    .to_owned();
                validate_text(&first_name)?;
                validate_text(&last_name)?;
                let personalized = format!("{first_name}'s Organization");
                if !first_name.is_empty()
                    && personalized.chars().count() > LEGACY_USER_ACCOUNT_TEXT_MAX_CHARACTERS
                {
                    return Err(LegacyUserAccountErrorV1::Database);
                }
                Ok(LegacyUserAccountMutationV1::Welcome {
                    first_name,
                    last_name,
                })
            }
            LegacyOnboardingStepInputV1::OrganizationSetup {
                organization_name,
                organization_icon,
            } => {
                let trimmed = trim_ecmascript(&organization_name);
                let organization_name = if trimmed.is_empty() {
                    organization_name
                } else {
                    trimmed.to_owned()
                };
                validate_text(&organization_name)?;
                let fallback_legacy_id = derive_legacy_organization_id(
                    actor_id,
                    LegacyUserAccountSurfaceV1::CompleteOnboardingStep,
                    idempotency_key,
                    b"organizationSetup",
                )?;
                let fallback_organization_id = mapped_organization(&fallback_legacy_id)?;
                Ok(LegacyUserAccountMutationV1::OrganizationSetup {
                    organization_name,
                    fallback_legacy_id,
                    fallback_organization_id,
                    active_hint: parse_hint(active_organization_legacy_id)?,
                    default_hint: parse_hint(default_organization_legacy_id)?,
                    organization_icon,
                })
            }
            LegacyOnboardingStepInputV1::CustomDomain => {
                Ok(LegacyUserAccountMutationV1::CustomDomain)
            }
            LegacyOnboardingStepInputV1::InviteTeam => Ok(LegacyUserAccountMutationV1::InviteTeam),
            LegacyOnboardingStepInputV1::SkipToDashboard => {
                let fallback_legacy_id = derive_legacy_organization_id(
                    actor_id,
                    LegacyUserAccountSurfaceV1::CompleteOnboardingStep,
                    idempotency_key,
                    b"skipToDashboard",
                )?;
                let fallback_organization_id = mapped_organization(&fallback_legacy_id)?;
                Ok(LegacyUserAccountMutationV1::SkipToDashboard {
                    fallback_legacy_id,
                    fallback_organization_id,
                    // Cap uses CurrentUser.activeOrganizationId, not the freshly
                    // selected fallback stored on the user row.
                    active_hint: parse_hint(active_organization_legacy_id)?,
                })
            }
        },
        LegacyUserAccountInputV1::UserUpdate {
            payload_id: _,
            image,
        } => match image {
            LegacyOptionalImageUpdateV1::Absent => Ok(LegacyUserAccountMutationV1::UserImageAbsent),
            LegacyOptionalImageUpdateV1::None => Ok(LegacyUserAccountMutationV1::UserImageClear),
            LegacyOptionalImageUpdateV1::Some(image) => {
                Ok(LegacyUserAccountMutationV1::UserImageSome(image))
            }
        },
        LegacyUserAccountInputV1::PatchAccountSettings {
            first_name,
            last_name,
            default_organization_legacy_id,
        } => {
            validate_patch(&first_name)?;
            validate_patch(&last_name)?;
            let default_organization_id = default_organization_legacy_id
                .map(|value| mapped_source_organization(&value))
                .transpose()?;
            if first_name.is_absent() && last_name.is_absent() && default_organization_id.is_none()
            {
                // Drizzle's mapUpdateSet filters `undefined`, then rejects an
                // empty SET list. Do not silently turn this into a success.
                return Err(LegacyUserAccountErrorV1::EmptyPatch);
            }
            Ok(LegacyUserAccountMutationV1::PatchAccountSettings {
                first_name,
                last_name,
                default_organization_id,
            })
        }
        LegacyUserAccountInputV1::SignOutAllDevices => {
            Ok(LegacyUserAccountMutationV1::SignOutAllDevices)
        }
        LegacyUserAccountInputV1::DemoteFromPro => Ok(LegacyUserAccountMutationV1::DemoteFromPro),
        LegacyUserAccountInputV1::PromoteToPro => Ok(LegacyUserAccountMutationV1::PromoteToPro),
        LegacyUserAccountInputV1::RestartOnboarding => {
            Ok(LegacyUserAccountMutationV1::RestartOnboarding)
        }
    }
}

fn validate_patch(value: &LegacyNullableTextPatchV1) -> Result<(), LegacyUserAccountErrorV1> {
    if let LegacyNullableTextPatchV1::Value(value) = value {
        validate_text(value)?;
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), LegacyUserAccountErrorV1> {
    if value.chars().count() > LEGACY_USER_ACCOUNT_TEXT_MAX_CHARACTERS {
        return Err(LegacyUserAccountErrorV1::Database);
    }
    Ok(())
}

fn parse_hint(
    value: Option<String>,
) -> Result<Option<LegacyOrganizationIdHintV1>, LegacyUserAccountErrorV1> {
    value
        .map(|value| {
            let legacy_id = LegacyCapNanoId::parse(value)
                .map_err(|_| LegacyUserAccountErrorV1::InvalidInput)?;
            let organization_id = mapped_organization(&legacy_id)?;
            Ok(LegacyOrganizationIdHintV1 {
                legacy_id,
                organization_id,
            })
        })
        .transpose()
}

fn mapped_organization(
    legacy_id: &LegacyCapNanoId,
) -> Result<OrganizationId, LegacyUserAccountErrorV1> {
    OrganizationId::parse(&legacy_id.mapped_uuid().to_string())
        .map_err(|_| LegacyUserAccountErrorV1::Internal)
}

fn mapped_source_organization(value: &str) -> Result<OrganizationId, LegacyUserAccountErrorV1> {
    if let Ok(legacy) = LegacyCapNanoId::parse(value.to_owned()) {
        return mapped_organization(&legacy);
    }
    // Effect brands and server-action TypeScript annotations do not refine at
    // runtime. Hash an opaque string into a disjoint UUIDv8 lookup key so it
    // reaches Cap's owner/member authorization result instead of a fake schema
    // rejection. Imported dirty identifiers remain behind the human gate.
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-opaque-organization-id-to-uuid-v1\0");
    digest.update(value.as_bytes());
    let digest = digest.finalize();
    let mut uuid_bytes = [0_u8; 16];
    uuid_bytes.copy_from_slice(&digest[..16]);
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x80;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    let encoded = hex(&uuid_bytes);
    let formatted = format!(
        "{}-{}-{}-{}-{}",
        &encoded[..8],
        &encoded[8..12],
        &encoded[12..16],
        &encoded[16..20],
        &encoded[20..32]
    );
    OrganizationId::parse(&formatted).map_err(|_| LegacyUserAccountErrorV1::Internal)
}

fn derive_legacy_organization_id(
    actor_id: UserId,
    surface: LegacyUserAccountSurfaceV1,
    idempotency_key: &IdempotencyKey,
    discriminator: &[u8],
) -> Result<LegacyCapNanoId, LegacyUserAccountErrorV1> {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-user-account-organization-id-v1\0");
    framed(&mut digest, actor_id.to_string().as_bytes());
    framed(&mut digest, surface.operation_id().as_bytes());
    framed(&mut digest, idempotency_key.expose().as_bytes());
    framed(&mut digest, discriminator);
    let digest = digest.finalize();
    let alphabet = CAP_NANOID_ALPHABET.as_bytes();
    let mut value = String::with_capacity(CAP_NANOID_LENGTH);
    for byte in digest.iter().take(CAP_NANOID_LENGTH) {
        value.push(char::from(alphabet[usize::from(*byte & 31)]));
    }
    LegacyCapNanoId::parse(value).map_err(|_| LegacyUserAccountErrorV1::Internal)
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

fn command_digest(
    surface: LegacyUserAccountSurfaceV1,
    actor_id: UserId,
    mutation: &LegacyUserAccountMutationV1,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-user-account-request-v1\0");
    framed(&mut digest, surface.operation_id().as_bytes());
    framed(&mut digest, actor_id.to_string().as_bytes());
    match mutation {
        LegacyUserAccountMutationV1::NameRoute {
            first_name,
            last_name,
        }
        | LegacyUserAccountMutationV1::PatchAccountSettings {
            first_name,
            last_name,
            default_organization_id: _,
        } => {
            patch_digest(&mut digest, first_name);
            patch_digest(&mut digest, last_name);
            if let LegacyUserAccountMutationV1::PatchAccountSettings {
                default_organization_id,
                ..
            } = mutation
            {
                optional_string(
                    &mut digest,
                    default_organization_id.map(|id| id.to_string()).as_deref(),
                );
            }
        }
        LegacyUserAccountMutationV1::Welcome {
            first_name,
            last_name,
        } => {
            framed(&mut digest, first_name.as_bytes());
            framed(&mut digest, last_name.as_bytes());
        }
        LegacyUserAccountMutationV1::OrganizationSetup {
            organization_name,
            fallback_legacy_id,
            active_hint,
            default_hint,
            organization_icon,
            ..
        } => {
            framed(&mut digest, organization_name.as_bytes());
            framed(&mut digest, fallback_legacy_id.as_str().as_bytes());
            hint_digest(&mut digest, active_hint.as_ref());
            hint_digest(&mut digest, default_hint.as_ref());
            image_digest(&mut digest, organization_icon.as_ref());
        }
        LegacyUserAccountMutationV1::SkipToDashboard {
            fallback_legacy_id,
            active_hint,
            ..
        } => {
            framed(&mut digest, fallback_legacy_id.as_str().as_bytes());
            hint_digest(&mut digest, active_hint.as_ref());
        }
        LegacyUserAccountMutationV1::UserImageSome(image) => {
            image_digest(&mut digest, Some(image));
        }
        LegacyUserAccountMutationV1::CustomDomain
        | LegacyUserAccountMutationV1::InviteTeam
        | LegacyUserAccountMutationV1::UserImageAbsent
        | LegacyUserAccountMutationV1::UserImageClear
        | LegacyUserAccountMutationV1::SignOutAllDevices
        | LegacyUserAccountMutationV1::DemoteFromPro
        | LegacyUserAccountMutationV1::PromoteToPro
        | LegacyUserAccountMutationV1::RestartOnboarding => {
            framed(&mut digest, surface.stable_code().as_bytes());
        }
    }
    digest.finalize().into()
}

fn patch_digest(digest: &mut Sha256, value: &LegacyNullableTextPatchV1) {
    match value {
        LegacyNullableTextPatchV1::Absent => framed(digest, b"absent"),
        LegacyNullableTextPatchV1::Null => framed(digest, b"null"),
        LegacyNullableTextPatchV1::Value(value) => framed(digest, value.as_bytes()),
    }
}

fn hint_digest(digest: &mut Sha256, value: Option<&LegacyOrganizationIdHintV1>) {
    optional_string(digest, value.map(|hint| hint.legacy_id.as_str()));
}

fn image_digest(digest: &mut Sha256, value: Option<&LegacyImageBytesV1>) {
    let Some(value) = value else {
        framed(digest, b"absent");
        return;
    };
    framed(digest, value.content_type.as_bytes());
    framed(digest, value.file_name.as_bytes());
    framed(digest, &Sha256::digest(&value.data));
}

fn optional_string(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => framed(digest, value.as_bytes()),
        None => framed(digest, b"absent"),
    }
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn map_atomic_error(error: LegacyUserAccountAtomicErrorV1) -> LegacyUserAccountErrorV1 {
    match error {
        LegacyUserAccountAtomicErrorV1::Forbidden
        | LegacyUserAccountAtomicErrorV1::OrganizationMissing => {
            LegacyUserAccountErrorV1::Forbidden
        }
        LegacyUserAccountAtomicErrorV1::ProviderRequired => {
            LegacyUserAccountErrorV1::ProviderRequired
        }
        LegacyUserAccountAtomicErrorV1::IdempotencyConflict
        | LegacyUserAccountAtomicErrorV1::Conflict
        | LegacyUserAccountAtomicErrorV1::StaleAuthority => LegacyUserAccountErrorV1::Conflict,
        LegacyUserAccountAtomicErrorV1::Unavailable => LegacyUserAccountErrorV1::Unavailable,
        LegacyUserAccountAtomicErrorV1::ProjectionUnavailable
        | LegacyUserAccountAtomicErrorV1::Corrupt => LegacyUserAccountErrorV1::Internal,
    }
}

fn project_success(
    surface: LegacyUserAccountSurfaceV1,
    outcome: LegacyUserAccountAtomicOutcomeV1,
) -> Result<LegacyUserAccountSuccessV1, LegacyUserAccountErrorV1> {
    match (surface, outcome.result) {
        (LegacyUserAccountSurfaceV1::NameRoute, LegacyUserAccountMutationResultV1::JsonTrue) => {
            Ok(LegacyUserAccountSuccessV1::JsonTrue {
                status: 200,
                body: true,
                replayed: outcome.replayed,
            })
        }
        (
            LegacyUserAccountSurfaceV1::CompleteOnboardingStep,
            LegacyUserAccountMutationResultV1::Onboarding { step },
        ) => Ok(LegacyUserAccountSuccessV1::OnboardingExit {
            envelope: LegacyEffectRpcSuccessEnvelopeV1 {
                id_key: LEGACY_EFFECT_RPC_EXIT_ID_KEY,
                tag_key: LEGACY_EFFECT_RPC_EXIT_TAG_KEY,
                id: LEGACY_EFFECT_RPC_EXIT_ID,
                tag: LEGACY_EFFECT_RPC_SUCCESS_TAG,
                value: step,
            },
            provider_effect: outcome.provider_effect,
            replayed: outcome.replayed,
        }),
        (LegacyUserAccountSurfaceV1::UserUpdate, LegacyUserAccountMutationResultV1::RpcVoid) => {
            Ok(LegacyUserAccountSuccessV1::RpcVoidExit {
                envelope: LegacyEffectRpcSuccessEnvelopeV1 {
                    id_key: LEGACY_EFFECT_RPC_EXIT_ID_KEY,
                    tag_key: LEGACY_EFFECT_RPC_EXIT_TAG_KEY,
                    id: LEGACY_EFFECT_RPC_EXIT_ID,
                    tag: LEGACY_EFFECT_RPC_SUCCESS_TAG,
                    value: LegacyEffectRpcVoidV1,
                },
                replayed: outcome.replayed,
            })
        }
        (
            LegacyUserAccountSurfaceV1::PatchAccountSettings
            | LegacyUserAccountSurfaceV1::SignOutAllDevices
            | LegacyUserAccountSurfaceV1::DemoteFromPro
            | LegacyUserAccountSurfaceV1::PromoteToPro
            | LegacyUserAccountSurfaceV1::RestartOnboarding,
            LegacyUserAccountMutationResultV1::ServerActionVoid,
        ) => Ok(LegacyUserAccountSuccessV1::ServerActionVoid {
            revalidate_path: (surface == LegacyUserAccountSurfaceV1::PatchAccountSettings)
                .then_some(LEGACY_ACCOUNT_REVALIDATION_PATH),
            replayed: outcome.replayed,
        }),
        _ => Err(LegacyUserAccountErrorV1::Internal),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct FakePort {
        commands: Mutex<Vec<LegacyUserAccountCommandV1>>,
        error: Mutex<Option<LegacyUserAccountAtomicErrorV1>>,
    }

    #[async_trait]
    impl LegacyUserAccountAtomicPortV1 for FakePort {
        async fn execute(
            &self,
            command: LegacyUserAccountCommandV1,
        ) -> Result<LegacyUserAccountAtomicOutcomeV1, LegacyUserAccountAtomicErrorV1> {
            if let Some(error) = *self.error.lock().expect("error") {
                return Err(error);
            }
            let provider_effect = match command.mutation() {
                LegacyUserAccountMutationV1::OrganizationSetup {
                    organization_icon: Some(_),
                    ..
                } => LegacyUserAccountProviderEffectV1::BestEffortProtectedGate,
                _ => LegacyUserAccountProviderEffectV1::NotRequested,
            };
            let result = match command.mutation() {
                LegacyUserAccountMutationV1::NameRoute { .. } => {
                    LegacyUserAccountMutationResultV1::JsonTrue
                }
                LegacyUserAccountMutationV1::Welcome { .. } => {
                    LegacyUserAccountMutationResultV1::Onboarding {
                        step: LegacyOnboardingStepResultV1::Welcome,
                    }
                }
                LegacyUserAccountMutationV1::OrganizationSetup {
                    fallback_legacy_id, ..
                } => LegacyUserAccountMutationResultV1::Onboarding {
                    step: LegacyOnboardingStepResultV1::OrganizationSetup {
                        legacy_organization_id: fallback_legacy_id.clone(),
                    },
                },
                LegacyUserAccountMutationV1::CustomDomain => {
                    LegacyUserAccountMutationResultV1::Onboarding {
                        step: LegacyOnboardingStepResultV1::CustomDomain,
                    }
                }
                LegacyUserAccountMutationV1::InviteTeam => {
                    LegacyUserAccountMutationResultV1::Onboarding {
                        step: LegacyOnboardingStepResultV1::InviteTeam,
                    }
                }
                LegacyUserAccountMutationV1::SkipToDashboard { .. } => {
                    LegacyUserAccountMutationResultV1::Onboarding {
                        step: LegacyOnboardingStepResultV1::SkipToDashboard,
                    }
                }
                LegacyUserAccountMutationV1::UserImageAbsent
                | LegacyUserAccountMutationV1::UserImageClear
                | LegacyUserAccountMutationV1::UserImageSome(_) => {
                    LegacyUserAccountMutationResultV1::RpcVoid
                }
                LegacyUserAccountMutationV1::PatchAccountSettings { .. }
                | LegacyUserAccountMutationV1::SignOutAllDevices
                | LegacyUserAccountMutationV1::DemoteFromPro
                | LegacyUserAccountMutationV1::PromoteToPro
                | LegacyUserAccountMutationV1::RestartOnboarding => {
                    LegacyUserAccountMutationResultV1::ServerActionVoid
                }
            };
            self.commands.lock().expect("commands").push(command);
            Ok(LegacyUserAccountAtomicOutcomeV1 {
                result,
                provider_effect,
                replayed: false,
            })
        }
    }

    fn actor() -> UserId {
        UserId::parse("00000000-0000-7000-8000-000000000001").expect("actor")
    }

    fn request(input: LegacyUserAccountInputV1) -> LegacyUserAccountRequestV1 {
        LegacyUserAccountRequestV1 {
            environment: LegacyUserAccountEnvironmentV1::Development,
            actor_id: Some(actor()),
            idempotency_key: Some("user-account-test-0001".into()),
            input,
        }
    }

    fn manifest(sources: &[LegacyUserAccountSourcePinV1]) -> String {
        let mut rows = sources
            .iter()
            .map(|source| {
                serde_json::to_string(&json!({
                    "path": source.path,
                    "sha256": source.sha256,
                    "symbol": source.symbol,
                }))
                .expect("source row")
            })
            .collect::<Vec<_>>();
        rows.sort();
        hex(&Sha256::digest(format!("[{}]", rows.join(","))))
    }

    #[test]
    fn profiles_cover_exact_inventory_and_source_observability() {
        assert_eq!(LEGACY_USER_ACCOUNT_PROFILES.len(), 8);
        assert_eq!(LEGACY_USER_NAME_SOURCES.len(), 4);
        assert_eq!(LEGACY_USER_ONBOARDING_SOURCES.len(), 19);
        assert_eq!(LEGACY_USER_UPDATE_SOURCES.len(), 15);
        assert!(LEGACY_USER_ACCOUNT_PROFILES[0].parses_body_before_authentication);
        assert!(
            LEGACY_USER_ACCOUNT_PROFILES[5..]
                .iter()
                .all(|profile| profile.environment_guard_before_authentication)
        );
        assert_eq!(
            LEGACY_USER_ACCOUNT_PROFILES
                .iter()
                .filter(|profile| profile.production_promoted)
                .count(),
            3
        );
        for (sources, expected) in [
            (
                LEGACY_USER_NAME_SOURCES,
                LEGACY_USER_NAME_SOURCE_MANIFEST_SHA256,
            ),
            (
                LEGACY_USER_ONBOARDING_SOURCES,
                LEGACY_USER_ONBOARDING_SOURCE_MANIFEST_SHA256,
            ),
            (
                LEGACY_USER_UPDATE_SOURCES,
                LEGACY_USER_UPDATE_SOURCE_MANIFEST_SHA256,
            ),
            (
                LEGACY_ACCOUNT_ACTION_SOURCES,
                LEGACY_ACCOUNT_ACTION_SOURCE_MANIFEST_SHA256,
            ),
            (
                LEGACY_DEVTOOL_ACTION_SOURCES,
                LEGACY_DEVTOOL_ACTION_SOURCE_MANIFEST_SHA256,
            ),
        ] {
            assert_eq!(manifest(sources), expected);
        }
    }

    #[tokio::test]
    async fn name_route_preserves_empty_null_and_untrimmed_values() {
        let port = FakePort::default();
        let success = LegacyUserAccountAdapterV1::new(&port)
            .execute(request(LegacyUserAccountInputV1::NameRoute {
                first_name: LegacyNullableTextPatchV1::Value("  Ada  ".into()),
                last_name: LegacyNullableTextPatchV1::Null,
            }))
            .await
            .expect("name route");
        assert_eq!(
            success,
            LegacyUserAccountSuccessV1::JsonTrue {
                status: 200,
                body: true,
                replayed: false,
            }
        );
        let commands = port.commands.lock().expect("commands");
        assert!(matches!(
            commands[0].mutation(),
            LegacyUserAccountMutationV1::NameRoute {
                first_name: LegacyNullableTextPatchV1::Value(value),
                last_name: LegacyNullableTextPatchV1::Null,
            } if value == "  Ada  "
        ));
    }

    #[tokio::test]
    async fn welcome_uses_ecmascript_trim_and_missing_last_name_defaults_empty() {
        let port = FakePort::default();
        LegacyUserAccountAdapterV1::new(&port)
            .execute(request(LegacyUserAccountInputV1::CompleteOnboardingStep {
                step: LegacyOnboardingStepInputV1::Welcome {
                    first_name: "\u{FEFF} Ada \u{00A0}".into(),
                    last_name: None,
                },
                active_organization_legacy_id: None,
                default_organization_legacy_id: None,
            }))
            .await
            .expect("welcome");
        let commands = port.commands.lock().expect("commands");
        assert!(matches!(
            commands[0].mutation(),
            LegacyUserAccountMutationV1::Welcome { first_name, last_name }
                if first_name == "Ada" && last_name.is_empty()
        ));
    }

    #[tokio::test]
    async fn organization_setup_keeps_all_whitespace_and_best_effort_icon_gate() {
        let port = FakePort::default();
        let success = LegacyUserAccountAdapterV1::new(&port)
            .execute(request(LegacyUserAccountInputV1::CompleteOnboardingStep {
                step: LegacyOnboardingStepInputV1::OrganizationSetup {
                    organization_name: "   ".into(),
                    organization_icon: Some(LegacyImageBytesV1 {
                        data: vec![1, 2, 3],
                        content_type: "image/tiff".into(),
                        file_name: "icon.tiff".into(),
                    }),
                },
                active_organization_legacy_id: None,
                default_organization_legacy_id: None,
            }))
            .await
            .expect("core onboarding succeeds despite icon provider");
        assert!(matches!(
            success,
            LegacyUserAccountSuccessV1::OnboardingExit {
                provider_effect: LegacyUserAccountProviderEffectV1::BestEffortProtectedGate,
                ..
            }
        ));
        let commands = port.commands.lock().expect("commands");
        assert!(matches!(
            commands[0].mutation(),
            LegacyUserAccountMutationV1::OrganizationSetup {
                organization_name,
                ..
            } if organization_name == "   "
        ));
    }

    #[tokio::test]
    async fn user_update_ignores_payload_id_and_preserves_option_layers() {
        let port = FakePort::default();
        for (payload_id, image) in [
            ("foreign-user", LegacyOptionalImageUpdateV1::Absent),
            ("also-foreign", LegacyOptionalImageUpdateV1::None),
        ] {
            LegacyUserAccountAdapterV1::new(&port)
                .execute(request(LegacyUserAccountInputV1::UserUpdate {
                    payload_id: payload_id.into(),
                    image,
                }))
                .await
                .expect("payload id ignored");
        }
        let commands = port.commands.lock().expect("commands");
        assert_eq!(commands[0].actor_id(), actor());
        assert!(matches!(
            commands[0].mutation(),
            LegacyUserAccountMutationV1::UserImageAbsent
        ));
        assert!(matches!(
            commands[1].mutation(),
            LegacyUserAccountMutationV1::UserImageClear
        ));
    }

    #[tokio::test]
    async fn patch_preserves_presence_and_rejects_drizzle_empty_set() {
        let port = FakePort::default();
        assert_eq!(
            LegacyUserAccountAdapterV1::new(&port)
                .execute(request(LegacyUserAccountInputV1::PatchAccountSettings {
                    first_name: LegacyNullableTextPatchV1::Absent,
                    last_name: LegacyNullableTextPatchV1::Absent,
                    default_organization_legacy_id: None,
                },))
                .await,
            Err(LegacyUserAccountErrorV1::EmptyPatch)
        );
        LegacyUserAccountAdapterV1::new(&port)
            .execute(request(LegacyUserAccountInputV1::PatchAccountSettings {
                first_name: LegacyNullableTextPatchV1::Value(String::new()),
                last_name: LegacyNullableTextPatchV1::Absent,
                default_organization_legacy_id: None,
            }))
            .await
            .expect("present empty string is a real update");
    }

    #[tokio::test]
    async fn devtool_environment_guard_precedes_authentication() {
        let port = FakePort::default();
        let mut blocked = request(LegacyUserAccountInputV1::PromoteToPro);
        blocked.environment = LegacyUserAccountEnvironmentV1::Production;
        blocked.actor_id = None;
        assert_eq!(
            LegacyUserAccountAdapterV1::new(&port)
                .execute(blocked)
                .await,
            Err(LegacyUserAccountErrorV1::DevelopmentOnly)
        );
    }

    #[tokio::test]
    async fn rpc_successes_keep_effect_exit_markers_and_onboarding_variants() {
        let port = FakePort::default();
        let success = LegacyUserAccountAdapterV1::new(&port)
            .execute(request(LegacyUserAccountInputV1::CompleteOnboardingStep {
                step: LegacyOnboardingStepInputV1::CustomDomain,
                active_organization_legacy_id: None,
                default_organization_legacy_id: None,
            }))
            .await
            .expect("rpc");
        let LegacyUserAccountSuccessV1::OnboardingExit { envelope, .. } = success else {
            panic!("onboarding exit")
        };
        assert_eq!(envelope.id, "Exit");
        assert_eq!(envelope.tag, "Success");
        assert_eq!(envelope.id_key, "_id");
        assert_eq!(envelope.tag_key, "_tag");
        assert_eq!(envelope.value, LegacyOnboardingStepResultV1::CustomDomain);
    }

    #[tokio::test]
    async fn mysql_bounds_are_database_failures_and_opaque_org_ids_reach_authority() {
        let port = FakePort::default();
        assert_eq!(
            LegacyUserAccountAdapterV1::new(&port)
                .execute(request(LegacyUserAccountInputV1::NameRoute {
                    first_name: LegacyNullableTextPatchV1::Value("😀".repeat(256)),
                    last_name: LegacyNullableTextPatchV1::Absent,
                }))
                .await,
            Err(LegacyUserAccountErrorV1::Database)
        );
        LegacyUserAccountAdapterV1::new(&port)
            .execute(request(LegacyUserAccountInputV1::PatchAccountSettings {
                first_name: LegacyNullableTextPatchV1::Absent,
                last_name: LegacyNullableTextPatchV1::Absent,
                default_organization_legacy_id: Some("opaque-brand".into()),
            }))
            .await
            .expect("opaque branded ID reaches atomic owner/member authority");
    }

    #[test]
    fn image_provider_metadata_keeps_cap_extension_and_icon_type_rules() {
        let trailing_dot = LegacyImageBytesV1 {
            data: vec![],
            content_type: "image/png".into(),
            file_name: "avatar.".into(),
        };
        assert_eq!(trailing_dot.source_file_extension(), "jpg");
        assert!(trailing_dot.organization_icon_content_type_allowed());
        let mismatched = LegacyImageBytesV1 {
            data: vec![],
            content_type: "image/tiff".into(),
            file_name: "avatar.png".into(),
        };
        assert_eq!(mismatched.source_file_extension(), "png");
        assert!(!mismatched.organization_icon_content_type_allowed());
    }

    #[test]
    fn command_debug_and_digests_do_not_expose_user_material() {
        let command = prepare(
            request(LegacyUserAccountInputV1::NameRoute {
                first_name: LegacyNullableTextPatchV1::Value("secret-name".into()),
                last_name: LegacyNullableTextPatchV1::Value("secret-last".into()),
            }),
            None,
        )
        .expect("command");
        let rendered = format!("{command:?}");
        assert!(!rendered.contains("secret-name"));
        assert!(!rendered.contains("user-account-test-0001"));
        assert_eq!(command.request_digest_hex().len(), 64);
        assert_eq!(command.idempotency_key_digest_hex().len(), 64);
    }
}
