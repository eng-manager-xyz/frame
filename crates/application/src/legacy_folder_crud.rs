//! Source-pinned contracts for Cap's mobile and Effect-RPC folder CRUD surfaces.
//!
//! The four identities share a database table but not one wire contract. The
//! mobile route trims the name, defaults the colour, creates a personal root
//! folder, and returns a folder projection. The Effect RPCs preserve explicit
//! scope/parent/public-page fields and return `void`. This module keeps those
//! differences visible while requiring a single tenant-scoped, replay-safe
//! atomic port for authority, mutation, receipt, and audit.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    CAP_NANOID_ALPHABET, CAP_NANOID_LENGTH, FolderId, IdempotencyKey, LegacyCapNanoId,
    OrganizationId, OrganizationOperationId, SpaceId, UserId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_FOLDER_CRUD_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_MOBILE_CREATE_FOLDER_OPERATION_ID: &str = "cap-v1-7160c4389375c682";
pub const LEGACY_RPC_FOLDER_CREATE_OPERATION_ID: &str = "cap-v1-9e125712cee9ce5a";
pub const LEGACY_RPC_FOLDER_DELETE_OPERATION_ID: &str = "cap-v1-eea1796482b3af28";
pub const LEGACY_RPC_FOLDER_UPDATE_OPERATION_ID: &str = "cap-v1-a193e9e08b2c3f7d";
pub const LEGACY_MOBILE_CREATE_FOLDER_IDENTITY: &str = "/api/mobile/folders";
pub const LEGACY_RPC_FOLDER_CREATE_IDENTITY: &str = "/api/erpc#FolderCreate";
pub const LEGACY_RPC_FOLDER_DELETE_IDENTITY: &str = "/api/erpc#FolderDelete";
pub const LEGACY_RPC_FOLDER_UPDATE_IDENTITY: &str = "/api/erpc#FolderUpdate";
pub const LEGACY_FOLDER_CRUD_POLICY: &str = "organization_library.v1";
pub const LEGACY_FOLDER_CRUD_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_FOLDER_CRUD_CROSS_NAMESPACE_PROTECTED_GATES: &[&str] = &["human_approval"];
pub const LEGACY_FOLDER_CRUD_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_FOLDER_CRUD_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_FOLDER_NAME_MAX_CHARACTERS: usize = 255;
pub const LEGACY_FOLDER_SETTINGS_MAX_BYTES: usize = 16 * 1024;
pub const LEGACY_PUBLIC_PAGE_TITLE_MAX_UTF16_CODE_UNITS: usize = 80;
pub const LEGACY_PUBLIC_PAGE_SUBTITLE_MAX_UTF16_CODE_UNITS: usize = 160;
pub const LEGACY_PUBLIC_PAGE_CTA_LABEL_MAX_UTF16_CODE_UNITS: usize = 40;
pub const LEGACY_PUBLIC_PAGE_CTA_URL_MAX_UTF16_CODE_UNITS: usize = 512;

pub const LEGACY_MOBILE_CREATE_FOLDER_SOURCE_MANIFEST_SHA256: &str =
    "fd34af459bb9b5bac46118ad808e9065887fa4201dd2db67dde80bc295d17897";
pub const LEGACY_RPC_FOLDER_CREATE_SOURCE_MANIFEST_SHA256: &str =
    "4bd7c5ed8e4c94c34649ea2b23b6366d3c40a79f77d8e67021ad36e37f914407";
pub const LEGACY_RPC_FOLDER_DELETE_SOURCE_MANIFEST_SHA256: &str =
    "798c68c4b062c78e6cdb845338adc4c45e5bd20bda9d9f942957795695be12a4";
pub const LEGACY_RPC_FOLDER_UPDATE_SOURCE_MANIFEST_SHA256: &str =
    "2881b2ca39ba24c104349eebcc87bdc5d9427b0d3b2e2dc71c41837371d61102";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderCrudSourceRoleV1 {
    Transport,
    Contract,
    Schema,
    Policy,
    Authentication,
    Repository,
    Service,
    Entitlement,
    Dependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderCrudSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyFolderCrudSourceRoleV1,
}

pub const LEGACY_MOBILE_CREATE_FOLDER_SOURCES: &[LegacyFolderCrudSourcePinV1] = &[
    LegacyFolderCrudSourcePinV1 {
        path: "apps/web/app/api/mobile/[...route]/route.ts",
        symbol: "mobile handler:createFolder",
        sha256: "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
        role: LegacyFolderCrudSourceRoleV1::Transport,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        symbol: "getServerSession+authOptions",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyFolderCrudSourceRoleV1::Authentication,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/database/helpers.ts",
        symbol: "nanoId",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyFolderCrudSourceRoleV1::Repository,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/database/schema.ts",
        symbol: "folders+organizations+organizationMembers+authApiKeys",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyFolderCrudSourceRoleV1::Schema,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/web-backend/src/Auth.ts",
        symbol: "HttpAuthMiddlewareLive+getCurrentUser+CurrentUser",
        sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
        role: LegacyFolderCrudSourceRoleV1::Authentication,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/web-backend/src/Database.ts",
        symbol: "Database",
        sha256: "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
        role: LegacyFolderCrudSourceRoleV1::Repository,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/web-domain/src/Authentication.ts",
        symbol: "CurrentUser+HttpAuthMiddleware",
        sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
        role: LegacyFolderCrudSourceRoleV1::Authentication,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "packages/web-domain/src/Mobile.ts",
        symbol: "createFolder",
        sha256: "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
        role: LegacyFolderCrudSourceRoleV1::Contract,
    },
    LegacyFolderCrudSourcePinV1 {
        path: "pnpm-lock.yaml",
        symbol: "nanoid@5.1.6",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyFolderCrudSourceRoleV1::Dependency,
    },
];

const RPC_TRANSPORT: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "apps/web/app/api/erpc/route.ts",
    symbol: "Effect RPC HTTP transport",
    sha256: "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
    role: LegacyFolderCrudSourceRoleV1::Transport,
};
const RPC_SCHEMA: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "folder persistence schema",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyFolderCrudSourceRoleV1::Schema,
};
const RPC_PLAN: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/utils/src/constants/plans.ts",
    symbol: "folder Pro entitlement policy",
    sha256: "e047a50e6f72e3fe33985fde475b25ea4d5f9701fbe15adda0c1cb3aaaa21385",
    role: LegacyFolderCrudSourceRoleV1::Entitlement,
};
const RPC_AUTH: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Auth.ts",
    symbol: "Effect RPC auth layer",
    sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    role: LegacyFolderCrudSourceRoleV1::Authentication,
};
const RPC_FOLDER_POLICY: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Folders/FoldersPolicy.ts",
    symbol: "FoldersPolicy",
    sha256: "a180c8d95819a4c9f06a44c7ca5cb529a8b68ffc85e671db9a75e983537fcb46",
    role: LegacyFolderCrudSourceRoleV1::Policy,
};
const RPC_REPOSITORY: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Folders/FoldersRepo.ts",
    symbol: "FoldersRepo.create",
    sha256: "0b21fc1a9979a0dc3ae52dd66d8d550e1284d1f6eaa3c7c0d6d6a7c2c0fd9dde",
    role: LegacyFolderCrudSourceRoleV1::Repository,
};
const RPC_RPCS_CREATE: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Folders/FoldersRpcs.ts",
    symbol: "FolderCreate",
    sha256: "6409521d4000ee2600129bd1b37ea7d5f85352fe16f5a23de61d970dca8dd5b7",
    role: LegacyFolderCrudSourceRoleV1::Service,
};
const RPC_SERVICE_CREATE: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Folders/index.ts",
    symbol: "Folders.create",
    sha256: "4a3b6fdbe4a86e9291c8443a7502a44afd5305fb29a1f74a4e72e592cc0dc1c8",
    role: LegacyFolderCrudSourceRoleV1::Service,
};
const RPC_ORG_POLICY: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Organisations/OrganisationsPolicy.ts",
    symbol: "organization folder policy",
    sha256: "a003866c7dd649252bd705f5530b2ac0a2f23387a5a8c433069a0dd9cf532736",
    role: LegacyFolderCrudSourceRoleV1::Policy,
};
const RPC_ORG_REPOSITORY: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Organisations/OrganisationsRepo.ts",
    symbol: "organization entitlement repository",
    sha256: "9aff093ca8e653aae756116b957a21c942c8402afc2611d005e9027dd393a18a",
    role: LegacyFolderCrudSourceRoleV1::Repository,
};
const RPC_ROOT: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Rpcs.ts",
    symbol: "RpcsLive+RpcAuthMiddlewareLive",
    sha256: "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    role: LegacyFolderCrudSourceRoleV1::Transport,
};
const RPC_SPACE_POLICY: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Spaces/SpacesPolicy.ts",
    symbol: "space policy",
    sha256: "defa3bfac731008c1384cf6972c34f44924eec59b8f771e2695ea7fa7e3cb437",
    role: LegacyFolderCrudSourceRoleV1::Policy,
};
const RPC_SPACE_REPOSITORY: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Spaces/SpacesRepo.ts",
    symbol: "space repository",
    sha256: "d6f641bab86883dcc139ba1a989657404892b81f14ac790fd393186de45e8d0f",
    role: LegacyFolderCrudSourceRoleV1::Repository,
};
const RPC_SPACE_SERVICE: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-backend/src/Spaces/index.ts",
    symbol: "space service",
    sha256: "8784a80bf08f10e02269b9a7ec66a6ee0ee6623b8862677ac1b828673a266b56",
    role: LegacyFolderCrudSourceRoleV1::Service,
};
const RPC_AUTH_CONTRACT: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-domain/src/Authentication.ts",
    symbol: "authentication contract",
    sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    role: LegacyFolderCrudSourceRoleV1::Contract,
};
const RPC_FOLDER_CONTRACT: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-domain/src/Folder.ts",
    symbol: "FolderCreate",
    sha256: "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
    role: LegacyFolderCrudSourceRoleV1::Contract,
};
const RPC_POLICY_CONTRACT: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-domain/src/Policy.ts",
    symbol: "policy error contract",
    sha256: "0621949aa1f994836d0d168b39dc3aada3ad0478052b712de564b105c94ebe5c",
    role: LegacyFolderCrudSourceRoleV1::Contract,
};
const RPC_PUBLIC_CONTRACT: LegacyFolderCrudSourcePinV1 = LegacyFolderCrudSourcePinV1 {
    path: "packages/web-domain/src/PublicCollection.ts",
    symbol: "folder public-page contract",
    sha256: "1fe75d8f9e3e395a5c4bcf0153fd617b23d9cbab24b3917957a672a4c1f22e58",
    role: LegacyFolderCrudSourceRoleV1::Contract,
};

pub const LEGACY_RPC_FOLDER_CREATE_SOURCES: &[LegacyFolderCrudSourcePinV1] = &[
    RPC_TRANSPORT,
    RPC_SCHEMA,
    RPC_PLAN,
    RPC_AUTH,
    RPC_FOLDER_POLICY,
    RPC_REPOSITORY,
    RPC_RPCS_CREATE,
    RPC_SERVICE_CREATE,
    RPC_ORG_POLICY,
    RPC_ORG_REPOSITORY,
    RPC_ROOT,
    RPC_SPACE_POLICY,
    RPC_SPACE_REPOSITORY,
    RPC_SPACE_SERVICE,
    RPC_AUTH_CONTRACT,
    RPC_FOLDER_CONTRACT,
    RPC_POLICY_CONTRACT,
    RPC_PUBLIC_CONTRACT,
];

pub const LEGACY_RPC_FOLDER_DELETE_SOURCES: &[LegacyFolderCrudSourcePinV1] = &[
    RPC_TRANSPORT,
    RPC_FOLDER_POLICY,
    LegacyFolderCrudSourcePinV1 {
        symbol: "FoldersRepo",
        ..RPC_REPOSITORY
    },
    LegacyFolderCrudSourcePinV1 {
        symbol: "FolderDelete",
        ..RPC_RPCS_CREATE
    },
    LegacyFolderCrudSourcePinV1 {
        symbol: "Folders.delete",
        ..RPC_SERVICE_CREATE
    },
    RPC_ROOT,
    LegacyFolderCrudSourcePinV1 {
        symbol: "FolderDelete",
        ..RPC_FOLDER_CONTRACT
    },
];

pub const LEGACY_RPC_FOLDER_UPDATE_SOURCES: &[LegacyFolderCrudSourcePinV1] = &[
    RPC_TRANSPORT,
    RPC_SCHEMA,
    RPC_PLAN,
    RPC_AUTH,
    RPC_FOLDER_POLICY,
    LegacyFolderCrudSourcePinV1 {
        symbol: "FoldersRepo.update",
        ..RPC_REPOSITORY
    },
    LegacyFolderCrudSourcePinV1 {
        symbol: "FolderUpdate",
        ..RPC_RPCS_CREATE
    },
    LegacyFolderCrudSourcePinV1 {
        symbol: "Folders.update",
        ..RPC_SERVICE_CREATE
    },
    RPC_ORG_POLICY,
    RPC_ORG_REPOSITORY,
    RPC_ROOT,
    RPC_SPACE_POLICY,
    RPC_SPACE_REPOSITORY,
    RPC_SPACE_SERVICE,
    RPC_AUTH_CONTRACT,
    LegacyFolderCrudSourcePinV1 {
        symbol: "FolderUpdate",
        ..RPC_FOLDER_CONTRACT
    },
    RPC_POLICY_CONTRACT,
    RPC_PUBLIC_CONTRACT,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderCrudSurfaceV1 {
    MobileCreate,
    RpcCreate,
    RpcDelete,
    RpcUpdate,
}

impl LegacyFolderCrudSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::MobileCreate => LEGACY_MOBILE_CREATE_FOLDER_OPERATION_ID,
            Self::RpcCreate => LEGACY_RPC_FOLDER_CREATE_OPERATION_ID,
            Self::RpcDelete => LEGACY_RPC_FOLDER_DELETE_OPERATION_ID,
            Self::RpcUpdate => LEGACY_RPC_FOLDER_UPDATE_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::MobileCreate => "mobile_create",
            Self::RpcCreate => "rpc_create",
            Self::RpcDelete => "rpc_delete",
            Self::RpcUpdate => "rpc_update",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderCrudObservedSuccessV1 {
    MobileFolderProjection,
    RpcVoid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderCrudIdempotencyV1 {
    OptionalServerGeneratedWhenAbsent,
    ForbiddenServerGenerated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderCrudProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub sources: &'static [LegacyFolderCrudSourcePinV1],
    pub source_manifest_sha256: &'static str,
    pub authentication: &'static str,
    pub policy: &'static str,
    pub max_body_bytes: usize,
    pub idempotency: LegacyFolderCrudIdempotencyV1,
    pub success: LegacyFolderCrudObservedSuccessV1,
    pub tenant_non_disclosure: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

pub const LEGACY_MOBILE_CREATE_FOLDER_PROFILE: LegacyFolderCrudProfileV1 =
    LegacyFolderCrudProfileV1 {
        operation_id: LEGACY_MOBILE_CREATE_FOLDER_OPERATION_ID,
        kind: "route",
        method: "POST",
        legacy_identity: LEGACY_MOBILE_CREATE_FOLDER_IDENTITY,
        pinned_commit: LEGACY_FOLDER_CRUD_CAP_COMMIT,
        sources: LEGACY_MOBILE_CREATE_FOLDER_SOURCES,
        source_manifest_sha256: LEGACY_MOBILE_CREATE_FOLDER_SOURCE_MANIFEST_SHA256,
        authentication: "session_or_api_key",
        policy: LEGACY_FOLDER_CRUD_POLICY,
        max_body_bytes: LEGACY_FOLDER_CRUD_MAX_BODY_BYTES,
        idempotency: LegacyFolderCrudIdempotencyV1::OptionalServerGeneratedWhenAbsent,
        success: LegacyFolderCrudObservedSuccessV1::MobileFolderProjection,
        tenant_non_disclosure: true,
        protected_gates: LEGACY_FOLDER_CRUD_NO_PROTECTED_GATES,
        production_promoted: true,
    };
pub const LEGACY_RPC_FOLDER_CREATE_PROFILE: LegacyFolderCrudProfileV1 = LegacyFolderCrudProfileV1 {
    operation_id: LEGACY_RPC_FOLDER_CREATE_OPERATION_ID,
    kind: "rpc",
    method: "RPC",
    legacy_identity: LEGACY_RPC_FOLDER_CREATE_IDENTITY,
    pinned_commit: LEGACY_FOLDER_CRUD_CAP_COMMIT,
    sources: LEGACY_RPC_FOLDER_CREATE_SOURCES,
    source_manifest_sha256: LEGACY_RPC_FOLDER_CREATE_SOURCE_MANIFEST_SHA256,
    authentication: "session",
    policy: LEGACY_FOLDER_CRUD_POLICY,
    max_body_bytes: LEGACY_FOLDER_CRUD_MAX_BODY_BYTES,
    idempotency: LegacyFolderCrudIdempotencyV1::OptionalServerGeneratedWhenAbsent,
    success: LegacyFolderCrudObservedSuccessV1::RpcVoid,
    tenant_non_disclosure: true,
    protected_gates: LEGACY_FOLDER_CRUD_CROSS_NAMESPACE_PROTECTED_GATES,
    production_promoted: false,
};
pub const LEGACY_RPC_FOLDER_DELETE_PROFILE: LegacyFolderCrudProfileV1 = LegacyFolderCrudProfileV1 {
    operation_id: LEGACY_RPC_FOLDER_DELETE_OPERATION_ID,
    kind: "rpc",
    method: "RPC",
    legacy_identity: LEGACY_RPC_FOLDER_DELETE_IDENTITY,
    pinned_commit: LEGACY_FOLDER_CRUD_CAP_COMMIT,
    sources: LEGACY_RPC_FOLDER_DELETE_SOURCES,
    source_manifest_sha256: LEGACY_RPC_FOLDER_DELETE_SOURCE_MANIFEST_SHA256,
    authentication: "session",
    policy: LEGACY_FOLDER_CRUD_POLICY,
    max_body_bytes: LEGACY_FOLDER_CRUD_MAX_BODY_BYTES,
    idempotency: LegacyFolderCrudIdempotencyV1::ForbiddenServerGenerated,
    success: LegacyFolderCrudObservedSuccessV1::RpcVoid,
    tenant_non_disclosure: true,
    protected_gates: LEGACY_FOLDER_CRUD_CROSS_NAMESPACE_PROTECTED_GATES,
    production_promoted: false,
};
pub const LEGACY_RPC_FOLDER_UPDATE_PROFILE: LegacyFolderCrudProfileV1 = LegacyFolderCrudProfileV1 {
    operation_id: LEGACY_RPC_FOLDER_UPDATE_OPERATION_ID,
    kind: "rpc",
    method: "RPC",
    legacy_identity: LEGACY_RPC_FOLDER_UPDATE_IDENTITY,
    pinned_commit: LEGACY_FOLDER_CRUD_CAP_COMMIT,
    sources: LEGACY_RPC_FOLDER_UPDATE_SOURCES,
    source_manifest_sha256: LEGACY_RPC_FOLDER_UPDATE_SOURCE_MANIFEST_SHA256,
    authentication: "session",
    policy: LEGACY_FOLDER_CRUD_POLICY,
    max_body_bytes: LEGACY_FOLDER_CRUD_MAX_BODY_BYTES,
    idempotency: LegacyFolderCrudIdempotencyV1::OptionalServerGeneratedWhenAbsent,
    success: LegacyFolderCrudObservedSuccessV1::RpcVoid,
    tenant_non_disclosure: true,
    protected_gates: LEGACY_FOLDER_CRUD_CROSS_NAMESPACE_PROTECTED_GATES,
    production_promoted: false,
};

pub const LEGACY_FOLDER_CRUD_PROFILES: &[LegacyFolderCrudProfileV1] = &[
    LEGACY_MOBILE_CREATE_FOLDER_PROFILE,
    LEGACY_RPC_FOLDER_CREATE_PROFILE,
    LEGACY_RPC_FOLDER_DELETE_PROFILE,
    LEGACY_RPC_FOLDER_UPDATE_PROFILE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderCrudCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderColorV1 {
    Normal,
    Blue,
    Red,
    Yellow,
}

impl LegacyFolderColorV1 {
    pub fn parse(value: &str) -> Result<Self, LegacyFolderCrudErrorV1> {
        match value {
            "normal" => Ok(Self::Normal),
            "blue" => Ok(Self::Blue),
            "red" => Ok(Self::Red),
            "yellow" => Ok(Self::Yellow),
            _ => Err(LegacyFolderCrudErrorV1::InvalidInput),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Yellow => "yellow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderLogoModeV1 {
    Cap,
    Organization,
    Custom,
    None,
}

impl LegacyFolderLogoModeV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cap => "cap",
            Self::Organization => "organization",
            Self::Custom => "custom",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderLayoutV1 {
    Grid,
    List,
}

impl LegacyFolderLayoutV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::List => "list",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LegacyFolderPublicPagePatchV1 {
    pub hide_title: Option<bool>,
    pub hide_copy_link: Option<bool>,
    pub logo_mode: Option<LegacyFolderLogoModeV1>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub cta_label: Option<String>,
    pub cta_url: Option<String>,
    pub layout: Option<LegacyFolderLayoutV1>,
    pub grid_columns: Option<u8>,
}

impl LegacyFolderPublicPagePatchV1 {
    fn validate(&self) -> Result<(), LegacyFolderCrudErrorV1> {
        for (value, maximum) in [
            (
                self.title.as_deref(),
                LEGACY_PUBLIC_PAGE_TITLE_MAX_UTF16_CODE_UNITS,
            ),
            (
                self.subtitle.as_deref(),
                LEGACY_PUBLIC_PAGE_SUBTITLE_MAX_UTF16_CODE_UNITS,
            ),
            (
                self.cta_label.as_deref(),
                LEGACY_PUBLIC_PAGE_CTA_LABEL_MAX_UTF16_CODE_UNITS,
            ),
            (
                self.cta_url.as_deref(),
                LEGACY_PUBLIC_PAGE_CTA_URL_MAX_UTF16_CODE_UNITS,
            ),
        ] {
            if value.is_some_and(|candidate| candidate.encode_utf16().count() > maximum) {
                return Err(LegacyFolderCrudErrorV1::InvalidInput);
            }
        }
        if self
            .grid_columns
            .is_some_and(|columns| !matches!(columns, 2..=5))
        {
            return Err(LegacyFolderCrudErrorV1::InvalidInput);
        }
        Ok(())
    }
}

/// `Absent` means do not update the parent; `Root` is an explicit null.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyFolderParentPatchV1 {
    Absent,
    Root,
    Parent(String),
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyFolderCrudInputV1 {
    MobileCreate {
        name: String,
        color: Option<String>,
    },
    RpcCreate {
        name: String,
        color: String,
        public: Option<bool>,
        space_id: Option<String>,
        parent_id: Option<String>,
    },
    RpcDelete {
        folder_id: String,
    },
    RpcUpdate {
        folder_id: String,
        name: Option<String>,
        color: Option<String>,
        public: Option<bool>,
        public_page: Option<LegacyFolderPublicPagePatchV1>,
        parent_id: LegacyFolderParentPatchV1,
    },
}

impl LegacyFolderCrudInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyFolderCrudSurfaceV1 {
        match self {
            Self::MobileCreate { .. } => LegacyFolderCrudSurfaceV1::MobileCreate,
            Self::RpcCreate { .. } => LegacyFolderCrudSurfaceV1::RpcCreate,
            Self::RpcDelete { .. } => LegacyFolderCrudSurfaceV1::RpcDelete,
            Self::RpcUpdate { .. } => LegacyFolderCrudSurfaceV1::RpcUpdate,
        }
    }
}

impl fmt::Debug for LegacyFolderCrudInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MobileCreate { .. } => "MobileCreate([redacted])",
            Self::RpcCreate { .. } => "RpcCreate([redacted])",
            Self::RpcDelete { .. } => "RpcDelete([redacted])",
            Self::RpcUpdate { .. } => "RpcUpdate([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderCrudRequestV1 {
    pub credential: Option<LegacyFolderCrudCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyFolderCrudInputV1,
}

impl fmt::Debug for LegacyFolderCrudRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderCrudRequestV1")
            .field("credential", &self.credential)
            .field("actor", &self.actor_id.map(|_| "<redacted>"))
            .field(
                "active_organization",
                &self.active_organization_id.map(|_| "<redacted>"),
            )
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("input", &self.input)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderScopeV1 {
    Personal,
    OrganizationLibrary,
    Space(SpaceId),
}

impl LegacyFolderScopeV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::OrganizationLibrary => "organization",
            Self::Space(_) => "space",
        }
    }

    #[must_use]
    pub const fn space_id(self) -> Option<SpaceId> {
        match self {
            Self::Space(space_id) => Some(space_id),
            Self::Personal | Self::OrganizationLibrary => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderCrudAuthorityV1 {
    actor_id: UserId,
    active_organization_id: OrganizationId,
    credential: LegacyFolderCrudCredentialV1,
}

impl LegacyFolderCrudAuthorityV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> OrganizationId {
        self.active_organization_id
    }

    #[must_use]
    pub const fn credential(&self) -> LegacyFolderCrudCredentialV1 {
        self.credential
    }
}

impl fmt::Debug for LegacyFolderCrudAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyFolderCrudAuthorityV1([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyFolderCrudMutationV1 {
    Create {
        legacy_folder_id: LegacyCapNanoId,
        folder_id: FolderId,
        name: String,
        color: LegacyFolderColorV1,
        is_public: bool,
        scope: LegacyFolderScopeV1,
        parent_id: Option<FolderId>,
    },
    Delete {
        folder_id: FolderId,
    },
    Update {
        folder_id: FolderId,
        name: Option<String>,
        color: Option<LegacyFolderColorV1>,
        is_public: Option<bool>,
        public_page: Option<LegacyFolderPublicPagePatchV1>,
        parent_id: LegacyMappedParentPatchV1,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMappedParentPatchV1 {
    Absent,
    Root,
    Parent(FolderId),
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderCrudCommandV1 {
    operation_id: OrganizationOperationId,
    surface: LegacyFolderCrudSurfaceV1,
    authority: LegacyFolderCrudAuthorityV1,
    idempotency_key: IdempotencyKey,
    request_digest: [u8; 32],
    mutation: LegacyFolderCrudMutationV1,
}

impl LegacyFolderCrudCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> OrganizationOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn surface(&self) -> LegacyFolderCrudSurfaceV1 {
        self.surface
    }

    #[must_use]
    pub const fn authority(&self) -> &LegacyFolderCrudAuthorityV1 {
        &self.authority
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    #[must_use]
    pub const fn request_digest(&self) -> &[u8; 32] {
        &self.request_digest
    }

    #[must_use]
    pub const fn mutation(&self) -> &LegacyFolderCrudMutationV1 {
        &self.mutation
    }

    #[must_use]
    pub fn request_digest_hex(&self) -> String {
        hex(&self.request_digest)
    }

    #[must_use]
    pub fn idempotency_key_digest_hex(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-legacy-folder-crud-key-v1\0");
        digest.update(self.idempotency_key.expose().as_bytes());
        hex(&digest.finalize())
    }
}

impl fmt::Debug for LegacyFolderCrudCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderCrudCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.surface)
            .field("authority", &self.authority)
            .field("request_digest", &"<redacted>")
            .field("mutation", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyFolderCrudMutationResultV1 {
    MobileCreated {
        legacy_folder_id: LegacyCapNanoId,
        name: String,
        color: LegacyFolderColorV1,
    },
    RpcVoid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyFolderCrudAtomicOutcomeV1 {
    pub result: LegacyFolderCrudMutationResultV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyFolderCrudSuccessV1 {
    MobileFolder {
        id: String,
        name: String,
        color: &'static str,
        parent_id: Option<String>,
        video_count: u32,
        replayed: bool,
    },
    RpcVoid {
        replayed: bool,
    },
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyFolderCrudAtomicErrorV1 {
    #[error("folder authority is stale")]
    StaleAuthority,
    #[error("folder access is denied")]
    AccessDenied,
    #[error("folder target is absent")]
    TargetMissing,
    #[error("folder parent is absent")]
    ParentMissing,
    #[error("folder hierarchy is recursive")]
    RecursiveDefinition,
    #[error("folder scope conflicts with Frame invariants")]
    ScopeConflict,
    #[error("folder mutation conflicts with current state")]
    Conflict,
    #[error("idempotency key was reused with another request")]
    IdempotencyConflict,
    #[error("folder mutation is still in flight")]
    InFlight,
    #[error("folder authority is unavailable")]
    Unavailable,
    #[error("folder authority returned corrupt state")]
    Corrupt,
}

#[async_trait]
pub trait LegacyFolderCrudAtomicPortV1: Send + Sync {
    async fn execute(
        &self,
        command: LegacyFolderCrudCommandV1,
    ) -> Result<LegacyFolderCrudAtomicOutcomeV1, LegacyFolderCrudAtomicErrorV1>;
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyFolderCrudErrorV1 {
    #[error("authentication is required")]
    Unauthorized,
    #[error("folder input is invalid")]
    InvalidInput,
    #[error("folder access is denied")]
    AccessDenied,
    #[error("folder was not found")]
    NotFound,
    #[error("folder parent was not found")]
    ParentNotFound,
    #[error("folder hierarchy is recursive")]
    RecursiveDefinition,
    #[error("folder scope conflicts with Frame invariants")]
    ScopeConflict,
    #[error("folder request conflicts with current state")]
    Conflict,
    #[error("folder authority is unavailable")]
    Unavailable,
    #[error("folder action failed")]
    Internal,
    #[error("folder persistence failed")]
    Database,
}

pub struct LegacyFolderCrudAdapterV1<'port, Port> {
    port: &'port Port,
}

impl<'port, Port> LegacyFolderCrudAdapterV1<'port, Port>
where
    Port: LegacyFolderCrudAtomicPortV1,
{
    #[must_use]
    pub const fn new(port: &'port Port) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        request: LegacyFolderCrudRequestV1,
    ) -> Result<LegacyFolderCrudSuccessV1, LegacyFolderCrudErrorV1> {
        let command = prepare(request)?;
        let expected_surface = command.surface();
        let outcome = self.port.execute(command).await.map_err(map_atomic_error)?;
        project_success(expected_surface, outcome)
    }
}

fn prepare(
    request: LegacyFolderCrudRequestV1,
) -> Result<LegacyFolderCrudCommandV1, LegacyFolderCrudErrorV1> {
    let surface = request.input.surface();
    let credential = request
        .credential
        .ok_or(LegacyFolderCrudErrorV1::Unauthorized)?;
    if surface != LegacyFolderCrudSurfaceV1::MobileCreate
        && credential != LegacyFolderCrudCredentialV1::Session
    {
        return Err(LegacyFolderCrudErrorV1::Unauthorized);
    }
    let actor_id = request
        .actor_id
        .ok_or(LegacyFolderCrudErrorV1::Unauthorized)?;
    let active_organization_id = request
        .active_organization_id
        .ok_or(LegacyFolderCrudErrorV1::Unauthorized)?;
    let operation_id = OrganizationOperationId::new();
    let idempotency_key = match request.idempotency_key {
        Some(_) if surface == LegacyFolderCrudSurfaceV1::RpcDelete => {
            return Err(LegacyFolderCrudErrorV1::InvalidInput);
        }
        Some(value) => {
            IdempotencyKey::parse(value).map_err(|_| LegacyFolderCrudErrorV1::InvalidInput)?
        }
        None => IdempotencyKey::parse(format!("folder-auto:{operation_id}"))
            .map_err(|_| LegacyFolderCrudErrorV1::Internal)?,
    };
    let authority = LegacyFolderCrudAuthorityV1 {
        actor_id,
        active_organization_id,
        credential,
    };
    let mutation = normalize_input(
        request.input,
        actor_id,
        active_organization_id,
        &idempotency_key,
    )?;
    let request_digest = command_digest(surface, &authority, &mutation);
    Ok(LegacyFolderCrudCommandV1 {
        operation_id,
        surface,
        authority,
        idempotency_key,
        request_digest,
        mutation,
    })
}

fn normalize_input(
    input: LegacyFolderCrudInputV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    idempotency_key: &IdempotencyKey,
) -> Result<LegacyFolderCrudMutationV1, LegacyFolderCrudErrorV1> {
    match input {
        LegacyFolderCrudInputV1::MobileCreate { name, color } => {
            let name = trim_ecmascript(&name).to_owned();
            if name.is_empty() {
                return Err(LegacyFolderCrudErrorV1::InvalidInput);
            }
            validate_persisted_name(&name)?;
            let color = LegacyFolderColorV1::parse(color.as_deref().unwrap_or("normal"))?;
            let legacy_folder_id = derive_legacy_folder_id(
                actor_id,
                organization_id,
                LegacyFolderCrudSurfaceV1::MobileCreate,
                idempotency_key,
            )?;
            let folder_id = mapped_folder_id(&legacy_folder_id)?;
            Ok(LegacyFolderCrudMutationV1::Create {
                legacy_folder_id,
                folder_id,
                name,
                color,
                is_public: false,
                scope: LegacyFolderScopeV1::Personal,
                parent_id: None,
            })
        }
        LegacyFolderCrudInputV1::RpcCreate {
            name,
            color,
            public,
            space_id,
            parent_id,
        } => {
            validate_persisted_name(&name)?;
            let color = LegacyFolderColorV1::parse(&color)?;
            let scope = match space_id {
                None => LegacyFolderScopeV1::Personal,
                Some(value) => {
                    let mapped = mapped_source_id(&value);
                    if mapped == organization_id.to_string() {
                        LegacyFolderScopeV1::OrganizationLibrary
                    } else {
                        LegacyFolderScopeV1::Space(
                            SpaceId::parse(&mapped)
                                .map_err(|_| LegacyFolderCrudErrorV1::InvalidInput)?,
                        )
                    }
                }
            };
            let parent_id = parent_id.map(mapped_folder_id_from_string).transpose()?;
            let legacy_folder_id = derive_legacy_folder_id(
                actor_id,
                organization_id,
                LegacyFolderCrudSurfaceV1::RpcCreate,
                idempotency_key,
            )?;
            let folder_id = mapped_folder_id(&legacy_folder_id)?;
            Ok(LegacyFolderCrudMutationV1::Create {
                legacy_folder_id,
                folder_id,
                name,
                color,
                is_public: public.unwrap_or(false),
                scope,
                parent_id,
            })
        }
        LegacyFolderCrudInputV1::RpcDelete { folder_id } => {
            Ok(LegacyFolderCrudMutationV1::Delete {
                folder_id: mapped_folder_id_from_string(folder_id)?,
            })
        }
        LegacyFolderCrudInputV1::RpcUpdate {
            folder_id,
            name,
            color,
            public,
            public_page,
            parent_id,
        } => {
            if let Some(name) = &name {
                validate_persisted_name(name)?;
            }
            let color = color
                .as_deref()
                .map(LegacyFolderColorV1::parse)
                .transpose()?;
            if let Some(public_page) = &public_page {
                public_page.validate()?;
            }
            let parent_id = match parent_id {
                LegacyFolderParentPatchV1::Absent => LegacyMappedParentPatchV1::Absent,
                LegacyFolderParentPatchV1::Root => LegacyMappedParentPatchV1::Root,
                LegacyFolderParentPatchV1::Parent(parent) => {
                    LegacyMappedParentPatchV1::Parent(mapped_folder_id_from_string(parent)?)
                }
            };
            Ok(LegacyFolderCrudMutationV1::Update {
                folder_id: mapped_folder_id_from_string(folder_id)?,
                name,
                color,
                is_public: public,
                public_page,
                parent_id,
            })
        }
    }
}

fn validate_persisted_name(value: &str) -> Result<(), LegacyFolderCrudErrorV1> {
    if value.chars().count() > LEGACY_FOLDER_NAME_MAX_CHARACTERS {
        // Cap's Effect schema accepts any string here. MySQL enforces the
        // VARCHAR bound, so this is a persistence failure rather than a wire
        // parse failure.
        return Err(LegacyFolderCrudErrorV1::Database);
    }
    Ok(())
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

fn mapped_folder_id_from_string(value: String) -> Result<FolderId, LegacyFolderCrudErrorV1> {
    FolderId::parse(&mapped_source_id(&value)).map_err(|_| LegacyFolderCrudErrorV1::InvalidInput)
}

fn mapped_folder_id(value: &LegacyCapNanoId) -> Result<FolderId, LegacyFolderCrudErrorV1> {
    FolderId::parse(&value.mapped_uuid().to_string())
        .map_err(|_| LegacyFolderCrudErrorV1::InvalidInput)
}

fn mapped_source_id(value: &str) -> String {
    if let Ok(legacy) = LegacyCapNanoId::parse(value.to_owned()) {
        return legacy.mapped_uuid().to_string();
    }
    // Effect's branded IDs are runtime strings with no NanoID refinement.
    // Preserve lookup semantics for arbitrary strings by deterministically
    // mapping them into Frame's UUID keyspace; a missing row then reaches the
    // policy/target layer instead of becoming a schema parse failure.
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-opaque-id-to-uuid-v1\0");
    digest.update(value.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let encoded = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &encoded[..8],
        &encoded[8..12],
        &encoded[12..16],
        &encoded[16..20],
        &encoded[20..32]
    )
}

fn derive_legacy_folder_id(
    actor_id: UserId,
    organization_id: OrganizationId,
    surface: LegacyFolderCrudSurfaceV1,
    idempotency_key: &IdempotencyKey,
) -> Result<LegacyCapNanoId, LegacyFolderCrudErrorV1> {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-folder-crud-id-v1\0");
    framed(&mut digest, actor_id.to_string().as_bytes());
    framed(&mut digest, organization_id.to_string().as_bytes());
    framed(&mut digest, surface.operation_id().as_bytes());
    framed(&mut digest, idempotency_key.expose().as_bytes());
    let digest = digest.finalize();
    let alphabet = CAP_NANOID_ALPHABET.as_bytes();
    let mut value = String::with_capacity(CAP_NANOID_LENGTH);
    for byte in digest.iter().take(CAP_NANOID_LENGTH) {
        value.push(char::from(alphabet[usize::from(*byte & 31)]));
    }
    LegacyCapNanoId::parse(value).map_err(|_| LegacyFolderCrudErrorV1::Internal)
}

fn command_digest(
    surface: LegacyFolderCrudSurfaceV1,
    authority: &LegacyFolderCrudAuthorityV1,
    mutation: &LegacyFolderCrudMutationV1,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-folder-crud-request-v1\0");
    framed(&mut digest, surface.operation_id().as_bytes());
    framed(&mut digest, authority.actor_id.to_string().as_bytes());
    framed(
        &mut digest,
        authority.active_organization_id.to_string().as_bytes(),
    );
    match mutation {
        LegacyFolderCrudMutationV1::Create {
            folder_id,
            name,
            color,
            is_public,
            scope,
            parent_id,
            ..
        } => {
            framed(&mut digest, b"create");
            framed(&mut digest, folder_id.to_string().as_bytes());
            framed(&mut digest, name.as_bytes());
            framed(&mut digest, color.as_str().as_bytes());
            framed(&mut digest, &[u8::from(*is_public)]);
            framed(&mut digest, scope.stable_code().as_bytes());
            optional_uuid(&mut digest, scope.space_id().map(|id| id.to_string()));
            optional_uuid(&mut digest, parent_id.map(|id| id.to_string()));
        }
        LegacyFolderCrudMutationV1::Delete { folder_id } => {
            framed(&mut digest, b"delete");
            framed(&mut digest, folder_id.to_string().as_bytes());
        }
        LegacyFolderCrudMutationV1::Update {
            folder_id,
            name,
            color,
            is_public,
            public_page,
            parent_id,
        } => {
            framed(&mut digest, b"update");
            framed(&mut digest, folder_id.to_string().as_bytes());
            optional_string(&mut digest, name.as_deref());
            optional_string(&mut digest, color.map(LegacyFolderColorV1::as_str));
            match is_public {
                None => framed(&mut digest, b"absent"),
                Some(value) => framed(&mut digest, &[u8::from(*value)]),
            }
            public_page_digest(&mut digest, public_page.as_ref());
            match parent_id {
                LegacyMappedParentPatchV1::Absent => framed(&mut digest, b"absent"),
                LegacyMappedParentPatchV1::Root => framed(&mut digest, b"root"),
                LegacyMappedParentPatchV1::Parent(parent) => {
                    framed(&mut digest, parent.to_string().as_bytes());
                }
            }
        }
    }
    digest.finalize().into()
}

fn public_page_digest(digest: &mut Sha256, value: Option<&LegacyFolderPublicPagePatchV1>) {
    let Some(value) = value else {
        framed(digest, b"absent");
        return;
    };
    framed(digest, b"present");
    optional_bool(digest, value.hide_title);
    optional_bool(digest, value.hide_copy_link);
    optional_string(digest, value.logo_mode.map(LegacyFolderLogoModeV1::as_str));
    optional_string(digest, value.title.as_deref());
    optional_string(digest, value.subtitle.as_deref());
    optional_string(digest, value.cta_label.as_deref());
    optional_string(digest, value.cta_url.as_deref());
    optional_string(digest, value.layout.map(LegacyFolderLayoutV1::as_str));
    match value.grid_columns {
        None => framed(digest, b"absent"),
        Some(columns) => framed(digest, &[columns]),
    }
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn optional_uuid(digest: &mut Sha256, value: Option<String>) {
    optional_string(digest, value.as_deref());
}

fn optional_string(digest: &mut Sha256, value: Option<&str>) {
    match value {
        None => framed(digest, b"absent"),
        Some(value) => framed(digest, value.as_bytes()),
    }
}

fn optional_bool(digest: &mut Sha256, value: Option<bool>) {
    match value {
        None => framed(digest, b"absent"),
        Some(value) => framed(digest, &[u8::from(value)]),
    }
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

fn map_atomic_error(error: LegacyFolderCrudAtomicErrorV1) -> LegacyFolderCrudErrorV1 {
    match error {
        LegacyFolderCrudAtomicErrorV1::StaleAuthority
        | LegacyFolderCrudAtomicErrorV1::AccessDenied => LegacyFolderCrudErrorV1::AccessDenied,
        LegacyFolderCrudAtomicErrorV1::TargetMissing => LegacyFolderCrudErrorV1::NotFound,
        LegacyFolderCrudAtomicErrorV1::ParentMissing => LegacyFolderCrudErrorV1::ParentNotFound,
        LegacyFolderCrudAtomicErrorV1::RecursiveDefinition => {
            LegacyFolderCrudErrorV1::RecursiveDefinition
        }
        LegacyFolderCrudAtomicErrorV1::ScopeConflict => LegacyFolderCrudErrorV1::ScopeConflict,
        LegacyFolderCrudAtomicErrorV1::Conflict
        | LegacyFolderCrudAtomicErrorV1::IdempotencyConflict
        | LegacyFolderCrudAtomicErrorV1::InFlight => LegacyFolderCrudErrorV1::Conflict,
        LegacyFolderCrudAtomicErrorV1::Unavailable => LegacyFolderCrudErrorV1::Unavailable,
        LegacyFolderCrudAtomicErrorV1::Corrupt => LegacyFolderCrudErrorV1::Internal,
    }
}

fn project_success(
    surface: LegacyFolderCrudSurfaceV1,
    outcome: LegacyFolderCrudAtomicOutcomeV1,
) -> Result<LegacyFolderCrudSuccessV1, LegacyFolderCrudErrorV1> {
    match (surface, outcome.result) {
        (
            LegacyFolderCrudSurfaceV1::MobileCreate,
            LegacyFolderCrudMutationResultV1::MobileCreated {
                legacy_folder_id,
                name,
                color,
            },
        ) => Ok(LegacyFolderCrudSuccessV1::MobileFolder {
            id: legacy_folder_id.as_str().to_owned(),
            name,
            color: color.as_str(),
            parent_id: None,
            video_count: 0,
            replayed: outcome.replayed,
        }),
        (
            LegacyFolderCrudSurfaceV1::RpcCreate
            | LegacyFolderCrudSurfaceV1::RpcDelete
            | LegacyFolderCrudSurfaceV1::RpcUpdate,
            LegacyFolderCrudMutationResultV1::RpcVoid,
        ) => Ok(LegacyFolderCrudSuccessV1::RpcVoid {
            replayed: outcome.replayed,
        }),
        _ => Err(LegacyFolderCrudErrorV1::Internal),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakePort {
        commands: Mutex<Vec<LegacyFolderCrudCommandV1>>,
        error: Mutex<Option<LegacyFolderCrudAtomicErrorV1>>,
    }

    #[async_trait]
    impl LegacyFolderCrudAtomicPortV1 for FakePort {
        async fn execute(
            &self,
            command: LegacyFolderCrudCommandV1,
        ) -> Result<LegacyFolderCrudAtomicOutcomeV1, LegacyFolderCrudAtomicErrorV1> {
            if let Some(error) = *self.error.lock().expect("error") {
                return Err(error);
            }
            let result = match command.mutation() {
                LegacyFolderCrudMutationV1::Create {
                    legacy_folder_id,
                    name,
                    color,
                    ..
                } if command.surface() == LegacyFolderCrudSurfaceV1::MobileCreate => {
                    LegacyFolderCrudMutationResultV1::MobileCreated {
                        legacy_folder_id: legacy_folder_id.clone(),
                        name: name.clone(),
                        color: *color,
                    }
                }
                _ => LegacyFolderCrudMutationResultV1::RpcVoid,
            };
            self.commands.lock().expect("commands").push(command);
            Ok(LegacyFolderCrudAtomicOutcomeV1 {
                result,
                replayed: false,
            })
        }
    }

    fn actor() -> UserId {
        UserId::parse("00000000-0000-7000-8000-000000000001").expect("actor")
    }

    fn organization() -> OrganizationId {
        OrganizationId::parse("00000000-0000-7000-8000-000000000002").expect("organization")
    }

    fn cap_id(seed: u8) -> String {
        let alphabet = CAP_NANOID_ALPHABET.as_bytes();
        (0..CAP_NANOID_LENGTH)
            .map(|offset| char::from(alphabet[(usize::from(seed) + offset) % alphabet.len()]))
            .collect()
    }

    fn request(input: LegacyFolderCrudInputV1) -> LegacyFolderCrudRequestV1 {
        LegacyFolderCrudRequestV1 {
            credential: Some(LegacyFolderCrudCredentialV1::Session),
            actor_id: Some(actor()),
            active_organization_id: Some(organization()),
            idempotency_key: Some("folder-request-0001".into()),
            input,
        }
    }

    #[test]
    fn profiles_pin_exact_inventory_identities_and_source_closures() {
        assert_eq!(LEGACY_FOLDER_CRUD_PROFILES.len(), 4);
        assert_eq!(LEGACY_MOBILE_CREATE_FOLDER_SOURCES.len(), 9);
        assert_eq!(LEGACY_RPC_FOLDER_CREATE_SOURCES.len(), 18);
        assert_eq!(LEGACY_RPC_FOLDER_DELETE_SOURCES.len(), 7);
        assert_eq!(LEGACY_RPC_FOLDER_UPDATE_SOURCES.len(), 18);
        assert_eq!(
            LEGACY_FOLDER_CRUD_PROFILES
                .iter()
                .map(|profile| profile.operation_id)
                .collect::<Vec<_>>(),
            vec![
                "cap-v1-7160c4389375c682",
                "cap-v1-9e125712cee9ce5a",
                "cap-v1-eea1796482b3af28",
                "cap-v1-a193e9e08b2c3f7d",
            ]
        );
        assert!(
            LEGACY_FOLDER_CRUD_PROFILES
                .iter()
                .all(|profile| profile.tenant_non_disclosure)
        );
        assert_eq!(
            LEGACY_FOLDER_CRUD_PROFILES
                .iter()
                .map(|profile| (profile.protected_gates, profile.production_promoted))
                .collect::<Vec<_>>(),
            vec![
                (&[][..], true),
                (&["human_approval"][..], false),
                (&["human_approval"][..], false),
                (&["human_approval"][..], false),
            ]
        );
    }

    #[tokio::test]
    async fn mobile_create_trims_defaults_and_projects_exact_folder() {
        let port = FakePort::default();
        let result = LegacyFolderCrudAdapterV1::new(&port)
            .execute(request(LegacyFolderCrudInputV1::MobileCreate {
                name: "  Launches  ".into(),
                color: None,
            }))
            .await
            .expect("mobile create");
        let LegacyFolderCrudSuccessV1::MobileFolder {
            id,
            name,
            color,
            parent_id,
            video_count,
            replayed,
        } = result
        else {
            panic!("mobile response")
        };
        assert_eq!(id.len(), CAP_NANOID_LENGTH);
        assert_eq!(name, "Launches");
        assert_eq!(color, "normal");
        assert_eq!(parent_id, None);
        assert_eq!(video_count, 0);
        assert!(!replayed);
        let commands = port.commands.lock().expect("commands");
        let LegacyFolderCrudMutationV1::Create {
            scope, parent_id, ..
        } = commands[0].mutation()
        else {
            panic!("create command")
        };
        assert_eq!(*scope, LegacyFolderScopeV1::Personal);
        assert_eq!(*parent_id, None);
    }

    #[tokio::test]
    async fn mobile_allows_api_key_but_rpc_requires_session() {
        let port = FakePort::default();
        let mut mobile = request(LegacyFolderCrudInputV1::MobileCreate {
            name: "Folder".into(),
            color: Some("blue".into()),
        });
        mobile.credential = Some(LegacyFolderCrudCredentialV1::ApiKey);
        assert!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(mobile)
                .await
                .is_ok()
        );

        let mut rpc = request(LegacyFolderCrudInputV1::RpcDelete {
            folder_id: cap_id(1),
        });
        rpc.credential = Some(LegacyFolderCrudCredentialV1::ApiKey);
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port).execute(rpc).await,
            Err(LegacyFolderCrudErrorV1::Unauthorized)
        );
    }

    #[tokio::test]
    async fn rpc_create_preserves_scope_parent_and_void_success() {
        let port = FakePort::default();
        let space = cap_id(2);
        let parent = cap_id(3);
        let result = LegacyFolderCrudAdapterV1::new(&port)
            .execute(request(LegacyFolderCrudInputV1::RpcCreate {
                name: "Roadmap".into(),
                color: "yellow".into(),
                public: Some(true),
                space_id: Some(space.clone()),
                parent_id: Some(parent.clone()),
            }))
            .await
            .expect("rpc create");
        assert_eq!(
            result,
            LegacyFolderCrudSuccessV1::RpcVoid { replayed: false }
        );
        let commands = port.commands.lock().expect("commands");
        let LegacyFolderCrudMutationV1::Create {
            scope,
            parent_id,
            is_public,
            ..
        } = commands[0].mutation()
        else {
            panic!("create command")
        };
        assert!(matches!(scope, LegacyFolderScopeV1::Space(_)));
        assert_eq!(
            *parent_id,
            Some(mapped_folder_id_from_string(parent).expect("parent"))
        );
        assert!(*is_public);
    }

    #[tokio::test]
    async fn organization_identifier_scope_is_not_confused_with_personal_root() {
        let port = FakePort::default();
        let legacy_org = LegacyCapNanoId::parse(cap_id(4)).expect("legacy org");
        let mapped_org = OrganizationId::parse(&legacy_org.mapped_uuid().to_string())
            .expect("mapped organization");
        let mut candidate = request(LegacyFolderCrudInputV1::RpcCreate {
            name: "Org library".into(),
            color: "normal".into(),
            public: None,
            space_id: Some(legacy_org.as_str().into()),
            parent_id: None,
        });
        candidate.active_organization_id = Some(mapped_org);
        LegacyFolderCrudAdapterV1::new(&port)
            .execute(candidate)
            .await
            .expect("organization library");
        let commands = port.commands.lock().expect("commands");
        let LegacyFolderCrudMutationV1::Create { scope, .. } = commands[0].mutation() else {
            panic!("create")
        };
        assert_eq!(*scope, LegacyFolderScopeV1::OrganizationLibrary);
    }

    #[tokio::test]
    async fn update_keeps_absent_root_and_parent_presence_distinct() {
        let port = FakePort::default();
        for (patch, expected) in [
            (
                LegacyFolderParentPatchV1::Absent,
                LegacyMappedParentPatchV1::Absent,
            ),
            (
                LegacyFolderParentPatchV1::Root,
                LegacyMappedParentPatchV1::Root,
            ),
        ] {
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::RpcUpdate {
                    folder_id: cap_id(5),
                    name: None,
                    color: None,
                    public: None,
                    public_page: None,
                    parent_id: patch,
                }))
                .await
                .expect("update");
            let commands = port.commands.lock().expect("commands");
            let LegacyFolderCrudMutationV1::Update { parent_id, .. } =
                commands.last().expect("command").mutation()
            else {
                panic!("update")
            };
            assert_eq!(*parent_id, expected);
        }
    }

    #[tokio::test]
    async fn validation_rejects_blank_names_bad_colors_and_oversized_public_page() {
        let port = FakePort::default();
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::MobileCreate {
                    name: "   ".into(),
                    color: None,
                }))
                .await,
            Err(LegacyFolderCrudErrorV1::InvalidInput)
        );
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::RpcCreate {
                    name: "Folder".into(),
                    color: "purple".into(),
                    public: None,
                    space_id: None,
                    parent_id: None,
                }))
                .await,
            Err(LegacyFolderCrudErrorV1::InvalidInput)
        );
        let page = LegacyFolderPublicPagePatchV1 {
            title: Some("x".repeat(LEGACY_PUBLIC_PAGE_TITLE_MAX_UTF16_CODE_UNITS + 1)),
            ..LegacyFolderPublicPagePatchV1::default()
        };
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::RpcUpdate {
                    folder_id: cap_id(6),
                    name: None,
                    color: None,
                    public: None,
                    public_page: Some(page),
                    parent_id: LegacyFolderParentPatchV1::Absent,
                }))
                .await,
            Err(LegacyFolderCrudErrorV1::InvalidInput)
        );
    }

    #[tokio::test]
    async fn rpc_names_preserve_empty_controls_and_mysql_character_bounds() {
        let port = FakePort::default();
        for name in [
            String::new(),
            "\u{0000}\u{0007}control".into(),
            "😀".repeat(255),
        ] {
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::RpcCreate {
                    name,
                    color: "normal".into(),
                    public: None,
                    space_id: None,
                    parent_id: None,
                }))
                .await
                .expect("plain Schema.String and MySQL VARCHAR-compatible name");
        }
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::RpcUpdate {
                    folder_id: cap_id(10),
                    name: Some("😀".repeat(LEGACY_FOLDER_NAME_MAX_CHARACTERS + 1)),
                    color: None,
                    public: None,
                    public_page: None,
                    parent_id: LegacyFolderParentPatchV1::Absent,
                }))
                .await,
            Err(LegacyFolderCrudErrorV1::Database)
        );
        let commands = port.commands.lock().expect("commands");
        let LegacyFolderCrudMutationV1::Create { name, .. } = commands[0].mutation() else {
            panic!("create")
        };
        assert!(name.is_empty());
        let LegacyFolderCrudMutationV1::Create { name, .. } = commands[1].mutation() else {
            panic!("create")
        };
        assert_eq!(name, "\u{0000}\u{0007}control");
    }

    #[tokio::test]
    async fn mobile_uses_ecmascript_trim_and_public_page_uses_utf16_lengths() {
        let port = FakePort::default();
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::MobileCreate {
                    name: "\u{feff}\u{3000}".into(),
                    color: None,
                }))
                .await,
            Err(LegacyFolderCrudErrorV1::InvalidInput)
        );
        let accepted = LegacyFolderPublicPagePatchV1 {
            title: Some("😀".repeat(40)),
            ..LegacyFolderPublicPagePatchV1::default()
        };
        LegacyFolderCrudAdapterV1::new(&port)
            .execute(request(LegacyFolderCrudInputV1::RpcUpdate {
                folder_id: cap_id(11),
                name: None,
                color: None,
                public: None,
                public_page: Some(accepted),
                parent_id: LegacyFolderParentPatchV1::Absent,
            }))
            .await
            .expect("80 UTF-16 code units");
        let rejected = LegacyFolderPublicPagePatchV1 {
            title: Some("😀".repeat(41)),
            ..LegacyFolderPublicPagePatchV1::default()
        };
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(request(LegacyFolderCrudInputV1::RpcUpdate {
                    folder_id: cap_id(12),
                    name: None,
                    color: None,
                    public: None,
                    public_page: Some(rejected),
                    parent_id: LegacyFolderParentPatchV1::Absent,
                }))
                .await,
            Err(LegacyFolderCrudErrorV1::InvalidInput)
        );
    }

    #[test]
    fn explicit_create_idempotency_key_derives_the_same_folder_identity() {
        let first = prepare(request(LegacyFolderCrudInputV1::MobileCreate {
            name: "Folder".into(),
            color: None,
        }))
        .expect("first");
        let second = prepare(request(LegacyFolderCrudInputV1::MobileCreate {
            name: "Folder".into(),
            color: None,
        }))
        .expect("second");
        let (
            LegacyFolderCrudMutationV1::Create {
                legacy_folder_id: first_id,
                ..
            },
            LegacyFolderCrudMutationV1::Create {
                legacy_folder_id: second_id,
                ..
            },
        ) = (first.mutation(), second.mutation())
        else {
            panic!("create")
        };
        assert_eq!(first_id, second_id);
        assert_eq!(first.request_digest(), second.request_digest());
    }

    #[tokio::test]
    async fn delete_forbids_a_wire_key_and_generates_its_atomic_key_internally() {
        let port = FakePort::default();
        let mut delete = request(LegacyFolderCrudInputV1::RpcDelete {
            folder_id: cap_id(7),
        });
        delete.idempotency_key = None;
        assert!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(delete)
                .await
                .is_ok()
        );
        let mut explicit = request(LegacyFolderCrudInputV1::RpcDelete {
            folder_id: cap_id(8),
        });
        explicit.idempotency_key = Some("caller-supplied-key".into());
        assert_eq!(
            LegacyFolderCrudAdapterV1::new(&port)
                .execute(explicit)
                .await,
            Err(LegacyFolderCrudErrorV1::InvalidInput)
        );
    }

    #[tokio::test]
    async fn arbitrary_effect_branded_ids_reach_the_atomic_policy_boundary() {
        let port = FakePort::default();
        let mut delete = request(LegacyFolderCrudInputV1::RpcDelete {
            folder_id: "not-a-cap-nanoid".into(),
        });
        delete.idempotency_key = None;
        LegacyFolderCrudAdapterV1::new(&port)
            .execute(delete)
            .await
            .expect("schema-branded string reaches the port");
        let commands = port.commands.lock().expect("commands");
        let LegacyFolderCrudMutationV1::Delete { folder_id } = commands[0].mutation() else {
            panic!("delete command")
        };
        assert_eq!(folder_id.to_string(), mapped_source_id("not-a-cap-nanoid"));
        assert_eq!(mapped_source_id("not-a-cap-nanoid").len(), 36);
    }

    #[tokio::test]
    async fn atomic_failures_project_to_stable_public_classes() {
        for (atomic, public) in [
            (
                LegacyFolderCrudAtomicErrorV1::TargetMissing,
                LegacyFolderCrudErrorV1::NotFound,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::ParentMissing,
                LegacyFolderCrudErrorV1::ParentNotFound,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::RecursiveDefinition,
                LegacyFolderCrudErrorV1::RecursiveDefinition,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::ScopeConflict,
                LegacyFolderCrudErrorV1::ScopeConflict,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::AccessDenied,
                LegacyFolderCrudErrorV1::AccessDenied,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::IdempotencyConflict,
                LegacyFolderCrudErrorV1::Conflict,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::Unavailable,
                LegacyFolderCrudErrorV1::Unavailable,
            ),
            (
                LegacyFolderCrudAtomicErrorV1::Corrupt,
                LegacyFolderCrudErrorV1::Internal,
            ),
        ] {
            let port = FakePort::default();
            *port.error.lock().expect("error") = Some(atomic);
            let mut delete = request(LegacyFolderCrudInputV1::RpcDelete {
                folder_id: cap_id(8),
            });
            delete.idempotency_key = None;
            assert_eq!(
                LegacyFolderCrudAdapterV1::new(&port).execute(delete).await,
                Err(public)
            );
        }
    }

    #[test]
    fn command_debug_and_key_storage_do_not_expose_raw_material() {
        let mut delete = request(LegacyFolderCrudInputV1::RpcDelete {
            folder_id: cap_id(9),
        });
        delete.idempotency_key = None;
        let command = prepare(delete).expect("command");
        let rendered = format!("{command:?}");
        assert!(!rendered.contains("folder-request-0001"));
        assert!(!rendered.contains(&cap_id(9)));
        assert_eq!(command.request_digest_hex().len(), 64);
        assert_eq!(command.idempotency_key_digest_hex().len(), 64);
    }
}
