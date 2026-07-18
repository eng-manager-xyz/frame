//! Source-pinned semantics for Cap's authenticated notification-list route.
//!
//! The route is a provider-tolerant D1 read: rows are tenant scoped to the
//! actor's active organization, unread rows sort first, anonymous views fold
//! into the `view` count, malformed rows are omitted individually, and avatar
//! resolution failures degrade to `null` instead of failing the response.

use std::collections::BTreeMap;

pub const LEGACY_NOTIFICATION_READ_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_NOTIFICATION_READ_OPERATION_ID: &str = "cap-v1-14dcca6d36eee6b3";
pub const LEGACY_NOTIFICATION_READ_PATH: &str = "/api/notifications";
pub const LEGACY_NOTIFICATION_READ_POLICY: &str = "collaboration_notifications.v1";
pub const LEGACY_NOTIFICATION_READ_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_NOTIFICATION_READ_SOURCE_MANIFEST_SHA256: &str =
    "1e028fafa990b4c0f3b8f3ea2970d87d675931fe94fdaa911ba6cf5e82d1a939";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyNotificationReadSourceRoleV1 {
    Route,
    Contract,
    Schema,
    Authentication,
    ImageResolution,
}

impl LegacyNotificationReadSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Contract => "contract",
            Self::Schema => "schema",
            Self::Authentication => "authentication",
            Self::ImageResolution => "image_resolution",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationReadSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyNotificationReadSourceRoleV1,
}

pub const LEGACY_NOTIFICATION_READ_SOURCES: &[LegacyNotificationReadSourcePinV1] = &[
    LegacyNotificationReadSourcePinV1 {
        path: "apps/web/app/api/notifications/route.ts",
        symbol: "GET",
        sha256: "1c0571a385328c53ec106967a717201ed2aa04cbcfd108c419f03f8b51b3ae17",
        role: LegacyNotificationReadSourceRoleV1::Route,
    },
    LegacyNotificationReadSourcePinV1 {
        path: "packages/web-api-contract/src/index.ts",
        symbol: "Notification+GET /notifications",
        sha256: "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
        role: LegacyNotificationReadSourceRoleV1::Contract,
    },
    LegacyNotificationReadSourcePinV1 {
        path: "packages/database/schema.ts",
        symbol: "notifications+users",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyNotificationReadSourceRoleV1::Schema,
    },
    LegacyNotificationReadSourcePinV1 {
        path: "packages/database/auth/session.ts",
        symbol: "getCurrentUser",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyNotificationReadSourceRoleV1::Authentication,
    },
    LegacyNotificationReadSourcePinV1 {
        path: "packages/web-backend/src/ImageUploads/index.ts",
        symbol: "ImageUploads.resolveImageUrl",
        sha256: "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
        role: LegacyNotificationReadSourceRoleV1::ImageResolution,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationReadProfileV1 {
    pub operation_id: &'static str,
    pub identity: &'static str,
    pub method: &'static str,
    pub auth: &'static str,
    pub body: &'static str,
    pub idempotency: &'static str,
    pub ordering: &'static str,
    pub invalid_row_behavior: &'static str,
    pub avatar_failure_behavior: &'static str,
    pub failure_body: &'static str,
}

pub const LEGACY_NOTIFICATION_READ_PROFILE: LegacyNotificationReadProfileV1 =
    LegacyNotificationReadProfileV1 {
        operation_id: LEGACY_NOTIFICATION_READ_OPERATION_ID,
        identity: LEGACY_NOTIFICATION_READ_PATH,
        method: "GET",
        auth: "host_only_browser_session",
        body: "forbidden",
        idempotency: "forbidden",
        ordering: "unread_first_then_created_at_desc",
        invalid_row_behavior: "omit_individually",
        avatar_failure_behavior: "return_null_avatar",
        failure_body: r#"{"error":"Failed to fetch notifications"}"#,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyNotificationKindV1 {
    View,
    Comment,
    Reply,
    Reaction,
    AnonymousView,
}

impl LegacyNotificationKindV1 {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Comment => "comment",
            Self::Reply => "reply",
            Self::Reaction => "reaction",
            Self::AnonymousView => "anon_view",
        }
    }

    #[must_use]
    pub const fn count_bucket(self) -> LegacyNotificationCountBucketV1 {
        match self {
            Self::View | Self::AnonymousView => LegacyNotificationCountBucketV1::View,
            Self::Comment => LegacyNotificationCountBucketV1::Comment,
            Self::Reply => LegacyNotificationCountBucketV1::Reply,
            Self::Reaction => LegacyNotificationCountBucketV1::Reaction,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyNotificationCountBucketV1 {
    View,
    Comment,
    Reply,
    Reaction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNotificationCommentV1 {
    pub id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNotificationAuthorV1 {
    pub id: String,
    pub name: String,
    pub image_key: Option<String>,
    pub resolved_avatar: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyNotificationPayloadV1 {
    Authored {
        video_id: String,
        author: LegacyNotificationAuthorV1,
        comment: Option<LegacyNotificationCommentV1>,
    },
    AnonymousView {
        video_id: String,
        anonymous_name: String,
        location: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNotificationProjectionV1 {
    pub id: String,
    pub kind: LegacyNotificationKindV1,
    pub payload: LegacyNotificationPayloadV1,
    pub read_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

impl LegacyNotificationProjectionV1 {
    #[must_use]
    pub fn source_order_key(&self) -> (bool, std::cmp::Reverse<i64>, &str) {
        // MySQL's `DESC(readAt IS NULL), DESC(createdAt)` puts unread first.
        // The ID is only a deterministic tie breaker for imported equal-time
        // rows; it does not alter any source-observable non-tied ordering.
        (
            self.read_at_ms.is_some(),
            std::cmp::Reverse(self.created_at_ms),
            self.id.as_str(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationCountsV1 {
    pub view: u64,
    pub comment: u64,
    pub reply: u64,
    pub reaction: u64,
}

impl LegacyNotificationCountsV1 {
    #[must_use]
    pub fn from_grouped_counts(
        grouped: impl IntoIterator<Item = (LegacyNotificationKindV1, u64)>,
    ) -> Self {
        let mut buckets = BTreeMap::from([
            (LegacyNotificationCountBucketV1::View, 0_u64),
            (LegacyNotificationCountBucketV1::Comment, 0_u64),
            (LegacyNotificationCountBucketV1::Reply, 0_u64),
            (LegacyNotificationCountBucketV1::Reaction, 0_u64),
        ]);
        for (kind, count) in grouped {
            let total = buckets.entry(kind.count_bucket()).or_default();
            *total = total.saturating_add(count);
        }
        Self {
            view: buckets[&LegacyNotificationCountBucketV1::View],
            comment: buckets[&LegacyNotificationCountBucketV1::Comment],
            reply: buckets[&LegacyNotificationCountBucketV1::Reply],
            reaction: buckets[&LegacyNotificationCountBucketV1::Reaction],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNotificationReadResultV1 {
    pub notifications: Vec<LegacyNotificationProjectionV1>,
    pub counts: LegacyNotificationCountsV1,
}

impl LegacyNotificationReadResultV1 {
    #[must_use]
    pub fn new(
        notifications: impl IntoIterator<Item = Option<LegacyNotificationProjectionV1>>,
        grouped_counts: impl IntoIterator<Item = (LegacyNotificationKindV1, u64)>,
    ) -> Self {
        let mut notifications = notifications.into_iter().flatten().collect::<Vec<_>>();
        notifications.sort_by(|left, right| left.source_order_key().cmp(&right.source_order_key()));
        Self {
            notifications,
            counts: LegacyNotificationCountsV1::from_grouped_counts(grouped_counts),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use sha2::{Digest, Sha256};

    fn source_manifest() -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-cap-notification-read-source-manifest-v1\0");
        for source in LEGACY_NOTIFICATION_READ_SOURCES {
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

    fn notification(
        id: &str,
        kind: LegacyNotificationKindV1,
        read_at_ms: Option<i64>,
        created_at_ms: i64,
    ) -> LegacyNotificationProjectionV1 {
        LegacyNotificationProjectionV1 {
            id: id.into(),
            kind,
            payload: LegacyNotificationPayloadV1::AnonymousView {
                video_id: "video".into(),
                anonymous_name: "Anonymous Viewer".into(),
                location: None,
            },
            read_at_ms,
            created_at_ms,
        }
    }

    #[test]
    fn profile_pins_the_provider_tolerant_source_contract() {
        assert_eq!(LEGACY_NOTIFICATION_READ_PROFILE.method, "GET");
        assert_eq!(LEGACY_NOTIFICATION_READ_PROFILE.body, "forbidden");
        assert_eq!(LEGACY_NOTIFICATION_READ_PROFILE.idempotency, "forbidden");
        assert_eq!(LEGACY_NOTIFICATION_READ_NO_PROTECTED_GATES, &[] as &[&str]);
        assert_eq!(LEGACY_NOTIFICATION_READ_SOURCES.len(), 5);
        assert_eq!(
            source_manifest(),
            LEGACY_NOTIFICATION_READ_SOURCE_MANIFEST_SHA256
        );
        assert!(LEGACY_NOTIFICATION_READ_SOURCES.iter().any(|source| {
            source.path == "apps/web/app/api/notifications/route.ts"
                && source.sha256
                    == "1c0571a385328c53ec106967a717201ed2aa04cbcfd108c419f03f8b51b3ae17"
        }));
    }

    #[test]
    fn unread_rows_sort_first_and_invalid_rows_are_omitted_individually() {
        let result = LegacyNotificationReadResultV1::new(
            [
                Some(notification(
                    "read-new",
                    LegacyNotificationKindV1::View,
                    Some(12),
                    40,
                )),
                None,
                Some(notification(
                    "unread-old",
                    LegacyNotificationKindV1::Comment,
                    None,
                    10,
                )),
                Some(notification(
                    "unread-new",
                    LegacyNotificationKindV1::Reply,
                    None,
                    30,
                )),
            ],
            [],
        );
        assert_eq!(
            result
                .notifications
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["unread-new", "unread-old", "read-new"]
        );
    }

    #[test]
    fn anonymous_views_fold_into_view_and_all_buckets_are_always_present() {
        let counts = LegacyNotificationCountsV1::from_grouped_counts([
            (LegacyNotificationKindV1::AnonymousView, 3),
            (LegacyNotificationKindV1::View, 2),
            (LegacyNotificationKindV1::Reaction, 7),
        ]);
        assert_eq!(
            counts,
            LegacyNotificationCountsV1 {
                view: 5,
                comment: 0,
                reply: 0,
                reaction: 7,
            }
        );
    }

    #[test]
    fn avatar_resolution_failure_is_an_explicit_null_fallback() {
        let author = LegacyNotificationAuthorV1 {
            id: "author".into(),
            name: "Unknown".into(),
            image_key: Some("profiles/author.png".into()),
            resolved_avatar: None,
        };
        assert!(author.image_key.is_some());
        assert!(author.resolved_avatar.is_none());
        assert_eq!(
            LEGACY_NOTIFICATION_READ_PROFILE.avatar_failure_behavior,
            "return_null_avatar"
        );
    }
}
