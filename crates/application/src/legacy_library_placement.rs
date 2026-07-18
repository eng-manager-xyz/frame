//! Source-pinned compatibility contracts for Cap's four library-root placement actions.
//!
//! Cap's actions mixed global lookups, permissive organization membership checks,
//! actor-owned-video filtering, and multi-statement writes without a durable replay
//! boundary. This module records those observed semantics without copying their
//! authority and atomicity defects. The Frame port instead receives canonical typed
//! identifiers plus a trusted active-tenant fence and must authorize and commit the
//! complete mutation and its replay journal in one transaction.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    IdempotencyKey, LegacyCapNanoId, OrganizationId, SessionId, SessionMutationGrantId, SpaceId,
    UserId, VideoId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;
use crate::legacy_folder_assignment::{
    LegacyFolderAssignmentCredentialV1, LegacyFolderAssignmentScopeV1,
    MAX_LEGACY_FOLDER_ASSIGNMENT_VIDEO_IDS,
};

pub const LEGACY_LIBRARY_PLACEMENT_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID: &str = "cap-v1-d96a1931942eb83b";
pub const LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID: &str = "cap-v1-0694e68a64976c9a";
pub const LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID: &str = "cap-v1-bb55b5eeeb5e31ab";
pub const LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID: &str = "cap-v1-ccbe5f1381eaa1b4";
pub const LEGACY_ADD_VIDEOS_TO_ORGANIZATION_IDENTITY: &str =
    "action://apps/web/actions/organizations/add-videos.ts#addVideosToOrganization";
pub const LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_IDENTITY: &str =
    "action://apps/web/actions/organizations/remove-videos.ts#removeVideosFromOrganization";
pub const LEGACY_ADD_VIDEOS_TO_SPACE_IDENTITY: &str =
    "action://apps/web/actions/spaces/add-videos.ts#addVideosToSpace";
pub const LEGACY_REMOVE_VIDEOS_FROM_SPACE_IDENTITY: &str =
    "action://apps/web/actions/spaces/remove-videos.ts#removeVideosFromSpace";
pub const LEGACY_LIBRARY_PLACEMENT_POLICY: &str = "organization_library.v1";
pub const LEGACY_LIBRARY_PLACEMENT_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_LIBRARY_PLACEMENT_MAX_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryPlacementSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
}

pub const LEGACY_ADD_VIDEOS_TO_ORGANIZATION_SOURCES: &[LegacyLibraryPlacementSourcePinV1] = &[
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/organizations/add-videos.ts",
        sha256: "127ccc6ab701c04082cf8010281dbf70daee2ec6c54c01b3af1ebec5b56310c9",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

pub const LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_SOURCES: &[LegacyLibraryPlacementSourcePinV1] = &[
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/organizations/remove-videos.ts",
        sha256: "c67c82c0d5d64229046075569d384bfee3766fea3f7a4adbd592bba1a204bfac",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

pub const LEGACY_ADD_VIDEOS_TO_SPACE_SOURCES: &[LegacyLibraryPlacementSourcePinV1] = &[
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/spaces/add-videos.ts",
        sha256: "b15a27c2e1e522f97dcdfcb1802ffd7c449a34420b311ca2273f8c8f737581fa",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/organization/authorization.ts",
        sha256: "6b1422de53d0915a985dc1dbf70f14494fd1c5fe49ca61fbacabd90bebb00980",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/organization/space-authorization.ts",
        sha256: "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

pub const LEGACY_REMOVE_VIDEOS_FROM_SPACE_SOURCES: &[LegacyLibraryPlacementSourcePinV1] = &[
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/spaces/remove-videos.ts",
        sha256: "a88805652fd94c0baba35afe2d8e3b46d5cf9100362ec8370cba8f43e9b611dc",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/organization/authorization.ts",
        sha256: "6b1422de53d0915a985dc1dbf70f14494fd1c5fe49ca61fbacabd90bebb00980",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/actions/organization/space-authorization.ts",
        sha256: "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    },
    LegacyLibraryPlacementSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementActionV1 {
    AddToOrganization,
    RemoveFromOrganization,
    AddToSpace,
    RemoveFromSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementInputShapeV1 {
    PositionalOrganizationAndVideoList,
    PositionalSpaceOrOrganizationAndVideoList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementObservedAuthorizationV1 {
    OwnerOrAnyMemberAndActorOwnedVideoFilter,
    OwnerOrAnyMemberAndAnyScopedShare,
    ActiveOrganizationBranchOrSpaceManagerAndActorOwnedVideoFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementRequiredAuthorizationV1 {
    SessionActorActiveTenantManagerAndEveryActorOwnedVideo,
    SessionActorActiveTenantManagerAndOnlyMatchingOrganizationShares,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementRequiredMutationV1 {
    UpsertOrganizationShareAndClearOnlyItsScopedFolder,
    DeleteOrganizationSharesAndClearOnlyDirectFoldersInOrganization,
    UpsertSelectedScopeMembershipAndClearOnlyItsScopedFolder,
    DeleteOnlySelectedScopeMembership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementObservedMutationV1 {
    MoveExistingToRootThenInsertWithoutTransaction,
    DeleteSharesThenClearFoldersWithoutTransaction,
    BranchScopedDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementObservedSuccessV1 {
    OrganizationRootMessage,
    OrganizationRemovedOrNoMatchingMessage,
    ScopeAddedMessage,
    ScopeRemovedMessageAndDeletedCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementObservedFailureV1 {
    CaughtObjectWithError,
    CaughtObjectWithMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementObservedRetryV1 {
    ConvergentButNonAtomic,
    NonAtomicReplayChangesToNoMatching,
    RepeatedDeleteReportsValidatedInputCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementReplayV1 {
    ReturnOriginalJournaledSuccessWithoutReapplyingMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementKeyReuseV1 {
    SameFingerprintReplaysDifferentFingerprintConflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementAtomicityV1 {
    AuthorityMutationOutcomeAndJournalInOneTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryPlacementRequiredRetryV1 {
    pub replay: LegacyLibraryPlacementReplayV1,
    pub key_reuse: LegacyLibraryPlacementKeyReuseV1,
    pub atomicity: LegacyLibraryPlacementAtomicityV1,
}

pub const LEGACY_LIBRARY_PLACEMENT_REQUIRED_RETRY: LegacyLibraryPlacementRequiredRetryV1 =
    LegacyLibraryPlacementRequiredRetryV1 {
        replay:
            LegacyLibraryPlacementReplayV1::ReturnOriginalJournaledSuccessWithoutReapplyingMutation,
        key_reuse:
            LegacyLibraryPlacementKeyReuseV1::SameFingerprintReplaysDifferentFingerprintConflicts,
        atomicity:
            LegacyLibraryPlacementAtomicityV1::AuthorityMutationOutcomeAndJournalInOneTransaction,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryPlacementProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub sources: &'static [LegacyLibraryPlacementSourcePinV1],
    pub authentication: &'static str,
    pub policy: &'static str,
    pub content_type: &'static str,
    pub max_body_bytes: usize,
    pub input: LegacyLibraryPlacementInputShapeV1,
    pub observed_authorization: LegacyLibraryPlacementObservedAuthorizationV1,
    pub required_authorization: LegacyLibraryPlacementRequiredAuthorizationV1,
    pub observed_mutation: LegacyLibraryPlacementObservedMutationV1,
    pub required_mutation: LegacyLibraryPlacementRequiredMutationV1,
    pub observed_success: LegacyLibraryPlacementObservedSuccessV1,
    pub observed_failure: LegacyLibraryPlacementObservedFailureV1,
    pub observed_retry: LegacyLibraryPlacementObservedRetryV1,
    pub required_retry: LegacyLibraryPlacementRequiredRetryV1,
    pub tenant_non_disclosure: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

pub const LEGACY_ADD_VIDEOS_TO_ORGANIZATION_PROFILE: LegacyLibraryPlacementProfileV1 =
    LegacyLibraryPlacementProfileV1 {
        operation_id: LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_ADD_VIDEOS_TO_ORGANIZATION_IDENTITY,
        pinned_commit: LEGACY_LIBRARY_PLACEMENT_CAP_COMMIT,
        sources: LEGACY_ADD_VIDEOS_TO_ORGANIZATION_SOURCES,
        authentication: "session",
        policy: LEGACY_LIBRARY_PLACEMENT_POLICY,
        content_type: LEGACY_LIBRARY_PLACEMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_LIBRARY_PLACEMENT_MAX_BODY_BYTES,
        input: LegacyLibraryPlacementInputShapeV1::PositionalOrganizationAndVideoList,
        observed_authorization:
            LegacyLibraryPlacementObservedAuthorizationV1::OwnerOrAnyMemberAndActorOwnedVideoFilter,
        required_authorization:
            LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndEveryActorOwnedVideo,
        observed_mutation:
            LegacyLibraryPlacementObservedMutationV1::MoveExistingToRootThenInsertWithoutTransaction,
        required_mutation:
            LegacyLibraryPlacementRequiredMutationV1::UpsertOrganizationShareAndClearOnlyItsScopedFolder,
        observed_success: LegacyLibraryPlacementObservedSuccessV1::OrganizationRootMessage,
        observed_failure: LegacyLibraryPlacementObservedFailureV1::CaughtObjectWithError,
        observed_retry: LegacyLibraryPlacementObservedRetryV1::ConvergentButNonAtomic,
        required_retry: LEGACY_LIBRARY_PLACEMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
        production_promoted: false,
    };

pub const LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_PROFILE: LegacyLibraryPlacementProfileV1 =
    LegacyLibraryPlacementProfileV1 {
        operation_id: LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_IDENTITY,
        pinned_commit: LEGACY_LIBRARY_PLACEMENT_CAP_COMMIT,
        sources: LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_SOURCES,
        authentication: "session",
        policy: LEGACY_LIBRARY_PLACEMENT_POLICY,
        content_type: LEGACY_LIBRARY_PLACEMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_LIBRARY_PLACEMENT_MAX_BODY_BYTES,
        input: LegacyLibraryPlacementInputShapeV1::PositionalOrganizationAndVideoList,
        observed_authorization:
            LegacyLibraryPlacementObservedAuthorizationV1::OwnerOrAnyMemberAndAnyScopedShare,
        required_authorization:
            LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndOnlyMatchingOrganizationShares,
        observed_mutation:
            LegacyLibraryPlacementObservedMutationV1::DeleteSharesThenClearFoldersWithoutTransaction,
        required_mutation:
            LegacyLibraryPlacementRequiredMutationV1::DeleteOrganizationSharesAndClearOnlyDirectFoldersInOrganization,
        observed_success:
            LegacyLibraryPlacementObservedSuccessV1::OrganizationRemovedOrNoMatchingMessage,
        observed_failure: LegacyLibraryPlacementObservedFailureV1::CaughtObjectWithError,
        observed_retry:
            LegacyLibraryPlacementObservedRetryV1::NonAtomicReplayChangesToNoMatching,
        required_retry: LEGACY_LIBRARY_PLACEMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
        production_promoted: false,
    };

pub const LEGACY_ADD_VIDEOS_TO_SPACE_PROFILE: LegacyLibraryPlacementProfileV1 =
    LegacyLibraryPlacementProfileV1 {
        operation_id: LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_ADD_VIDEOS_TO_SPACE_IDENTITY,
        pinned_commit: LEGACY_LIBRARY_PLACEMENT_CAP_COMMIT,
        sources: LEGACY_ADD_VIDEOS_TO_SPACE_SOURCES,
        authentication: "session",
        policy: LEGACY_LIBRARY_PLACEMENT_POLICY,
        content_type: LEGACY_LIBRARY_PLACEMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_LIBRARY_PLACEMENT_MAX_BODY_BYTES,
        input: LegacyLibraryPlacementInputShapeV1::PositionalSpaceOrOrganizationAndVideoList,
        observed_authorization:
            LegacyLibraryPlacementObservedAuthorizationV1::ActiveOrganizationBranchOrSpaceManagerAndActorOwnedVideoFilter,
        required_authorization:
            LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndEveryActorOwnedVideo,
        observed_mutation:
            LegacyLibraryPlacementObservedMutationV1::MoveExistingToRootThenInsertWithoutTransaction,
        required_mutation:
            LegacyLibraryPlacementRequiredMutationV1::UpsertSelectedScopeMembershipAndClearOnlyItsScopedFolder,
        observed_success: LegacyLibraryPlacementObservedSuccessV1::ScopeAddedMessage,
        observed_failure: LegacyLibraryPlacementObservedFailureV1::CaughtObjectWithError,
        observed_retry: LegacyLibraryPlacementObservedRetryV1::ConvergentButNonAtomic,
        required_retry: LEGACY_LIBRARY_PLACEMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
        production_promoted: false,
    };

pub const LEGACY_REMOVE_VIDEOS_FROM_SPACE_PROFILE: LegacyLibraryPlacementProfileV1 =
    LegacyLibraryPlacementProfileV1 {
        operation_id: LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_REMOVE_VIDEOS_FROM_SPACE_IDENTITY,
        pinned_commit: LEGACY_LIBRARY_PLACEMENT_CAP_COMMIT,
        sources: LEGACY_REMOVE_VIDEOS_FROM_SPACE_SOURCES,
        authentication: "session",
        policy: LEGACY_LIBRARY_PLACEMENT_POLICY,
        content_type: LEGACY_LIBRARY_PLACEMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_LIBRARY_PLACEMENT_MAX_BODY_BYTES,
        input: LegacyLibraryPlacementInputShapeV1::PositionalSpaceOrOrganizationAndVideoList,
        observed_authorization:
            LegacyLibraryPlacementObservedAuthorizationV1::ActiveOrganizationBranchOrSpaceManagerAndActorOwnedVideoFilter,
        required_authorization:
            LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndEveryActorOwnedVideo,
        observed_mutation: LegacyLibraryPlacementObservedMutationV1::BranchScopedDelete,
        required_mutation:
            LegacyLibraryPlacementRequiredMutationV1::DeleteOnlySelectedScopeMembership,
        observed_success:
            LegacyLibraryPlacementObservedSuccessV1::ScopeRemovedMessageAndDeletedCount,
        observed_failure: LegacyLibraryPlacementObservedFailureV1::CaughtObjectWithMessage,
        observed_retry:
            LegacyLibraryPlacementObservedRetryV1::RepeatedDeleteReportsValidatedInputCount,
        required_retry: LEGACY_LIBRARY_PLACEMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
        production_promoted: false,
    };

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyLibraryPlacementInputV1 {
    AddToOrganization {
        legacy_organization_id: String,
        legacy_video_ids: Vec<String>,
    },
    RemoveFromOrganization {
        legacy_organization_id: String,
        legacy_video_ids: Vec<String>,
    },
    AddToSpace {
        legacy_scope_id: String,
        legacy_video_ids: Vec<String>,
    },
    RemoveFromSpace {
        legacy_scope_id: String,
        legacy_video_ids: Vec<String>,
    },
}

impl LegacyLibraryPlacementInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyLibraryPlacementActionV1 {
        match self {
            Self::AddToOrganization { .. } => LegacyLibraryPlacementActionV1::AddToOrganization,
            Self::RemoveFromOrganization { .. } => {
                LegacyLibraryPlacementActionV1::RemoveFromOrganization
            }
            Self::AddToSpace { .. } => LegacyLibraryPlacementActionV1::AddToSpace,
            Self::RemoveFromSpace { .. } => LegacyLibraryPlacementActionV1::RemoveFromSpace,
        }
    }

    fn video_ids(&self) -> &[String] {
        match self {
            Self::AddToOrganization {
                legacy_video_ids, ..
            }
            | Self::RemoveFromOrganization {
                legacy_video_ids, ..
            }
            | Self::AddToSpace {
                legacy_video_ids, ..
            }
            | Self::RemoveFromSpace {
                legacy_video_ids, ..
            } => legacy_video_ids,
        }
    }
}

impl fmt::Debug for LegacyLibraryPlacementInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::AddToOrganization { .. } => "AddToOrganization",
                Self::RemoveFromOrganization { .. } => "RemoveFromOrganization",
                Self::AddToSpace { .. } => "AddToSpace",
                Self::RemoveFromSpace { .. } => "RemoveFromSpace",
            })
            .field("scope", &"<redacted>")
            .field("targets", &"<redacted>")
            .field("video_count", &self.video_ids().len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyLibraryPlacementRequestV1 {
    pub credential: Option<LegacyFolderAssignmentCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyLibraryPlacementInputV1,
}

impl fmt::Debug for LegacyLibraryPlacementRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyLibraryPlacementRequestV1")
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

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyLibraryPlacementAuthorityV1 {
    actor_id: UserId,
    active_organization_id: OrganizationId,
}

impl LegacyLibraryPlacementAuthorityV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> OrganizationId {
        self.active_organization_id
    }
}

impl fmt::Debug for LegacyLibraryPlacementAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyLibraryPlacementAuthorityV1([redacted])")
    }
}

/// Unforgeable browser boundary material carried into the atomic placement port.
///
/// Production callers can construct this only from the authentication service's
/// validated, one-use browser mutation proof. The D1 implementation must assert
/// and consume that proof in the same batch as the placement and journal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryPlacementBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyLibraryPlacementBrowserFenceV1 {
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

impl fmt::Debug for LegacyLibraryPlacementBrowserFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyLibraryPlacementBrowserFenceV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyLibraryPlacementFenceV1 {
    authority: LegacyLibraryPlacementAuthorityV1,
    idempotency_key: IdempotencyKey,
    request_fingerprint: [u8; 32],
}

impl LegacyLibraryPlacementFenceV1 {
    #[must_use]
    pub const fn authority(&self) -> &LegacyLibraryPlacementAuthorityV1 {
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

impl fmt::Debug for LegacyLibraryPlacementFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyLibraryPlacementFenceV1")
            .field("authority", &self.authority)
            .field("idempotency_key", &"<redacted>")
            .field("request_fingerprint", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyLibraryPlacementCommandV1 {
    AddToOrganization {
        fence: LegacyLibraryPlacementFenceV1,
        organization_id: OrganizationId,
        video_ids: Vec<VideoId>,
    },
    RemoveFromOrganization {
        fence: LegacyLibraryPlacementFenceV1,
        organization_id: OrganizationId,
        video_ids: Vec<VideoId>,
    },
    AddToSpace {
        fence: LegacyLibraryPlacementFenceV1,
        scope: LegacyFolderAssignmentScopeV1,
        video_ids: Vec<VideoId>,
    },
    RemoveFromSpace {
        fence: LegacyLibraryPlacementFenceV1,
        scope: LegacyFolderAssignmentScopeV1,
        video_ids: Vec<VideoId>,
    },
}

impl LegacyLibraryPlacementCommandV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyLibraryPlacementActionV1 {
        match self {
            Self::AddToOrganization { .. } => LegacyLibraryPlacementActionV1::AddToOrganization,
            Self::RemoveFromOrganization { .. } => {
                LegacyLibraryPlacementActionV1::RemoveFromOrganization
            }
            Self::AddToSpace { .. } => LegacyLibraryPlacementActionV1::AddToSpace,
            Self::RemoveFromSpace { .. } => LegacyLibraryPlacementActionV1::RemoveFromSpace,
        }
    }

    #[must_use]
    pub const fn fence(&self) -> &LegacyLibraryPlacementFenceV1 {
        match self {
            Self::AddToOrganization { fence, .. }
            | Self::RemoveFromOrganization { fence, .. }
            | Self::AddToSpace { fence, .. }
            | Self::RemoveFromSpace { fence, .. } => fence,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> LegacyFolderAssignmentScopeV1 {
        match self {
            Self::AddToOrganization {
                organization_id, ..
            }
            | Self::RemoveFromOrganization {
                organization_id, ..
            } => LegacyFolderAssignmentScopeV1::OrganizationLibrary {
                organization_id: *organization_id,
            },
            Self::AddToSpace { scope, .. } | Self::RemoveFromSpace { scope, .. } => *scope,
        }
    }

    #[must_use]
    pub fn video_ids(&self) -> &[VideoId] {
        match self {
            Self::AddToOrganization { video_ids, .. }
            | Self::RemoveFromOrganization { video_ids, .. }
            | Self::AddToSpace { video_ids, .. }
            | Self::RemoveFromSpace { video_ids, .. } => video_ids,
        }
    }

    #[must_use]
    pub fn requested_video_count(&self) -> usize {
        self.video_ids().len()
    }

    #[must_use]
    pub const fn required_authorization(&self) -> LegacyLibraryPlacementRequiredAuthorizationV1 {
        match self {
            Self::RemoveFromOrganization { .. } => {
                LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndOnlyMatchingOrganizationShares
            }
            Self::AddToOrganization { .. }
            | Self::AddToSpace { .. }
            | Self::RemoveFromSpace { .. } => {
                LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndEveryActorOwnedVideo
            }
        }
    }

    #[must_use]
    pub const fn required_mutation(&self) -> LegacyLibraryPlacementRequiredMutationV1 {
        match self {
            Self::AddToOrganization { .. } => {
                LegacyLibraryPlacementRequiredMutationV1::UpsertOrganizationShareAndClearOnlyItsScopedFolder
            }
            Self::RemoveFromOrganization { .. } => {
                LegacyLibraryPlacementRequiredMutationV1::DeleteOrganizationSharesAndClearOnlyDirectFoldersInOrganization
            }
            Self::AddToSpace { .. } => {
                LegacyLibraryPlacementRequiredMutationV1::UpsertSelectedScopeMembershipAndClearOnlyItsScopedFolder
            }
            Self::RemoveFromSpace { .. } => {
                LegacyLibraryPlacementRequiredMutationV1::DeleteOnlySelectedScopeMembership
            }
        }
    }
}

impl fmt::Debug for LegacyLibraryPlacementCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::AddToOrganization { .. } => "AddToOrganization",
                Self::RemoveFromOrganization { .. } => "RemoveFromOrganization",
                Self::AddToSpace { .. } => "AddToSpace",
                Self::RemoveFromSpace { .. } => "RemoveFromSpace",
            })
            .field("fence", self.fence())
            .field("scope", &self.scope())
            .field("targets", &"<redacted>")
            .field("video_count", &self.requested_video_count())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementMutationResultV1 {
    OrganizationAdded { total_updated: u16 },
    OrganizationRemoved { existing_shared: u16 },
    ScopeAdded { valid_video_count: u16 },
    ScopeRemoved { valid_video_count: u16 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryPlacementEffectsV1 {
    scope: LegacyFolderAssignmentScopeV1,
    invalidates_scope_root: bool,
    invalidates_caps: bool,
}

impl LegacyLibraryPlacementEffectsV1 {
    fn for_result(
        command: &LegacyLibraryPlacementCommandV1,
        result: LegacyLibraryPlacementMutationResultV1,
    ) -> Self {
        // Cap returns its organization no-match success before either
        // revalidatePath call. Persist the absence of effects in the receipt
        // so a replay cannot invent cache invalidation that the first result
        // did not perform.
        let organization_no_match = matches!(
            (command, result),
            (
                LegacyLibraryPlacementCommandV1::RemoveFromOrganization { .. },
                LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared: 0 }
            )
        );
        Self {
            scope: command.scope(),
            invalidates_scope_root: !organization_no_match,
            invalidates_caps: !organization_no_match
                && !matches!(
                    command,
                    LegacyLibraryPlacementCommandV1::RemoveFromSpace { .. }
                ),
        }
    }

    #[must_use]
    pub const fn scope(&self) -> LegacyFolderAssignmentScopeV1 {
        self.scope
    }

    #[must_use]
    pub const fn invalidates_scope_root(&self) -> bool {
        self.invalidates_scope_root
    }

    #[must_use]
    pub const fn invalidates_caps(&self) -> bool {
        self.invalidates_caps
    }
}

impl fmt::Debug for LegacyLibraryPlacementEffectsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyLibraryPlacementEffectsV1")
            .field("scope", &self.scope)
            .field("invalidates_scope_root", &self.invalidates_scope_root)
            .field("invalidates_caps", &self.invalidates_caps)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyLibraryPlacementMutationReceiptV1 {
    result: LegacyLibraryPlacementMutationResultV1,
    effects: LegacyLibraryPlacementEffectsV1,
}

impl LegacyLibraryPlacementMutationReceiptV1 {
    pub fn new(
        command: &LegacyLibraryPlacementCommandV1,
        result: LegacyLibraryPlacementMutationResultV1,
    ) -> Result<Self, LegacyLibraryPlacementAtomicErrorV1> {
        let requested = command.requested_video_count();
        let valid = match (command, result) {
            (
                LegacyLibraryPlacementCommandV1::AddToOrganization { .. },
                LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated },
            ) => usize::from(total_updated) == requested,
            (
                LegacyLibraryPlacementCommandV1::RemoveFromOrganization { .. },
                LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared },
            ) => usize::from(existing_shared) <= requested,
            (
                LegacyLibraryPlacementCommandV1::AddToSpace { .. },
                LegacyLibraryPlacementMutationResultV1::ScopeAdded { valid_video_count },
            )
            | (
                LegacyLibraryPlacementCommandV1::RemoveFromSpace { .. },
                LegacyLibraryPlacementMutationResultV1::ScopeRemoved { valid_video_count },
            ) => usize::from(valid_video_count) == requested,
            _ => false,
        };
        if !valid {
            return Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            result,
            effects: LegacyLibraryPlacementEffectsV1::for_result(command, result),
        })
    }

    #[must_use]
    pub const fn result(&self) -> LegacyLibraryPlacementMutationResultV1 {
        self.result
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyLibraryPlacementEffectsV1 {
        &self.effects
    }
}

impl fmt::Debug for LegacyLibraryPlacementMutationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyLibraryPlacementMutationReceiptV1")
            .field("result", &self.result)
            .field("effects", &self.effects)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyLibraryPlacementAtomicOutcomeV1 {
    Applied(LegacyLibraryPlacementMutationReceiptV1),
    Replay(LegacyLibraryPlacementMutationReceiptV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryPlacementSuccessV1 {
    OrganizationAdded {
        total_updated: u16,
    },
    OrganizationRemoved {
        removed_count: u16,
    },
    OrganizationNoMatching,
    ScopeAdded {
        valid_video_count: u16,
        scope: LegacyFolderAssignmentScopeV1,
    },
    ScopeRemoved {
        valid_video_count: u16,
        scope: LegacyFolderAssignmentScopeV1,
    },
}

impl LegacyLibraryPlacementSuccessV1 {
    #[must_use]
    pub const fn object_success(&self) -> bool {
        true
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::OrganizationAdded { total_updated } => format!(
                "{total_updated} video{} {} now in organization root",
                if *total_updated == 1 { "" } else { "s" },
                if *total_updated == 1 { "is" } else { "are" }
            ),
            Self::OrganizationRemoved { removed_count } => format!(
                "{removed_count} video{} removed from organization",
                if *removed_count == 1 { "" } else { "s" }
            ),
            Self::OrganizationNoMatching => {
                "No matching shared videos found in organization".into()
            }
            Self::ScopeAdded {
                valid_video_count,
                scope,
            } => format!(
                "{valid_video_count} video{} added to {}",
                if *valid_video_count == 1 { "" } else { "s" },
                scope_label(*scope)
            ),
            Self::ScopeRemoved {
                valid_video_count,
                scope,
            } => format!(
                "Removed {valid_video_count} video(s) from {} and folders",
                scope_label(*scope)
            ),
        }
    }

    #[must_use]
    pub const fn deleted_count(&self) -> Option<u16> {
        match self {
            Self::ScopeRemoved {
                valid_video_count, ..
            } => Some(*valid_video_count),
            Self::OrganizationAdded { .. }
            | Self::OrganizationRemoved { .. }
            | Self::OrganizationNoMatching
            | Self::ScopeAdded { .. } => None,
        }
    }
}

fn scope_label(scope: LegacyFolderAssignmentScopeV1) -> &'static str {
    match scope {
        LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. } => "organization",
        LegacyFolderAssignmentScopeV1::Space { .. } => "space",
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyLibraryPlacementExecutionV1 {
    success: LegacyLibraryPlacementSuccessV1,
    effects: LegacyLibraryPlacementEffectsV1,
    replayed: bool,
}

impl LegacyLibraryPlacementExecutionV1 {
    #[must_use]
    pub const fn success(&self) -> &LegacyLibraryPlacementSuccessV1 {
        &self.success
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyLibraryPlacementEffectsV1 {
        &self.effects
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

impl fmt::Debug for LegacyLibraryPlacementExecutionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyLibraryPlacementExecutionV1")
            .field("success", &self.success)
            .field("effects", &self.effects)
            .field("replayed", &self.replayed)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyLibraryPlacementAtomicErrorV1 {
    #[error("library placement target was not found")]
    TargetMissing,
    #[error("library placement target was not found")]
    AccessDenied,
    #[error("library placement target was not found")]
    CrossTenant,
    #[error("library placement target was not found")]
    StaleAuthority,
    #[error("library placement conflicts with a prior request")]
    Conflict,
    #[error("library placement is already in flight")]
    InFlight,
    #[error("library placement authority is unavailable")]
    Unavailable,
    #[error("library placement authority returned invalid state")]
    Corrupt,
}

/// Provider-free transaction boundary for the four placement actions.
///
/// An implementation must perform one atomic transaction which:
///
/// 1. revalidates the session actor and active organization in the fence;
/// 2. proves the organization or space belongs to that active tenant and the
///    actor has manager authority for the selected library;
/// 3. for add-organization, add-space, and remove-space, proves every canonical
///    video exists and `videos.owner_id == actor_id`, failing the entire request
///    without disclosing which target failed; remove-organization deliberately
///    does not require actor ownership and may consider only requested
///    `shared_videos` rows in the exact active organization, with unmatched IDs
///    contributing to the source-compatible no-match/partial count;
/// 4. binds the actor/tenant-scoped idempotency key to the canonical request
///    fingerprint;
/// 5. applies exactly the command's typed storage postcondition: organization
///    add creates-or-roots only the active organization's `shared_videos` rows;
///    real-space add creates-or-roots only that space's `space_videos` rows;
///    organization removal deletes only matching active-organization shares and
///    clears a direct `videos.folder_id` only when that folder belongs to the
///    active organization; space removal deletes only the selected
///    `shared_videos` or `space_videos` membership and does not clear unrelated
///    direct or other-scope folders; and
/// 6. journals the exact result and typed invalidation effects.
///
/// The same key and fingerprint returns `Replay`; different semantics under the
/// key return `Conflict`. Mutation, browser-proof consumption, audit, and journal
/// commits must not be split. This source-only contract does not authorize a
/// production allowlist entry.
#[async_trait]
pub trait LegacyLibraryPlacementAtomicPortV1: Send + Sync {
    async fn execute_atomic(
        &self,
        command: &LegacyLibraryPlacementCommandV1,
        browser_fence: &LegacyLibraryPlacementBrowserFenceV1,
    ) -> Result<LegacyLibraryPlacementAtomicOutcomeV1, LegacyLibraryPlacementAtomicErrorV1>;
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyLibraryPlacementErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid library placement request")]
    Invalid,
    #[error("An idempotency key is required")]
    IdempotencyRequired,
    #[error("Library placement target not found")]
    TargetNotFound,
    #[error("Library placement request conflicts with a prior request")]
    Conflict,
    #[error("Library placement authority is unavailable")]
    AuthorityUnavailable,
    #[error("Library placement failed")]
    Internal,
}

impl fmt::Debug for LegacyLibraryPlacementErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "Unauthorized",
            Self::Invalid => "Invalid",
            Self::IdempotencyRequired => "IdempotencyRequired",
            Self::TargetNotFound => "TargetNotFound",
            Self::Conflict => "Conflict",
            Self::AuthorityUnavailable => "AuthorityUnavailable",
            Self::Internal => "Internal",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryPlacementAdapterV1 {
    action: LegacyLibraryPlacementActionV1,
}

impl LegacyLibraryPlacementAdapterV1 {
    #[must_use]
    pub const fn add_videos_to_organization() -> Self {
        Self {
            action: LegacyLibraryPlacementActionV1::AddToOrganization,
        }
    }

    #[must_use]
    pub const fn remove_videos_from_organization() -> Self {
        Self {
            action: LegacyLibraryPlacementActionV1::RemoveFromOrganization,
        }
    }

    #[must_use]
    pub const fn add_videos_to_space() -> Self {
        Self {
            action: LegacyLibraryPlacementActionV1::AddToSpace,
        }
    }

    #[must_use]
    pub const fn remove_videos_from_space() -> Self {
        Self {
            action: LegacyLibraryPlacementActionV1::RemoveFromSpace,
        }
    }

    #[must_use]
    pub const fn profile(self) -> &'static LegacyLibraryPlacementProfileV1 {
        match self.action {
            LegacyLibraryPlacementActionV1::AddToOrganization => {
                &LEGACY_ADD_VIDEOS_TO_ORGANIZATION_PROFILE
            }
            LegacyLibraryPlacementActionV1::RemoveFromOrganization => {
                &LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_PROFILE
            }
            LegacyLibraryPlacementActionV1::AddToSpace => &LEGACY_ADD_VIDEOS_TO_SPACE_PROFILE,
            LegacyLibraryPlacementActionV1::RemoveFromSpace => {
                &LEGACY_REMOVE_VIDEOS_FROM_SPACE_PROFILE
            }
        }
    }

    pub fn prepare(
        self,
        request: &LegacyLibraryPlacementRequestV1,
    ) -> Result<LegacyLibraryPlacementCommandV1, LegacyLibraryPlacementErrorV1> {
        if request.input.action() != self.action {
            return Err(LegacyLibraryPlacementErrorV1::Invalid);
        }
        if request.credential != Some(LegacyFolderAssignmentCredentialV1::Session) {
            return Err(LegacyLibraryPlacementErrorV1::Unauthorized);
        }
        let Some(actor_id) = request.actor_id else {
            return Err(LegacyLibraryPlacementErrorV1::Unauthorized);
        };
        let Some(active_organization_id) = request.active_organization_id else {
            return Err(LegacyLibraryPlacementErrorV1::Unauthorized);
        };
        let idempotency_key = request
            .idempotency_key
            .as_ref()
            .ok_or(LegacyLibraryPlacementErrorV1::IdempotencyRequired)
            .and_then(|key| {
                IdempotencyKey::parse(key.clone())
                    .map_err(|_| LegacyLibraryPlacementErrorV1::Invalid)
            })?;
        let authority = LegacyLibraryPlacementAuthorityV1 {
            actor_id,
            active_organization_id,
        };
        let video_ids = map_video_list(request.input.video_ids())?;

        let (scope, command) = match &request.input {
            LegacyLibraryPlacementInputV1::AddToOrganization {
                legacy_organization_id,
                ..
            } => {
                let organization_id = map_organization_id(legacy_organization_id)?;
                require_active_organization(organization_id, active_organization_id)?;
                let scope = LegacyFolderAssignmentScopeV1::OrganizationLibrary { organization_id };
                (scope, 0_u8)
            }
            LegacyLibraryPlacementInputV1::RemoveFromOrganization {
                legacy_organization_id,
                ..
            } => {
                let organization_id = map_organization_id(legacy_organization_id)?;
                require_active_organization(organization_id, active_organization_id)?;
                let scope = LegacyFolderAssignmentScopeV1::OrganizationLibrary { organization_id };
                (scope, 1_u8)
            }
            LegacyLibraryPlacementInputV1::AddToSpace {
                legacy_scope_id, ..
            } => (map_scope(legacy_scope_id, active_organization_id)?, 2_u8),
            LegacyLibraryPlacementInputV1::RemoveFromSpace {
                legacy_scope_id, ..
            } => (map_scope(legacy_scope_id, active_organization_id)?, 3_u8),
        };
        let fingerprint = fingerprint(command, &authority, scope, &video_ids);
        let fence = LegacyLibraryPlacementFenceV1 {
            authority,
            idempotency_key,
            request_fingerprint: fingerprint,
        };
        Ok(match &request.input {
            LegacyLibraryPlacementInputV1::AddToOrganization { .. } => {
                let LegacyFolderAssignmentScopeV1::OrganizationLibrary { organization_id } = scope
                else {
                    return Err(LegacyLibraryPlacementErrorV1::Internal);
                };
                LegacyLibraryPlacementCommandV1::AddToOrganization {
                    fence,
                    organization_id,
                    video_ids,
                }
            }
            LegacyLibraryPlacementInputV1::RemoveFromOrganization { .. } => {
                let LegacyFolderAssignmentScopeV1::OrganizationLibrary { organization_id } = scope
                else {
                    return Err(LegacyLibraryPlacementErrorV1::Internal);
                };
                LegacyLibraryPlacementCommandV1::RemoveFromOrganization {
                    fence,
                    organization_id,
                    video_ids,
                }
            }
            LegacyLibraryPlacementInputV1::AddToSpace { .. } => {
                LegacyLibraryPlacementCommandV1::AddToSpace {
                    fence,
                    scope,
                    video_ids,
                }
            }
            LegacyLibraryPlacementInputV1::RemoveFromSpace { .. } => {
                LegacyLibraryPlacementCommandV1::RemoveFromSpace {
                    fence,
                    scope,
                    video_ids,
                }
            }
        })
    }

    pub async fn execute<Port>(
        self,
        port: &Port,
        request: &LegacyLibraryPlacementRequestV1,
        proof: &ValidatedBrowserMutationProof,
    ) -> Result<LegacyLibraryPlacementExecutionV1, LegacyLibraryPlacementErrorV1>
    where
        Port: LegacyLibraryPlacementAtomicPortV1,
    {
        let browser_fence = LegacyLibraryPlacementBrowserFenceV1::from_validated_proof(proof);
        if request.actor_id != Some(browser_fence.actor_id()) {
            return Err(LegacyLibraryPlacementErrorV1::Unauthorized);
        }
        self.execute_fenced(port, request, &browser_fence).await
    }

    async fn execute_fenced<Port>(
        self,
        port: &Port,
        request: &LegacyLibraryPlacementRequestV1,
        browser_fence: &LegacyLibraryPlacementBrowserFenceV1,
    ) -> Result<LegacyLibraryPlacementExecutionV1, LegacyLibraryPlacementErrorV1>
    where
        Port: LegacyLibraryPlacementAtomicPortV1,
    {
        let command = self.prepare(request)?;
        if command.fence().authority().actor_id() != browser_fence.actor_id() {
            return Err(LegacyLibraryPlacementErrorV1::Unauthorized);
        }
        let (receipt, replayed) = match port
            .execute_atomic(&command, browser_fence)
            .await
            .map_err(map_atomic_error)?
        {
            LegacyLibraryPlacementAtomicOutcomeV1::Applied(receipt) => (receipt, false),
            LegacyLibraryPlacementAtomicOutcomeV1::Replay(receipt) => (receipt, true),
        };
        let success = project_success(&command, receipt.result())?;
        Ok(LegacyLibraryPlacementExecutionV1 {
            success,
            effects: receipt.effects,
            replayed,
        })
    }
}

fn project_success(
    command: &LegacyLibraryPlacementCommandV1,
    result: LegacyLibraryPlacementMutationResultV1,
) -> Result<LegacyLibraryPlacementSuccessV1, LegacyLibraryPlacementErrorV1> {
    match (command, result) {
        (
            LegacyLibraryPlacementCommandV1::AddToOrganization { .. },
            LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated },
        ) => Ok(LegacyLibraryPlacementSuccessV1::OrganizationAdded { total_updated }),
        (
            LegacyLibraryPlacementCommandV1::RemoveFromOrganization { .. },
            LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared: 0 },
        ) => Ok(LegacyLibraryPlacementSuccessV1::OrganizationNoMatching),
        (
            LegacyLibraryPlacementCommandV1::RemoveFromOrganization { .. },
            LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared },
        ) => Ok(LegacyLibraryPlacementSuccessV1::OrganizationRemoved {
            removed_count: existing_shared,
        }),
        (
            LegacyLibraryPlacementCommandV1::AddToSpace { scope, .. },
            LegacyLibraryPlacementMutationResultV1::ScopeAdded { valid_video_count },
        ) => Ok(LegacyLibraryPlacementSuccessV1::ScopeAdded {
            valid_video_count,
            scope: *scope,
        }),
        (
            LegacyLibraryPlacementCommandV1::RemoveFromSpace { scope, .. },
            LegacyLibraryPlacementMutationResultV1::ScopeRemoved { valid_video_count },
        ) => Ok(LegacyLibraryPlacementSuccessV1::ScopeRemoved {
            valid_video_count,
            scope: *scope,
        }),
        _ => Err(LegacyLibraryPlacementErrorV1::Internal),
    }
}

fn map_atomic_error(error: LegacyLibraryPlacementAtomicErrorV1) -> LegacyLibraryPlacementErrorV1 {
    match error {
        LegacyLibraryPlacementAtomicErrorV1::TargetMissing
        | LegacyLibraryPlacementAtomicErrorV1::AccessDenied
        | LegacyLibraryPlacementAtomicErrorV1::CrossTenant
        | LegacyLibraryPlacementAtomicErrorV1::StaleAuthority => {
            LegacyLibraryPlacementErrorV1::TargetNotFound
        }
        LegacyLibraryPlacementAtomicErrorV1::Conflict
        | LegacyLibraryPlacementAtomicErrorV1::InFlight => LegacyLibraryPlacementErrorV1::Conflict,
        LegacyLibraryPlacementAtomicErrorV1::Unavailable => {
            LegacyLibraryPlacementErrorV1::AuthorityUnavailable
        }
        LegacyLibraryPlacementAtomicErrorV1::Corrupt => LegacyLibraryPlacementErrorV1::Internal,
    }
}

fn cap_uuid(value: &str) -> Result<String, LegacyLibraryPlacementErrorV1> {
    LegacyCapNanoId::parse(value.to_owned())
        .map(|legacy_id| legacy_id.mapped_uuid().to_string())
        .map_err(|_| LegacyLibraryPlacementErrorV1::TargetNotFound)
}

fn map_organization_id(value: &str) -> Result<OrganizationId, LegacyLibraryPlacementErrorV1> {
    OrganizationId::parse(&cap_uuid(value)?).map_err(|_| LegacyLibraryPlacementErrorV1::Internal)
}

fn map_space_id(value: &str) -> Result<SpaceId, LegacyLibraryPlacementErrorV1> {
    SpaceId::parse(&cap_uuid(value)?).map_err(|_| LegacyLibraryPlacementErrorV1::Internal)
}

fn map_video_id(value: &str) -> Result<VideoId, LegacyLibraryPlacementErrorV1> {
    cap_uuid(value)?
        .parse::<VideoId>()
        .map_err(|_| LegacyLibraryPlacementErrorV1::Internal)
}

fn require_active_organization(
    organization_id: OrganizationId,
    active_organization_id: OrganizationId,
) -> Result<(), LegacyLibraryPlacementErrorV1> {
    if organization_id == active_organization_id {
        Ok(())
    } else {
        Err(LegacyLibraryPlacementErrorV1::TargetNotFound)
    }
}

fn map_scope(
    value: &str,
    active_organization_id: OrganizationId,
) -> Result<LegacyFolderAssignmentScopeV1, LegacyLibraryPlacementErrorV1> {
    let mapped = cap_uuid(value)?;
    if mapped == active_organization_id.to_string() {
        Ok(LegacyFolderAssignmentScopeV1::OrganizationLibrary {
            organization_id: active_organization_id,
        })
    } else {
        Ok(LegacyFolderAssignmentScopeV1::Space {
            space_id: map_space_id(value)?,
        })
    }
}

fn map_video_list(values: &[String]) -> Result<Vec<VideoId>, LegacyLibraryPlacementErrorV1> {
    if values.is_empty() || values.len() > MAX_LEGACY_FOLDER_ASSIGNMENT_VIDEO_IDS {
        return Err(LegacyLibraryPlacementErrorV1::Invalid);
    }
    let mut mapped = Vec::with_capacity(values.len());
    for value in values {
        let video_id = map_video_id(value)?;
        if !mapped.contains(&video_id) {
            mapped.push(video_id);
        }
    }
    mapped.sort_unstable_by_key(ToString::to_string);
    Ok(mapped)
}

fn hash_value(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn hash_scope(hasher: &mut Sha256, scope: LegacyFolderAssignmentScopeV1) {
    match scope {
        LegacyFolderAssignmentScopeV1::OrganizationLibrary { organization_id } => {
            hasher.update([0]);
            hash_value(hasher, &organization_id.to_string());
        }
        LegacyFolderAssignmentScopeV1::Space { space_id } => {
            hasher.update([1]);
            hash_value(hasher, &space_id.to_string());
        }
    }
}

fn fingerprint(
    action_tag: u8,
    authority: &LegacyLibraryPlacementAuthorityV1,
    scope: LegacyFolderAssignmentScopeV1,
    video_ids: &[VideoId],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"frame-legacy-library-placement-v1\0");
    hasher.update([action_tag]);
    hash_value(&mut hasher, &authority.actor_id.to_string());
    hash_value(&mut hasher, &authority.active_organization_id.to_string());
    hash_scope(&mut hasher, scope);
    hasher.update((video_ids.len() as u64).to_be_bytes());
    for video_id in video_ids {
        hash_value(&mut hasher, &video_id.to_string());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const ORGANIZATION: &str = "0123456789abcde";
    const OTHER_ORGANIZATION: &str = "1123456789abcde";
    const SPACE: &str = "2123456789abcde";
    const VIDEO_ONE: &str = "3123456789abcde";
    const VIDEO_TWO: &str = "4123456789abcde";

    fn actor() -> UserId {
        UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a8").expect("actor")
    }

    fn other_actor() -> UserId {
        UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a9").expect("actor")
    }

    fn mapped_organization(value: &str) -> OrganizationId {
        let mapped = LegacyCapNanoId::parse(value)
            .expect("legacy ID")
            .mapped_uuid()
            .to_string();
        OrganizationId::parse(&mapped).expect("organization")
    }

    fn request(input: LegacyLibraryPlacementInputV1) -> LegacyLibraryPlacementRequestV1 {
        LegacyLibraryPlacementRequestV1 {
            credential: Some(LegacyFolderAssignmentCredentialV1::Session),
            actor_id: Some(actor()),
            active_organization_id: Some(mapped_organization(ORGANIZATION)),
            idempotency_key: Some("library-placement-0001".into()),
            input,
        }
    }

    fn organization_add(videos: Vec<&str>) -> LegacyLibraryPlacementRequestV1 {
        request(LegacyLibraryPlacementInputV1::AddToOrganization {
            legacy_organization_id: ORGANIZATION.into(),
            legacy_video_ids: videos.into_iter().map(String::from).collect(),
        })
    }

    fn space_add(scope: &str, videos: Vec<&str>) -> LegacyLibraryPlacementRequestV1 {
        request(LegacyLibraryPlacementInputV1::AddToSpace {
            legacy_scope_id: scope.into(),
            legacy_video_ids: videos.into_iter().map(String::from).collect(),
        })
    }

    fn space_remove(scope: &str, videos: Vec<&str>) -> LegacyLibraryPlacementRequestV1 {
        request(LegacyLibraryPlacementInputV1::RemoveFromSpace {
            legacy_scope_id: scope.into(),
            legacy_video_ids: videos.into_iter().map(String::from).collect(),
        })
    }

    #[test]
    fn profiles_freeze_four_exact_actions_without_claiming_promotion() {
        let cases = [
            (
                LegacyLibraryPlacementAdapterV1::add_videos_to_organization(),
                LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID,
                LEGACY_ADD_VIDEOS_TO_ORGANIZATION_IDENTITY,
                "apps/web/actions/organizations/add-videos.ts",
                "127ccc6ab701c04082cf8010281dbf70daee2ec6c54c01b3af1ebec5b56310c9",
                7,
            ),
            (
                LegacyLibraryPlacementAdapterV1::remove_videos_from_organization(),
                LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID,
                LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_IDENTITY,
                "apps/web/actions/organizations/remove-videos.ts",
                "c67c82c0d5d64229046075569d384bfee3766fea3f7a4adbd592bba1a204bfac",
                6,
            ),
            (
                LegacyLibraryPlacementAdapterV1::add_videos_to_space(),
                LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID,
                LEGACY_ADD_VIDEOS_TO_SPACE_IDENTITY,
                "apps/web/actions/spaces/add-videos.ts",
                "b15a27c2e1e522f97dcdfcb1802ffd7c449a34420b311ca2273f8c8f737581fa",
                12,
            ),
            (
                LegacyLibraryPlacementAdapterV1::remove_videos_from_space(),
                LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID,
                LEGACY_REMOVE_VIDEOS_FROM_SPACE_IDENTITY,
                "apps/web/actions/spaces/remove-videos.ts",
                "a88805652fd94c0baba35afe2d8e3b46d5cf9100362ec8370cba8f43e9b611dc",
                11,
            ),
        ];
        for (adapter, operation, identity, direct_path, direct_hash, source_count) in cases {
            let profile = adapter.profile();
            assert_eq!(profile.operation_id, operation);
            assert_eq!(profile.legacy_identity, identity);
            assert_eq!(profile.pinned_commit, LEGACY_LIBRARY_PLACEMENT_CAP_COMMIT);
            assert_eq!(profile.kind, "server_action");
            assert_eq!(profile.method, "ACTION");
            assert_eq!(profile.authentication, "session");
            assert_eq!(profile.policy, "organization_library.v1");
            assert_eq!(profile.content_type, "application/json");
            assert_eq!(profile.max_body_bytes, 262_144);
            assert!(profile.tenant_non_disclosure);
            assert_eq!(profile.protected_gates, ["released_legacy_client_e2e"]);
            assert!(!profile.production_promoted);
            assert_eq!(profile.sources.len(), source_count);
            assert_eq!(profile.sources[0].path, direct_path);
            assert_eq!(profile.sources[0].sha256, direct_hash);
            assert_eq!(
                profile.required_retry,
                LEGACY_LIBRARY_PLACEMENT_REQUIRED_RETRY
            );
        }
    }

    #[test]
    fn every_pin_is_complete_lowercase_sha256_and_closures_are_present() {
        for pins in [
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_SOURCES,
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_SOURCES,
            LEGACY_ADD_VIDEOS_TO_SPACE_SOURCES,
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_SOURCES,
        ] {
            for pin in pins {
                assert!(!pin.path.is_empty());
                assert_eq!(pin.sha256.len(), 64);
                assert!(
                    pin.sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                );
            }
            for required in [
                "packages/database/auth/session.ts",
                "packages/database/auth/auth-options.ts",
                "packages/database/schema.ts",
                "packages/web-domain/src/Organisation.ts",
                "packages/web-domain/src/Video.ts",
            ] {
                assert!(pins.iter().any(|pin| pin.path == required));
            }
        }
        for pins in [
            LEGACY_ADD_VIDEOS_TO_SPACE_SOURCES,
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_SOURCES,
        ] {
            for required in [
                "apps/web/actions/organization/authorization.ts",
                "apps/web/actions/organization/space-authorization.ts",
                "apps/web/lib/permissions/roles.ts",
                "packages/web-domain/src/Space.ts",
                "packages/web-domain/src/User.ts",
            ] {
                assert!(pins.iter().any(|pin| pin.path == required));
            }
        }
    }

    #[test]
    fn observed_defects_and_required_manager_atomicity_are_separate() {
        assert_eq!(
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_PROFILE.observed_authorization,
            LegacyLibraryPlacementObservedAuthorizationV1::OwnerOrAnyMemberAndActorOwnedVideoFilter
        );
        assert_eq!(
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_PROFILE.observed_authorization,
            LegacyLibraryPlacementObservedAuthorizationV1::OwnerOrAnyMemberAndAnyScopedShare
        );
        assert_eq!(
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_PROFILE.required_authorization,
            LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndOnlyMatchingOrganizationShares
        );
        for profile in [
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_PROFILE,
            LEGACY_ADD_VIDEOS_TO_SPACE_PROFILE,
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_PROFILE,
        ] {
            assert_eq!(
                profile.required_authorization,
                LegacyLibraryPlacementRequiredAuthorizationV1::SessionActorActiveTenantManagerAndEveryActorOwnedVideo
            );
        }
        for profile in [
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_PROFILE,
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_PROFILE,
            LEGACY_ADD_VIDEOS_TO_SPACE_PROFILE,
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_PROFILE,
        ] {
            assert_eq!(
                profile.required_retry.atomicity,
                LegacyLibraryPlacementAtomicityV1::AuthorityMutationOutcomeAndJournalInOneTransaction
            );
        }
        assert_eq!(
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_PROFILE.required_mutation,
            LegacyLibraryPlacementRequiredMutationV1::UpsertOrganizationShareAndClearOnlyItsScopedFolder
        );
        assert_eq!(
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_PROFILE.required_mutation,
            LegacyLibraryPlacementRequiredMutationV1::DeleteOrganizationSharesAndClearOnlyDirectFoldersInOrganization
        );
        assert_eq!(
            LEGACY_ADD_VIDEOS_TO_SPACE_PROFILE.required_mutation,
            LegacyLibraryPlacementRequiredMutationV1::UpsertSelectedScopeMembershipAndClearOnlyItsScopedFolder
        );
        assert_eq!(
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_PROFILE.required_mutation,
            LegacyLibraryPlacementRequiredMutationV1::DeleteOnlySelectedScopeMembership
        );
    }

    #[test]
    fn video_lists_are_bounded_deduplicated_and_canonicalized() {
        let adapter = LegacyLibraryPlacementAdapterV1::add_videos_to_organization();
        let first = adapter
            .prepare(&organization_add(vec![VIDEO_TWO, VIDEO_ONE, VIDEO_TWO]))
            .expect("first");
        let second = adapter
            .prepare(&organization_add(vec![VIDEO_ONE, VIDEO_TWO]))
            .expect("second");
        assert_eq!(first.video_ids().len(), 2);
        assert!(first.video_ids()[0].to_string() < first.video_ids()[1].to_string());
        assert_eq!(
            first.fence().request_fingerprint(),
            second.fence().request_fingerprint()
        );

        assert_eq!(
            adapter.prepare(&organization_add(Vec::new())),
            Err(LegacyLibraryPlacementErrorV1::Invalid)
        );
        let too_many = vec![VIDEO_ONE; MAX_LEGACY_FOLDER_ASSIGNMENT_VIDEO_IDS + 1];
        assert_eq!(
            adapter.prepare(&organization_add(too_many)),
            Err(LegacyLibraryPlacementErrorV1::Invalid)
        );
    }

    #[test]
    fn organization_actions_can_only_target_the_trusted_active_tenant() {
        let mut candidate = organization_add(vec![VIDEO_ONE]);
        let LegacyLibraryPlacementInputV1::AddToOrganization {
            legacy_organization_id,
            ..
        } = &mut candidate.input
        else {
            unreachable!()
        };
        *legacy_organization_id = OTHER_ORGANIZATION.into();
        assert_eq!(
            LegacyLibraryPlacementAdapterV1::add_videos_to_organization().prepare(&candidate),
            Err(LegacyLibraryPlacementErrorV1::TargetNotFound)
        );
    }

    #[test]
    fn space_action_branch_is_selected_only_from_the_trusted_active_tenant() {
        let organization_command = LegacyLibraryPlacementAdapterV1::add_videos_to_space()
            .prepare(&space_add(ORGANIZATION, vec![VIDEO_ONE]))
            .expect("organization branch");
        assert_eq!(
            organization_command.scope(),
            LegacyFolderAssignmentScopeV1::OrganizationLibrary {
                organization_id: mapped_organization(ORGANIZATION)
            }
        );

        let space_command = LegacyLibraryPlacementAdapterV1::add_videos_to_space()
            .prepare(&space_add(SPACE, vec![VIDEO_ONE]))
            .expect("space branch");
        assert!(matches!(
            space_command.scope(),
            LegacyFolderAssignmentScopeV1::Space { .. }
        ));
    }

    #[test]
    fn malformed_ids_and_every_authority_denial_are_non_disclosing() {
        let mut malformed = organization_add(vec![VIDEO_ONE]);
        let LegacyLibraryPlacementInputV1::AddToOrganization {
            legacy_organization_id,
            ..
        } = &mut malformed.input
        else {
            unreachable!()
        };
        *legacy_organization_id = "not-a-cap-id".into();
        assert_eq!(
            LegacyLibraryPlacementAdapterV1::add_videos_to_organization().prepare(&malformed),
            Err(LegacyLibraryPlacementErrorV1::TargetNotFound)
        );
        let malformed_video = organization_add(vec!["not-a-cap-id"]);
        assert_eq!(
            LegacyLibraryPlacementAdapterV1::add_videos_to_organization().prepare(&malformed_video),
            Err(LegacyLibraryPlacementErrorV1::TargetNotFound)
        );
        for error in [
            LegacyLibraryPlacementAtomicErrorV1::TargetMissing,
            LegacyLibraryPlacementAtomicErrorV1::AccessDenied,
            LegacyLibraryPlacementAtomicErrorV1::CrossTenant,
            LegacyLibraryPlacementAtomicErrorV1::StaleAuthority,
        ] {
            assert_eq!(
                map_atomic_error(error),
                LegacyLibraryPlacementErrorV1::TargetNotFound
            );
        }
    }

    #[test]
    fn session_active_tenant_and_idempotency_are_mandatory() {
        let adapter = LegacyLibraryPlacementAdapterV1::add_videos_to_organization();
        let mut candidate = organization_add(vec![VIDEO_ONE]);
        candidate.credential = Some(LegacyFolderAssignmentCredentialV1::ApiKey);
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyLibraryPlacementErrorV1::Unauthorized)
        );
        let mut candidate = organization_add(vec![VIDEO_ONE]);
        candidate.active_organization_id = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyLibraryPlacementErrorV1::Unauthorized)
        );
        let mut candidate = organization_add(vec![VIDEO_ONE]);
        candidate.idempotency_key = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyLibraryPlacementErrorV1::IdempotencyRequired)
        );
        let mut candidate = organization_add(vec![VIDEO_ONE]);
        candidate.idempotency_key = Some("short".into());
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyLibraryPlacementErrorV1::Invalid)
        );
    }

    #[test]
    fn fingerprints_bind_action_actor_tenant_scope_and_canonical_targets() {
        let add = LegacyLibraryPlacementAdapterV1::add_videos_to_space()
            .prepare(&space_add(SPACE, vec![VIDEO_TWO, VIDEO_ONE]))
            .expect("add");
        let remove = LegacyLibraryPlacementAdapterV1::remove_videos_from_space()
            .prepare(&space_remove(SPACE, vec![VIDEO_ONE, VIDEO_TWO]))
            .expect("remove");
        assert_ne!(
            add.fence().request_fingerprint(),
            remove.fence().request_fingerprint()
        );

        let mut different_actor = space_add(SPACE, vec![VIDEO_ONE, VIDEO_TWO]);
        different_actor.actor_id = Some(other_actor());
        let different_actor = LegacyLibraryPlacementAdapterV1::add_videos_to_space()
            .prepare(&different_actor)
            .expect("different actor");
        assert_ne!(
            add.fence().request_fingerprint(),
            different_actor.fence().request_fingerprint()
        );

        let root = LegacyLibraryPlacementAdapterV1::add_videos_to_space()
            .prepare(&space_add(ORGANIZATION, vec![VIDEO_ONE, VIDEO_TWO]))
            .expect("root");
        assert_ne!(
            add.fence().request_fingerprint(),
            root.fence().request_fingerprint()
        );
    }

    #[test]
    fn action_mismatch_is_rejected_before_the_port() {
        assert_eq!(
            LegacyLibraryPlacementAdapterV1::remove_videos_from_organization()
                .prepare(&organization_add(vec![VIDEO_ONE])),
            Err(LegacyLibraryPlacementErrorV1::Invalid)
        );
    }

    #[test]
    fn request_command_and_errors_do_not_disclose_targets_or_keys() {
        let request = organization_add(vec![VIDEO_ONE]);
        let request_debug = format!("{request:?}");
        let actor_id = actor().to_string();
        for secret in [
            ORGANIZATION,
            VIDEO_ONE,
            "library-placement-0001",
            actor_id.as_str(),
        ] {
            assert!(!request_debug.contains(secret));
        }
        let command = LegacyLibraryPlacementAdapterV1::add_videos_to_organization()
            .prepare(&request)
            .expect("command");
        let command_debug = format!("{command:?}");
        assert!(!command_debug.contains("library-placement-0001"));
        assert!(!command_debug.contains(&command.video_ids()[0].to_string()));
        assert_eq!(
            format!("{:?}", LegacyLibraryPlacementErrorV1::Internal),
            "Internal"
        );
    }

    #[test]
    fn exact_success_messages_preserve_all_legacy_asymmetries() {
        let one = LegacyLibraryPlacementSuccessV1::OrganizationAdded { total_updated: 1 };
        assert!(one.object_success());
        assert_eq!(one.message(), "1 video is now in organization root");
        let many = LegacyLibraryPlacementSuccessV1::OrganizationAdded { total_updated: 2 };
        assert_eq!(many.message(), "2 videos are now in organization root");

        let none = LegacyLibraryPlacementSuccessV1::OrganizationNoMatching;
        assert_eq!(
            none.message(),
            "No matching shared videos found in organization"
        );
        let removed = LegacyLibraryPlacementSuccessV1::OrganizationRemoved { removed_count: 2 };
        assert_eq!(removed.message(), "2 videos removed from organization");
        assert_eq!(removed.deleted_count(), None);

        let organization_scope = LegacyFolderAssignmentScopeV1::OrganizationLibrary {
            organization_id: mapped_organization(ORGANIZATION),
        };
        let added = LegacyLibraryPlacementSuccessV1::ScopeAdded {
            valid_video_count: 2,
            scope: organization_scope,
        };
        assert_eq!(added.message(), "2 videos added to organization");
        let space_scope = LegacyLibraryPlacementAdapterV1::add_videos_to_space()
            .prepare(&space_add(SPACE, vec![VIDEO_ONE]))
            .expect("space")
            .scope();
        let removed = LegacyLibraryPlacementSuccessV1::ScopeRemoved {
            valid_video_count: 1,
            scope: space_scope,
        };
        assert_eq!(
            removed.message(),
            "Removed 1 video(s) from space and folders"
        );
        assert_eq!(removed.deleted_count(), Some(1));
    }

    fn result_for(
        command: &LegacyLibraryPlacementCommandV1,
    ) -> LegacyLibraryPlacementMutationResultV1 {
        let count = u16::try_from(command.requested_video_count()).expect("bounded");
        match command {
            LegacyLibraryPlacementCommandV1::AddToOrganization { .. } => {
                LegacyLibraryPlacementMutationResultV1::OrganizationAdded {
                    total_updated: count,
                }
            }
            LegacyLibraryPlacementCommandV1::RemoveFromOrganization { .. } => {
                LegacyLibraryPlacementMutationResultV1::OrganizationRemoved {
                    existing_shared: count,
                }
            }
            LegacyLibraryPlacementCommandV1::AddToSpace { .. } => {
                LegacyLibraryPlacementMutationResultV1::ScopeAdded {
                    valid_video_count: count,
                }
            }
            LegacyLibraryPlacementCommandV1::RemoveFromSpace { .. } => {
                LegacyLibraryPlacementMutationResultV1::ScopeRemoved {
                    valid_video_count: count,
                }
            }
        }
    }

    #[test]
    fn receipts_reject_wrong_variants_or_impossible_counts() {
        let command = LegacyLibraryPlacementAdapterV1::add_videos_to_organization()
            .prepare(&organization_add(vec![VIDEO_ONE]))
            .expect("command");
        assert_eq!(
            LegacyLibraryPlacementMutationReceiptV1::new(
                &command,
                LegacyLibraryPlacementMutationResultV1::OrganizationAdded { total_updated: 0 }
            ),
            Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt)
        );
        assert_eq!(
            LegacyLibraryPlacementMutationReceiptV1::new(
                &command,
                LegacyLibraryPlacementMutationResultV1::ScopeAdded {
                    valid_video_count: 1
                }
            ),
            Err(LegacyLibraryPlacementAtomicErrorV1::Corrupt)
        );

        let remove = LegacyLibraryPlacementAdapterV1::remove_videos_from_organization()
            .prepare(&request(
                LegacyLibraryPlacementInputV1::RemoveFromOrganization {
                    legacy_organization_id: ORGANIZATION.into(),
                    legacy_video_ids: vec![VIDEO_ONE.into()],
                },
            ))
            .expect("remove");
        let no_match = LegacyLibraryPlacementMutationReceiptV1::new(
            &remove,
            LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared: 0 },
        )
        .expect("no matching is valid");
        assert_eq!(
            no_match.result(),
            LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared: 0 }
        );
        assert!(!no_match.effects().invalidates_scope_root());
        assert!(!no_match.effects().invalidates_caps());
    }

    #[test]
    fn invalidation_effects_match_each_source_action_exactly() {
        let add_org = LegacyLibraryPlacementAdapterV1::add_videos_to_organization()
            .prepare(&organization_add(vec![VIDEO_ONE]))
            .expect("add org");
        let add_org = LegacyLibraryPlacementMutationReceiptV1::new(&add_org, result_for(&add_org))
            .expect("receipt");
        assert!(add_org.effects().invalidates_scope_root());
        assert!(add_org.effects().invalidates_caps());

        let remove_space = LegacyLibraryPlacementAdapterV1::remove_videos_from_space()
            .prepare(&space_remove(SPACE, vec![VIDEO_ONE]))
            .expect("remove space");
        let remove_space =
            LegacyLibraryPlacementMutationReceiptV1::new(&remove_space, result_for(&remove_space))
                .expect("receipt");
        assert!(remove_space.effects().invalidates_scope_root());
        assert!(!remove_space.effects().invalidates_caps());
    }

    struct RecordingPort {
        calls: Mutex<
            Vec<(
                LegacyLibraryPlacementCommandV1,
                LegacyLibraryPlacementBrowserFenceV1,
            )>,
        >,
        result: Mutex<
            Option<
                Result<LegacyLibraryPlacementAtomicOutcomeV1, LegacyLibraryPlacementAtomicErrorV1>,
            >,
        >,
    }

    impl RecordingPort {
        fn returning(
            result: Result<
                LegacyLibraryPlacementAtomicOutcomeV1,
                LegacyLibraryPlacementAtomicErrorV1,
            >,
        ) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Mutex::new(Some(result)),
            }
        }
    }

    #[async_trait]
    impl LegacyLibraryPlacementAtomicPortV1 for RecordingPort {
        async fn execute_atomic(
            &self,
            command: &LegacyLibraryPlacementCommandV1,
            browser_fence: &LegacyLibraryPlacementBrowserFenceV1,
        ) -> Result<LegacyLibraryPlacementAtomicOutcomeV1, LegacyLibraryPlacementAtomicErrorV1>
        {
            self.calls
                .lock()
                .expect("calls")
                .push((command.clone(), *browser_fence));
            self.result
                .lock()
                .expect("result")
                .take()
                .expect("one result")
        }
    }

    #[tokio::test]
    async fn adapter_calls_one_atomic_boundary_and_projects_exact_success() {
        let adapter = LegacyLibraryPlacementAdapterV1::add_videos_to_space();
        let request = space_add(SPACE, vec![VIDEO_ONE, VIDEO_TWO]);
        let command = adapter.prepare(&request).expect("command");
        let receipt = LegacyLibraryPlacementMutationReceiptV1::new(&command, result_for(&command))
            .expect("receipt");
        let port =
            RecordingPort::returning(Ok(LegacyLibraryPlacementAtomicOutcomeV1::Applied(receipt)));
        let browser_fence = LegacyLibraryPlacementBrowserFenceV1::fixture(actor());
        let execution = adapter
            .execute_fenced(&port, &request, &browser_fence)
            .await
            .expect("execution");
        assert_eq!(execution.success().message(), "2 videos added to space");
        assert!(!execution.replayed());
        assert!(execution.mutation_was_applied());
        let calls = port.calls.lock().expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.actor_id(), actor());
    }

    #[tokio::test]
    async fn replay_returns_the_original_no_matching_result_without_new_mutation() {
        let adapter = LegacyLibraryPlacementAdapterV1::remove_videos_from_organization();
        let request = request(LegacyLibraryPlacementInputV1::RemoveFromOrganization {
            legacy_organization_id: ORGANIZATION.into(),
            legacy_video_ids: vec![VIDEO_ONE.into()],
        });
        let command = adapter.prepare(&request).expect("command");
        let receipt = LegacyLibraryPlacementMutationReceiptV1::new(
            &command,
            LegacyLibraryPlacementMutationResultV1::OrganizationRemoved { existing_shared: 0 },
        )
        .expect("receipt");
        let port =
            RecordingPort::returning(Ok(LegacyLibraryPlacementAtomicOutcomeV1::Replay(receipt)));
        let execution = adapter
            .execute_fenced(
                &port,
                &request,
                &LegacyLibraryPlacementBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        assert_eq!(
            execution.success(),
            &LegacyLibraryPlacementSuccessV1::OrganizationNoMatching
        );
        assert!(!execution.effects().invalidates_scope_root());
        assert!(!execution.effects().invalidates_caps());
        assert!(execution.replayed());
        assert!(!execution.mutation_was_applied());
    }

    #[tokio::test]
    async fn atomic_failures_map_to_stable_redacted_public_errors() {
        for (atomic, public) in [
            (
                LegacyLibraryPlacementAtomicErrorV1::TargetMissing,
                LegacyLibraryPlacementErrorV1::TargetNotFound,
            ),
            (
                LegacyLibraryPlacementAtomicErrorV1::AccessDenied,
                LegacyLibraryPlacementErrorV1::TargetNotFound,
            ),
            (
                LegacyLibraryPlacementAtomicErrorV1::Conflict,
                LegacyLibraryPlacementErrorV1::Conflict,
            ),
            (
                LegacyLibraryPlacementAtomicErrorV1::InFlight,
                LegacyLibraryPlacementErrorV1::Conflict,
            ),
            (
                LegacyLibraryPlacementAtomicErrorV1::Unavailable,
                LegacyLibraryPlacementErrorV1::AuthorityUnavailable,
            ),
        ] {
            let adapter = LegacyLibraryPlacementAdapterV1::add_videos_to_organization();
            let request = organization_add(vec![VIDEO_ONE]);
            let port = RecordingPort::returning(Err(atomic));
            assert_eq!(
                adapter
                    .execute_fenced(
                        &port,
                        &request,
                        &LegacyLibraryPlacementBrowserFenceV1::fixture(actor()),
                    )
                    .await,
                Err(public)
            );
        }
    }

    #[tokio::test]
    async fn browser_fence_actor_must_match_before_the_atomic_port() {
        let adapter = LegacyLibraryPlacementAdapterV1::add_videos_to_organization();
        let request = organization_add(vec![VIDEO_ONE]);
        let command = adapter.prepare(&request).expect("command");
        let receipt = LegacyLibraryPlacementMutationReceiptV1::new(&command, result_for(&command))
            .expect("receipt");
        let port =
            RecordingPort::returning(Ok(LegacyLibraryPlacementAtomicOutcomeV1::Applied(receipt)));
        assert_eq!(
            adapter
                .execute_fenced(
                    &port,
                    &request,
                    &LegacyLibraryPlacementBrowserFenceV1::fixture(other_actor()),
                )
                .await,
            Err(LegacyLibraryPlacementErrorV1::Unauthorized)
        );
        assert!(port.calls.lock().expect("calls").is_empty());
    }
}
