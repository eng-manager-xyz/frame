//! Exact, source-pinned contracts for Cap's six retained comment/reaction mutations.
//!
//! The six identities intentionally do not collapse into one generic CRUD API.
//! Mobile creation trims content and checks video feedback authority; the web
//! action preserves whitespace and permits orphan parents. The three delete
//! variants remove different row sets, and only the server action deletes
//! notifications, selecting its branch from the caller-supplied `parentId`.
//! Notification creation happens after comment persistence and failures are
//! swallowed by both create implementations. These differences are part of
//! the compatibility contract and are bound into every replay fingerprint.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    IdempotencyKey, LegacyCapNanoId, OrganizationId, OrganizationOperationId, UserId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_COLLABORATION_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_MOBILE_CREATE_COMMENT_OPERATION_ID: &str = "cap-v1-661d23fdcca80bd2";
pub const LEGACY_MOBILE_CREATE_REACTION_OPERATION_ID: &str = "cap-v1-bd59425c2e7074ae";
pub const LEGACY_MOBILE_DELETE_COMMENT_OPERATION_ID: &str = "cap-v1-b6ec2f719de27105";
pub const LEGACY_WEB_DELETE_COMMENT_ROUTE_OPERATION_ID: &str = "cap-v1-f3f5e53c019f944a";
pub const LEGACY_WEB_DELETE_COMMENT_ACTION_OPERATION_ID: &str = "cap-v1-f74174457880eadc";
pub const LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID: &str = "cap-v1-dbe600b35683c827";

pub const LEGACY_MOBILE_CREATE_COMMENT_IDENTITY: &str = "/api/mobile/caps/:id/comments";
pub const LEGACY_MOBILE_CREATE_REACTION_IDENTITY: &str = "/api/mobile/caps/:id/reactions";
pub const LEGACY_MOBILE_DELETE_COMMENT_IDENTITY: &str = "/api/mobile/comments/:id";
pub const LEGACY_WEB_DELETE_COMMENT_ROUTE_IDENTITY: &str = "/api/video/comment/delete";
pub const LEGACY_WEB_DELETE_COMMENT_ACTION_IDENTITY: &str =
    "action://apps/web/actions/videos/delete-comment.ts#deleteComment";
pub const LEGACY_WEB_NEW_COMMENT_ACTION_IDENTITY: &str =
    "action://apps/web/actions/videos/new-comment.ts#newComment";

pub const LEGACY_MOBILE_CREATE_COMMENT_SOURCE_MANIFEST_SHA256: &str =
    "9ff2a74a8cedde6c7164ca890a85733a38623ef5de0dedffae49b7d6947b2b60";
pub const LEGACY_MOBILE_CREATE_REACTION_SOURCE_MANIFEST_SHA256: &str =
    "8cf2f03083338bc9a28948ed7399533d66fcbcced7d02138801337b8710720d8";
pub const LEGACY_MOBILE_DELETE_COMMENT_SOURCE_MANIFEST_SHA256: &str =
    "43c02de6f1b9b5b7b028f878f019ba2d4b593ea2b177949d1dc7824e49e151e7";
pub const LEGACY_WEB_DELETE_COMMENT_ROUTE_SOURCE_MANIFEST_SHA256: &str =
    "b61bb2da1d688d3b291755938dd79def4c67c74443674339f2c7873180999907";
pub const LEGACY_WEB_DELETE_COMMENT_ACTION_SOURCE_MANIFEST_SHA256: &str =
    "8bb9e6475b36c3950f0715d4bf0ab7ed2349dc921680235ddef4e96e19165f91";
pub const LEGACY_WEB_NEW_COMMENT_ACTION_SOURCE_MANIFEST_SHA256: &str =
    "9918435d3881d495be8733f7593f8d4438bf176e9a64cfbdcb4723ed8beed033";

pub const LEGACY_COLLABORATION_POLICY: &str = "collaboration_notifications.v1";
pub const LEGACY_COLLABORATION_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_COLLABORATION_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_COLLABORATION_MAX_DELETE_ROWS: usize = 100_000;
pub const LEGACY_COLLABORATION_NO_PROTECTED_GATES: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationSourceRoleV1 {
    Transport,
    Contract,
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCollaborationSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyCollaborationSourceRoleV1,
}

const MOBILE_TRANSPORT_SOURCE: LegacyCollaborationSourcePinV1 = LegacyCollaborationSourcePinV1 {
    path: "apps/web/app/api/mobile/[...route]/route.ts",
    symbol: "mobile handler",
    sha256: "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
    role: LegacyCollaborationSourceRoleV1::Transport,
};
const MOBILE_CONTRACT_SOURCE: LegacyCollaborationSourcePinV1 = LegacyCollaborationSourcePinV1 {
    path: "packages/web-domain/src/Mobile.ts",
    symbol: "mobile contract",
    sha256: "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
    role: LegacyCollaborationSourceRoleV1::Contract,
};

pub const LEGACY_MOBILE_CREATE_COMMENT_SOURCES: &[LegacyCollaborationSourcePinV1] = &[
    LegacyCollaborationSourcePinV1 {
        symbol: "mobile handler:createComment",
        ..MOBILE_TRANSPORT_SOURCE
    },
    LegacyCollaborationSourcePinV1 {
        symbol: "createComment",
        ..MOBILE_CONTRACT_SOURCE
    },
];
pub const LEGACY_MOBILE_CREATE_REACTION_SOURCES: &[LegacyCollaborationSourcePinV1] = &[
    LegacyCollaborationSourcePinV1 {
        symbol: "mobile handler:createReaction",
        ..MOBILE_TRANSPORT_SOURCE
    },
    LegacyCollaborationSourcePinV1 {
        symbol: "createReaction",
        ..MOBILE_CONTRACT_SOURCE
    },
];
pub const LEGACY_MOBILE_DELETE_COMMENT_SOURCES: &[LegacyCollaborationSourcePinV1] = &[
    LegacyCollaborationSourcePinV1 {
        symbol: "mobile handler:deleteComment",
        ..MOBILE_TRANSPORT_SOURCE
    },
    LegacyCollaborationSourcePinV1 {
        symbol: "deleteComment",
        ..MOBILE_CONTRACT_SOURCE
    },
];
pub const LEGACY_WEB_DELETE_COMMENT_ROUTE_SOURCES: &[LegacyCollaborationSourcePinV1] =
    &[LegacyCollaborationSourcePinV1 {
        path: "apps/web/app/api/video/comment/delete/route.ts",
        symbol: "DELETE",
        sha256: "14ef1d8346aa29ff90628f0971c78c48d368d966d9e408ea2876ab8aae1df529",
        role: LegacyCollaborationSourceRoleV1::Transport,
    }];
pub const LEGACY_WEB_DELETE_COMMENT_ACTION_SOURCES: &[LegacyCollaborationSourcePinV1] =
    &[LegacyCollaborationSourcePinV1 {
        path: "apps/web/actions/videos/delete-comment.ts",
        symbol: "deleteComment",
        sha256: "7e1cf2a1141e56ec28cb256b35cd47583838fae3745750ea4f72db36fc37ff5e",
        role: LegacyCollaborationSourceRoleV1::Action,
    }];
pub const LEGACY_WEB_NEW_COMMENT_ACTION_SOURCES: &[LegacyCollaborationSourcePinV1] =
    &[LegacyCollaborationSourcePinV1 {
        path: "apps/web/actions/videos/new-comment.ts",
        symbol: "newComment",
        sha256: "66b1386d37d9f0cd04ca37825ecbeef6e57d10a4f9042562bdd655c3badf317e",
        role: LegacyCollaborationSourceRoleV1::Action,
    }];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegacyCollaborationSurfaceV1 {
    MobileCreateComment,
    MobileCreateReaction,
    MobileDeleteComment,
    WebDeleteCommentRoute,
    WebDeleteCommentAction,
    WebNewCommentAction,
}

impl LegacyCollaborationSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::MobileCreateComment => LEGACY_MOBILE_CREATE_COMMENT_OPERATION_ID,
            Self::MobileCreateReaction => LEGACY_MOBILE_CREATE_REACTION_OPERATION_ID,
            Self::MobileDeleteComment => LEGACY_MOBILE_DELETE_COMMENT_OPERATION_ID,
            Self::WebDeleteCommentRoute => LEGACY_WEB_DELETE_COMMENT_ROUTE_OPERATION_ID,
            Self::WebDeleteCommentAction => LEGACY_WEB_DELETE_COMMENT_ACTION_OPERATION_ID,
            Self::WebNewCommentAction => LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::MobileCreateComment => "mobile_create_comment",
            Self::MobileCreateReaction => "mobile_create_reaction",
            Self::MobileDeleteComment => "mobile_delete_comment",
            Self::WebDeleteCommentRoute => "web_delete_comment_route",
            Self::WebDeleteCommentAction => "web_delete_comment_action",
            Self::WebNewCommentAction => "web_new_comment_action",
        }
    }

    #[must_use]
    pub const fn creates_comment(self) -> bool {
        matches!(
            self,
            Self::MobileCreateComment | Self::MobileCreateReaction | Self::WebNewCommentAction
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationObservedValidationV1 {
    MobileTrimThenRejectEmpty,
    MobilePathOnly,
    QueryParameterMustBePresentButMayBeEmpty,
    ActionTruthyCommentAndVideoCallerParentUntrusted,
    ActionTruthyContentAndVideoPreserveWhitespaceAndParent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationObservedAuthorityV1 {
    MobileVideoOwnerOrSharedActiveOrganizationMember,
    AuthoredCommentOnly,
    SessionOnlyNoVideoOrParentAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationObservedMutationV1 {
    InsertTrimmedTextComment,
    InsertTrimmedEmojiReaction,
    DeleteExactAuthoredComment,
    DeleteAuthoredTargetAndAuthoredDirectReplies,
    DeleteExactAuthoredCommentAndCallerSelectedNotificationsTransactionally,
    InsertUntrimmedCommentAllowEmptyRootAndOrphanParent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationNotificationTimingV1 {
    AfterInsertBestEffortFailureSwallowed,
    None,
    SameTransactionCallerParentBranchFailureRollsBackDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationSuccessShapeV1 {
    MobileCreatedCommentProjection,
    SuccessObject,
    WebCreatedCommentProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCollaborationProfileV1 {
    pub operation_id: &'static str,
    pub identity: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub source_manifest_sha256: &'static str,
    pub sources: &'static [LegacyCollaborationSourcePinV1],
    pub validation: LegacyCollaborationObservedValidationV1,
    pub authority: LegacyCollaborationObservedAuthorityV1,
    pub mutation: LegacyCollaborationObservedMutationV1,
    pub notification_timing: LegacyCollaborationNotificationTimingV1,
    pub success: LegacyCollaborationSuccessShapeV1,
    pub authentication: &'static str,
    pub policy: &'static str,
    pub content_type: &'static str,
    pub max_body_bytes: usize,
    pub idempotency: &'static str,
    pub tenant_non_disclosure: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

macro_rules! profile {
    (
        $name:ident, $operation:expr, $identity:expr, $kind:expr, $method:expr,
        $manifest:expr, $sources:expr, $validation:expr, $authority:expr,
        $mutation:expr, $notification:expr, $success:expr, $authentication:expr
    ) => {
        pub const $name: LegacyCollaborationProfileV1 = LegacyCollaborationProfileV1 {
            operation_id: $operation,
            identity: $identity,
            kind: $kind,
            method: $method,
            source_manifest_sha256: $manifest,
            sources: $sources,
            validation: $validation,
            authority: $authority,
            mutation: $mutation,
            notification_timing: $notification,
            success: $success,
            authentication: $authentication,
            policy: LEGACY_COLLABORATION_POLICY,
            content_type: LEGACY_COLLABORATION_CONTENT_TYPE,
            max_body_bytes: LEGACY_COLLABORATION_MAX_BODY_BYTES,
            idempotency: "required",
            tenant_non_disclosure: true,
            protected_gates: LEGACY_COLLABORATION_NO_PROTECTED_GATES,
            production_promoted: true,
        };
    };
}

profile!(
    LEGACY_MOBILE_CREATE_COMMENT_PROFILE,
    LEGACY_MOBILE_CREATE_COMMENT_OPERATION_ID,
    LEGACY_MOBILE_CREATE_COMMENT_IDENTITY,
    "route",
    "POST",
    LEGACY_MOBILE_CREATE_COMMENT_SOURCE_MANIFEST_SHA256,
    LEGACY_MOBILE_CREATE_COMMENT_SOURCES,
    LegacyCollaborationObservedValidationV1::MobileTrimThenRejectEmpty,
    LegacyCollaborationObservedAuthorityV1::MobileVideoOwnerOrSharedActiveOrganizationMember,
    LegacyCollaborationObservedMutationV1::InsertTrimmedTextComment,
    LegacyCollaborationNotificationTimingV1::AfterInsertBestEffortFailureSwallowed,
    LegacyCollaborationSuccessShapeV1::MobileCreatedCommentProjection,
    "session_or_api_key"
);
profile!(
    LEGACY_MOBILE_CREATE_REACTION_PROFILE,
    LEGACY_MOBILE_CREATE_REACTION_OPERATION_ID,
    LEGACY_MOBILE_CREATE_REACTION_IDENTITY,
    "route",
    "POST",
    LEGACY_MOBILE_CREATE_REACTION_SOURCE_MANIFEST_SHA256,
    LEGACY_MOBILE_CREATE_REACTION_SOURCES,
    LegacyCollaborationObservedValidationV1::MobileTrimThenRejectEmpty,
    LegacyCollaborationObservedAuthorityV1::MobileVideoOwnerOrSharedActiveOrganizationMember,
    LegacyCollaborationObservedMutationV1::InsertTrimmedEmojiReaction,
    LegacyCollaborationNotificationTimingV1::AfterInsertBestEffortFailureSwallowed,
    LegacyCollaborationSuccessShapeV1::MobileCreatedCommentProjection,
    "session_or_api_key"
);
profile!(
    LEGACY_MOBILE_DELETE_COMMENT_PROFILE,
    LEGACY_MOBILE_DELETE_COMMENT_OPERATION_ID,
    LEGACY_MOBILE_DELETE_COMMENT_IDENTITY,
    "route",
    "DELETE",
    LEGACY_MOBILE_DELETE_COMMENT_SOURCE_MANIFEST_SHA256,
    LEGACY_MOBILE_DELETE_COMMENT_SOURCES,
    LegacyCollaborationObservedValidationV1::MobilePathOnly,
    LegacyCollaborationObservedAuthorityV1::AuthoredCommentOnly,
    LegacyCollaborationObservedMutationV1::DeleteExactAuthoredComment,
    LegacyCollaborationNotificationTimingV1::None,
    LegacyCollaborationSuccessShapeV1::SuccessObject,
    "session_or_api_key"
);
profile!(
    LEGACY_WEB_DELETE_COMMENT_ROUTE_PROFILE,
    LEGACY_WEB_DELETE_COMMENT_ROUTE_OPERATION_ID,
    LEGACY_WEB_DELETE_COMMENT_ROUTE_IDENTITY,
    "route",
    "DELETE",
    LEGACY_WEB_DELETE_COMMENT_ROUTE_SOURCE_MANIFEST_SHA256,
    LEGACY_WEB_DELETE_COMMENT_ROUTE_SOURCES,
    LegacyCollaborationObservedValidationV1::QueryParameterMustBePresentButMayBeEmpty,
    LegacyCollaborationObservedAuthorityV1::AuthoredCommentOnly,
    LegacyCollaborationObservedMutationV1::DeleteAuthoredTargetAndAuthoredDirectReplies,
    LegacyCollaborationNotificationTimingV1::None,
    LegacyCollaborationSuccessShapeV1::SuccessObject,
    "session"
);
profile!(
    LEGACY_WEB_DELETE_COMMENT_ACTION_PROFILE,
    LEGACY_WEB_DELETE_COMMENT_ACTION_OPERATION_ID,
    LEGACY_WEB_DELETE_COMMENT_ACTION_IDENTITY,
    "server_action",
    "ACTION",
    LEGACY_WEB_DELETE_COMMENT_ACTION_SOURCE_MANIFEST_SHA256,
    LEGACY_WEB_DELETE_COMMENT_ACTION_SOURCES,
    LegacyCollaborationObservedValidationV1::ActionTruthyCommentAndVideoCallerParentUntrusted,
    LegacyCollaborationObservedAuthorityV1::AuthoredCommentOnly,
    LegacyCollaborationObservedMutationV1::DeleteExactAuthoredCommentAndCallerSelectedNotificationsTransactionally,
    LegacyCollaborationNotificationTimingV1::SameTransactionCallerParentBranchFailureRollsBackDelete,
    LegacyCollaborationSuccessShapeV1::SuccessObject,
    "session"
);
profile!(
    LEGACY_WEB_NEW_COMMENT_ACTION_PROFILE,
    LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID,
    LEGACY_WEB_NEW_COMMENT_ACTION_IDENTITY,
    "server_action",
    "ACTION",
    LEGACY_WEB_NEW_COMMENT_ACTION_SOURCE_MANIFEST_SHA256,
    LEGACY_WEB_NEW_COMMENT_ACTION_SOURCES,
    LegacyCollaborationObservedValidationV1::ActionTruthyContentAndVideoPreserveWhitespaceAndParent,
    LegacyCollaborationObservedAuthorityV1::SessionOnlyNoVideoOrParentAuthority,
    LegacyCollaborationObservedMutationV1::InsertUntrimmedCommentAllowEmptyRootAndOrphanParent,
    LegacyCollaborationNotificationTimingV1::AfterInsertBestEffortFailureSwallowed,
    LegacyCollaborationSuccessShapeV1::WebCreatedCommentProjection,
    "session"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCommentKindV1 {
    Text,
    Emoji,
}

impl LegacyCommentKindV1 {
    pub fn parse(value: &str) -> Result<Self, LegacyCollaborationErrorV1> {
        match value {
            "text" => Ok(Self::Text),
            "emoji" => Ok(Self::Emoji),
            _ => Err(LegacyCollaborationErrorV1::InvalidInput),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Emoji => "emoji",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCollaborationNotificationKindV1 {
    Comment,
    Reply,
    Reaction,
}

impl LegacyCollaborationNotificationKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::Reply => "reply",
            Self::Reaction => "reaction",
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum LegacyCollaborationInputV1 {
    MobileCreateComment {
        legacy_video_id: String,
        content: String,
        timestamp: Option<f64>,
        legacy_parent_comment_id: Option<String>,
    },
    MobileCreateReaction {
        legacy_video_id: String,
        content: String,
        timestamp: Option<f64>,
    },
    MobileDeleteComment {
        legacy_comment_id: String,
    },
    WebDeleteCommentRoute {
        legacy_comment_id: Option<String>,
    },
    WebDeleteCommentAction {
        legacy_comment_id: String,
        caller_parent_id: Option<String>,
        legacy_video_id: String,
    },
    WebNewCommentAction {
        content: String,
        legacy_video_id: String,
        kind: String,
        author_image: Option<String>,
        legacy_parent_comment_id: String,
        timestamp: Option<f64>,
    },
}

impl LegacyCollaborationInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyCollaborationSurfaceV1 {
        match self {
            Self::MobileCreateComment { .. } => LegacyCollaborationSurfaceV1::MobileCreateComment,
            Self::MobileCreateReaction { .. } => LegacyCollaborationSurfaceV1::MobileCreateReaction,
            Self::MobileDeleteComment { .. } => LegacyCollaborationSurfaceV1::MobileDeleteComment,
            Self::WebDeleteCommentRoute { .. } => {
                LegacyCollaborationSurfaceV1::WebDeleteCommentRoute
            }
            Self::WebDeleteCommentAction { .. } => {
                LegacyCollaborationSurfaceV1::WebDeleteCommentAction
            }
            Self::WebNewCommentAction { .. } => LegacyCollaborationSurfaceV1::WebNewCommentAction,
        }
    }
}

impl fmt::Debug for LegacyCollaborationInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MobileCreateComment { .. } => "MobileCreateComment([redacted])",
            Self::MobileCreateReaction { .. } => "MobileCreateReaction([redacted])",
            Self::MobileDeleteComment { .. } => "MobileDeleteComment([redacted])",
            Self::WebDeleteCommentRoute { .. } => "WebDeleteCommentRoute([redacted])",
            Self::WebDeleteCommentAction { .. } => "WebDeleteCommentAction([redacted])",
            Self::WebNewCommentAction { .. } => "WebNewCommentAction([redacted])",
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyCollaborationRequestV1 {
    pub credential: Option<LegacyCollaborationCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyCollaborationInputV1,
}

impl fmt::Debug for LegacyCollaborationRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCollaborationRequestV1")
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

#[derive(Clone, PartialEq)]
pub struct LegacyCreateCommentMutationV1 {
    legacy_comment_id: String,
    mapped_comment_id: String,
    legacy_video_id: String,
    kind: LegacyCommentKindV1,
    content: String,
    timestamp: Option<f64>,
    legacy_parent_comment_id: Option<String>,
    author_image: Option<String>,
    notification_kind: LegacyCollaborationNotificationKindV1,
    requires_video_authority: bool,
}

impl LegacyCreateCommentMutationV1 {
    #[must_use]
    pub fn legacy_comment_id(&self) -> &str {
        &self.legacy_comment_id
    }

    #[must_use]
    pub fn mapped_comment_id(&self) -> &str {
        &self.mapped_comment_id
    }

    #[must_use]
    pub fn legacy_video_id(&self) -> &str {
        &self.legacy_video_id
    }

    #[must_use]
    pub const fn kind(&self) -> LegacyCommentKindV1 {
        self.kind
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    #[must_use]
    pub const fn timestamp(&self) -> Option<f64> {
        self.timestamp
    }

    #[must_use]
    pub fn legacy_parent_comment_id(&self) -> Option<&str> {
        self.legacy_parent_comment_id.as_deref()
    }

    #[must_use]
    pub fn author_image(&self) -> Option<&str> {
        self.author_image.as_deref()
    }

    #[must_use]
    pub const fn notification_kind(&self) -> LegacyCollaborationNotificationKindV1 {
        self.notification_kind
    }

    #[must_use]
    pub const fn requires_video_authority(&self) -> bool {
        self.requires_video_authority
    }
}

impl fmt::Debug for LegacyCreateCommentMutationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCreateCommentMutationV1")
            .field("kind", &self.kind)
            .field("content_bytes", &self.content.len())
            .field("has_timestamp", &self.timestamp.is_some())
            .field("has_parent", &self.legacy_parent_comment_id.is_some())
            .field("has_author_image", &self.author_image.is_some())
            .field("notification_kind", &self.notification_kind)
            .field("requires_video_authority", &self.requires_video_authority)
            .field("scope", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub enum LegacyCollaborationMutationV1 {
    Create(LegacyCreateCommentMutationV1),
    Delete {
        legacy_comment_id: String,
        caller_parent_id: Option<String>,
        caller_video_id: Option<String>,
    },
}

impl fmt::Debug for LegacyCollaborationMutationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create(value) => value.fmt(formatter),
            Self::Delete {
                caller_parent_id,
                caller_video_id,
                ..
            } => formatter
                .debug_struct("LegacyDeleteCommentMutationV1")
                .field("has_caller_parent", &caller_parent_id.is_some())
                .field("has_caller_video", &caller_video_id.is_some())
                .field("target", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyCollaborationCommandV1 {
    operation_id: OrganizationOperationId,
    surface: LegacyCollaborationSurfaceV1,
    actor_id: UserId,
    active_organization_id: OrganizationId,
    idempotency_key: IdempotencyKey,
    idempotency_key_digest: [u8; 32],
    request_digest: [u8; 32],
    mutation: LegacyCollaborationMutationV1,
}

impl LegacyCollaborationCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> OrganizationOperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn surface(&self) -> LegacyCollaborationSurfaceV1 {
        self.surface
    }

    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> OrganizationId {
        self.active_organization_id
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    #[must_use]
    pub const fn idempotency_key_digest(&self) -> &[u8; 32] {
        &self.idempotency_key_digest
    }

    #[must_use]
    pub const fn request_digest(&self) -> &[u8; 32] {
        &self.request_digest
    }

    #[must_use]
    pub const fn mutation(&self) -> &LegacyCollaborationMutationV1 {
        &self.mutation
    }

    #[must_use]
    pub fn request_digest_hex(&self) -> String {
        lower_hex(&self.request_digest)
    }

    #[must_use]
    pub fn idempotency_key_digest_hex(&self) -> String {
        lower_hex(&self.idempotency_key_digest)
    }
}

impl fmt::Debug for LegacyCollaborationCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCollaborationCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.surface)
            .field("authority", &"<redacted>")
            .field("request_digest", &"<redacted>")
            .field("mutation", &self.mutation)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyCreatedCommentV1 {
    pub legacy_comment_id: String,
    pub legacy_video_id: String,
    pub legacy_author_id: String,
    pub author_name: Option<String>,
    pub author_image: Option<String>,
    pub kind: LegacyCommentKindV1,
    pub content: String,
    pub timestamp: Option<f64>,
    pub legacy_parent_comment_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub notification_kind: LegacyCollaborationNotificationKindV1,
}

impl fmt::Debug for LegacyCreatedCommentV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCreatedCommentV1")
            .field("kind", &self.kind)
            .field("content_bytes", &self.content.len())
            .field("has_timestamp", &self.timestamp.is_some())
            .field("has_parent", &self.legacy_parent_comment_id.is_some())
            .field("has_author_name", &self.author_name.is_some())
            .field("has_author_image", &self.author_image.is_some())
            .field("notification_kind", &self.notification_kind)
            .field("identifiers", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyDeleteCommentResultV1 {
    pub deleted_comment_count: u32,
    pub deleted_notification_count: u32,
    pub notification_selector: Option<LegacyDeleteNotificationSelectorV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeleteNotificationSelectorV1 {
    ReplyByCommentId,
    RootCommentAndRepliesByParentId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyCollaborationMutationResultV1 {
    Created(LegacyCreatedCommentV1),
    Deleted(LegacyDeleteCommentResultV1),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyCollaborationAtomicOutcomeV1 {
    pub request_digest: [u8; 32],
    pub result: LegacyCollaborationMutationResultV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyMobileCommentAuthorV1 {
    pub id: String,
    pub name: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyMobileCommentV1 {
    pub id: String,
    pub video_id: String,
    pub kind: LegacyCommentKindV1,
    pub content: String,
    pub timestamp: Option<f64>,
    pub parent_comment_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub author: LegacyMobileCommentAuthorV1,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyWebCommentV1 {
    pub id: String,
    pub author_id: String,
    pub author_name: Option<String>,
    pub author_image: Option<String>,
    pub kind: LegacyCommentKindV1,
    pub content: String,
    pub video_id: String,
    pub timestamp: Option<f64>,
    pub parent_comment_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub sending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyCollaborationSuccessV1 {
    MobileComment(LegacyMobileCommentV1),
    Success { success: bool },
    WebComment(LegacyWebCommentV1),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyCollaborationExecutionV1 {
    pub success: LegacyCollaborationSuccessV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyCollaborationAtomicErrorV1 {
    #[error("collaboration authority is stale")]
    StaleAuthority,
    #[error("collaboration target is not found")]
    TargetMissing,
    #[error("collaboration access is denied")]
    AccessDenied,
    #[error("collaboration request conflicts with current state")]
    Conflict,
    #[error("collaboration request is still in flight")]
    InFlight,
    #[error("collaboration authority is unavailable")]
    Unavailable,
    #[error("collaboration authority returned corrupt state")]
    Corrupt,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyCollaborationErrorV1 {
    #[error("authentication is required")]
    Unauthorized,
    #[error("collaboration input is invalid")]
    InvalidInput,
    #[error("collaboration target was not found")]
    NotFound,
    #[error("collaboration request conflicts with current state")]
    Conflict,
    #[error("collaboration authority is unavailable")]
    Unavailable,
    #[error("collaboration action failed")]
    Internal,
}

#[async_trait]
pub trait LegacyCollaborationAtomicPortV1: Send + Sync {
    async fn execute(
        &self,
        command: &LegacyCollaborationCommandV1,
    ) -> Result<LegacyCollaborationAtomicOutcomeV1, LegacyCollaborationAtomicErrorV1>;
}

pub struct LegacyCollaborationAdapterV1<'port, Port> {
    port: &'port Port,
}

impl<'port, Port> LegacyCollaborationAdapterV1<'port, Port>
where
    Port: LegacyCollaborationAtomicPortV1,
{
    #[must_use]
    pub const fn new(port: &'port Port) -> Self {
        Self { port }
    }

    pub fn prepare(
        request: LegacyCollaborationRequestV1,
    ) -> Result<LegacyCollaborationCommandV1, LegacyCollaborationErrorV1> {
        prepare(request)
    }

    pub async fn execute(
        &self,
        request: LegacyCollaborationRequestV1,
    ) -> Result<LegacyCollaborationExecutionV1, LegacyCollaborationErrorV1> {
        let command = prepare(request)?;
        let expected_surface = command.surface();
        let outcome = self
            .port
            .execute(&command)
            .await
            .map_err(map_atomic_error)?;
        if outcome.request_digest != *command.request_digest() {
            return Err(LegacyCollaborationErrorV1::Internal);
        }
        let success = project_success(expected_surface, outcome.result)?;
        Ok(LegacyCollaborationExecutionV1 {
            success,
            replayed: outcome.replayed,
        })
    }
}

fn prepare(
    request: LegacyCollaborationRequestV1,
) -> Result<LegacyCollaborationCommandV1, LegacyCollaborationErrorV1> {
    let surface = request.input.surface();
    let valid_credential = matches!(
        (surface, request.credential),
        (
            LegacyCollaborationSurfaceV1::MobileCreateComment
                | LegacyCollaborationSurfaceV1::MobileCreateReaction
                | LegacyCollaborationSurfaceV1::MobileDeleteComment,
            Some(
                LegacyCollaborationCredentialV1::Session | LegacyCollaborationCredentialV1::ApiKey,
            ),
        ) | (
            LegacyCollaborationSurfaceV1::WebDeleteCommentRoute
                | LegacyCollaborationSurfaceV1::WebDeleteCommentAction
                | LegacyCollaborationSurfaceV1::WebNewCommentAction,
            Some(LegacyCollaborationCredentialV1::Session),
        )
    );
    if !valid_credential {
        return Err(LegacyCollaborationErrorV1::Unauthorized);
    }
    let actor_id = request
        .actor_id
        .ok_or(LegacyCollaborationErrorV1::Unauthorized)?;
    let active_organization_id = request
        .active_organization_id
        .ok_or(LegacyCollaborationErrorV1::Unauthorized)?;
    let idempotency_key = IdempotencyKey::parse(
        request
            .idempotency_key
            .ok_or(LegacyCollaborationErrorV1::InvalidInput)?,
    )
    .map_err(|_| LegacyCollaborationErrorV1::InvalidInput)?;
    let operation_id = OrganizationOperationId::new();
    let mutation = normalize_input(
        request.input,
        actor_id,
        active_organization_id,
        &idempotency_key,
    )?;
    let request_digest = command_digest(surface, actor_id, active_organization_id, &mutation);
    let mut key_digest = Sha256::new();
    key_digest.update(b"frame-legacy-collaboration-key-v1\0");
    key_digest.update(idempotency_key.expose().as_bytes());
    Ok(LegacyCollaborationCommandV1 {
        operation_id,
        surface,
        actor_id,
        active_organization_id,
        idempotency_key,
        idempotency_key_digest: key_digest.finalize().into(),
        request_digest,
        mutation,
    })
}

fn normalize_input(
    input: LegacyCollaborationInputV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    idempotency_key: &IdempotencyKey,
) -> Result<LegacyCollaborationMutationV1, LegacyCollaborationErrorV1> {
    let surface = input.surface();
    match input {
        LegacyCollaborationInputV1::MobileCreateComment {
            legacy_video_id,
            content,
            timestamp,
            legacy_parent_comment_id,
        } => create_mutation(
            surface,
            actor_id,
            organization_id,
            idempotency_key,
            legacy_video_id,
            trim_ecmascript(&content).to_owned(),
            timestamp,
            legacy_parent_comment_id,
            LegacyCommentKindV1::Text,
            None,
            true,
        ),
        LegacyCollaborationInputV1::MobileCreateReaction {
            legacy_video_id,
            content,
            timestamp,
        } => create_mutation(
            surface,
            actor_id,
            organization_id,
            idempotency_key,
            legacy_video_id,
            trim_ecmascript(&content).to_owned(),
            timestamp,
            None,
            LegacyCommentKindV1::Emoji,
            None,
            true,
        ),
        LegacyCollaborationInputV1::WebNewCommentAction {
            content,
            legacy_video_id,
            kind,
            author_image,
            legacy_parent_comment_id,
            timestamp,
        } => create_mutation(
            surface,
            actor_id,
            organization_id,
            idempotency_key,
            legacy_video_id,
            content,
            timestamp,
            Some(legacy_parent_comment_id),
            LegacyCommentKindV1::parse(&kind)?,
            author_image,
            false,
        ),
        LegacyCollaborationInputV1::MobileDeleteComment { legacy_comment_id } => {
            validate_bounded_id(&legacy_comment_id)?;
            Ok(LegacyCollaborationMutationV1::Delete {
                legacy_comment_id,
                caller_parent_id: None,
                caller_video_id: None,
            })
        }
        LegacyCollaborationInputV1::WebDeleteCommentRoute { legacy_comment_id } => {
            let legacy_comment_id =
                legacy_comment_id.ok_or(LegacyCollaborationErrorV1::InvalidInput)?;
            validate_bounded_id(&legacy_comment_id)?;
            Ok(LegacyCollaborationMutationV1::Delete {
                legacy_comment_id,
                caller_parent_id: None,
                caller_video_id: None,
            })
        }
        LegacyCollaborationInputV1::WebDeleteCommentAction {
            legacy_comment_id,
            caller_parent_id,
            legacy_video_id,
        } => {
            if legacy_comment_id.is_empty() || legacy_video_id.is_empty() {
                return Err(LegacyCollaborationErrorV1::InvalidInput);
            }
            validate_bounded_id(&legacy_comment_id)?;
            validate_bounded_id(&legacy_video_id)?;
            if let Some(parent_id) = &caller_parent_id {
                validate_bounded_id(parent_id)?;
            }
            Ok(LegacyCollaborationMutationV1::Delete {
                legacy_comment_id,
                caller_parent_id,
                caller_video_id: Some(legacy_video_id),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_mutation(
    surface: LegacyCollaborationSurfaceV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    idempotency_key: &IdempotencyKey,
    legacy_video_id: String,
    content: String,
    timestamp: Option<f64>,
    legacy_parent_comment_id: Option<String>,
    kind: LegacyCommentKindV1,
    author_image: Option<String>,
    requires_video_authority: bool,
) -> Result<LegacyCollaborationMutationV1, LegacyCollaborationErrorV1> {
    // Both source functions reject an actually empty string. Mobile has already
    // applied JavaScript trim above; the web action deliberately has not.
    if legacy_video_id.is_empty() || content.is_empty() {
        return Err(LegacyCollaborationErrorV1::InvalidInput);
    }
    validate_bounded_id(&legacy_video_id)?;
    validate_bounded_content(&content)?;
    validate_timestamp(timestamp)?;
    if let Some(parent_id) = &legacy_parent_comment_id {
        validate_bounded_id(parent_id)?;
    }
    if let Some(image) = &author_image {
        validate_bounded_content(image)?;
    }
    let legacy_comment_id = derive_legacy_comment_id(
        surface,
        actor_id,
        organization_id,
        idempotency_key,
        &legacy_video_id,
    );
    let legacy = LegacyCapNanoId::parse(legacy_comment_id.clone())
        .map_err(|_| LegacyCollaborationErrorV1::Internal)?;
    let notification_kind = if legacy_parent_comment_id
        .as_deref()
        .is_some_and(|parent| !parent.is_empty())
    {
        LegacyCollaborationNotificationKindV1::Reply
    } else if kind == LegacyCommentKindV1::Emoji {
        LegacyCollaborationNotificationKindV1::Reaction
    } else {
        LegacyCollaborationNotificationKindV1::Comment
    };
    Ok(LegacyCollaborationMutationV1::Create(
        LegacyCreateCommentMutationV1 {
            legacy_comment_id,
            mapped_comment_id: legacy.mapped_uuid().to_string(),
            legacy_video_id,
            kind,
            content,
            timestamp,
            legacy_parent_comment_id,
            author_image,
            notification_kind,
            requires_video_authority,
        },
    ))
}

fn validate_bounded_id(value: &str) -> Result<(), LegacyCollaborationErrorV1> {
    if value.len() > LEGACY_COLLABORATION_MAX_BODY_BYTES {
        return Err(LegacyCollaborationErrorV1::InvalidInput);
    }
    Ok(())
}

fn validate_bounded_content(value: &str) -> Result<(), LegacyCollaborationErrorV1> {
    if value.len() > LEGACY_COLLABORATION_MAX_BODY_BYTES {
        return Err(LegacyCollaborationErrorV1::InvalidInput);
    }
    Ok(())
}

fn validate_timestamp(value: Option<f64>) -> Result<(), LegacyCollaborationErrorV1> {
    if value.is_some_and(|timestamp| !timestamp.is_finite()) {
        return Err(LegacyCollaborationErrorV1::InvalidInput);
    }
    Ok(())
}

fn derive_legacy_comment_id(
    surface: LegacyCollaborationSurfaceV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    idempotency_key: &IdempotencyKey,
    legacy_video_id: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-collaboration-comment-id-v1\0");
    digest.update(surface.stable_code().as_bytes());
    digest.update(actor_id.as_uuid().as_bytes());
    digest.update(organization_id.as_uuid().as_bytes());
    digest.update(idempotency_key.expose().as_bytes());
    digest.update(legacy_video_id.as_bytes());
    lower_hex(&digest.finalize())[..15].to_owned()
}

fn command_digest(
    surface: LegacyCollaborationSurfaceV1,
    actor_id: UserId,
    organization_id: OrganizationId,
    mutation: &LegacyCollaborationMutationV1,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-collaboration-request-v1\0");
    framed(&mut digest, surface.stable_code().as_bytes());
    framed(&mut digest, actor_id.as_uuid().as_bytes());
    framed(&mut digest, organization_id.as_uuid().as_bytes());
    match mutation {
        LegacyCollaborationMutationV1::Create(create) => {
            framed(&mut digest, b"create");
            framed(&mut digest, create.legacy_comment_id.as_bytes());
            framed(&mut digest, create.legacy_video_id.as_bytes());
            framed(&mut digest, create.kind.as_str().as_bytes());
            framed(&mut digest, create.content.as_bytes());
            optional_f64(&mut digest, create.timestamp);
            optional_string(&mut digest, create.legacy_parent_comment_id.as_deref());
            optional_string(&mut digest, create.author_image.as_deref());
            framed(&mut digest, create.notification_kind.as_str().as_bytes());
            digest.update([u8::from(create.requires_video_authority)]);
        }
        LegacyCollaborationMutationV1::Delete {
            legacy_comment_id,
            caller_parent_id,
            caller_video_id,
        } => {
            framed(&mut digest, b"delete");
            framed(&mut digest, legacy_comment_id.as_bytes());
            optional_string(&mut digest, caller_parent_id.as_deref());
            optional_string(&mut digest, caller_video_id.as_deref());
        }
    }
    digest.finalize().into()
}

fn optional_string(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            framed(digest, value.as_bytes());
        }
        None => digest.update([0]),
    }
}

fn optional_f64(digest: &mut Sha256, value: Option<f64>) {
    match value {
        Some(value) => {
            digest.update([1]);
            digest.update(value.to_bits().to_be_bytes());
        }
        None => digest.update([0]),
    }
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn trim_ecmascript(value: &str) -> &str {
    value.trim_matches(is_ecmascript_whitespace)
}

fn is_ecmascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn project_success(
    surface: LegacyCollaborationSurfaceV1,
    result: LegacyCollaborationMutationResultV1,
) -> Result<LegacyCollaborationSuccessV1, LegacyCollaborationErrorV1> {
    match (surface, result) {
        (
            LegacyCollaborationSurfaceV1::MobileCreateComment
            | LegacyCollaborationSurfaceV1::MobileCreateReaction,
            LegacyCollaborationMutationResultV1::Created(created),
        ) => Ok(LegacyCollaborationSuccessV1::MobileComment(
            LegacyMobileCommentV1 {
                id: created.legacy_comment_id,
                video_id: created.legacy_video_id,
                kind: created.kind,
                content: created.content,
                timestamp: created.timestamp,
                parent_comment_id: created.legacy_parent_comment_id,
                created_at_ms: created.created_at_ms,
                updated_at_ms: created.updated_at_ms,
                author: LegacyMobileCommentAuthorV1 {
                    id: created.legacy_author_id,
                    name: created.author_name,
                    image_url: created.author_image,
                },
            },
        )),
        (
            LegacyCollaborationSurfaceV1::WebNewCommentAction,
            LegacyCollaborationMutationResultV1::Created(created),
        ) => Ok(LegacyCollaborationSuccessV1::WebComment(
            LegacyWebCommentV1 {
                id: created.legacy_comment_id,
                author_id: created.legacy_author_id,
                author_name: created.author_name,
                author_image: created.author_image,
                kind: created.kind,
                content: created.content,
                video_id: created.legacy_video_id,
                timestamp: created.timestamp,
                parent_comment_id: created.legacy_parent_comment_id.unwrap_or_default(),
                created_at_ms: created.created_at_ms,
                updated_at_ms: created.updated_at_ms,
                sending: false,
            },
        )),
        (
            LegacyCollaborationSurfaceV1::MobileDeleteComment
            | LegacyCollaborationSurfaceV1::WebDeleteCommentRoute
            | LegacyCollaborationSurfaceV1::WebDeleteCommentAction,
            LegacyCollaborationMutationResultV1::Deleted(deleted),
        ) if delete_result_matches_surface(surface, &deleted) => {
            Ok(LegacyCollaborationSuccessV1::Success { success: true })
        }
        _ => Err(LegacyCollaborationErrorV1::Internal),
    }
}

fn delete_result_matches_surface(
    surface: LegacyCollaborationSurfaceV1,
    result: &LegacyDeleteCommentResultV1,
) -> bool {
    match surface {
        LegacyCollaborationSurfaceV1::MobileDeleteComment => {
            result.deleted_comment_count == 1
                && result.deleted_notification_count == 0
                && result.notification_selector.is_none()
        }
        LegacyCollaborationSurfaceV1::WebDeleteCommentRoute => {
            (1..=u32::try_from(LEGACY_COLLABORATION_MAX_DELETE_ROWS).unwrap_or(u32::MAX))
                .contains(&result.deleted_comment_count)
                && result.deleted_notification_count == 0
                && result.notification_selector.is_none()
        }
        LegacyCollaborationSurfaceV1::WebDeleteCommentAction => {
            result.deleted_comment_count == 1 && result.notification_selector.is_some()
        }
        LegacyCollaborationSurfaceV1::MobileCreateComment
        | LegacyCollaborationSurfaceV1::MobileCreateReaction
        | LegacyCollaborationSurfaceV1::WebNewCommentAction => false,
    }
}

fn map_atomic_error(error: LegacyCollaborationAtomicErrorV1) -> LegacyCollaborationErrorV1 {
    match error {
        LegacyCollaborationAtomicErrorV1::TargetMissing
        | LegacyCollaborationAtomicErrorV1::AccessDenied
        | LegacyCollaborationAtomicErrorV1::StaleAuthority => LegacyCollaborationErrorV1::NotFound,
        LegacyCollaborationAtomicErrorV1::Conflict | LegacyCollaborationAtomicErrorV1::InFlight => {
            LegacyCollaborationErrorV1::Conflict
        }
        LegacyCollaborationAtomicErrorV1::Unavailable => LegacyCollaborationErrorV1::Unavailable,
        LegacyCollaborationAtomicErrorV1::Corrupt => LegacyCollaborationErrorV1::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const ACTOR: &str = "0123456789abcde";
    const ORGANIZATION: &str = "1123456789abcde";
    const VIDEO: &str = "2123456789abcde";
    const COMMENT: &str = "3123456789abcde";
    const PARENT: &str = "4123456789abcde";

    fn mapped<T, E: fmt::Debug>(value: &str, parse: impl FnOnce(&str) -> Result<T, E>) -> T {
        parse(
            &LegacyCapNanoId::parse(value)
                .expect("legacy id")
                .mapped_uuid()
                .to_string(),
        )
        .expect("mapped")
    }

    fn request(input: LegacyCollaborationInputV1) -> LegacyCollaborationRequestV1 {
        LegacyCollaborationRequestV1 {
            credential: Some(LegacyCollaborationCredentialV1::Session),
            actor_id: Some(mapped(ACTOR, UserId::parse)),
            active_organization_id: Some(mapped(ORGANIZATION, OrganizationId::parse)),
            idempotency_key: Some("collaboration-action-0001".into()),
            input,
        }
    }

    fn mobile_create(content: &str) -> LegacyCollaborationRequestV1 {
        request(LegacyCollaborationInputV1::MobileCreateComment {
            legacy_video_id: VIDEO.into(),
            content: content.into(),
            timestamp: Some(1.25),
            legacy_parent_comment_id: None,
        })
    }

    fn created(command: &LegacyCollaborationCommandV1) -> LegacyCreatedCommentV1 {
        let LegacyCollaborationMutationV1::Create(create) = command.mutation() else {
            panic!("create command")
        };
        LegacyCreatedCommentV1 {
            legacy_comment_id: create.legacy_comment_id().into(),
            legacy_video_id: create.legacy_video_id().into(),
            legacy_author_id: ACTOR.into(),
            author_name: Some("Ada".into()),
            author_image: create.author_image().map(str::to_owned),
            kind: create.kind(),
            content: create.content().into(),
            timestamp: create.timestamp(),
            legacy_parent_comment_id: create.legacy_parent_comment_id().map(str::to_owned),
            created_at_ms: 100,
            updated_at_ms: 100,
            notification_kind: create.notification_kind(),
        }
    }

    #[test]
    fn profiles_pin_all_six_exact_source_manifests_and_asymmetries() {
        let profiles = [
            LEGACY_MOBILE_CREATE_COMMENT_PROFILE,
            LEGACY_MOBILE_CREATE_REACTION_PROFILE,
            LEGACY_MOBILE_DELETE_COMMENT_PROFILE,
            LEGACY_WEB_DELETE_COMMENT_ROUTE_PROFILE,
            LEGACY_WEB_DELETE_COMMENT_ACTION_PROFILE,
            LEGACY_WEB_NEW_COMMENT_ACTION_PROFILE,
        ];
        for (index, profile) in profiles.into_iter().enumerate() {
            assert_eq!(profile.max_body_bytes, 262_144);
            assert_eq!(
                profile.authentication,
                if index < 3 {
                    "session_or_api_key"
                } else {
                    "session"
                }
            );
            assert_eq!(profile.policy, "collaboration_notifications.v1");
            assert_eq!(profile.idempotency, "required");
            assert!(profile.tenant_non_disclosure);
            assert!(profile.protected_gates.is_empty());
            assert!(profile.production_promoted);
            assert_eq!(profile.source_manifest_sha256.len(), 64);
            assert!(!profile.sources.is_empty());
        }
        assert_eq!(
            LEGACY_WEB_DELETE_COMMENT_ACTION_PROFILE.notification_timing,
            LegacyCollaborationNotificationTimingV1::SameTransactionCallerParentBranchFailureRollsBackDelete
        );
        assert_eq!(
            LEGACY_WEB_NEW_COMMENT_ACTION_PROFILE.mutation,
            LegacyCollaborationObservedMutationV1::InsertUntrimmedCommentAllowEmptyRootAndOrphanParent
        );
    }

    #[test]
    fn mobile_accepts_api_keys_but_web_surfaces_remain_session_only() {
        let mut mobile = mobile_create("hello");
        mobile.credential = Some(LegacyCollaborationCredentialV1::ApiKey);
        assert!(LegacyCollaborationAdapterV1::<RecordingPort>::prepare(mobile).is_ok());

        let mut web = request(LegacyCollaborationInputV1::WebDeleteCommentRoute {
            legacy_comment_id: Some(COMMENT.into()),
        });
        web.credential = Some(LegacyCollaborationCredentialV1::ApiKey);
        assert_eq!(
            LegacyCollaborationAdapterV1::<RecordingPort>::prepare(web),
            Err(LegacyCollaborationErrorV1::Unauthorized)
        );
    }

    #[test]
    fn mobile_creation_uses_ecmascript_trim_and_rejects_trimmed_empty() {
        let command = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(mobile_create(
            "\u{feff}\u{00a0} hello \u{3000}",
        ))
        .expect("mobile comment");
        let LegacyCollaborationMutationV1::Create(create) = command.mutation() else {
            unreachable!()
        };
        assert_eq!(create.content(), "hello");
        assert!(create.requires_video_authority());
        assert_eq!(create.kind(), LegacyCommentKindV1::Text);
        assert_eq!(
            create.notification_kind(),
            LegacyCollaborationNotificationKindV1::Comment
        );
        assert_eq!(
            LegacyCollaborationAdapterV1::<RecordingPort>::prepare(mobile_create(
                "\u{feff}\u{3000}"
            )),
            Err(LegacyCollaborationErrorV1::InvalidInput)
        );
    }

    #[test]
    fn new_comment_preserves_whitespace_empty_root_and_orphan_parent() {
        for parent in ["", "orphan-parent"] {
            let command = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request(
                LegacyCollaborationInputV1::WebNewCommentAction {
                    content: "   ".into(),
                    legacy_video_id: "orphan-video".into(),
                    kind: "text".into(),
                    author_image: Some("https://images.invalid/a".into()),
                    legacy_parent_comment_id: parent.into(),
                    timestamp: None,
                },
            ))
            .expect("web comment");
            let LegacyCollaborationMutationV1::Create(create) = command.mutation() else {
                unreachable!()
            };
            assert_eq!(create.content(), "   ");
            assert_eq!(create.legacy_parent_comment_id(), Some(parent));
            assert!(!create.requires_video_authority());
            assert_eq!(
                create.notification_kind(),
                if parent.is_empty() {
                    LegacyCollaborationNotificationKindV1::Comment
                } else {
                    LegacyCollaborationNotificationKindV1::Reply
                }
            );
        }
    }

    #[test]
    fn action_delete_fingerprint_binds_caller_parent_and_video_even_though_untrusted() {
        let make = |parent: Option<&str>, video: &str| {
            LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request(
                LegacyCollaborationInputV1::WebDeleteCommentAction {
                    legacy_comment_id: COMMENT.into(),
                    caller_parent_id: parent.map(str::to_owned),
                    legacy_video_id: video.into(),
                },
            ))
            .expect("delete action")
        };
        let root = make(None, VIDEO);
        let root_empty = make(Some(""), VIDEO);
        let reply = make(Some(PARENT), VIDEO);
        let other_video = make(None, "orphan-video");
        assert_ne!(root.request_digest(), root_empty.request_digest());
        assert_ne!(root.request_digest(), reply.request_digest());
        assert_ne!(root.request_digest(), other_video.request_digest());
    }

    #[test]
    fn route_query_presence_and_action_truthiness_remain_distinct() {
        assert_eq!(
            LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request(
                LegacyCollaborationInputV1::WebDeleteCommentRoute {
                    legacy_comment_id: None,
                }
            )),
            Err(LegacyCollaborationErrorV1::InvalidInput)
        );
        assert!(
            LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request(
                LegacyCollaborationInputV1::WebDeleteCommentRoute {
                    legacy_comment_id: Some(String::new()),
                }
            ))
            .is_ok()
        );
        assert_eq!(
            LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request(
                LegacyCollaborationInputV1::WebDeleteCommentAction {
                    legacy_comment_id: String::new(),
                    caller_parent_id: None,
                    legacy_video_id: VIDEO.into(),
                }
            )),
            Err(LegacyCollaborationErrorV1::InvalidInput)
        );
    }

    #[test]
    fn generated_comment_id_and_fingerprint_are_deterministic_but_surface_separated() {
        let first = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(mobile_create("hello"))
            .expect("first");
        let second = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(mobile_create("hello"))
            .expect("second");
        assert_eq!(first.request_digest(), second.request_digest());
        let (
            LegacyCollaborationMutationV1::Create(first_create),
            LegacyCollaborationMutationV1::Create(second_create),
        ) = (first.mutation(), second.mutation())
        else {
            unreachable!()
        };
        assert_eq!(
            first_create.legacy_comment_id(),
            second_create.legacy_comment_id()
        );

        let reaction = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request(
            LegacyCollaborationInputV1::MobileCreateReaction {
                legacy_video_id: VIDEO.into(),
                content: "hello".into(),
                timestamp: Some(1.25),
            },
        ))
        .expect("reaction");
        assert_ne!(first.request_digest(), reaction.request_digest());
    }

    #[test]
    fn secrets_and_raw_identifiers_are_redacted_from_debug() {
        let request = mobile_create("secret body");
        let command = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request.clone())
            .expect("command");
        for rendered in [format!("{request:?}"), format!("{command:?}")] {
            for secret in [VIDEO, "secret body", "collaboration-action-0001"] {
                assert!(!rendered.contains(secret));
            }
        }
    }

    struct RecordingPort {
        outcome: Mutex<
            Option<Result<LegacyCollaborationAtomicOutcomeV1, LegacyCollaborationAtomicErrorV1>>,
        >,
        calls: Mutex<Vec<LegacyCollaborationCommandV1>>,
    }

    impl RecordingPort {
        fn returning(
            outcome: Result<LegacyCollaborationAtomicOutcomeV1, LegacyCollaborationAtomicErrorV1>,
        ) -> Self {
            Self {
                outcome: Mutex::new(Some(outcome)),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LegacyCollaborationAtomicPortV1 for RecordingPort {
        async fn execute(
            &self,
            command: &LegacyCollaborationCommandV1,
        ) -> Result<LegacyCollaborationAtomicOutcomeV1, LegacyCollaborationAtomicErrorV1> {
            self.calls.lock().expect("calls").push(command.clone());
            self.outcome
                .lock()
                .expect("outcome")
                .take()
                .expect("one outcome")
        }
    }

    #[tokio::test]
    async fn adapter_projects_exact_mobile_create_and_replay() {
        let request = mobile_create(" hello ");
        let command = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request.clone())
            .expect("command");
        let port = RecordingPort::returning(Ok(LegacyCollaborationAtomicOutcomeV1 {
            request_digest: *command.request_digest(),
            result: LegacyCollaborationMutationResultV1::Created(created(&command)),
            replayed: true,
        }));
        let execution = LegacyCollaborationAdapterV1::new(&port)
            .execute(request)
            .await
            .expect("execution");
        let LegacyCollaborationSuccessV1::MobileComment(comment) = execution.success else {
            unreachable!()
        };
        assert_eq!(comment.content, "hello");
        assert_eq!(comment.author.id, ACTOR);
        assert_eq!(comment.kind, LegacyCommentKindV1::Text);
        assert!(execution.replayed);
        assert_eq!(port.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn mismatched_receipt_or_delete_cardinality_fails_closed() {
        let request = request(LegacyCollaborationInputV1::MobileDeleteComment {
            legacy_comment_id: COMMENT.into(),
        });
        let command = LegacyCollaborationAdapterV1::<RecordingPort>::prepare(request.clone())
            .expect("command");
        let port = RecordingPort::returning(Ok(LegacyCollaborationAtomicOutcomeV1 {
            request_digest: [9; 32],
            result: LegacyCollaborationMutationResultV1::Deleted(LegacyDeleteCommentResultV1 {
                deleted_comment_count: 1,
                deleted_notification_count: 0,
                notification_selector: None,
            }),
            replayed: false,
        }));
        assert_eq!(
            LegacyCollaborationAdapterV1::new(&port)
                .execute(request)
                .await,
            Err(LegacyCollaborationErrorV1::Internal)
        );

        assert!(!delete_result_matches_surface(
            command.surface(),
            &LegacyDeleteCommentResultV1 {
                deleted_comment_count: 2,
                deleted_notification_count: 0,
                notification_selector: None,
            }
        ));
    }

    #[tokio::test]
    async fn atomic_errors_map_to_stable_non_disclosing_failures() {
        for (atomic, expected) in [
            (
                LegacyCollaborationAtomicErrorV1::TargetMissing,
                LegacyCollaborationErrorV1::NotFound,
            ),
            (
                LegacyCollaborationAtomicErrorV1::AccessDenied,
                LegacyCollaborationErrorV1::NotFound,
            ),
            (
                LegacyCollaborationAtomicErrorV1::Conflict,
                LegacyCollaborationErrorV1::Conflict,
            ),
            (
                LegacyCollaborationAtomicErrorV1::Unavailable,
                LegacyCollaborationErrorV1::Unavailable,
            ),
            (
                LegacyCollaborationAtomicErrorV1::Corrupt,
                LegacyCollaborationErrorV1::Internal,
            ),
        ] {
            let request = mobile_create("hello");
            let port = RecordingPort::returning(Err(atomic));
            assert_eq!(
                LegacyCollaborationAdapterV1::new(&port)
                    .execute(request)
                    .await,
                Err(expected)
            );
        }
    }
}
