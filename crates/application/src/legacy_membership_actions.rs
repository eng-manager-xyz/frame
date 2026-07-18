//! Source-pinned compatibility contracts for six provider-free Cap membership actions.
//!
//! This module deliberately separates what Cap did at commit
//! `6ba69561ac86b8efdb17616d6727f9638015546b` from the authority, atomicity, and
//! replay guarantees required by Frame. Cap authenticated each server action,
//! but trusted client-selected organization/space identifiers, performed
//! authority reads separately from writes, and had no idempotency journal. The
//! `setSpaceMembers` action additionally deleted every row before inserting the
//! replacement set without a transaction. Frame must preserve the observable
//! success shapes while closing those defects at one provider-free D1 boundary.

use std::{collections::BTreeMap, fmt};

use async_trait::async_trait;
use frame_domain::{
    IdempotencyKey, LegacyCapNanoId, OrganizationId, OrganizationInviteId, SessionId,
    SessionMutationGrantId, SpaceId, SpaceRole, UserId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;

pub const LEGACY_MEMBERSHIP_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID: &str = "cap-v1-866dbe8fbbfd7887";
pub const LEGACY_ADD_SPACE_MEMBER_OPERATION_ID: &str = "cap-v1-455046db3d6ef019";
pub const LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID: &str = "cap-v1-b177854e2386c877";
pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID: &str = "cap-v1-38aff8e7221d0260";
pub const LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID: &str = "cap-v1-135614e516c47bf4";
pub const LEGACY_SET_SPACE_MEMBERS_OPERATION_ID: &str = "cap-v1-9fc80bdec80fb248";
pub const LEGACY_REMOVE_ORGANIZATION_INVITE_SOURCE_MANIFEST_SHA256: &str =
    "ea3f6239587e553b7f2950819a8c93b7b8c117073c69f42b79ebedc7363273b6";
pub const LEGACY_ADD_SPACE_MEMBER_SOURCE_MANIFEST_SHA256: &str =
    "c2a5f769b737c905167f7eba012aac4597b63dc671ac0abd4ab235bb0ed4765b";
pub const LEGACY_ADD_SPACE_MEMBERS_SOURCE_MANIFEST_SHA256: &str =
    "a59e80018af3b40e7123fd002957d70a53f3d8a9736264cae682768c875eb740";
pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCE_MANIFEST_SHA256: &str =
    "65736ad49aefa91f15d4df0d06b4e479c84f5d1298cd084b5e4fa1b2d383f863";
pub const LEGACY_REMOVE_SPACE_MEMBER_SOURCE_MANIFEST_SHA256: &str =
    "4eb3b743365bb539d11223f48c374cdca88189078d71753a983da871afd1a3f7";
pub const LEGACY_SET_SPACE_MEMBERS_SOURCE_MANIFEST_SHA256: &str =
    "54155e7d69a029bbcd0182c0c6a4f0ea5ce62600691084d4366128f8c9a874c7";
pub const LEGACY_REMOVE_ORGANIZATION_INVITE_IDENTITY: &str =
    "action://apps/web/actions/organization/remove-invite.ts#removeOrganizationInvite";
pub const LEGACY_ADD_SPACE_MEMBER_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#addSpaceMember";
pub const LEGACY_ADD_SPACE_MEMBERS_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#addSpaceMembers";
pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#batchRemoveSpaceMembers";
pub const LEGACY_REMOVE_SPACE_MEMBER_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#removeSpaceMember";
pub const LEGACY_SET_SPACE_MEMBERS_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts#setSpaceMembers";
pub const LEGACY_MEMBERSHIP_POLICY: &str = "organization_library.v1";
pub const LEGACY_MEMBERSHIP_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_ORGANIZATION_SETTINGS_REVALIDATION_PATH: &str = "/dashboard/settings/organization";
pub const LEGACY_SPACE_REVALIDATION_PATH_TEMPLATE: &str = "/dashboard/spaces/{spaceId}";
pub const LEGACY_MEMBERSHIP_MAX_BODY_BYTES: usize = 256 * 1024;
pub const MAX_LEGACY_MEMBERSHIP_TARGETS: usize = 500;
pub const MAX_LEGACY_DISCOVERED_SPACE_MEMBERS: usize = 100_000;
pub const LEGACY_MEMBERSHIP_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"];
pub const LEGACY_MEMBERSHIP_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_REMOVE_INVITE_SOURCE_DECLARED_FAILURES: &[&str] = &[
    "Unauthorized",
    "Organization not found",
    "Forbidden",
    "Organization settings are only available to admins and owners",
    "Only admins and owners can manage organization settings",
    "Invite not found",
];
pub const LEGACY_REMOVE_INVITE_REACHABLE_FAILURES: &[&str] = &[
    "Unauthorized",
    "Organization not found",
    "Forbidden",
    "Organization settings are only available to admins and owners",
    "Invite not found",
    "unprojected database-driver error",
];
pub const LEGACY_REMOVE_INVITE_SOURCE_DECLARED_BUT_UNREACHABLE_FAILURES: &[&str] =
    &["Only admins and owners can manage organization settings"];
pub const LEGACY_SPACE_MEMBERSHIP_SOURCE_DECLARED_FAILURES: &[&str] = &[
    "Invalid input",
    "Unauthorized",
    "Space not found",
    "Only space admins, organization admins, and owners can manage this space",
    "All space members must belong to the organization",
];
pub const LEGACY_SPACE_MEMBERSHIP_REACHABLE_FAILURES: &[&str] = &[
    "Invalid input",
    "Unauthorized",
    "Space not found",
    "Only space admins, organization admins, and owners can manage this space",
    "All space members must belong to the organization",
    "unprojected database-driver error",
];
pub const LEGACY_REMOVE_SPACE_MEMBER_SOURCE_DECLARED_FAILURES: &[&str] = &[
    "Invalid input",
    "Unauthorized",
    "Member not found",
    "Space ID not found",
    "Only space admins, organization admins, and owners can manage this space",
    "You do not have permission to remove this space member",
];
pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCE_DECLARED_FAILURES: &[&str] = &[
    "Invalid input",
    "Unauthorized",
    "Cannot remove members from multiple spaces at once",
    "Only space admins, organization admins, and owners can manage this space",
    "You do not have permission to remove one or more members",
];
pub const LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES: &[&str] = &[
    "Unauthorized",
    "Invalid input",
    "An idempotency key is required",
    "Membership target not found",
    "Membership request conflicts with current state",
    "Membership authority is unavailable",
    "Membership action failed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipSourceRoleV1 {
    Action,
    Caller,
    Session,
    Authorization,
    Roles,
    Schema,
    Identifier,
    Database,
    DependencyLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMembershipSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
    pub role: LegacyMembershipSourceRoleV1,
}

pub const LEGACY_REMOVE_ORGANIZATION_INVITE_SOURCES: &[LegacyMembershipSourcePinV1] = &[
    LegacyMembershipSourcePinV1 {
        path: "apps/web/actions/organization/remove-invite.ts",
        sha256: "614aed36f22c5187b7ac27d0367b6c5467da1a87f30d83ea2b05582f14d7a5b0",
        role: LegacyMembershipSourceRoleV1::Action,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/settings/organization/components/MembersCard.tsx",
        sha256: "65e4e28028188a3ee29c25d94161419dbfdec04cb458e7aff9a450c51dbed743",
        role: LegacyMembershipSourceRoleV1::Caller,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyMembershipSourceRoleV1::Session,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyMembershipSourceRoleV1::Session,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/actions/organization/authorization.ts",
        sha256: "6b1422de53d0915a985dc1dbf70f14494fd1c5fe49ca61fbacabd90bebb00980",
        role: LegacyMembershipSourceRoleV1::Authorization,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
        role: LegacyMembershipSourceRoleV1::Roles,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyMembershipSourceRoleV1::Schema,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/index.ts",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyMembershipSourceRoleV1::Database,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyMembershipSourceRoleV1::DependencyLock,
    },
];

pub const LEGACY_ADD_SPACE_MEMBER_SOURCES: &[LegacyMembershipSourcePinV1] = &[
    LegacyMembershipSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        sha256: "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        role: LegacyMembershipSourceRoleV1::Action,
    },
    // No call site for `addSpaceMember` exists in this pinned snapshot. The
    // sibling bulk action is also distinct and is not silently folded into it.
    LegacyMembershipSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyMembershipSourceRoleV1::Session,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyMembershipSourceRoleV1::Session,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/actions/organization/space-authorization.ts",
        sha256: "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
        role: LegacyMembershipSourceRoleV1::Authorization,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
        role: LegacyMembershipSourceRoleV1::Roles,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyMembershipSourceRoleV1::Schema,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/index.ts",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyMembershipSourceRoleV1::Database,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyMembershipSourceRoleV1::DependencyLock,
    },
];

pub const LEGACY_SET_SPACE_MEMBERS_SOURCES: &[LegacyMembershipSourcePinV1] = &[
    LegacyMembershipSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/actions.ts",
        sha256: "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745",
        role: LegacyMembershipSourceRoleV1::Action,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/MembersIndicator.tsx",
        sha256: "7981d1f2320f618efbf8de916d6a2a8828dfa832ebbb1a93b8555955209d4790",
        role: LegacyMembershipSourceRoleV1::Caller,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyMembershipSourceRoleV1::Session,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyMembershipSourceRoleV1::Session,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/actions/organization/space-authorization.ts",
        sha256: "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445",
        role: LegacyMembershipSourceRoleV1::Authorization,
    },
    LegacyMembershipSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
        role: LegacyMembershipSourceRoleV1::Roles,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyMembershipSourceRoleV1::Schema,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/database/index.ts",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyMembershipSourceRoleV1::Database,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
        role: LegacyMembershipSourceRoleV1::Identifier,
    },
    LegacyMembershipSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyMembershipSourceRoleV1::DependencyLock,
    },
];

// The three sibling actions live in the same pinned module and close over the
// same session, authority, role, schema, identifier, and database sources.
// Keeping the full closure here prevents a one-file source pin from hiding a
// future authorization or schema drift.
pub const LEGACY_ADD_SPACE_MEMBERS_SOURCES: &[LegacyMembershipSourcePinV1] =
    LEGACY_ADD_SPACE_MEMBER_SOURCES;
pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCES: &[LegacyMembershipSourcePinV1] =
    LEGACY_ADD_SPACE_MEMBER_SOURCES;
pub const LEGACY_REMOVE_SPACE_MEMBER_SOURCES: &[LegacyMembershipSourcePinV1] =
    LEGACY_ADD_SPACE_MEMBER_SOURCES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipActionV1 {
    RemoveOrganizationInvite,
    AddSpaceMember,
    AddSpaceMembers,
    BatchRemoveSpaceMembers,
    RemoveSpaceMember,
    SetSpaceMembers,
}

impl LegacyMembershipActionV1 {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::RemoveOrganizationInvite => "remove_organization_invite",
            Self::AddSpaceMember => "add_space_member",
            Self::AddSpaceMembers => "add_space_members",
            Self::BatchRemoveSpaceMembers => "batch_remove_space_members",
            Self::RemoveSpaceMember => "remove_space_member",
            Self::SetSpaceMembers => "set_space_members",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipObservedAuthorizationV1 {
    ClientOrganizationThenOwnerOrAdmin,
    GlobalSpaceThenOrganizationOwnerOrAdminOrSpaceAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipRequiredAuthorizationV1 {
    SessionActorActiveTenantOwnerOrAdminAndScopedInvite,
    SessionActorActiveTenantSpaceManagerAndAllTargetsInOrganization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipObservedMutationV1 {
    DeleteInviteByIdAndOrganization,
    InsertOneSpaceMember,
    InsertOnlyUsersNotAlreadyInSpace,
    DeleteOneSpaceMemberByOpaqueRowId,
    DeleteResolvedSpaceMembersByOpaqueRowIds,
    DeleteAllThenInsertCreatorInclusiveReplacementWithoutTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipRequiredMutationV1 {
    DeleteExactlyOneScopedInvite,
    InsertExactlyOnePreviouslyAbsentSpaceMember,
    InsertExactAbsentSubsetOrConflictOnDuplicateNewTargets,
    DeleteExactlyOneNonCreatorMember,
    DeleteExactSingleSpaceNonCreatorMemberSet,
    ReplaceMembershipSetAndForceCreatorAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipObservedSuccessV1 {
    SuccessObject,
    SuccessObjectWithAddedAndAlreadyMemberIds,
    SuccessObjectWithRemovedMemberIds,
    SuccessObjectWithCreatorInclusiveCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipObservedFailureV1 {
    ThrownAuthenticationAuthorizationNotFoundOrDatabaseError,
    ThrownValidationAuthorizationMembershipOrDatabaseError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipObservedInvalidationV1 {
    RevalidateOrganizationSettingsDashboard,
    RevalidateSelectedSpaceDashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipObservedRetryDefectV1 {
    SecondDeleteBecomesInviteNotFound,
    DuplicateInsertHitsUniqueConstraint,
    RetryRecomputesAddedAndAlreadySets,
    RetryCanSilentlyRemoveNothing,
    DeleteInsertWindowCanEraseMembershipsAndReplayRewritesIds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipInputShapeV1 {
    PositionalInviteAndOrganizationIdentifiers,
    ObjectSpaceUserAndRole,
    ObjectSpaceUserIdsAndRole,
    ObjectMemberId,
    ObjectMemberIds,
    ObjectSpaceUserIdsFallbackRoleAndOptionalMemberRoles,
}

/// Presence rules from the pinned Zod object, before this module receives its
/// normalized application input. `Option` below means "field was absent"; an
/// HTTP decoder must not collapse an explicit JSON `null` into that state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipInputPresenceV1 {
    RoleMissingDefaultsToMember,
    MembersMissingSelectsUserIds,
    ExplicitNullRoleOrMembersIsInvalid,
}

pub const LEGACY_SET_SPACE_MEMBERS_INPUT_PRESENCE: &[LegacyMembershipInputPresenceV1] = &[
    LegacyMembershipInputPresenceV1::RoleMissingDefaultsToMember,
    LegacyMembershipInputPresenceV1::MembersMissingSelectsUserIds,
    LegacyMembershipInputPresenceV1::ExplicitNullRoleOrMembersIsInvalid,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipRequiredReplayV1 {
    ReturnOriginalJournaledSuccessWithoutReapplyingMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipRequiredKeyReuseV1 {
    SameFingerprintReplaysDifferentFingerprintConflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipRequiredAtomicityV1 {
    BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMembershipRequiredRetryV1 {
    pub replay: LegacyMembershipRequiredReplayV1,
    pub key_reuse: LegacyMembershipRequiredKeyReuseV1,
    pub atomicity: LegacyMembershipRequiredAtomicityV1,
}

pub const LEGACY_MEMBERSHIP_REQUIRED_RETRY: LegacyMembershipRequiredRetryV1 =
    LegacyMembershipRequiredRetryV1 {
        replay: LegacyMembershipRequiredReplayV1::ReturnOriginalJournaledSuccessWithoutReapplyingMutation,
        key_reuse:
            LegacyMembershipRequiredKeyReuseV1::SameFingerprintReplaysDifferentFingerprintConflicts,
        atomicity:
            LegacyMembershipRequiredAtomicityV1::BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMembershipProfileV1 {
    pub operation_id: &'static str,
    pub source_manifest_sha256: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub sources: &'static [LegacyMembershipSourcePinV1],
    pub authentication: &'static str,
    pub policy: &'static str,
    pub content_type: &'static str,
    pub max_body_bytes: usize,
    pub input: LegacyMembershipInputShapeV1,
    pub observed_authorization: LegacyMembershipObservedAuthorizationV1,
    pub required_authorization: LegacyMembershipRequiredAuthorizationV1,
    pub observed_mutation: LegacyMembershipObservedMutationV1,
    pub required_mutation: LegacyMembershipRequiredMutationV1,
    pub observed_success: LegacyMembershipObservedSuccessV1,
    pub observed_failure: LegacyMembershipObservedFailureV1,
    pub observed_invalidation: LegacyMembershipObservedInvalidationV1,
    pub observed_revalidation_path: &'static str,
    pub input_presence: &'static [LegacyMembershipInputPresenceV1],
    pub source_declared_failure_messages: &'static [&'static str],
    pub observed_failure_messages: &'static [&'static str],
    pub source_declared_but_unreachable_failure_messages: &'static [&'static str],
    pub required_public_failure_messages: &'static [&'static str],
    pub observed_retry_defect: LegacyMembershipObservedRetryDefectV1,
    pub required_retry: LegacyMembershipRequiredRetryV1,
    pub provider_effect: Option<&'static str>,
    pub tenant_non_disclosure: bool,
    pub observed_idempotency: &'static str,
    pub idempotency: &'static str,
    pub rate_limit_bucket: &'static str,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

pub const LEGACY_REMOVE_ORGANIZATION_INVITE_PROFILE: LegacyMembershipProfileV1 =
    LegacyMembershipProfileV1 {
        operation_id: LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID,
        source_manifest_sha256: LEGACY_REMOVE_ORGANIZATION_INVITE_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_REMOVE_ORGANIZATION_INVITE_IDENTITY,
        pinned_commit: LEGACY_MEMBERSHIP_CAP_COMMIT,
        sources: LEGACY_REMOVE_ORGANIZATION_INVITE_SOURCES,
        authentication: "session",
        policy: LEGACY_MEMBERSHIP_POLICY,
        content_type: LEGACY_MEMBERSHIP_CONTENT_TYPE,
        max_body_bytes: LEGACY_MEMBERSHIP_MAX_BODY_BYTES,
        input: LegacyMembershipInputShapeV1::PositionalInviteAndOrganizationIdentifiers,
        observed_authorization:
            LegacyMembershipObservedAuthorizationV1::ClientOrganizationThenOwnerOrAdmin,
        required_authorization:
            LegacyMembershipRequiredAuthorizationV1::SessionActorActiveTenantOwnerOrAdminAndScopedInvite,
        observed_mutation: LegacyMembershipObservedMutationV1::DeleteInviteByIdAndOrganization,
        required_mutation: LegacyMembershipRequiredMutationV1::DeleteExactlyOneScopedInvite,
        observed_success: LegacyMembershipObservedSuccessV1::SuccessObject,
        observed_failure:
            LegacyMembershipObservedFailureV1::ThrownAuthenticationAuthorizationNotFoundOrDatabaseError,
        observed_invalidation:
            LegacyMembershipObservedInvalidationV1::RevalidateOrganizationSettingsDashboard,
        observed_revalidation_path: LEGACY_ORGANIZATION_SETTINGS_REVALIDATION_PATH,
        input_presence: &[],
        source_declared_failure_messages: LEGACY_REMOVE_INVITE_SOURCE_DECLARED_FAILURES,
        observed_failure_messages: LEGACY_REMOVE_INVITE_REACHABLE_FAILURES,
        source_declared_but_unreachable_failure_messages:
            LEGACY_REMOVE_INVITE_SOURCE_DECLARED_BUT_UNREACHABLE_FAILURES,
        required_public_failure_messages: LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES,
        observed_retry_defect:
            LegacyMembershipObservedRetryDefectV1::SecondDeleteBecomesInviteNotFound,
        required_retry: LEGACY_MEMBERSHIP_REQUIRED_RETRY,
        provider_effect: None,
        tenant_non_disclosure: true,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_MEMBERSHIP_POLICY,
        protected_gates: LEGACY_MEMBERSHIP_PROTECTED_GATES,
        production_promoted: false,
    };

pub const LEGACY_ADD_SPACE_MEMBER_PROFILE: LegacyMembershipProfileV1 =
    LegacyMembershipProfileV1 {
        operation_id: LEGACY_ADD_SPACE_MEMBER_OPERATION_ID,
        source_manifest_sha256: LEGACY_ADD_SPACE_MEMBER_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_ADD_SPACE_MEMBER_IDENTITY,
        pinned_commit: LEGACY_MEMBERSHIP_CAP_COMMIT,
        sources: LEGACY_ADD_SPACE_MEMBER_SOURCES,
        authentication: "session",
        policy: LEGACY_MEMBERSHIP_POLICY,
        content_type: LEGACY_MEMBERSHIP_CONTENT_TYPE,
        max_body_bytes: LEGACY_MEMBERSHIP_MAX_BODY_BYTES,
        input: LegacyMembershipInputShapeV1::ObjectSpaceUserAndRole,
        observed_authorization:
            LegacyMembershipObservedAuthorizationV1::GlobalSpaceThenOrganizationOwnerOrAdminOrSpaceAdmin,
        required_authorization:
            LegacyMembershipRequiredAuthorizationV1::SessionActorActiveTenantSpaceManagerAndAllTargetsInOrganization,
        observed_mutation: LegacyMembershipObservedMutationV1::InsertOneSpaceMember,
        required_mutation:
            LegacyMembershipRequiredMutationV1::InsertExactlyOnePreviouslyAbsentSpaceMember,
        observed_success: LegacyMembershipObservedSuccessV1::SuccessObject,
        observed_failure:
            LegacyMembershipObservedFailureV1::ThrownValidationAuthorizationMembershipOrDatabaseError,
        observed_invalidation:
            LegacyMembershipObservedInvalidationV1::RevalidateSelectedSpaceDashboard,
        observed_revalidation_path: LEGACY_SPACE_REVALIDATION_PATH_TEMPLATE,
        input_presence: &[],
        source_declared_failure_messages: LEGACY_SPACE_MEMBERSHIP_SOURCE_DECLARED_FAILURES,
        observed_failure_messages: LEGACY_SPACE_MEMBERSHIP_REACHABLE_FAILURES,
        source_declared_but_unreachable_failure_messages: &[],
        required_public_failure_messages: LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES,
        observed_retry_defect:
            LegacyMembershipObservedRetryDefectV1::DuplicateInsertHitsUniqueConstraint,
        required_retry: LEGACY_MEMBERSHIP_REQUIRED_RETRY,
        provider_effect: None,
        tenant_non_disclosure: true,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_MEMBERSHIP_POLICY,
        protected_gates: LEGACY_MEMBERSHIP_PROTECTED_GATES,
        production_promoted: false,
    };

pub const LEGACY_ADD_SPACE_MEMBERS_PROFILE: LegacyMembershipProfileV1 =
    LegacyMembershipProfileV1 {
        operation_id: LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID,
        source_manifest_sha256: LEGACY_ADD_SPACE_MEMBERS_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_ADD_SPACE_MEMBERS_IDENTITY,
        pinned_commit: LEGACY_MEMBERSHIP_CAP_COMMIT,
        sources: LEGACY_ADD_SPACE_MEMBERS_SOURCES,
        authentication: "session",
        policy: LEGACY_MEMBERSHIP_POLICY,
        content_type: LEGACY_MEMBERSHIP_CONTENT_TYPE,
        max_body_bytes: LEGACY_MEMBERSHIP_MAX_BODY_BYTES,
        input: LegacyMembershipInputShapeV1::ObjectSpaceUserIdsAndRole,
        observed_authorization:
            LegacyMembershipObservedAuthorizationV1::GlobalSpaceThenOrganizationOwnerOrAdminOrSpaceAdmin,
        required_authorization:
            LegacyMembershipRequiredAuthorizationV1::SessionActorActiveTenantSpaceManagerAndAllTargetsInOrganization,
        observed_mutation: LegacyMembershipObservedMutationV1::InsertOnlyUsersNotAlreadyInSpace,
        required_mutation:
            LegacyMembershipRequiredMutationV1::InsertExactAbsentSubsetOrConflictOnDuplicateNewTargets,
        observed_success:
            LegacyMembershipObservedSuccessV1::SuccessObjectWithAddedAndAlreadyMemberIds,
        observed_failure:
            LegacyMembershipObservedFailureV1::ThrownValidationAuthorizationMembershipOrDatabaseError,
        observed_invalidation:
            LegacyMembershipObservedInvalidationV1::RevalidateSelectedSpaceDashboard,
        observed_revalidation_path: LEGACY_SPACE_REVALIDATION_PATH_TEMPLATE,
        input_presence: &[],
        source_declared_failure_messages: LEGACY_SPACE_MEMBERSHIP_SOURCE_DECLARED_FAILURES,
        observed_failure_messages: LEGACY_SPACE_MEMBERSHIP_REACHABLE_FAILURES,
        source_declared_but_unreachable_failure_messages: &[],
        required_public_failure_messages: LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES,
        observed_retry_defect: LegacyMembershipObservedRetryDefectV1::RetryRecomputesAddedAndAlreadySets,
        required_retry: LEGACY_MEMBERSHIP_REQUIRED_RETRY,
        provider_effect: None,
        tenant_non_disclosure: true,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_MEMBERSHIP_POLICY,
        protected_gates: LEGACY_MEMBERSHIP_NO_PROTECTED_GATES,
        production_promoted: true,
    };

pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_PROFILE: LegacyMembershipProfileV1 =
    LegacyMembershipProfileV1 {
        operation_id: LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID,
        source_manifest_sha256: LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_BATCH_REMOVE_SPACE_MEMBERS_IDENTITY,
        pinned_commit: LEGACY_MEMBERSHIP_CAP_COMMIT,
        sources: LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCES,
        authentication: "session",
        policy: LEGACY_MEMBERSHIP_POLICY,
        content_type: LEGACY_MEMBERSHIP_CONTENT_TYPE,
        max_body_bytes: LEGACY_MEMBERSHIP_MAX_BODY_BYTES,
        input: LegacyMembershipInputShapeV1::ObjectMemberIds,
        observed_authorization:
            LegacyMembershipObservedAuthorizationV1::GlobalSpaceThenOrganizationOwnerOrAdminOrSpaceAdmin,
        required_authorization:
            LegacyMembershipRequiredAuthorizationV1::SessionActorActiveTenantSpaceManagerAndAllTargetsInOrganization,
        observed_mutation:
            LegacyMembershipObservedMutationV1::DeleteResolvedSpaceMembersByOpaqueRowIds,
        required_mutation:
            LegacyMembershipRequiredMutationV1::DeleteExactSingleSpaceNonCreatorMemberSet,
        observed_success: LegacyMembershipObservedSuccessV1::SuccessObjectWithRemovedMemberIds,
        observed_failure:
            LegacyMembershipObservedFailureV1::ThrownValidationAuthorizationMembershipOrDatabaseError,
        observed_invalidation:
            LegacyMembershipObservedInvalidationV1::RevalidateSelectedSpaceDashboard,
        observed_revalidation_path: LEGACY_SPACE_REVALIDATION_PATH_TEMPLATE,
        input_presence: &[],
        source_declared_failure_messages:
            LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCE_DECLARED_FAILURES,
        observed_failure_messages: LEGACY_BATCH_REMOVE_SPACE_MEMBERS_SOURCE_DECLARED_FAILURES,
        source_declared_but_unreachable_failure_messages: &[],
        required_public_failure_messages: LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES,
        observed_retry_defect: LegacyMembershipObservedRetryDefectV1::RetryCanSilentlyRemoveNothing,
        required_retry: LEGACY_MEMBERSHIP_REQUIRED_RETRY,
        provider_effect: None,
        tenant_non_disclosure: true,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_MEMBERSHIP_POLICY,
        protected_gates: LEGACY_MEMBERSHIP_NO_PROTECTED_GATES,
        production_promoted: true,
    };

pub const LEGACY_REMOVE_SPACE_MEMBER_PROFILE: LegacyMembershipProfileV1 =
    LegacyMembershipProfileV1 {
        operation_id: LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID,
        source_manifest_sha256: LEGACY_REMOVE_SPACE_MEMBER_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_REMOVE_SPACE_MEMBER_IDENTITY,
        pinned_commit: LEGACY_MEMBERSHIP_CAP_COMMIT,
        sources: LEGACY_REMOVE_SPACE_MEMBER_SOURCES,
        authentication: "session",
        policy: LEGACY_MEMBERSHIP_POLICY,
        content_type: LEGACY_MEMBERSHIP_CONTENT_TYPE,
        max_body_bytes: LEGACY_MEMBERSHIP_MAX_BODY_BYTES,
        input: LegacyMembershipInputShapeV1::ObjectMemberId,
        observed_authorization:
            LegacyMembershipObservedAuthorizationV1::GlobalSpaceThenOrganizationOwnerOrAdminOrSpaceAdmin,
        required_authorization:
            LegacyMembershipRequiredAuthorizationV1::SessionActorActiveTenantSpaceManagerAndAllTargetsInOrganization,
        observed_mutation: LegacyMembershipObservedMutationV1::DeleteOneSpaceMemberByOpaqueRowId,
        required_mutation: LegacyMembershipRequiredMutationV1::DeleteExactlyOneNonCreatorMember,
        observed_success: LegacyMembershipObservedSuccessV1::SuccessObject,
        observed_failure:
            LegacyMembershipObservedFailureV1::ThrownValidationAuthorizationMembershipOrDatabaseError,
        observed_invalidation:
            LegacyMembershipObservedInvalidationV1::RevalidateSelectedSpaceDashboard,
        observed_revalidation_path: LEGACY_SPACE_REVALIDATION_PATH_TEMPLATE,
        input_presence: &[],
        source_declared_failure_messages: LEGACY_REMOVE_SPACE_MEMBER_SOURCE_DECLARED_FAILURES,
        observed_failure_messages: LEGACY_REMOVE_SPACE_MEMBER_SOURCE_DECLARED_FAILURES,
        source_declared_but_unreachable_failure_messages: &[],
        required_public_failure_messages: LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES,
        observed_retry_defect: LegacyMembershipObservedRetryDefectV1::RetryCanSilentlyRemoveNothing,
        required_retry: LEGACY_MEMBERSHIP_REQUIRED_RETRY,
        provider_effect: None,
        tenant_non_disclosure: true,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_MEMBERSHIP_POLICY,
        protected_gates: LEGACY_MEMBERSHIP_NO_PROTECTED_GATES,
        production_promoted: true,
    };

pub const LEGACY_SET_SPACE_MEMBERS_PROFILE: LegacyMembershipProfileV1 =
    LegacyMembershipProfileV1 {
        operation_id: LEGACY_SET_SPACE_MEMBERS_OPERATION_ID,
        source_manifest_sha256: LEGACY_SET_SPACE_MEMBERS_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_SET_SPACE_MEMBERS_IDENTITY,
        pinned_commit: LEGACY_MEMBERSHIP_CAP_COMMIT,
        sources: LEGACY_SET_SPACE_MEMBERS_SOURCES,
        authentication: "session",
        policy: LEGACY_MEMBERSHIP_POLICY,
        content_type: LEGACY_MEMBERSHIP_CONTENT_TYPE,
        max_body_bytes: LEGACY_MEMBERSHIP_MAX_BODY_BYTES,
        input: LegacyMembershipInputShapeV1::ObjectSpaceUserIdsFallbackRoleAndOptionalMemberRoles,
        observed_authorization:
            LegacyMembershipObservedAuthorizationV1::GlobalSpaceThenOrganizationOwnerOrAdminOrSpaceAdmin,
        required_authorization:
            LegacyMembershipRequiredAuthorizationV1::SessionActorActiveTenantSpaceManagerAndAllTargetsInOrganization,
        observed_mutation:
            LegacyMembershipObservedMutationV1::DeleteAllThenInsertCreatorInclusiveReplacementWithoutTransaction,
        required_mutation:
            LegacyMembershipRequiredMutationV1::ReplaceMembershipSetAndForceCreatorAdmin,
        observed_success:
            LegacyMembershipObservedSuccessV1::SuccessObjectWithCreatorInclusiveCount,
        observed_failure:
            LegacyMembershipObservedFailureV1::ThrownValidationAuthorizationMembershipOrDatabaseError,
        observed_invalidation:
            LegacyMembershipObservedInvalidationV1::RevalidateSelectedSpaceDashboard,
        observed_revalidation_path: LEGACY_SPACE_REVALIDATION_PATH_TEMPLATE,
        input_presence: LEGACY_SET_SPACE_MEMBERS_INPUT_PRESENCE,
        source_declared_failure_messages: LEGACY_SPACE_MEMBERSHIP_SOURCE_DECLARED_FAILURES,
        observed_failure_messages: LEGACY_SPACE_MEMBERSHIP_REACHABLE_FAILURES,
        source_declared_but_unreachable_failure_messages: &[],
        required_public_failure_messages: LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES,
        observed_retry_defect:
            LegacyMembershipObservedRetryDefectV1::DeleteInsertWindowCanEraseMembershipsAndReplayRewritesIds,
        required_retry: LEGACY_MEMBERSHIP_REQUIRED_RETRY,
        provider_effect: None,
        tenant_non_disclosure: true,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_MEMBERSHIP_POLICY,
        protected_gates: LEGACY_MEMBERSHIP_PROTECTED_GATES,
        production_promoted: false,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacySpaceMemberRoleV1 {
    Admin,
    Member,
}

impl LegacySpaceMemberRoleV1 {
    fn parse_source(value: &str) -> Option<Self> {
        match value {
            "admin" | "Admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    /// Exact non-broadening translation into Frame's richer space-role model.
    /// Cap `member` had no contributor authority, so it maps to `viewer`.
    #[must_use]
    pub const fn frame_role(self) -> SpaceRole {
        match self {
            Self::Admin => SpaceRole::Manager,
            Self::Member => SpaceRole::Viewer,
        }
    }

    /// Translate a database role back into the exact legacy role. Frame's
    /// `contributor` role has no equivalent in these actions and fails closed.
    #[must_use]
    pub const fn from_frame_role(role: SpaceRole) -> Option<Self> {
        match role {
            SpaceRole::Manager => Some(Self::Admin),
            SpaceRole::Viewer => Some(Self::Member),
            SpaceRole::Contributor => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacySubmittedSpaceMemberV1 {
    pub legacy_user_id: String,
    pub role: String,
}

impl fmt::Debug for LegacySubmittedSpaceMemberV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacySubmittedSpaceMemberV1")
            .field("user", &"<redacted>")
            .field("role", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyMembershipInputV1 {
    RemoveOrganizationInvite {
        legacy_invite_id: String,
        legacy_organization_id: String,
    },
    AddSpaceMember {
        legacy_space_id: String,
        legacy_user_id: String,
        role: String,
    },
    AddSpaceMembers {
        legacy_space_id: String,
        legacy_user_ids: Vec<String>,
        role: String,
    },
    BatchRemoveSpaceMembers {
        legacy_member_ids: Vec<String>,
    },
    RemoveSpaceMember {
        legacy_member_id: String,
    },
    SetSpaceMembers {
        legacy_space_id: String,
        legacy_user_ids: Vec<String>,
        role: Option<String>,
        members: Option<Vec<LegacySubmittedSpaceMemberV1>>,
    },
}

impl LegacyMembershipInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyMembershipActionV1 {
        match self {
            Self::RemoveOrganizationInvite { .. } => {
                LegacyMembershipActionV1::RemoveOrganizationInvite
            }
            Self::AddSpaceMember { .. } => LegacyMembershipActionV1::AddSpaceMember,
            Self::AddSpaceMembers { .. } => LegacyMembershipActionV1::AddSpaceMembers,
            Self::BatchRemoveSpaceMembers { .. } => {
                LegacyMembershipActionV1::BatchRemoveSpaceMembers
            }
            Self::RemoveSpaceMember { .. } => LegacyMembershipActionV1::RemoveSpaceMember,
            Self::SetSpaceMembers { .. } => LegacyMembershipActionV1::SetSpaceMembers,
        }
    }
}

impl fmt::Debug for LegacyMembershipInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoveOrganizationInvite { .. } => formatter
                .debug_struct("RemoveOrganizationInvite")
                .field("organization", &"<redacted>")
                .field("invite", &"<redacted>")
                .finish(),
            Self::AddSpaceMember { .. } => formatter
                .debug_struct("AddSpaceMember")
                .field("space", &"<redacted>")
                .field("target", &"<redacted>")
                .field("role", &"<redacted>")
                .finish(),
            Self::AddSpaceMembers {
                legacy_user_ids, ..
            } => formatter
                .debug_struct("AddSpaceMembers")
                .field("space", &"<redacted>")
                .field("target_count", &legacy_user_ids.len())
                .field("targets", &"<redacted>")
                .field("role", &"<redacted>")
                .finish(),
            Self::BatchRemoveSpaceMembers { legacy_member_ids } => formatter
                .debug_struct("BatchRemoveSpaceMembers")
                .field("target_count", &legacy_member_ids.len())
                .field("targets", &"<redacted>")
                .finish(),
            Self::RemoveSpaceMember { .. } => formatter
                .debug_struct("RemoveSpaceMember")
                .field("target", &"<redacted>")
                .finish(),
            Self::SetSpaceMembers {
                legacy_user_ids,
                members,
                ..
            } => formatter
                .debug_struct("SetSpaceMembers")
                .field("space", &"<redacted>")
                .field("user_id_count", &legacy_user_ids.len())
                .field("member_count", &members.as_ref().map(Vec::len))
                .field("targets", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMembershipRequestV1 {
    pub credential: Option<LegacyMembershipCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyMembershipInputV1,
}

impl fmt::Debug for LegacyMembershipRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMembershipRequestV1")
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
pub struct LegacyMembershipAuthorityV1 {
    actor_id: UserId,
    active_organization_id: OrganizationId,
}

impl LegacyMembershipAuthorityV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> OrganizationId {
        self.active_organization_id
    }
}

impl fmt::Debug for LegacyMembershipAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyMembershipAuthorityV1([redacted])")
    }
}

/// Unforgeable browser material for the atomic membership boundary.
///
/// The D1 implementation must assert this grant is live, belongs to the same
/// actor/session, and consume it in the same transaction as authority reads,
/// mutation, business audit, invalidation context, and idempotency journal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyMembershipBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyMembershipBrowserFenceV1 {
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

impl fmt::Debug for LegacyMembershipBrowserFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyMembershipBrowserFenceV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMembershipFenceV1 {
    authority: LegacyMembershipAuthorityV1,
    idempotency_key: IdempotencyKey,
}

impl LegacyMembershipFenceV1 {
    #[must_use]
    pub const fn authority(&self) -> &LegacyMembershipAuthorityV1 {
        &self.authority
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }
}

impl fmt::Debug for LegacyMembershipFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMembershipFenceV1")
            .field("authority", &self.authority)
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacySpaceMemberTargetV1 {
    user_id: UserId,
    role: LegacySpaceMemberRoleV1,
}

impl LegacySpaceMemberTargetV1 {
    #[must_use]
    pub const fn user_id(self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub const fn role(self) -> LegacySpaceMemberRoleV1 {
        self.role
    }

    /// Construct a final-state row from a database role. A contributor row is
    /// corrupt for this exact compatibility family and cannot be represented.
    pub fn from_database_role(
        user_id: UserId,
        role: SpaceRole,
    ) -> Result<Self, LegacyMembershipAtomicErrorV1> {
        let role = LegacySpaceMemberRoleV1::from_frame_role(role)
            .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?;
        Ok(Self { user_id, role })
    }
}

impl fmt::Debug for LegacySpaceMemberTargetV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacySpaceMemberTargetV1")
            .field("user", &"<redacted>")
            .field("role", &self.role)
            .finish()
    }
}

/// Canonical UUID alias for Cap's opaque `spaceMembers.id` row identifier.
/// Frame's native membership table is keyed by `(space_id, user_id)`, so the
/// D1 adapter retains this alias separately for the two legacy removal inputs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegacySpaceMemberIdV1 {
    legacy_id: String,
    mapped_uuid: String,
}

impl LegacySpaceMemberIdV1 {
    pub fn from_legacy(value: String) -> Result<Self, LegacyMembershipErrorV1> {
        let legacy =
            LegacyCapNanoId::parse(value).map_err(|_| LegacyMembershipErrorV1::TargetNotFound)?;
        Ok(Self {
            legacy_id: legacy.as_str().to_owned(),
            mapped_uuid: legacy.mapped_uuid().to_string(),
        })
    }

    pub fn from_verified_database_row(
        legacy_id: String,
        mapped_uuid: String,
    ) -> Result<Self, LegacyMembershipAtomicErrorV1> {
        let legacy = LegacyCapNanoId::parse(legacy_id.clone())
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        if !is_canonical_uuid_text(&mapped_uuid) || legacy.mapped_uuid().to_string() != mapped_uuid
        {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            legacy_id,
            mapped_uuid,
        })
    }

    #[must_use]
    pub fn legacy_id(&self) -> &str {
        &self.legacy_id
    }

    #[must_use]
    pub fn mapped_uuid(&self) -> &str {
        &self.mapped_uuid
    }
}

impl fmt::Debug for LegacySpaceMemberIdV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacySpaceMemberIdV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacySpaceMemberRemovalTargetV1 {
    member_id: LegacySpaceMemberIdV1,
    user_id: UserId,
}

impl LegacySpaceMemberRemovalTargetV1 {
    pub fn from_verified_database_row(member_id: LegacySpaceMemberIdV1, user_id: UserId) -> Self {
        Self { member_id, user_id }
    }

    #[must_use]
    pub const fn member_id(&self) -> &LegacySpaceMemberIdV1 {
        &self.member_id
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }
}

impl fmt::Debug for LegacySpaceMemberRemovalTargetV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacySpaceMemberRemovalTargetV1")
            .field("member", &"<redacted>")
            .field("user", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacySpaceMemberUserAliasV1 {
    legacy_user_id: String,
    user_id: UserId,
}

impl LegacySpaceMemberUserAliasV1 {
    pub fn from_verified_database_row(
        legacy_user_id: String,
        user_id: UserId,
    ) -> Result<Self, LegacyMembershipAtomicErrorV1> {
        let legacy = LegacyCapNanoId::parse(legacy_user_id.clone())
            .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
        if legacy.mapped_uuid().to_string() != user_id.to_string() {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            legacy_user_id,
            user_id,
        })
    }

    #[must_use]
    pub fn legacy_user_id(&self) -> &str {
        &self.legacy_user_id
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }
}

impl fmt::Debug for LegacySpaceMemberUserAliasV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacySpaceMemberUserAliasV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyMembershipCommandV1 {
    RemoveOrganizationInvite {
        fence: LegacyMembershipFenceV1,
        organization_id: OrganizationId,
        invite_id: OrganizationInviteId,
    },
    AddSpaceMember {
        fence: LegacyMembershipFenceV1,
        space_id: SpaceId,
        legacy_user_id: String,
        target: LegacySpaceMemberTargetV1,
    },
    AddSpaceMembers {
        fence: LegacyMembershipFenceV1,
        space_id: SpaceId,
        legacy_user_ids: Vec<String>,
        submitted_members: Vec<LegacySpaceMemberTargetV1>,
    },
    BatchRemoveSpaceMembers {
        fence: LegacyMembershipFenceV1,
        member_ids: Vec<LegacySpaceMemberIdV1>,
    },
    RemoveSpaceMember {
        fence: LegacyMembershipFenceV1,
        member_id: LegacySpaceMemberIdV1,
    },
    SetSpaceMembers {
        fence: LegacyMembershipFenceV1,
        space_id: SpaceId,
        legacy_user_ids: Vec<String>,
        submitted_members: Vec<LegacySpaceMemberTargetV1>,
    },
}

impl LegacyMembershipCommandV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyMembershipActionV1 {
        match self {
            Self::RemoveOrganizationInvite { .. } => {
                LegacyMembershipActionV1::RemoveOrganizationInvite
            }
            Self::AddSpaceMember { .. } => LegacyMembershipActionV1::AddSpaceMember,
            Self::AddSpaceMembers { .. } => LegacyMembershipActionV1::AddSpaceMembers,
            Self::BatchRemoveSpaceMembers { .. } => {
                LegacyMembershipActionV1::BatchRemoveSpaceMembers
            }
            Self::RemoveSpaceMember { .. } => LegacyMembershipActionV1::RemoveSpaceMember,
            Self::SetSpaceMembers { .. } => LegacyMembershipActionV1::SetSpaceMembers,
        }
    }

    #[must_use]
    pub const fn fence(&self) -> &LegacyMembershipFenceV1 {
        match self {
            Self::RemoveOrganizationInvite { fence, .. }
            | Self::AddSpaceMember { fence, .. }
            | Self::AddSpaceMembers { fence, .. }
            | Self::BatchRemoveSpaceMembers { fence, .. }
            | Self::RemoveSpaceMember { fence, .. }
            | Self::SetSpaceMembers { fence, .. } => fence,
        }
    }

    #[must_use]
    pub const fn space_id(&self) -> Option<SpaceId> {
        match self {
            Self::RemoveOrganizationInvite { .. } => None,
            Self::AddSpaceMember { space_id, .. }
            | Self::AddSpaceMembers { space_id, .. }
            | Self::SetSpaceMembers { space_id, .. } => Some(*space_id),
            Self::BatchRemoveSpaceMembers { .. } | Self::RemoveSpaceMember { .. } => None,
        }
    }

    #[must_use]
    pub fn submitted_members(&self) -> &[LegacySpaceMemberTargetV1] {
        match self {
            Self::RemoveOrganizationInvite { .. } => &[],
            Self::AddSpaceMember { target, .. } => std::slice::from_ref(target),
            Self::AddSpaceMembers {
                submitted_members, ..
            }
            | Self::SetSpaceMembers {
                submitted_members, ..
            } => submitted_members,
            Self::BatchRemoveSpaceMembers { .. } | Self::RemoveSpaceMember { .. } => &[],
        }
    }

    #[must_use]
    pub fn submitted_member_ids(&self) -> &[LegacySpaceMemberIdV1] {
        match self {
            Self::BatchRemoveSpaceMembers { member_ids, .. } => member_ids,
            Self::RemoveSpaceMember { member_id, .. } => std::slice::from_ref(member_id),
            Self::RemoveOrganizationInvite { .. }
            | Self::AddSpaceMember { .. }
            | Self::AddSpaceMembers { .. }
            | Self::SetSpaceMembers { .. } => &[],
        }
    }

    #[must_use]
    pub fn submitted_legacy_user_ids(&self) -> &[String] {
        match self {
            Self::AddSpaceMember { legacy_user_id, .. } => std::slice::from_ref(legacy_user_id),
            Self::AddSpaceMembers {
                legacy_user_ids, ..
            } => legacy_user_ids,
            Self::SetSpaceMembers {
                legacy_user_ids, ..
            } => legacy_user_ids,
            Self::RemoveOrganizationInvite { .. }
            | Self::BatchRemoveSpaceMembers { .. }
            | Self::RemoveSpaceMember { .. } => &[],
        }
    }

    /// Canonical semantic fingerprint after the port has discovered the
    /// trusted database context. Set-membership fingerprints use the effective
    /// creator-inclusive set, so omitting the creator or submitting the
    /// creator as `member` cannot manufacture a distinct idempotency request.
    pub fn request_fingerprint_for_context(
        &self,
        context: &LegacyMembershipDiscoveredContextV1,
    ) -> Result<[u8; 32], LegacyMembershipAtomicErrorV1> {
        if context.organization_id() != self.fence().authority().active_organization_id() {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let actor_id = self.fence().authority().actor_id();
        let organization_id = self.fence().authority().active_organization_id();
        match (self, context) {
            (
                Self::RemoveOrganizationInvite { invite_id, .. },
                LegacyMembershipDiscoveredContextV1::OrganizationInvite {
                    organization_id: discovered_organization,
                    invite_id: discovered_invite,
                },
            ) if discovered_organization == &organization_id && discovered_invite == invite_id => {
                Ok(fingerprint_remove_invite(
                    actor_id,
                    organization_id,
                    *invite_id,
                ))
            }
            (
                Self::AddSpaceMember {
                    space_id, target, ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceAdd {
                    space_id: discovered_space,
                    ..
                },
            ) if discovered_space == space_id => Ok(fingerprint_space_members(
                self.action(),
                actor_id,
                organization_id,
                *space_id,
                std::slice::from_ref(target),
            )),
            (
                Self::AddSpaceMembers {
                    space_id,
                    submitted_members,
                    ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                    space_id: discovered_space,
                    ..
                },
            ) if discovered_space == space_id => Ok(fingerprint_space_members(
                self.action(),
                actor_id,
                organization_id,
                *space_id,
                submitted_members,
            )),
            (
                Self::BatchRemoveSpaceMembers { member_ids, .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    removed_members, ..
                },
            ) if resolved_removals_belong_to_submission(member_ids, removed_members) => Ok(
                fingerprint_member_ids(self.action(), actor_id, organization_id, member_ids),
            ),
            (
                Self::BatchRemoveSpaceMembers { member_ids, .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop { .. },
            ) => Ok(fingerprint_member_ids(
                self.action(),
                actor_id,
                organization_id,
                member_ids,
            )),
            (
                Self::RemoveSpaceMember { member_id, .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    removed_members, ..
                },
            ) if removal_ids_match(std::slice::from_ref(member_id), removed_members) => {
                Ok(fingerprint_member_ids(
                    self.action(),
                    actor_id,
                    organization_id,
                    std::slice::from_ref(member_id),
                ))
            }
            (
                Self::SetSpaceMembers {
                    space_id,
                    submitted_members,
                    ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                    space_id: discovered_space,
                    creator_id,
                    ..
                },
            ) if discovered_space == space_id => {
                let effective_members = creator_inclusive_members(submitted_members, *creator_id);
                Ok(fingerprint_space_members(
                    self.action(),
                    actor_id,
                    organization_id,
                    *space_id,
                    &effective_members,
                ))
            }
            _ => Err(LegacyMembershipAtomicErrorV1::Corrupt),
        }
    }
}

impl fmt::Debug for LegacyMembershipCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::RemoveOrganizationInvite { .. } => "RemoveOrganizationInvite",
                Self::AddSpaceMember { .. } => "AddSpaceMember",
                Self::AddSpaceMembers { .. } => "AddSpaceMembers",
                Self::BatchRemoveSpaceMembers { .. } => "BatchRemoveSpaceMembers",
                Self::RemoveSpaceMember { .. } => "RemoveSpaceMember",
                Self::SetSpaceMembers { .. } => "SetSpaceMembers",
            })
            .field("fence", self.fence())
            .field("target", &"<redacted>")
            .field("submitted_member_count", &self.submitted_members().len())
            .finish()
    }
}

/// Context discovered under the same database snapshot as the mutation.
///
/// `previous_member_ids` is required for a replacement so authorization caches
/// for removed users are invalidated as well as caches for the submitted set.
/// The creator is database-owned context and is always included in the final
/// set with the `admin` role, regardless of a submitted role.
#[derive(Clone, PartialEq, Eq)]
pub enum LegacyMembershipDiscoveredContextV1 {
    OrganizationInvite {
        organization_id: OrganizationId,
        invite_id: OrganizationInviteId,
    },
    SpaceAdd {
        organization_id: OrganizationId,
        space_id: SpaceId,
        creator_id: UserId,
    },
    SpaceBulkAdd {
        organization_id: OrganizationId,
        space_id: SpaceId,
        creator_id: UserId,
        previous_members: Vec<LegacySpaceMemberUserAliasV1>,
    },
    SpaceRemoval {
        organization_id: OrganizationId,
        space_id: SpaceId,
        creator_id: UserId,
        removed_members: Vec<LegacySpaceMemberRemovalTargetV1>,
    },
    SpaceRemovalNoop {
        organization_id: OrganizationId,
    },
    SpaceReplacement {
        organization_id: OrganizationId,
        space_id: SpaceId,
        creator_id: UserId,
        previous_member_ids: Vec<UserId>,
    },
}

impl LegacyMembershipDiscoveredContextV1 {
    #[must_use]
    pub const fn organization_id(&self) -> OrganizationId {
        match self {
            Self::OrganizationInvite {
                organization_id, ..
            }
            | Self::SpaceAdd {
                organization_id, ..
            }
            | Self::SpaceBulkAdd {
                organization_id, ..
            }
            | Self::SpaceRemoval {
                organization_id, ..
            }
            | Self::SpaceRemovalNoop { organization_id }
            | Self::SpaceReplacement {
                organization_id, ..
            } => *organization_id,
        }
    }

    #[must_use]
    pub const fn space_id(&self) -> Option<SpaceId> {
        match self {
            Self::OrganizationInvite { .. } | Self::SpaceRemovalNoop { .. } => None,
            Self::SpaceAdd { space_id, .. }
            | Self::SpaceBulkAdd { space_id, .. }
            | Self::SpaceRemoval { space_id, .. }
            | Self::SpaceReplacement { space_id, .. } => Some(*space_id),
        }
    }

    #[must_use]
    pub const fn creator_id(&self) -> Option<UserId> {
        match self {
            Self::OrganizationInvite { .. } | Self::SpaceRemovalNoop { .. } => None,
            Self::SpaceAdd { creator_id, .. }
            | Self::SpaceBulkAdd { creator_id, .. }
            | Self::SpaceRemoval { creator_id, .. }
            | Self::SpaceReplacement { creator_id, .. } => Some(*creator_id),
        }
    }

    #[must_use]
    pub fn previous_member_ids(&self) -> &[UserId] {
        match self {
            Self::SpaceReplacement {
                previous_member_ids,
                ..
            } => previous_member_ids,
            Self::OrganizationInvite { .. }
            | Self::SpaceAdd { .. }
            | Self::SpaceBulkAdd { .. }
            | Self::SpaceRemoval { .. }
            | Self::SpaceRemovalNoop { .. } => &[],
        }
    }

    #[must_use]
    pub fn previous_member_aliases(&self) -> &[LegacySpaceMemberUserAliasV1] {
        match self {
            Self::SpaceBulkAdd {
                previous_members, ..
            } => previous_members,
            Self::OrganizationInvite { .. }
            | Self::SpaceAdd { .. }
            | Self::SpaceRemoval { .. }
            | Self::SpaceRemovalNoop { .. }
            | Self::SpaceReplacement { .. } => &[],
        }
    }

    #[must_use]
    pub fn removed_members(&self) -> &[LegacySpaceMemberRemovalTargetV1] {
        match self {
            Self::SpaceRemoval {
                removed_members, ..
            } => removed_members,
            Self::OrganizationInvite { .. }
            | Self::SpaceAdd { .. }
            | Self::SpaceBulkAdd { .. }
            | Self::SpaceRemovalNoop { .. }
            | Self::SpaceReplacement { .. } => &[],
        }
    }
}

impl fmt::Debug for LegacyMembershipDiscoveredContextV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::OrganizationInvite { .. } => "OrganizationInvite",
                Self::SpaceAdd { .. } => "SpaceAdd",
                Self::SpaceBulkAdd { .. } => "SpaceBulkAdd",
                Self::SpaceRemoval { .. } => "SpaceRemoval",
                Self::SpaceRemovalNoop { .. } => "SpaceRemovalNoop",
                Self::SpaceReplacement { .. } => "SpaceReplacement",
            })
            .field("scope", &"<redacted>")
            .field("creator_present", &self.creator_id().is_some())
            .field(
                "previous_member_count",
                &if self.previous_member_aliases().is_empty() {
                    self.previous_member_ids().len()
                } else {
                    self.previous_member_aliases().len()
                },
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyMembershipMutationResultV1 {
    InviteRemoved,
    SpaceMemberAdded,
    SpaceMembersAdded {
        added: Vec<String>,
        already_members: Vec<String>,
    },
    SpaceMembersRemoved {
        removed_member_ids: Vec<LegacySpaceMemberIdV1>,
    },
    SpaceMemberRemoved,
    SpaceMembersSet {
        count: u32,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMembershipEffectsV1 {
    organization_id: OrganizationId,
    space_id: Option<SpaceId>,
    invalidates_organization_invites: bool,
    invalidates_space_page: bool,
    invalidates_space_members: bool,
    bumps_authority_generation: bool,
    authority_subjects: Vec<UserId>,
}

impl LegacyMembershipEffectsV1 {
    #[must_use]
    pub const fn organization_id(&self) -> OrganizationId {
        self.organization_id
    }

    #[must_use]
    pub const fn space_id(&self) -> Option<SpaceId> {
        self.space_id
    }

    #[must_use]
    pub const fn invalidates_organization_invites(&self) -> bool {
        self.invalidates_organization_invites
    }

    #[must_use]
    pub const fn invalidates_space_page(&self) -> bool {
        self.invalidates_space_page
    }

    #[must_use]
    pub const fn invalidates_space_members(&self) -> bool {
        self.invalidates_space_members
    }

    #[must_use]
    pub const fn bumps_authority_generation(&self) -> bool {
        self.bumps_authority_generation
    }

    #[must_use]
    pub fn authority_subjects(&self) -> &[UserId] {
        &self.authority_subjects
    }
}

impl fmt::Debug for LegacyMembershipEffectsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMembershipEffectsV1")
            .field("scope", &"<redacted>")
            .field(
                "invalidates_organization_invites",
                &self.invalidates_organization_invites,
            )
            .field("invalidates_space_page", &self.invalidates_space_page)
            .field("invalidates_space_members", &self.invalidates_space_members)
            .field(
                "bumps_authority_generation",
                &self.bumps_authority_generation,
            )
            .field("authority_subject_count", &self.authority_subjects.len())
            .finish()
    }
}

/// Exact authority class observed from active database rows at the committing
/// boundary. There is deliberately no organization-member, contributor, or
/// viewer variant capable of authorizing a space mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMembershipActorAuthorityV1 {
    OrganizationOwner,
    OrganizationAdmin,
    ActiveOrganizationMember,
    SpaceCreator,
    SpaceManager,
}

impl LegacyMembershipActorAuthorityV1 {
    /// Only Frame `manager` is equivalent to Cap `admin` for authorization.
    #[must_use]
    pub const fn from_frame_space_role(role: SpaceRole) -> Option<Self> {
        match role {
            SpaceRole::Manager => Some(Self::SpaceManager),
            SpaceRole::Contributor | SpaceRole::Viewer => None,
        }
    }
}

/// Database-derived proof shape for the complete active authority graph and
/// the authority-cache side effects committed with a membership mutation.
///
/// A D1 implementation may call `from_verified_database_rows` only after one
/// transaction has proved: active/undeleted actor and session; active
/// organization and actor membership (unless the actor is the owner);
/// undeleted exact-tenant space; active/undeleted target users with active
/// organization memberships (or the organization owner); and the same facts
/// for the database-owned creator. `SpaceManager` means Frame `manager` exactly.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMembershipAuthorityPostconditionV1 {
    active_organization_id: OrganizationId,
    active_actor_id: UserId,
    actor_authority: LegacyMembershipActorAuthorityV1,
    active_space_id: Option<SpaceId>,
    active_target_ids: Vec<UserId>,
    active_creator_id: Option<UserId>,
    generation_bumped_subjects: Vec<UserId>,
    mutation_grants_revoked_subjects: Vec<UserId>,
}

impl LegacyMembershipAuthorityPostconditionV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn from_verified_database_rows(
        active_organization_id: OrganizationId,
        active_actor_id: UserId,
        actor_authority: LegacyMembershipActorAuthorityV1,
        active_space_id: Option<SpaceId>,
        active_target_ids: Vec<UserId>,
        active_creator_id: Option<UserId>,
        generation_bumped_subjects: Vec<UserId>,
        mutation_grants_revoked_subjects: Vec<UserId>,
    ) -> Result<Self, LegacyMembershipAtomicErrorV1> {
        Ok(Self {
            active_organization_id,
            active_actor_id,
            actor_authority,
            active_space_id,
            active_target_ids: canonical_user_ids(
                active_target_ids,
                MAX_LEGACY_MEMBERSHIP_TARGETS,
            )?,
            active_creator_id,
            generation_bumped_subjects: canonical_user_ids(
                generation_bumped_subjects,
                MAX_LEGACY_DISCOVERED_SPACE_MEMBERS + MAX_LEGACY_MEMBERSHIP_TARGETS + 1,
            )?,
            mutation_grants_revoked_subjects: canonical_user_ids(
                mutation_grants_revoked_subjects,
                MAX_LEGACY_DISCOVERED_SPACE_MEMBERS + MAX_LEGACY_MEMBERSHIP_TARGETS + 1,
            )?,
        })
    }

    #[must_use]
    pub const fn actor_authority(&self) -> LegacyMembershipActorAuthorityV1 {
        self.actor_authority
    }

    #[must_use]
    pub fn generation_bumped_subjects(&self) -> &[UserId] {
        &self.generation_bumped_subjects
    }

    #[must_use]
    pub fn mutation_grants_revoked_subjects(&self) -> &[UserId] {
        &self.mutation_grants_revoked_subjects
    }

    fn covers_command(
        &self,
        command: &LegacyMembershipCommandV1,
        context: &LegacyMembershipDiscoveredContextV1,
        effects: &LegacyMembershipEffectsV1,
    ) -> bool {
        let expected_targets = if matches!(
            command,
            LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. }
                | LegacyMembershipCommandV1::RemoveSpaceMember { .. }
        ) {
            canonical_user_ids(
                context
                    .removed_members()
                    .iter()
                    .map(LegacySpaceMemberRemovalTargetV1::user_id)
                    .collect(),
                MAX_LEGACY_MEMBERSHIP_TARGETS,
            )
            .unwrap_or_default()
        } else {
            canonical_user_ids(
                command
                    .submitted_members()
                    .iter()
                    .map(|member| member.user_id())
                    .collect(),
                MAX_LEGACY_MEMBERSHIP_TARGETS,
            )
            .unwrap_or_default()
        };
        let valid_authority = match (command, context) {
            (LegacyMembershipCommandV1::RemoveOrganizationInvite { .. }, _) => matches!(
                self.actor_authority,
                LegacyMembershipActorAuthorityV1::OrganizationOwner
                    | LegacyMembershipActorAuthorityV1::OrganizationAdmin
            ),
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop { .. },
            ) => matches!(
                self.actor_authority,
                LegacyMembershipActorAuthorityV1::OrganizationOwner
                    | LegacyMembershipActorAuthorityV1::OrganizationAdmin
                    | LegacyMembershipActorAuthorityV1::ActiveOrganizationMember
            ),
            (
                LegacyMembershipCommandV1::AddSpaceMember { .. }
                | LegacyMembershipCommandV1::AddSpaceMembers { .. }
                | LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. }
                | LegacyMembershipCommandV1::RemoveSpaceMember { .. }
                | LegacyMembershipCommandV1::SetSpaceMembers { .. },
                _,
            ) => matches!(
                self.actor_authority,
                LegacyMembershipActorAuthorityV1::OrganizationOwner
                    | LegacyMembershipActorAuthorityV1::OrganizationAdmin
                    | LegacyMembershipActorAuthorityV1::SpaceCreator
                    | LegacyMembershipActorAuthorityV1::SpaceManager
            ),
        };
        let creator_authority_matches = self.actor_authority
            != LegacyMembershipActorAuthorityV1::SpaceCreator
            || self.active_creator_id == Some(self.active_actor_id);
        self.active_organization_id == command.fence().authority().active_organization_id()
            && self.active_actor_id == command.fence().authority().actor_id()
            && self.active_space_id == context.space_id()
            && self.active_target_ids == expected_targets
            && self.active_creator_id == context.creator_id()
            && valid_authority
            && creator_authority_matches
            && self.generation_bumped_subjects == effects.authority_subjects
            && self.mutation_grants_revoked_subjects == effects.authority_subjects
    }
}

impl fmt::Debug for LegacyMembershipAuthorityPostconditionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMembershipAuthorityPostconditionV1")
            .field("scope", &"<redacted>")
            .field("actor_authority", &self.actor_authority)
            .field("has_active_space", &self.active_space_id.is_some())
            .field("active_target_count", &self.active_target_ids.len())
            .field("has_active_creator", &self.active_creator_id.is_some())
            .field(
                "generation_bump_count",
                &self.generation_bumped_subjects.len(),
            )
            .field(
                "grant_revocation_count",
                &self.mutation_grants_revoked_subjects.len(),
            )
            .finish()
    }
}

/// Exact database mutation and final-state facts measured in the committing
/// transaction. Public success remains source-compatible; this richer shape
/// exists only to prevent a port from manufacturing a receipt for a partial or
/// semantically different write.
#[derive(Clone, PartialEq, Eq)]
pub enum LegacyMembershipMutationPostconditionV1 {
    OrganizationInviteRemoved {
        matching_before: u32,
        deleted_rows: u32,
        matching_after: u32,
    },
    SpaceMemberInserted {
        matching_before: u32,
        inserted_rows: u32,
        matching_after: u32,
        final_member: LegacySpaceMemberTargetV1,
    },
    SpaceMembersInserted {
        matching_before: u32,
        inserted_rows: u32,
        matching_after: u32,
        added_members: Vec<LegacySpaceMemberTargetV1>,
        already_member_ids: Vec<String>,
    },
    SpaceMembersRemoved {
        matching_before: u32,
        deleted_rows: u32,
        matching_after: u32,
        removed_members: Vec<LegacySpaceMemberRemovalTargetV1>,
    },
    SpaceMembersReplaced {
        matching_before: u32,
        deleted_rows: u32,
        inserted_rows: u32,
        matching_after: u32,
        final_members: Vec<LegacySpaceMemberTargetV1>,
    },
}

impl LegacyMembershipMutationPostconditionV1 {
    fn covers_command(
        &self,
        command: &LegacyMembershipCommandV1,
        result: &LegacyMembershipMutationResultV1,
        context: &LegacyMembershipDiscoveredContextV1,
    ) -> Result<bool, LegacyMembershipAtomicErrorV1> {
        match (command, result, context, self) {
            (
                LegacyMembershipCommandV1::RemoveOrganizationInvite { .. },
                LegacyMembershipMutationResultV1::InviteRemoved,
                LegacyMembershipDiscoveredContextV1::OrganizationInvite { .. },
                Self::OrganizationInviteRemoved {
                    matching_before,
                    deleted_rows,
                    matching_after,
                },
            ) => Ok((*matching_before, *deleted_rows, *matching_after) == (1, 1, 0)),
            (
                LegacyMembershipCommandV1::AddSpaceMember { target, .. },
                LegacyMembershipMutationResultV1::SpaceMemberAdded,
                LegacyMembershipDiscoveredContextV1::SpaceAdd { .. },
                Self::SpaceMemberInserted {
                    matching_before,
                    inserted_rows,
                    matching_after,
                    final_member,
                },
            ) => Ok(
                (*matching_before, *inserted_rows, *matching_after) == (0, 1, 1)
                    && final_member == target
                    && final_member.role().frame_role() != SpaceRole::Contributor,
            ),
            (
                LegacyMembershipCommandV1::AddSpaceMembers {
                    legacy_user_ids,
                    submitted_members,
                    ..
                },
                LegacyMembershipMutationResultV1::SpaceMembersAdded {
                    added,
                    already_members,
                },
                LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                    previous_members, ..
                },
                Self::SpaceMembersInserted {
                    matching_before,
                    inserted_rows,
                    matching_after,
                    added_members,
                    already_member_ids,
                },
            ) => {
                if legacy_user_ids.len() != submitted_members.len() {
                    return Ok(false);
                }
                let previous_by_id = previous_members
                    .iter()
                    .map(|member| (member.user_id().to_string(), member))
                    .collect::<BTreeMap<_, _>>();
                let expected_added = legacy_user_ids
                    .iter()
                    .zip(submitted_members)
                    .filter(|(_, target)| {
                        !previous_by_id.contains_key(&target.user_id().to_string())
                    })
                    .map(|(legacy_user_id, target)| (legacy_user_id.clone(), *target))
                    .collect::<Vec<_>>();
                let canonical_added = canonical_unique_members(
                    expected_added.iter().map(|(_, target)| *target).collect(),
                )?;
                if canonical_added.len() != expected_added.len() {
                    return Ok(false);
                }
                let expected_already = if expected_added.is_empty() {
                    legacy_user_ids.clone()
                } else {
                    previous_members
                        .iter()
                        .map(|member| member.legacy_user_id().to_owned())
                        .collect()
                };
                let before = u32::try_from(previous_members.len())
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                let inserted = u32::try_from(expected_added.len())
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                let after = before
                    .checked_add(inserted)
                    .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)?;
                Ok(*matching_before == before
                    && *inserted_rows == inserted
                    && *matching_after == after
                    && added
                        == &expected_added
                            .iter()
                            .map(|(legacy_user_id, _)| legacy_user_id.clone())
                            .collect::<Vec<_>>()
                    && already_members == &expected_already
                    && canonical_unique_members(added_members.clone())? == canonical_added
                    && already_member_ids == &expected_already)
            }
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. },
                LegacyMembershipMutationResultV1::SpaceMembersRemoved { removed_member_ids },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    creator_id,
                    removed_members: discovered,
                    ..
                },
                Self::SpaceMembersRemoved {
                    matching_before,
                    deleted_rows,
                    matching_after,
                    removed_members,
                },
            ) => removal_postcondition_matches(
                member_ids,
                removed_member_ids,
                discovered,
                removed_members,
                *creator_id,
                *matching_before,
                *deleted_rows,
                *matching_after,
            ),
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. },
                LegacyMembershipMutationResultV1::SpaceMembersRemoved { removed_member_ids },
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop { .. },
                Self::SpaceMembersRemoved {
                    matching_before,
                    deleted_rows,
                    matching_after,
                    removed_members,
                },
            ) => Ok(removed_member_ids.is_empty()
                && removed_members.is_empty()
                && (*matching_before, *deleted_rows, *matching_after) == (0, 0, 0)),
            (
                LegacyMembershipCommandV1::RemoveSpaceMember { member_id, .. },
                LegacyMembershipMutationResultV1::SpaceMemberRemoved,
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    creator_id,
                    removed_members: discovered,
                    ..
                },
                Self::SpaceMembersRemoved {
                    matching_before,
                    deleted_rows,
                    matching_after,
                    removed_members,
                },
            ) => removal_postcondition_matches(
                std::slice::from_ref(member_id),
                std::slice::from_ref(member_id),
                discovered,
                removed_members,
                *creator_id,
                *matching_before,
                *deleted_rows,
                *matching_after,
            ),
            (
                LegacyMembershipCommandV1::SetSpaceMembers {
                    submitted_members, ..
                },
                LegacyMembershipMutationResultV1::SpaceMembersSet { count },
                LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                    creator_id,
                    previous_member_ids,
                    ..
                },
                Self::SpaceMembersReplaced {
                    matching_before,
                    deleted_rows,
                    inserted_rows,
                    matching_after,
                    final_members,
                },
            ) => {
                let expected = creator_inclusive_members(submitted_members, *creator_id);
                let final_members = canonical_final_members(final_members.clone())?;
                let previous = u32::try_from(previous_member_ids.len())
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                let expected_count = u32::try_from(expected.len())
                    .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
                Ok(*matching_before == previous
                    && *deleted_rows == previous
                    && *inserted_rows == expected_count
                    && *matching_after == expected_count
                    && *count == expected_count
                    && final_members == expected)
            }
            _ => Ok(false),
        }
    }
}

impl fmt::Debug for LegacyMembershipMutationPostconditionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrganizationInviteRemoved {
                matching_before,
                deleted_rows,
                matching_after,
            } => formatter
                .debug_struct("OrganizationInviteRemoved")
                .field("matching_before", matching_before)
                .field("deleted_rows", deleted_rows)
                .field("matching_after", matching_after)
                .finish(),
            Self::SpaceMemberInserted {
                matching_before,
                inserted_rows,
                matching_after,
                ..
            } => formatter
                .debug_struct("SpaceMemberInserted")
                .field("matching_before", matching_before)
                .field("inserted_rows", inserted_rows)
                .field("matching_after", matching_after)
                .field("final_member", &"<redacted>")
                .finish(),
            Self::SpaceMembersInserted {
                matching_before,
                inserted_rows,
                matching_after,
                added_members,
                already_member_ids,
            } => formatter
                .debug_struct("SpaceMembersInserted")
                .field("matching_before", matching_before)
                .field("inserted_rows", inserted_rows)
                .field("matching_after", matching_after)
                .field("added_member_count", &added_members.len())
                .field("already_member_count", &already_member_ids.len())
                .finish(),
            Self::SpaceMembersRemoved {
                matching_before,
                deleted_rows,
                matching_after,
                removed_members,
            } => formatter
                .debug_struct("SpaceMembersRemoved")
                .field("matching_before", matching_before)
                .field("deleted_rows", deleted_rows)
                .field("matching_after", matching_after)
                .field("removed_member_count", &removed_members.len())
                .finish(),
            Self::SpaceMembersReplaced {
                matching_before,
                deleted_rows,
                inserted_rows,
                matching_after,
                final_members,
            } => formatter
                .debug_struct("SpaceMembersReplaced")
                .field("matching_before", matching_before)
                .field("deleted_rows", deleted_rows)
                .field("inserted_rows", inserted_rows)
                .field("matching_after", matching_after)
                .field("final_member_count", &final_members.len())
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMembershipMutationReceiptV1 {
    request_fingerprint: [u8; 32],
    result: LegacyMembershipMutationResultV1,
    context: LegacyMembershipDiscoveredContextV1,
    effects: LegacyMembershipEffectsV1,
    mutation_postcondition: LegacyMembershipMutationPostconditionV1,
    authority_postcondition: LegacyMembershipAuthorityPostconditionV1,
}

impl LegacyMembershipMutationReceiptV1 {
    pub fn new(
        command: &LegacyMembershipCommandV1,
        result: LegacyMembershipMutationResultV1,
        context: LegacyMembershipDiscoveredContextV1,
        mutation_postcondition: LegacyMembershipMutationPostconditionV1,
        authority_postcondition: LegacyMembershipAuthorityPostconditionV1,
    ) -> Result<Self, LegacyMembershipAtomicErrorV1> {
        let context = canonicalize_discovered_context(context)?;
        if context.organization_id() != command.fence().authority().active_organization_id() {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }

        let effects = match (command, &result, &context) {
            (
                LegacyMembershipCommandV1::RemoveOrganizationInvite {
                    organization_id,
                    invite_id,
                    ..
                },
                LegacyMembershipMutationResultV1::InviteRemoved,
                LegacyMembershipDiscoveredContextV1::OrganizationInvite {
                    organization_id: discovered_organization,
                    invite_id: discovered_invite,
                },
            ) if organization_id == discovered_organization && invite_id == discovered_invite => {
                LegacyMembershipEffectsV1 {
                    organization_id: *organization_id,
                    space_id: None,
                    invalidates_organization_invites: true,
                    invalidates_space_page: false,
                    invalidates_space_members: false,
                    bumps_authority_generation: false,
                    authority_subjects: Vec::new(),
                }
            }
            (
                LegacyMembershipCommandV1::AddSpaceMember {
                    space_id, target, ..
                },
                LegacyMembershipMutationResultV1::SpaceMemberAdded,
                LegacyMembershipDiscoveredContextV1::SpaceAdd {
                    organization_id,
                    space_id: discovered_space,
                    ..
                },
            ) if space_id == discovered_space => LegacyMembershipEffectsV1 {
                organization_id: *organization_id,
                space_id: Some(*space_id),
                invalidates_organization_invites: false,
                invalidates_space_page: true,
                invalidates_space_members: true,
                bumps_authority_generation: true,
                authority_subjects: vec![target.user_id()],
            },
            (
                LegacyMembershipCommandV1::AddSpaceMembers {
                    space_id,
                    legacy_user_ids,
                    submitted_members,
                    ..
                },
                LegacyMembershipMutationResultV1::SpaceMembersAdded { added, .. },
                LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                    organization_id,
                    space_id: discovered_space,
                    ..
                },
            ) if space_id == discovered_space => {
                if legacy_user_ids.len() != submitted_members.len() {
                    return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                }
                let authority_subjects = canonical_user_ids(
                    added
                        .iter()
                        .map(|added_id| {
                            legacy_user_ids
                                .iter()
                                .zip(submitted_members)
                                .find_map(|(legacy_user_id, target)| {
                                    (legacy_user_id == added_id).then_some(target.user_id())
                                })
                                .ok_or(LegacyMembershipAtomicErrorV1::Corrupt)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    MAX_LEGACY_MEMBERSHIP_TARGETS,
                )?;
                LegacyMembershipEffectsV1 {
                    organization_id: *organization_id,
                    space_id: Some(*space_id),
                    invalidates_organization_invites: false,
                    invalidates_space_page: true,
                    invalidates_space_members: true,
                    bumps_authority_generation: !authority_subjects.is_empty(),
                    authority_subjects,
                }
            }
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. },
                LegacyMembershipMutationResultV1::SpaceMembersRemoved { removed_member_ids },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    organization_id,
                    space_id,
                    creator_id,
                    removed_members,
                },
            ) if member_ids == removed_member_ids
                && !removed_members.is_empty()
                && resolved_removals_belong_to_submission(member_ids, removed_members)
                && removed_members
                    .iter()
                    .all(|target| target.user_id() != *creator_id) =>
            {
                LegacyMembershipEffectsV1 {
                    organization_id: *organization_id,
                    space_id: Some(*space_id),
                    invalidates_organization_invites: false,
                    invalidates_space_page: true,
                    invalidates_space_members: true,
                    bumps_authority_generation: true,
                    authority_subjects: canonical_user_ids(
                        removed_members
                            .iter()
                            .map(LegacySpaceMemberRemovalTargetV1::user_id)
                            .collect(),
                        MAX_LEGACY_MEMBERSHIP_TARGETS,
                    )?,
                }
            }
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. },
                LegacyMembershipMutationResultV1::SpaceMembersRemoved { removed_member_ids },
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop { organization_id },
            ) if removed_member_ids.is_empty() => LegacyMembershipEffectsV1 {
                organization_id: *organization_id,
                space_id: None,
                invalidates_organization_invites: false,
                invalidates_space_page: false,
                invalidates_space_members: false,
                bumps_authority_generation: false,
                authority_subjects: Vec::new(),
            },
            (
                LegacyMembershipCommandV1::RemoveSpaceMember { member_id, .. },
                LegacyMembershipMutationResultV1::SpaceMemberRemoved,
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    organization_id,
                    space_id,
                    creator_id,
                    removed_members,
                },
            ) if removal_ids_match(std::slice::from_ref(member_id), removed_members)
                && removed_members
                    .iter()
                    .all(|target| target.user_id() != *creator_id) =>
            {
                LegacyMembershipEffectsV1 {
                    organization_id: *organization_id,
                    space_id: Some(*space_id),
                    invalidates_organization_invites: false,
                    invalidates_space_page: true,
                    invalidates_space_members: true,
                    bumps_authority_generation: true,
                    authority_subjects: removed_members
                        .iter()
                        .map(LegacySpaceMemberRemovalTargetV1::user_id)
                        .collect(),
                }
            }
            (
                LegacyMembershipCommandV1::SetSpaceMembers {
                    space_id,
                    submitted_members,
                    ..
                },
                LegacyMembershipMutationResultV1::SpaceMembersSet { count },
                LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                    organization_id,
                    space_id: discovered_space,
                    creator_id,
                    previous_member_ids,
                },
            ) if space_id == discovered_space
                && previous_member_ids.len() <= MAX_LEGACY_DISCOVERED_SPACE_MEMBERS =>
            {
                let expected = creator_inclusive_count(submitted_members, *creator_id)?;
                if *count != expected {
                    return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                }
                let authority_subjects = canonical_authority_subjects(
                    previous_member_ids,
                    submitted_members,
                    *creator_id,
                );
                LegacyMembershipEffectsV1 {
                    organization_id: *organization_id,
                    space_id: Some(*space_id),
                    invalidates_organization_invites: false,
                    invalidates_space_page: true,
                    invalidates_space_members: true,
                    bumps_authority_generation: true,
                    authority_subjects,
                }
            }
            _ => return Err(LegacyMembershipAtomicErrorV1::Corrupt),
        };

        if !mutation_postcondition.covers_command(command, &result, &context)?
            || !authority_postcondition.covers_command(command, &context, &effects)
        {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
        let request_fingerprint = command.request_fingerprint_for_context(&context)?;

        Ok(Self {
            request_fingerprint,
            result,
            context,
            effects,
            mutation_postcondition,
            authority_postcondition,
        })
    }

    #[must_use]
    pub fn result(&self) -> LegacyMembershipMutationResultV1 {
        self.result.clone()
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    #[must_use]
    pub fn matches_command(&self, command: &LegacyMembershipCommandV1) -> bool {
        command
            .request_fingerprint_for_context(&self.context)
            .is_ok_and(|fingerprint| self.request_fingerprint == fingerprint)
    }

    #[must_use]
    pub const fn context(&self) -> &LegacyMembershipDiscoveredContextV1 {
        &self.context
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyMembershipEffectsV1 {
        &self.effects
    }

    #[must_use]
    pub const fn mutation_postcondition(&self) -> &LegacyMembershipMutationPostconditionV1 {
        &self.mutation_postcondition
    }

    #[must_use]
    pub const fn authority_postcondition(&self) -> &LegacyMembershipAuthorityPostconditionV1 {
        &self.authority_postcondition
    }
}

fn canonicalize_discovered_context(
    context: LegacyMembershipDiscoveredContextV1,
) -> Result<LegacyMembershipDiscoveredContextV1, LegacyMembershipAtomicErrorV1> {
    match context {
        LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
            organization_id,
            space_id,
            creator_id,
            previous_members,
        } => {
            if previous_members.len() > MAX_LEGACY_DISCOVERED_SPACE_MEMBERS {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            let mut by_user = BTreeMap::new();
            for member in previous_members {
                if by_user
                    .insert(member.user_id().to_string(), member)
                    .is_some()
                {
                    return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                }
            }
            Ok(LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                organization_id,
                space_id,
                creator_id,
                previous_members: by_user.into_values().collect(),
            })
        }
        LegacyMembershipDiscoveredContextV1::SpaceRemoval {
            organization_id,
            space_id,
            creator_id,
            removed_members,
        } => {
            if removed_members.is_empty()
                || removed_members.len() > MAX_LEGACY_MEMBERSHIP_TARGETS
                || removed_members
                    .iter()
                    .any(|target| target.user_id() == creator_id)
            {
                return Err(LegacyMembershipAtomicErrorV1::Corrupt);
            }
            let mut resolved = BTreeMap::new();
            for target in &removed_members {
                match resolved.insert(target.member_id().mapped_uuid(), target.user_id()) {
                    Some(user_id) if user_id != target.user_id() => {
                        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
                    }
                    _ => {}
                }
            }
            Ok(LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                organization_id,
                space_id,
                creator_id,
                removed_members,
            })
        }
        LegacyMembershipDiscoveredContextV1::SpaceReplacement {
            organization_id,
            space_id,
            creator_id,
            previous_member_ids,
        } => Ok(LegacyMembershipDiscoveredContextV1::SpaceReplacement {
            organization_id,
            space_id,
            creator_id,
            previous_member_ids: canonical_user_ids(
                previous_member_ids,
                MAX_LEGACY_DISCOVERED_SPACE_MEMBERS,
            )?,
        }),
        other => Ok(other),
    }
}

impl fmt::Debug for LegacyMembershipMutationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMembershipMutationReceiptV1")
            .field("request_fingerprint", &"<redacted>")
            .field("result", &self.result)
            .field("context", &self.context)
            .field("effects", &self.effects)
            .field("mutation_postcondition", &self.mutation_postcondition)
            .field("authority_postcondition", &self.authority_postcondition)
            .finish()
    }
}

fn creator_inclusive_count(
    submitted_members: &[LegacySpaceMemberTargetV1],
    creator_id: UserId,
) -> Result<u32, LegacyMembershipAtomicErrorV1> {
    u32::try_from(creator_inclusive_members(submitted_members, creator_id).len())
        .map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)
}

fn creator_inclusive_members(
    submitted_members: &[LegacySpaceMemberTargetV1],
    creator_id: UserId,
) -> Vec<LegacySpaceMemberTargetV1> {
    let mut by_id = BTreeMap::new();
    for member in submitted_members {
        by_id.insert(member.user_id().to_string(), *member);
    }
    by_id.insert(
        creator_id.to_string(),
        LegacySpaceMemberTargetV1 {
            user_id: creator_id,
            role: LegacySpaceMemberRoleV1::Admin,
        },
    );
    by_id.into_values().collect()
}

fn canonical_user_ids(
    values: Vec<UserId>,
    max: usize,
) -> Result<Vec<UserId>, LegacyMembershipAtomicErrorV1> {
    if values.len() > max {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    let mut by_id = BTreeMap::new();
    for value in values {
        by_id.insert(value.to_string(), value);
    }
    Ok(by_id.into_values().collect())
}

fn canonical_final_members(
    values: Vec<LegacySpaceMemberTargetV1>,
) -> Result<Vec<LegacySpaceMemberTargetV1>, LegacyMembershipAtomicErrorV1> {
    if values.len() > MAX_LEGACY_MEMBERSHIP_TARGETS + 1 {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    let mut by_id = BTreeMap::new();
    for value in values {
        if by_id.insert(value.user_id().to_string(), value).is_some() {
            return Err(LegacyMembershipAtomicErrorV1::Corrupt);
        }
    }
    Ok(by_id.into_values().collect())
}

fn canonical_unique_members(
    values: Vec<LegacySpaceMemberTargetV1>,
) -> Result<Vec<LegacySpaceMemberTargetV1>, LegacyMembershipAtomicErrorV1> {
    if values.len() > MAX_LEGACY_MEMBERSHIP_TARGETS {
        return Err(LegacyMembershipAtomicErrorV1::Corrupt);
    }
    let mut by_id = BTreeMap::new();
    for value in values {
        by_id.insert(value.user_id().to_string(), value);
    }
    Ok(by_id.into_values().collect())
}

fn removal_ids_match(
    member_ids: &[LegacySpaceMemberIdV1],
    removed_members: &[LegacySpaceMemberRemovalTargetV1],
) -> bool {
    member_ids.len() == removed_members.len()
        && member_ids
            .iter()
            .zip(removed_members)
            .all(|(member_id, target)| member_id == target.member_id())
}

fn resolved_removals_belong_to_submission(
    submitted_ids: &[LegacySpaceMemberIdV1],
    removed_members: &[LegacySpaceMemberRemovalTargetV1],
) -> bool {
    removed_members.iter().all(|target| {
        submitted_ids
            .iter()
            .any(|member_id| member_id.mapped_uuid() == target.member_id().mapped_uuid())
    })
}

#[allow(clippy::too_many_arguments)]
fn removal_postcondition_matches(
    submitted_ids: &[LegacySpaceMemberIdV1],
    result_ids: &[LegacySpaceMemberIdV1],
    discovered: &[LegacySpaceMemberRemovalTargetV1],
    removed: &[LegacySpaceMemberRemovalTargetV1],
    creator_id: UserId,
    matching_before: u32,
    deleted_rows: u32,
    matching_after: u32,
) -> Result<bool, LegacyMembershipAtomicErrorV1> {
    if !resolved_removals_belong_to_submission(submitted_ids, discovered)
        || submitted_ids != result_ids
        || discovered != removed
        || discovered
            .iter()
            .any(|target| target.user_id() == creator_id)
    {
        return Ok(false);
    }
    let unique_ids = discovered
        .iter()
        .map(|target| target.member_id().mapped_uuid())
        .collect::<std::collections::BTreeSet<_>>();
    let expected =
        u32::try_from(unique_ids.len()).map_err(|_| LegacyMembershipAtomicErrorV1::Corrupt)?;
    Ok(matching_before == expected && deleted_rows == expected && matching_after == 0)
}

fn is_canonical_uuid_text(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
        })
}

fn canonical_authority_subjects(
    previous: &[UserId],
    submitted: &[LegacySpaceMemberTargetV1],
    creator_id: UserId,
) -> Vec<UserId> {
    let mut by_id = BTreeMap::new();
    for user_id in previous {
        by_id.insert(user_id.to_string(), *user_id);
    }
    for member in submitted {
        by_id.insert(member.user_id().to_string(), member.user_id());
    }
    by_id.insert(creator_id.to_string(), creator_id);
    by_id.into_values().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyMembershipAtomicOutcomeV1 {
    Applied(LegacyMembershipMutationReceiptV1),
    Replay(LegacyMembershipMutationReceiptV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyMembershipSuccessV1 {
    InviteRemoved,
    SpaceMemberAdded,
    SpaceMembersAdded {
        added: Vec<String>,
        already_members: Vec<String>,
    },
    SpaceMembersRemoved {
        removed_member_ids: Vec<LegacySpaceMemberIdV1>,
    },
    SpaceMemberRemoved,
    SpaceMembersSet {
        count: u32,
    },
}

impl LegacyMembershipSuccessV1 {
    #[must_use]
    pub const fn success(&self) -> bool {
        true
    }

    #[must_use]
    pub fn count(&self) -> Option<u32> {
        match self {
            Self::SpaceMembersSet { count } => Some(*count),
            Self::SpaceMembersAdded { added, .. } => u32::try_from(added.len()).ok(),
            Self::SpaceMembersRemoved { removed_member_ids } => {
                u32::try_from(removed_member_ids.len()).ok()
            }
            Self::InviteRemoved | Self::SpaceMemberAdded | Self::SpaceMemberRemoved => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMembershipExecutionV1 {
    success: LegacyMembershipSuccessV1,
    effects: LegacyMembershipEffectsV1,
    replayed: bool,
}

impl LegacyMembershipExecutionV1 {
    #[must_use]
    pub fn success(&self) -> LegacyMembershipSuccessV1 {
        self.success.clone()
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyMembershipEffectsV1 {
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

impl fmt::Debug for LegacyMembershipExecutionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMembershipExecutionV1")
            .field("success", &self.success)
            .field("effects", &self.effects)
            .field("replayed", &self.replayed)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyMembershipAtomicErrorV1 {
    #[error("membership target was not found")]
    TargetMissing,
    #[error("membership target was not found")]
    AccessDenied,
    #[error("membership target was not found")]
    CrossTenant,
    #[error("membership target was not found")]
    StaleAuthority,
    #[error("membership request conflicts with current state")]
    Conflict,
    #[error("membership request is already in flight")]
    InFlight,
    #[error("membership authority is unavailable")]
    Unavailable,
    #[error("membership authority returned invalid state")]
    Corrupt,
}

/// Provider-free atomic boundary for the six exact actions in this module.
///
/// An implementation must use one D1 transaction/batch and, before committing:
///
/// 1. assert and consume the one-use browser grant for the command actor;
/// 2. reassert the session actor is active/undeleted, the session is live, the
///    trusted organization is active, and the actor is its owner or has an
///    active organization membership;
/// 3. for invite removal, require active-organization owner/admin authority and
///    require the invite row itself to belong to that organization;
/// 4. for space actions, require an undeleted space in the active organization
///    and require an organization owner/admin, the active creator, or a current
///    Frame `manager` space member; contributor/viewer never authorize;
/// 5. require every submitted target and the database-discovered creator to be
///    active/undeleted users who are the organization owner or active members
///    (never silently filter an invalid target or tolerate a dirty graph);
/// 6. after discovering creator context, bind the actor/tenant-scoped key to
///    the canonical effective fingerprint;
/// 7. apply and assert the exact typed mutation/final-state postcondition, with
///    `set` replacing the set, forcing the creator to Frame `manager`, mapping
///    Cap `member` to Frame `viewer`, and never producing `contributor`;
/// 8. bump authority generation and revoke outstanding mutation grants for
///    every database-derived affected subject, then persist those exact
///    postconditions, cache invalidations, business audit, and replay journal
///    atomically.
///
/// Same-key/same-fingerprint retries return `Replay` without generating new
/// member IDs or rerunning writes. Same-key/different-fingerprint requests
/// return `Conflict`. This source contract does not promote a production route.
#[async_trait]
pub trait LegacyMembershipAtomicPortV1: Send + Sync {
    async fn execute_atomic(
        &self,
        command: &LegacyMembershipCommandV1,
        browser_fence: &LegacyMembershipBrowserFenceV1,
    ) -> Result<LegacyMembershipAtomicOutcomeV1, LegacyMembershipAtomicErrorV1>;
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyMembershipErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid input")]
    Invalid,
    #[error("An idempotency key is required")]
    IdempotencyRequired,
    #[error("Membership target not found")]
    TargetNotFound,
    #[error("Membership request conflicts with current state")]
    Conflict,
    #[error("Membership authority is unavailable")]
    AuthorityUnavailable,
    #[error("Membership action failed")]
    Internal,
}

impl fmt::Debug for LegacyMembershipErrorV1 {
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
pub struct LegacyMembershipAdapterV1 {
    action: LegacyMembershipActionV1,
}

impl LegacyMembershipAdapterV1 {
    #[must_use]
    pub const fn remove_organization_invite() -> Self {
        Self {
            action: LegacyMembershipActionV1::RemoveOrganizationInvite,
        }
    }

    #[must_use]
    pub const fn add_space_member() -> Self {
        Self {
            action: LegacyMembershipActionV1::AddSpaceMember,
        }
    }

    #[must_use]
    pub const fn add_space_members() -> Self {
        Self {
            action: LegacyMembershipActionV1::AddSpaceMembers,
        }
    }

    #[must_use]
    pub const fn batch_remove_space_members() -> Self {
        Self {
            action: LegacyMembershipActionV1::BatchRemoveSpaceMembers,
        }
    }

    #[must_use]
    pub const fn remove_space_member() -> Self {
        Self {
            action: LegacyMembershipActionV1::RemoveSpaceMember,
        }
    }

    #[must_use]
    pub const fn set_space_members() -> Self {
        Self {
            action: LegacyMembershipActionV1::SetSpaceMembers,
        }
    }

    #[must_use]
    pub const fn action(self) -> LegacyMembershipActionV1 {
        self.action
    }

    pub fn prepare(
        &self,
        request: &LegacyMembershipRequestV1,
    ) -> Result<LegacyMembershipCommandV1, LegacyMembershipErrorV1> {
        if request.input.action() != self.action {
            return Err(LegacyMembershipErrorV1::Invalid);
        }
        if request.credential != Some(LegacyMembershipCredentialV1::Session) {
            return Err(LegacyMembershipErrorV1::Unauthorized);
        }
        let actor_id = request
            .actor_id
            .ok_or(LegacyMembershipErrorV1::Unauthorized)?;
        let active_organization_id = request
            .active_organization_id
            .ok_or(LegacyMembershipErrorV1::Unauthorized)?;
        let idempotency_key = request
            .idempotency_key
            .as_ref()
            .ok_or(LegacyMembershipErrorV1::IdempotencyRequired)
            .and_then(|value| {
                IdempotencyKey::parse(value.clone()).map_err(|_| LegacyMembershipErrorV1::Invalid)
            })?;
        let authority = LegacyMembershipAuthorityV1 {
            actor_id,
            active_organization_id,
        };

        match &request.input {
            LegacyMembershipInputV1::RemoveOrganizationInvite {
                legacy_invite_id,
                legacy_organization_id,
            } => {
                let organization_id = map_organization(legacy_organization_id)?;
                if organization_id != active_organization_id {
                    return Err(LegacyMembershipErrorV1::TargetNotFound);
                }
                let invite_id = map_invite(legacy_invite_id)?;
                Ok(LegacyMembershipCommandV1::RemoveOrganizationInvite {
                    fence: LegacyMembershipFenceV1 {
                        authority,
                        idempotency_key,
                    },
                    organization_id,
                    invite_id,
                })
            }
            LegacyMembershipInputV1::AddSpaceMember {
                legacy_space_id,
                legacy_user_id,
                role,
            } => {
                let space_id = map_space(legacy_space_id)?;
                let target = LegacySpaceMemberTargetV1 {
                    user_id: map_user(legacy_user_id)?,
                    role: LegacySpaceMemberRoleV1::parse_source(role)
                        .ok_or(LegacyMembershipErrorV1::Invalid)?,
                };
                Ok(LegacyMembershipCommandV1::AddSpaceMember {
                    fence: LegacyMembershipFenceV1 {
                        authority,
                        idempotency_key,
                    },
                    space_id,
                    legacy_user_id: legacy_user_id.clone(),
                    target,
                })
            }
            LegacyMembershipInputV1::AddSpaceMembers {
                legacy_space_id,
                legacy_user_ids,
                role,
            } => {
                if legacy_user_ids.len() > MAX_LEGACY_MEMBERSHIP_TARGETS {
                    return Err(LegacyMembershipErrorV1::Invalid);
                }
                let role = LegacySpaceMemberRoleV1::parse_source(role)
                    .ok_or(LegacyMembershipErrorV1::Invalid)?;
                let submitted_members = legacy_user_ids
                    .iter()
                    .map(|legacy_user_id| {
                        map_user(legacy_user_id)
                            .map(|user_id| LegacySpaceMemberTargetV1 { user_id, role })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(LegacyMembershipCommandV1::AddSpaceMembers {
                    fence: LegacyMembershipFenceV1 {
                        authority,
                        idempotency_key,
                    },
                    space_id: map_space(legacy_space_id)?,
                    legacy_user_ids: legacy_user_ids.clone(),
                    submitted_members,
                })
            }
            LegacyMembershipInputV1::BatchRemoveSpaceMembers { legacy_member_ids } => {
                if legacy_member_ids.len() > MAX_LEGACY_MEMBERSHIP_TARGETS {
                    return Err(LegacyMembershipErrorV1::Invalid);
                }
                let member_ids = legacy_member_ids
                    .iter()
                    .map(|value| map_space_member_id(value))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(LegacyMembershipCommandV1::BatchRemoveSpaceMembers {
                    fence: LegacyMembershipFenceV1 {
                        authority,
                        idempotency_key,
                    },
                    member_ids,
                })
            }
            LegacyMembershipInputV1::RemoveSpaceMember { legacy_member_id } => {
                Ok(LegacyMembershipCommandV1::RemoveSpaceMember {
                    fence: LegacyMembershipFenceV1 {
                        authority,
                        idempotency_key,
                    },
                    member_id: map_space_member_id(legacy_member_id)?,
                })
            }
            LegacyMembershipInputV1::SetSpaceMembers {
                legacy_space_id,
                legacy_user_ids,
                role,
                members,
            } => {
                if legacy_user_ids.len() > MAX_LEGACY_MEMBERSHIP_TARGETS
                    || members
                        .as_ref()
                        .is_some_and(|members| members.len() > MAX_LEGACY_MEMBERSHIP_TARGETS)
                {
                    return Err(LegacyMembershipErrorV1::Invalid);
                }
                let space_id = map_space(legacy_space_id)?;
                let submitted = canonical_submitted_members(
                    legacy_user_ids,
                    role.as_deref().unwrap_or("member"),
                    members.as_deref(),
                )?;
                let (legacy_user_ids, submitted_members): (Vec<_>, Vec<_>) =
                    submitted.into_iter().unzip();
                Ok(LegacyMembershipCommandV1::SetSpaceMembers {
                    fence: LegacyMembershipFenceV1 {
                        authority,
                        idempotency_key,
                    },
                    space_id,
                    legacy_user_ids,
                    submitted_members,
                })
            }
        }
    }

    pub async fn execute<P>(
        &self,
        port: &P,
        request: &LegacyMembershipRequestV1,
        proof: &ValidatedBrowserMutationProof,
    ) -> Result<LegacyMembershipExecutionV1, LegacyMembershipErrorV1>
    where
        P: LegacyMembershipAtomicPortV1,
    {
        let browser_fence = LegacyMembershipBrowserFenceV1::from_validated_proof(proof);
        self.execute_with_fence(port, request, &browser_fence).await
    }

    async fn execute_with_fence<P>(
        &self,
        port: &P,
        request: &LegacyMembershipRequestV1,
        browser_fence: &LegacyMembershipBrowserFenceV1,
    ) -> Result<LegacyMembershipExecutionV1, LegacyMembershipErrorV1>
    where
        P: LegacyMembershipAtomicPortV1,
    {
        if request.credential != Some(LegacyMembershipCredentialV1::Session)
            || request.actor_id != Some(browser_fence.actor_id())
        {
            return Err(LegacyMembershipErrorV1::Unauthorized);
        }
        let command = self.prepare(request)?;
        let (receipt, replayed) = match port.execute_atomic(&command, browser_fence).await {
            Ok(LegacyMembershipAtomicOutcomeV1::Applied(receipt)) => (receipt, false),
            Ok(LegacyMembershipAtomicOutcomeV1::Replay(receipt)) => (receipt, true),
            Err(error) => return Err(map_atomic_error(error)),
        };
        if !receipt.matches_command(&command) {
            return Err(LegacyMembershipErrorV1::Internal);
        }
        let success = project_success(&command, receipt.result())?;
        Ok(LegacyMembershipExecutionV1 {
            success,
            effects: receipt.effects().clone(),
            replayed,
        })
    }
}

fn canonical_submitted_members(
    legacy_user_ids: &[String],
    fallback_role: &str,
    members: Option<&[LegacySubmittedSpaceMemberV1]>,
) -> Result<Vec<(String, LegacySpaceMemberTargetV1)>, LegacyMembershipErrorV1> {
    // Zod validates both `userIds` and `members`; when `members` exists, Cap
    // ignores `userIds` for the mutation. The Rust request already guarantees
    // both containers contain strings, so only the selected branch is mapped.
    // The fallback `role` field is part of the Zod object and is validated even
    // when the optional `members` array wins for mutation semantics.
    let fallback_role = LegacySpaceMemberRoleV1::parse_source(fallback_role)
        .ok_or(LegacyMembershipErrorV1::Invalid)?;
    let mut by_user = BTreeMap::<String, (String, LegacySpaceMemberTargetV1)>::new();
    if let Some(members) = members {
        for member in members {
            let user_id = map_user(&member.legacy_user_id)?;
            let role = LegacySpaceMemberRoleV1::parse_source(&member.role)
                .ok_or(LegacyMembershipErrorV1::Invalid)?;
            // Cap's Map preserves first insertion position but the last role.
            // Output only exposes a count, so sorting by canonical UUID retains
            // the semantic last-role winner and makes the fingerprint stable.
            by_user.insert(
                user_id.to_string(),
                (
                    member.legacy_user_id.clone(),
                    LegacySpaceMemberTargetV1 { user_id, role },
                ),
            );
        }
    } else {
        for legacy_user_id in legacy_user_ids {
            let user_id = map_user(legacy_user_id)?;
            by_user.insert(
                user_id.to_string(),
                (
                    legacy_user_id.clone(),
                    LegacySpaceMemberTargetV1 {
                        user_id,
                        role: fallback_role,
                    },
                ),
            );
        }
    }
    Ok(by_user.into_values().collect())
}

fn map_legacy_uuid(value: &str) -> Result<String, LegacyMembershipErrorV1> {
    LegacyCapNanoId::parse(value.to_owned())
        .map(|legacy| legacy.mapped_uuid().to_string())
        .map_err(|_| LegacyMembershipErrorV1::TargetNotFound)
}

fn map_organization(value: &str) -> Result<OrganizationId, LegacyMembershipErrorV1> {
    OrganizationId::parse(&map_legacy_uuid(value)?)
        .map_err(|_| LegacyMembershipErrorV1::TargetNotFound)
}

fn map_invite(value: &str) -> Result<OrganizationInviteId, LegacyMembershipErrorV1> {
    OrganizationInviteId::parse(&map_legacy_uuid(value)?)
        .map_err(|_| LegacyMembershipErrorV1::TargetNotFound)
}

fn map_space(value: &str) -> Result<SpaceId, LegacyMembershipErrorV1> {
    SpaceId::parse(&map_legacy_uuid(value)?).map_err(|_| LegacyMembershipErrorV1::TargetNotFound)
}

fn map_user(value: &str) -> Result<UserId, LegacyMembershipErrorV1> {
    UserId::parse(&map_legacy_uuid(value)?).map_err(|_| LegacyMembershipErrorV1::TargetNotFound)
}

fn map_space_member_id(value: &str) -> Result<LegacySpaceMemberIdV1, LegacyMembershipErrorV1> {
    LegacySpaceMemberIdV1::from_legacy(value.to_owned())
}

fn fingerprint_remove_invite(
    actor_id: UserId,
    organization_id: OrganizationId,
    invite_id: OrganizationInviteId,
) -> [u8; 32] {
    let mut digest = fingerprint_prefix(
        LegacyMembershipActionV1::RemoveOrganizationInvite,
        actor_id,
        organization_id,
    );
    digest.update(invite_id.as_uuid().as_bytes());
    digest.finalize().into()
}

fn fingerprint_space_members(
    action: LegacyMembershipActionV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    space_id: SpaceId,
    members: &[LegacySpaceMemberTargetV1],
) -> [u8; 32] {
    let mut digest = fingerprint_prefix(action, actor_id, organization_id);
    digest.update(space_id.as_uuid().as_bytes());
    digest.update((members.len() as u64).to_be_bytes());
    for member in members {
        digest.update(member.user_id().as_uuid().as_bytes());
        digest.update([match member.role() {
            LegacySpaceMemberRoleV1::Admin => 1,
            LegacySpaceMemberRoleV1::Member => 2,
        }]);
    }
    digest.finalize().into()
}

fn fingerprint_member_ids(
    action: LegacyMembershipActionV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    member_ids: &[LegacySpaceMemberIdV1],
) -> [u8; 32] {
    let mut digest = fingerprint_prefix(action, actor_id, organization_id);
    digest.update((member_ids.len() as u64).to_be_bytes());
    for member_id in member_ids {
        digest.update(member_id.legacy_id().len().to_be_bytes());
        digest.update(member_id.legacy_id().as_bytes());
    }
    digest.finalize().into()
}

fn fingerprint_prefix(
    action: LegacyMembershipActionV1,
    actor_id: UserId,
    organization_id: OrganizationId,
) -> Sha256 {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-membership-actions-v1\0");
    digest.update(action.stable_code().as_bytes());
    digest.update([0]);
    digest.update(actor_id.as_uuid().as_bytes());
    digest.update(organization_id.as_uuid().as_bytes());
    digest
}

fn project_success(
    command: &LegacyMembershipCommandV1,
    result: LegacyMembershipMutationResultV1,
) -> Result<LegacyMembershipSuccessV1, LegacyMembershipErrorV1> {
    match (command, result) {
        (
            LegacyMembershipCommandV1::RemoveOrganizationInvite { .. },
            LegacyMembershipMutationResultV1::InviteRemoved,
        ) => Ok(LegacyMembershipSuccessV1::InviteRemoved),
        (
            LegacyMembershipCommandV1::AddSpaceMember { .. },
            LegacyMembershipMutationResultV1::SpaceMemberAdded,
        ) => Ok(LegacyMembershipSuccessV1::SpaceMemberAdded),
        (
            LegacyMembershipCommandV1::AddSpaceMembers { .. },
            LegacyMembershipMutationResultV1::SpaceMembersAdded {
                added,
                already_members,
            },
        ) => Ok(LegacyMembershipSuccessV1::SpaceMembersAdded {
            added,
            already_members,
        }),
        (
            LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. },
            LegacyMembershipMutationResultV1::SpaceMembersRemoved { removed_member_ids },
        ) => Ok(LegacyMembershipSuccessV1::SpaceMembersRemoved { removed_member_ids }),
        (
            LegacyMembershipCommandV1::RemoveSpaceMember { .. },
            LegacyMembershipMutationResultV1::SpaceMemberRemoved,
        ) => Ok(LegacyMembershipSuccessV1::SpaceMemberRemoved),
        (
            LegacyMembershipCommandV1::SetSpaceMembers { .. },
            LegacyMembershipMutationResultV1::SpaceMembersSet { count },
        ) => Ok(LegacyMembershipSuccessV1::SpaceMembersSet { count }),
        _ => Err(LegacyMembershipErrorV1::Internal),
    }
}

fn map_atomic_error(error: LegacyMembershipAtomicErrorV1) -> LegacyMembershipErrorV1 {
    match error {
        LegacyMembershipAtomicErrorV1::TargetMissing
        | LegacyMembershipAtomicErrorV1::AccessDenied
        | LegacyMembershipAtomicErrorV1::CrossTenant
        | LegacyMembershipAtomicErrorV1::StaleAuthority => LegacyMembershipErrorV1::TargetNotFound,
        LegacyMembershipAtomicErrorV1::Conflict | LegacyMembershipAtomicErrorV1::InFlight => {
            LegacyMembershipErrorV1::Conflict
        }
        LegacyMembershipAtomicErrorV1::Unavailable => LegacyMembershipErrorV1::AuthorityUnavailable,
        LegacyMembershipAtomicErrorV1::Corrupt => LegacyMembershipErrorV1::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const ORGANIZATION: &str = "0123456789abcde";
    const OTHER_ORGANIZATION: &str = "0123456789abcdf";
    const SPACE: &str = "1123456789abcde";
    const INVITE: &str = "2123456789abcde";
    const USER_ONE: &str = "3123456789abcde";
    const USER_TWO: &str = "4123456789abcde";
    const USER_THREE: &str = "5123456789abcde";
    const MEMBER_ONE: &str = "6123456789abcde";
    const MEMBER_TWO: &str = "7123456789abcde";
    const MEMBER_THREE: &str = "8123456789abcde";

    fn mapped<T, E: fmt::Debug>(value: &str, parse: impl FnOnce(&str) -> Result<T, E>) -> T {
        let value = LegacyCapNanoId::parse(value)
            .expect("legacy id")
            .mapped_uuid()
            .to_string();
        parse(&value).expect("mapped id")
    }

    fn actor() -> UserId {
        mapped(USER_ONE, UserId::parse)
    }

    fn other_actor() -> UserId {
        mapped(USER_THREE, UserId::parse)
    }

    fn organization() -> OrganizationId {
        mapped(ORGANIZATION, OrganizationId::parse)
    }

    fn space() -> SpaceId {
        mapped(SPACE, SpaceId::parse)
    }

    fn invite() -> OrganizationInviteId {
        mapped(INVITE, OrganizationInviteId::parse)
    }

    fn member_id(value: &str) -> LegacySpaceMemberIdV1 {
        LegacySpaceMemberIdV1::from_legacy(value.to_owned()).expect("member id")
    }

    fn request(input: LegacyMembershipInputV1) -> LegacyMembershipRequestV1 {
        LegacyMembershipRequestV1 {
            credential: Some(LegacyMembershipCredentialV1::Session),
            actor_id: Some(actor()),
            active_organization_id: Some(organization()),
            idempotency_key: Some("membership-action-0001".into()),
            input,
        }
    }

    fn remove_invite_request() -> LegacyMembershipRequestV1 {
        request(LegacyMembershipInputV1::RemoveOrganizationInvite {
            legacy_invite_id: INVITE.into(),
            legacy_organization_id: ORGANIZATION.into(),
        })
    }

    fn add_request(role: &str) -> LegacyMembershipRequestV1 {
        request(LegacyMembershipInputV1::AddSpaceMember {
            legacy_space_id: SPACE.into(),
            legacy_user_id: USER_TWO.into(),
            role: role.into(),
        })
    }

    fn bulk_add_request(user_ids: Vec<&str>, role: &str) -> LegacyMembershipRequestV1 {
        request(LegacyMembershipInputV1::AddSpaceMembers {
            legacy_space_id: SPACE.into(),
            legacy_user_ids: user_ids.into_iter().map(str::to_owned).collect(),
            role: role.into(),
        })
    }

    fn batch_remove_request(member_ids: Vec<&str>) -> LegacyMembershipRequestV1 {
        request(LegacyMembershipInputV1::BatchRemoveSpaceMembers {
            legacy_member_ids: member_ids.into_iter().map(str::to_owned).collect(),
        })
    }

    fn remove_member_request(member_id: &str) -> LegacyMembershipRequestV1 {
        request(LegacyMembershipInputV1::RemoveSpaceMember {
            legacy_member_id: member_id.into(),
        })
    }

    fn set_request(
        user_ids: Vec<&str>,
        role: Option<&str>,
        members: Option<Vec<(&str, &str)>>,
    ) -> LegacyMembershipRequestV1 {
        request(LegacyMembershipInputV1::SetSpaceMembers {
            legacy_space_id: SPACE.into(),
            legacy_user_ids: user_ids.into_iter().map(str::to_owned).collect(),
            role: role.map(str::to_owned),
            members: members.map(|members| {
                members
                    .into_iter()
                    .map(|(user, role)| LegacySubmittedSpaceMemberV1 {
                        legacy_user_id: user.into(),
                        role: role.into(),
                    })
                    .collect()
            }),
        })
    }

    fn invite_context() -> LegacyMembershipDiscoveredContextV1 {
        LegacyMembershipDiscoveredContextV1::OrganizationInvite {
            organization_id: organization(),
            invite_id: invite(),
        }
    }

    fn add_context() -> LegacyMembershipDiscoveredContextV1 {
        LegacyMembershipDiscoveredContextV1::SpaceAdd {
            organization_id: organization(),
            space_id: space(),
            creator_id: actor(),
        }
    }

    fn bulk_add_context(
        previous_legacy_user_ids: Vec<&str>,
    ) -> LegacyMembershipDiscoveredContextV1 {
        LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
            organization_id: organization(),
            space_id: space(),
            creator_id: actor(),
            previous_members: previous_legacy_user_ids
                .into_iter()
                .map(|legacy_user_id| {
                    LegacySpaceMemberUserAliasV1::from_verified_database_row(
                        legacy_user_id.to_owned(),
                        mapped(legacy_user_id, UserId::parse),
                    )
                    .expect("user alias")
                })
                .collect(),
        }
    }

    fn removal_context(targets: Vec<(&str, &str)>) -> LegacyMembershipDiscoveredContextV1 {
        LegacyMembershipDiscoveredContextV1::SpaceRemoval {
            organization_id: organization(),
            space_id: space(),
            creator_id: actor(),
            removed_members: targets
                .into_iter()
                .map(|(member, user)| {
                    LegacySpaceMemberRemovalTargetV1::from_verified_database_row(
                        member_id(member),
                        mapped(user, UserId::parse),
                    )
                })
                .collect(),
        }
    }

    fn replacement_context(
        previous_member_ids: Vec<UserId>,
    ) -> LegacyMembershipDiscoveredContextV1 {
        LegacyMembershipDiscoveredContextV1::SpaceReplacement {
            organization_id: organization(),
            space_id: space(),
            creator_id: actor(),
            previous_member_ids,
        }
    }

    fn final_database_members(
        members: &[LegacySpaceMemberTargetV1],
    ) -> Vec<LegacySpaceMemberTargetV1> {
        members
            .iter()
            .map(|member| {
                LegacySpaceMemberTargetV1::from_database_role(
                    member.user_id(),
                    member.role().frame_role(),
                )
                .expect("database role")
            })
            .collect()
    }

    fn exact_mutation_postcondition(
        command: &LegacyMembershipCommandV1,
        context: &LegacyMembershipDiscoveredContextV1,
    ) -> (
        LegacyMembershipMutationResultV1,
        LegacyMembershipMutationPostconditionV1,
    ) {
        match (command, context) {
            (
                LegacyMembershipCommandV1::RemoveOrganizationInvite { .. },
                LegacyMembershipDiscoveredContextV1::OrganizationInvite { .. },
            ) => (
                LegacyMembershipMutationResultV1::InviteRemoved,
                LegacyMembershipMutationPostconditionV1::OrganizationInviteRemoved {
                    matching_before: 1,
                    deleted_rows: 1,
                    matching_after: 0,
                },
            ),
            (
                LegacyMembershipCommandV1::AddSpaceMember { target, .. },
                LegacyMembershipDiscoveredContextV1::SpaceAdd { .. },
            ) => (
                LegacyMembershipMutationResultV1::SpaceMemberAdded,
                LegacyMembershipMutationPostconditionV1::SpaceMemberInserted {
                    matching_before: 0,
                    inserted_rows: 1,
                    matching_after: 1,
                    final_member: LegacySpaceMemberTargetV1::from_database_role(
                        target.user_id(),
                        target.role().frame_role(),
                    )
                    .expect("database role"),
                },
            ),
            (
                LegacyMembershipCommandV1::AddSpaceMembers {
                    legacy_user_ids,
                    submitted_members,
                    ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                    previous_members, ..
                },
            ) => {
                let previous = previous_members
                    .iter()
                    .map(|member| member.user_id().to_string())
                    .collect::<std::collections::BTreeSet<_>>();
                let added_pairs = legacy_user_ids
                    .iter()
                    .zip(submitted_members)
                    .filter(|(_, target)| !previous.contains(&target.user_id().to_string()))
                    .collect::<Vec<_>>();
                let added = added_pairs
                    .iter()
                    .map(|(legacy_user_id, _)| (*legacy_user_id).clone())
                    .collect::<Vec<_>>();
                let already_members = if added.is_empty() {
                    legacy_user_ids.clone()
                } else {
                    previous_members
                        .iter()
                        .map(|member| member.legacy_user_id().to_owned())
                        .collect()
                };
                let matching_before = u32::try_from(previous_members.len()).expect("before");
                let inserted_rows = u32::try_from(added.len()).expect("inserted");
                (
                    LegacyMembershipMutationResultV1::SpaceMembersAdded {
                        added,
                        already_members: already_members.clone(),
                    },
                    LegacyMembershipMutationPostconditionV1::SpaceMembersInserted {
                        matching_before,
                        inserted_rows,
                        matching_after: matching_before + inserted_rows,
                        added_members: added_pairs.iter().map(|(_, target)| **target).collect(),
                        already_member_ids: already_members,
                    },
                )
            }
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { member_ids, .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    removed_members, ..
                },
            ) => {
                let count = u32::try_from(
                    removed_members
                        .iter()
                        .map(|target| target.member_id().mapped_uuid())
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                )
                .expect("removed count");
                (
                    LegacyMembershipMutationResultV1::SpaceMembersRemoved {
                        removed_member_ids: member_ids.clone(),
                    },
                    LegacyMembershipMutationPostconditionV1::SpaceMembersRemoved {
                        matching_before: count,
                        deleted_rows: count,
                        matching_after: 0,
                        removed_members: removed_members.clone(),
                    },
                )
            }
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop { .. },
            ) => (
                LegacyMembershipMutationResultV1::SpaceMembersRemoved {
                    removed_member_ids: Vec::new(),
                },
                LegacyMembershipMutationPostconditionV1::SpaceMembersRemoved {
                    matching_before: 0,
                    deleted_rows: 0,
                    matching_after: 0,
                    removed_members: Vec::new(),
                },
            ),
            (
                LegacyMembershipCommandV1::RemoveSpaceMember { .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    removed_members, ..
                },
            ) => (
                LegacyMembershipMutationResultV1::SpaceMemberRemoved,
                LegacyMembershipMutationPostconditionV1::SpaceMembersRemoved {
                    matching_before: 1,
                    deleted_rows: 1,
                    matching_after: 0,
                    removed_members: removed_members.clone(),
                },
            ),
            (
                LegacyMembershipCommandV1::SetSpaceMembers {
                    submitted_members, ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                    creator_id,
                    previous_member_ids,
                    ..
                },
            ) => {
                let final_members = creator_inclusive_members(submitted_members, *creator_id);
                let previous = u32::try_from(previous_member_ids.len()).expect("previous count");
                let count = u32::try_from(final_members.len()).expect("final count");
                (
                    LegacyMembershipMutationResultV1::SpaceMembersSet { count },
                    LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced {
                        matching_before: previous,
                        deleted_rows: previous,
                        inserted_rows: count,
                        matching_after: count,
                        final_members: final_database_members(&final_members),
                    },
                )
            }
            _ => panic!("matching command context"),
        }
    }

    fn exact_authority_postcondition(
        command: &LegacyMembershipCommandV1,
        context: &LegacyMembershipDiscoveredContextV1,
    ) -> LegacyMembershipAuthorityPostconditionV1 {
        let active_targets = match command {
            LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. }
            | LegacyMembershipCommandV1::RemoveSpaceMember { .. } => context
                .removed_members()
                .iter()
                .map(LegacySpaceMemberRemovalTargetV1::user_id)
                .collect::<Vec<_>>(),
            _ => command
                .submitted_members()
                .iter()
                .map(|member| member.user_id())
                .collect::<Vec<_>>(),
        };
        let affected = match (command, context) {
            (LegacyMembershipCommandV1::RemoveOrganizationInvite { .. }, _) => Vec::new(),
            (LegacyMembershipCommandV1::AddSpaceMember { target, .. }, _) => {
                vec![target.user_id()]
            }
            (
                LegacyMembershipCommandV1::AddSpaceMembers {
                    submitted_members, ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceBulkAdd {
                    previous_members, ..
                },
            ) => {
                let previous = previous_members
                    .iter()
                    .map(|member| member.user_id().to_string())
                    .collect::<std::collections::BTreeSet<_>>();
                submitted_members
                    .iter()
                    .map(|member| member.user_id())
                    .filter(|user_id| !previous.contains(&user_id.to_string()))
                    .collect()
            }
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. }
                | LegacyMembershipCommandV1::RemoveSpaceMember { .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemoval {
                    removed_members, ..
                },
            ) => removed_members
                .iter()
                .map(LegacySpaceMemberRemovalTargetV1::user_id)
                .collect(),
            (
                LegacyMembershipCommandV1::BatchRemoveSpaceMembers { .. },
                LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop { .. },
            ) => Vec::new(),
            (
                LegacyMembershipCommandV1::SetSpaceMembers {
                    submitted_members, ..
                },
                LegacyMembershipDiscoveredContextV1::SpaceReplacement {
                    creator_id,
                    previous_member_ids,
                    ..
                },
            ) => canonical_authority_subjects(previous_member_ids, submitted_members, *creator_id),
            _ => panic!("matching command context"),
        };
        LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
            organization(),
            command.fence().authority().actor_id(),
            LegacyMembershipActorAuthorityV1::OrganizationOwner,
            context.space_id(),
            active_targets,
            context.creator_id(),
            affected.clone(),
            affected,
        )
        .expect("authority postcondition")
    }

    fn exact_receipt(
        command: &LegacyMembershipCommandV1,
        context: LegacyMembershipDiscoveredContextV1,
    ) -> LegacyMembershipMutationReceiptV1 {
        let (result, mutation) = exact_mutation_postcondition(command, &context);
        let authority = exact_authority_postcondition(command, &context);
        LegacyMembershipMutationReceiptV1::new(command, result, context, mutation, authority)
            .expect("exact receipt")
    }

    fn fingerprint(
        command: &LegacyMembershipCommandV1,
        context: &LegacyMembershipDiscoveredContextV1,
    ) -> [u8; 32] {
        command
            .request_fingerprint_for_context(context)
            .expect("fingerprint")
    }

    #[test]
    fn profiles_pin_exact_provider_free_source_closures() {
        for profile in [
            LEGACY_REMOVE_ORGANIZATION_INVITE_PROFILE,
            LEGACY_ADD_SPACE_MEMBER_PROFILE,
            LEGACY_SET_SPACE_MEMBERS_PROFILE,
        ] {
            assert_eq!(profile.pinned_commit, LEGACY_MEMBERSHIP_CAP_COMMIT);
            assert!(!profile.sources.is_empty());
            assert!(
                profile
                    .sources
                    .iter()
                    .all(|source| source.sha256.len() == 64)
            );
            assert_eq!(profile.provider_effect, None);
            assert!(profile.tenant_non_disclosure);
            assert_eq!(profile.kind, "server_action");
            assert_eq!(profile.method, "ACTION");
            assert_eq!(profile.source_manifest_sha256.len(), 64);
            assert_eq!(profile.observed_idempotency, "none");
            assert_eq!(profile.idempotency, "required");
            assert!(!profile.source_declared_failure_messages.is_empty());
            assert!(!profile.observed_failure_messages.is_empty());
            assert_eq!(
                profile.required_public_failure_messages,
                LEGACY_MEMBERSHIP_REQUIRED_PUBLIC_FAILURES
            );
            assert_eq!(profile.protected_gates, ["released_legacy_client_e2e"]);
            assert!(!profile.production_promoted);
            assert_eq!(
                profile.required_retry.atomicity,
                LegacyMembershipRequiredAtomicityV1::BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction
            );
        }
        for (profile, operation_id) in [
            (
                LEGACY_ADD_SPACE_MEMBERS_PROFILE,
                LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID,
            ),
            (
                LEGACY_BATCH_REMOVE_SPACE_MEMBERS_PROFILE,
                LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID,
            ),
            (
                LEGACY_REMOVE_SPACE_MEMBER_PROFILE,
                LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID,
            ),
        ] {
            assert_eq!(profile.operation_id, operation_id);
            assert_eq!(profile.pinned_commit, LEGACY_MEMBERSHIP_CAP_COMMIT);
            assert!(!profile.sources.is_empty());
            assert!(
                profile
                    .sources
                    .iter()
                    .all(|source| source.sha256.len() == 64)
            );
            assert_eq!(profile.provider_effect, None);
            assert!(profile.tenant_non_disclosure);
            assert_eq!(profile.kind, "server_action");
            assert_eq!(profile.method, "ACTION");
            assert_eq!(profile.source_manifest_sha256.len(), 64);
            assert_eq!(profile.observed_idempotency, "none");
            assert_eq!(profile.idempotency, "required");
            assert!(profile.protected_gates.is_empty());
            assert!(profile.production_promoted);
            assert_eq!(
                profile.required_retry.atomicity,
                LegacyMembershipRequiredAtomicityV1::BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction
            );
        }
        assert_eq!(
            LEGACY_REMOVE_ORGANIZATION_INVITE_PROFILE.sources[0].sha256,
            "614aed36f22c5187b7ac27d0367b6c5467da1a87f30d83ea2b05582f14d7a5b0"
        );
        assert_eq!(
            LEGACY_ADD_SPACE_MEMBER_PROFILE.sources[0].sha256,
            "e8d738b63989d18c47cad13309de6728080df7a943b53b10fd45f19c05420745"
        );
        assert_eq!(
            LEGACY_REMOVE_ORGANIZATION_INVITE_PROFILE.observed_failure_messages,
            LEGACY_REMOVE_INVITE_REACHABLE_FAILURES
        );
        assert_eq!(
            LEGACY_REMOVE_ORGANIZATION_INVITE_PROFILE
                .source_declared_but_unreachable_failure_messages,
            ["Only admins and owners can manage organization settings"]
        );
        assert!(
            LEGACY_REMOVE_ORGANIZATION_INVITE_PROFILE
                .observed_failure_messages
                .contains(&"unprojected database-driver error")
        );
        assert_eq!(
            LEGACY_SET_SPACE_MEMBERS_PROFILE.input_presence,
            LEGACY_SET_SPACE_MEMBERS_INPUT_PRESENCE
        );
    }

    #[test]
    fn observed_and_required_contracts_do_not_conflate_legacy_defects() {
        assert_eq!(
            LEGACY_SET_SPACE_MEMBERS_PROFILE.observed_mutation,
            LegacyMembershipObservedMutationV1::DeleteAllThenInsertCreatorInclusiveReplacementWithoutTransaction
        );
        assert_eq!(
            LEGACY_SET_SPACE_MEMBERS_PROFILE.required_mutation,
            LegacyMembershipRequiredMutationV1::ReplaceMembershipSetAndForceCreatorAdmin
        );
        assert_eq!(
            LEGACY_ADD_SPACE_MEMBER_PROFILE.observed_retry_defect,
            LegacyMembershipObservedRetryDefectV1::DuplicateInsertHitsUniqueConstraint
        );
    }

    #[test]
    fn session_active_tenant_and_idempotency_are_mandatory() {
        let adapter = LegacyMembershipAdapterV1::remove_organization_invite();
        let mut candidate = remove_invite_request();
        candidate.credential = Some(LegacyMembershipCredentialV1::ApiKey);
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyMembershipErrorV1::Unauthorized)
        );

        let mut candidate = remove_invite_request();
        candidate.active_organization_id = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyMembershipErrorV1::Unauthorized)
        );

        let mut candidate = remove_invite_request();
        candidate.idempotency_key = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyMembershipErrorV1::IdempotencyRequired)
        );

        let mut candidate = remove_invite_request();
        candidate.idempotency_key = Some("short".into());
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyMembershipErrorV1::Invalid)
        );
    }

    #[test]
    fn invite_organization_must_equal_the_trusted_active_tenant() {
        let mut candidate = remove_invite_request();
        let LegacyMembershipInputV1::RemoveOrganizationInvite {
            legacy_organization_id,
            ..
        } = &mut candidate.input
        else {
            unreachable!()
        };
        *legacy_organization_id = OTHER_ORGANIZATION.into();
        assert_eq!(
            LegacyMembershipAdapterV1::remove_organization_invite().prepare(&candidate),
            Err(LegacyMembershipErrorV1::TargetNotFound)
        );
    }

    #[test]
    fn cap_role_normalization_is_exact() {
        for (source, expected) in [
            ("admin", LegacySpaceMemberRoleV1::Admin),
            ("Admin", LegacySpaceMemberRoleV1::Admin),
            ("member", LegacySpaceMemberRoleV1::Member),
        ] {
            let command = LegacyMembershipAdapterV1::add_space_member()
                .prepare(&add_request(source))
                .expect("valid role");
            assert_eq!(command.submitted_members()[0].role(), expected);
        }
        for rejected in ["ADMIN", "Member", "owner", ""] {
            assert_eq!(
                LegacyMembershipAdapterV1::add_space_member().prepare(&add_request(rejected)),
                Err(LegacyMembershipErrorV1::Invalid)
            );
        }
        assert_eq!(
            LegacySpaceMemberRoleV1::Admin.frame_role(),
            SpaceRole::Manager
        );
        assert_eq!(
            LegacySpaceMemberRoleV1::Member.frame_role(),
            SpaceRole::Viewer
        );
        assert_eq!(
            LegacySpaceMemberRoleV1::from_frame_role(SpaceRole::Contributor),
            None
        );
        assert_eq!(
            LegacyMembershipActorAuthorityV1::from_frame_space_role(SpaceRole::Manager),
            Some(LegacyMembershipActorAuthorityV1::SpaceManager)
        );
        assert_eq!(
            LegacyMembershipActorAuthorityV1::from_frame_space_role(SpaceRole::Contributor),
            None
        );
        assert_eq!(
            LegacyMembershipActorAuthorityV1::from_frame_space_role(SpaceRole::Viewer),
            None
        );
        assert_eq!(
            LegacySpaceMemberTargetV1::from_database_role(
                mapped(USER_TWO, UserId::parse),
                SpaceRole::Contributor,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn members_array_takes_precedence_and_duplicate_last_role_wins() {
        let first = set_request(
            vec![USER_THREE],
            Some("admin"),
            Some(vec![
                (USER_TWO, "member"),
                (USER_ONE, "member"),
                (USER_TWO, "Admin"),
            ]),
        );
        let command = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&first)
            .expect("command");
        assert_eq!(command.submitted_members().len(), 2);
        let target_two = command
            .submitted_members()
            .iter()
            .find(|candidate| candidate.user_id() == mapped(USER_TWO, UserId::parse))
            .expect("second user");
        assert_eq!(target_two.role(), LegacySpaceMemberRoleV1::Admin);
        assert!(
            command
                .submitted_members()
                .iter()
                .all(|candidate| candidate.user_id() != mapped(USER_THREE, UserId::parse))
        );

        let equivalent = set_request(
            vec![],
            None,
            Some(vec![(USER_TWO, "admin"), (USER_ONE, "member")]),
        );
        let equivalent = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&equivalent)
            .expect("equivalent");
        let context = replacement_context(Vec::new());
        assert_eq!(
            fingerprint(&command, &context),
            fingerprint(&equivalent, &context)
        );

        let invalid_fallback = set_request(vec![], Some("owner"), Some(vec![(USER_TWO, "member")]));
        assert_eq!(
            LegacyMembershipAdapterV1::set_space_members().prepare(&invalid_fallback),
            Err(LegacyMembershipErrorV1::Invalid)
        );
    }

    #[test]
    fn fallback_role_defaults_to_member_and_empty_set_is_valid() {
        let command = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_TWO], None, None))
            .expect("default role");
        assert_eq!(
            command.submitted_members()[0].role(),
            LegacySpaceMemberRoleV1::Member
        );

        let empty = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![], None, None))
            .expect("source permits an empty submitted set");
        assert!(empty.submitted_members().is_empty());
    }

    #[test]
    fn every_raw_target_container_is_bounded() {
        let too_many = vec![USER_TWO.to_owned(); MAX_LEGACY_MEMBERSHIP_TARGETS + 1];
        let overlong_user_ids_request = request(LegacyMembershipInputV1::SetSpaceMembers {
            legacy_space_id: SPACE.into(),
            legacy_user_ids: too_many,
            role: None,
            members: Some(Vec::new()),
        });
        assert_eq!(
            LegacyMembershipAdapterV1::set_space_members().prepare(&overlong_user_ids_request),
            Err(LegacyMembershipErrorV1::Invalid)
        );

        let members = (0..=MAX_LEGACY_MEMBERSHIP_TARGETS)
            .map(|_| LegacySubmittedSpaceMemberV1 {
                legacy_user_id: USER_TWO.into(),
                role: "member".into(),
            })
            .collect();
        let overlong_members_request = request(LegacyMembershipInputV1::SetSpaceMembers {
            legacy_space_id: SPACE.into(),
            legacy_user_ids: Vec::new(),
            role: None,
            members: Some(members),
        });
        assert_eq!(
            LegacyMembershipAdapterV1::set_space_members().prepare(&overlong_members_request),
            Err(LegacyMembershipErrorV1::Invalid)
        );

        let too_many_users = vec![USER_TWO; MAX_LEGACY_MEMBERSHIP_TARGETS + 1];
        assert_eq!(
            LegacyMembershipAdapterV1::add_space_members()
                .prepare(&bulk_add_request(too_many_users, "member")),
            Err(LegacyMembershipErrorV1::Invalid)
        );
        let too_many_members = vec![MEMBER_ONE; MAX_LEGACY_MEMBERSHIP_TARGETS + 1];
        assert_eq!(
            LegacyMembershipAdapterV1::batch_remove_space_members()
                .prepare(&batch_remove_request(too_many_members)),
            Err(LegacyMembershipErrorV1::Invalid)
        );
    }

    #[test]
    fn malformed_and_cross_tenant_targets_are_non_disclosing() {
        let mut candidate = add_request("member");
        let LegacyMembershipInputV1::AddSpaceMember { legacy_user_id, .. } = &mut candidate.input
        else {
            unreachable!()
        };
        *legacy_user_id = "not-a-cap-id".into();
        assert_eq!(
            LegacyMembershipAdapterV1::add_space_member().prepare(&candidate),
            Err(LegacyMembershipErrorV1::TargetNotFound)
        );
        for error in [
            LegacyMembershipAtomicErrorV1::TargetMissing,
            LegacyMembershipAtomicErrorV1::AccessDenied,
            LegacyMembershipAtomicErrorV1::CrossTenant,
            LegacyMembershipAtomicErrorV1::StaleAuthority,
        ] {
            assert_eq!(
                map_atomic_error(error),
                LegacyMembershipErrorV1::TargetNotFound
            );
        }
    }

    #[test]
    fn new_membership_commands_preserve_exact_cap_identifiers_and_order() {
        let bulk = LegacyMembershipAdapterV1::add_space_members()
            .prepare(&bulk_add_request(
                vec![USER_TWO, USER_ONE, USER_TWO],
                "admin",
            ))
            .expect("bulk add");
        let LegacyMembershipCommandV1::AddSpaceMembers {
            legacy_user_ids,
            submitted_members,
            ..
        } = bulk
        else {
            unreachable!()
        };
        assert_eq!(legacy_user_ids, [USER_TWO, USER_ONE, USER_TWO]);
        assert_eq!(submitted_members.len(), 3);
        assert!(
            submitted_members
                .iter()
                .all(|member| member.role() == LegacySpaceMemberRoleV1::Admin)
        );
        assert_eq!(submitted_members[0], submitted_members[2]);

        let batch = LegacyMembershipAdapterV1::batch_remove_space_members()
            .prepare(&batch_remove_request(vec![
                MEMBER_TWO, MEMBER_ONE, MEMBER_TWO,
            ]))
            .expect("batch removal");
        assert_eq!(
            batch
                .submitted_member_ids()
                .iter()
                .map(LegacySpaceMemberIdV1::legacy_id)
                .collect::<Vec<_>>(),
            [MEMBER_TWO, MEMBER_ONE, MEMBER_TWO]
        );

        let single = LegacyMembershipAdapterV1::remove_space_member()
            .prepare(&remove_member_request(MEMBER_ONE))
            .expect("single removal");
        assert_eq!(single.submitted_member_ids(), [member_id(MEMBER_ONE)]);
    }

    #[test]
    fn removal_fingerprints_bind_raw_order_duplicates_and_action_not_discovery() {
        let ordered = LegacyMembershipAdapterV1::batch_remove_space_members()
            .prepare(&batch_remove_request(vec![MEMBER_ONE, MEMBER_TWO]))
            .expect("ordered");
        let reversed = LegacyMembershipAdapterV1::batch_remove_space_members()
            .prepare(&batch_remove_request(vec![MEMBER_TWO, MEMBER_ONE]))
            .expect("reversed");
        let duplicate = LegacyMembershipAdapterV1::batch_remove_space_members()
            .prepare(&batch_remove_request(vec![
                MEMBER_ONE, MEMBER_TWO, MEMBER_ONE,
            ]))
            .expect("duplicate");
        let resolved = removal_context(vec![(MEMBER_ONE, USER_TWO)]);
        let noop = LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop {
            organization_id: organization(),
        };
        assert_ne!(
            fingerprint(&ordered, &resolved),
            fingerprint(&reversed, &resolved)
        );
        assert_ne!(
            fingerprint(&ordered, &resolved),
            fingerprint(&duplicate, &resolved)
        );
        assert_eq!(
            fingerprint(&ordered, &resolved),
            fingerprint(&ordered, &noop)
        );

        let single = LegacyMembershipAdapterV1::remove_space_member()
            .prepare(&remove_member_request(MEMBER_ONE))
            .expect("single");
        assert_ne!(
            fingerprint(&ordered, &resolved),
            fingerprint(&single, &resolved)
        );

        let unrelated = removal_context(vec![(MEMBER_THREE, USER_TWO)]);
        assert_eq!(
            ordered.request_fingerprint_for_context(&unrelated),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn bulk_add_receipts_preserve_mutating_and_all_already_cap_results() {
        let command = LegacyMembershipAdapterV1::add_space_members()
            .prepare(&bulk_add_request(vec![USER_ONE, USER_TWO], "member"))
            .expect("bulk add");
        let receipt = exact_receipt(&command, bulk_add_context(vec![USER_ONE]));
        assert_eq!(
            receipt.result(),
            LegacyMembershipMutationResultV1::SpaceMembersAdded {
                added: vec![USER_TWO.into()],
                already_members: vec![USER_ONE.into()],
            }
        );
        assert_eq!(
            receipt.effects().authority_subjects(),
            [mapped(USER_TWO, UserId::parse)]
        );
        assert!(receipt.effects().bumps_authority_generation());

        let all_already = LegacyMembershipAdapterV1::add_space_members()
            .prepare(&bulk_add_request(vec![USER_ONE, USER_ONE], "member"))
            .expect("all already");
        let receipt = exact_receipt(&all_already, bulk_add_context(vec![USER_ONE]));
        assert_eq!(
            receipt.result(),
            LegacyMembershipMutationResultV1::SpaceMembersAdded {
                added: Vec::new(),
                already_members: vec![USER_ONE.into(), USER_ONE.into()],
            }
        );
        assert!(receipt.effects().authority_subjects().is_empty());
        assert!(!receipt.effects().bumps_authority_generation());
    }

    #[test]
    fn duplicate_new_bulk_targets_cannot_manufacture_partial_success() {
        let command = LegacyMembershipAdapterV1::add_space_members()
            .prepare(&bulk_add_request(vec![USER_TWO, USER_TWO], "member"))
            .expect("source preserves duplicates");
        let context = bulk_add_context(vec![USER_ONE]);
        let (result, mutation) = exact_mutation_postcondition(&command, &context);
        let authority = exact_authority_postcondition(&command, &context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(&command, result, context, mutation, authority,),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn batch_removal_preserves_submitted_ids_and_only_proves_resolved_rows() {
        let command = LegacyMembershipAdapterV1::batch_remove_space_members()
            .prepare(&batch_remove_request(vec![
                MEMBER_ONE, MEMBER_TWO, MEMBER_ONE,
            ]))
            .expect("batch removal");
        let context = removal_context(vec![(MEMBER_ONE, USER_TWO)]);
        let receipt = exact_receipt(&command, context);
        assert_eq!(
            receipt.result(),
            LegacyMembershipMutationResultV1::SpaceMembersRemoved {
                removed_member_ids: vec![
                    member_id(MEMBER_ONE),
                    member_id(MEMBER_TWO),
                    member_id(MEMBER_ONE),
                ],
            }
        );
        assert_eq!(
            receipt.effects().authority_subjects(),
            [mapped(USER_TWO, UserId::parse)]
        );
        assert!(receipt.effects().invalidates_space_page());
        assert!(receipt.effects().invalidates_space_members());
    }

    #[test]
    fn all_unmatched_batch_removal_is_an_authorized_noop_with_empty_result() {
        let command = LegacyMembershipAdapterV1::batch_remove_space_members()
            .prepare(&batch_remove_request(vec![MEMBER_ONE, MEMBER_TWO]))
            .expect("batch removal");
        let context = LegacyMembershipDiscoveredContextV1::SpaceRemovalNoop {
            organization_id: organization(),
        };
        let (result, mutation) = exact_mutation_postcondition(&command, &context);
        let authority = LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
            organization(),
            actor(),
            LegacyMembershipActorAuthorityV1::ActiveOrganizationMember,
            None,
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
        )
        .expect("tenant member authority");
        let receipt =
            LegacyMembershipMutationReceiptV1::new(&command, result, context, mutation, authority)
                .expect("no-op receipt");
        assert_eq!(
            receipt.result(),
            LegacyMembershipMutationResultV1::SpaceMembersRemoved {
                removed_member_ids: Vec::new(),
            }
        );
        assert!(!receipt.effects().invalidates_space_page());
        assert!(!receipt.effects().invalidates_space_members());
        assert!(!receipt.effects().bumps_authority_generation());
    }

    #[test]
    fn single_removal_requires_one_exact_non_creator_target() {
        let command = LegacyMembershipAdapterV1::remove_space_member()
            .prepare(&remove_member_request(MEMBER_ONE))
            .expect("single removal");
        let receipt = exact_receipt(&command, removal_context(vec![(MEMBER_ONE, USER_TWO)]));
        assert_eq!(
            receipt.result(),
            LegacyMembershipMutationResultV1::SpaceMemberRemoved
        );

        let creator_context = removal_context(vec![(MEMBER_ONE, USER_ONE)]);
        let (result, mutation) = exact_mutation_postcondition(&command, &creator_context);
        let authority = exact_authority_postcondition(&command, &creator_context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result,
                creator_context,
                mutation,
                authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn fingerprints_bind_action_actor_tenant_scope_roles_and_targets() {
        let add = LegacyMembershipAdapterV1::add_space_member()
            .prepare(&add_request("member"))
            .expect("add");
        let admin = LegacyMembershipAdapterV1::add_space_member()
            .prepare(&add_request("admin"))
            .expect("admin");
        let add_context = add_context();
        assert_ne!(
            fingerprint(&add, &add_context),
            fingerprint(&admin, &add_context)
        );

        let set = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_TWO], Some("member"), None))
            .expect("set");
        assert_ne!(
            fingerprint(&add, &add_context),
            fingerprint(&set, &replacement_context(Vec::new()))
        );

        let mut other = add_request("member");
        other.actor_id = Some(other_actor());
        let other = LegacyMembershipAdapterV1::add_space_member()
            .prepare(&other)
            .expect("other actor");
        assert_ne!(
            fingerprint(&add, &add_context),
            fingerprint(&other, &add_context)
        );
    }

    #[test]
    fn set_fingerprint_uses_the_effective_creator_inclusive_set() {
        let omitted = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_TWO], Some("member"), None))
            .expect("omitted creator");
        let creator_as_member = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(
                vec![],
                None,
                Some(vec![(USER_TWO, "member"), (USER_ONE, "member")]),
            ))
            .expect("creator member");
        let creator_as_admin = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(
                vec![],
                None,
                Some(vec![(USER_ONE, "admin"), (USER_TWO, "member")]),
            ))
            .expect("creator admin");
        let context = replacement_context(Vec::new());
        assert_eq!(
            fingerprint(&omitted, &context),
            fingerprint(&creator_as_member, &context)
        );
        assert_eq!(
            fingerprint(&omitted, &context),
            fingerprint(&creator_as_admin, &context)
        );

        let changed_non_creator = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_TWO], Some("admin"), None))
            .expect("changed target role");
        assert_ne!(
            fingerprint(&omitted, &context),
            fingerprint(&changed_non_creator, &context)
        );
    }

    #[test]
    fn action_mismatch_is_rejected() {
        assert_eq!(
            LegacyMembershipAdapterV1::set_space_members().prepare(&add_request("member")),
            Err(LegacyMembershipErrorV1::Invalid)
        );
    }

    #[test]
    fn request_command_context_and_errors_redact_secrets() {
        let request = set_request(
            vec![],
            None,
            Some(vec![(USER_ONE, "member"), (USER_TWO, "admin")]),
        );
        let command = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&request)
            .expect("command");
        let context = LegacyMembershipDiscoveredContextV1::SpaceReplacement {
            organization_id: organization(),
            space_id: space(),
            creator_id: actor(),
            previous_member_ids: vec![other_actor()],
        };
        let receipt = exact_receipt(&command, context.clone());
        for rendered in [
            format!("{request:?}"),
            format!("{command:?}"),
            format!("{context:?}"),
            format!("{receipt:?}"),
        ] {
            for secret in [SPACE, USER_ONE, USER_TWO, "membership-action-0001"] {
                assert!(!rendered.contains(secret));
            }
            for user_id in [actor(), mapped(USER_TWO, UserId::parse), other_actor()] {
                assert!(!rendered.contains(&user_id.to_string()));
            }
        }
        assert_eq!(
            format!("{:?}", LegacyMembershipErrorV1::Internal),
            "Internal"
        );
    }

    #[test]
    fn invite_receipt_requires_exact_db_discovered_scope_and_target() {
        let command = LegacyMembershipAdapterV1::remove_organization_invite()
            .prepare(&remove_invite_request())
            .expect("command");
        let context = invite_context();
        let receipt = exact_receipt(&command, context.clone());
        assert!(receipt.effects().invalidates_organization_invites());
        assert!(!receipt.effects().invalidates_space_page());
        assert!(!receipt.effects().bumps_authority_generation());

        let (result, mutation) = exact_mutation_postcondition(&command, &context);
        let authority = exact_authority_postcondition(&command, &context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                LegacyMembershipDiscoveredContextV1::OrganizationInvite {
                    organization_id: OrganizationId::new(),
                    invite_id: invite(),
                },
                mutation,
                authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let (result, mutation) = exact_mutation_postcondition(&command, &context);
        let invalid_actor_authority =
            LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
                organization(),
                actor(),
                LegacyMembershipActorAuthorityV1::SpaceManager,
                None,
                Vec::new(),
                None,
                Vec::new(),
                Vec::new(),
            )
            .expect("authority shape");
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                context,
                mutation,
                invalid_actor_authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn set_receipt_requires_creator_inclusive_count_and_invalidates_removed_users() {
        let command = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_TWO], Some("member"), None))
            .expect("command");
        let context = LegacyMembershipDiscoveredContextV1::SpaceReplacement {
            organization_id: organization(),
            space_id: space(),
            creator_id: actor(),
            previous_member_ids: vec![other_actor(), actor()],
        };
        let (_, mutation) = exact_mutation_postcondition(&command, &context);
        let authority = exact_authority_postcondition(&command, &context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                LegacyMembershipMutationResultV1::SpaceMembersSet { count: 1 },
                context.clone(),
                mutation,
                authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
        let receipt = exact_receipt(&command, context);
        assert!(receipt.effects().invalidates_space_page());
        assert!(receipt.effects().invalidates_space_members());
        assert!(receipt.effects().bumps_authority_generation());
        assert!(
            receipt
                .effects()
                .authority_subjects()
                .contains(&other_actor())
        );
        assert!(receipt.effects().authority_subjects().contains(&actor()));
        assert!(
            receipt
                .effects()
                .authority_subjects()
                .contains(&mapped(USER_TWO, UserId::parse))
        );
    }

    #[test]
    fn invite_and_add_receipts_reject_claimed_success_without_exact_row_effects() {
        let invite_command = LegacyMembershipAdapterV1::remove_organization_invite()
            .prepare(&remove_invite_request())
            .expect("invite command");
        let invite_context = invite_context();
        let invite_authority = exact_authority_postcondition(&invite_command, &invite_context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &invite_command,
                LegacyMembershipMutationResultV1::InviteRemoved,
                invite_context,
                LegacyMembershipMutationPostconditionV1::OrganizationInviteRemoved {
                    matching_before: 1,
                    deleted_rows: 0,
                    matching_after: 1,
                },
                invite_authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let add_command = LegacyMembershipAdapterV1::add_space_member()
            .prepare(&add_request("member"))
            .expect("add command");
        let add_context = add_context();
        let add_authority = exact_authority_postcondition(&add_command, &add_context);
        let target = add_command.submitted_members()[0];
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &add_command,
                LegacyMembershipMutationResultV1::SpaceMemberAdded,
                add_context,
                LegacyMembershipMutationPostconditionV1::SpaceMemberInserted {
                    matching_before: 1,
                    inserted_rows: 0,
                    matching_after: 1,
                    final_member: target,
                },
                add_authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn set_receipt_requires_exact_final_roles_members_and_before_after_counts() {
        let command = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_TWO], Some("member"), None))
            .expect("command");
        let context = replacement_context(vec![other_actor()]);
        let (result, exact_mutation) = exact_mutation_postcondition(&command, &context);
        let authority = exact_authority_postcondition(&command, &context);

        let LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced { final_members, .. } =
            exact_mutation
        else {
            unreachable!()
        };
        let mut wrong_creator_role = final_members.clone();
        let creator = wrong_creator_role
            .iter_mut()
            .find(|member| member.user_id() == actor())
            .expect("creator");
        *creator = LegacySpaceMemberTargetV1::from_database_role(actor(), SpaceRole::Viewer)
            .expect("viewer");
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                context.clone(),
                LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced {
                    matching_before: 1,
                    deleted_rows: 1,
                    inserted_rows: 2,
                    matching_after: 2,
                    final_members: wrong_creator_role,
                },
                authority.clone(),
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let mut extra_member = final_members;
        extra_member.push(
            LegacySpaceMemberTargetV1::from_database_role(other_actor(), SpaceRole::Viewer)
                .expect("extra member"),
        );
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                context.clone(),
                LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced {
                    matching_before: 1,
                    deleted_rows: 1,
                    inserted_rows: 3,
                    matching_after: 3,
                    final_members: extra_member,
                },
                authority.clone(),
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let (_, exact_mutation) = exact_mutation_postcondition(&command, &context);
        let LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced {
            inserted_rows,
            matching_after,
            final_members,
            ..
        } = exact_mutation
        else {
            unreachable!()
        };
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                context,
                LegacyMembershipMutationPostconditionV1::SpaceMembersReplaced {
                    matching_before: 1,
                    deleted_rows: 0,
                    inserted_rows,
                    matching_after,
                    final_members,
                },
                authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn authority_receipt_requires_complete_active_graph_bumps_and_revocations() {
        let command = LegacyMembershipAdapterV1::add_space_member()
            .prepare(&add_request("member"))
            .expect("command");
        let context = add_context();
        let (result, mutation) = exact_mutation_postcondition(&command, &context);
        let target = command.submitted_members()[0].user_id();
        let missing_target = LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
            organization(),
            actor(),
            LegacyMembershipActorAuthorityV1::OrganizationOwner,
            Some(space()),
            Vec::new(),
            Some(actor()),
            vec![target],
            vec![target],
        )
        .expect("authority shape");
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                context.clone(),
                mutation.clone(),
                missing_target,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let missing_active_space =
            LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
                organization(),
                actor(),
                LegacyMembershipActorAuthorityV1::OrganizationOwner,
                None,
                vec![target],
                Some(actor()),
                vec![target],
                vec![target],
            )
            .expect("authority shape");
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result.clone(),
                context.clone(),
                mutation.clone(),
                missing_active_space,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );

        let missing_grant_revocation =
            LegacyMembershipAuthorityPostconditionV1::from_verified_database_rows(
                organization(),
                actor(),
                LegacyMembershipActorAuthorityV1::SpaceCreator,
                Some(space()),
                vec![target],
                Some(actor()),
                vec![target],
                Vec::new(),
            )
            .expect("authority shape");
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                result,
                context,
                mutation,
                missing_grant_revocation,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn creator_submitted_with_member_role_is_still_counted_once() {
        let command = LegacyMembershipAdapterV1::set_space_members()
            .prepare(&set_request(vec![USER_ONE], Some("member"), None))
            .expect("command");
        let receipt = exact_receipt(&command, replacement_context(Vec::new()));
        assert_eq!(
            receipt.result(),
            LegacyMembershipMutationResultV1::SpaceMembersSet { count: 1 }
        );
    }

    #[test]
    fn wrong_receipt_variant_or_context_is_corrupt() {
        let command = LegacyMembershipAdapterV1::add_space_member()
            .prepare(&add_request("member"))
            .expect("command");
        let context = add_context();
        let (_, mutation) = exact_mutation_postcondition(&command, &context);
        let authority = exact_authority_postcondition(&command, &context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                LegacyMembershipMutationResultV1::InviteRemoved,
                context.clone(),
                mutation,
                authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
        let (_, mutation) = exact_mutation_postcondition(&command, &context);
        let authority = exact_authority_postcondition(&command, &context);
        assert_eq!(
            LegacyMembershipMutationReceiptV1::new(
                &command,
                LegacyMembershipMutationResultV1::SpaceMemberAdded,
                LegacyMembershipDiscoveredContextV1::SpaceAdd {
                    organization_id: organization(),
                    space_id: SpaceId::new(),
                    creator_id: actor(),
                },
                mutation,
                authority,
            ),
            Err(LegacyMembershipAtomicErrorV1::Corrupt)
        );
    }

    struct RecordingPort {
        calls: Mutex<Vec<(LegacyMembershipCommandV1, LegacyMembershipBrowserFenceV1)>>,
        result:
            Mutex<Option<Result<LegacyMembershipAtomicOutcomeV1, LegacyMembershipAtomicErrorV1>>>,
    }

    impl RecordingPort {
        fn returning(
            result: Result<LegacyMembershipAtomicOutcomeV1, LegacyMembershipAtomicErrorV1>,
        ) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Mutex::new(Some(result)),
            }
        }
    }

    #[async_trait]
    impl LegacyMembershipAtomicPortV1 for RecordingPort {
        async fn execute_atomic(
            &self,
            command: &LegacyMembershipCommandV1,
            browser_fence: &LegacyMembershipBrowserFenceV1,
        ) -> Result<LegacyMembershipAtomicOutcomeV1, LegacyMembershipAtomicErrorV1> {
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

    fn add_receipt(command: &LegacyMembershipCommandV1) -> LegacyMembershipMutationReceiptV1 {
        exact_receipt(command, add_context())
    }

    #[tokio::test]
    async fn adapter_calls_one_atomic_boundary_and_projects_exact_success() {
        let adapter = LegacyMembershipAdapterV1::add_space_member();
        let request = add_request("member");
        let command = adapter.prepare(&request).expect("command");
        let port = RecordingPort::returning(Ok(LegacyMembershipAtomicOutcomeV1::Applied(
            add_receipt(&command),
        )));
        let execution = adapter
            .execute_with_fence(
                &port,
                &request,
                &LegacyMembershipBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        assert_eq!(
            execution.success(),
            LegacyMembershipSuccessV1::SpaceMemberAdded
        );
        assert!(execution.success().success());
        assert_eq!(execution.success().count(), None);
        assert!(execution.mutation_was_applied());
        assert!(!execution.replayed());
        assert_eq!(port.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn replay_returns_the_original_result_without_a_new_projection() {
        let adapter = LegacyMembershipAdapterV1::add_space_member();
        let request = add_request("member");
        let command = adapter.prepare(&request).expect("command");
        let port = RecordingPort::returning(Ok(LegacyMembershipAtomicOutcomeV1::Replay(
            add_receipt(&command),
        )));
        let execution = adapter
            .execute_with_fence(
                &port,
                &request,
                &LegacyMembershipBrowserFenceV1::fixture(actor()),
            )
            .await
            .expect("execution");
        assert_eq!(
            execution.success(),
            LegacyMembershipSuccessV1::SpaceMemberAdded
        );
        assert!(execution.replayed());
        assert!(!execution.mutation_was_applied());
    }

    #[tokio::test]
    async fn receipt_for_a_different_fingerprint_is_rejected_as_corrupt() {
        let adapter = LegacyMembershipAdapterV1::add_space_member();
        let request = add_request("member");
        let different_command = adapter.prepare(&add_request("admin")).expect("different");
        let port = RecordingPort::returning(Ok(LegacyMembershipAtomicOutcomeV1::Replay(
            add_receipt(&different_command),
        )));
        assert_eq!(
            adapter
                .execute_with_fence(
                    &port,
                    &request,
                    &LegacyMembershipBrowserFenceV1::fixture(actor()),
                )
                .await,
            Err(LegacyMembershipErrorV1::Internal)
        );
    }

    #[tokio::test]
    async fn browser_proof_actor_must_match_before_the_port() {
        let adapter = LegacyMembershipAdapterV1::add_space_member();
        let request = add_request("member");
        let command = adapter.prepare(&request).expect("command");
        let port = RecordingPort::returning(Ok(LegacyMembershipAtomicOutcomeV1::Applied(
            add_receipt(&command),
        )));
        assert_eq!(
            adapter
                .execute_with_fence(
                    &port,
                    &request,
                    &LegacyMembershipBrowserFenceV1::fixture(other_actor()),
                )
                .await,
            Err(LegacyMembershipErrorV1::Unauthorized)
        );
        assert!(port.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn atomic_failures_map_to_stable_redacted_errors() {
        for (atomic, public) in [
            (
                LegacyMembershipAtomicErrorV1::TargetMissing,
                LegacyMembershipErrorV1::TargetNotFound,
            ),
            (
                LegacyMembershipAtomicErrorV1::AccessDenied,
                LegacyMembershipErrorV1::TargetNotFound,
            ),
            (
                LegacyMembershipAtomicErrorV1::Conflict,
                LegacyMembershipErrorV1::Conflict,
            ),
            (
                LegacyMembershipAtomicErrorV1::InFlight,
                LegacyMembershipErrorV1::Conflict,
            ),
            (
                LegacyMembershipAtomicErrorV1::Unavailable,
                LegacyMembershipErrorV1::AuthorityUnavailable,
            ),
            (
                LegacyMembershipAtomicErrorV1::Corrupt,
                LegacyMembershipErrorV1::Internal,
            ),
        ] {
            let adapter = LegacyMembershipAdapterV1::add_space_member();
            let request = add_request("member");
            let port = RecordingPort::returning(Err(atomic));
            assert_eq!(
                adapter
                    .execute_with_fence(
                        &port,
                        &request,
                        &LegacyMembershipBrowserFenceV1::fixture(actor()),
                    )
                    .await,
                Err(public)
            );
        }
    }
}
