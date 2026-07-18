//! Source-pinned compatibility contracts for Cap's notification write actions.
//!
//! The pinned actions are small, but their database authority is not safe to
//! reproduce literally. `markAsRead(id)` updated a globally addressed row and
//! the no-id branch updated every notification for a recipient across all
//! organizations. `updatePreferences` spread a user snapshot loaded during
//! authentication, so a concurrent update to another preference branch could
//! be lost. Frame preserves the actions' observable `void` success and
//! dashboard revalidation while requiring a one-use browser proof, tenant- and
//! recipient-scoped notification writes, an atomic preferences JSON merge, and
//! a durable idempotency journal.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    IdempotencyKey, LegacyCapNanoId, NotificationId, OrganizationId, SessionId,
    SessionMutationGrantId, TimestampMillis, UserId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;

pub const LEGACY_NOTIFICATION_ACTIONS_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID: &str = "cap-v1-74a775753d3863c7";
pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID: &str = "cap-v1-1f6a43a05f2f297c";
pub const LEGACY_MARK_NOTIFICATIONS_READ_IDENTITY: &str =
    "action://apps/web/actions/notifications/mark-as-read.ts#markAsRead";
pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_IDENTITY: &str =
    "action://apps/web/actions/notifications/update-preferences.ts#updatePreferences";
pub const LEGACY_MARK_NOTIFICATIONS_READ_SOURCE_MANIFEST_SHA256: &str =
    "70fecd8bfb742ba205b1defde182f1c501410847591e34878545b13e06947a78";
pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_SOURCE_MANIFEST_SHA256: &str =
    "ccdd7b63a77a18c5367160996028c6d8d7deabe8bbac5f79941d397435ee50ca";
pub const LEGACY_NOTIFICATION_ACTION_POLICY: &str = "collaboration_notifications.v1";
pub const LEGACY_NOTIFICATION_ACTION_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_NOTIFICATION_ACTION_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_NOTIFICATION_DASHBOARD_REVALIDATION_PATH: &str = "/dashboard";
pub const LEGACY_NOTIFICATION_ACTION_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"];
pub const LEGACY_MARK_NOTIFICATIONS_READ_OBSERVED_FAILURES: &[&str] = &[
    "User not found",
    "unprojected session/user lookup error",
    "Error marking notification(s) as read",
    "unprojected dashboard revalidation error",
];
pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OBSERVED_FAILURES: &[&str] = &[
    "User not found",
    "unprojected session/user lookup error",
    "Error updating preferences",
];
pub const LEGACY_NOTIFICATION_ACTION_REQUIRED_PUBLIC_FAILURES: &[&str] = &[
    "Unauthorized",
    "Invalid input",
    "An idempotency key is required",
    "Notification request conflicts with current state",
    "Notification authority is unavailable",
    "Notification action failed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationSourceRoleV1 {
    Action,
    Caller,
    ReadProjection,
    Session,
    Schema,
    Identifier,
    Database,
    DependencyDeclaration,
    DependencyLock,
}

impl LegacyNotificationSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Caller => "caller",
            Self::ReadProjection => "read_projection",
            Self::Session => "session",
            Self::Schema => "schema",
            Self::Identifier => "identifier",
            Self::Database => "database",
            Self::DependencyDeclaration => "dependency_declaration",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
    pub role: LegacyNotificationSourceRoleV1,
}

pub const LEGACY_MARK_NOTIFICATIONS_READ_SOURCES: &[LegacyNotificationSourcePinV1] = &[
    LegacyNotificationSourcePinV1 {
        path: "apps/web/actions/notifications/mark-as-read.ts",
        sha256: "d25181538c6463e95f787902ed52dbb1eec758b14fcdbaa00a77e2408d35bd49",
        role: LegacyNotificationSourceRoleV1::Action,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/Notifications/NotificationItem.tsx",
        sha256: "d32121e41e5eee0cf84da0e992c14497378e28361c015ffa3d8b4637d42deab1",
        role: LegacyNotificationSourceRoleV1::Caller,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/Notifications/NotificationHeader.tsx",
        sha256: "2b748c4dbba2d943caaf67083ee9835bbbc68dbf25f124652e3c3f61570a4711",
        role: LegacyNotificationSourceRoleV1::Caller,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/Navbar/Top.tsx",
        sha256: "beae5ea8e688fd5c0d7d239cd61788e30e99964bd7323e8c93acdebf584919dd",
        role: LegacyNotificationSourceRoleV1::Caller,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/app/api/notifications/route.ts",
        sha256: "1c0571a385328c53ec106967a717201ed2aa04cbcfd108c419f03f8b51b3ae17",
        role: LegacyNotificationSourceRoleV1::ReadProjection,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyNotificationSourceRoleV1::Session,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyNotificationSourceRoleV1::Session,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyNotificationSourceRoleV1::Schema,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyNotificationSourceRoleV1::Identifier,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/index.ts",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyNotificationSourceRoleV1::Database,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
        role: LegacyNotificationSourceRoleV1::Identifier,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/package.json",
        sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
        role: LegacyNotificationSourceRoleV1::DependencyDeclaration,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/package.json",
        sha256: "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
        role: LegacyNotificationSourceRoleV1::DependencyDeclaration,
    },
    LegacyNotificationSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyNotificationSourceRoleV1::DependencyLock,
    },
];

pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_SOURCES: &[LegacyNotificationSourcePinV1] = &[
    LegacyNotificationSourcePinV1 {
        path: "apps/web/actions/notifications/update-preferences.ts",
        sha256: "c66025cde3f0b179440a60a4570368e335d474a4a323e83c2043111a3baf5ee8",
        role: LegacyNotificationSourceRoleV1::Action,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/settings/notifications/NotificationsSettings.tsx",
        sha256: "94b3d9ac1e93a7a46e7c1e20d942b16fc82cfc6b5c96e1563835acb8d70f910f",
        role: LegacyNotificationSourceRoleV1::Caller,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/dashboard-data.ts",
        sha256: "73115151676d808c5e5731fd717792d12a87bbfa2bd827c69d3fbf16ac42fdad",
        role: LegacyNotificationSourceRoleV1::ReadProjection,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyNotificationSourceRoleV1::Session,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyNotificationSourceRoleV1::Session,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyNotificationSourceRoleV1::Schema,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyNotificationSourceRoleV1::Identifier,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/index.ts",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyNotificationSourceRoleV1::Database,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/web-domain/src/User.ts",
        sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
        role: LegacyNotificationSourceRoleV1::Identifier,
    },
    LegacyNotificationSourcePinV1 {
        path: "apps/web/package.json",
        sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
        role: LegacyNotificationSourceRoleV1::DependencyDeclaration,
    },
    LegacyNotificationSourcePinV1 {
        path: "packages/database/package.json",
        sha256: "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
        role: LegacyNotificationSourceRoleV1::DependencyDeclaration,
    },
    LegacyNotificationSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyNotificationSourceRoleV1::DependencyLock,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationActionV1 {
    MarkAsRead,
    UpdatePreferences,
}

impl LegacyNotificationActionV1 {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::MarkAsRead => "mark_as_read",
            Self::UpdatePreferences => "update_preferences",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationObservedAuthorizationV1 {
    SessionActorThenGlobalNotificationIdOrRecipientOnly,
    SessionActorUserRowOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationRequiredAuthorizationV1 {
    SessionActorActiveTenantRecipientScopedNotification,
    SessionActorScopedPreferences,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationObservedMutationV1 {
    OverwriteReadTimeByGlobalIdOrRecipientAcrossOrganizations,
    ReplaceNotificationsBranchFromAuthenticationSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationRequiredMutationV1 {
    SetReadTimeOnlyForActorAndActiveTenantSelection,
    AtomicallyMergeNotificationsBranchPreservingOtherPreferences,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationInputShapeV1 {
    OptionalNotificationIdentifier,
    NotificationBooleanObjectWithOptionalAnonymousViews,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationObservedFailureV1 {
    ThrownAuthenticationOrCaughtMutationError,
    ThrownAuthenticationOrCaughtMutationAndRevalidationError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationObservedRetryDefectV1 {
    RetryOverwritesReadTimestamp,
    RetryCanOverwriteConcurrentPreferenceBranchesFromStaleSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationRequiredReplayV1 {
    ReturnOriginalJournaledVoidSuccessWithoutReapplyingMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationRequiredKeyReuseV1 {
    SameFingerprintReplaysDifferentFingerprintConflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationRequiredAtomicityV1 {
    BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationRequiredRetryV1 {
    pub replay: LegacyNotificationRequiredReplayV1,
    pub key_reuse: LegacyNotificationRequiredKeyReuseV1,
    pub atomicity: LegacyNotificationRequiredAtomicityV1,
}

pub const LEGACY_NOTIFICATION_ACTION_REQUIRED_RETRY: LegacyNotificationRequiredRetryV1 =
    LegacyNotificationRequiredRetryV1 {
        replay:
            LegacyNotificationRequiredReplayV1::ReturnOriginalJournaledVoidSuccessWithoutReapplyingMutation,
        key_reuse:
            LegacyNotificationRequiredKeyReuseV1::SameFingerprintReplaysDifferentFingerprintConflicts,
        atomicity:
            LegacyNotificationRequiredAtomicityV1::BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationProfileV1 {
    pub operation_id: &'static str,
    pub source_manifest_sha256: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub pinned_commit: &'static str,
    pub sources: &'static [LegacyNotificationSourcePinV1],
    pub authentication: &'static str,
    pub policy: &'static str,
    pub content_type: &'static str,
    pub max_body_bytes: usize,
    pub input: LegacyNotificationInputShapeV1,
    pub observed_authorization: LegacyNotificationObservedAuthorizationV1,
    pub required_authorization: LegacyNotificationRequiredAuthorizationV1,
    pub observed_mutation: LegacyNotificationObservedMutationV1,
    pub required_mutation: LegacyNotificationRequiredMutationV1,
    pub observed_success: &'static str,
    pub observed_failure: LegacyNotificationObservedFailureV1,
    pub observed_failure_messages: &'static [&'static str],
    pub required_public_failure_messages: &'static [&'static str],
    pub observed_revalidation_path: &'static str,
    pub observed_retry_defect: LegacyNotificationObservedRetryDefectV1,
    pub required_retry: LegacyNotificationRequiredRetryV1,
    pub observed_idempotency: &'static str,
    pub idempotency: &'static str,
    pub rate_limit_bucket: &'static str,
    pub provider_effect: Option<&'static str>,
    pub tenant_non_disclosure: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

pub const LEGACY_MARK_NOTIFICATIONS_READ_PROFILE: LegacyNotificationProfileV1 =
    LegacyNotificationProfileV1 {
        operation_id: LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID,
        source_manifest_sha256: LEGACY_MARK_NOTIFICATIONS_READ_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_MARK_NOTIFICATIONS_READ_IDENTITY,
        pinned_commit: LEGACY_NOTIFICATION_ACTIONS_CAP_COMMIT,
        sources: LEGACY_MARK_NOTIFICATIONS_READ_SOURCES,
        authentication: "session",
        policy: LEGACY_NOTIFICATION_ACTION_POLICY,
        content_type: LEGACY_NOTIFICATION_ACTION_CONTENT_TYPE,
        max_body_bytes: LEGACY_NOTIFICATION_ACTION_MAX_BODY_BYTES,
        input: LegacyNotificationInputShapeV1::OptionalNotificationIdentifier,
        observed_authorization:
            LegacyNotificationObservedAuthorizationV1::SessionActorThenGlobalNotificationIdOrRecipientOnly,
        required_authorization:
            LegacyNotificationRequiredAuthorizationV1::SessionActorActiveTenantRecipientScopedNotification,
        observed_mutation:
            LegacyNotificationObservedMutationV1::OverwriteReadTimeByGlobalIdOrRecipientAcrossOrganizations,
        required_mutation:
            LegacyNotificationRequiredMutationV1::SetReadTimeOnlyForActorAndActiveTenantSelection,
        observed_success: "void",
        observed_failure: LegacyNotificationObservedFailureV1::ThrownAuthenticationOrCaughtMutationError,
        observed_failure_messages: LEGACY_MARK_NOTIFICATIONS_READ_OBSERVED_FAILURES,
        required_public_failure_messages: LEGACY_NOTIFICATION_ACTION_REQUIRED_PUBLIC_FAILURES,
        observed_revalidation_path: LEGACY_NOTIFICATION_DASHBOARD_REVALIDATION_PATH,
        observed_retry_defect: LegacyNotificationObservedRetryDefectV1::RetryOverwritesReadTimestamp,
        required_retry: LEGACY_NOTIFICATION_ACTION_REQUIRED_RETRY,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_NOTIFICATION_ACTION_POLICY,
        provider_effect: None,
        tenant_non_disclosure: true,
        protected_gates: LEGACY_NOTIFICATION_ACTION_PROTECTED_GATES,
        production_promoted: false,
    };

pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE: LegacyNotificationProfileV1 =
    LegacyNotificationProfileV1 {
        operation_id: LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID,
        source_manifest_sha256: LEGACY_UPDATE_NOTIFICATION_PREFERENCES_SOURCE_MANIFEST_SHA256,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_UPDATE_NOTIFICATION_PREFERENCES_IDENTITY,
        pinned_commit: LEGACY_NOTIFICATION_ACTIONS_CAP_COMMIT,
        sources: LEGACY_UPDATE_NOTIFICATION_PREFERENCES_SOURCES,
        authentication: "session",
        policy: LEGACY_NOTIFICATION_ACTION_POLICY,
        content_type: LEGACY_NOTIFICATION_ACTION_CONTENT_TYPE,
        max_body_bytes: LEGACY_NOTIFICATION_ACTION_MAX_BODY_BYTES,
        input: LegacyNotificationInputShapeV1::NotificationBooleanObjectWithOptionalAnonymousViews,
        observed_authorization: LegacyNotificationObservedAuthorizationV1::SessionActorUserRowOnly,
        required_authorization:
            LegacyNotificationRequiredAuthorizationV1::SessionActorScopedPreferences,
        observed_mutation:
            LegacyNotificationObservedMutationV1::ReplaceNotificationsBranchFromAuthenticationSnapshot,
        required_mutation:
            LegacyNotificationRequiredMutationV1::AtomicallyMergeNotificationsBranchPreservingOtherPreferences,
        observed_success: "void",
        observed_failure:
            LegacyNotificationObservedFailureV1::ThrownAuthenticationOrCaughtMutationAndRevalidationError,
        observed_failure_messages: LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OBSERVED_FAILURES,
        required_public_failure_messages: LEGACY_NOTIFICATION_ACTION_REQUIRED_PUBLIC_FAILURES,
        observed_revalidation_path: LEGACY_NOTIFICATION_DASHBOARD_REVALIDATION_PATH,
        observed_retry_defect:
            LegacyNotificationObservedRetryDefectV1::RetryCanOverwriteConcurrentPreferenceBranchesFromStaleSnapshot,
        required_retry: LEGACY_NOTIFICATION_ACTION_REQUIRED_RETRY,
        observed_idempotency: "none",
        idempotency: "required",
        rate_limit_bucket: LEGACY_NOTIFICATION_ACTION_POLICY,
        provider_effect: None,
        tenant_non_disclosure: true,
        protected_gates: LEGACY_NOTIFICATION_ACTION_PROTECTED_GATES,
        production_promoted: false,
    };

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationPreferencesUpdateV1 {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    pause_anon_views: Option<bool>,
}

impl fmt::Debug for LegacyNotificationPreferencesUpdateV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyNotificationPreferencesUpdateV1([redacted])")
    }
}

impl LegacyNotificationPreferencesUpdateV1 {
    #[must_use]
    pub const fn new(
        pause_comments: bool,
        pause_replies: bool,
        pause_views: bool,
        pause_reactions: bool,
        pause_anon_views: Option<bool>,
    ) -> Self {
        Self {
            pause_comments,
            pause_replies,
            pause_views,
            pause_reactions,
            pause_anon_views,
        }
    }

    #[must_use]
    pub const fn pause_comments(self) -> bool {
        self.pause_comments
    }

    #[must_use]
    pub const fn pause_replies(self) -> bool {
        self.pause_replies
    }

    #[must_use]
    pub const fn pause_views(self) -> bool {
        self.pause_views
    }

    #[must_use]
    pub const fn pause_reactions(self) -> bool {
        self.pause_reactions
    }

    /// The source TypeScript property was optional. `None` means the property
    /// must remain absent from the stored notifications object, while the
    /// existing read projection treats that absence as effective `false`.
    #[must_use]
    pub const fn pause_anon_views(self) -> Option<bool> {
        self.pause_anon_views
    }

    #[must_use]
    pub const fn effective_pause_anon_views(self) -> bool {
        match self.pause_anon_views {
            Some(value) => value,
            None => false,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyNotificationInputV1 {
    MarkAsRead {
        legacy_notification_id: Option<String>,
    },
    UpdatePreferences {
        notifications: LegacyNotificationPreferencesUpdateV1,
    },
}

impl LegacyNotificationInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyNotificationActionV1 {
        match self {
            Self::MarkAsRead { .. } => LegacyNotificationActionV1::MarkAsRead,
            Self::UpdatePreferences { .. } => LegacyNotificationActionV1::UpdatePreferences,
        }
    }
}

impl fmt::Debug for LegacyNotificationInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MarkAsRead {
                legacy_notification_id,
            } => formatter
                .debug_struct("MarkAsRead")
                .field("selector_present", &legacy_notification_id.is_some())
                .field("selector", &"<redacted>")
                .finish(),
            Self::UpdatePreferences { .. } => formatter
                .debug_struct("UpdatePreferences")
                .field("notifications", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNotificationRequestV1 {
    pub credential: Option<LegacyNotificationCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyNotificationInputV1,
}

impl fmt::Debug for LegacyNotificationRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyNotificationRequestV1")
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
pub struct LegacyNotificationAuthorityV1 {
    actor_id: UserId,
    active_organization_id: Option<OrganizationId>,
}

impl LegacyNotificationAuthorityV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> Option<OrganizationId> {
        self.active_organization_id
    }
}

impl fmt::Debug for LegacyNotificationAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyNotificationAuthorityV1([redacted])")
    }
}

/// Unforgeable browser material for the notification atomic boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyNotificationBrowserFenceV1 {
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

impl fmt::Debug for LegacyNotificationBrowserFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyNotificationBrowserFenceV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNotificationFenceV1 {
    authority: LegacyNotificationAuthorityV1,
    idempotency_key: IdempotencyKey,
    request_fingerprint: [u8; 32],
}

impl LegacyNotificationFenceV1 {
    #[must_use]
    pub const fn authority(&self) -> &LegacyNotificationAuthorityV1 {
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

impl fmt::Debug for LegacyNotificationFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyNotificationFenceV1")
            .field("authority", &self.authority)
            .field("idempotency_key", &"<redacted>")
            .field("request_fingerprint", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LegacyNotificationCommandV1 {
    MarkAsRead {
        fence: LegacyNotificationFenceV1,
        notification_id: Option<NotificationId>,
    },
    UpdatePreferences {
        fence: LegacyNotificationFenceV1,
        notifications: LegacyNotificationPreferencesUpdateV1,
    },
}

impl LegacyNotificationCommandV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyNotificationActionV1 {
        match self {
            Self::MarkAsRead { .. } => LegacyNotificationActionV1::MarkAsRead,
            Self::UpdatePreferences { .. } => LegacyNotificationActionV1::UpdatePreferences,
        }
    }

    #[must_use]
    pub const fn fence(&self) -> &LegacyNotificationFenceV1 {
        match self {
            Self::MarkAsRead { fence, .. } | Self::UpdatePreferences { fence, .. } => fence,
        }
    }

    #[must_use]
    pub const fn notification_id(&self) -> Option<NotificationId> {
        match self {
            Self::MarkAsRead {
                notification_id, ..
            } => *notification_id,
            Self::UpdatePreferences { .. } => None,
        }
    }

    #[must_use]
    pub const fn notifications(&self) -> Option<LegacyNotificationPreferencesUpdateV1> {
        match self {
            Self::UpdatePreferences { notifications, .. } => Some(*notifications),
            Self::MarkAsRead { .. } => None,
        }
    }
}

impl fmt::Debug for LegacyNotificationCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::MarkAsRead { .. } => "MarkAsRead",
                Self::UpdatePreferences { .. } => "UpdatePreferences",
            })
            .field("fence", self.fence())
            .field("selector_present", &self.notification_id().is_some())
            .field("payload", &"<redacted>")
            .finish()
    }
}

/// Authority context reasserted under the same snapshot as the write.
#[derive(Clone, PartialEq, Eq)]
pub enum LegacyNotificationDiscoveredContextV1 {
    MarkAsRead {
        organization_id: OrganizationId,
        recipient_id: UserId,
        selected_notification_id: Option<NotificationId>,
    },
    UpdatePreferences {
        actor_id: UserId,
    },
}

impl fmt::Debug for LegacyNotificationDiscoveredContextV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self {
                Self::MarkAsRead { .. } => "MarkAsRead",
                Self::UpdatePreferences { .. } => "UpdatePreferences",
            })
            .field("authority", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationMutationResultV1 {
    MarkedRead {
        matched_count: u32,
        read_at: TimestampMillis,
    },
    PreferencesUpdated {
        notifications: LegacyNotificationPreferencesUpdateV1,
    },
}

impl fmt::Debug for LegacyNotificationMutationResultV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MarkedRead { .. } => "MarkedRead([redacted])",
            Self::PreferencesUpdated { .. } => "PreferencesUpdated([redacted])",
        })
    }
}

/// Authority class for an active, undeleted organization edge observed inside
/// the committing transaction. Both variants authorize only the actor's own
/// notification rows; the distinction exists so an adapter cannot substitute
/// a stale or unrelated organization identifier for a verified graph edge.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationOrganizationAuthorityV1 {
    Owner,
    Member,
}

impl fmt::Debug for LegacyNotificationOrganizationAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyNotificationOrganizationAuthorityV1([redacted])")
    }
}

/// Database-derived proof that the current one-use browser grant was consumed
/// and the complete action-specific authority graph was live at commit time.
///
/// `from_verified_mark_rows` may be called only after the same D1 transaction
/// has consumed the exact grant, joined it to the live session and undeleted
/// actor, proved the actor's current active organization is undeleted, and
/// proved an owner/member edge for that organization. `active_recipient_id`
/// is the database predicate used for both selected and bulk notification
/// writes. `from_verified_preferences_row` additionally proves the exact actor
/// preference row exists under that live session. The private shape prevents
/// callers from weakening a mark-read proof into an actor-only proof.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNotificationAuthorityPostconditionV1 {
    action: LegacyNotificationActionV1,
    consumed_mutation_grant_id: SessionMutationGrantId,
    active_session_id: SessionId,
    active_actor_id: UserId,
    active_organization_id: Option<OrganizationId>,
    organization_authority: Option<LegacyNotificationOrganizationAuthorityV1>,
    active_recipient_id: Option<UserId>,
}

impl LegacyNotificationAuthorityPostconditionV1 {
    #[must_use]
    pub const fn from_verified_mark_rows(
        consumed_mutation_grant_id: SessionMutationGrantId,
        active_session_id: SessionId,
        active_actor_id: UserId,
        active_organization_id: OrganizationId,
        organization_authority: LegacyNotificationOrganizationAuthorityV1,
        active_recipient_id: UserId,
    ) -> Self {
        Self {
            action: LegacyNotificationActionV1::MarkAsRead,
            consumed_mutation_grant_id,
            active_session_id,
            active_actor_id,
            active_organization_id: Some(active_organization_id),
            organization_authority: Some(organization_authority),
            active_recipient_id: Some(active_recipient_id),
        }
    }

    #[must_use]
    pub const fn from_verified_preferences_row(
        consumed_mutation_grant_id: SessionMutationGrantId,
        active_session_id: SessionId,
        active_actor_id: UserId,
    ) -> Self {
        Self {
            action: LegacyNotificationActionV1::UpdatePreferences,
            consumed_mutation_grant_id,
            active_session_id,
            active_actor_id,
            active_organization_id: None,
            organization_authority: None,
            active_recipient_id: None,
        }
    }

    fn covers_command(
        &self,
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
        context: &LegacyNotificationDiscoveredContextV1,
    ) -> bool {
        if self.action != command.action()
            || self.consumed_mutation_grant_id != browser_fence.mutation_grant_id()
            || self.active_session_id != browser_fence.session_id()
            || self.active_actor_id != browser_fence.actor_id()
            || self.active_actor_id != command.fence().authority().actor_id()
        {
            return false;
        }

        match (command, context) {
            (
                LegacyNotificationCommandV1::MarkAsRead { .. },
                LegacyNotificationDiscoveredContextV1::MarkAsRead {
                    organization_id,
                    recipient_id,
                    ..
                },
            ) => {
                self.active_organization_id == Some(*organization_id)
                    && self.active_organization_id
                        == command.fence().authority().active_organization_id()
                    && self.organization_authority.is_some()
                    && self.active_recipient_id == Some(*recipient_id)
                    && self.active_recipient_id == Some(self.active_actor_id)
            }
            (
                LegacyNotificationCommandV1::UpdatePreferences { .. },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences { actor_id },
            ) => {
                self.active_organization_id.is_none()
                    && self.organization_authority.is_none()
                    && self.active_recipient_id.is_none()
                    && *actor_id == self.active_actor_id
            }
            _ => false,
        }
    }
}

impl fmt::Debug for LegacyNotificationAuthorityPostconditionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyNotificationAuthorityPostconditionV1")
            .field("action", &self.action)
            .field("database_authority", &"<redacted>")
            .field("browser_proof", &"<consumed-redacted>")
            .finish()
    }
}

/// SHA-256 of the canonical top-level preferences object after removing its
/// `notifications` property. Canonicalization recursively sorts object keys by
/// UTF-8 key bytes, retains array order, emits compact JSON with standard JSON
/// escaping, and permits no non-JSON numeric values. A null preferences value
/// is normalized to an empty object; malformed/non-object values fail closed
/// before construction. Comparing the pre- and post-write digests proves the
/// notification update did not replace any concurrent sibling preference
/// branch without retaining its potentially sensitive contents in the receipt.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationPreservedPreferencesDigestV1([u8; 32]);

impl LegacyNotificationPreservedPreferencesDigestV1 {
    #[must_use]
    pub const fn from_canonical_sha256(value: [u8; 32]) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for LegacyNotificationPreservedPreferencesDigestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyNotificationPreservedPreferencesDigestV1([redacted])")
    }
}

/// Exact database mutation and final-state facts measured under the committing
/// snapshot. The counts and timestamp prevent a port from returning success for
/// a partial mark-read write; the preference digests and exact stored branch
/// prevent replacement of unrelated JSON fields.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationMutationPostconditionV1 {
    MarkedRead {
        matching_before: u32,
        updated_rows: u32,
        matching_at_read_time_after: u32,
        out_of_scope_updated_rows: u32,
        read_at: TimestampMillis,
    },
    PreferencesMerged {
        matching_before: u32,
        updated_rows: u32,
        matching_after: u32,
        other_actor_rows_updated: u32,
        stored_notifications: LegacyNotificationPreferencesUpdateV1,
        preserved_before: LegacyNotificationPreservedPreferencesDigestV1,
        preserved_after: LegacyNotificationPreservedPreferencesDigestV1,
    },
}

impl LegacyNotificationMutationPostconditionV1 {
    fn covers_command(
        self,
        command: &LegacyNotificationCommandV1,
        result: LegacyNotificationMutationResultV1,
        context: &LegacyNotificationDiscoveredContextV1,
    ) -> bool {
        match (command, result, context, self) {
            (
                LegacyNotificationCommandV1::MarkAsRead {
                    notification_id, ..
                },
                LegacyNotificationMutationResultV1::MarkedRead {
                    matched_count,
                    read_at: result_read_at,
                },
                LegacyNotificationDiscoveredContextV1::MarkAsRead {
                    selected_notification_id,
                    ..
                },
                Self::MarkedRead {
                    matching_before,
                    updated_rows,
                    matching_at_read_time_after,
                    out_of_scope_updated_rows,
                    read_at,
                },
            ) => {
                selected_notification_id == notification_id
                    && matching_before == matched_count
                    && updated_rows == matched_count
                    && matching_at_read_time_after == matched_count
                    && out_of_scope_updated_rows == 0
                    && read_at == result_read_at
                    && (notification_id.is_none() || matched_count <= 1)
            }
            (
                LegacyNotificationCommandV1::UpdatePreferences { notifications, .. },
                LegacyNotificationMutationResultV1::PreferencesUpdated {
                    notifications: result_notifications,
                },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences { .. },
                Self::PreferencesMerged {
                    matching_before,
                    updated_rows,
                    matching_after,
                    other_actor_rows_updated,
                    stored_notifications,
                    preserved_before,
                    preserved_after,
                },
            ) => {
                (matching_before, updated_rows, matching_after) == (1, 1, 1)
                    && other_actor_rows_updated == 0
                    && result_notifications == *notifications
                    && stored_notifications == *notifications
                    && preserved_before == preserved_after
            }
            _ => false,
        }
    }
}

impl fmt::Debug for LegacyNotificationMutationPostconditionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MarkedRead { .. } => "MarkedReadPostcondition([redacted])",
            Self::PreferencesMerged { .. } => "PreferencesMergedPostcondition([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNotificationEffectsV1 {
    actor_id: UserId,
    organization_id: Option<OrganizationId>,
    revalidation_path: &'static str,
    invalidates_notification_list: bool,
    invalidates_notification_preferences: bool,
}

impl LegacyNotificationEffectsV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn organization_id(&self) -> Option<OrganizationId> {
        self.organization_id
    }

    #[must_use]
    pub const fn revalidation_path(&self) -> &'static str {
        self.revalidation_path
    }

    #[must_use]
    pub const fn invalidates_notification_list(&self) -> bool {
        self.invalidates_notification_list
    }

    #[must_use]
    pub const fn invalidates_notification_preferences(&self) -> bool {
        self.invalidates_notification_preferences
    }
}

impl fmt::Debug for LegacyNotificationEffectsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyNotificationEffectsV1")
            .field("authority", &"<redacted>")
            .field("revalidation_path", &self.revalidation_path)
            .field(
                "invalidates_notification_list",
                &self.invalidates_notification_list,
            )
            .field(
                "invalidates_notification_preferences",
                &self.invalidates_notification_preferences,
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNotificationMutationReceiptV1 {
    idempotency_key_digest: [u8; 32],
    request_fingerprint: [u8; 32],
    result: LegacyNotificationMutationResultV1,
    context: LegacyNotificationDiscoveredContextV1,
    effects: LegacyNotificationEffectsV1,
    authority_postcondition: LegacyNotificationAuthorityPostconditionV1,
    mutation_postcondition: LegacyNotificationMutationPostconditionV1,
}

impl LegacyNotificationMutationReceiptV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
        result: LegacyNotificationMutationResultV1,
        context: LegacyNotificationDiscoveredContextV1,
        authority_postcondition: LegacyNotificationAuthorityPostconditionV1,
        mutation_postcondition: LegacyNotificationMutationPostconditionV1,
    ) -> Result<Self, LegacyNotificationAtomicErrorV1> {
        let actor_id = command.fence().authority().actor_id();
        let effects = match (command, result, &context) {
            (
                LegacyNotificationCommandV1::MarkAsRead {
                    notification_id, ..
                },
                LegacyNotificationMutationResultV1::MarkedRead { matched_count, .. },
                LegacyNotificationDiscoveredContextV1::MarkAsRead {
                    organization_id,
                    recipient_id,
                    selected_notification_id,
                },
            ) if Some(*organization_id) == command.fence().authority().active_organization_id()
                && *recipient_id == actor_id
                && selected_notification_id == notification_id
                && (notification_id.is_none() || matched_count <= 1) =>
            {
                LegacyNotificationEffectsV1 {
                    actor_id,
                    organization_id: Some(*organization_id),
                    revalidation_path: LEGACY_NOTIFICATION_DASHBOARD_REVALIDATION_PATH,
                    invalidates_notification_list: true,
                    invalidates_notification_preferences: false,
                }
            }
            (
                LegacyNotificationCommandV1::UpdatePreferences { notifications, .. },
                LegacyNotificationMutationResultV1::PreferencesUpdated {
                    notifications: stored,
                },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences {
                    actor_id: discovered_actor,
                },
            ) if *discovered_actor == actor_id && stored == *notifications => {
                LegacyNotificationEffectsV1 {
                    actor_id,
                    organization_id: None,
                    revalidation_path: LEGACY_NOTIFICATION_DASHBOARD_REVALIDATION_PATH,
                    invalidates_notification_list: false,
                    invalidates_notification_preferences: true,
                }
            }
            _ => return Err(LegacyNotificationAtomicErrorV1::Corrupt),
        };
        if !authority_postcondition.covers_command(command, browser_fence, &context)
            || !mutation_postcondition.covers_command(command, result, &context)
        {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok(Self {
            idempotency_key_digest: digest_idempotency_key(command.fence().idempotency_key()),
            request_fingerprint: *command.fence().request_fingerprint(),
            result,
            context,
            effects,
            authority_postcondition,
            mutation_postcondition,
        })
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    #[must_use]
    pub const fn result(&self) -> LegacyNotificationMutationResultV1 {
        self.result
    }

    #[must_use]
    pub const fn context(&self) -> &LegacyNotificationDiscoveredContextV1 {
        &self.context
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyNotificationEffectsV1 {
        &self.effects
    }

    #[must_use]
    pub const fn authority_postcondition(&self) -> &LegacyNotificationAuthorityPostconditionV1 {
        &self.authority_postcondition
    }

    #[must_use]
    pub const fn mutation_postcondition(&self) -> LegacyNotificationMutationPostconditionV1 {
        self.mutation_postcondition
    }

    #[must_use]
    pub fn matches_command(
        &self,
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> bool {
        self.idempotency_key_digest == digest_idempotency_key(command.fence().idempotency_key())
            && self.request_fingerprint == *command.fence().request_fingerprint()
            && self
                .authority_postcondition
                .covers_command(command, browser_fence, &self.context)
            && self
                .mutation_postcondition
                .covers_command(command, self.result, &self.context)
    }
}

impl fmt::Debug for LegacyNotificationMutationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyNotificationMutationReceiptV1([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyNotificationAtomicOutcomeV1 {
    Applied(LegacyNotificationMutationReceiptV1),
    Replay(LegacyNotificationMutationReceiptV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationSuccessV1 {
    MarkedRead,
    PreferencesUpdated,
}

impl LegacyNotificationSuccessV1 {
    /// Both source actions resolve with JavaScript `void`.
    #[must_use]
    pub const fn output(self) -> &'static str {
        "void"
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNotificationExecutionV1 {
    success: LegacyNotificationSuccessV1,
    effects: LegacyNotificationEffectsV1,
    replayed: bool,
}

impl LegacyNotificationExecutionV1 {
    #[must_use]
    pub const fn success(&self) -> LegacyNotificationSuccessV1 {
        self.success
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyNotificationEffectsV1 {
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

impl fmt::Debug for LegacyNotificationExecutionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyNotificationExecutionV1")
            .field("success", &self.success)
            .field("effects", &self.effects)
            .field("replayed", &self.replayed)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyNotificationAtomicErrorV1 {
    #[error("notification authority was denied")]
    AccessDenied,
    #[error("notification authority was denied")]
    CrossTenant,
    #[error("notification authority was denied")]
    StaleAuthority,
    #[error("notification request conflicts with current state")]
    Conflict,
    #[error("notification request is already in flight")]
    InFlight,
    #[error("notification authority is unavailable")]
    Unavailable,
    #[error("notification authority returned invalid state")]
    Corrupt,
}

/// Provider-free atomic boundary for both exact notification actions.
///
/// An implementation must use one D1 transaction/batch and, before commit:
///
/// 1. assert and consume the one-use browser grant for the command actor;
/// 2. reassert the session actor and user are live;
/// 3. for `MarkAsRead`, reassert the trusted active organization and update
///    only rows whose recipient is the actor and whose organization is that
///    active organization. A selected absent, foreign, or cross-tenant row is
///    an indistinguishable successful zero-row mutation, never a disclosure;
/// 4. for `UpdatePreferences`, merge the complete submitted notifications
///    branch into the latest database JSON value while preserving every other
///    top-level preference field. `pauseAnonViews: None` remains absent;
/// 5. use a database-owned timestamp for a new mark-read mutation and retain
///    that timestamp in the receipt so replay does not advance it;
/// 6. bind the actor/tenant-scoped idempotency key to the canonical fingerprint;
/// 7. persist the exact result, required cache invalidation, business audit,
///    consumed proof, and replay journal atomically. The returned authority
///    postcondition must contain the grant identifier returned by consumption,
///    not a request-echoed value. The mutation postcondition must measure the
///    scoped rows before/after, the exact database-owned read timestamp, zero
///    out-of-scope changes, and equal canonical non-notification preference
///    digests where applicable.
///
/// Same-key/same-fingerprint retries return `Replay` without rerunning writes;
/// they still consume the current one-use browser grant and return a receipt
/// whose authority postcondition is bound to that current proof while retaining
/// the original result, database timestamp, mutation postcondition, and cache
/// effects. Same-key/different-fingerprint requests return `Conflict`. This
/// application contract does not promote a production route.
#[async_trait]
pub trait LegacyNotificationAtomicPortV1: Send + Sync {
    async fn execute_atomic(
        &self,
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> Result<LegacyNotificationAtomicOutcomeV1, LegacyNotificationAtomicErrorV1>;
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyNotificationErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid input")]
    Invalid,
    #[error("An idempotency key is required")]
    IdempotencyRequired,
    #[error("Notification request conflicts with current state")]
    Conflict,
    #[error("Notification authority is unavailable")]
    AuthorityUnavailable,
    #[error("Notification action failed")]
    Internal,
}

impl fmt::Debug for LegacyNotificationErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "Unauthorized",
            Self::Invalid => "Invalid",
            Self::IdempotencyRequired => "IdempotencyRequired",
            Self::Conflict => "Conflict",
            Self::AuthorityUnavailable => "AuthorityUnavailable",
            Self::Internal => "Internal",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationAdapterV1 {
    action: LegacyNotificationActionV1,
}

impl LegacyNotificationAdapterV1 {
    #[must_use]
    pub const fn mark_as_read() -> Self {
        Self {
            action: LegacyNotificationActionV1::MarkAsRead,
        }
    }

    #[must_use]
    pub const fn update_preferences() -> Self {
        Self {
            action: LegacyNotificationActionV1::UpdatePreferences,
        }
    }

    #[must_use]
    pub const fn action(self) -> LegacyNotificationActionV1 {
        self.action
    }

    pub fn prepare(
        &self,
        request: &LegacyNotificationRequestV1,
    ) -> Result<LegacyNotificationCommandV1, LegacyNotificationErrorV1> {
        if request.credential != Some(LegacyNotificationCredentialV1::Session) {
            return Err(LegacyNotificationErrorV1::Unauthorized);
        }
        let actor_id = request
            .actor_id
            .ok_or(LegacyNotificationErrorV1::Unauthorized)?;
        if request.input.action() != self.action {
            return Err(LegacyNotificationErrorV1::Invalid);
        }
        let idempotency_key = request
            .idempotency_key
            .as_ref()
            .ok_or(LegacyNotificationErrorV1::IdempotencyRequired)
            .and_then(|value| {
                IdempotencyKey::parse(value.clone()).map_err(|_| LegacyNotificationErrorV1::Invalid)
            })?;

        match &request.input {
            LegacyNotificationInputV1::MarkAsRead {
                legacy_notification_id,
            } => {
                let organization_id = request
                    .active_organization_id
                    .ok_or(LegacyNotificationErrorV1::Unauthorized)?;
                let notification_id = legacy_notification_id
                    .as_deref()
                    .map(map_notification)
                    .transpose()?;
                let request_fingerprint =
                    fingerprint_mark_read(actor_id, organization_id, notification_id);
                Ok(LegacyNotificationCommandV1::MarkAsRead {
                    fence: LegacyNotificationFenceV1 {
                        authority: LegacyNotificationAuthorityV1 {
                            actor_id,
                            active_organization_id: Some(organization_id),
                        },
                        idempotency_key,
                        request_fingerprint,
                    },
                    notification_id,
                })
            }
            LegacyNotificationInputV1::UpdatePreferences { notifications } => {
                let request_fingerprint = fingerprint_preferences(actor_id, *notifications);
                Ok(LegacyNotificationCommandV1::UpdatePreferences {
                    fence: LegacyNotificationFenceV1 {
                        authority: LegacyNotificationAuthorityV1 {
                            actor_id,
                            active_organization_id: None,
                        },
                        idempotency_key,
                        request_fingerprint,
                    },
                    notifications: *notifications,
                })
            }
        }
    }

    pub async fn execute<P>(
        &self,
        port: &P,
        request: &LegacyNotificationRequestV1,
        proof: &ValidatedBrowserMutationProof,
    ) -> Result<LegacyNotificationExecutionV1, LegacyNotificationErrorV1>
    where
        P: LegacyNotificationAtomicPortV1,
    {
        let browser_fence = LegacyNotificationBrowserFenceV1::from_validated_proof(proof);
        self.execute_with_fence(port, request, &browser_fence).await
    }

    async fn execute_with_fence<P>(
        &self,
        port: &P,
        request: &LegacyNotificationRequestV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> Result<LegacyNotificationExecutionV1, LegacyNotificationErrorV1>
    where
        P: LegacyNotificationAtomicPortV1,
    {
        if request.credential != Some(LegacyNotificationCredentialV1::Session)
            || request.actor_id != Some(browser_fence.actor_id())
        {
            return Err(LegacyNotificationErrorV1::Unauthorized);
        }
        let command = self.prepare(request)?;
        let (receipt, replayed) = match port.execute_atomic(&command, browser_fence).await {
            Ok(LegacyNotificationAtomicOutcomeV1::Applied(receipt)) => (receipt, false),
            Ok(LegacyNotificationAtomicOutcomeV1::Replay(receipt)) => (receipt, true),
            Err(error) => return Err(map_atomic_error(error)),
        };
        if !receipt.matches_command(&command, browser_fence) {
            return Err(LegacyNotificationErrorV1::Internal);
        }
        let success = project_success(&command, receipt.result())?;
        Ok(LegacyNotificationExecutionV1 {
            success,
            effects: receipt.effects().clone(),
            replayed,
        })
    }
}

fn map_notification(value: &str) -> Result<NotificationId, LegacyNotificationErrorV1> {
    let mapped = LegacyCapNanoId::parse(value.to_owned())
        .map_err(|_| LegacyNotificationErrorV1::Invalid)?
        .mapped_uuid()
        .to_string();
    NotificationId::parse(&mapped).map_err(|_| LegacyNotificationErrorV1::Invalid)
}

fn fingerprint_mark_read(
    actor_id: UserId,
    organization_id: OrganizationId,
    notification_id: Option<NotificationId>,
) -> [u8; 32] {
    let mut digest = fingerprint_prefix(LegacyNotificationActionV1::MarkAsRead, actor_id);
    digest.update(organization_id.as_uuid().as_bytes());
    match notification_id {
        Some(notification_id) => {
            digest.update([1]);
            digest.update(notification_id.as_uuid().as_bytes());
        }
        None => digest.update([0]),
    }
    digest.finalize().into()
}

fn fingerprint_preferences(
    actor_id: UserId,
    notifications: LegacyNotificationPreferencesUpdateV1,
) -> [u8; 32] {
    let mut digest = fingerprint_prefix(LegacyNotificationActionV1::UpdatePreferences, actor_id);
    digest.update([
        u8::from(notifications.pause_comments()),
        u8::from(notifications.pause_replies()),
        u8::from(notifications.pause_views()),
        u8::from(notifications.pause_reactions()),
    ]);
    match notifications.pause_anon_views() {
        Some(value) => digest.update([1, u8::from(value)]),
        None => digest.update([0, 0]),
    }
    digest.finalize().into()
}

fn fingerprint_prefix(action: LegacyNotificationActionV1, actor_id: UserId) -> Sha256 {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-notification-actions-v1\0");
    digest.update(action.stable_code().as_bytes());
    digest.update([0]);
    digest.update(actor_id.as_uuid().as_bytes());
    digest
}

fn digest_idempotency_key(idempotency_key: &IdempotencyKey) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-notification-idempotency-key-v1\0");
    digest.update(idempotency_key.expose().as_bytes());
    digest.finalize().into()
}

fn project_success(
    command: &LegacyNotificationCommandV1,
    result: LegacyNotificationMutationResultV1,
) -> Result<LegacyNotificationSuccessV1, LegacyNotificationErrorV1> {
    match (command, result) {
        (
            LegacyNotificationCommandV1::MarkAsRead { .. },
            LegacyNotificationMutationResultV1::MarkedRead { .. },
        ) => Ok(LegacyNotificationSuccessV1::MarkedRead),
        (
            LegacyNotificationCommandV1::UpdatePreferences { .. },
            LegacyNotificationMutationResultV1::PreferencesUpdated { .. },
        ) => Ok(LegacyNotificationSuccessV1::PreferencesUpdated),
        _ => Err(LegacyNotificationErrorV1::Internal),
    }
}

fn map_atomic_error(error: LegacyNotificationAtomicErrorV1) -> LegacyNotificationErrorV1 {
    match error {
        LegacyNotificationAtomicErrorV1::AccessDenied
        | LegacyNotificationAtomicErrorV1::CrossTenant
        | LegacyNotificationAtomicErrorV1::StaleAuthority => {
            LegacyNotificationErrorV1::Unauthorized
        }
        LegacyNotificationAtomicErrorV1::Conflict | LegacyNotificationAtomicErrorV1::InFlight => {
            LegacyNotificationErrorV1::Conflict
        }
        LegacyNotificationAtomicErrorV1::Unavailable => {
            LegacyNotificationErrorV1::AuthorityUnavailable
        }
        LegacyNotificationAtomicErrorV1::Corrupt => LegacyNotificationErrorV1::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write as _, sync::Mutex};

    use super::*;

    const ORGANIZATION: &str = "0123456789abcde";
    const OTHER_ORGANIZATION: &str = "0123456789abcdf";
    const ACTOR: &str = "1123456789abcde";
    const OTHER_ACTOR: &str = "1123456789abcdf";
    const NOTIFICATION: &str = "2123456789abcde";
    const OTHER_NOTIFICATION: &str = "2123456789abcdf";

    fn mapped<T, E: fmt::Debug>(value: &str, parse: impl FnOnce(&str) -> Result<T, E>) -> T {
        let mapped = LegacyCapNanoId::parse(value)
            .expect("legacy id")
            .mapped_uuid()
            .to_string();
        parse(&mapped).expect("mapped id")
    }

    fn actor() -> UserId {
        mapped(ACTOR, UserId::parse)
    }

    fn other_actor() -> UserId {
        mapped(OTHER_ACTOR, UserId::parse)
    }

    fn organization() -> OrganizationId {
        mapped(ORGANIZATION, OrganizationId::parse)
    }

    fn other_organization() -> OrganizationId {
        mapped(OTHER_ORGANIZATION, OrganizationId::parse)
    }

    fn notification() -> NotificationId {
        mapped(NOTIFICATION, NotificationId::parse)
    }

    fn preferences(anon: Option<bool>) -> LegacyNotificationPreferencesUpdateV1 {
        LegacyNotificationPreferencesUpdateV1::new(true, false, true, false, anon)
    }

    fn request(input: LegacyNotificationInputV1) -> LegacyNotificationRequestV1 {
        LegacyNotificationRequestV1 {
            credential: Some(LegacyNotificationCredentialV1::Session),
            actor_id: Some(actor()),
            active_organization_id: Some(organization()),
            idempotency_key: Some("notification-action-0001".into()),
            input,
        }
    }

    fn mark_request(selector: Option<&str>) -> LegacyNotificationRequestV1 {
        request(LegacyNotificationInputV1::MarkAsRead {
            legacy_notification_id: selector.map(str::to_owned),
        })
    }

    fn preferences_request(
        value: LegacyNotificationPreferencesUpdateV1,
    ) -> LegacyNotificationRequestV1 {
        request(LegacyNotificationInputV1::UpdatePreferences {
            notifications: value,
        })
    }

    fn source_manifest(sources: &[LegacyNotificationSourcePinV1]) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-cap-notification-source-manifest-v1\0");
        for source in sources {
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

    #[test]
    fn profiles_pin_exact_provider_free_source_closures() {
        for profile in [
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE,
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE,
        ] {
            assert_eq!(
                profile.pinned_commit,
                LEGACY_NOTIFICATION_ACTIONS_CAP_COMMIT
            );
            assert_eq!(profile.kind, "server_action");
            assert_eq!(profile.method, "ACTION");
            assert_eq!(profile.authentication, "session");
            assert_eq!(profile.policy, "collaboration_notifications.v1");
            assert_eq!(profile.content_type, "application/json");
            assert_eq!(profile.max_body_bytes, 256 * 1024);
            assert_eq!(profile.observed_success, "void");
            assert_eq!(profile.observed_idempotency, "none");
            assert_eq!(profile.idempotency, "required");
            assert_eq!(profile.provider_effect, None);
            assert!(profile.tenant_non_disclosure);
            assert_eq!(profile.protected_gates, ["released_legacy_client_e2e"]);
            assert!(!profile.production_promoted);
            assert_eq!(
                source_manifest(profile.sources),
                profile.source_manifest_sha256
            );
            assert!(profile.sources.iter().all(|source| {
                source.sha256.len() == 64
                    && source
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            }));
            assert_eq!(
                profile.required_retry.atomicity,
                LegacyNotificationRequiredAtomicityV1::BrowserProofAuthorityMutationAuditInvalidationAndJournalInOneTransaction
            );
        }
        assert_eq!(LEGACY_MARK_NOTIFICATIONS_READ_PROFILE.sources.len(), 14);
        assert_eq!(
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE.sources[0].sha256,
            "d25181538c6463e95f787902ed52dbb1eec758b14fcdbaa00a77e2408d35bd49"
        );
        assert_eq!(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE.sources.len(),
            12
        );
        assert_eq!(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE.sources[0].sha256,
            "c66025cde3f0b179440a60a4570368e335d474a4a323e83c2043111a3baf5ee8"
        );
    }

    #[test]
    fn profiles_make_the_observed_and_required_authority_split_explicit() {
        assert_eq!(
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE.observed_authorization,
            LegacyNotificationObservedAuthorizationV1::SessionActorThenGlobalNotificationIdOrRecipientOnly
        );
        assert_eq!(
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE.required_authorization,
            LegacyNotificationRequiredAuthorizationV1::SessionActorActiveTenantRecipientScopedNotification
        );
        assert_eq!(
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE.observed_mutation,
            LegacyNotificationObservedMutationV1::OverwriteReadTimeByGlobalIdOrRecipientAcrossOrganizations
        );
        assert_eq!(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE.required_mutation,
            LegacyNotificationRequiredMutationV1::AtomicallyMergeNotificationsBranchPreservingOtherPreferences
        );
        assert_eq!(
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE.observed_revalidation_path,
            "/dashboard"
        );
        assert_eq!(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE.observed_revalidation_path,
            "/dashboard"
        );
        assert!(
            LEGACY_MARK_NOTIFICATIONS_READ_PROFILE
                .observed_failure_messages
                .contains(&"unprojected session/user lookup error")
        );
        assert!(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_PROFILE
                .observed_failure_messages
                .contains(&"unprojected session/user lookup error")
        );
    }

    #[test]
    fn session_actor_and_idempotency_are_required_before_mapping_input() {
        for mutate in [
            |candidate: &mut LegacyNotificationRequestV1| candidate.credential = None,
            |candidate: &mut LegacyNotificationRequestV1| {
                candidate.credential = Some(LegacyNotificationCredentialV1::ApiKey);
            },
            |candidate: &mut LegacyNotificationRequestV1| candidate.actor_id = None,
        ] {
            let mut candidate = mark_request(Some("not-a-cap-id"));
            mutate(&mut candidate);
            assert_eq!(
                LegacyNotificationAdapterV1::mark_as_read().prepare(&candidate),
                Err(LegacyNotificationErrorV1::Unauthorized)
            );
        }

        let mut candidate = mark_request(Some(NOTIFICATION));
        candidate.idempotency_key = None;
        assert_eq!(
            LegacyNotificationAdapterV1::mark_as_read().prepare(&candidate),
            Err(LegacyNotificationErrorV1::IdempotencyRequired)
        );
        candidate.idempotency_key = Some("bad key".into());
        assert_eq!(
            LegacyNotificationAdapterV1::mark_as_read().prepare(&candidate),
            Err(LegacyNotificationErrorV1::Invalid)
        );
    }

    #[test]
    fn mark_read_requires_a_trusted_active_tenant_but_preferences_are_actor_global() {
        let mut mark = mark_request(None);
        mark.active_organization_id = None;
        assert_eq!(
            LegacyNotificationAdapterV1::mark_as_read().prepare(&mark),
            Err(LegacyNotificationErrorV1::Unauthorized)
        );

        let mut update = preferences_request(preferences(Some(false)));
        update.active_organization_id = None;
        let command = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&update)
            .expect("actor-global preferences command");
        assert_eq!(command.fence().authority().active_organization_id(), None);
    }

    #[test]
    fn optional_mark_selector_maps_exact_cap_ids_and_none_means_active_tenant_bulk() {
        let selected = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(Some(NOTIFICATION)))
            .expect("selected command");
        assert_eq!(selected.notification_id(), Some(notification()));
        assert_eq!(
            selected.fence().authority().active_organization_id(),
            Some(organization())
        );

        let bulk = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(None))
            .expect("bulk command");
        assert_eq!(bulk.notification_id(), None);

        for invalid in ["", "not-a-cap-id", "0123456789abcdef", "0123456789abcd!"] {
            assert_eq!(
                LegacyNotificationAdapterV1::mark_as_read().prepare(&mark_request(Some(invalid))),
                Err(LegacyNotificationErrorV1::Invalid)
            );
        }
    }

    #[test]
    fn preference_flags_and_optional_property_presence_are_preserved_exactly() {
        let absent = preferences(None);
        assert!(absent.pause_comments());
        assert!(!absent.pause_replies());
        assert!(absent.pause_views());
        assert!(!absent.pause_reactions());
        assert_eq!(absent.pause_anon_views(), None);
        assert!(!absent.effective_pause_anon_views());

        let explicit_false = preferences(Some(false));
        assert_eq!(explicit_false.pause_anon_views(), Some(false));
        assert!(!explicit_false.effective_pause_anon_views());
        assert_ne!(absent, explicit_false);

        let explicit_true = preferences(Some(true));
        assert!(explicit_true.effective_pause_anon_views());
    }

    #[test]
    fn fingerprints_bind_action_actor_tenant_selector_and_every_preference_bit() {
        let selected = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(Some(NOTIFICATION)))
            .expect("selected");
        let bulk = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(None))
            .expect("bulk");
        assert_ne!(
            selected.fence().request_fingerprint(),
            bulk.fence().request_fingerprint()
        );

        let mut other_tenant = mark_request(Some(NOTIFICATION));
        other_tenant.active_organization_id = Some(other_organization());
        let other_tenant = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&other_tenant)
            .expect("other tenant");
        assert_ne!(
            selected.fence().request_fingerprint(),
            other_tenant.fence().request_fingerprint()
        );

        let other_selector = mark_request(Some(OTHER_NOTIFICATION));
        let other_selector = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&other_selector)
            .expect("other selector");
        assert_ne!(
            selected.fence().request_fingerprint(),
            other_selector.fence().request_fingerprint()
        );

        let missing_anon = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&preferences_request(preferences(None)))
            .expect("missing anon");
        let explicit_false = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&preferences_request(preferences(Some(false))))
            .expect("explicit false");
        assert_ne!(
            missing_anon.fence().request_fingerprint(),
            explicit_false.fence().request_fingerprint()
        );

        let mut other = preferences_request(preferences(None));
        other.actor_id = Some(other_actor());
        let other = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&other)
            .expect("other actor");
        assert_ne!(
            missing_anon.fence().request_fingerprint(),
            other.fence().request_fingerprint()
        );

        let mut different_active_tenant = preferences_request(preferences(None));
        different_active_tenant.active_organization_id = Some(other_organization());
        let different_active_tenant = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&different_active_tenant)
            .expect("preferences remain actor global");
        assert_eq!(
            missing_anon.fence().request_fingerprint(),
            different_active_tenant.fence().request_fingerprint()
        );
    }

    #[test]
    fn action_mismatch_is_rejected() {
        assert_eq!(
            LegacyNotificationAdapterV1::update_preferences()
                .prepare(&mark_request(Some(NOTIFICATION))),
            Err(LegacyNotificationErrorV1::Invalid)
        );

        let mut unauthenticated = preferences_request(preferences(None));
        unauthenticated.credential = None;
        unauthenticated.actor_id = None;
        assert_eq!(
            LegacyNotificationAdapterV1::mark_as_read().prepare(&unauthenticated),
            Err(LegacyNotificationErrorV1::Unauthorized)
        );
        assert_eq!(
            LegacyNotificationAdapterV1::mark_as_read()
                .prepare(&preferences_request(preferences(None))),
            Err(LegacyNotificationErrorV1::Invalid)
        );
    }

    #[test]
    fn request_command_context_and_errors_redact_sensitive_values() {
        let request = mark_request(Some(NOTIFICATION));
        let command = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&request)
            .expect("command");
        let context = LegacyNotificationDiscoveredContextV1::MarkAsRead {
            organization_id: organization(),
            recipient_id: actor(),
            selected_notification_id: Some(notification()),
        };
        for rendered in [
            format!("{request:?}"),
            format!("{command:?}"),
            format!("{context:?}"),
        ] {
            for secret in [NOTIFICATION, "notification-action-0001"] {
                assert!(!rendered.contains(secret));
            }
            assert!(!rendered.contains(&actor().to_string()));
            assert!(!rendered.contains(&organization().to_string()));
        }
        assert_eq!(
            format!("{:?}", LegacyNotificationErrorV1::Internal),
            "Internal"
        );
    }

    fn mark_context(
        command: &LegacyNotificationCommandV1,
    ) -> LegacyNotificationDiscoveredContextV1 {
        LegacyNotificationDiscoveredContextV1::MarkAsRead {
            organization_id: organization(),
            recipient_id: actor(),
            selected_notification_id: command.notification_id(),
        }
    }

    fn mark_authority(
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> LegacyNotificationAuthorityPostconditionV1 {
        LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
            browser_fence.mutation_grant_id(),
            browser_fence.session_id(),
            actor(),
            organization(),
            LegacyNotificationOrganizationAuthorityV1::Member,
            actor(),
        )
    }

    fn mark_postcondition(
        matched_count: u32,
        read_at: TimestampMillis,
    ) -> LegacyNotificationMutationPostconditionV1 {
        LegacyNotificationMutationPostconditionV1::MarkedRead {
            matching_before: matched_count,
            updated_rows: matched_count,
            matching_at_read_time_after: matched_count,
            out_of_scope_updated_rows: 0,
            read_at,
        }
    }

    fn preferences_authority(
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> LegacyNotificationAuthorityPostconditionV1 {
        LegacyNotificationAuthorityPostconditionV1::from_verified_preferences_row(
            browser_fence.mutation_grant_id(),
            browser_fence.session_id(),
            actor(),
        )
    }

    fn preferences_postcondition(
        notifications: LegacyNotificationPreferencesUpdateV1,
    ) -> LegacyNotificationMutationPostconditionV1 {
        let preserved =
            LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256([0x5a; 32]);
        LegacyNotificationMutationPostconditionV1::PreferencesMerged {
            matching_before: 1,
            updated_rows: 1,
            matching_after: 1,
            other_actor_rows_updated: 0,
            stored_notifications: notifications,
            preserved_before: preserved,
            preserved_after: preserved,
        }
    }

    fn mark_receipt(
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
        matched_count: u32,
    ) -> LegacyNotificationMutationReceiptV1 {
        let read_at = TimestampMillis::new(1_700_000_000_000).expect("timestamp");
        LegacyNotificationMutationReceiptV1::new(
            command,
            browser_fence,
            LegacyNotificationMutationResultV1::MarkedRead {
                matched_count,
                read_at,
            },
            mark_context(command),
            mark_authority(browser_fence),
            mark_postcondition(matched_count, read_at),
        )
        .expect("mark receipt")
    }

    fn preferences_receipt(
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> LegacyNotificationMutationReceiptV1 {
        let notifications = command.notifications().expect("preferences command");
        LegacyNotificationMutationReceiptV1::new(
            command,
            browser_fence,
            LegacyNotificationMutationResultV1::PreferencesUpdated { notifications },
            LegacyNotificationDiscoveredContextV1::UpdatePreferences { actor_id: actor() },
            preferences_authority(browser_fence),
            preferences_postcondition(notifications),
        )
        .expect("preferences receipt")
    }

    #[test]
    fn mark_receipt_accepts_zero_row_non_disclosure_and_validates_selected_cardinality() {
        let selected = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(Some(NOTIFICATION)))
            .expect("selected");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let zero = mark_receipt(&selected, &browser_fence, 0);
        assert_eq!(
            zero.result(),
            LegacyNotificationMutationResultV1::MarkedRead {
                matched_count: 0,
                read_at: TimestampMillis::new(1_700_000_000_000).expect("timestamp"),
            }
        );
        assert!(zero.effects().invalidates_notification_list());
        assert!(!zero.effects().invalidates_notification_preferences());
        assert_eq!(zero.effects().organization_id(), Some(organization()));
        assert_eq!(zero.effects().revalidation_path(), "/dashboard");

        assert_eq!(
            LegacyNotificationMutationReceiptV1::new(
                &selected,
                &browser_fence,
                LegacyNotificationMutationResultV1::MarkedRead {
                    matched_count: 2,
                    read_at: TimestampMillis::new(1).expect("timestamp"),
                },
                LegacyNotificationDiscoveredContextV1::MarkAsRead {
                    organization_id: organization(),
                    recipient_id: actor(),
                    selected_notification_id: Some(notification()),
                },
                LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                    browser_fence.mutation_grant_id(),
                    browser_fence.session_id(),
                    actor(),
                    organization(),
                    LegacyNotificationOrganizationAuthorityV1::Member,
                    actor(),
                ),
                LegacyNotificationMutationPostconditionV1::MarkedRead {
                    matching_before: 2,
                    updated_rows: 2,
                    matching_at_read_time_after: 2,
                    out_of_scope_updated_rows: 0,
                    read_at: TimestampMillis::new(1).expect("timestamp"),
                },
            ),
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn bulk_mark_receipt_allows_many_rows_but_never_a_different_authority() {
        let bulk = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(None))
            .expect("bulk");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        assert_eq!(
            mark_receipt(&bulk, &browser_fence, 4_294).result(),
            LegacyNotificationMutationResultV1::MarkedRead {
                matched_count: 4_294,
                read_at: TimestampMillis::new(1_700_000_000_000).expect("timestamp"),
            }
        );
        for context in [
            LegacyNotificationDiscoveredContextV1::MarkAsRead {
                organization_id: other_organization(),
                recipient_id: actor(),
                selected_notification_id: None,
            },
            LegacyNotificationDiscoveredContextV1::MarkAsRead {
                organization_id: organization(),
                recipient_id: other_actor(),
                selected_notification_id: None,
            },
        ] {
            assert_eq!(
                LegacyNotificationMutationReceiptV1::new(
                    &bulk,
                    &browser_fence,
                    LegacyNotificationMutationResultV1::MarkedRead {
                        matched_count: 1,
                        read_at: TimestampMillis::new(1).expect("timestamp"),
                    },
                    context,
                    LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                        browser_fence.mutation_grant_id(),
                        browser_fence.session_id(),
                        actor(),
                        organization(),
                        LegacyNotificationOrganizationAuthorityV1::Member,
                        actor(),
                    ),
                    LegacyNotificationMutationPostconditionV1::MarkedRead {
                        matching_before: 1,
                        updated_rows: 1,
                        matching_at_read_time_after: 1,
                        out_of_scope_updated_rows: 0,
                        read_at: TimestampMillis::new(1).expect("timestamp"),
                    },
                ),
                Err(LegacyNotificationAtomicErrorV1::Corrupt)
            );
        }
    }

    #[test]
    fn preference_receipt_requires_the_exact_stored_branch_and_actor() {
        let command = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&preferences_request(preferences(None)))
            .expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let receipt = preferences_receipt(&command, &browser_fence);
        let preserved =
            LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256([0x5a; 32]);
        assert_eq!(
            receipt.result(),
            LegacyNotificationMutationResultV1::PreferencesUpdated {
                notifications: preferences(None),
            }
        );
        assert_eq!(receipt.effects().organization_id(), None);
        assert!(!receipt.effects().invalidates_notification_list());
        assert!(receipt.effects().invalidates_notification_preferences());

        assert_eq!(
            LegacyNotificationMutationReceiptV1::new(
                &command,
                &browser_fence,
                LegacyNotificationMutationResultV1::PreferencesUpdated {
                    notifications: preferences(Some(false)),
                },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences { actor_id: actor() },
                LegacyNotificationAuthorityPostconditionV1::from_verified_preferences_row(
                    browser_fence.mutation_grant_id(),
                    browser_fence.session_id(),
                    actor(),
                ),
                LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                    matching_before: 1,
                    updated_rows: 1,
                    matching_after: 1,
                    other_actor_rows_updated: 0,
                    stored_notifications: preferences(None),
                    preserved_before: preserved,
                    preserved_after: preserved,
                },
            ),
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        );
        assert_eq!(
            LegacyNotificationMutationReceiptV1::new(
                &command,
                &browser_fence,
                LegacyNotificationMutationResultV1::PreferencesUpdated {
                    notifications: preferences(None),
                },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences {
                    actor_id: other_actor(),
                },
                LegacyNotificationAuthorityPostconditionV1::from_verified_preferences_row(
                    browser_fence.mutation_grant_id(),
                    browser_fence.session_id(),
                    actor(),
                ),
                LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                    matching_before: 1,
                    updated_rows: 1,
                    matching_after: 1,
                    other_actor_rows_updated: 0,
                    stored_notifications: preferences(None),
                    preserved_before: preserved,
                    preserved_after: preserved,
                },
            ),
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        );
    }

    #[test]
    fn mark_receipt_rejects_partial_wrong_time_and_out_of_scope_mutations() {
        let command = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(Some(NOTIFICATION)))
            .expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let read_at = TimestampMillis::new(1_700_000_000_000).expect("timestamp");
        let result = LegacyNotificationMutationResultV1::MarkedRead {
            matched_count: 1,
            read_at,
        };
        for postcondition in [
            LegacyNotificationMutationPostconditionV1::MarkedRead {
                matching_before: 1,
                updated_rows: 0,
                matching_at_read_time_after: 1,
                out_of_scope_updated_rows: 0,
                read_at,
            },
            LegacyNotificationMutationPostconditionV1::MarkedRead {
                matching_before: 1,
                updated_rows: 1,
                matching_at_read_time_after: 0,
                out_of_scope_updated_rows: 0,
                read_at,
            },
            LegacyNotificationMutationPostconditionV1::MarkedRead {
                matching_before: 1,
                updated_rows: 1,
                matching_at_read_time_after: 1,
                out_of_scope_updated_rows: 1,
                read_at,
            },
            LegacyNotificationMutationPostconditionV1::MarkedRead {
                matching_before: 1,
                updated_rows: 1,
                matching_at_read_time_after: 1,
                out_of_scope_updated_rows: 0,
                read_at: TimestampMillis::new(1).expect("timestamp"),
            },
        ] {
            assert_eq!(
                LegacyNotificationMutationReceiptV1::new(
                    &command,
                    &browser_fence,
                    result,
                    mark_context(&command),
                    mark_authority(&browser_fence),
                    postcondition,
                ),
                Err(LegacyNotificationAtomicErrorV1::Corrupt)
            );
        }
    }

    #[test]
    fn preference_receipt_proves_exact_branch_and_preserved_sibling_json() {
        let command = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&preferences_request(preferences(None)))
            .expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let result = LegacyNotificationMutationResultV1::PreferencesUpdated {
            notifications: preferences(None),
        };
        let preserved =
            LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256([0x5a; 32]);
        let changed =
            LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256([0xa5; 32]);
        for postcondition in [
            LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                matching_before: 1,
                updated_rows: 0,
                matching_after: 1,
                other_actor_rows_updated: 0,
                stored_notifications: preferences(None),
                preserved_before: preserved,
                preserved_after: preserved,
            },
            LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                matching_before: 1,
                updated_rows: 1,
                matching_after: 0,
                other_actor_rows_updated: 0,
                stored_notifications: preferences(None),
                preserved_before: preserved,
                preserved_after: preserved,
            },
            LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                matching_before: 1,
                updated_rows: 1,
                matching_after: 1,
                other_actor_rows_updated: 1,
                stored_notifications: preferences(None),
                preserved_before: preserved,
                preserved_after: preserved,
            },
            LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                matching_before: 1,
                updated_rows: 1,
                matching_after: 1,
                other_actor_rows_updated: 0,
                stored_notifications: preferences(Some(false)),
                preserved_before: preserved,
                preserved_after: preserved,
            },
            LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                matching_before: 1,
                updated_rows: 1,
                matching_after: 1,
                other_actor_rows_updated: 0,
                stored_notifications: preferences(None),
                preserved_before: preserved,
                preserved_after: changed,
            },
        ] {
            assert_eq!(
                LegacyNotificationMutationReceiptV1::new(
                    &command,
                    &browser_fence,
                    result,
                    LegacyNotificationDiscoveredContextV1::UpdatePreferences { actor_id: actor() },
                    preferences_authority(&browser_fence),
                    postcondition,
                ),
                Err(LegacyNotificationAtomicErrorV1::Corrupt)
            );
        }
    }

    #[test]
    fn authority_postcondition_binds_consumed_proof_and_active_graph() {
        let command = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(Some(NOTIFICATION)))
            .expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let read_at = TimestampMillis::new(1_700_000_000_000).expect("timestamp");
        let result = LegacyNotificationMutationResultV1::MarkedRead {
            matched_count: 1,
            read_at,
        };
        for authority in [
            LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                SessionMutationGrantId::new(),
                browser_fence.session_id(),
                actor(),
                organization(),
                LegacyNotificationOrganizationAuthorityV1::Member,
                actor(),
            ),
            LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                browser_fence.mutation_grant_id(),
                SessionId::new(),
                actor(),
                organization(),
                LegacyNotificationOrganizationAuthorityV1::Member,
                actor(),
            ),
            LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                browser_fence.mutation_grant_id(),
                browser_fence.session_id(),
                other_actor(),
                organization(),
                LegacyNotificationOrganizationAuthorityV1::Member,
                actor(),
            ),
            LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                browser_fence.mutation_grant_id(),
                browser_fence.session_id(),
                actor(),
                other_organization(),
                LegacyNotificationOrganizationAuthorityV1::Member,
                actor(),
            ),
            LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                browser_fence.mutation_grant_id(),
                browser_fence.session_id(),
                actor(),
                organization(),
                LegacyNotificationOrganizationAuthorityV1::Member,
                other_actor(),
            ),
            LegacyNotificationAuthorityPostconditionV1::from_verified_preferences_row(
                browser_fence.mutation_grant_id(),
                browser_fence.session_id(),
                actor(),
            ),
        ] {
            assert_eq!(
                LegacyNotificationMutationReceiptV1::new(
                    &command,
                    &browser_fence,
                    result,
                    mark_context(&command),
                    authority,
                    mark_postcondition(1, read_at),
                ),
                Err(LegacyNotificationAtomicErrorV1::Corrupt)
            );
        }
    }

    #[test]
    fn preference_results_postconditions_and_receipts_redact_every_value() {
        let command = LegacyNotificationAdapterV1::update_preferences()
            .prepare(&preferences_request(preferences(Some(true))))
            .expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let notifications = preferences(Some(true));
        let result = LegacyNotificationMutationResultV1::PreferencesUpdated { notifications };
        let postcondition = preferences_postcondition(notifications);
        let authority = preferences_authority(&browser_fence);
        let receipt = preferences_receipt(&command, &browser_fence);
        let marked = LegacyNotificationMutationResultV1::MarkedRead {
            matched_count: 17,
            read_at: TimestampMillis::new(1_700_000_000_000).expect("timestamp"),
        };

        for rendered in [
            format!("{notifications:?}"),
            format!("{result:?}"),
            format!("{marked:?}"),
            format!("{postcondition:?}"),
            format!("{authority:?}"),
            format!("{:?}", LegacyNotificationOrganizationAuthorityV1::Member),
            format!("{receipt:?}"),
        ] {
            for secret in [
                "true",
                "false",
                "Some",
                "Member",
                "Owner",
                "17",
                "1700000000000",
                "notification-action-0001",
                &actor().to_string(),
                &organization().to_string(),
            ] {
                assert!(!rendered.contains(secret), "leaked {secret} in {rendered}");
            }
        }
    }

    #[test]
    fn wrong_result_or_context_variant_is_corrupt() {
        let mark = LegacyNotificationAdapterV1::mark_as_read()
            .prepare(&mark_request(None))
            .expect("mark");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let preserved =
            LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256([0x5a; 32]);
        assert_eq!(
            LegacyNotificationMutationReceiptV1::new(
                &mark,
                &browser_fence,
                LegacyNotificationMutationResultV1::PreferencesUpdated {
                    notifications: preferences(None),
                },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences { actor_id: actor() },
                LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                    browser_fence.mutation_grant_id(),
                    browser_fence.session_id(),
                    actor(),
                    organization(),
                    LegacyNotificationOrganizationAuthorityV1::Member,
                    actor(),
                ),
                LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                    matching_before: 1,
                    updated_rows: 1,
                    matching_after: 1,
                    other_actor_rows_updated: 0,
                    stored_notifications: preferences(None),
                    preserved_before: preserved,
                    preserved_after: preserved,
                },
            ),
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        );
    }

    struct RecordingPort {
        calls: Mutex<
            Vec<(
                LegacyNotificationCommandV1,
                LegacyNotificationBrowserFenceV1,
            )>,
        >,
        result: Mutex<
            Option<Result<LegacyNotificationAtomicOutcomeV1, LegacyNotificationAtomicErrorV1>>,
        >,
    }

    impl RecordingPort {
        fn returning(
            result: Result<LegacyNotificationAtomicOutcomeV1, LegacyNotificationAtomicErrorV1>,
        ) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Mutex::new(Some(result)),
            }
        }
    }

    #[async_trait]
    impl LegacyNotificationAtomicPortV1 for RecordingPort {
        async fn execute_atomic(
            &self,
            command: &LegacyNotificationCommandV1,
            browser_fence: &LegacyNotificationBrowserFenceV1,
        ) -> Result<LegacyNotificationAtomicOutcomeV1, LegacyNotificationAtomicErrorV1> {
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
    async fn adapter_calls_one_atomic_boundary_and_projects_exact_void_success() {
        let adapter = LegacyNotificationAdapterV1::mark_as_read();
        let request = mark_request(Some(NOTIFICATION));
        let command = adapter.prepare(&request).expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let port = RecordingPort::returning(Ok(LegacyNotificationAtomicOutcomeV1::Applied(
            mark_receipt(&command, &browser_fence, 1),
        )));
        let execution = adapter
            .execute_with_fence(&port, &request, &browser_fence)
            .await
            .expect("execution");
        assert_eq!(execution.success(), LegacyNotificationSuccessV1::MarkedRead);
        assert_eq!(execution.success().output(), "void");
        assert!(execution.mutation_was_applied());
        assert!(!execution.replayed());
        assert_eq!(port.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn replay_returns_original_preferences_success_without_reapplying() {
        let adapter = LegacyNotificationAdapterV1::update_preferences();
        let request = preferences_request(preferences(Some(true)));
        let command = adapter.prepare(&request).expect("command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let port = RecordingPort::returning(Ok(LegacyNotificationAtomicOutcomeV1::Replay(
            preferences_receipt(&command, &browser_fence),
        )));
        let execution = adapter
            .execute_with_fence(&port, &request, &browser_fence)
            .await
            .expect("execution");
        assert_eq!(
            execution.success(),
            LegacyNotificationSuccessV1::PreferencesUpdated
        );
        assert_eq!(execution.success().output(), "void");
        assert!(execution.replayed());
        assert!(!execution.mutation_was_applied());
    }

    #[tokio::test]
    async fn receipt_for_a_different_fingerprint_is_rejected_as_corrupt() {
        let adapter = LegacyNotificationAdapterV1::update_preferences();
        let request = preferences_request(preferences(None));
        let different = adapter
            .prepare(&preferences_request(preferences(Some(false))))
            .expect("different command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let port = RecordingPort::returning(Ok(LegacyNotificationAtomicOutcomeV1::Replay(
            preferences_receipt(&different, &browser_fence),
        )));
        assert_eq!(
            adapter
                .execute_with_fence(&port, &request, &browser_fence,)
                .await,
            Err(LegacyNotificationErrorV1::Internal)
        );
    }

    #[tokio::test]
    async fn receipt_for_a_different_idempotency_key_is_rejected() {
        let adapter = LegacyNotificationAdapterV1::update_preferences();
        let original_request = preferences_request(preferences(None));
        let original_command = adapter
            .prepare(&original_request)
            .expect("original command");
        let browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let receipt = preferences_receipt(&original_command, &browser_fence);

        let mut retried_request = original_request;
        retried_request.idempotency_key = Some("notification-action-0002".into());
        let retried_command = adapter.prepare(&retried_request).expect("retried command");
        assert_eq!(
            original_command.fence().request_fingerprint(),
            retried_command.fence().request_fingerprint()
        );
        assert_ne!(
            original_command.fence().idempotency_key(),
            retried_command.fence().idempotency_key()
        );

        let port = RecordingPort::returning(Ok(LegacyNotificationAtomicOutcomeV1::Replay(receipt)));
        assert_eq!(
            adapter
                .execute_with_fence(&port, &retried_request, &browser_fence)
                .await,
            Err(LegacyNotificationErrorV1::Internal)
        );
    }

    #[tokio::test]
    async fn replay_receipt_must_prove_consumption_of_the_current_browser_grant() {
        let adapter = LegacyNotificationAdapterV1::mark_as_read();
        let request = mark_request(None);
        let command = adapter.prepare(&request).expect("command");
        let stale_browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let current_browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        assert_ne!(
            stale_browser_fence.mutation_grant_id(),
            current_browser_fence.mutation_grant_id()
        );
        let port = RecordingPort::returning(Ok(LegacyNotificationAtomicOutcomeV1::Replay(
            mark_receipt(&command, &stale_browser_fence, 3),
        )));
        assert_eq!(
            adapter
                .execute_with_fence(&port, &request, &current_browser_fence)
                .await,
            Err(LegacyNotificationErrorV1::Internal)
        );
    }

    #[tokio::test]
    async fn browser_proof_actor_must_match_before_the_port() {
        let adapter = LegacyNotificationAdapterV1::mark_as_read();
        let request = mark_request(None);
        let command = adapter.prepare(&request).expect("command");
        let receipt_browser_fence = LegacyNotificationBrowserFenceV1::fixture(actor());
        let port = RecordingPort::returning(Ok(LegacyNotificationAtomicOutcomeV1::Applied(
            mark_receipt(&command, &receipt_browser_fence, 2),
        )));
        assert_eq!(
            adapter
                .execute_with_fence(
                    &port,
                    &request,
                    &LegacyNotificationBrowserFenceV1::fixture(other_actor()),
                )
                .await,
            Err(LegacyNotificationErrorV1::Unauthorized)
        );
        assert!(port.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn atomic_failures_map_to_stable_redacted_errors() {
        for (atomic, public) in [
            (
                LegacyNotificationAtomicErrorV1::AccessDenied,
                LegacyNotificationErrorV1::Unauthorized,
            ),
            (
                LegacyNotificationAtomicErrorV1::CrossTenant,
                LegacyNotificationErrorV1::Unauthorized,
            ),
            (
                LegacyNotificationAtomicErrorV1::StaleAuthority,
                LegacyNotificationErrorV1::Unauthorized,
            ),
            (
                LegacyNotificationAtomicErrorV1::Conflict,
                LegacyNotificationErrorV1::Conflict,
            ),
            (
                LegacyNotificationAtomicErrorV1::InFlight,
                LegacyNotificationErrorV1::Conflict,
            ),
            (
                LegacyNotificationAtomicErrorV1::Unavailable,
                LegacyNotificationErrorV1::AuthorityUnavailable,
            ),
            (
                LegacyNotificationAtomicErrorV1::Corrupt,
                LegacyNotificationErrorV1::Internal,
            ),
        ] {
            let adapter = LegacyNotificationAdapterV1::mark_as_read();
            let request = mark_request(None);
            let port = RecordingPort::returning(Err(atomic));
            assert_eq!(
                adapter
                    .execute_with_fence(
                        &port,
                        &request,
                        &LegacyNotificationBrowserFenceV1::fixture(actor()),
                    )
                    .await,
                Err(public)
            );
        }
    }
}
