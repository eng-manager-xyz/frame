//! Source-pinned, provider-free contracts for Cap's developer-dashboard writes.
//!
//! The legacy actions are user-owned rather than organization-owned. They also
//! combine several subtle wire behaviours: Cap `NanoID`s, an empty update patch
//! that still succeeds, a nullable logo patch, permissive full-origin parsing,
//! zero-row domain/video mutations that still succeed, and API credentials that
//! are revealed exactly once. Frame preserves those observable behaviours while
//! moving session authority, one-use browser-proof consumption, storage
//! postconditions, audit effects, and idempotency into one atomic boundary.
//!
//! Raw API keys never enter a command, fingerprint, audit record, `Debug`
//! implementation, or durable mutation receipt. An injected authority generates
//! and protects key material. The atomic port journals only an encrypted replay
//! envelope and the application asks the same authority to reveal that envelope
//! after either an applied mutation or a replay. Thus an uncertain first response
//! can be retried without rotating keys or returning different credentials.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    IdempotencyKey, LegacyCapNanoId, MAX_LEDGER_AMOUNT, SecretDigest, SessionId,
    SessionMutationGrantId, TimestampMillis, UserId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;

pub const LEGACY_DEVELOPER_ACTIONS_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID: &str = "cap-v1-f303e703a4237888";
pub const LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID: &str = "cap-v1-87fd6af55b891cb9";
pub const LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID: &str = "cap-v1-9833b16bb80a3299";
pub const LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID: &str = "cap-v1-aa86dd3d5351ec06";
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID: &str = "cap-v1-f7d8036af53d0eb9";
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID: &str = "cap-v1-1f1465957551f1c4";
pub const LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID: &str = "cap-v1-8328214ed9647abb";
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID: &str = "cap-v1-b822700b545118f6";

pub const LEGACY_CREATE_DEVELOPER_APP_IDENTITY: &str =
    "action://apps/web/actions/developers/create-app.ts#createDeveloperApp";
pub const LEGACY_UPDATE_DEVELOPER_APP_IDENTITY: &str =
    "action://apps/web/actions/developers/update-app.ts#updateDeveloperApp";
pub const LEGACY_DELETE_DEVELOPER_APP_IDENTITY: &str =
    "action://apps/web/actions/developers/delete-app.ts#deleteDeveloperApp";
pub const LEGACY_ADD_DEVELOPER_DOMAIN_IDENTITY: &str =
    "action://apps/web/actions/developers/add-domain.ts#addDeveloperDomain";
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_IDENTITY: &str =
    "action://apps/web/actions/developers/remove-domain.ts#removeDeveloperDomain";
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_IDENTITY: &str =
    "action://apps/web/actions/developers/regenerate-keys.ts#regenerateDeveloperKeys";
pub const LEGACY_DELETE_DEVELOPER_VIDEO_IDENTITY: &str =
    "action://apps/web/actions/developers/delete-video.ts#deleteDeveloperVideo";
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_IDENTITY: &str =
    "action://apps/web/actions/developers/update-auto-topup.ts#updateDeveloperAutoTopUp";

pub const LEGACY_CREATE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256: &str =
    "994d8a71989f837e3cbbff15f97ef2ad58ea27651cb398db473224c8df01b9b9";
pub const LEGACY_UPDATE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256: &str =
    "7925fd09260f34535054c05622c82fe11c0ece86a58173acbd477b9fb5253592";
pub const LEGACY_DELETE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256: &str =
    "c82200a4c80e568c200d0aa283dba5299996b126f4a8fdaa0ce7ff60f883afe8";
pub const LEGACY_ADD_DEVELOPER_DOMAIN_SOURCE_MANIFEST_SHA256: &str =
    "56a5d84404a70427ee75357f5d8fe5d780a9befa64922c435b57df4c0c2b6694";
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_SOURCE_MANIFEST_SHA256: &str =
    "1a8e9038188941a830eb48a4e3085f6945a0e2b5f583fab2215bbac47e46b0b0";
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_SOURCE_MANIFEST_SHA256: &str =
    "b2abca5e8463be69784d20084ed87653d29d263b9d0f9114e5589d2b9fabc71e";
pub const LEGACY_DELETE_DEVELOPER_VIDEO_SOURCE_MANIFEST_SHA256: &str =
    "9f54bdb8196b97efea5bacff931e201cd34000f0a8177d82826b798f1dea682c";
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_SOURCE_MANIFEST_SHA256: &str =
    "20e9d18a2883ddd43b570ce0e4521db4d09612ed8da0511eae2ef7978c6d45f4";

pub const LEGACY_DEVELOPER_POLICY: &str = "developer_dashboard_owner.v1";
pub const LEGACY_DEVELOPER_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_DEVELOPER_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_DEVELOPER_DASHBOARD_REVALIDATION_PATH: &str = "/dashboard/developers";
pub const LEGACY_DEVELOPER_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"];
pub const LEGACY_DEVELOPER_MAX_APP_NAME_CHARS: usize = 255;
pub const LEGACY_DEVELOPER_MAX_LOGO_URL_CHARS: usize = 1024;
pub const LEGACY_DEVELOPER_MAX_DOMAIN_CHARS: usize = 253;
pub const LEGACY_DEVELOPER_MAX_TOP_UP_CENTS: i64 = 100_000;
pub const LEGACY_DEVELOPER_KEY_BODY_LENGTH: usize = 30;
pub const LEGACY_DEVELOPER_KEY_PREFIX_LENGTH: usize = 12;
pub const LEGACY_DEVELOPER_MAX_PROTECTED_BLOB_BYTES: usize = 16 * 1024;
const LEGACY_DEVELOPER_LONG_NANOID_ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperSourceRoleV1 {
    Action,
    Caller,
    UnitTest,
    ReadProjection,
    Session,
    Schema,
    Identifier,
    Database,
    Crypto,
    KeyHash,
    Environment,
    DependencyDeclaration,
    DependencyLock,
}

impl LegacyDeveloperSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Caller => "caller",
            Self::UnitTest => "unit_test",
            Self::ReadProjection => "read_projection",
            Self::Session => "session",
            Self::Schema => "schema",
            Self::Identifier => "identifier",
            Self::Database => "database",
            Self::Crypto => "crypto",
            Self::KeyHash => "key_hash",
            Self::Environment => "environment",
            Self::DependencyDeclaration => "dependency_declaration",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
    pub role: LegacyDeveloperSourceRoleV1,
}

pub const LEGACY_CREATE_DEVELOPER_APP_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/create-app.ts",
        sha256: "d2149a30c6a3657b224458dd946b9d621f5fb3b1f84b4293ffd84549738c4a0b",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_UPDATE_DEVELOPER_APP_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/update-app.ts",
        sha256: "41a00c87464b6d799ae93bcb5d44b0bc5dfec6adb320ff7e0b93841bb1adb025",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_DELETE_DEVELOPER_APP_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/delete-app.ts",
        sha256: "c708ff132594d8523c160c978ab27812f4eec2afc2ec8cba977d6afe17eb7dcc",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/add-domain.ts",
        sha256: "d25987a9c3a0eb4df30576e9e1a1ca21b96876bfb16d9e152f75a96225ee795f",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/remove-domain.ts",
        sha256: "7e50b46b02a212315ed60ce357ceba12356cbb05e6193d04c61abc745ddfddee",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/regenerate-keys.ts",
        sha256: "a64dcc1684ef8327f2d953590e307c83fcf0cd23fc5a604233878bca4e0c46c4",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/delete-video.ts",
        sha256: "63d75809a7be974610908e70aee859173d645c64ed997c24889e4b132425fe16",
        role: LegacyDeveloperSourceRoleV1::Action,
    };
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_SOURCE: LegacyDeveloperSourcePinV1 =
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/actions/developers/update-auto-topup.ts",
        sha256: "9e6882d1de03d4418ab45286f7c2d0b5bb17f073062955627e884a0774967420",
        role: LegacyDeveloperSourceRoleV1::Action,
    };

pub const LEGACY_CREATE_DEVELOPER_APP_CALLERS: &[LegacyDeveloperSourcePinV1] =
    &[LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/_components/CreateAppDialog.tsx",
        sha256: "f8ccf4849312d0359af406d79bdc0436294204eb608484249164bb5bed10e2b2",
        role: LegacyDeveloperSourceRoleV1::Caller,
    }];
pub const LEGACY_UPDATE_AND_DELETE_DEVELOPER_APP_CALLERS: &[LegacyDeveloperSourcePinV1] =
    &[LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/apps/[appId]/settings/AppSettingsClient.tsx",
        sha256: "3c12fbdc9d52f487c5ccfb4973630bfd187741d56f93c4ac2d8f00f09097b6eb",
        role: LegacyDeveloperSourceRoleV1::Caller,
    }];
pub const LEGACY_ADD_DEVELOPER_DOMAIN_CALLERS: &[LegacyDeveloperSourcePinV1] =
    &[LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/apps/[appId]/domains/DomainsClient.tsx",
        sha256: "d001d24859dea17a89f5ef39d89c5bbd00ba65662c1bd1961c0f5a4ca163e61d",
        role: LegacyDeveloperSourceRoleV1::Caller,
    }];
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_CALLERS: &[LegacyDeveloperSourcePinV1] =
    &[LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/_components/DomainRow.tsx",
        sha256: "1aec74bbd8388993894f0bfe06763761a027f5c749997521ee998d1550d3aa90",
        role: LegacyDeveloperSourceRoleV1::Caller,
    }];
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_CALLERS: &[LegacyDeveloperSourcePinV1] =
    &[LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/apps/[appId]/api-keys/ApiKeysClient.tsx",
        sha256: "202260b8aeebd6bf449cd99359a6d3d6bab9f756cc7db655cd9a94d46218c312",
        role: LegacyDeveloperSourceRoleV1::Caller,
    }];
pub const LEGACY_DELETE_DEVELOPER_VIDEO_CALLERS: &[LegacyDeveloperSourcePinV1] =
    &[LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/apps/[appId]/videos/VideosClient.tsx",
        sha256: "2312fef40823f173c695bc157f4c1d586833f1a2aa351b98e1a98b5cc904b5de",
        role: LegacyDeveloperSourceRoleV1::Caller,
    }];
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_CALLERS: &[LegacyDeveloperSourcePinV1] = &[];

pub const LEGACY_DEVELOPER_COMMON_SUPPORTING_SOURCES: &[LegacyDeveloperSourcePinV1] = &[
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/__tests__/unit/developer-actions.test.ts",
        sha256: "8bdac7dc68cf8a76476333e2c2875bde863e9a6f6076c4d010a1b62a43d09552",
        role: LegacyDeveloperSourceRoleV1::UnitTest,
    },
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/developer-data.ts",
        sha256: "74e819f058c1fc88fe0ee4af1cd52428c6074f29485e1991f5d66b97297c6d07",
        role: LegacyDeveloperSourceRoleV1::ReadProjection,
    },
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/developers/layout.tsx",
        sha256: "ea20cbedbf18cd564b74efbeb551586fd468d95530aea134a013d2c1e62ada7c",
        role: LegacyDeveloperSourceRoleV1::Session,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyDeveloperSourceRoleV1::Session,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyDeveloperSourceRoleV1::Session,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyDeveloperSourceRoleV1::Schema,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyDeveloperSourceRoleV1::Identifier,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/index.ts",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyDeveloperSourceRoleV1::Database,
    },
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/package.json",
        sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
        role: LegacyDeveloperSourceRoleV1::DependencyDeclaration,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/package.json",
        sha256: "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
        role: LegacyDeveloperSourceRoleV1::DependencyDeclaration,
    },
    LegacyDeveloperSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyDeveloperSourceRoleV1::DependencyLock,
    },
];

pub const LEGACY_DEVELOPER_SECRET_SUPPORTING_SOURCES: &[LegacyDeveloperSourcePinV1] = &[
    LegacyDeveloperSourcePinV1 {
        path: "apps/web/lib/developer-key-hash.ts",
        sha256: "ecc93fc2828647aeaa88dcb9dda0cb2fbcb8b87d4f1a326476878834c06620b1",
        role: LegacyDeveloperSourceRoleV1::KeyHash,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/database/crypto.ts",
        sha256: "d547c7ba0f984d1e625d807e4a1e64cfb400ed2fcc796cf9f6e43713805efb6f",
        role: LegacyDeveloperSourceRoleV1::Crypto,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/env/index.ts",
        sha256: "c15990c4bfb98c65518003ba9692dd8d2c173c36e78991be1f519cce89e96dc9",
        role: LegacyDeveloperSourceRoleV1::Environment,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/env/server.ts",
        sha256: "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
        role: LegacyDeveloperSourceRoleV1::Environment,
    },
    LegacyDeveloperSourcePinV1 {
        path: "packages/env/package.json",
        sha256: "4a12ca3b40acec2340015815c2517b0513ee1024ad0832c80fd8824a9d7948f2",
        role: LegacyDeveloperSourceRoleV1::DependencyDeclaration,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperSourceClosureV1 {
    pub action: LegacyDeveloperSourcePinV1,
    pub callers: &'static [LegacyDeveloperSourcePinV1],
    pub supporting: &'static [LegacyDeveloperSourcePinV1],
    pub secret_supporting: &'static [LegacyDeveloperSourcePinV1],
}

impl LegacyDeveloperSourceClosureV1 {
    #[must_use]
    pub const fn source_count(self) -> usize {
        1 + self.callers.len() + self.supporting.len() + self.secret_supporting.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperActionV1 {
    CreateApp,
    UpdateApp,
    DeleteApp,
    AddDomain,
    RemoveDomain,
    RegenerateKeys,
    DeleteVideo,
    UpdateAutoTopUp,
}

impl LegacyDeveloperActionV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::CreateApp => "create_app",
            Self::UpdateApp => "update_app",
            Self::DeleteApp => "delete_app",
            Self::AddDomain => "add_domain",
            Self::RemoveDomain => "remove_domain",
            Self::RegenerateKeys => "regenerate_keys",
            Self::DeleteVideo => "delete_video",
            Self::UpdateAutoTopUp => "update_auto_top_up",
        }
    }

    const fn fingerprint_tag(self) -> u8 {
        match self {
            Self::CreateApp => 0,
            Self::UpdateApp => 1,
            Self::DeleteApp => 2,
            Self::AddDomain => 3,
            Self::RemoveDomain => 4,
            Self::RegenerateKeys => 5,
            Self::DeleteVideo => 6,
            Self::UpdateAutoTopUp => 7,
        }
    }

    #[must_use]
    pub const fn requires_secret_generation(self) -> bool {
        matches!(self, Self::CreateApp | Self::RegenerateKeys)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperObservedInputV1 {
    NameAndEnvironmentObject,
    OptionalNameEnvironmentAndNullableLogoObject,
    AppIdentifier,
    PositionalAppAndFullOrigin,
    PositionalAppAndDomainIdentifier,
    PositionalAppAndVideoIdentifier,
    AutoTopUpPatchObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperObservedSuccessV1 {
    AppIdentifierAndPlaintextKeyPair,
    PlaintextKeyPair,
    SuccessTrueObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperRequiredMutationV1 {
    InsertOwnedAppTwoKeysAndCreditAccount,
    PatchOnlyPresentFieldsOrNoOp,
    RevokeActiveKeysAndSoftDeleteApp,
    InsertNormalizedOrigin,
    DeleteMatchingDomainIgnoringAffectedCount,
    RevokeActiveKeysAndInsertTwoKeys,
    SoftDeleteMatchingVideoIgnoringAffectedCount,
    PatchEnabledAndOnlyPresentAutoTopUpFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub source_manifest_sha256: &'static str,
    pub source_closure: LegacyDeveloperSourceClosureV1,
    pub authentication: &'static str,
    pub policy: &'static str,
    pub content_type: &'static str,
    pub max_body_bytes: usize,
    pub observed_input: LegacyDeveloperObservedInputV1,
    pub observed_success: LegacyDeveloperObservedSuccessV1,
    pub required_mutation: LegacyDeveloperRequiredMutationV1,
    pub user_owner_non_disclosure: bool,
    pub idempotency_required: bool,
    pub one_use_browser_proof_required: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

const fn source_closure(
    action: LegacyDeveloperSourcePinV1,
    callers: &'static [LegacyDeveloperSourcePinV1],
    secret: bool,
) -> LegacyDeveloperSourceClosureV1 {
    LegacyDeveloperSourceClosureV1 {
        action,
        callers,
        supporting: LEGACY_DEVELOPER_COMMON_SUPPORTING_SOURCES,
        secret_supporting: if secret {
            LEGACY_DEVELOPER_SECRET_SUPPORTING_SOURCES
        } else {
            &[]
        },
    }
}

const fn profile(
    operation_id: &'static str,
    identity: &'static str,
    manifest: &'static str,
    source_closure: LegacyDeveloperSourceClosureV1,
    input: LegacyDeveloperObservedInputV1,
    success: LegacyDeveloperObservedSuccessV1,
    mutation: LegacyDeveloperRequiredMutationV1,
) -> LegacyDeveloperProfileV1 {
    LegacyDeveloperProfileV1 {
        operation_id,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: identity,
        pinned_commit: LEGACY_DEVELOPER_ACTIONS_CAP_COMMIT,
        source_manifest_sha256: manifest,
        source_closure,
        authentication: "session",
        policy: LEGACY_DEVELOPER_POLICY,
        content_type: LEGACY_DEVELOPER_CONTENT_TYPE,
        max_body_bytes: LEGACY_DEVELOPER_MAX_BODY_BYTES,
        observed_input: input,
        observed_success: success,
        required_mutation: mutation,
        user_owner_non_disclosure: true,
        idempotency_required: true,
        one_use_browser_proof_required: true,
        protected_gates: LEGACY_DEVELOPER_PROTECTED_GATES,
        production_promoted: false,
    }
}

pub const LEGACY_CREATE_DEVELOPER_APP_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID,
    LEGACY_CREATE_DEVELOPER_APP_IDENTITY,
    LEGACY_CREATE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_CREATE_DEVELOPER_APP_ACTION_SOURCE,
        LEGACY_CREATE_DEVELOPER_APP_CALLERS,
        true,
    ),
    LegacyDeveloperObservedInputV1::NameAndEnvironmentObject,
    LegacyDeveloperObservedSuccessV1::AppIdentifierAndPlaintextKeyPair,
    LegacyDeveloperRequiredMutationV1::InsertOwnedAppTwoKeysAndCreditAccount,
);
pub const LEGACY_UPDATE_DEVELOPER_APP_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID,
    LEGACY_UPDATE_DEVELOPER_APP_IDENTITY,
    LEGACY_UPDATE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_UPDATE_DEVELOPER_APP_ACTION_SOURCE,
        LEGACY_UPDATE_AND_DELETE_DEVELOPER_APP_CALLERS,
        false,
    ),
    LegacyDeveloperObservedInputV1::OptionalNameEnvironmentAndNullableLogoObject,
    LegacyDeveloperObservedSuccessV1::SuccessTrueObject,
    LegacyDeveloperRequiredMutationV1::PatchOnlyPresentFieldsOrNoOp,
);
pub const LEGACY_DELETE_DEVELOPER_APP_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID,
    LEGACY_DELETE_DEVELOPER_APP_IDENTITY,
    LEGACY_DELETE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_DELETE_DEVELOPER_APP_ACTION_SOURCE,
        LEGACY_UPDATE_AND_DELETE_DEVELOPER_APP_CALLERS,
        false,
    ),
    LegacyDeveloperObservedInputV1::AppIdentifier,
    LegacyDeveloperObservedSuccessV1::SuccessTrueObject,
    LegacyDeveloperRequiredMutationV1::RevokeActiveKeysAndSoftDeleteApp,
);
pub const LEGACY_ADD_DEVELOPER_DOMAIN_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID,
    LEGACY_ADD_DEVELOPER_DOMAIN_IDENTITY,
    LEGACY_ADD_DEVELOPER_DOMAIN_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_SOURCE,
        LEGACY_ADD_DEVELOPER_DOMAIN_CALLERS,
        false,
    ),
    LegacyDeveloperObservedInputV1::PositionalAppAndFullOrigin,
    LegacyDeveloperObservedSuccessV1::SuccessTrueObject,
    LegacyDeveloperRequiredMutationV1::InsertNormalizedOrigin,
);
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID,
    LEGACY_REMOVE_DEVELOPER_DOMAIN_IDENTITY,
    LEGACY_REMOVE_DEVELOPER_DOMAIN_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_SOURCE,
        LEGACY_REMOVE_DEVELOPER_DOMAIN_CALLERS,
        false,
    ),
    LegacyDeveloperObservedInputV1::PositionalAppAndDomainIdentifier,
    LegacyDeveloperObservedSuccessV1::SuccessTrueObject,
    LegacyDeveloperRequiredMutationV1::DeleteMatchingDomainIgnoringAffectedCount,
);
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID,
    LEGACY_REGENERATE_DEVELOPER_KEYS_IDENTITY,
    LEGACY_REGENERATE_DEVELOPER_KEYS_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_SOURCE,
        LEGACY_REGENERATE_DEVELOPER_KEYS_CALLERS,
        true,
    ),
    LegacyDeveloperObservedInputV1::AppIdentifier,
    LegacyDeveloperObservedSuccessV1::PlaintextKeyPair,
    LegacyDeveloperRequiredMutationV1::RevokeActiveKeysAndInsertTwoKeys,
);
pub const LEGACY_DELETE_DEVELOPER_VIDEO_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID,
    LEGACY_DELETE_DEVELOPER_VIDEO_IDENTITY,
    LEGACY_DELETE_DEVELOPER_VIDEO_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_SOURCE,
        LEGACY_DELETE_DEVELOPER_VIDEO_CALLERS,
        false,
    ),
    LegacyDeveloperObservedInputV1::PositionalAppAndVideoIdentifier,
    LegacyDeveloperObservedSuccessV1::SuccessTrueObject,
    LegacyDeveloperRequiredMutationV1::SoftDeleteMatchingVideoIgnoringAffectedCount,
);
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_PROFILE: LegacyDeveloperProfileV1 = profile(
    LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID,
    LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_IDENTITY,
    LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_SOURCE_MANIFEST_SHA256,
    source_closure(
        LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_SOURCE,
        LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_CALLERS,
        false,
    ),
    LegacyDeveloperObservedInputV1::AutoTopUpPatchObject,
    LegacyDeveloperObservedSuccessV1::SuccessTrueObject,
    LegacyDeveloperRequiredMutationV1::PatchEnabledAndOnlyPresentAutoTopUpFields,
);

pub const LEGACY_DEVELOPER_PROFILES: &[LegacyDeveloperProfileV1] = &[
    LEGACY_CREATE_DEVELOPER_APP_PROFILE,
    LEGACY_UPDATE_DEVELOPER_APP_PROFILE,
    LEGACY_DELETE_DEVELOPER_APP_PROFILE,
    LEGACY_ADD_DEVELOPER_DOMAIN_PROFILE,
    LEGACY_REMOVE_DEVELOPER_DOMAIN_PROFILE,
    LEGACY_REGENERATE_DEVELOPER_KEYS_PROFILE,
    LEGACY_DELETE_DEVELOPER_VIDEO_PROFILE,
    LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_PROFILE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperEnvironmentV1 {
    Development,
    Production,
}

impl LegacyDeveloperEnvironmentV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyDeveloperNullableLogoPatchV1 {
    Missing,
    Null,
    Value(String),
}

impl fmt::Debug for LegacyDeveloperNullableLogoPatchV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Missing",
            Self::Null => "Null",
            Self::Value(_) => "Value([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyDeveloperInputV1 {
    CreateApp {
        name: String,
        environment: LegacyDeveloperEnvironmentV1,
    },
    UpdateApp {
        legacy_app_id: String,
        name: Option<String>,
        environment: Option<LegacyDeveloperEnvironmentV1>,
        logo_url: LegacyDeveloperNullableLogoPatchV1,
    },
    DeleteApp {
        legacy_app_id: String,
    },
    AddDomain {
        legacy_app_id: String,
        domain: String,
    },
    RemoveDomain {
        legacy_app_id: String,
        legacy_domain_id: String,
    },
    RegenerateKeys {
        legacy_app_id: String,
    },
    DeleteVideo {
        legacy_app_id: String,
        legacy_video_id: String,
    },
    UpdateAutoTopUp {
        legacy_app_id: String,
        enabled: bool,
        threshold_micro_credits: Option<i64>,
        amount_cents: Option<i64>,
    },
}

impl LegacyDeveloperInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyDeveloperActionV1 {
        match self {
            Self::CreateApp { .. } => LegacyDeveloperActionV1::CreateApp,
            Self::UpdateApp { .. } => LegacyDeveloperActionV1::UpdateApp,
            Self::DeleteApp { .. } => LegacyDeveloperActionV1::DeleteApp,
            Self::AddDomain { .. } => LegacyDeveloperActionV1::AddDomain,
            Self::RemoveDomain { .. } => LegacyDeveloperActionV1::RemoveDomain,
            Self::RegenerateKeys { .. } => LegacyDeveloperActionV1::RegenerateKeys,
            Self::DeleteVideo { .. } => LegacyDeveloperActionV1::DeleteVideo,
            Self::UpdateAutoTopUp { .. } => LegacyDeveloperActionV1::UpdateAutoTopUp,
        }
    }
}

impl fmt::Debug for LegacyDeveloperInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(self.action().stable_code())
            .field("values", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperRequestV1 {
    pub credential: Option<LegacyDeveloperCredentialV1>,
    pub actor_id: Option<UserId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyDeveloperInputV1,
}

impl fmt::Debug for LegacyDeveloperRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperRequestV1")
            .field("credential", &self.credential)
            .field("actor", &self.actor_id.map(|_| "<redacted>"))
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("input", &self.input)
            .finish()
    }
}

macro_rules! legacy_developer_id {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(LegacyCapNanoId);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, LegacyDeveloperErrorV1> {
                LegacyCapNanoId::parse(value)
                    .map(Self)
                    .map_err(|_| LegacyDeveloperErrorV1::Invalid)
            }

            #[must_use]
            pub fn legacy_value(&self) -> &str {
                self.0.as_str()
            }

            #[must_use]
            pub fn mapped_uuid(&self) -> String {
                self.0.mapped_uuid().to_string()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}

legacy_developer_id!(LegacyDeveloperAppIdV1);
legacy_developer_id!(LegacyDeveloperDomainIdV1);
legacy_developer_id!(LegacyDeveloperVideoIdV1);
legacy_developer_id!(LegacyDeveloperApiKeyIdV1);
legacy_developer_id!(LegacyDeveloperCreditAccountIdV1);

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperAppPatchV1 {
    name: Option<String>,
    environment: Option<LegacyDeveloperEnvironmentV1>,
    logo_url: LegacyDeveloperNullableLogoPatchV1,
}

impl LegacyDeveloperAppPatchV1 {
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub const fn environment(&self) -> Option<LegacyDeveloperEnvironmentV1> {
        self.environment
    }

    #[must_use]
    pub const fn logo_url(&self) -> &LegacyDeveloperNullableLogoPatchV1 {
        &self.logo_url
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.environment.is_none()
            && matches!(self.logo_url, LegacyDeveloperNullableLogoPatchV1::Missing)
    }
}

impl fmt::Debug for LegacyDeveloperAppPatchV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperAppPatchV1")
            .field("name_present", &self.name.is_some())
            .field("environment", &self.environment)
            .field("logo_url", &self.logo_url)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperAutoTopUpPatchV1 {
    enabled: bool,
    threshold_micro_credits: Option<u64>,
    amount_cents: Option<u32>,
}

impl LegacyDeveloperAutoTopUpPatchV1 {
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn threshold_micro_credits(self) -> Option<u64> {
        self.threshold_micro_credits
    }

    #[must_use]
    pub const fn amount_cents(self) -> Option<u32> {
        self.amount_cents
    }
}

impl fmt::Debug for LegacyDeveloperAutoTopUpPatchV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperAutoTopUpPatchV1")
            .field("enabled", &self.enabled)
            .field(
                "threshold_micro_credits_present",
                &self.threshold_micro_credits.is_some(),
            )
            .field("amount_cents_present", &self.amount_cents.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperAuthorityV1 {
    actor_id: UserId,
}

impl LegacyDeveloperAuthorityV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }
}

impl fmt::Debug for LegacyDeveloperAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyDeveloperAuthorityV1([redacted])")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyDeveloperBrowserFenceV1 {
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

    #[cfg(test)]
    fn fixture(actor_id: UserId) -> Self {
        Self {
            mutation_grant_id: SessionMutationGrantId::new(),
            session_id: SessionId::new(),
            actor_id,
        }
    }
}

impl fmt::Debug for LegacyDeveloperBrowserFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyDeveloperBrowserFenceV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperFenceV1 {
    authority: LegacyDeveloperAuthorityV1,
    idempotency_key: IdempotencyKey,
    request_fingerprint: [u8; 32],
}

impl LegacyDeveloperFenceV1 {
    #[must_use]
    pub const fn authority(&self) -> &LegacyDeveloperAuthorityV1 {
        &self.authority
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }
}

impl fmt::Debug for LegacyDeveloperFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperFenceV1")
            .field("authority", &self.authority)
            .field("idempotency_key", &"<redacted>")
            .field("request_fingerprint", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyDeveloperCommandV1 {
    CreateApp {
        fence: LegacyDeveloperFenceV1,
        name: String,
        environment: LegacyDeveloperEnvironmentV1,
    },
    UpdateApp {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
        patch: LegacyDeveloperAppPatchV1,
    },
    DeleteApp {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
    },
    AddDomain {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
        normalized_origin: String,
    },
    RemoveDomain {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
        domain_id: LegacyDeveloperDomainIdV1,
    },
    RegenerateKeys {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
    },
    DeleteVideo {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
        video_id: LegacyDeveloperVideoIdV1,
    },
    UpdateAutoTopUp {
        fence: LegacyDeveloperFenceV1,
        app_id: LegacyDeveloperAppIdV1,
        patch: LegacyDeveloperAutoTopUpPatchV1,
    },
}

impl LegacyDeveloperCommandV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyDeveloperActionV1 {
        match self {
            Self::CreateApp { .. } => LegacyDeveloperActionV1::CreateApp,
            Self::UpdateApp { .. } => LegacyDeveloperActionV1::UpdateApp,
            Self::DeleteApp { .. } => LegacyDeveloperActionV1::DeleteApp,
            Self::AddDomain { .. } => LegacyDeveloperActionV1::AddDomain,
            Self::RemoveDomain { .. } => LegacyDeveloperActionV1::RemoveDomain,
            Self::RegenerateKeys { .. } => LegacyDeveloperActionV1::RegenerateKeys,
            Self::DeleteVideo { .. } => LegacyDeveloperActionV1::DeleteVideo,
            Self::UpdateAutoTopUp { .. } => LegacyDeveloperActionV1::UpdateAutoTopUp,
        }
    }

    #[must_use]
    pub const fn fence(&self) -> &LegacyDeveloperFenceV1 {
        match self {
            Self::CreateApp { fence, .. }
            | Self::UpdateApp { fence, .. }
            | Self::DeleteApp { fence, .. }
            | Self::AddDomain { fence, .. }
            | Self::RemoveDomain { fence, .. }
            | Self::RegenerateKeys { fence, .. }
            | Self::DeleteVideo { fence, .. }
            | Self::UpdateAutoTopUp { fence, .. } => fence,
        }
    }

    #[must_use]
    pub const fn app_id(&self) -> Option<&LegacyDeveloperAppIdV1> {
        match self {
            Self::CreateApp { .. } => None,
            Self::UpdateApp { app_id, .. }
            | Self::DeleteApp { app_id, .. }
            | Self::AddDomain { app_id, .. }
            | Self::RemoveDomain { app_id, .. }
            | Self::RegenerateKeys { app_id, .. }
            | Self::DeleteVideo { app_id, .. }
            | Self::UpdateAutoTopUp { app_id, .. } => Some(app_id),
        }
    }

    #[must_use]
    pub fn secret_generation_context(&self) -> Option<LegacyDeveloperSecretGenerationContextV1> {
        if !self.action().requires_secret_generation() {
            return None;
        }
        Some(LegacyDeveloperSecretGenerationContextV1 {
            action: self.action(),
            actor_id: self.fence().authority.actor_id,
            app_id: self.app_id().cloned(),
            request_fingerprint: self.fence().request_fingerprint,
        })
    }
}

impl fmt::Debug for LegacyDeveloperCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(self.action().stable_code())
            .field("fence", self.fence())
            .field("app", &self.app_id().map(|_| "<redacted>"))
            .field("payload", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperSecretGenerationContextV1 {
    action: LegacyDeveloperActionV1,
    actor_id: UserId,
    app_id: Option<LegacyDeveloperAppIdV1>,
    request_fingerprint: [u8; 32],
}

impl LegacyDeveloperSecretGenerationContextV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyDeveloperActionV1 {
        self.action
    }

    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn app_id(&self) -> Option<&LegacyDeveloperAppIdV1> {
        self.app_id.as_ref()
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    #[must_use]
    pub fn replay_binding(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"frame-legacy-developer-secret-replay-v1\0");
        hasher.update([self.action.fingerprint_tag()]);
        hash_value(&mut hasher, &self.actor_id.to_string());
        match &self.app_id {
            Some(app_id) => {
                hasher.update([1]);
                hash_value(&mut hasher, &app_id.mapped_uuid());
            }
            None => hasher.update([0]),
        }
        hasher.update(self.request_fingerprint);
        hasher.finalize().into()
    }
}

impl fmt::Debug for LegacyDeveloperSecretGenerationContextV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperSecretGenerationContextV1")
            .field("action", &self.action)
            .field("actor", &"<redacted>")
            .field("app", &self.app_id.as_ref().map(|_| "<redacted>"))
            .field("request_fingerprint", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperProtectedBlobV1(String);

impl LegacyDeveloperProtectedBlobV1 {
    pub fn new(value: impl Into<String>) -> Result<Self, LegacyDeveloperSecretErrorV1> {
        let value = value.into();
        if value.is_empty() || value.len() > LEGACY_DEVELOPER_MAX_PROTECTED_BLOB_BYTES {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        Ok(Self(value))
    }

    /// Exposes provider ciphertext only to a persistence adapter.
    #[must_use]
    pub fn expose_for_persistence(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LegacyDeveloperProtectedBlobV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyDeveloperProtectedBlobV1([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperKeyKindV1 {
    Public,
    Secret,
}

impl LegacyDeveloperKeyKindV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Secret => "secret",
        }
    }

    const fn raw_prefix(self) -> &'static str {
        match self {
            Self::Public => "cpk_",
            Self::Secret => "csk_",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperStoredKeyV1 {
    key_id: LegacyDeveloperApiKeyIdV1,
    kind: LegacyDeveloperKeyKindV1,
    key_prefix: String,
    key_hash: SecretDigest,
    encrypted_key: LegacyDeveloperProtectedBlobV1,
}

impl LegacyDeveloperStoredKeyV1 {
    pub fn new(
        key_id: LegacyDeveloperApiKeyIdV1,
        kind: LegacyDeveloperKeyKindV1,
        key_prefix: impl Into<String>,
        key_hash: impl Into<String>,
        encrypted_key: LegacyDeveloperProtectedBlobV1,
    ) -> Result<Self, LegacyDeveloperSecretErrorV1> {
        let key_prefix = key_prefix.into();
        if !valid_key_prefix(kind, &key_prefix) {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        let key_hash = SecretDigest::parse_sha256(key_hash)
            .map_err(|_| LegacyDeveloperSecretErrorV1::InvalidMaterial)?;
        Ok(Self {
            key_id,
            kind,
            key_prefix,
            key_hash,
            encrypted_key,
        })
    }

    #[must_use]
    pub const fn key_id(&self) -> &LegacyDeveloperApiKeyIdV1 {
        &self.key_id
    }

    #[must_use]
    pub const fn kind(&self) -> LegacyDeveloperKeyKindV1 {
        self.kind
    }

    #[must_use]
    pub fn key_prefix(&self) -> &str {
        &self.key_prefix
    }

    #[must_use]
    pub const fn key_hash(&self) -> &SecretDigest {
        &self.key_hash
    }

    #[must_use]
    pub const fn encrypted_key(&self) -> &LegacyDeveloperProtectedBlobV1 {
        &self.encrypted_key
    }
}

impl fmt::Debug for LegacyDeveloperStoredKeyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperStoredKeyV1")
            .field("key_id", &"<redacted>")
            .field("kind", &self.kind)
            .field("key_prefix", &"<redacted>")
            .field("key_hash", &self.key_hash)
            .field("encrypted_key", &self.encrypted_key)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperSealedKeyReplayV1 {
    ciphertext: LegacyDeveloperProtectedBlobV1,
    binding: [u8; 32],
}

impl LegacyDeveloperSealedKeyReplayV1 {
    pub fn new(ciphertext: LegacyDeveloperProtectedBlobV1, binding: [u8; 32]) -> Self {
        Self {
            ciphertext,
            binding,
        }
    }

    #[must_use]
    pub const fn ciphertext(&self) -> &LegacyDeveloperProtectedBlobV1 {
        &self.ciphertext
    }

    #[must_use]
    pub const fn binding(&self) -> &[u8; 32] {
        &self.binding
    }
}

impl fmt::Debug for LegacyDeveloperSealedKeyReplayV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyDeveloperSealedKeyReplayV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperProtectedKeyPairV1 {
    public_key: LegacyDeveloperStoredKeyV1,
    secret_key: LegacyDeveloperStoredKeyV1,
    replay: LegacyDeveloperSealedKeyReplayV1,
}

impl LegacyDeveloperProtectedKeyPairV1 {
    pub fn new(
        public_key: LegacyDeveloperStoredKeyV1,
        secret_key: LegacyDeveloperStoredKeyV1,
        replay: LegacyDeveloperSealedKeyReplayV1,
    ) -> Result<Self, LegacyDeveloperSecretErrorV1> {
        if public_key.kind != LegacyDeveloperKeyKindV1::Public
            || secret_key.kind != LegacyDeveloperKeyKindV1::Secret
            || public_key.key_id == secret_key.key_id
            || public_key.key_hash == secret_key.key_hash
        {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        Ok(Self {
            public_key,
            secret_key,
            replay,
        })
    }

    #[must_use]
    pub const fn public_key(&self) -> &LegacyDeveloperStoredKeyV1 {
        &self.public_key
    }

    #[must_use]
    pub const fn secret_key(&self) -> &LegacyDeveloperStoredKeyV1 {
        &self.secret_key
    }

    #[must_use]
    pub const fn replay(&self) -> &LegacyDeveloperSealedKeyReplayV1 {
        &self.replay
    }
}

impl fmt::Debug for LegacyDeveloperProtectedKeyPairV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyDeveloperProtectedKeyPairV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyDeveloperProtectedProvisioningV1 {
    CreateApp {
        app_id: LegacyDeveloperAppIdV1,
        credit_account_id: LegacyDeveloperCreditAccountIdV1,
        keys: LegacyDeveloperProtectedKeyPairV1,
    },
    RegenerateKeys {
        keys: LegacyDeveloperProtectedKeyPairV1,
    },
}

impl LegacyDeveloperProtectedProvisioningV1 {
    #[must_use]
    pub const fn keys(&self) -> &LegacyDeveloperProtectedKeyPairV1 {
        match self {
            Self::CreateApp { keys, .. } | Self::RegenerateKeys { keys } => keys,
        }
    }

    #[must_use]
    pub const fn created_app_id(&self) -> Option<&LegacyDeveloperAppIdV1> {
        match self {
            Self::CreateApp { app_id, .. } => Some(app_id),
            Self::RegenerateKeys { .. } => None,
        }
    }

    #[must_use]
    pub const fn credit_account_id(&self) -> Option<&LegacyDeveloperCreditAccountIdV1> {
        match self {
            Self::CreateApp {
                credit_account_id, ..
            } => Some(credit_account_id),
            Self::RegenerateKeys { .. } => None,
        }
    }
}

impl fmt::Debug for LegacyDeveloperProtectedProvisioningV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreateApp { .. } => "CreateApp([redacted])",
            Self::RegenerateKeys { .. } => "RegenerateKeys([redacted])",
        })
    }
}

#[derive(PartialEq, Eq)]
pub struct LegacyDeveloperRevealedKeyPairV1 {
    public_key: String,
    secret_key: String,
}

impl LegacyDeveloperRevealedKeyPairV1 {
    pub fn new(
        public_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Result<Self, LegacyDeveloperSecretErrorV1> {
        let public_key = public_key.into();
        let secret_key = secret_key.into();
        if !valid_raw_key(LegacyDeveloperKeyKindV1::Public, &public_key)
            || !valid_raw_key(LegacyDeveloperKeyKindV1::Secret, &secret_key)
            || public_key == secret_key
        {
            return Err(LegacyDeveloperSecretErrorV1::InvalidMaterial);
        }
        Ok(Self {
            public_key,
            secret_key,
        })
    }

    /// Explicitly exposes the public credential for the source-compatible response.
    #[must_use]
    pub fn expose_public_key(&self) -> &str {
        &self.public_key
    }

    /// Explicitly exposes the secret credential for the source-compatible response.
    #[must_use]
    pub fn expose_secret_key(&self) -> &str {
        &self.secret_key
    }
}

impl fmt::Debug for LegacyDeveloperRevealedKeyPairV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyDeveloperRevealedKeyPairV1([redacted])")
    }
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyDeveloperSecretErrorV1 {
    #[error("developer credential generation is unavailable")]
    Unavailable,
    #[error("developer credential material is invalid")]
    InvalidMaterial,
}

impl fmt::Debug for LegacyDeveloperSecretErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "Unavailable",
            Self::InvalidMaterial => "InvalidMaterial",
        })
    }
}

/// Provider-free generation, hashing, encryption, and replay-envelope boundary.
///
/// `generate_protected` must generate Cap-compatible `cpk_`/`csk_` credentials,
/// HMAC-SHA-256 hashes, encrypted-at-rest values, unique Cap `NanoID`s, and an
/// authenticated replay envelope bound to `context.replay_binding()`. The
/// atomic port must call it only after it has claimed a new request; a replay
/// uses the journaled envelope. `reveal` may return plaintext only to the
/// application response projection and must authenticate the binding first.
#[async_trait]
pub trait LegacyDeveloperSecretAuthorityV1: Send + Sync {
    async fn generate_protected(
        &self,
        context: &LegacyDeveloperSecretGenerationContextV1,
    ) -> Result<LegacyDeveloperProtectedProvisioningV1, LegacyDeveloperSecretErrorV1>;

    async fn reveal(
        &self,
        replay: &LegacyDeveloperSealedKeyReplayV1,
    ) -> Result<LegacyDeveloperRevealedKeyPairV1, LegacyDeveloperSecretErrorV1>;
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyDeveloperAuthorityPostconditionV1 {
    NewAppOwnedByActor {
        actor_id: UserId,
    },
    ExistingLiveAppOwnedByActor {
        app_id: LegacyDeveloperAppIdV1,
        owner_id: UserId,
    },
}

impl LegacyDeveloperAuthorityPostconditionV1 {
    #[must_use]
    pub const fn owner_id(&self) -> UserId {
        match self {
            Self::NewAppOwnedByActor { actor_id }
            | Self::ExistingLiveAppOwnedByActor {
                owner_id: actor_id, ..
            } => *actor_id,
        }
    }

    #[must_use]
    pub const fn app_id(&self) -> Option<&LegacyDeveloperAppIdV1> {
        match self {
            Self::NewAppOwnedByActor { .. } => None,
            Self::ExistingLiveAppOwnedByActor { app_id, .. } => Some(app_id),
        }
    }
}

impl fmt::Debug for LegacyDeveloperAuthorityPostconditionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NewAppOwnedByActor { .. } => "NewAppOwnedByActor([redacted])",
            Self::ExistingLiveAppOwnedByActor { .. } => "ExistingLiveAppOwnedByActor([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperAutoTopUpStateV1 {
    enabled: bool,
    threshold_micro_credits: u64,
    amount_cents: u32,
}

impl LegacyDeveloperAutoTopUpStateV1 {
    pub fn new(
        enabled: bool,
        threshold_micro_credits: u64,
        amount_cents: u32,
    ) -> Result<Self, LegacyDeveloperAtomicErrorV1> {
        if threshold_micro_credits > MAX_LEDGER_AMOUNT as u64 || amount_cents > 100_000 {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            enabled,
            threshold_micro_credits,
            amount_cents,
        })
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn threshold_micro_credits(&self) -> u64 {
        self.threshold_micro_credits
    }

    #[must_use]
    pub const fn amount_cents(&self) -> u32 {
        self.amount_cents
    }
}

impl fmt::Debug for LegacyDeveloperAutoTopUpStateV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperAutoTopUpStateV1")
            .field("enabled", &self.enabled)
            .field("financial_values", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyDeveloperMutationPostconditionV1 {
    AppCreated {
        owner_id: UserId,
        stored_name: String,
        environment: LegacyDeveloperEnvironmentV1,
        provisioning: LegacyDeveloperProtectedProvisioningV1,
        active_key_count_after: u32,
        credit_account_owner_id: UserId,
        credit_balance_micro_credits: u64,
        auto_top_up: LegacyDeveloperAutoTopUpStateV1,
    },
    AppUpdated {
        app_id: LegacyDeveloperAppIdV1,
        final_name: String,
        final_environment: LegacyDeveloperEnvironmentV1,
        final_logo_url: Option<String>,
        update_statement_executed: bool,
    },
    AppDeleted {
        app_id: LegacyDeveloperAppIdV1,
        deleted_at: TimestampMillis,
        revoked_active_key_count: u32,
        active_key_count_after: u32,
    },
    DomainAdded {
        app_id: LegacyDeveloperAppIdV1,
        domain_id: LegacyDeveloperDomainIdV1,
        stored_origin: String,
    },
    DomainDeleteAttempted {
        app_id: LegacyDeveloperAppIdV1,
        domain_id: LegacyDeveloperDomainIdV1,
        matched_rows: u8,
    },
    KeysRegenerated {
        app_id: LegacyDeveloperAppIdV1,
        revoked_active_key_count: u32,
        active_key_count_after: u32,
        provisioning: LegacyDeveloperProtectedProvisioningV1,
    },
    VideoDeleteAttempted {
        app_id: LegacyDeveloperAppIdV1,
        video_id: LegacyDeveloperVideoIdV1,
        matched_rows: u8,
        deleted_at: Option<TimestampMillis>,
    },
    AutoTopUpUpdated {
        app_id: LegacyDeveloperAppIdV1,
        account_state: Option<LegacyDeveloperAutoTopUpStateV1>,
    },
}

impl LegacyDeveloperMutationPostconditionV1 {
    #[must_use]
    pub const fn protected_keys(&self) -> Option<&LegacyDeveloperProtectedKeyPairV1> {
        match self {
            Self::AppCreated { provisioning, .. } | Self::KeysRegenerated { provisioning, .. } => {
                Some(provisioning.keys())
            }
            Self::AppUpdated { .. }
            | Self::AppDeleted { .. }
            | Self::DomainAdded { .. }
            | Self::DomainDeleteAttempted { .. }
            | Self::VideoDeleteAttempted { .. }
            | Self::AutoTopUpUpdated { .. } => None,
        }
    }

    #[must_use]
    pub const fn created_app_id(&self) -> Option<&LegacyDeveloperAppIdV1> {
        match self {
            Self::AppCreated { provisioning, .. } => provisioning.created_app_id(),
            _ => None,
        }
    }
}

impl fmt::Debug for LegacyDeveloperMutationPostconditionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::AppCreated { .. } => "AppCreated",
            Self::AppUpdated { .. } => "AppUpdated",
            Self::AppDeleted { .. } => "AppDeleted",
            Self::DomainAdded { .. } => "DomainAdded",
            Self::DomainDeleteAttempted { .. } => "DomainDeleteAttempted",
            Self::KeysRegenerated { .. } => "KeysRegenerated",
            Self::VideoDeleteAttempted { .. } => "VideoDeleteAttempted",
            Self::AutoTopUpUpdated { .. } => "AutoTopUpUpdated",
        };
        formatter
            .debug_struct(name)
            .field("database_postcondition", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperEffectsV1 {
    revalidate_developer_dashboard: bool,
}

impl LegacyDeveloperEffectsV1 {
    fn for_action(action: LegacyDeveloperActionV1) -> Self {
        // createDeveloperApp has no server-side revalidatePath call. Its caller
        // refreshes after receiving the newly generated credentials.
        Self {
            revalidate_developer_dashboard: action != LegacyDeveloperActionV1::CreateApp,
        }
    }

    #[must_use]
    pub const fn revalidate_developer_dashboard(self) -> bool {
        self.revalidate_developer_dashboard
    }

    #[must_use]
    pub const fn path(self) -> Option<&'static str> {
        if self.revalidate_developer_dashboard {
            Some(LEGACY_DEVELOPER_DASHBOARD_REVALIDATION_PATH)
        } else {
            None
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyDeveloperMutationReceiptV1 {
    authority: LegacyDeveloperAuthorityPostconditionV1,
    mutation: LegacyDeveloperMutationPostconditionV1,
    effects: LegacyDeveloperEffectsV1,
}

impl LegacyDeveloperMutationReceiptV1 {
    pub fn new(
        command: &LegacyDeveloperCommandV1,
        authority: LegacyDeveloperAuthorityPostconditionV1,
        mutation: LegacyDeveloperMutationPostconditionV1,
    ) -> Result<Self, LegacyDeveloperAtomicErrorV1> {
        validate_authority_postcondition(command, &authority)?;
        validate_mutation_postcondition(command, &mutation)?;
        Ok(Self {
            authority,
            mutation,
            effects: LegacyDeveloperEffectsV1::for_action(command.action()),
        })
    }

    pub fn validate_against(
        &self,
        command: &LegacyDeveloperCommandV1,
    ) -> Result<(), LegacyDeveloperAtomicErrorV1> {
        validate_authority_postcondition(command, &self.authority)?;
        validate_mutation_postcondition(command, &self.mutation)?;
        if self.effects != LegacyDeveloperEffectsV1::for_action(command.action()) {
            return Err(LegacyDeveloperAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    #[must_use]
    pub const fn authority(&self) -> &LegacyDeveloperAuthorityPostconditionV1 {
        &self.authority
    }

    #[must_use]
    pub const fn mutation(&self) -> &LegacyDeveloperMutationPostconditionV1 {
        &self.mutation
    }

    #[must_use]
    pub const fn effects(&self) -> LegacyDeveloperEffectsV1 {
        self.effects
    }
}

impl fmt::Debug for LegacyDeveloperMutationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperMutationReceiptV1")
            .field("authority", &self.authority)
            .field("mutation", &self.mutation)
            .field("effects", &self.effects)
            .finish()
    }
}

fn validate_authority_postcondition(
    command: &LegacyDeveloperCommandV1,
    authority: &LegacyDeveloperAuthorityPostconditionV1,
) -> Result<(), LegacyDeveloperAtomicErrorV1> {
    let actor_id = command.fence().authority.actor_id;
    let valid = match (command, authority) {
        (
            LegacyDeveloperCommandV1::CreateApp { .. },
            LegacyDeveloperAuthorityPostconditionV1::NewAppOwnedByActor { actor_id: owner_id },
        ) => *owner_id == actor_id,
        (
            command,
            LegacyDeveloperAuthorityPostconditionV1::ExistingLiveAppOwnedByActor {
                app_id,
                owner_id,
            },
        ) => command.app_id() == Some(app_id) && *owner_id == actor_id,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(LegacyDeveloperAtomicErrorV1::Corrupt)
    }
}

fn validate_mutation_postcondition(
    command: &LegacyDeveloperCommandV1,
    mutation: &LegacyDeveloperMutationPostconditionV1,
) -> Result<(), LegacyDeveloperAtomicErrorV1> {
    let valid = match (command, mutation) {
        (
            LegacyDeveloperCommandV1::CreateApp {
                name, environment, ..
            },
            LegacyDeveloperMutationPostconditionV1::AppCreated {
                owner_id,
                stored_name,
                environment: stored_environment,
                provisioning,
                active_key_count_after,
                credit_account_owner_id,
                credit_balance_micro_credits,
                auto_top_up,
            },
        ) => {
            *owner_id == command.fence().authority.actor_id
                && stored_name == name
                && *stored_environment == *environment
                && *active_key_count_after == 2
                && *credit_account_owner_id == command.fence().authority.actor_id
                && *credit_balance_micro_credits == 0
                && !auto_top_up.enabled
                && auto_top_up.threshold_micro_credits == 0
                && auto_top_up.amount_cents == 0
                && matches!(
                    provisioning,
                    LegacyDeveloperProtectedProvisioningV1::CreateApp { .. }
                )
                && valid_provisioning_binding(command, provisioning)
        }
        (
            LegacyDeveloperCommandV1::UpdateApp { app_id, patch, .. },
            LegacyDeveloperMutationPostconditionV1::AppUpdated {
                app_id: stored_app_id,
                final_name,
                final_environment,
                final_logo_url,
                update_statement_executed,
            },
        ) => {
            app_id == stored_app_id
                && valid_stored_name(final_name)
                && final_logo_url.as_ref().is_none_or(|value| {
                    value.chars().count() <= LEGACY_DEVELOPER_MAX_LOGO_URL_CHARS
                })
                && patch.name.as_ref().is_none_or(|value| value == final_name)
                && patch
                    .environment
                    .is_none_or(|value| value == *final_environment)
                && match &patch.logo_url {
                    LegacyDeveloperNullableLogoPatchV1::Missing => true,
                    LegacyDeveloperNullableLogoPatchV1::Null => final_logo_url.is_none(),
                    LegacyDeveloperNullableLogoPatchV1::Value(value) => {
                        final_logo_url.as_ref() == Some(value)
                    }
                }
                && *update_statement_executed != patch.is_empty()
        }
        (
            LegacyDeveloperCommandV1::DeleteApp { app_id, .. },
            LegacyDeveloperMutationPostconditionV1::AppDeleted {
                app_id: stored_app_id,
                active_key_count_after,
                ..
            },
        ) => app_id == stored_app_id && *active_key_count_after == 0,
        (
            LegacyDeveloperCommandV1::AddDomain {
                app_id,
                normalized_origin,
                ..
            },
            LegacyDeveloperMutationPostconditionV1::DomainAdded {
                app_id: stored_app_id,
                stored_origin,
                ..
            },
        ) => app_id == stored_app_id && normalized_origin == stored_origin,
        (
            LegacyDeveloperCommandV1::RemoveDomain {
                app_id, domain_id, ..
            },
            LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted {
                app_id: stored_app_id,
                domain_id: stored_domain_id,
                matched_rows,
            },
        ) => app_id == stored_app_id && domain_id == stored_domain_id && *matched_rows <= 1,
        (
            LegacyDeveloperCommandV1::RegenerateKeys { app_id, .. },
            LegacyDeveloperMutationPostconditionV1::KeysRegenerated {
                app_id: stored_app_id,
                active_key_count_after,
                provisioning,
                ..
            },
        ) => {
            app_id == stored_app_id
                && *active_key_count_after == 2
                && matches!(
                    provisioning,
                    LegacyDeveloperProtectedProvisioningV1::RegenerateKeys { .. }
                )
                && valid_provisioning_binding(command, provisioning)
        }
        (
            LegacyDeveloperCommandV1::DeleteVideo {
                app_id, video_id, ..
            },
            LegacyDeveloperMutationPostconditionV1::VideoDeleteAttempted {
                app_id: stored_app_id,
                video_id: stored_video_id,
                matched_rows,
                deleted_at,
            },
        ) => {
            app_id == stored_app_id
                && video_id == stored_video_id
                && *matched_rows <= 1
                && ((*matched_rows == 1) == deleted_at.is_some())
        }
        (
            LegacyDeveloperCommandV1::UpdateAutoTopUp { app_id, patch, .. },
            LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated {
                app_id: stored_app_id,
                account_state,
            },
        ) => {
            app_id == stored_app_id
                && account_state.as_ref().is_none_or(|state| {
                    state.enabled == patch.enabled
                        && patch
                            .threshold_micro_credits
                            .is_none_or(|value| value == state.threshold_micro_credits)
                        && patch
                            .amount_cents
                            .is_none_or(|value| value == state.amount_cents)
                })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(LegacyDeveloperAtomicErrorV1::Corrupt)
    }
}

fn valid_provisioning_binding(
    command: &LegacyDeveloperCommandV1,
    provisioning: &LegacyDeveloperProtectedProvisioningV1,
) -> bool {
    let Some(context) = command.secret_generation_context() else {
        return false;
    };
    provisioning.keys().replay.binding == context.replay_binding()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyDeveloperAtomicOutcomeV1 {
    Applied(LegacyDeveloperMutationReceiptV1),
    Replay(LegacyDeveloperMutationReceiptV1),
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyDeveloperAtomicErrorV1 {
    #[error("developer app was not found")]
    AppMissing,
    #[error("developer app was not found")]
    NotOwner,
    #[error("developer app was not found")]
    Deleted,
    #[error("developer app was not found")]
    StaleAuthority,
    #[error("developer action conflicts with current state")]
    DuplicateDomain,
    #[error("developer action conflicts with a prior request")]
    Conflict,
    #[error("developer action is already in flight")]
    InFlight,
    #[error("developer authority is unavailable")]
    Unavailable,
    #[error("developer credential generation is unavailable")]
    SecretUnavailable,
    #[error("developer authority returned invalid state")]
    Corrupt,
}

/// Atomic boundary for the eight developer-dashboard actions.
///
/// An implementation must, in one transaction:
///
/// 1. assert and consume the one-use browser grant and current session actor;
/// 2. claim `(actor, operation, idempotency key)` against the canonical
///    fingerprint, returning the exact journaled receipt for a replay;
/// 3. for every action except create, load a non-deleted app whose `ownerId`
///    equals the actor, mapping missing, deleted, and foreign apps identically;
/// 4. apply only the typed postcondition for the selected action. In particular,
///    an empty app patch performs no update but succeeds, logo `Missing` and
///    `Null` remain distinct, and remove-domain/delete-video constrain by both
///    target and app but succeed for zero affected rows;
/// 5. on create/regenerate only after a new claim (and after owner authority for
///    regenerate), call `secrets.generate_protected`, insert the protected key
///    rows, and journal its encrypted replay envelope. Create also inserts the
///    owned app and zero-balance/default-auto-top-up credit account; regenerate
///    revokes every active prior key before inserting exactly one public and one
///    secret key; and
/// 6. persist the DB-derived authority/mutation postconditions, audit evidence,
///    exact invalidation effect, proof consumption, and journal outcome together.
///
/// Concurrent same-key claims must resolve to one apply plus replay, or a stable
/// `InFlight`/`Conflict`; they must never produce two apps/key pairs. This
/// source-only contract does not authorize a production allowlist entry.
#[async_trait]
pub trait LegacyDeveloperAtomicPortV1: Send + Sync {
    async fn execute_atomic(
        &self,
        command: &LegacyDeveloperCommandV1,
        browser_fence: &LegacyDeveloperBrowserFenceV1,
        secrets: &dyn LegacyDeveloperSecretAuthorityV1,
    ) -> Result<LegacyDeveloperAtomicOutcomeV1, LegacyDeveloperAtomicErrorV1>;
}

#[derive(PartialEq, Eq)]
pub enum LegacyDeveloperSuccessV1 {
    AppCreated {
        legacy_app_id: String,
        keys: LegacyDeveloperRevealedKeyPairV1,
    },
    KeysRegenerated {
        keys: LegacyDeveloperRevealedKeyPairV1,
    },
    SuccessObject,
}

impl LegacyDeveloperSuccessV1 {
    #[must_use]
    pub fn app_id(&self) -> Option<&str> {
        match self {
            Self::AppCreated { legacy_app_id, .. } => Some(legacy_app_id),
            Self::KeysRegenerated { .. } | Self::SuccessObject => None,
        }
    }

    #[must_use]
    pub const fn keys(&self) -> Option<&LegacyDeveloperRevealedKeyPairV1> {
        match self {
            Self::AppCreated { keys, .. } | Self::KeysRegenerated { keys } => Some(keys),
            Self::SuccessObject => None,
        }
    }

    #[must_use]
    pub const fn object_success(&self) -> Option<bool> {
        match self {
            Self::SuccessObject => Some(true),
            Self::AppCreated { .. } | Self::KeysRegenerated { .. } => None,
        }
    }
}

impl fmt::Debug for LegacyDeveloperSuccessV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AppCreated { .. } => "AppCreated([redacted])",
            Self::KeysRegenerated { .. } => "KeysRegenerated([redacted])",
            Self::SuccessObject => "SuccessObject",
        })
    }
}

#[derive(PartialEq, Eq)]
pub struct LegacyDeveloperExecutionV1 {
    success: LegacyDeveloperSuccessV1,
    effects: LegacyDeveloperEffectsV1,
    replayed: bool,
}

impl LegacyDeveloperExecutionV1 {
    #[must_use]
    pub const fn success(&self) -> &LegacyDeveloperSuccessV1 {
        &self.success
    }

    #[must_use]
    pub const fn effects(&self) -> LegacyDeveloperEffectsV1 {
        self.effects
    }

    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }

    #[must_use]
    pub const fn mutation_was_applied(&self) -> bool {
        !self.replayed
    }
}

impl fmt::Debug for LegacyDeveloperExecutionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperExecutionV1")
            .field("success", &self.success)
            .field("effects", &self.effects)
            .field("replayed", &self.replayed)
            .finish()
    }
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyDeveloperErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid developer request")]
    Invalid,
    #[error("An idempotency key is required")]
    IdempotencyRequired,
    #[error("App name is required")]
    AppNameRequired,
    #[error("App name cannot be empty")]
    AppNameEmpty,
    #[error("Domain is required")]
    DomainRequired,
    #[error("Domain must be a valid origin (e.g. https://myapp.com)")]
    DomainInvalid,
    #[error("Threshold must be non-negative")]
    ThresholdNegative,
    #[error("Top-up amount must be positive")]
    TopUpAmountNonPositive,
    #[error("Top-up amount must be between $0.01 and $1,000.00")]
    TopUpAmountTooLarge,
    #[error("App not found")]
    AppNotFound,
    #[error("Developer action conflicts with a prior request")]
    Conflict,
    #[error("Developer authority is unavailable")]
    AuthorityUnavailable,
    #[error("Developer credential generation is unavailable")]
    SecretUnavailable,
    #[error("Developer action failed")]
    Internal,
}

impl fmt::Debug for LegacyDeveloperErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "Unauthorized",
            Self::Invalid => "Invalid",
            Self::IdempotencyRequired => "IdempotencyRequired",
            Self::AppNameRequired => "AppNameRequired",
            Self::AppNameEmpty => "AppNameEmpty",
            Self::DomainRequired => "DomainRequired",
            Self::DomainInvalid => "DomainInvalid",
            Self::ThresholdNegative => "ThresholdNegative",
            Self::TopUpAmountNonPositive => "TopUpAmountNonPositive",
            Self::TopUpAmountTooLarge => "TopUpAmountTooLarge",
            Self::AppNotFound => "AppNotFound",
            Self::Conflict => "Conflict",
            Self::AuthorityUnavailable => "AuthorityUnavailable",
            Self::SecretUnavailable => "SecretUnavailable",
            Self::Internal => "Internal",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperAdapterV1 {
    action: LegacyDeveloperActionV1,
}

impl LegacyDeveloperAdapterV1 {
    #[must_use]
    pub const fn create_app() -> Self {
        Self {
            action: LegacyDeveloperActionV1::CreateApp,
        }
    }

    #[must_use]
    pub const fn update_app() -> Self {
        Self {
            action: LegacyDeveloperActionV1::UpdateApp,
        }
    }

    #[must_use]
    pub const fn delete_app() -> Self {
        Self {
            action: LegacyDeveloperActionV1::DeleteApp,
        }
    }

    #[must_use]
    pub const fn add_domain() -> Self {
        Self {
            action: LegacyDeveloperActionV1::AddDomain,
        }
    }

    #[must_use]
    pub const fn remove_domain() -> Self {
        Self {
            action: LegacyDeveloperActionV1::RemoveDomain,
        }
    }

    #[must_use]
    pub const fn regenerate_keys() -> Self {
        Self {
            action: LegacyDeveloperActionV1::RegenerateKeys,
        }
    }

    #[must_use]
    pub const fn delete_video() -> Self {
        Self {
            action: LegacyDeveloperActionV1::DeleteVideo,
        }
    }

    #[must_use]
    pub const fn update_auto_top_up() -> Self {
        Self {
            action: LegacyDeveloperActionV1::UpdateAutoTopUp,
        }
    }

    #[must_use]
    pub const fn profile(self) -> &'static LegacyDeveloperProfileV1 {
        match self.action {
            LegacyDeveloperActionV1::CreateApp => &LEGACY_CREATE_DEVELOPER_APP_PROFILE,
            LegacyDeveloperActionV1::UpdateApp => &LEGACY_UPDATE_DEVELOPER_APP_PROFILE,
            LegacyDeveloperActionV1::DeleteApp => &LEGACY_DELETE_DEVELOPER_APP_PROFILE,
            LegacyDeveloperActionV1::AddDomain => &LEGACY_ADD_DEVELOPER_DOMAIN_PROFILE,
            LegacyDeveloperActionV1::RemoveDomain => &LEGACY_REMOVE_DEVELOPER_DOMAIN_PROFILE,
            LegacyDeveloperActionV1::RegenerateKeys => &LEGACY_REGENERATE_DEVELOPER_KEYS_PROFILE,
            LegacyDeveloperActionV1::DeleteVideo => &LEGACY_DELETE_DEVELOPER_VIDEO_PROFILE,
            LegacyDeveloperActionV1::UpdateAutoTopUp => {
                &LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_PROFILE
            }
        }
    }

    pub fn prepare(
        self,
        request: &LegacyDeveloperRequestV1,
    ) -> Result<LegacyDeveloperCommandV1, LegacyDeveloperErrorV1> {
        if request.credential != Some(LegacyDeveloperCredentialV1::Session) {
            return Err(LegacyDeveloperErrorV1::Unauthorized);
        }
        let actor_id = request
            .actor_id
            .ok_or(LegacyDeveloperErrorV1::Unauthorized)?;
        if request.input.action() != self.action {
            return Err(LegacyDeveloperErrorV1::Invalid);
        }
        let idempotency_key = request
            .idempotency_key
            .as_ref()
            .ok_or(LegacyDeveloperErrorV1::IdempotencyRequired)
            .and_then(|value| {
                IdempotencyKey::parse(value.clone()).map_err(|_| LegacyDeveloperErrorV1::Invalid)
            })?;
        let authority = LegacyDeveloperAuthorityV1 { actor_id };

        match &request.input {
            LegacyDeveloperInputV1::CreateApp { name, environment } => {
                let name = normalize_name(name, LegacyDeveloperErrorV1::AppNameRequired)?;
                let request_fingerprint = fingerprint_create(actor_id, &name, *environment);
                Ok(LegacyDeveloperCommandV1::CreateApp {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    name,
                    environment: *environment,
                })
            }
            LegacyDeveloperInputV1::UpdateApp {
                legacy_app_id,
                name,
                environment,
                logo_url,
            } => {
                let app_id = map_app_id(legacy_app_id)?;
                let name = name
                    .as_ref()
                    .map(|value| normalize_name(value, LegacyDeveloperErrorV1::AppNameEmpty))
                    .transpose()?;
                let logo_url = normalize_logo_patch(logo_url)?;
                let patch = LegacyDeveloperAppPatchV1 {
                    name,
                    environment: *environment,
                    logo_url,
                };
                let request_fingerprint = fingerprint_update(actor_id, &app_id, &patch);
                Ok(LegacyDeveloperCommandV1::UpdateApp {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                    patch,
                })
            }
            LegacyDeveloperInputV1::DeleteApp { legacy_app_id } => {
                let app_id = map_app_id(legacy_app_id)?;
                let request_fingerprint = fingerprint_app_only(self.action, actor_id, &app_id);
                Ok(LegacyDeveloperCommandV1::DeleteApp {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                })
            }
            LegacyDeveloperInputV1::AddDomain {
                legacy_app_id,
                domain,
            } => {
                let app_id = map_app_id(legacy_app_id)?;
                let normalized_origin = normalize_origin(domain)?;
                let request_fingerprint =
                    fingerprint_app_string(self.action, actor_id, &app_id, &normalized_origin);
                Ok(LegacyDeveloperCommandV1::AddDomain {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                    normalized_origin,
                })
            }
            LegacyDeveloperInputV1::RemoveDomain {
                legacy_app_id,
                legacy_domain_id,
            } => {
                let app_id = map_app_id(legacy_app_id)?;
                let domain_id = LegacyDeveloperDomainIdV1::parse(legacy_domain_id.clone())?;
                let request_fingerprint = fingerprint_app_string(
                    self.action,
                    actor_id,
                    &app_id,
                    &domain_id.mapped_uuid(),
                );
                Ok(LegacyDeveloperCommandV1::RemoveDomain {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                    domain_id,
                })
            }
            LegacyDeveloperInputV1::RegenerateKeys { legacy_app_id } => {
                let app_id = map_app_id(legacy_app_id)?;
                let request_fingerprint = fingerprint_app_only(self.action, actor_id, &app_id);
                Ok(LegacyDeveloperCommandV1::RegenerateKeys {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                })
            }
            LegacyDeveloperInputV1::DeleteVideo {
                legacy_app_id,
                legacy_video_id,
            } => {
                let app_id = map_app_id(legacy_app_id)?;
                let video_id = LegacyDeveloperVideoIdV1::parse(legacy_video_id.clone())?;
                let request_fingerprint =
                    fingerprint_app_string(self.action, actor_id, &app_id, &video_id.mapped_uuid());
                Ok(LegacyDeveloperCommandV1::DeleteVideo {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                    video_id,
                })
            }
            LegacyDeveloperInputV1::UpdateAutoTopUp {
                legacy_app_id,
                enabled,
                threshold_micro_credits,
                amount_cents,
            } => {
                let app_id = map_app_id(legacy_app_id)?;
                let patch =
                    normalize_auto_top_up_patch(*enabled, *threshold_micro_credits, *amount_cents)?;
                let request_fingerprint = fingerprint_auto_top_up(actor_id, &app_id, patch);
                Ok(LegacyDeveloperCommandV1::UpdateAutoTopUp {
                    fence: fence(authority, idempotency_key, request_fingerprint),
                    app_id,
                    patch,
                })
            }
        }
    }

    pub async fn execute<Port, Secrets>(
        self,
        port: &Port,
        secrets: &Secrets,
        request: &LegacyDeveloperRequestV1,
        proof: &ValidatedBrowserMutationProof,
    ) -> Result<LegacyDeveloperExecutionV1, LegacyDeveloperErrorV1>
    where
        Port: LegacyDeveloperAtomicPortV1,
        Secrets: LegacyDeveloperSecretAuthorityV1,
    {
        let browser_fence = LegacyDeveloperBrowserFenceV1::from_validated_proof(proof);
        if request.actor_id != Some(browser_fence.actor_id()) {
            return Err(LegacyDeveloperErrorV1::Unauthorized);
        }
        self.execute_fenced(port, secrets, request, &browser_fence)
            .await
    }

    async fn execute_fenced<Port, Secrets>(
        self,
        port: &Port,
        secrets: &Secrets,
        request: &LegacyDeveloperRequestV1,
        browser_fence: &LegacyDeveloperBrowserFenceV1,
    ) -> Result<LegacyDeveloperExecutionV1, LegacyDeveloperErrorV1>
    where
        Port: LegacyDeveloperAtomicPortV1,
        Secrets: LegacyDeveloperSecretAuthorityV1,
    {
        let command = self.prepare(request)?;
        if command.fence().authority.actor_id != browser_fence.actor_id() {
            return Err(LegacyDeveloperErrorV1::Unauthorized);
        }
        let (receipt, replayed) = match port
            .execute_atomic(&command, browser_fence, secrets)
            .await
            .map_err(map_atomic_error)?
        {
            LegacyDeveloperAtomicOutcomeV1::Applied(receipt) => (receipt, false),
            LegacyDeveloperAtomicOutcomeV1::Replay(receipt) => (receipt, true),
        };
        receipt
            .validate_against(&command)
            .map_err(|_| LegacyDeveloperErrorV1::Internal)?;
        let success = project_success(&command, &receipt, secrets).await?;
        Ok(LegacyDeveloperExecutionV1 {
            success,
            effects: receipt.effects,
            replayed,
        })
    }
}

async fn project_success<Secrets: LegacyDeveloperSecretAuthorityV1>(
    command: &LegacyDeveloperCommandV1,
    receipt: &LegacyDeveloperMutationReceiptV1,
    secrets: &Secrets,
) -> Result<LegacyDeveloperSuccessV1, LegacyDeveloperErrorV1> {
    match (command, &receipt.mutation) {
        (
            LegacyDeveloperCommandV1::CreateApp { .. },
            LegacyDeveloperMutationPostconditionV1::AppCreated { provisioning, .. },
        ) => {
            let app_id = provisioning
                .created_app_id()
                .ok_or(LegacyDeveloperErrorV1::Internal)?;
            let keys = reveal_and_validate(provisioning.keys(), secrets).await?;
            Ok(LegacyDeveloperSuccessV1::AppCreated {
                legacy_app_id: app_id.legacy_value().to_owned(),
                keys,
            })
        }
        (
            LegacyDeveloperCommandV1::RegenerateKeys { .. },
            LegacyDeveloperMutationPostconditionV1::KeysRegenerated { provisioning, .. },
        ) => Ok(LegacyDeveloperSuccessV1::KeysRegenerated {
            keys: reveal_and_validate(provisioning.keys(), secrets).await?,
        }),
        (
            LegacyDeveloperCommandV1::UpdateApp { .. }
            | LegacyDeveloperCommandV1::DeleteApp { .. }
            | LegacyDeveloperCommandV1::AddDomain { .. }
            | LegacyDeveloperCommandV1::RemoveDomain { .. }
            | LegacyDeveloperCommandV1::DeleteVideo { .. }
            | LegacyDeveloperCommandV1::UpdateAutoTopUp { .. },
            LegacyDeveloperMutationPostconditionV1::AppUpdated { .. }
            | LegacyDeveloperMutationPostconditionV1::AppDeleted { .. }
            | LegacyDeveloperMutationPostconditionV1::DomainAdded { .. }
            | LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted { .. }
            | LegacyDeveloperMutationPostconditionV1::VideoDeleteAttempted { .. }
            | LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated { .. },
        ) => Ok(LegacyDeveloperSuccessV1::SuccessObject),
        _ => Err(LegacyDeveloperErrorV1::Internal),
    }
}

async fn reveal_and_validate<Secrets: LegacyDeveloperSecretAuthorityV1>(
    protected: &LegacyDeveloperProtectedKeyPairV1,
    secrets: &Secrets,
) -> Result<LegacyDeveloperRevealedKeyPairV1, LegacyDeveloperErrorV1> {
    let revealed = secrets.reveal(protected.replay()).await.map_err(|error| {
        if error == LegacyDeveloperSecretErrorV1::Unavailable {
            LegacyDeveloperErrorV1::SecretUnavailable
        } else {
            LegacyDeveloperErrorV1::Internal
        }
    })?;
    let public_prefix = revealed
        .expose_public_key()
        .get(..LEGACY_DEVELOPER_KEY_PREFIX_LENGTH);
    let secret_prefix = revealed
        .expose_secret_key()
        .get(..LEGACY_DEVELOPER_KEY_PREFIX_LENGTH);
    if public_prefix != Some(protected.public_key().key_prefix())
        || secret_prefix != Some(protected.secret_key().key_prefix())
    {
        return Err(LegacyDeveloperErrorV1::Internal);
    }
    Ok(revealed)
}

fn map_atomic_error(error: LegacyDeveloperAtomicErrorV1) -> LegacyDeveloperErrorV1 {
    match error {
        LegacyDeveloperAtomicErrorV1::AppMissing
        | LegacyDeveloperAtomicErrorV1::NotOwner
        | LegacyDeveloperAtomicErrorV1::Deleted
        | LegacyDeveloperAtomicErrorV1::StaleAuthority => LegacyDeveloperErrorV1::AppNotFound,
        LegacyDeveloperAtomicErrorV1::DuplicateDomain
        | LegacyDeveloperAtomicErrorV1::Conflict
        | LegacyDeveloperAtomicErrorV1::InFlight => LegacyDeveloperErrorV1::Conflict,
        LegacyDeveloperAtomicErrorV1::Unavailable => LegacyDeveloperErrorV1::AuthorityUnavailable,
        LegacyDeveloperAtomicErrorV1::SecretUnavailable => {
            LegacyDeveloperErrorV1::SecretUnavailable
        }
        LegacyDeveloperAtomicErrorV1::Corrupt => LegacyDeveloperErrorV1::Internal,
    }
}

fn fence(
    authority: LegacyDeveloperAuthorityV1,
    idempotency_key: IdempotencyKey,
    request_fingerprint: [u8; 32],
) -> LegacyDeveloperFenceV1 {
    LegacyDeveloperFenceV1 {
        authority,
        idempotency_key,
        request_fingerprint,
    }
}

fn map_app_id(value: &str) -> Result<LegacyDeveloperAppIdV1, LegacyDeveloperErrorV1> {
    LegacyDeveloperAppIdV1::parse(value.to_owned()).map_err(|_| LegacyDeveloperErrorV1::AppNotFound)
}

fn normalize_name(
    value: &str,
    empty_error: LegacyDeveloperErrorV1,
) -> Result<String, LegacyDeveloperErrorV1> {
    let value = value.trim();
    if value.is_empty() {
        return Err(empty_error);
    }
    if value.chars().count() > LEGACY_DEVELOPER_MAX_APP_NAME_CHARS {
        return Err(LegacyDeveloperErrorV1::Invalid);
    }
    Ok(value.to_owned())
}

fn valid_stored_name(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= LEGACY_DEVELOPER_MAX_APP_NAME_CHARS
}

fn normalize_logo_patch(
    value: &LegacyDeveloperNullableLogoPatchV1,
) -> Result<LegacyDeveloperNullableLogoPatchV1, LegacyDeveloperErrorV1> {
    match value {
        LegacyDeveloperNullableLogoPatchV1::Value(value)
            if value.chars().count() > LEGACY_DEVELOPER_MAX_LOGO_URL_CHARS =>
        {
            Err(LegacyDeveloperErrorV1::Invalid)
        }
        _ => Ok(value.clone()),
    }
}

fn normalize_origin(value: &str) -> Result<String, LegacyDeveloperErrorV1> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err(LegacyDeveloperErrorV1::DomainRequired);
    }
    if value.chars().count() > LEGACY_DEVELOPER_MAX_DOMAIN_CHARS || !valid_full_origin(&value) {
        return Err(LegacyDeveloperErrorV1::DomainInvalid);
    }
    Ok(value)
}

fn valid_full_origin(value: &str) -> bool {
    let Some(authority) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    if authority.is_empty() {
        return false;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    !host.is_empty()
        && host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        && port
            .is_none_or(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
}

fn normalize_auto_top_up_patch(
    enabled: bool,
    threshold_micro_credits: Option<i64>,
    amount_cents: Option<i64>,
) -> Result<LegacyDeveloperAutoTopUpPatchV1, LegacyDeveloperErrorV1> {
    if threshold_micro_credits.is_some_and(|value| value < 0) {
        return Err(LegacyDeveloperErrorV1::ThresholdNegative);
    }
    if threshold_micro_credits.is_some_and(|value| value > MAX_LEDGER_AMOUNT) {
        return Err(LegacyDeveloperErrorV1::Invalid);
    }
    if amount_cents.is_some_and(|value| value <= 0) {
        return Err(LegacyDeveloperErrorV1::TopUpAmountNonPositive);
    }
    if amount_cents.is_some_and(|value| value > LEGACY_DEVELOPER_MAX_TOP_UP_CENTS) {
        return Err(LegacyDeveloperErrorV1::TopUpAmountTooLarge);
    }
    let threshold_micro_credits = threshold_micro_credits
        .map(u64::try_from)
        .transpose()
        .map_err(|_| LegacyDeveloperErrorV1::Invalid)?;
    let amount_cents = amount_cents
        .map(u32::try_from)
        .transpose()
        .map_err(|_| LegacyDeveloperErrorV1::Invalid)?;
    Ok(LegacyDeveloperAutoTopUpPatchV1 {
        enabled,
        threshold_micro_credits,
        amount_cents,
    })
}

fn valid_key_prefix(kind: LegacyDeveloperKeyKindV1, value: &str) -> bool {
    value.len() == LEGACY_DEVELOPER_KEY_PREFIX_LENGTH
        && value.starts_with(kind.raw_prefix())
        && value[kind.raw_prefix().len()..].bytes().all(|byte| {
            LEGACY_DEVELOPER_LONG_NANOID_ALPHABET
                .as_bytes()
                .contains(&byte)
        })
}

fn valid_raw_key(kind: LegacyDeveloperKeyKindV1, value: &str) -> bool {
    value.len() == kind.raw_prefix().len() + LEGACY_DEVELOPER_KEY_BODY_LENGTH
        && value.starts_with(kind.raw_prefix())
        && value[kind.raw_prefix().len()..].bytes().all(|byte| {
            LEGACY_DEVELOPER_LONG_NANOID_ALPHABET
                .as_bytes()
                .contains(&byte)
        })
}

fn hash_value(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn base_fingerprint(action: LegacyDeveloperActionV1, actor_id: UserId) -> Sha256 {
    let mut hasher = Sha256::new();
    hasher.update(b"frame-legacy-developer-action-v1\0");
    hasher.update([action.fingerprint_tag()]);
    hash_value(&mut hasher, &actor_id.to_string());
    hasher
}

fn fingerprint_create(
    actor_id: UserId,
    name: &str,
    environment: LegacyDeveloperEnvironmentV1,
) -> [u8; 32] {
    let mut hasher = base_fingerprint(LegacyDeveloperActionV1::CreateApp, actor_id);
    hash_value(&mut hasher, name);
    hash_value(&mut hasher, environment.stable_code());
    hasher.finalize().into()
}

fn fingerprint_update(
    actor_id: UserId,
    app_id: &LegacyDeveloperAppIdV1,
    patch: &LegacyDeveloperAppPatchV1,
) -> [u8; 32] {
    let mut hasher = base_fingerprint(LegacyDeveloperActionV1::UpdateApp, actor_id);
    hash_value(&mut hasher, &app_id.mapped_uuid());
    hash_optional_string(&mut hasher, patch.name.as_deref());
    match patch.environment {
        Some(environment) => {
            hasher.update([1]);
            hash_value(&mut hasher, environment.stable_code());
        }
        None => hasher.update([0]),
    }
    match &patch.logo_url {
        LegacyDeveloperNullableLogoPatchV1::Missing => hasher.update([0]),
        LegacyDeveloperNullableLogoPatchV1::Null => hasher.update([1]),
        LegacyDeveloperNullableLogoPatchV1::Value(value) => {
            hasher.update([2]);
            hash_value(&mut hasher, value);
        }
    }
    hasher.finalize().into()
}

fn fingerprint_app_only(
    action: LegacyDeveloperActionV1,
    actor_id: UserId,
    app_id: &LegacyDeveloperAppIdV1,
) -> [u8; 32] {
    let mut hasher = base_fingerprint(action, actor_id);
    hash_value(&mut hasher, &app_id.mapped_uuid());
    hasher.finalize().into()
}

fn fingerprint_app_string(
    action: LegacyDeveloperActionV1,
    actor_id: UserId,
    app_id: &LegacyDeveloperAppIdV1,
    value: &str,
) -> [u8; 32] {
    let mut hasher = base_fingerprint(action, actor_id);
    hash_value(&mut hasher, &app_id.mapped_uuid());
    hash_value(&mut hasher, value);
    hasher.finalize().into()
}

fn fingerprint_auto_top_up(
    actor_id: UserId,
    app_id: &LegacyDeveloperAppIdV1,
    patch: LegacyDeveloperAutoTopUpPatchV1,
) -> [u8; 32] {
    let mut hasher = base_fingerprint(LegacyDeveloperActionV1::UpdateAutoTopUp, actor_id);
    hash_value(&mut hasher, &app_id.mapped_uuid());
    hasher.update([u8::from(patch.enabled)]);
    hash_optional_u64(&mut hasher, patch.threshold_micro_credits);
    hash_optional_u64(&mut hasher, patch.amount_cents.map(u64::from));
    hasher.finalize().into()
}

fn hash_optional_string(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_value(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const APP: &str = "0123456789abcde";
    const OTHER_APP: &str = "1123456789abcde";
    const DOMAIN: &str = "2123456789abcde";
    const VIDEO: &str = "3123456789abcde";
    const PUBLIC_KEY_ID: &str = "4123456789abcde";
    const SECRET_KEY_ID: &str = "5123456789abcde";
    const CREDIT_ACCOUNT_ID: &str = "6123456789abcde";
    const PUBLIC_RAW: &str = "cpk_000000000000000000000000000000";
    const SECRET_RAW: &str = "csk_111111111111111111111111111111";

    fn actor() -> UserId {
        UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a8").expect("actor")
    }

    fn other_actor() -> UserId {
        UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a9").expect("actor")
    }

    fn request(input: LegacyDeveloperInputV1) -> LegacyDeveloperRequestV1 {
        LegacyDeveloperRequestV1 {
            credential: Some(LegacyDeveloperCredentialV1::Session),
            actor_id: Some(actor()),
            idempotency_key: Some("developer-action-0001".into()),
            input,
        }
    }

    fn create_request(name: &str) -> LegacyDeveloperRequestV1 {
        request(LegacyDeveloperInputV1::CreateApp {
            name: name.into(),
            environment: LegacyDeveloperEnvironmentV1::Development,
        })
    }

    fn update_request(
        name: Option<&str>,
        logo_url: LegacyDeveloperNullableLogoPatchV1,
    ) -> LegacyDeveloperRequestV1 {
        request(LegacyDeveloperInputV1::UpdateApp {
            legacy_app_id: APP.into(),
            name: name.map(str::to_owned),
            environment: None,
            logo_url,
        })
    }

    fn app_request(action: LegacyDeveloperActionV1) -> LegacyDeveloperRequestV1 {
        request(match action {
            LegacyDeveloperActionV1::DeleteApp => LegacyDeveloperInputV1::DeleteApp {
                legacy_app_id: APP.into(),
            },
            LegacyDeveloperActionV1::RegenerateKeys => LegacyDeveloperInputV1::RegenerateKeys {
                legacy_app_id: APP.into(),
            },
            _ => panic!("app-only fixture"),
        })
    }

    fn protected_provisioning(
        context: &LegacyDeveloperSecretGenerationContextV1,
    ) -> LegacyDeveloperProtectedProvisioningV1 {
        let public_key = LegacyDeveloperStoredKeyV1::new(
            LegacyDeveloperApiKeyIdV1::parse(PUBLIC_KEY_ID).expect("public id"),
            LegacyDeveloperKeyKindV1::Public,
            &PUBLIC_RAW[..LEGACY_DEVELOPER_KEY_PREFIX_LENGTH],
            "a".repeat(64),
            LegacyDeveloperProtectedBlobV1::new("encrypted-public-key").expect("encrypted"),
        )
        .expect("public key");
        let secret_key = LegacyDeveloperStoredKeyV1::new(
            LegacyDeveloperApiKeyIdV1::parse(SECRET_KEY_ID).expect("secret id"),
            LegacyDeveloperKeyKindV1::Secret,
            &SECRET_RAW[..LEGACY_DEVELOPER_KEY_PREFIX_LENGTH],
            "b".repeat(64),
            LegacyDeveloperProtectedBlobV1::new("encrypted-secret-key").expect("encrypted"),
        )
        .expect("secret key");
        let keys = LegacyDeveloperProtectedKeyPairV1::new(
            public_key,
            secret_key,
            LegacyDeveloperSealedKeyReplayV1::new(
                LegacyDeveloperProtectedBlobV1::new("sealed-response-key-pair").expect("sealed"),
                context.replay_binding(),
            ),
        )
        .expect("pair");
        match context.action() {
            LegacyDeveloperActionV1::CreateApp => {
                LegacyDeveloperProtectedProvisioningV1::CreateApp {
                    app_id: LegacyDeveloperAppIdV1::parse(APP).expect("app"),
                    credit_account_id: LegacyDeveloperCreditAccountIdV1::parse(CREDIT_ACCOUNT_ID)
                        .expect("credit"),
                    keys,
                }
            }
            LegacyDeveloperActionV1::RegenerateKeys => {
                LegacyDeveloperProtectedProvisioningV1::RegenerateKeys { keys }
            }
            _ => panic!("secret action"),
        }
    }

    fn authority_for(
        command: &LegacyDeveloperCommandV1,
    ) -> LegacyDeveloperAuthorityPostconditionV1 {
        match command {
            LegacyDeveloperCommandV1::CreateApp { .. } => {
                LegacyDeveloperAuthorityPostconditionV1::NewAppOwnedByActor {
                    actor_id: command.fence().authority.actor_id,
                }
            }
            _ => LegacyDeveloperAuthorityPostconditionV1::ExistingLiveAppOwnedByActor {
                app_id: command.app_id().expect("app").clone(),
                owner_id: command.fence().authority.actor_id,
            },
        }
    }

    fn mutation_for(
        command: &LegacyDeveloperCommandV1,
        provisioning: Option<LegacyDeveloperProtectedProvisioningV1>,
    ) -> LegacyDeveloperMutationPostconditionV1 {
        match command {
            LegacyDeveloperCommandV1::CreateApp {
                name, environment, ..
            } => LegacyDeveloperMutationPostconditionV1::AppCreated {
                owner_id: command.fence().authority.actor_id,
                stored_name: name.clone(),
                environment: *environment,
                provisioning: provisioning.expect("provisioning"),
                active_key_count_after: 2,
                credit_account_owner_id: command.fence().authority.actor_id,
                credit_balance_micro_credits: 0,
                auto_top_up: LegacyDeveloperAutoTopUpStateV1::new(false, 0, 0).expect("state"),
            },
            LegacyDeveloperCommandV1::UpdateApp { app_id, patch, .. } => {
                let final_logo_url = match &patch.logo_url {
                    LegacyDeveloperNullableLogoPatchV1::Missing => {
                        Some("https://existing.example/logo.png".into())
                    }
                    LegacyDeveloperNullableLogoPatchV1::Null => None,
                    LegacyDeveloperNullableLogoPatchV1::Value(value) => Some(value.clone()),
                };
                LegacyDeveloperMutationPostconditionV1::AppUpdated {
                    app_id: app_id.clone(),
                    final_name: patch.name.clone().unwrap_or_else(|| "Existing App".into()),
                    final_environment: patch
                        .environment
                        .unwrap_or(LegacyDeveloperEnvironmentV1::Development),
                    final_logo_url,
                    update_statement_executed: !patch.is_empty(),
                }
            }
            LegacyDeveloperCommandV1::DeleteApp { app_id, .. } => {
                LegacyDeveloperMutationPostconditionV1::AppDeleted {
                    app_id: app_id.clone(),
                    deleted_at: TimestampMillis::new(1_700_000_000_000).expect("time"),
                    revoked_active_key_count: 2,
                    active_key_count_after: 0,
                }
            }
            LegacyDeveloperCommandV1::AddDomain {
                app_id,
                normalized_origin,
                ..
            } => LegacyDeveloperMutationPostconditionV1::DomainAdded {
                app_id: app_id.clone(),
                domain_id: LegacyDeveloperDomainIdV1::parse(DOMAIN).expect("domain"),
                stored_origin: normalized_origin.clone(),
            },
            LegacyDeveloperCommandV1::RemoveDomain {
                app_id, domain_id, ..
            } => LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted {
                app_id: app_id.clone(),
                domain_id: domain_id.clone(),
                matched_rows: 0,
            },
            LegacyDeveloperCommandV1::RegenerateKeys { app_id, .. } => {
                LegacyDeveloperMutationPostconditionV1::KeysRegenerated {
                    app_id: app_id.clone(),
                    revoked_active_key_count: 2,
                    active_key_count_after: 2,
                    provisioning: provisioning.expect("provisioning"),
                }
            }
            LegacyDeveloperCommandV1::DeleteVideo {
                app_id, video_id, ..
            } => LegacyDeveloperMutationPostconditionV1::VideoDeleteAttempted {
                app_id: app_id.clone(),
                video_id: video_id.clone(),
                matched_rows: 0,
                deleted_at: None,
            },
            LegacyDeveloperCommandV1::UpdateAutoTopUp { app_id, patch, .. } => {
                LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated {
                    app_id: app_id.clone(),
                    account_state: Some(
                        LegacyDeveloperAutoTopUpStateV1::new(
                            patch.enabled,
                            patch.threshold_micro_credits.unwrap_or(50),
                            patch.amount_cents.unwrap_or(100),
                        )
                        .expect("state"),
                    ),
                }
            }
        }
    }

    fn receipt_for(
        command: &LegacyDeveloperCommandV1,
        provisioning: Option<LegacyDeveloperProtectedProvisioningV1>,
    ) -> LegacyDeveloperMutationReceiptV1 {
        LegacyDeveloperMutationReceiptV1::new(
            command,
            authority_for(command),
            mutation_for(command, provisioning),
        )
        .expect("receipt")
    }

    #[derive(Debug, Clone, Copy)]
    enum PortMode {
        Applied,
        Replay,
        Error(LegacyDeveloperAtomicErrorV1),
    }

    struct FakePort {
        mode: PortMode,
    }

    #[async_trait]
    impl LegacyDeveloperAtomicPortV1 for FakePort {
        async fn execute_atomic(
            &self,
            command: &LegacyDeveloperCommandV1,
            browser_fence: &LegacyDeveloperBrowserFenceV1,
            secrets: &dyn LegacyDeveloperSecretAuthorityV1,
        ) -> Result<LegacyDeveloperAtomicOutcomeV1, LegacyDeveloperAtomicErrorV1> {
            assert_eq!(browser_fence.actor_id(), command.fence().authority.actor_id);
            match self.mode {
                PortMode::Error(error) => Err(error),
                PortMode::Applied => {
                    let provisioning = if let Some(context) = command.secret_generation_context() {
                        Some(
                            secrets
                                .generate_protected(&context)
                                .await
                                .map_err(|_| LegacyDeveloperAtomicErrorV1::SecretUnavailable)?,
                        )
                    } else {
                        None
                    };
                    Ok(LegacyDeveloperAtomicOutcomeV1::Applied(receipt_for(
                        command,
                        provisioning,
                    )))
                }
                PortMode::Replay => {
                    let provisioning = command
                        .secret_generation_context()
                        .as_ref()
                        .map(protected_provisioning);
                    Ok(LegacyDeveloperAtomicOutcomeV1::Replay(receipt_for(
                        command,
                        provisioning,
                    )))
                }
            }
        }
    }

    #[derive(Default)]
    struct FakeSecrets {
        generated: Mutex<usize>,
        revealed: Mutex<usize>,
        fail_generation: bool,
        fail_reveal: bool,
    }

    #[async_trait]
    impl LegacyDeveloperSecretAuthorityV1 for FakeSecrets {
        async fn generate_protected(
            &self,
            context: &LegacyDeveloperSecretGenerationContextV1,
        ) -> Result<LegacyDeveloperProtectedProvisioningV1, LegacyDeveloperSecretErrorV1> {
            *self.generated.lock().expect("generated") += 1;
            if self.fail_generation {
                Err(LegacyDeveloperSecretErrorV1::Unavailable)
            } else {
                Ok(protected_provisioning(context))
            }
        }

        async fn reveal(
            &self,
            _replay: &LegacyDeveloperSealedKeyReplayV1,
        ) -> Result<LegacyDeveloperRevealedKeyPairV1, LegacyDeveloperSecretErrorV1> {
            *self.revealed.lock().expect("revealed") += 1;
            if self.fail_reveal {
                Err(LegacyDeveloperSecretErrorV1::Unavailable)
            } else {
                LegacyDeveloperRevealedKeyPairV1::new(PUBLIC_RAW, SECRET_RAW)
            }
        }
    }

    #[test]
    fn profiles_freeze_all_eight_exact_operations_without_promotion() {
        let expected = [
            (
                LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID,
                LEGACY_CREATE_DEVELOPER_APP_IDENTITY,
                LEGACY_CREATE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256,
                LEGACY_CREATE_DEVELOPER_APP_ACTION_SOURCE,
            ),
            (
                LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID,
                LEGACY_UPDATE_DEVELOPER_APP_IDENTITY,
                LEGACY_UPDATE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256,
                LEGACY_UPDATE_DEVELOPER_APP_ACTION_SOURCE,
            ),
            (
                LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID,
                LEGACY_DELETE_DEVELOPER_APP_IDENTITY,
                LEGACY_DELETE_DEVELOPER_APP_SOURCE_MANIFEST_SHA256,
                LEGACY_DELETE_DEVELOPER_APP_ACTION_SOURCE,
            ),
            (
                LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID,
                LEGACY_ADD_DEVELOPER_DOMAIN_IDENTITY,
                LEGACY_ADD_DEVELOPER_DOMAIN_SOURCE_MANIFEST_SHA256,
                LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_SOURCE,
            ),
            (
                LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID,
                LEGACY_REMOVE_DEVELOPER_DOMAIN_IDENTITY,
                LEGACY_REMOVE_DEVELOPER_DOMAIN_SOURCE_MANIFEST_SHA256,
                LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_SOURCE,
            ),
            (
                LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID,
                LEGACY_REGENERATE_DEVELOPER_KEYS_IDENTITY,
                LEGACY_REGENERATE_DEVELOPER_KEYS_SOURCE_MANIFEST_SHA256,
                LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_SOURCE,
            ),
            (
                LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID,
                LEGACY_DELETE_DEVELOPER_VIDEO_IDENTITY,
                LEGACY_DELETE_DEVELOPER_VIDEO_SOURCE_MANIFEST_SHA256,
                LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_SOURCE,
            ),
            (
                LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID,
                LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_IDENTITY,
                LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_SOURCE_MANIFEST_SHA256,
                LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_SOURCE,
            ),
        ];
        assert_eq!(LEGACY_DEVELOPER_PROFILES.len(), expected.len());
        for (profile, (operation, identity, manifest, action_source)) in
            LEGACY_DEVELOPER_PROFILES.iter().zip(expected)
        {
            assert_eq!(profile.operation_id, operation);
            assert_eq!(profile.legacy_identity, identity);
            assert_eq!(profile.source_manifest_sha256, manifest);
            assert_eq!(profile.source_closure.action, action_source);
            assert_eq!(profile.pinned_commit, LEGACY_DEVELOPER_ACTIONS_CAP_COMMIT);
            assert_eq!(profile.kind, "server_action");
            assert_eq!(profile.method, "ACTION");
            assert_eq!(profile.authentication, "session");
            assert_eq!(profile.policy, LEGACY_DEVELOPER_POLICY);
            assert!(profile.user_owner_non_disclosure);
            assert!(profile.idempotency_required);
            assert!(profile.one_use_browser_proof_required);
            assert_eq!(profile.protected_gates, ["released_legacy_client_e2e"]);
            assert!(!profile.production_promoted);
        }
    }

    #[test]
    fn every_source_closure_pin_is_complete_and_role_annotated() {
        for profile in LEGACY_DEVELOPER_PROFILES {
            assert!(profile.source_closure.source_count() >= 12);
            let pins = std::iter::once(&profile.source_closure.action)
                .chain(profile.source_closure.callers)
                .chain(profile.source_closure.supporting)
                .chain(profile.source_closure.secret_supporting);
            for pin in pins {
                assert!(!pin.path.is_empty());
                assert_eq!(pin.sha256.len(), 64);
                assert!(
                    pin.sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                );
                assert!(!pin.role.stable_code().is_empty());
            }
        }
        assert!(
            LEGACY_CREATE_DEVELOPER_APP_PROFILE
                .source_closure
                .secret_supporting
                .iter()
                .any(|pin| pin.role == LegacyDeveloperSourceRoleV1::Crypto)
        );
        assert!(
            LEGACY_REGENERATE_DEVELOPER_KEYS_PROFILE
                .source_closure
                .secret_supporting
                .iter()
                .any(|pin| pin.role == LegacyDeveloperSourceRoleV1::KeyHash)
        );
        assert!(
            LEGACY_UPDATE_DEVELOPER_APP_PROFILE
                .source_closure
                .secret_supporting
                .is_empty()
        );
        assert!(
            LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_PROFILE
                .source_closure
                .callers
                .is_empty()
        );
    }

    #[test]
    fn profile_shapes_freeze_plaintext_and_zero_row_quirks() {
        assert_eq!(
            LEGACY_CREATE_DEVELOPER_APP_PROFILE.observed_success,
            LegacyDeveloperObservedSuccessV1::AppIdentifierAndPlaintextKeyPair
        );
        assert_eq!(
            LEGACY_REGENERATE_DEVELOPER_KEYS_PROFILE.observed_success,
            LegacyDeveloperObservedSuccessV1::PlaintextKeyPair
        );
        assert_eq!(
            LEGACY_UPDATE_DEVELOPER_APP_PROFILE.required_mutation,
            LegacyDeveloperRequiredMutationV1::PatchOnlyPresentFieldsOrNoOp
        );
        assert_eq!(
            LEGACY_REMOVE_DEVELOPER_DOMAIN_PROFILE.required_mutation,
            LegacyDeveloperRequiredMutationV1::DeleteMatchingDomainIgnoringAffectedCount
        );
        assert_eq!(
            LEGACY_DELETE_DEVELOPER_VIDEO_PROFILE.required_mutation,
            LegacyDeveloperRequiredMutationV1::SoftDeleteMatchingVideoIgnoringAffectedCount
        );
    }

    #[test]
    fn session_actor_and_idempotency_are_mandatory() {
        let adapter = LegacyDeveloperAdapterV1::create_app();
        let mut candidate = create_request("App");
        candidate.credential = Some(LegacyDeveloperCredentialV1::ApiKey);
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyDeveloperErrorV1::Unauthorized)
        );
        let mut candidate = create_request("App");
        candidate.actor_id = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyDeveloperErrorV1::Unauthorized)
        );
        let mut candidate = create_request("App");
        candidate.idempotency_key = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyDeveloperErrorV1::IdempotencyRequired)
        );
        let mut candidate = create_request("App");
        candidate.idempotency_key = Some("short".into());
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyDeveloperErrorV1::Invalid)
        );
    }

    #[test]
    fn create_trims_name_and_preserves_environment() {
        let mut candidate = create_request("  My App  ");
        let LegacyDeveloperInputV1::CreateApp { environment, .. } = &mut candidate.input else {
            unreachable!()
        };
        *environment = LegacyDeveloperEnvironmentV1::Production;
        let command = LegacyDeveloperAdapterV1::create_app()
            .prepare(&candidate)
            .expect("command");
        let LegacyDeveloperCommandV1::CreateApp {
            name, environment, ..
        } = command
        else {
            unreachable!()
        };
        assert_eq!(name, "My App");
        assert_eq!(environment.stable_code(), "production");
    }

    #[test]
    fn create_and_update_preserve_their_distinct_empty_name_failures() {
        for value in ["", "   "] {
            assert_eq!(
                LegacyDeveloperAdapterV1::create_app().prepare(&create_request(value)),
                Err(LegacyDeveloperErrorV1::AppNameRequired)
            );
            assert_eq!(
                LegacyDeveloperAdapterV1::update_app().prepare(&update_request(
                    Some(value),
                    LegacyDeveloperNullableLogoPatchV1::Missing,
                )),
                Err(LegacyDeveloperErrorV1::AppNameEmpty)
            );
        }
        assert_eq!(
            LegacyDeveloperAdapterV1::create_app().prepare(&create_request(
                &"a".repeat(LEGACY_DEVELOPER_MAX_APP_NAME_CHARS + 1)
            )),
            Err(LegacyDeveloperErrorV1::Invalid)
        );
    }

    #[test]
    fn update_freezes_missing_null_and_value_logo_semantics() {
        let missing = LegacyDeveloperAdapterV1::update_app()
            .prepare(&update_request(
                None,
                LegacyDeveloperNullableLogoPatchV1::Missing,
            ))
            .expect("missing");
        let null = LegacyDeveloperAdapterV1::update_app()
            .prepare(&update_request(
                None,
                LegacyDeveloperNullableLogoPatchV1::Null,
            ))
            .expect("null");
        let value = LegacyDeveloperAdapterV1::update_app()
            .prepare(&update_request(
                None,
                LegacyDeveloperNullableLogoPatchV1::Value(String::new()),
            ))
            .expect("value");
        assert_ne!(
            missing.fence().request_fingerprint(),
            null.fence().request_fingerprint()
        );
        assert_ne!(
            null.fence().request_fingerprint(),
            value.fence().request_fingerprint()
        );
        let LegacyDeveloperCommandV1::UpdateApp { patch, .. } = missing else {
            unreachable!()
        };
        assert!(patch.is_empty());
        assert_eq!(
            LegacyDeveloperAdapterV1::update_app().prepare(&update_request(
                None,
                LegacyDeveloperNullableLogoPatchV1::Value(
                    "x".repeat(LEGACY_DEVELOPER_MAX_LOGO_URL_CHARS + 1),
                ),
            )),
            Err(LegacyDeveloperErrorV1::Invalid)
        );
    }

    #[test]
    fn full_origins_follow_the_pinned_regex_and_lowercase_rule() {
        for origin in [
            "HTTPS://EXAMPLE.COM",
            "http://localhost:3000",
            "https://sub-domain.example.",
        ] {
            let command = LegacyDeveloperAdapterV1::add_domain()
                .prepare(&request(LegacyDeveloperInputV1::AddDomain {
                    legacy_app_id: APP.into(),
                    domain: origin.into(),
                }))
                .expect("origin");
            let LegacyDeveloperCommandV1::AddDomain {
                normalized_origin, ..
            } = command
            else {
                unreachable!()
            };
            assert_eq!(normalized_origin, origin.trim().to_ascii_lowercase());
        }
    }

    #[test]
    fn invalid_and_empty_origins_keep_exact_failures() {
        let build = |domain: &str| {
            LegacyDeveloperAdapterV1::add_domain().prepare(&request(
                LegacyDeveloperInputV1::AddDomain {
                    legacy_app_id: APP.into(),
                    domain: domain.into(),
                },
            ))
        };
        assert_eq!(build("   "), Err(LegacyDeveloperErrorV1::DomainRequired));
        for domain in [
            "example.com",
            "ftp://example.com",
            "https://example.com/path",
            "https://example.com:",
            "https://exa_mple.com",
        ] {
            assert_eq!(build(domain), Err(LegacyDeveloperErrorV1::DomainInvalid));
        }
    }

    #[test]
    fn cap_nanoids_are_exact_and_malformed_app_ids_are_non_disclosing() {
        let mut candidate = app_request(LegacyDeveloperActionV1::DeleteApp);
        let LegacyDeveloperInputV1::DeleteApp { legacy_app_id } = &mut candidate.input else {
            unreachable!()
        };
        *legacy_app_id = "app-456".into();
        assert_eq!(
            LegacyDeveloperAdapterV1::delete_app().prepare(&candidate),
            Err(LegacyDeveloperErrorV1::AppNotFound)
        );
        assert!(LegacyDeveloperAppIdV1::parse(APP).is_ok());
        assert!(LegacyDeveloperAppIdV1::parse("0123456789abcdi").is_err());
    }

    #[test]
    fn auto_top_up_validation_order_and_limits_match_cap() {
        let prepare = |threshold, amount| {
            LegacyDeveloperAdapterV1::update_auto_top_up().prepare(&request(
                LegacyDeveloperInputV1::UpdateAutoTopUp {
                    legacy_app_id: APP.into(),
                    enabled: true,
                    threshold_micro_credits: threshold,
                    amount_cents: amount,
                },
            ))
        };
        assert_eq!(
            prepare(Some(-1), Some(0)),
            Err(LegacyDeveloperErrorV1::ThresholdNegative)
        );
        assert_eq!(
            prepare(None, Some(0)),
            Err(LegacyDeveloperErrorV1::TopUpAmountNonPositive)
        );
        assert_eq!(
            prepare(None, Some(-1)),
            Err(LegacyDeveloperErrorV1::TopUpAmountNonPositive)
        );
        assert_eq!(
            prepare(None, Some(LEGACY_DEVELOPER_MAX_TOP_UP_CENTS + 1)),
            Err(LegacyDeveloperErrorV1::TopUpAmountTooLarge)
        );
        assert!(prepare(Some(0), Some(1)).is_ok());
        assert!(prepare(Some(MAX_LEDGER_AMOUNT), Some(100_000)).is_ok());
    }

    #[test]
    fn auto_top_up_optional_fields_are_fingerprint_significant() {
        let build = |threshold, amount| {
            LegacyDeveloperAdapterV1::update_auto_top_up()
                .prepare(&request(LegacyDeveloperInputV1::UpdateAutoTopUp {
                    legacy_app_id: APP.into(),
                    enabled: true,
                    threshold_micro_credits: threshold,
                    amount_cents: amount,
                }))
                .expect("command")
        };
        let missing = build(None, None);
        let zero_threshold = build(Some(0), None);
        let amount = build(None, Some(1));
        assert_ne!(
            missing.fence().request_fingerprint(),
            zero_threshold.fence().request_fingerprint()
        );
        assert_ne!(
            missing.fence().request_fingerprint(),
            amount.fence().request_fingerprint()
        );
    }

    #[test]
    fn fingerprints_bind_action_actor_app_and_canonical_values() {
        let first = LegacyDeveloperAdapterV1::add_domain()
            .prepare(&request(LegacyDeveloperInputV1::AddDomain {
                legacy_app_id: APP.into(),
                domain: " HTTPS://EXAMPLE.COM ".into(),
            }))
            .expect("first");
        let second = LegacyDeveloperAdapterV1::add_domain()
            .prepare(&request(LegacyDeveloperInputV1::AddDomain {
                legacy_app_id: APP.into(),
                domain: "https://example.com".into(),
            }))
            .expect("second");
        assert_eq!(
            first.fence().request_fingerprint(),
            second.fence().request_fingerprint()
        );

        let mut other = request(LegacyDeveloperInputV1::AddDomain {
            legacy_app_id: OTHER_APP.into(),
            domain: "https://example.com".into(),
        });
        other.actor_id = Some(other_actor());
        let other = LegacyDeveloperAdapterV1::add_domain()
            .prepare(&other)
            .expect("other");
        assert_ne!(
            first.fence().request_fingerprint(),
            other.fence().request_fingerprint()
        );
    }

    #[test]
    fn action_mismatch_is_rejected_before_the_port() {
        assert_eq!(
            LegacyDeveloperAdapterV1::delete_app().prepare(&create_request("App")),
            Err(LegacyDeveloperErrorV1::Invalid)
        );
    }

    #[test]
    fn request_command_and_fence_debug_are_redacted() {
        let request = request(LegacyDeveloperInputV1::AddDomain {
            legacy_app_id: APP.into(),
            domain: "https://private.example".into(),
        });
        let request_debug = format!("{request:?}");
        assert!(!request_debug.contains(APP));
        assert!(!request_debug.contains("private.example"));
        assert!(!request_debug.contains("developer-action-0001"));
        let command = LegacyDeveloperAdapterV1::add_domain()
            .prepare(&request)
            .expect("command");
        let command_debug = format!("{command:?}");
        assert!(!command_debug.contains(APP));
        assert!(!command_debug.contains("private.example"));
    }

    #[test]
    fn secret_material_validates_ids_prefixes_hashes_and_distinctness() {
        assert!(LegacyDeveloperRevealedKeyPairV1::new(PUBLIC_RAW, SECRET_RAW).is_ok());
        assert!(LegacyDeveloperRevealedKeyPairV1::new("cpk_short", SECRET_RAW).is_err());
        assert!(
            LegacyDeveloperStoredKeyV1::new(
                LegacyDeveloperApiKeyIdV1::parse(PUBLIC_KEY_ID).expect("id"),
                LegacyDeveloperKeyKindV1::Public,
                "csk_11111111",
                "a".repeat(64),
                LegacyDeveloperProtectedBlobV1::new("encrypted").expect("blob"),
            )
            .is_err()
        );
        assert!(
            LegacyDeveloperStoredKeyV1::new(
                LegacyDeveloperApiKeyIdV1::parse(PUBLIC_KEY_ID).expect("id"),
                LegacyDeveloperKeyKindV1::Public,
                &PUBLIC_RAW[..12],
                "not-a-hash",
                LegacyDeveloperProtectedBlobV1::new("encrypted").expect("blob"),
            )
            .is_err()
        );
        assert!(LegacyDeveloperProtectedBlobV1::new("").is_err());
    }

    #[test]
    fn plaintext_and_ciphertext_never_appear_in_debug_output() {
        let command = LegacyDeveloperAdapterV1::create_app()
            .prepare(&create_request("Private App"))
            .expect("command");
        let context = command.secret_generation_context().expect("context");
        let provisioning = protected_provisioning(&context);
        let receipt = receipt_for(&command, Some(provisioning.clone()));
        let revealed = LegacyDeveloperRevealedKeyPairV1::new(PUBLIC_RAW, SECRET_RAW).expect("keys");
        for debug in [
            format!("{provisioning:?}"),
            format!("{receipt:?}"),
            format!("{revealed:?}"),
        ] {
            assert!(!debug.contains(PUBLIC_RAW));
            assert!(!debug.contains(SECRET_RAW));
            assert!(!debug.contains("encrypted-public-key"));
            assert!(!debug.contains("sealed-response-key-pair"));
            assert!(!debug.contains("Private App"));
        }
    }

    #[test]
    fn create_receipt_requires_exact_owner_defaults_and_binding() {
        let command = LegacyDeveloperAdapterV1::create_app()
            .prepare(&create_request("App"))
            .expect("command");
        let context = command.secret_generation_context().expect("context");
        assert!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                mutation_for(&command, Some(protected_provisioning(&context))),
            )
            .is_ok()
        );

        let wrong_authority = LegacyDeveloperAuthorityPostconditionV1::NewAppOwnedByActor {
            actor_id: other_actor(),
        };
        assert_eq!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                wrong_authority,
                mutation_for(&command, Some(protected_provisioning(&context))),
            ),
            Err(LegacyDeveloperAtomicErrorV1::Corrupt)
        );

        let wrong_context = LegacyDeveloperSecretGenerationContextV1 {
            actor_id: other_actor(),
            ..context
        };
        assert_eq!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                mutation_for(&command, Some(protected_provisioning(&wrong_context))),
            ),
            Err(LegacyDeveloperAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn empty_update_patch_succeeds_only_without_an_update_statement() {
        let command = LegacyDeveloperAdapterV1::update_app()
            .prepare(&update_request(
                None,
                LegacyDeveloperNullableLogoPatchV1::Missing,
            ))
            .expect("command");
        let mutation = mutation_for(&command, None);
        assert!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                mutation.clone(),
            )
            .is_ok()
        );
        let LegacyDeveloperMutationPostconditionV1::AppUpdated {
            app_id,
            final_name,
            final_environment,
            final_logo_url,
            ..
        } = mutation
        else {
            unreachable!()
        };
        let corrupt = LegacyDeveloperMutationPostconditionV1::AppUpdated {
            app_id,
            final_name,
            final_environment,
            final_logo_url,
            update_statement_executed: true,
        };
        assert_eq!(
            LegacyDeveloperMutationReceiptV1::new(&command, authority_for(&command), corrupt,),
            Err(LegacyDeveloperAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn domain_removal_accepts_zero_or_one_affected_row_and_nothing_else() {
        let command = LegacyDeveloperAdapterV1::remove_domain()
            .prepare(&request(LegacyDeveloperInputV1::RemoveDomain {
                legacy_app_id: APP.into(),
                legacy_domain_id: DOMAIN.into(),
            }))
            .expect("command");
        for matched_rows in [0, 1] {
            let LegacyDeveloperCommandV1::RemoveDomain {
                app_id, domain_id, ..
            } = &command
            else {
                unreachable!()
            };
            assert!(
                LegacyDeveloperMutationReceiptV1::new(
                    &command,
                    authority_for(&command),
                    LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted {
                        app_id: app_id.clone(),
                        domain_id: domain_id.clone(),
                        matched_rows,
                    },
                )
                .is_ok()
            );
        }
        let LegacyDeveloperCommandV1::RemoveDomain {
            app_id, domain_id, ..
        } = &command
        else {
            unreachable!()
        };
        assert_eq!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                LegacyDeveloperMutationPostconditionV1::DomainDeleteAttempted {
                    app_id: app_id.clone(),
                    domain_id: domain_id.clone(),
                    matched_rows: 2,
                },
            ),
            Err(LegacyDeveloperAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn video_delete_timestamp_is_present_exactly_when_a_row_matched() {
        let command = LegacyDeveloperAdapterV1::delete_video()
            .prepare(&request(LegacyDeveloperInputV1::DeleteVideo {
                legacy_app_id: APP.into(),
                legacy_video_id: VIDEO.into(),
            }))
            .expect("command");
        let LegacyDeveloperCommandV1::DeleteVideo {
            app_id, video_id, ..
        } = &command
        else {
            unreachable!()
        };
        for (matched_rows, deleted_at, valid) in [
            (0, None, true),
            (1, Some(TimestampMillis::new(1).expect("time")), true),
            (0, Some(TimestampMillis::new(1).expect("time")), false),
            (1, None, false),
        ] {
            let result = LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                LegacyDeveloperMutationPostconditionV1::VideoDeleteAttempted {
                    app_id: app_id.clone(),
                    video_id: video_id.clone(),
                    matched_rows,
                    deleted_at,
                },
            );
            assert_eq!(result.is_ok(), valid);
        }
    }

    #[test]
    fn auto_top_up_zero_row_success_and_present_field_postconditions_are_typed() {
        let command = LegacyDeveloperAdapterV1::update_auto_top_up()
            .prepare(&request(LegacyDeveloperInputV1::UpdateAutoTopUp {
                legacy_app_id: APP.into(),
                enabled: true,
                threshold_micro_credits: Some(50),
                amount_cents: None,
            }))
            .expect("command");
        let app_id = command.app_id().expect("app").clone();
        assert!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated {
                    app_id: app_id.clone(),
                    account_state: None,
                },
            )
            .is_ok()
        );
        let wrong = LegacyDeveloperAutoTopUpStateV1::new(true, 49, 100).expect("state");
        assert_eq!(
            LegacyDeveloperMutationReceiptV1::new(
                &command,
                authority_for(&command),
                LegacyDeveloperMutationPostconditionV1::AutoTopUpUpdated {
                    app_id,
                    account_state: Some(wrong),
                },
            ),
            Err(LegacyDeveloperAtomicErrorV1::Corrupt)
        );
    }

    #[tokio::test]
    async fn create_applied_returns_exact_keys_and_does_not_server_revalidate() {
        let secrets = FakeSecrets::default();
        let execution = LegacyDeveloperAdapterV1::create_app()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Applied,
                },
                &secrets,
                &create_request("App"),
                &LegacyDeveloperBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        assert_eq!(execution.success().app_id(), Some(APP));
        let keys = execution.success().keys().expect("keys");
        assert_eq!(keys.expose_public_key(), PUBLIC_RAW);
        assert_eq!(keys.expose_secret_key(), SECRET_RAW);
        assert!(!execution.replayed());
        assert_eq!(execution.effects().path(), None);
        assert_eq!(*secrets.generated.lock().expect("generated"), 1);
        assert_eq!(*secrets.revealed.lock().expect("revealed"), 1);
    }

    #[tokio::test]
    async fn create_replay_uses_journaled_envelope_without_generating_new_keys() {
        let secrets = FakeSecrets::default();
        let execution = LegacyDeveloperAdapterV1::create_app()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Replay,
                },
                &secrets,
                &create_request("App"),
                &LegacyDeveloperBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        assert!(execution.replayed());
        assert_eq!(execution.success().app_id(), Some(APP));
        assert_eq!(
            execution
                .success()
                .keys()
                .expect("keys")
                .expose_secret_key(),
            SECRET_RAW
        );
        assert_eq!(*secrets.generated.lock().expect("generated"), 0);
        assert_eq!(*secrets.revealed.lock().expect("revealed"), 1);
    }

    #[tokio::test]
    async fn regenerate_applied_revokes_and_returns_exact_new_pair() {
        let secrets = FakeSecrets::default();
        let execution = LegacyDeveloperAdapterV1::regenerate_keys()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Applied,
                },
                &secrets,
                &app_request(LegacyDeveloperActionV1::RegenerateKeys),
                &LegacyDeveloperBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        assert_eq!(execution.success().app_id(), None);
        assert_eq!(
            execution
                .success()
                .keys()
                .expect("keys")
                .expose_public_key(),
            PUBLIC_RAW
        );
        assert_eq!(
            execution.effects().path(),
            Some(LEGACY_DEVELOPER_DASHBOARD_REVALIDATION_PATH)
        );
    }

    #[tokio::test]
    async fn object_success_actions_preserve_success_true_and_revalidation() {
        let cases = [
            (
                LegacyDeveloperAdapterV1::update_app(),
                update_request(None, LegacyDeveloperNullableLogoPatchV1::Missing),
            ),
            (
                LegacyDeveloperAdapterV1::delete_app(),
                app_request(LegacyDeveloperActionV1::DeleteApp),
            ),
            (
                LegacyDeveloperAdapterV1::add_domain(),
                request(LegacyDeveloperInputV1::AddDomain {
                    legacy_app_id: APP.into(),
                    domain: "https://example.com".into(),
                }),
            ),
            (
                LegacyDeveloperAdapterV1::remove_domain(),
                request(LegacyDeveloperInputV1::RemoveDomain {
                    legacy_app_id: APP.into(),
                    legacy_domain_id: DOMAIN.into(),
                }),
            ),
            (
                LegacyDeveloperAdapterV1::delete_video(),
                request(LegacyDeveloperInputV1::DeleteVideo {
                    legacy_app_id: APP.into(),
                    legacy_video_id: VIDEO.into(),
                }),
            ),
            (
                LegacyDeveloperAdapterV1::update_auto_top_up(),
                request(LegacyDeveloperInputV1::UpdateAutoTopUp {
                    legacy_app_id: APP.into(),
                    enabled: false,
                    threshold_micro_credits: None,
                    amount_cents: None,
                }),
            ),
        ];
        for (adapter, request) in cases {
            let execution = adapter
                .execute_fenced(
                    &FakePort {
                        mode: PortMode::Applied,
                    },
                    &FakeSecrets::default(),
                    &request,
                    &LegacyDeveloperBrowserFenceV1::fixture(actor()),
                )
                .await
                .expect("execution");
            assert_eq!(execution.success().object_success(), Some(true));
            assert_eq!(
                execution.effects().path(),
                Some(LEGACY_DEVELOPER_DASHBOARD_REVALIDATION_PATH)
            );
        }
    }

    #[tokio::test]
    async fn browser_fence_actor_mismatch_fails_before_the_port() {
        let error = LegacyDeveloperAdapterV1::delete_app()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Applied,
                },
                &FakeSecrets::default(),
                &app_request(LegacyDeveloperActionV1::DeleteApp),
                &LegacyDeveloperBrowserFenceV1::fixture(other_actor()),
            )
            .await
            .expect_err("mismatch");
        assert_eq!(error, LegacyDeveloperErrorV1::Unauthorized);
    }

    #[tokio::test]
    async fn all_owner_authority_denials_are_non_disclosing() {
        for atomic_error in [
            LegacyDeveloperAtomicErrorV1::AppMissing,
            LegacyDeveloperAtomicErrorV1::NotOwner,
            LegacyDeveloperAtomicErrorV1::Deleted,
            LegacyDeveloperAtomicErrorV1::StaleAuthority,
        ] {
            let error = LegacyDeveloperAdapterV1::delete_app()
                .execute_fenced(
                    &FakePort {
                        mode: PortMode::Error(atomic_error),
                    },
                    &FakeSecrets::default(),
                    &app_request(LegacyDeveloperActionV1::DeleteApp),
                    &LegacyDeveloperBrowserFenceV1::fixture(actor()),
                )
                .await
                .expect_err("denied");
            assert_eq!(error, LegacyDeveloperErrorV1::AppNotFound);
        }
    }

    #[tokio::test]
    async fn replay_conflict_and_race_have_one_stable_public_failure() {
        for atomic_error in [
            LegacyDeveloperAtomicErrorV1::DuplicateDomain,
            LegacyDeveloperAtomicErrorV1::Conflict,
            LegacyDeveloperAtomicErrorV1::InFlight,
        ] {
            let error = LegacyDeveloperAdapterV1::delete_app()
                .execute_fenced(
                    &FakePort {
                        mode: PortMode::Error(atomic_error),
                    },
                    &FakeSecrets::default(),
                    &app_request(LegacyDeveloperActionV1::DeleteApp),
                    &LegacyDeveloperBrowserFenceV1::fixture(actor()),
                )
                .await
                .expect_err("conflict");
            assert_eq!(error, LegacyDeveloperErrorV1::Conflict);
        }
    }

    #[tokio::test]
    async fn generation_and_reveal_fail_closed_without_leaking_provider_details() {
        let generation_error = LegacyDeveloperAdapterV1::create_app()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Applied,
                },
                &FakeSecrets {
                    fail_generation: true,
                    ..FakeSecrets::default()
                },
                &create_request("App"),
                &LegacyDeveloperBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect_err("generation");
        assert_eq!(generation_error, LegacyDeveloperErrorV1::SecretUnavailable);

        let reveal_error = LegacyDeveloperAdapterV1::create_app()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Replay,
                },
                &FakeSecrets {
                    fail_reveal: true,
                    ..FakeSecrets::default()
                },
                &create_request("App"),
                &LegacyDeveloperBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect_err("reveal");
        assert_eq!(reveal_error, LegacyDeveloperErrorV1::SecretUnavailable);
        assert_eq!(format!("{reveal_error:?}"), "SecretUnavailable");
    }

    #[test]
    fn secret_replay_binding_changes_with_action_actor_app_and_request() {
        let create = LegacyDeveloperAdapterV1::create_app()
            .prepare(&create_request("App"))
            .expect("create")
            .secret_generation_context()
            .expect("context");
        let create_other = LegacyDeveloperAdapterV1::create_app()
            .prepare(&create_request("Other App"))
            .expect("create")
            .secret_generation_context()
            .expect("context");
        let regenerate = LegacyDeveloperAdapterV1::regenerate_keys()
            .prepare(&app_request(LegacyDeveloperActionV1::RegenerateKeys))
            .expect("regenerate")
            .secret_generation_context()
            .expect("context");
        assert_ne!(create.replay_binding(), create_other.replay_binding());
        assert_ne!(create.replay_binding(), regenerate.replay_binding());
        let mut other_actor_context = create.clone();
        other_actor_context.actor_id = other_actor();
        assert_ne!(
            create.replay_binding(),
            other_actor_context.replay_binding()
        );
    }

    #[tokio::test]
    async fn execution_debug_redacts_revealed_credentials() {
        let execution = LegacyDeveloperAdapterV1::create_app()
            .execute_fenced(
                &FakePort {
                    mode: PortMode::Replay,
                },
                &FakeSecrets::default(),
                &create_request("App"),
                &LegacyDeveloperBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        let debug = format!("{execution:?}");
        assert!(!debug.contains(PUBLIC_RAW));
        assert!(!debug.contains(SECRET_RAW));
        assert!(!debug.contains(APP));
    }
}
