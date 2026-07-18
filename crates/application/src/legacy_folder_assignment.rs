//! Audited compatibility contract for Cap's three folder-assignment actions.
//!
//! The source actions are intentionally described, not imitated. In
//! particular, Cap looked folders up globally, let one move path update a
//! globally-addressed video, and split some writes across multiple statements.
//! Frame must never reproduce those cross-tenant and partial-write behaviours.
//! The atomic port below therefore revalidates actor, active organization,
//! folder, scope, and every video at the write boundary while binding the
//! mutation and its idempotency journal in one transaction.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    FolderId, IdempotencyKey, LegacyCapNanoId, OrganizationId, SessionId, SessionMutationGrantId,
    SpaceId, UserId, VideoId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;

pub const LEGACY_FOLDER_ASSIGNMENT_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID: &str = "cap-v1-f5daa7be337a2979";
pub const LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID: &str = "cap-v1-1af3645bf2ae7168";
pub const LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID: &str = "cap-v1-eaf277e644aa4b92";
pub const LEGACY_ADD_VIDEOS_TO_FOLDER_IDENTITY: &str =
    "action://apps/web/actions/folders/add-videos.ts#addVideosToFolder";
pub const LEGACY_REMOVE_VIDEOS_FROM_FOLDER_IDENTITY: &str =
    "action://apps/web/actions/folders/remove-videos.ts#removeVideosFromFolder";
pub const LEGACY_MOVE_VIDEO_TO_FOLDER_IDENTITY: &str =
    "action://apps/web/actions/folders/moveVideoToFolder.ts#moveVideoToFolder";
pub const LEGACY_FOLDER_ASSIGNMENT_POLICY: &str = "organization_library.v1";
pub const LEGACY_FOLDER_ASSIGNMENT_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_FOLDER_ASSIGNMENT_MAX_BODY_BYTES: usize = 256 * 1024;
pub const MAX_LEGACY_FOLDER_ASSIGNMENT_VIDEO_IDS: usize = 500;
pub const MAX_LEGACY_FOLDER_ASSIGNMENT_INVALIDATIONS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderAssignmentSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
}

pub const LEGACY_ADD_VIDEOS_TO_FOLDER_SOURCES: &[LegacyFolderAssignmentSourcePinV1] = &[
    LegacyFolderAssignmentSourcePinV1 {
        path: "apps/web/actions/folders/add-videos.ts",
        sha256: "cb4bcfab7d466e54fa77c09fdc4bac24d4041468c5c857b32ea0038f195132aa",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Folder.ts",
        sha256: "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

pub const LEGACY_REMOVE_VIDEOS_FROM_FOLDER_SOURCES: &[LegacyFolderAssignmentSourcePinV1] = &[
    LegacyFolderAssignmentSourcePinV1 {
        path: "apps/web/actions/folders/remove-videos.ts",
        sha256: "f4ce4a28ff1c3f8f2fc23779606a7530945f47fc2e44f49536687ed6209a2d5f",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Folder.ts",
        sha256: "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

pub const LEGACY_MOVE_VIDEO_TO_FOLDER_SOURCES: &[LegacyFolderAssignmentSourcePinV1] = &[
    LegacyFolderAssignmentSourcePinV1 {
        path: "apps/web/actions/folders/moveVideoToFolder.ts",
        sha256: "08f943871c4bdc0f931e140f994dff77c27f249fa3585cc50c1dbd6b8241c045",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Folder.ts",
        sha256: "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Space.ts",
        sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    },
    LegacyFolderAssignmentSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentActionV1 {
    Add,
    Remove,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentObservedAuthorizationV1 {
    /// The source globally loaded the folder, then silently retained only
    /// videos owned by the session user.
    GlobalFolderLookupAndActorOwnedVideoFilter,
    /// The source globally loaded the video and only scoped a non-null target
    /// folder to the active organization.
    GlobalVideoLookupAndActiveOrganizationFolderLookup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentRequiredAuthorizationV1 {
    /// Add/remove preserve Cap's actor-owned selection rule but fail the whole
    /// canonical request instead of silently filtering a foreign video. The
    /// atomic writer must also prove the folder and selected storage context
    /// belong to the trusted active tenant.
    SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo,
    /// Move did not have Cap's actor-owner filter. Frame narrows it to an
    /// active-tenant video and a manager-authorized selected storage context,
    /// so the source's global lookup cannot become cross-tenant authority.
    SessionActorActiveTenantManagerSelectedContextAndTenantVideo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentObservedMutationV1 {
    InsertMembershipThenUpdateFolderWithoutTransaction,
    ClearVideoThenClearMembershipWithoutTransaction,
    LastWriteWinsTargetUpdate,
}

/// Required normalized storage effect. These variants deliberately separate
/// Cap's user-visible contexts from its unsafe global lookups and split
/// writes. The D1 port must prove the selected branch and its postcondition in
/// the same transaction as the receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentRequiredMutationV1 {
    /// Organization-library scope writes `shared_videos.folder_id`; a real
    /// space scope writes `space_videos.folder_id`. Add creates the scoped
    /// membership when absent and otherwise reaches the same desired state.
    AddScopedMembershipOrSetFolder,
    /// Clear only the matching scoped membership folder, and clear
    /// `videos.folder_id` only when it equals the requested folder.
    RemoveMatchingScopedFolderAndDirectMatch,
    /// With no scope, update `videos.folder_id`. Organization-library scope
    /// updates `shared_videos`; real-space scope updates `space_videos`.
    /// A missing scoped membership remains a successful void no-op.
    MoveOnlySelectedStorageContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentInputShapeV1 {
    PositionalFolderVideoListAndSpace,
    ObjectVideoOptionalFolderAndOptionalSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentObservedSuccessV1 {
    CaughtObjectWithAddedCountAndMessage,
    CaughtObjectWithRemovedCountAndMessage,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentObservedFailureV1 {
    CaughtObjectWithErrorMessage,
    ThrownError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentObservedRetryV1 {
    NonAtomicInsertThenUpdate,
    NonAtomicClearThenSecondaryClear,
    LastWriteWins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentReplayV1 {
    ReturnOriginalJournaledSuccessWithoutReapplyingMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentKeyReuseV1 {
    SameFingerprintReplaysDifferentFingerprintConflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentAtomicityV1 {
    AuthorityMutationOutcomeAndJournalInOneTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderAssignmentRequiredRetryV1 {
    pub replay: LegacyFolderAssignmentReplayV1,
    pub key_reuse: LegacyFolderAssignmentKeyReuseV1,
    pub atomicity: LegacyFolderAssignmentAtomicityV1,
}

pub const LEGACY_FOLDER_ASSIGNMENT_REQUIRED_RETRY: LegacyFolderAssignmentRequiredRetryV1 =
    LegacyFolderAssignmentRequiredRetryV1 {
        replay:
            LegacyFolderAssignmentReplayV1::ReturnOriginalJournaledSuccessWithoutReapplyingMutation,
        key_reuse:
            LegacyFolderAssignmentKeyReuseV1::SameFingerprintReplaysDifferentFingerprintConflicts,
        atomicity:
            LegacyFolderAssignmentAtomicityV1::AuthorityMutationOutcomeAndJournalInOneTransaction,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderAssignmentProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub sources: &'static [LegacyFolderAssignmentSourcePinV1],
    pub authentication: &'static str,
    pub policy: &'static str,
    pub content_type: &'static str,
    pub max_body_bytes: usize,
    pub input: LegacyFolderAssignmentInputShapeV1,
    pub observed_authorization: LegacyFolderAssignmentObservedAuthorizationV1,
    pub required_authorization: LegacyFolderAssignmentRequiredAuthorizationV1,
    pub observed_mutation: LegacyFolderAssignmentObservedMutationV1,
    pub required_mutation: LegacyFolderAssignmentRequiredMutationV1,
    pub observed_success: LegacyFolderAssignmentObservedSuccessV1,
    pub observed_failure: LegacyFolderAssignmentObservedFailureV1,
    pub observed_retry: LegacyFolderAssignmentObservedRetryV1,
    pub required_retry: LegacyFolderAssignmentRequiredRetryV1,
    pub tenant_non_disclosure: bool,
    pub protected_gates: &'static [&'static str],
}

pub const LEGACY_ADD_VIDEOS_TO_FOLDER_PROFILE: LegacyFolderAssignmentProfileV1 =
    LegacyFolderAssignmentProfileV1 {
        operation_id: LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_ADD_VIDEOS_TO_FOLDER_IDENTITY,
        pinned_commit: LEGACY_FOLDER_ASSIGNMENT_CAP_COMMIT,
        sources: LEGACY_ADD_VIDEOS_TO_FOLDER_SOURCES,
        authentication: "session",
        policy: LEGACY_FOLDER_ASSIGNMENT_POLICY,
        content_type: LEGACY_FOLDER_ASSIGNMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_FOLDER_ASSIGNMENT_MAX_BODY_BYTES,
        input: LegacyFolderAssignmentInputShapeV1::PositionalFolderVideoListAndSpace,
        observed_authorization:
            LegacyFolderAssignmentObservedAuthorizationV1::GlobalFolderLookupAndActorOwnedVideoFilter,
        required_authorization:
            LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo,
        observed_mutation:
            LegacyFolderAssignmentObservedMutationV1::InsertMembershipThenUpdateFolderWithoutTransaction,
        required_mutation:
            LegacyFolderAssignmentRequiredMutationV1::AddScopedMembershipOrSetFolder,
        observed_success:
            LegacyFolderAssignmentObservedSuccessV1::CaughtObjectWithAddedCountAndMessage,
        observed_failure: LegacyFolderAssignmentObservedFailureV1::CaughtObjectWithErrorMessage,
        observed_retry:
            LegacyFolderAssignmentObservedRetryV1::NonAtomicInsertThenUpdate,
        required_retry: LEGACY_FOLDER_ASSIGNMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
    };

pub const LEGACY_REMOVE_VIDEOS_FROM_FOLDER_PROFILE: LegacyFolderAssignmentProfileV1 =
    LegacyFolderAssignmentProfileV1 {
        operation_id: LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_REMOVE_VIDEOS_FROM_FOLDER_IDENTITY,
        pinned_commit: LEGACY_FOLDER_ASSIGNMENT_CAP_COMMIT,
        sources: LEGACY_REMOVE_VIDEOS_FROM_FOLDER_SOURCES,
        authentication: "session",
        policy: LEGACY_FOLDER_ASSIGNMENT_POLICY,
        content_type: LEGACY_FOLDER_ASSIGNMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_FOLDER_ASSIGNMENT_MAX_BODY_BYTES,
        input: LegacyFolderAssignmentInputShapeV1::PositionalFolderVideoListAndSpace,
        observed_authorization:
            LegacyFolderAssignmentObservedAuthorizationV1::GlobalFolderLookupAndActorOwnedVideoFilter,
        required_authorization:
            LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo,
        observed_mutation:
            LegacyFolderAssignmentObservedMutationV1::ClearVideoThenClearMembershipWithoutTransaction,
        required_mutation:
            LegacyFolderAssignmentRequiredMutationV1::RemoveMatchingScopedFolderAndDirectMatch,
        observed_success:
            LegacyFolderAssignmentObservedSuccessV1::CaughtObjectWithRemovedCountAndMessage,
        observed_failure: LegacyFolderAssignmentObservedFailureV1::CaughtObjectWithErrorMessage,
        observed_retry:
            LegacyFolderAssignmentObservedRetryV1::NonAtomicClearThenSecondaryClear,
        required_retry: LEGACY_FOLDER_ASSIGNMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
    };

pub const LEGACY_MOVE_VIDEO_TO_FOLDER_PROFILE: LegacyFolderAssignmentProfileV1 =
    LegacyFolderAssignmentProfileV1 {
        operation_id: LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_MOVE_VIDEO_TO_FOLDER_IDENTITY,
        pinned_commit: LEGACY_FOLDER_ASSIGNMENT_CAP_COMMIT,
        sources: LEGACY_MOVE_VIDEO_TO_FOLDER_SOURCES,
        authentication: "session",
        policy: LEGACY_FOLDER_ASSIGNMENT_POLICY,
        content_type: LEGACY_FOLDER_ASSIGNMENT_CONTENT_TYPE,
        max_body_bytes: LEGACY_FOLDER_ASSIGNMENT_MAX_BODY_BYTES,
        input: LegacyFolderAssignmentInputShapeV1::ObjectVideoOptionalFolderAndOptionalSpace,
        observed_authorization:
            LegacyFolderAssignmentObservedAuthorizationV1::GlobalVideoLookupAndActiveOrganizationFolderLookup,
        required_authorization:
            LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantManagerSelectedContextAndTenantVideo,
        observed_mutation: LegacyFolderAssignmentObservedMutationV1::LastWriteWinsTargetUpdate,
        required_mutation:
            LegacyFolderAssignmentRequiredMutationV1::MoveOnlySelectedStorageContext,
        observed_success: LegacyFolderAssignmentObservedSuccessV1::Void,
        observed_failure: LegacyFolderAssignmentObservedFailureV1::ThrownError,
        observed_retry:
            LegacyFolderAssignmentObservedRetryV1::LastWriteWins,
        required_retry: LEGACY_FOLDER_ASSIGNMENT_REQUIRED_RETRY,
        tenant_non_disclosure: true,
        protected_gates: &["released_legacy_client_e2e"],
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyFolderAssignmentInputV1 {
    Add {
        legacy_folder_id: String,
        legacy_video_ids: Vec<String>,
        legacy_scope_id: String,
    },
    Remove {
        legacy_folder_id: String,
        legacy_video_ids: Vec<String>,
        legacy_scope_id: String,
    },
    Move {
        legacy_video_id: String,
        legacy_folder_id: Option<String>,
        legacy_scope_id: Option<String>,
    },
}

impl LegacyFolderAssignmentInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyFolderAssignmentActionV1 {
        match self {
            Self::Add { .. } => LegacyFolderAssignmentActionV1::Add,
            Self::Remove { .. } => LegacyFolderAssignmentActionV1::Remove,
            Self::Move { .. } => LegacyFolderAssignmentActionV1::Move,
        }
    }
}

impl fmt::Debug for LegacyFolderAssignmentInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add {
                legacy_video_ids, ..
            } => formatter
                .debug_struct("Add")
                .field("targets", &"<redacted>")
                .field("video_count", &legacy_video_ids.len())
                .finish(),
            Self::Remove {
                legacy_video_ids, ..
            } => formatter
                .debug_struct("Remove")
                .field("targets", &"<redacted>")
                .field("video_count", &legacy_video_ids.len())
                .finish(),
            Self::Move {
                legacy_folder_id,
                legacy_scope_id,
                ..
            } => formatter
                .debug_struct("Move")
                .field("video", &"<redacted>")
                .field("folder_present", &legacy_folder_id.is_some())
                .field("scope_present", &legacy_scope_id.is_some())
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderAssignmentRequestV1 {
    pub credential: Option<LegacyFolderAssignmentCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyFolderAssignmentInputV1,
}

impl fmt::Debug for LegacyFolderAssignmentRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderAssignmentRequestV1")
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentScopeV1 {
    OrganizationLibrary { organization_id: OrganizationId },
    Space { space_id: SpaceId },
}

impl fmt::Debug for LegacyFolderAssignmentScopeV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OrganizationLibrary { .. } => "OrganizationLibrary([redacted])",
            Self::Space { .. } => "Space([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderAssignmentAuthorityV1 {
    actor_id: UserId,
    active_organization_id: OrganizationId,
}

impl LegacyFolderAssignmentAuthorityV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> OrganizationId {
        self.active_organization_id
    }
}

impl fmt::Debug for LegacyFolderAssignmentAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyFolderAssignmentAuthorityV1([redacted])")
    }
}

/// Unforgeable browser boundary material carried into the atomic D1 port.
///
/// Production callers can construct this value only from the application
/// authentication service's `ValidatedBrowserMutationProof`. The database
/// adapter must assert all three identifiers against the live one-use grant
/// and consume that grant in the same batch as the assignment, receipt, and
/// audit record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderAssignmentBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyFolderAssignmentBrowserFenceV1 {
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

impl fmt::Debug for LegacyFolderAssignmentBrowserFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyFolderAssignmentBrowserFenceV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderAssignmentFenceV1 {
    authority: LegacyFolderAssignmentAuthorityV1,
    idempotency_key: IdempotencyKey,
    request_fingerprint: [u8; 32],
}

impl LegacyFolderAssignmentFenceV1 {
    #[must_use]
    pub const fn authority(&self) -> &LegacyFolderAssignmentAuthorityV1 {
        &self.authority
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// Exposed only for comparison with the tenant-scoped durable journal.
    #[must_use]
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }
}

impl fmt::Debug for LegacyFolderAssignmentFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderAssignmentFenceV1")
            .field("authority", &self.authority)
            .field("idempotency_key", &"<redacted>")
            .field("request_fingerprint", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyFolderAssignmentCommandV1 {
    Add {
        fence: LegacyFolderAssignmentFenceV1,
        folder_id: FolderId,
        video_ids: Vec<VideoId>,
        scope: LegacyFolderAssignmentScopeV1,
    },
    Remove {
        fence: LegacyFolderAssignmentFenceV1,
        folder_id: FolderId,
        video_ids: Vec<VideoId>,
        scope: LegacyFolderAssignmentScopeV1,
    },
    Move {
        fence: LegacyFolderAssignmentFenceV1,
        video_id: VideoId,
        folder_id: Option<FolderId>,
        scope: Option<LegacyFolderAssignmentScopeV1>,
    },
}

impl LegacyFolderAssignmentCommandV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyFolderAssignmentActionV1 {
        match self {
            Self::Add { .. } => LegacyFolderAssignmentActionV1::Add,
            Self::Remove { .. } => LegacyFolderAssignmentActionV1::Remove,
            Self::Move { .. } => LegacyFolderAssignmentActionV1::Move,
        }
    }

    #[must_use]
    pub const fn fence(&self) -> &LegacyFolderAssignmentFenceV1 {
        match self {
            Self::Add { fence, .. } | Self::Remove { fence, .. } | Self::Move { fence, .. } => {
                fence
            }
        }
    }

    #[must_use]
    pub fn requested_video_count(&self) -> usize {
        match self {
            Self::Add { video_ids, .. } | Self::Remove { video_ids, .. } => video_ids.len(),
            Self::Move { .. } => 1,
        }
    }

    #[must_use]
    pub const fn required_authorization(&self) -> LegacyFolderAssignmentRequiredAuthorizationV1 {
        match self {
            Self::Add { .. } | Self::Remove { .. } => {
                LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo
            }
            Self::Move { .. } => {
                LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantManagerSelectedContextAndTenantVideo
            }
        }
    }
}

impl fmt::Debug for LegacyFolderAssignmentCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add {
                fence, video_ids, ..
            } => formatter
                .debug_struct("Add")
                .field("fence", fence)
                .field("targets", &"<redacted>")
                .field("video_count", &video_ids.len())
                .finish(),
            Self::Remove {
                fence, video_ids, ..
            } => formatter
                .debug_struct("Remove")
                .field("fence", fence)
                .field("targets", &"<redacted>")
                .field("video_count", &video_ids.len())
                .finish(),
            Self::Move {
                fence,
                folder_id,
                scope,
                ..
            } => formatter
                .debug_struct("Move")
                .field("fence", fence)
                .field("video", &"<redacted>")
                .field("folder_present", &folder_id.is_some())
                .field("scope_present", &scope.is_some())
                .finish(),
        }
    }
}

/// Cache domains returned only after the atomic authority check. Raw Cap IDs
/// and arbitrary path fragments never enter this type.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LegacyFolderAssignmentInvalidationTargetV1 {
    Folder {
        folder_id: FolderId,
    },
    SpaceRoot {
        space_id: SpaceId,
    },
    SpaceFolder {
        space_id: SpaceId,
        folder_id: FolderId,
    },
}

impl fmt::Debug for LegacyFolderAssignmentInvalidationTargetV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Folder { .. } => "Folder([redacted])",
            Self::SpaceRoot { .. } => "SpaceRoot([redacted])",
            Self::SpaceFolder { .. } => "SpaceFolder([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderAssignmentEffectsV1 {
    /// `/dashboard/caps` is implicit and always first; the vector contains
    /// validated structured targets only.
    targets: Vec<LegacyFolderAssignmentInvalidationTargetV1>,
}

impl LegacyFolderAssignmentEffectsV1 {
    pub fn from_authorized_targets(
        targets: impl IntoIterator<Item = LegacyFolderAssignmentInvalidationTargetV1>,
    ) -> Result<Self, LegacyFolderAssignmentErrorV1> {
        let mut deduplicated = Vec::new();
        for target in targets {
            if !deduplicated.contains(&target) {
                if deduplicated.len() == MAX_LEGACY_FOLDER_ASSIGNMENT_INVALIDATIONS {
                    return Err(LegacyFolderAssignmentErrorV1::Internal);
                }
                deduplicated.push(target);
            }
        }
        Ok(Self {
            targets: deduplicated,
        })
    }

    #[must_use]
    pub const fn invalidates_caps(&self) -> bool {
        true
    }

    #[must_use]
    pub fn targets(&self) -> &[LegacyFolderAssignmentInvalidationTargetV1] {
        &self.targets
    }

    fn contains(&self, target: LegacyFolderAssignmentInvalidationTargetV1) -> bool {
        self.targets.contains(&target)
    }

    fn covers_command(
        &self,
        command: &LegacyFolderAssignmentCommandV1,
        context: LegacyFolderAssignmentAuthorizedContextV1,
    ) -> bool {
        match command {
            LegacyFolderAssignmentCommandV1::Add { folder_id, .. }
            | LegacyFolderAssignmentCommandV1::Remove { folder_id, .. } => {
                self.contains(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                    folder_id: *folder_id,
                }) && context.target_folder_space_id().is_none_or(|space_id| {
                    self.contains(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                        space_id,
                        folder_id: *folder_id,
                    })
                })
            }
            LegacyFolderAssignmentCommandV1::Move {
                folder_id, scope, ..
            } => {
                let target_folder_covered = folder_id.is_none_or(|folder_id| {
                    self.contains(LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id })
                });
                let original_folder_covered =
                    context.original_folder_id().is_none_or(|folder_id| {
                        self.contains(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                            folder_id,
                        })
                    });
                let parents_covered = match (context.original_folder_id(), *folder_id) {
                    (Some(original), Some(target)) if original != target => {
                        context.original_parent_id().is_none_or(|folder_id| {
                            self.contains(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                                folder_id,
                            })
                        }) && context.target_parent_id().is_none_or(|folder_id| {
                            self.contains(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                                folder_id,
                            })
                        })
                    }
                    _ => true,
                };
                let scope_covered = match scope {
                    Some(LegacyFolderAssignmentScopeV1::Space { space_id }) => {
                        self.contains(LegacyFolderAssignmentInvalidationTargetV1::SpaceRoot {
                            space_id: *space_id,
                        }) && folder_id.is_none_or(|folder_id| {
                            self.contains(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                                space_id: *space_id,
                                folder_id,
                            })
                        })
                    }
                    Some(LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. }) => folder_id
                        .is_none_or(|folder_id| {
                            context.target_folder_space_id().is_none_or(|space_id| {
                                self.contains(
                                    LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                                        space_id,
                                        folder_id,
                                    },
                                )
                            })
                        }),
                    None => folder_id.is_none_or(|folder_id| {
                        context.target_folder_space_id().is_none_or(|space_id| {
                            self.contains(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                                space_id,
                                folder_id,
                            })
                        })
                    }),
                };
                target_folder_covered && original_folder_covered && parents_covered && scope_covered
            }
        }
    }
}

impl fmt::Debug for LegacyFolderAssignmentEffectsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderAssignmentEffectsV1")
            .field("invalidates_caps", &true)
            .field("validated_target_count", &self.targets.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyFolderAssignmentSuccessV1 {
    Added { added_count: u16 },
    Removed { removed_count: u16 },
    MoveVoid,
}

impl LegacyFolderAssignmentSuccessV1 {
    /// Add/remove project to Cap's `{ success: true, message, *Count }`
    /// object. Move resolves with JavaScript `undefined` and has no message.
    #[must_use]
    pub const fn object_success(&self) -> Option<bool> {
        match self {
            Self::Added { .. } | Self::Removed { .. } => Some(true),
            Self::MoveVoid => None,
        }
    }

    #[must_use]
    pub fn message(&self) -> Option<String> {
        match self {
            Self::Added { added_count } => Some(format!(
                "{added_count} video{} added to folder",
                if *added_count == 1 { "" } else { "s" }
            )),
            Self::Removed { removed_count } => Some(format!(
                "{removed_count} video{} removed from folder",
                if *removed_count == 1 { "" } else { "s" }
            )),
            Self::MoveVoid => None,
        }
    }

    #[must_use]
    pub const fn added_count(&self) -> Option<u16> {
        match self {
            Self::Added { added_count } => Some(*added_count),
            Self::Removed { .. } | Self::MoveVoid => None,
        }
    }

    #[must_use]
    pub const fn removed_count(&self) -> Option<u16> {
        match self {
            Self::Removed { removed_count } => Some(*removed_count),
            Self::Added { .. } | Self::MoveVoid => None,
        }
    }

    #[must_use]
    pub const fn resolves_void(&self) -> bool {
        matches!(self, Self::MoveVoid)
    }
}

/// Folder graph facts loaded and authority-checked by the atomic repository.
/// They make cache effects derived from database state part of the durable
/// receipt instead of optional best-effort work at the HTTP boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyFolderAssignmentAuthorizedContextV1 {
    target_folder_space_id: Option<SpaceId>,
    original_folder_id: Option<FolderId>,
    original_parent_id: Option<FolderId>,
    target_parent_id: Option<FolderId>,
}

impl LegacyFolderAssignmentAuthorizedContextV1 {
    pub fn new(
        command: &LegacyFolderAssignmentCommandV1,
        target_folder_space_id: Option<SpaceId>,
        original_folder_id: Option<FolderId>,
        original_parent_id: Option<FolderId>,
        target_parent_id: Option<FolderId>,
    ) -> Result<Self, LegacyFolderAssignmentAtomicErrorV1> {
        let valid = match command {
            LegacyFolderAssignmentCommandV1::Add { scope, .. }
            | LegacyFolderAssignmentCommandV1::Remove { scope, .. } => {
                original_folder_id.is_none()
                    && original_parent_id.is_none()
                    && target_parent_id.is_none()
                    && match scope {
                        LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. } => true,
                        LegacyFolderAssignmentScopeV1::Space { space_id } => {
                            target_folder_space_id == Some(*space_id)
                        }
                    }
            }
            LegacyFolderAssignmentCommandV1::Move {
                folder_id, scope, ..
            } => {
                let target_shape = match folder_id {
                    Some(_) => match scope {
                        Some(LegacyFolderAssignmentScopeV1::Space { space_id }) => {
                            target_folder_space_id == Some(*space_id)
                        }
                        Some(LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. }) | None => {
                            true
                        }
                    },
                    None => target_folder_space_id.is_none() && target_parent_id.is_none(),
                };
                target_shape
                    && original_parent_id.is_none_or(|_| original_folder_id.is_some())
                    && target_parent_id.is_none_or(|_| folder_id.is_some())
            }
        };
        valid
            .then_some(Self {
                target_folder_space_id,
                original_folder_id,
                original_parent_id,
                target_parent_id,
            })
            .ok_or(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
    }

    #[must_use]
    pub const fn target_folder_space_id(self) -> Option<SpaceId> {
        self.target_folder_space_id
    }

    #[must_use]
    pub const fn original_folder_id(self) -> Option<FolderId> {
        self.original_folder_id
    }

    #[must_use]
    pub const fn original_parent_id(self) -> Option<FolderId> {
        self.original_parent_id
    }

    #[must_use]
    pub const fn target_parent_id(self) -> Option<FolderId> {
        self.target_parent_id
    }
}

impl fmt::Debug for LegacyFolderAssignmentAuthorizedContextV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderAssignmentAuthorizedContextV1")
            .field(
                "target_folder_space",
                &self.target_folder_space_id.is_some(),
            )
            .field("original_folder", &self.original_folder_id.is_some())
            .field("original_parent", &self.original_parent_id.is_some())
            .field("target_parent", &self.target_parent_id.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderAssignmentMutationReceiptV1 {
    affected_count: Option<u16>,
    effects: LegacyFolderAssignmentEffectsV1,
    authorized_context: LegacyFolderAssignmentAuthorizedContextV1,
}

impl LegacyFolderAssignmentMutationReceiptV1 {
    pub fn new(
        command: &LegacyFolderAssignmentCommandV1,
        affected_count: Option<u16>,
        effects: LegacyFolderAssignmentEffectsV1,
        authorized_context: LegacyFolderAssignmentAuthorizedContextV1,
    ) -> Result<Self, LegacyFolderAssignmentAtomicErrorV1> {
        let valid = match command {
            LegacyFolderAssignmentCommandV1::Add { video_ids, .. }
            | LegacyFolderAssignmentCommandV1::Remove { video_ids, .. } => {
                affected_count.map(usize::from) == Some(video_ids.len())
            }
            LegacyFolderAssignmentCommandV1::Move { .. } => affected_count.is_none(),
        } && effects.covers_command(command, authorized_context);
        if !valid {
            return Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            affected_count,
            effects,
            authorized_context,
        })
    }

    #[must_use]
    pub const fn affected_count(&self) -> Option<u16> {
        self.affected_count
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyFolderAssignmentEffectsV1 {
        &self.effects
    }

    #[must_use]
    pub const fn authorized_context(&self) -> LegacyFolderAssignmentAuthorizedContextV1 {
        self.authorized_context
    }
}

impl fmt::Debug for LegacyFolderAssignmentMutationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderAssignmentMutationReceiptV1")
            .field("affected_count", &self.affected_count)
            .field("effects", &self.effects)
            .field("authorized_context", &self.authorized_context)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyFolderAssignmentAtomicOutcomeV1 {
    Applied(LegacyFolderAssignmentMutationReceiptV1),
    Replay(LegacyFolderAssignmentMutationReceiptV1),
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFolderAssignmentExecutionV1 {
    success: LegacyFolderAssignmentSuccessV1,
    effects: LegacyFolderAssignmentEffectsV1,
    replayed: bool,
}

impl LegacyFolderAssignmentExecutionV1 {
    #[must_use]
    pub const fn success(&self) -> &LegacyFolderAssignmentSuccessV1 {
        &self.success
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyFolderAssignmentEffectsV1 {
        &self.effects
    }

    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }

    /// Durable mutation effects are never re-applied on replay. Cache
    /// invalidation may be emitted again by ingress because it is idempotent.
    #[must_use]
    pub const fn mutation_was_applied(&self) -> bool {
        !self.replayed
    }
}

impl fmt::Debug for LegacyFolderAssignmentExecutionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFolderAssignmentExecutionV1")
            .field("success", &self.success)
            .field("effects", &self.effects)
            .field("replayed", &self.replayed)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyFolderAssignmentAtomicErrorV1 {
    #[error("folder assignment target was not found")]
    TargetMissing,
    #[error("folder assignment target was not found")]
    AccessDenied,
    #[error("folder assignment target was not found")]
    CrossTenant,
    #[error("folder assignment target was not found")]
    StaleAuthority,
    #[error("folder assignment conflicts with a prior request")]
    Conflict,
    #[error("folder assignment is already in flight")]
    InFlight,
    #[error("folder assignment authority is unavailable")]
    Unavailable,
    #[error("folder assignment authority returned invalid state")]
    Corrupt,
}

/// Provider-free transaction boundary for assignment and its durable journal.
///
/// Implementations must perform one atomic transaction which:
///
/// 1. revalidates the session actor and active organization in `authority`;
/// 2. proves the folder and scope belong to that active organization and that
///    the actor may mutate them. For a real-space scope, the folder's
///    `space_id` must equal that exact space. For organization-library scope,
///    the folder may belong to any active space in the same active
///    organization, but never another organization. A direct move with no
///    scope writes only `videos.folder_id`;
/// 3. for add/remove, proves every canonical video exists in the tenant and
///    `videos.owner_id == actor_id`, failing the complete request rather than
///    silently dropping an unowned ID; for move, proves the video belongs to
///    the active tenant and the actor has manager authority for the selected
///    context (never inheriting Cap's global-video lookup);
/// 4. binds the tenant-scoped key to the normalized request fingerprint;
/// 5. applies the action-specific `required_mutation` postcondition: add
///    creates-or-sets only the selected scoped membership, remove clears only
///    the matching scoped folder plus an exactly matching direct folder, and
///    move updates only `videos`, `shared_videos`, or `space_videos` as chosen
///    by its scope (a missing scoped membership is a void no-op); and
/// 6. journals the exact canonical input count and every required validated
///    invalidation target, appends the business audit outcome, and asserts and
///    consumes the supplied one-use browser fence.
///
/// The same key and fingerprint returns `Replay` with the original receipt. A
/// different fingerprint under the key returns `Conflict`. No implementation
/// may split journal and mutation commits or authorize from a global lookup.
#[async_trait]
pub trait LegacyFolderAssignmentAtomicPortV1: Send + Sync {
    async fn execute_atomic(
        &self,
        command: &LegacyFolderAssignmentCommandV1,
        browser_fence: &LegacyFolderAssignmentBrowserFenceV1,
    ) -> Result<LegacyFolderAssignmentAtomicOutcomeV1, LegacyFolderAssignmentAtomicErrorV1>;
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyFolderAssignmentErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid folder assignment request")]
    Invalid,
    #[error("An idempotency key is required")]
    IdempotencyRequired,
    #[error("Folder assignment target not found")]
    TargetNotFound,
    #[error("Folder assignment request conflicts with a prior request")]
    Conflict,
    #[error("Folder assignment authority is unavailable")]
    AuthorityUnavailable,
    #[error("Folder assignment failed")]
    Internal,
}

impl fmt::Debug for LegacyFolderAssignmentErrorV1 {
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
pub struct LegacyFolderAssignmentAdapterV1 {
    action: LegacyFolderAssignmentActionV1,
}

impl LegacyFolderAssignmentAdapterV1 {
    #[must_use]
    pub const fn add_videos_to_folder() -> Self {
        Self {
            action: LegacyFolderAssignmentActionV1::Add,
        }
    }

    #[must_use]
    pub const fn remove_videos_from_folder() -> Self {
        Self {
            action: LegacyFolderAssignmentActionV1::Remove,
        }
    }

    #[must_use]
    pub const fn move_video_to_folder() -> Self {
        Self {
            action: LegacyFolderAssignmentActionV1::Move,
        }
    }

    #[must_use]
    pub const fn profile(self) -> &'static LegacyFolderAssignmentProfileV1 {
        match self.action {
            LegacyFolderAssignmentActionV1::Add => &LEGACY_ADD_VIDEOS_TO_FOLDER_PROFILE,
            LegacyFolderAssignmentActionV1::Remove => &LEGACY_REMOVE_VIDEOS_FROM_FOLDER_PROFILE,
            LegacyFolderAssignmentActionV1::Move => &LEGACY_MOVE_VIDEO_TO_FOLDER_PROFILE,
        }
    }

    pub fn prepare(
        self,
        request: &LegacyFolderAssignmentRequestV1,
    ) -> Result<LegacyFolderAssignmentCommandV1, LegacyFolderAssignmentErrorV1> {
        if request.input.action() != self.action {
            return Err(LegacyFolderAssignmentErrorV1::Invalid);
        }
        let Some(actor_id) = request.actor_id else {
            return Err(LegacyFolderAssignmentErrorV1::Unauthorized);
        };
        let Some(active_organization_id) = request.active_organization_id else {
            return Err(LegacyFolderAssignmentErrorV1::Unauthorized);
        };
        if request.credential != Some(LegacyFolderAssignmentCredentialV1::Session) {
            return Err(LegacyFolderAssignmentErrorV1::Unauthorized);
        }
        let idempotency_key = request
            .idempotency_key
            .as_ref()
            .ok_or(LegacyFolderAssignmentErrorV1::IdempotencyRequired)
            .and_then(|key| {
                IdempotencyKey::parse(key.clone())
                    .map_err(|_| LegacyFolderAssignmentErrorV1::Invalid)
            })?;
        let authority = LegacyFolderAssignmentAuthorityV1 {
            actor_id,
            active_organization_id,
        };

        match &request.input {
            LegacyFolderAssignmentInputV1::Add {
                legacy_folder_id,
                legacy_video_ids,
                legacy_scope_id,
            } => {
                let folder_id = map_folder_id(legacy_folder_id)?;
                let video_ids = map_video_list(legacy_video_ids)?;
                let scope = map_scope(legacy_scope_id, active_organization_id)?;
                let fingerprint = fingerprint_list(
                    LegacyFolderAssignmentActionV1::Add,
                    folder_id,
                    &video_ids,
                    scope,
                );
                Ok(LegacyFolderAssignmentCommandV1::Add {
                    fence: LegacyFolderAssignmentFenceV1 {
                        authority,
                        idempotency_key,
                        request_fingerprint: fingerprint,
                    },
                    folder_id,
                    video_ids,
                    scope,
                })
            }
            LegacyFolderAssignmentInputV1::Remove {
                legacy_folder_id,
                legacy_video_ids,
                legacy_scope_id,
            } => {
                let folder_id = map_folder_id(legacy_folder_id)?;
                let video_ids = map_video_list(legacy_video_ids)?;
                let scope = map_scope(legacy_scope_id, active_organization_id)?;
                let fingerprint = fingerprint_list(
                    LegacyFolderAssignmentActionV1::Remove,
                    folder_id,
                    &video_ids,
                    scope,
                );
                Ok(LegacyFolderAssignmentCommandV1::Remove {
                    fence: LegacyFolderAssignmentFenceV1 {
                        authority,
                        idempotency_key,
                        request_fingerprint: fingerprint,
                    },
                    folder_id,
                    video_ids,
                    scope,
                })
            }
            LegacyFolderAssignmentInputV1::Move {
                legacy_video_id,
                legacy_folder_id,
                legacy_scope_id,
            } => {
                let video_id = map_video_id(legacy_video_id)?;
                let folder_id = legacy_folder_id.as_deref().map(map_folder_id).transpose()?;
                let scope = legacy_scope_id
                    .as_deref()
                    .map(|value| map_scope(value, active_organization_id))
                    .transpose()?;
                let fingerprint = fingerprint_move(video_id, folder_id, scope);
                Ok(LegacyFolderAssignmentCommandV1::Move {
                    fence: LegacyFolderAssignmentFenceV1 {
                        authority,
                        idempotency_key,
                        request_fingerprint: fingerprint,
                    },
                    video_id,
                    folder_id,
                    scope,
                })
            }
        }
    }

    pub async fn execute<Port>(
        self,
        port: &Port,
        request: &LegacyFolderAssignmentRequestV1,
        proof: &ValidatedBrowserMutationProof,
    ) -> Result<LegacyFolderAssignmentExecutionV1, LegacyFolderAssignmentErrorV1>
    where
        Port: LegacyFolderAssignmentAtomicPortV1,
    {
        let browser_fence = LegacyFolderAssignmentBrowserFenceV1::from_validated_proof(proof);
        if request.actor_id != Some(browser_fence.actor_id()) {
            return Err(LegacyFolderAssignmentErrorV1::Unauthorized);
        }
        self.execute_fenced(port, request, &browser_fence).await
    }

    async fn execute_fenced<Port>(
        self,
        port: &Port,
        request: &LegacyFolderAssignmentRequestV1,
        browser_fence: &LegacyFolderAssignmentBrowserFenceV1,
    ) -> Result<LegacyFolderAssignmentExecutionV1, LegacyFolderAssignmentErrorV1>
    where
        Port: LegacyFolderAssignmentAtomicPortV1,
    {
        let command = self.prepare(request)?;
        if command.fence().authority().actor_id() != browser_fence.actor_id() {
            return Err(LegacyFolderAssignmentErrorV1::Unauthorized);
        }
        let (receipt, replayed) = match port
            .execute_atomic(&command, browser_fence)
            .await
            .map_err(map_atomic_error)?
        {
            LegacyFolderAssignmentAtomicOutcomeV1::Applied(receipt) => (receipt, false),
            LegacyFolderAssignmentAtomicOutcomeV1::Replay(receipt) => (receipt, true),
        };
        let success = match (&command, receipt.affected_count()) {
            (LegacyFolderAssignmentCommandV1::Add { .. }, Some(added_count)) => {
                LegacyFolderAssignmentSuccessV1::Added { added_count }
            }
            (LegacyFolderAssignmentCommandV1::Remove { .. }, Some(removed_count)) => {
                LegacyFolderAssignmentSuccessV1::Removed { removed_count }
            }
            (LegacyFolderAssignmentCommandV1::Move { .. }, None) => {
                LegacyFolderAssignmentSuccessV1::MoveVoid
            }
            _ => return Err(LegacyFolderAssignmentErrorV1::Internal),
        };
        Ok(LegacyFolderAssignmentExecutionV1 {
            success,
            effects: receipt.effects,
            replayed,
        })
    }
}

fn map_atomic_error(error: LegacyFolderAssignmentAtomicErrorV1) -> LegacyFolderAssignmentErrorV1 {
    match error {
        LegacyFolderAssignmentAtomicErrorV1::TargetMissing
        | LegacyFolderAssignmentAtomicErrorV1::AccessDenied
        | LegacyFolderAssignmentAtomicErrorV1::CrossTenant
        | LegacyFolderAssignmentAtomicErrorV1::StaleAuthority => {
            LegacyFolderAssignmentErrorV1::TargetNotFound
        }
        LegacyFolderAssignmentAtomicErrorV1::Conflict
        | LegacyFolderAssignmentAtomicErrorV1::InFlight => LegacyFolderAssignmentErrorV1::Conflict,
        LegacyFolderAssignmentAtomicErrorV1::Unavailable => {
            LegacyFolderAssignmentErrorV1::AuthorityUnavailable
        }
        LegacyFolderAssignmentAtomicErrorV1::Corrupt => LegacyFolderAssignmentErrorV1::Internal,
    }
}

fn cap_uuid(value: &str) -> Result<String, LegacyFolderAssignmentErrorV1> {
    LegacyCapNanoId::parse(value.to_owned())
        .map(|legacy_id| legacy_id.mapped_uuid().to_string())
        .map_err(|_| LegacyFolderAssignmentErrorV1::TargetNotFound)
}

fn map_folder_id(value: &str) -> Result<FolderId, LegacyFolderAssignmentErrorV1> {
    FolderId::parse(&cap_uuid(value)?).map_err(|_| LegacyFolderAssignmentErrorV1::Internal)
}

fn map_video_id(value: &str) -> Result<VideoId, LegacyFolderAssignmentErrorV1> {
    cap_uuid(value)?
        .parse::<VideoId>()
        .map_err(|_| LegacyFolderAssignmentErrorV1::Internal)
}

fn map_space_id(value: &str) -> Result<SpaceId, LegacyFolderAssignmentErrorV1> {
    SpaceId::parse(&cap_uuid(value)?).map_err(|_| LegacyFolderAssignmentErrorV1::Internal)
}

fn map_scope(
    value: &str,
    active_organization_id: OrganizationId,
) -> Result<LegacyFolderAssignmentScopeV1, LegacyFolderAssignmentErrorV1> {
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

fn map_video_list(values: &[String]) -> Result<Vec<VideoId>, LegacyFolderAssignmentErrorV1> {
    if values.is_empty() || values.len() > MAX_LEGACY_FOLDER_ASSIGNMENT_VIDEO_IDS {
        return Err(LegacyFolderAssignmentErrorV1::Invalid);
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

fn fingerprint_list(
    action: LegacyFolderAssignmentActionV1,
    folder_id: FolderId,
    video_ids: &[VideoId],
    scope: LegacyFolderAssignmentScopeV1,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"frame-legacy-folder-assignment-v1\0");
    hasher.update([match action {
        LegacyFolderAssignmentActionV1::Add => 0,
        LegacyFolderAssignmentActionV1::Remove => 1,
        LegacyFolderAssignmentActionV1::Move => 2,
    }]);
    hash_value(&mut hasher, &folder_id.to_string());
    hasher.update((video_ids.len() as u64).to_be_bytes());
    for video_id in video_ids {
        hash_value(&mut hasher, &video_id.to_string());
    }
    hash_scope(&mut hasher, scope);
    hasher.finalize().into()
}

fn fingerprint_move(
    video_id: VideoId,
    folder_id: Option<FolderId>,
    scope: Option<LegacyFolderAssignmentScopeV1>,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"frame-legacy-folder-assignment-v1\0");
    hasher.update([2]);
    hash_value(&mut hasher, &video_id.to_string());
    match folder_id {
        Some(folder_id) => {
            hasher.update([1]);
            hash_value(&mut hasher, &folder_id.to_string());
        }
        None => hasher.update([0]),
    }
    match scope {
        Some(scope) => {
            hasher.update([1]);
            hash_scope(&mut hasher, scope);
        }
        None => hasher.update([0]),
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const FOLDER: &str = "0123456789abcde";
    const VIDEO_ONE: &str = "1123456789abcde";
    const VIDEO_TWO: &str = "2123456789abcde";
    const SCOPE: &str = "3123456789abcde";

    fn actor() -> UserId {
        UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a8").expect("actor")
    }

    fn mapped_organization(value: &str) -> OrganizationId {
        let mapped = LegacyCapNanoId::parse(value)
            .expect("legacy ID")
            .mapped_uuid()
            .to_string();
        OrganizationId::parse(&mapped).expect("organization")
    }

    fn request(input: LegacyFolderAssignmentInputV1) -> LegacyFolderAssignmentRequestV1 {
        LegacyFolderAssignmentRequestV1 {
            credential: Some(LegacyFolderAssignmentCredentialV1::Session),
            actor_id: Some(actor()),
            active_organization_id: Some(mapped_organization(SCOPE)),
            idempotency_key: Some("folder-command-0001".into()),
            input,
        }
    }

    fn add_request(videos: Vec<&str>) -> LegacyFolderAssignmentRequestV1 {
        request(LegacyFolderAssignmentInputV1::Add {
            legacy_folder_id: FOLDER.into(),
            legacy_video_ids: videos.into_iter().map(String::from).collect(),
            legacy_scope_id: SCOPE.into(),
        })
    }

    #[test]
    fn profiles_freeze_all_three_direct_sources_and_shared_closures() {
        let cases = [
            (
                LegacyFolderAssignmentAdapterV1::add_videos_to_folder(),
                LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID,
                LEGACY_ADD_VIDEOS_TO_FOLDER_IDENTITY,
                "apps/web/actions/folders/add-videos.ts",
                "cb4bcfab7d466e54fa77c09fdc4bac24d4041468c5c857b32ea0038f195132aa",
                8,
            ),
            (
                LegacyFolderAssignmentAdapterV1::remove_videos_from_folder(),
                LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID,
                LEGACY_REMOVE_VIDEOS_FROM_FOLDER_IDENTITY,
                "apps/web/actions/folders/remove-videos.ts",
                "f4ce4a28ff1c3f8f2fc23779606a7530945f47fc2e44f49536687ed6209a2d5f",
                7,
            ),
            (
                LegacyFolderAssignmentAdapterV1::move_video_to_folder(),
                LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID,
                LEGACY_MOVE_VIDEO_TO_FOLDER_IDENTITY,
                "apps/web/actions/folders/moveVideoToFolder.ts",
                "08f943871c4bdc0f931e140f994dff77c27f249fa3585cc50c1dbd6b8241c045",
                7,
            ),
        ];
        for (adapter, operation, identity, direct_path, direct_hash, source_count) in cases {
            let profile = adapter.profile();
            assert_eq!(profile.operation_id, operation);
            assert_eq!(profile.legacy_identity, identity);
            assert_eq!(profile.pinned_commit, LEGACY_FOLDER_ASSIGNMENT_CAP_COMMIT);
            assert_eq!(profile.kind, "server_action");
            assert_eq!(profile.method, "ACTION");
            assert_eq!(profile.authentication, "session");
            assert_eq!(profile.policy, "organization_library.v1");
            assert_eq!(profile.content_type, "application/json");
            assert_eq!(profile.max_body_bytes, 262_144);
            assert!(profile.tenant_non_disclosure);
            assert_eq!(profile.protected_gates, ["released_legacy_client_e2e"]);
            assert_eq!(profile.sources.len(), source_count);
            assert_eq!(profile.sources[0].path, direct_path);
            assert_eq!(profile.sources[0].sha256, direct_hash);
            assert_eq!(
                profile.required_retry,
                LEGACY_FOLDER_ASSIGNMENT_REQUIRED_RETRY
            );
        }
    }

    #[test]
    fn every_source_pin_is_a_lowercase_complete_sha256_and_transitives_are_retained() {
        for pins in [
            LEGACY_ADD_VIDEOS_TO_FOLDER_SOURCES,
            LEGACY_REMOVE_VIDEOS_FROM_FOLDER_SOURCES,
            LEGACY_MOVE_VIDEO_TO_FOLDER_SOURCES,
        ] {
            for pin in pins {
                assert!(!pin.path.is_empty());
                assert_eq!(pin.sha256.len(), 64);
                assert!(
                    pin.sha256
                        .bytes()
                        .all(|byte| { byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase() })
                );
            }
            for required in [
                "packages/database/auth/session.ts",
                "packages/database/auth/auth-options.ts",
                "packages/database/schema.ts",
                "packages/web-domain/src/Folder.ts",
                "packages/web-domain/src/Space.ts",
                "packages/web-domain/src/Video.ts",
            ] {
                assert!(pins.iter().any(|pin| pin.path == required));
            }
        }
        assert!(
            LEGACY_ADD_VIDEOS_TO_FOLDER_SOURCES
                .iter()
                .any(|pin| pin.path == "packages/database/helpers.ts")
        );
    }

    #[test]
    fn observed_bugs_and_frame_requirements_are_separate_profile_fields() {
        let add = LEGACY_ADD_VIDEOS_TO_FOLDER_PROFILE;
        assert_eq!(
            add.observed_authorization,
            LegacyFolderAssignmentObservedAuthorizationV1::GlobalFolderLookupAndActorOwnedVideoFilter
        );
        assert_eq!(
            add.required_authorization,
            LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo
        );
        assert_eq!(
            LEGACY_REMOVE_VIDEOS_FROM_FOLDER_PROFILE.required_authorization,
            LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantFolderScopeAndEveryActorOwnedVideo
        );
        assert_eq!(
            LEGACY_MOVE_VIDEO_TO_FOLDER_PROFILE.required_authorization,
            LegacyFolderAssignmentRequiredAuthorizationV1::SessionActorActiveTenantManagerSelectedContextAndTenantVideo
        );
        assert_eq!(
            add.observed_retry,
            LegacyFolderAssignmentObservedRetryV1::NonAtomicInsertThenUpdate
        );
        assert_eq!(
            add.required_retry.atomicity,
            LegacyFolderAssignmentAtomicityV1::AuthorityMutationOutcomeAndJournalInOneTransaction
        );
        assert_eq!(
            add.required_mutation,
            LegacyFolderAssignmentRequiredMutationV1::AddScopedMembershipOrSetFolder
        );
        assert_eq!(
            LEGACY_REMOVE_VIDEOS_FROM_FOLDER_PROFILE.required_mutation,
            LegacyFolderAssignmentRequiredMutationV1::RemoveMatchingScopedFolderAndDirectMatch
        );
        assert_eq!(
            LEGACY_MOVE_VIDEO_TO_FOLDER_PROFILE.observed_authorization,
            LegacyFolderAssignmentObservedAuthorizationV1::GlobalVideoLookupAndActiveOrganizationFolderLookup
        );
        assert_eq!(
            LEGACY_MOVE_VIDEO_TO_FOLDER_PROFILE.required_mutation,
            LegacyFolderAssignmentRequiredMutationV1::MoveOnlySelectedStorageContext
        );
    }

    #[test]
    fn lists_are_bounded_deduplicated_and_canonicalized_before_the_port() {
        let adapter = LegacyFolderAssignmentAdapterV1::add_videos_to_folder();
        let first = adapter
            .prepare(&add_request(vec![VIDEO_TWO, VIDEO_ONE, VIDEO_TWO]))
            .expect("first");
        let second = adapter
            .prepare(&add_request(vec![VIDEO_ONE, VIDEO_TWO]))
            .expect("second");
        let LegacyFolderAssignmentCommandV1::Add { video_ids, .. } = &first else {
            panic!("add command");
        };
        assert_eq!(video_ids.len(), 2);
        assert!(video_ids[0].to_string() < video_ids[1].to_string());
        assert_eq!(
            first.fence().request_fingerprint(),
            second.fence().request_fingerprint()
        );

        assert_eq!(
            adapter.prepare(&add_request(Vec::new())),
            Err(LegacyFolderAssignmentErrorV1::Invalid)
        );
        let too_many = vec![VIDEO_ONE; MAX_LEGACY_FOLDER_ASSIGNMENT_VIDEO_IDS + 1];
        assert_eq!(
            adapter.prepare(&add_request(too_many)),
            Err(LegacyFolderAssignmentErrorV1::Invalid)
        );
    }

    #[test]
    fn cap_ids_map_forward_only_and_scope_uses_the_trusted_active_organization() {
        let command = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&add_request(vec![VIDEO_ONE]))
            .expect("command");
        let LegacyFolderAssignmentCommandV1::Add {
            folder_id, scope, ..
        } = command
        else {
            panic!("add command");
        };
        assert_eq!(
            folder_id.to_string(),
            "2a6a8a87-d5ca-8c83-8666-2e92c2a69404"
        );
        assert_eq!(
            scope,
            LegacyFolderAssignmentScopeV1::OrganizationLibrary {
                organization_id: mapped_organization(SCOPE)
            }
        );

        let mut space_request = add_request(vec![VIDEO_ONE]);
        let LegacyFolderAssignmentInputV1::Add {
            legacy_scope_id, ..
        } = &mut space_request.input
        else {
            unreachable!()
        };
        *legacy_scope_id = VIDEO_TWO.into();
        let LegacyFolderAssignmentCommandV1::Add { scope, .. } =
            LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
                .prepare(&space_request)
                .expect("space command")
        else {
            unreachable!()
        };
        assert!(matches!(scope, LegacyFolderAssignmentScopeV1::Space { .. }));
    }

    #[test]
    fn malformed_or_arbitrary_targets_share_one_non_disclosing_error() {
        let adapter = LegacyFolderAssignmentAdapterV1::add_videos_to_folder();
        for input in [
            LegacyFolderAssignmentInputV1::Add {
                legacy_folder_id: "not-a-cap-id".into(),
                legacy_video_ids: vec![VIDEO_ONE.into()],
                legacy_scope_id: SCOPE.into(),
            },
            LegacyFolderAssignmentInputV1::Add {
                legacy_folder_id: FOLDER.into(),
                legacy_video_ids: vec!["not-a-cap-id".into()],
                legacy_scope_id: SCOPE.into(),
            },
            LegacyFolderAssignmentInputV1::Add {
                legacy_folder_id: FOLDER.into(),
                legacy_video_ids: vec![VIDEO_ONE.into()],
                legacy_scope_id: "not-a-cap-id".into(),
            },
        ] {
            assert_eq!(
                adapter.prepare(&request(input)),
                Err(LegacyFolderAssignmentErrorV1::TargetNotFound)
            );
        }
        for error in [
            LegacyFolderAssignmentAtomicErrorV1::TargetMissing,
            LegacyFolderAssignmentAtomicErrorV1::AccessDenied,
            LegacyFolderAssignmentAtomicErrorV1::CrossTenant,
            LegacyFolderAssignmentAtomicErrorV1::StaleAuthority,
        ] {
            assert_eq!(
                map_atomic_error(error),
                LegacyFolderAssignmentErrorV1::TargetNotFound
            );
        }
    }

    #[test]
    fn session_active_tenant_and_idempotency_fence_are_mandatory() {
        let adapter = LegacyFolderAssignmentAdapterV1::add_videos_to_folder();
        let mut candidate = add_request(vec![VIDEO_ONE]);
        candidate.credential = Some(LegacyFolderAssignmentCredentialV1::ApiKey);
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyFolderAssignmentErrorV1::Unauthorized)
        );
        let mut candidate = add_request(vec![VIDEO_ONE]);
        candidate.active_organization_id = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyFolderAssignmentErrorV1::Unauthorized)
        );
        let mut candidate = add_request(vec![VIDEO_ONE]);
        candidate.idempotency_key = None;
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyFolderAssignmentErrorV1::IdempotencyRequired)
        );
        let mut candidate = add_request(vec![VIDEO_ONE]);
        candidate.idempotency_key = Some("short".into());
        assert_eq!(
            adapter.prepare(&candidate),
            Err(LegacyFolderAssignmentErrorV1::Invalid)
        );
    }

    #[test]
    fn action_mismatch_is_rejected_before_any_port_call() {
        assert_eq!(
            LegacyFolderAssignmentAdapterV1::remove_videos_from_folder()
                .prepare(&add_request(vec![VIDEO_ONE])),
            Err(LegacyFolderAssignmentErrorV1::Invalid)
        );
    }

    #[test]
    fn request_and_command_debug_output_do_not_disclose_targets_or_keys() {
        let request = add_request(vec![VIDEO_ONE]);
        let request_debug = format!("{request:?}");
        assert!(!request_debug.contains(FOLDER));
        assert!(!request_debug.contains(VIDEO_ONE));
        assert!(!request_debug.contains(SCOPE));
        assert!(!request_debug.contains("folder-command-0001"));
        assert!(!request_debug.contains(&actor().to_string()));

        let command = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&request)
            .expect("command");
        let command_debug = format!("{command:?}");
        assert!(!command_debug.contains(&map_folder_id(FOLDER).expect("folder").to_string()));
        assert!(!command_debug.contains("folder-command-0001"));
    }

    #[test]
    fn exact_legacy_success_projection_handles_pluralization_and_move_void() {
        let one = LegacyFolderAssignmentSuccessV1::Added { added_count: 1 };
        assert_eq!(one.object_success(), Some(true));
        assert_eq!(one.message().as_deref(), Some("1 video added to folder"));
        assert_eq!(one.added_count(), Some(1));
        assert_eq!(one.removed_count(), None);

        let many = LegacyFolderAssignmentSuccessV1::Removed { removed_count: 2 };
        assert_eq!(many.object_success(), Some(true));
        assert_eq!(
            many.message().as_deref(),
            Some("2 videos removed from folder")
        );
        assert_eq!(many.removed_count(), Some(2));

        let moved = LegacyFolderAssignmentSuccessV1::MoveVoid;
        assert_eq!(moved.object_success(), None);
        assert_eq!(moved.message(), None);
        assert!(moved.resolves_void());
    }

    #[test]
    fn effects_are_typed_bounded_deduplicated_and_always_include_caps() {
        let folder_id = map_folder_id(FOLDER).expect("folder");
        let target = LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id };
        let effects = LegacyFolderAssignmentEffectsV1::from_authorized_targets([target, target])
            .expect("effects");
        assert!(effects.invalidates_caps());
        assert_eq!(effects.targets(), &[target]);

        let too_many = (0..=MAX_LEGACY_FOLDER_ASSIGNMENT_INVALIDATIONS)
            .map(|_| LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: FolderId::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            LegacyFolderAssignmentEffectsV1::from_authorized_targets(too_many),
            Err(LegacyFolderAssignmentErrorV1::Internal)
        );

        let command = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&add_request(vec![VIDEO_ONE]))
            .expect("command");
        let empty = LegacyFolderAssignmentEffectsV1::from_authorized_targets([]).expect("empty");
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &command,
                Some(1),
                empty,
                authorized_context(&command),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt),
            "caps alone cannot replace the source-required target-folder invalidation"
        );

        let mut space_request = add_request(vec![VIDEO_ONE]);
        let LegacyFolderAssignmentInputV1::Add {
            legacy_scope_id, ..
        } = &mut space_request.input
        else {
            unreachable!()
        };
        *legacy_scope_id = VIDEO_TWO.into();
        let space_command = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&space_request)
            .expect("space command");
        let only_folder = LegacyFolderAssignmentEffectsV1::from_authorized_targets([target])
            .expect("folder target");
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &space_command,
                Some(1),
                only_folder,
                authorized_context(&space_command),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt),
            "space actions must invalidate their exact space-folder view"
        );
    }

    struct RecordingPort {
        commands: Mutex<Vec<LegacyFolderAssignmentCommandV1>>,
        result: Mutex<
            Option<
                Result<LegacyFolderAssignmentAtomicOutcomeV1, LegacyFolderAssignmentAtomicErrorV1>,
            >,
        >,
    }

    impl RecordingPort {
        fn returning(
            result: Result<
                LegacyFolderAssignmentAtomicOutcomeV1,
                LegacyFolderAssignmentAtomicErrorV1,
            >,
        ) -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                result: Mutex::new(Some(result)),
            }
        }
    }

    #[async_trait]
    impl LegacyFolderAssignmentAtomicPortV1 for RecordingPort {
        async fn execute_atomic(
            &self,
            command: &LegacyFolderAssignmentCommandV1,
            browser_fence: &LegacyFolderAssignmentBrowserFenceV1,
        ) -> Result<LegacyFolderAssignmentAtomicOutcomeV1, LegacyFolderAssignmentAtomicErrorV1>
        {
            assert_eq!(
                command.fence().authority().actor_id(),
                browser_fence.actor_id()
            );
            self.commands
                .lock()
                .expect("commands")
                .push(command.clone());
            self.result
                .lock()
                .expect("result")
                .take()
                .expect("one result")
        }
    }

    fn authorized_context(
        command: &LegacyFolderAssignmentCommandV1,
    ) -> LegacyFolderAssignmentAuthorizedContextV1 {
        let target_folder_space_id = match command {
            LegacyFolderAssignmentCommandV1::Add { scope, .. }
            | LegacyFolderAssignmentCommandV1::Remove { scope, .. } => Some(match scope {
                LegacyFolderAssignmentScopeV1::Space { space_id } => *space_id,
                LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. } => {
                    map_space_id(VIDEO_TWO).expect("target folder space")
                }
            }),
            LegacyFolderAssignmentCommandV1::Move {
                folder_id: Some(_),
                scope,
                ..
            } => Some(match scope {
                Some(LegacyFolderAssignmentScopeV1::Space { space_id }) => *space_id,
                Some(LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. }) | None => {
                    map_space_id(VIDEO_TWO).expect("target folder space")
                }
            }),
            LegacyFolderAssignmentCommandV1::Move {
                folder_id: None, ..
            } => None,
        };
        LegacyFolderAssignmentAuthorizedContextV1::new(
            command,
            target_folder_space_id,
            None,
            None,
            None,
        )
        .expect("authorized context")
    }

    fn receipt(
        command: &LegacyFolderAssignmentCommandV1,
        count: Option<u16>,
    ) -> LegacyFolderAssignmentMutationReceiptV1 {
        let mut targets = Vec::new();
        match command {
            LegacyFolderAssignmentCommandV1::Add { folder_id, .. }
            | LegacyFolderAssignmentCommandV1::Remove { folder_id, .. } => {
                targets.push(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                    folder_id: *folder_id,
                });
                let space_id = authorized_context(command)
                    .target_folder_space_id()
                    .expect("folder space");
                targets.push(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                    space_id,
                    folder_id: *folder_id,
                });
            }
            LegacyFolderAssignmentCommandV1::Move {
                folder_id, scope, ..
            } => {
                if let Some(folder_id) = folder_id {
                    targets.push(LegacyFolderAssignmentInvalidationTargetV1::Folder {
                        folder_id: *folder_id,
                    });
                }
                if let Some(LegacyFolderAssignmentScopeV1::Space { space_id }) = scope {
                    targets.push(LegacyFolderAssignmentInvalidationTargetV1::SpaceRoot {
                        space_id: *space_id,
                    });
                    if let Some(folder_id) = folder_id {
                        targets.push(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                            space_id: *space_id,
                            folder_id: *folder_id,
                        });
                    }
                } else if matches!(
                    scope,
                    Some(LegacyFolderAssignmentScopeV1::OrganizationLibrary { .. })
                ) && let Some(folder_id) = folder_id
                {
                    targets.push(LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                        space_id: authorized_context(command)
                            .target_folder_space_id()
                            .expect("target folder space"),
                        folder_id: *folder_id,
                    });
                }
            }
        }
        LegacyFolderAssignmentMutationReceiptV1::new(
            command,
            count,
            LegacyFolderAssignmentEffectsV1::from_authorized_targets(targets).expect("effects"),
            authorized_context(command),
        )
        .expect("receipt")
    }

    #[tokio::test]
    async fn adapter_calls_one_atomic_boundary_and_returns_exact_add_success() {
        let adapter = LegacyFolderAssignmentAdapterV1::add_videos_to_folder();
        let request = add_request(vec![VIDEO_ONE, VIDEO_TWO]);
        let command = adapter.prepare(&request).expect("command");
        let port = RecordingPort::returning(Ok(LegacyFolderAssignmentAtomicOutcomeV1::Applied(
            receipt(&command, Some(2)),
        )));
        let browser_fence = LegacyFolderAssignmentBrowserFenceV1::fixture(actor());
        let execution = adapter
            .execute_fenced(&port, &request, &browser_fence)
            .await
            .expect("execution");
        assert_eq!(
            execution.success(),
            &LegacyFolderAssignmentSuccessV1::Added { added_count: 2 }
        );
        assert!(!execution.replayed());
        assert!(execution.mutation_was_applied());
        assert_eq!(port.commands.lock().expect("commands").len(), 1);
    }

    #[tokio::test]
    async fn replay_returns_the_journaled_success_without_claiming_a_new_mutation() {
        let adapter = LegacyFolderAssignmentAdapterV1::move_video_to_folder();
        let move_request = request(LegacyFolderAssignmentInputV1::Move {
            legacy_video_id: VIDEO_ONE.into(),
            legacy_folder_id: Some(FOLDER.into()),
            legacy_scope_id: Some(SCOPE.into()),
        });
        let command = adapter.prepare(&move_request).expect("command");
        let port = RecordingPort::returning(Ok(LegacyFolderAssignmentAtomicOutcomeV1::Replay(
            receipt(&command, None),
        )));
        let browser_fence = LegacyFolderAssignmentBrowserFenceV1::fixture(actor());
        let execution = adapter
            .execute_fenced(&port, &move_request, &browser_fence)
            .await
            .expect("execution");
        assert_eq!(
            execution.success(),
            &LegacyFolderAssignmentSuccessV1::MoveVoid
        );
        assert!(execution.replayed());
        assert!(!execution.mutation_was_applied());
    }

    #[tokio::test]
    async fn every_authority_denial_projects_to_the_same_public_not_found() {
        for atomic_error in [
            LegacyFolderAssignmentAtomicErrorV1::TargetMissing,
            LegacyFolderAssignmentAtomicErrorV1::AccessDenied,
            LegacyFolderAssignmentAtomicErrorV1::CrossTenant,
            LegacyFolderAssignmentAtomicErrorV1::StaleAuthority,
        ] {
            let adapter = LegacyFolderAssignmentAdapterV1::add_videos_to_folder();
            let request = add_request(vec![VIDEO_ONE]);
            let port = RecordingPort::returning(Err(atomic_error));
            let browser_fence = LegacyFolderAssignmentBrowserFenceV1::fixture(actor());
            assert_eq!(
                adapter
                    .execute_fenced(&port, &request, &browser_fence)
                    .await,
                Err(LegacyFolderAssignmentErrorV1::TargetNotFound)
            );
        }
    }

    #[tokio::test]
    async fn browser_fence_is_actor_bound_and_redacted_before_the_port() {
        let adapter = LegacyFolderAssignmentAdapterV1::add_videos_to_folder();
        let request = add_request(vec![VIDEO_ONE]);
        let command = adapter.prepare(&request).expect("command");
        let port = RecordingPort::returning(Ok(LegacyFolderAssignmentAtomicOutcomeV1::Applied(
            receipt(&command, Some(1)),
        )));
        let other = UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a9").expect("other");
        let browser_fence = LegacyFolderAssignmentBrowserFenceV1::fixture(other);
        let debug = format!("{browser_fence:?}");
        assert!(!debug.contains(&other.to_string()));
        assert_eq!(
            adapter
                .execute_fenced(&port, &request, &browser_fence)
                .await,
            Err(LegacyFolderAssignmentErrorV1::Unauthorized)
        );
        assert!(port.commands.lock().expect("commands").is_empty());
    }

    #[test]
    fn receipts_bind_database_discovered_original_parent_and_real_space_invalidations() {
        let move_request = request(LegacyFolderAssignmentInputV1::Move {
            legacy_video_id: VIDEO_ONE.into(),
            legacy_folder_id: Some(FOLDER.into()),
            legacy_scope_id: Some(SCOPE.into()),
        });
        let command = LegacyFolderAssignmentAdapterV1::move_video_to_folder()
            .prepare(&move_request)
            .expect("move command");
        let LegacyFolderAssignmentCommandV1::Move {
            folder_id: Some(target),
            ..
        } = &command
        else {
            unreachable!()
        };
        let target = *target;
        let target_space = map_space_id(VIDEO_TWO).expect("target space");
        let original = map_folder_id("4123456789abcde").expect("original");
        let original_parent = map_folder_id("5123456789abcde").expect("original parent");
        let target_parent = map_folder_id("6123456789abcde").expect("target parent");
        let context = LegacyFolderAssignmentAuthorizedContextV1::new(
            &command,
            Some(target_space),
            Some(original),
            Some(original_parent),
            Some(target_parent),
        )
        .expect("authorized graph context");
        let only_target = LegacyFolderAssignmentEffectsV1::from_authorized_targets([
            LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id: target },
            LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                space_id: target_space,
                folder_id: target,
            },
        ])
        .expect("target effects");
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(&command, None, only_target, context,),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
        );
        let complete = LegacyFolderAssignmentEffectsV1::from_authorized_targets([
            LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id: target },
            LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                space_id: target_space,
                folder_id: target,
            },
            LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: original,
            },
            LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: original_parent,
            },
            LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: target_parent,
            },
        ])
        .expect("complete effects");
        assert!(
            LegacyFolderAssignmentMutationReceiptV1::new(&command, None, complete, context).is_ok()
        );

        let clear_request = request(LegacyFolderAssignmentInputV1::Move {
            legacy_video_id: VIDEO_ONE.into(),
            legacy_folder_id: None,
            legacy_scope_id: None,
        });
        let clear = LegacyFolderAssignmentAdapterV1::move_video_to_folder()
            .prepare(&clear_request)
            .expect("clear command");
        let clear_context = LegacyFolderAssignmentAuthorizedContextV1::new(
            &clear,
            None,
            Some(original),
            Some(original_parent),
            None,
        )
        .expect("clear graph context");
        let empty = LegacyFolderAssignmentEffectsV1::from_authorized_targets([]).expect("empty");
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(&clear, None, empty, clear_context),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
        );
        let original_effect = LegacyFolderAssignmentEffectsV1::from_authorized_targets([
            LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: original,
            },
        ])
        .expect("original folder effect");
        assert!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &clear,
                None,
                original_effect,
                clear_context,
            )
            .is_ok()
        );
    }

    #[test]
    fn nullable_personal_folders_require_folder_but_not_space_invalidation() {
        let add = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&add_request(vec![VIDEO_ONE]))
            .expect("organization-library add");
        let LegacyFolderAssignmentCommandV1::Add { folder_id, .. } = &add else {
            unreachable!()
        };
        let personal_context =
            LegacyFolderAssignmentAuthorizedContextV1::new(&add, None, None, None, None)
                .expect("nullable personal folder");
        let personal_effects = LegacyFolderAssignmentEffectsV1::from_authorized_targets([
            LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: *folder_id,
            },
        ])
        .expect("personal-folder effects");
        assert!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &add,
                Some(1),
                personal_effects,
                personal_context,
            )
            .is_ok()
        );

        for scope in [Some(SCOPE), None] {
            let move_request = request(LegacyFolderAssignmentInputV1::Move {
                legacy_video_id: VIDEO_ONE.into(),
                legacy_folder_id: Some(FOLDER.into()),
                legacy_scope_id: scope.map(String::from),
            });
            let command = LegacyFolderAssignmentAdapterV1::move_video_to_folder()
                .prepare(&move_request)
                .expect("personal-folder move");
            let LegacyFolderAssignmentCommandV1::Move {
                folder_id: Some(folder_id),
                ..
            } = &command
            else {
                unreachable!()
            };
            let context =
                LegacyFolderAssignmentAuthorizedContextV1::new(&command, None, None, None, None)
                    .expect("nullable personal target");
            let effects = LegacyFolderAssignmentEffectsV1::from_authorized_targets([
                LegacyFolderAssignmentInvalidationTargetV1::Folder {
                    folder_id: *folder_id,
                },
            ])
            .expect("personal target effects");
            assert!(
                LegacyFolderAssignmentMutationReceiptV1::new(&command, None, effects, context,)
                    .is_ok()
            );
        }

        let mut real_space_request = add_request(vec![VIDEO_ONE]);
        let LegacyFolderAssignmentInputV1::Add {
            legacy_scope_id, ..
        } = &mut real_space_request.input
        else {
            unreachable!()
        };
        *legacy_scope_id = VIDEO_TWO.into();
        let real_space = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&real_space_request)
            .expect("real-space add");
        assert_eq!(
            LegacyFolderAssignmentAuthorizedContextV1::new(&real_space, None, None, None, None,),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt),
            "real-space folders must retain an exact space binding"
        );
    }

    #[test]
    fn receipts_reject_counts_that_could_not_come_from_the_prepared_command() {
        let command = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&add_request(vec![VIDEO_ONE]))
            .expect("command");
        let effects = || {
            let LegacyFolderAssignmentCommandV1::Add { folder_id, .. } = command else {
                unreachable!()
            };
            let space_id = authorized_context(&command)
                .target_folder_space_id()
                .expect("folder space");
            LegacyFolderAssignmentEffectsV1::from_authorized_targets([
                LegacyFolderAssignmentInvalidationTargetV1::Folder { folder_id },
                LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                    space_id,
                    folder_id,
                },
            ])
            .expect("effects")
        };
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &command,
                None,
                effects(),
                authorized_context(&command),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
        );
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &command,
                Some(0),
                effects(),
                authorized_context(&command),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
        );
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &command,
                Some(2),
                effects(),
                authorized_context(&command),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt)
        );

        let two_video_command = LegacyFolderAssignmentAdapterV1::add_videos_to_folder()
            .prepare(&add_request(vec![VIDEO_ONE, VIDEO_TWO]))
            .expect("two-video command");
        let LegacyFolderAssignmentCommandV1::Add { folder_id, .. } = &two_video_command else {
            unreachable!()
        };
        let space_id = authorized_context(&two_video_command)
            .target_folder_space_id()
            .expect("folder space");
        let two_video_effects = LegacyFolderAssignmentEffectsV1::from_authorized_targets([
            LegacyFolderAssignmentInvalidationTargetV1::Folder {
                folder_id: *folder_id,
            },
            LegacyFolderAssignmentInvalidationTargetV1::SpaceFolder {
                space_id,
                folder_id: *folder_id,
            },
        ])
        .expect("effects");
        assert_eq!(
            LegacyFolderAssignmentMutationReceiptV1::new(
                &two_video_command,
                Some(1),
                two_video_effects,
                authorized_context(&two_video_command),
            ),
            Err(LegacyFolderAssignmentAtomicErrorV1::Corrupt),
            "a fully authorized two-video command cannot report partial success"
        );
    }
}
