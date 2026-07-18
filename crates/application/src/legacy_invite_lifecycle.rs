//! Source-pinned contracts for Cap's invite accept/decline routes.
//!
//! The pinned routes are provider-free database transactions. They authenticate
//! before parsing JSON, compare the invitee email case-insensitively, and delete
//! the invite after either decision. Accept creates a normalized organization
//! membership when absent, conditionally allocates a paid seat, completes three
//! onboarding steps, and selects the organization. Decline removes an existing
//! membership and all of its space memberships, repairs active/default
//! organization pointers, and clears the inherited subscription only when no
//! other paid seat remains.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{OrganizationId, UserId};
use thiserror::Error;

pub const LEGACY_INVITE_LIFECYCLE_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_INVITE_ACCEPT_OPERATION_ID: &str = "cap-v1-447e3212d20351f6";
pub const LEGACY_INVITE_DECLINE_OPERATION_ID: &str = "cap-v1-cddad884de1190b1";
pub const LEGACY_INVITE_ACCEPT_PATH: &str = "/api/invite/accept";
pub const LEGACY_INVITE_DECLINE_PATH: &str = "/api/invite/decline";
pub const LEGACY_INVITE_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_INVITE_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_INVITE_RATE_LIMIT_BUCKET: &str = "auth_session.v1";
pub const LEGACY_INVITE_ACCEPT_SOURCE_MANIFEST_SHA256: &str =
    "319060081ae9a039e5068c4ddd7626a320590928ae456f3c08fc9dada1525409";
pub const LEGACY_INVITE_DECLINE_SOURCE_MANIFEST_SHA256: &str =
    "d50a659fa4d5155c15ddd3804165be3124cdb429f351ad90ed3985b7b1f4decb";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyInviteSourceRoleV1 {
    Route,
    Caller,
    Test,
    Session,
    Roles,
    ProSeats,
    Schema,
    Identifier,
    Database,
    DependencyLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyInviteSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
    pub role: LegacyInviteSourceRoleV1,
}

const CALLER: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "apps/web/app/(org)/invite/[inviteId]/InviteAccept.tsx",
    sha256: "987b73562aef6f5c5d6c8cfc4189572961fdd4ee5b992182f3b022d3d5dcb832",
    role: LegacyInviteSourceRoleV1::Caller,
};
const SESSION: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "packages/database/auth/session.ts",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyInviteSourceRoleV1::Session,
};
const AUTH_OPTIONS: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "packages/database/auth/auth-options.ts",
    sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    role: LegacyInviteSourceRoleV1::Session,
};
const SCHEMA: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "packages/database/schema.ts",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyInviteSourceRoleV1::Schema,
};
const DATABASE: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "packages/database/index.ts",
    sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    role: LegacyInviteSourceRoleV1::Database,
};
const ORGANISATION_ID: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "packages/web-domain/src/Organisation.ts",
    sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    role: LegacyInviteSourceRoleV1::Identifier,
};
const LOCKFILE: LegacyInviteSourcePinV1 = LegacyInviteSourcePinV1 {
    path: "pnpm-lock.yaml",
    sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    role: LegacyInviteSourceRoleV1::DependencyLock,
};

pub const LEGACY_INVITE_ACCEPT_SOURCES: &[LegacyInviteSourcePinV1] = &[
    LegacyInviteSourcePinV1 {
        path: "apps/web/app/api/invite/accept/route.ts",
        sha256: "e45eaf177c0608bc6cbfe41792da56fbb3397d0a43c2c85ae532d6240876f790",
        role: LegacyInviteSourceRoleV1::Route,
    },
    CALLER,
    SESSION,
    AUTH_OPTIONS,
    LegacyInviteSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        sha256: "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407",
        role: LegacyInviteSourceRoleV1::Roles,
    },
    LegacyInviteSourcePinV1 {
        path: "apps/web/utils/organization.ts",
        sha256: "dc966112b9258abb6ad4888651185614e6c48c2bd5e2abf536711b2d02af0e3b",
        role: LegacyInviteSourceRoleV1::ProSeats,
    },
    SCHEMA,
    LegacyInviteSourcePinV1 {
        path: "packages/database/helpers.ts",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
        role: LegacyInviteSourceRoleV1::Identifier,
    },
    DATABASE,
    ORGANISATION_ID,
    LOCKFILE,
];

pub const LEGACY_INVITE_DECLINE_SOURCES: &[LegacyInviteSourcePinV1] = &[
    LegacyInviteSourcePinV1 {
        path: "apps/web/app/api/invite/decline/route.ts",
        sha256: "df4e61e983c8691e359d5adb053b4c342ac2bd184f937740214c5ab345ef9c3e",
        role: LegacyInviteSourceRoleV1::Route,
    },
    CALLER,
    LegacyInviteSourcePinV1 {
        path: "apps/web/__tests__/unit/invite-decline.test.ts",
        sha256: "47a63825d40eb87a252ba74ee777584a3c7b85317aa3d3d935e93cf88947236e",
        role: LegacyInviteSourceRoleV1::Test,
    },
    SESSION,
    AUTH_OPTIONS,
    SCHEMA,
    DATABASE,
    ORGANISATION_ID,
    LOCKFILE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyInviteActionV1 {
    Accept,
    Decline,
}

impl LegacyInviteActionV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::Accept => LEGACY_INVITE_ACCEPT_OPERATION_ID,
            Self::Decline => LEGACY_INVITE_DECLINE_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Accept => LEGACY_INVITE_ACCEPT_PATH,
            Self::Decline => LEGACY_INVITE_DECLINE_PATH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyInviteProfileV1 {
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub pinned_commit: &'static str,
    pub source_manifest_sha256: &'static str,
    pub sources: &'static [LegacyInviteSourcePinV1],
    pub authentication: &'static str,
    pub idempotency: &'static str,
    pub rate_limit_bucket: &'static str,
    pub success_status: u16,
    pub success_body: &'static str,
    pub provider_effect: Option<&'static str>,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

pub const LEGACY_INVITE_ACCEPT_PROFILE: LegacyInviteProfileV1 = LegacyInviteProfileV1 {
    operation_id: LEGACY_INVITE_ACCEPT_OPERATION_ID,
    method: "POST",
    path: LEGACY_INVITE_ACCEPT_PATH,
    pinned_commit: LEGACY_INVITE_LIFECYCLE_CAP_COMMIT,
    source_manifest_sha256: LEGACY_INVITE_ACCEPT_SOURCE_MANIFEST_SHA256,
    sources: LEGACY_INVITE_ACCEPT_SOURCES,
    authentication: "session",
    idempotency: "forbidden",
    rate_limit_bucket: LEGACY_INVITE_RATE_LIMIT_BUCKET,
    success_status: 200,
    success_body: r#"{"success":true}"#,
    provider_effect: None,
    protected_gates: &[],
    production_promoted: true,
};

pub const LEGACY_INVITE_DECLINE_PROFILE: LegacyInviteProfileV1 = LegacyInviteProfileV1 {
    operation_id: LEGACY_INVITE_DECLINE_OPERATION_ID,
    method: "POST",
    path: LEGACY_INVITE_DECLINE_PATH,
    pinned_commit: LEGACY_INVITE_LIFECYCLE_CAP_COMMIT,
    source_manifest_sha256: LEGACY_INVITE_DECLINE_SOURCE_MANIFEST_SHA256,
    sources: LEGACY_INVITE_DECLINE_SOURCES,
    authentication: "session",
    idempotency: "forbidden",
    rate_limit_bucket: LEGACY_INVITE_RATE_LIMIT_BUCKET,
    success_status: 200,
    success_body: r#"{"success":true}"#,
    provider_effect: None,
    protected_gates: &[],
    production_promoted: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyInviteCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyInviteRequestV1 {
    pub credential: Option<LegacyInviteCredentialV1>,
    pub actor_id: Option<UserId>,
    pub action: LegacyInviteActionV1,
    pub legacy_invite_id: String,
}

impl fmt::Debug for LegacyInviteRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyInviteRequestV1")
            .field("credential", &self.credential)
            .field("actor", &self.actor_id.map(|_| "<redacted>"))
            .field("action", &self.action)
            .field("invite", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyInviteCommandV1 {
    actor_id: UserId,
    action: LegacyInviteActionV1,
    legacy_invite_id: String,
}

impl LegacyInviteCommandV1 {
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }

    #[must_use]
    pub const fn action(&self) -> LegacyInviteActionV1 {
        self.action
    }

    #[must_use]
    pub fn legacy_invite_id(&self) -> &str {
        &self.legacy_invite_id
    }
}

impl fmt::Debug for LegacyInviteCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyInviteCommandV1")
            .field("actor", &"<redacted>")
            .field("action", &self.action)
            .field("invite", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyInviteReceiptV1 {
    pub action: LegacyInviteActionV1,
    pub organization_id: OrganizationId,
    pub membership_created: bool,
    pub membership_removed: bool,
    pub pro_seat_assigned: bool,
    pub inherited_subscription_cleared: bool,
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyInviteErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid request body")]
    InvalidRequestBody,
    #[error("Invalid invite ID")]
    InvalidInviteId,
    #[error("Invite not found")]
    InviteNotFound,
    #[error("Email mismatch")]
    EmailMismatch,
    #[error("Internal server error")]
    Internal,
}

impl fmt::Debug for LegacyInviteErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "Unauthorized",
            Self::InvalidRequestBody => "InvalidRequestBody",
            Self::InvalidInviteId => "InvalidInviteId",
            Self::InviteNotFound => "InviteNotFound",
            Self::EmailMismatch => "EmailMismatch",
            Self::Internal => "Internal",
        })
    }
}

#[async_trait]
pub trait LegacyInviteAtomicPortV1: Send + Sync {
    async fn execute_atomic(
        &self,
        command: &LegacyInviteCommandV1,
    ) -> Result<LegacyInviteReceiptV1, LegacyInviteErrorV1>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyInviteAdapterV1 {
    action: LegacyInviteActionV1,
}

impl LegacyInviteAdapterV1 {
    #[must_use]
    pub const fn accept() -> Self {
        Self {
            action: LegacyInviteActionV1::Accept,
        }
    }

    #[must_use]
    pub const fn decline() -> Self {
        Self {
            action: LegacyInviteActionV1::Decline,
        }
    }

    pub fn prepare(
        self,
        request: &LegacyInviteRequestV1,
    ) -> Result<LegacyInviteCommandV1, LegacyInviteErrorV1> {
        if request.credential != Some(LegacyInviteCredentialV1::Session) {
            return Err(LegacyInviteErrorV1::Unauthorized);
        }
        if request.action != self.action {
            return Err(LegacyInviteErrorV1::InvalidRequestBody);
        }
        let actor_id = request.actor_id.ok_or(LegacyInviteErrorV1::Unauthorized)?;
        if request.legacy_invite_id.is_empty() {
            return Err(LegacyInviteErrorV1::InvalidInviteId);
        }
        Ok(LegacyInviteCommandV1 {
            actor_id,
            action: self.action,
            legacy_invite_id: request.legacy_invite_id.clone(),
        })
    }

    pub async fn execute<P>(
        self,
        port: &P,
        request: &LegacyInviteRequestV1,
    ) -> Result<LegacyInviteReceiptV1, LegacyInviteErrorV1>
    where
        P: LegacyInviteAtomicPortV1,
    {
        port.execute_atomic(&self.prepare(request)?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor() -> UserId {
        UserId::parse("00000000-0000-4000-8000-000000000001").expect("actor")
    }

    #[test]
    fn profiles_pin_complete_provider_free_closures() {
        assert_eq!(LEGACY_INVITE_ACCEPT_PROFILE.sources.len(), 11);
        assert_eq!(LEGACY_INVITE_DECLINE_PROFILE.sources.len(), 9);
        for profile in [LEGACY_INVITE_ACCEPT_PROFILE, LEGACY_INVITE_DECLINE_PROFILE] {
            assert_eq!(profile.pinned_commit, LEGACY_INVITE_LIFECYCLE_CAP_COMMIT);
            assert_eq!(profile.method, "POST");
            assert_eq!(profile.authentication, "session");
            assert_eq!(profile.idempotency, "forbidden");
            assert_eq!(profile.rate_limit_bucket, "auth_session.v1");
            assert_eq!(profile.success_status, 200);
            assert_eq!(profile.success_body, r#"{"success":true}"#);
            assert!(profile.provider_effect.is_none());
            assert!(profile.protected_gates.is_empty());
            assert!(profile.production_promoted);
            assert_eq!(profile.source_manifest_sha256.len(), 64);
            assert!(
                profile
                    .sources
                    .iter()
                    .all(|source| source.sha256.len() == 64)
            );
        }
    }

    #[test]
    fn adapter_preserves_auth_and_presence_ordering() {
        let anonymous = LegacyInviteRequestV1 {
            credential: None,
            actor_id: None,
            action: LegacyInviteActionV1::Accept,
            legacy_invite_id: String::new(),
        };
        assert_eq!(
            LegacyInviteAdapterV1::accept().prepare(&anonymous),
            Err(LegacyInviteErrorV1::Unauthorized)
        );
        let empty = LegacyInviteRequestV1 {
            credential: Some(LegacyInviteCredentialV1::Session),
            actor_id: Some(actor()),
            action: LegacyInviteActionV1::Accept,
            legacy_invite_id: String::new(),
        };
        assert_eq!(
            LegacyInviteAdapterV1::accept().prepare(&empty),
            Err(LegacyInviteErrorV1::InvalidInviteId)
        );
    }

    #[test]
    fn selector_cannot_cross_actions() {
        let request = LegacyInviteRequestV1 {
            credential: Some(LegacyInviteCredentialV1::Session),
            actor_id: Some(actor()),
            action: LegacyInviteActionV1::Decline,
            legacy_invite_id: "0123456789abcde".into(),
        };
        assert_eq!(
            LegacyInviteAdapterV1::accept().prepare(&request),
            Err(LegacyInviteErrorV1::InvalidRequestBody)
        );
        let command = LegacyInviteAdapterV1::decline()
            .prepare(&request)
            .expect("matching action");
        assert_eq!(command.action(), LegacyInviteActionV1::Decline);
        assert_eq!(command.actor_id(), actor());
    }
}
