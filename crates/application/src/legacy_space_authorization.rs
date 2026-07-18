//! Source-pinned contract for Cap's space authorization server actions.
//!
//! Cap accepted a caller-supplied user ID. Frame preserves the returned
//! access projection and the two `requireSpaceManager` error messages while
//! deriving the actor from the host-only session and restricting the legacy
//! space alias to that actor's live active organization.

use async_trait::async_trait;
use thiserror::Error;

pub const LEGACY_SPACE_AUTHORIZATION_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_GET_SPACE_ACCESS_OPERATION_ID: &str = "cap-v1-5595a9d384765e76";
pub const LEGACY_REQUIRE_SPACE_MANAGER_OPERATION_ID: &str = "cap-v1-14cb48febfd0fa5a";
pub const LEGACY_GET_SPACE_ACCESS_IDENTITY: &str =
    "action://apps/web/actions/organization/space-authorization.ts#getSpaceAccess";
pub const LEGACY_REQUIRE_SPACE_MANAGER_IDENTITY: &str =
    "action://apps/web/actions/organization/space-authorization.ts#requireSpaceManager";
pub const LEGACY_SPACE_AUTHORIZATION_POLICY: &str = "organization_library.v1";
pub const LEGACY_SPACE_AUTHORIZATION_MAX_BODY_BYTES: usize = 1024;
pub const LEGACY_SPACE_AUTHORIZATION_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_SPACE_NOT_FOUND_MESSAGE: &str = "Space not found";
pub const LEGACY_SPACE_MANAGER_REQUIRED_MESSAGE: &str =
    "Only space admins, organization admins, and owners can manage this space";

// These are the deterministic two-source manifests in the pinned operation
// catalog. Keeping them beside the typed source closure makes report drift
// visible to the application tests and the central checker.
pub const LEGACY_GET_SPACE_ACCESS_SOURCE_MANIFEST_SHA256: &str =
    "b3cc205e302c0a50208b4a31e2c40144f438ba07965eabf0769e6458881e7183";
pub const LEGACY_REQUIRE_SPACE_MANAGER_SOURCE_MANIFEST_SHA256: &str =
    "69438e783f7cac4aeae4faa061188d6a38ea2ca2e23b9d42205c592c64ad3667";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySpaceAuthorizationSourceRoleV1 {
    Action,
    RolePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacySpaceAuthorizationSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacySpaceAuthorizationSourceRoleV1,
}

const SPACE_AUTHORIZATION_SOURCE_SHA256: &str =
    "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445";
const SPACE_ROLE_POLICY_SOURCE_SHA256: &str =
    "97bf35a09f4ef403dd0ffaa572c40c29f5776c4e6ae73c3e1e511ca376d5a407";

pub const LEGACY_GET_SPACE_ACCESS_SOURCES: &[LegacySpaceAuthorizationSourcePinV1] = &[
    LegacySpaceAuthorizationSourcePinV1 {
        path: "apps/web/actions/organization/space-authorization.ts",
        symbol: "getSpaceAccess",
        sha256: SPACE_AUTHORIZATION_SOURCE_SHA256,
        role: LegacySpaceAuthorizationSourceRoleV1::Action,
    },
    LegacySpaceAuthorizationSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        symbol: "space access roles",
        sha256: SPACE_ROLE_POLICY_SOURCE_SHA256,
        role: LegacySpaceAuthorizationSourceRoleV1::RolePolicy,
    },
];

pub const LEGACY_REQUIRE_SPACE_MANAGER_SOURCES: &[LegacySpaceAuthorizationSourcePinV1] = &[
    LegacySpaceAuthorizationSourcePinV1 {
        path: "apps/web/actions/organization/space-authorization.ts",
        symbol: "requireSpaceManager",
        sha256: SPACE_AUTHORIZATION_SOURCE_SHA256,
        role: LegacySpaceAuthorizationSourceRoleV1::Action,
    },
    LegacySpaceAuthorizationSourcePinV1 {
        path: "apps/web/lib/permissions/roles.ts",
        symbol: "space manager roles",
        sha256: SPACE_ROLE_POLICY_SOURCE_SHA256,
        role: LegacySpaceAuthorizationSourceRoleV1::RolePolicy,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySpaceAuthorizationActionV1 {
    GetSpaceAccess,
    RequireSpaceManager,
}

impl LegacySpaceAuthorizationActionV1 {
    #[must_use]
    pub fn parse(operation_id: &str) -> Option<Self> {
        match operation_id {
            LEGACY_GET_SPACE_ACCESS_OPERATION_ID => Some(Self::GetSpaceAccess),
            LEGACY_REQUIRE_SPACE_MANAGER_OPERATION_ID => Some(Self::RequireSpaceManager),
            _ => None,
        }
    }

    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::GetSpaceAccess => LEGACY_GET_SPACE_ACCESS_OPERATION_ID,
            Self::RequireSpaceManager => LEGACY_REQUIRE_SPACE_MANAGER_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::GetSpaceAccess => LEGACY_GET_SPACE_ACCESS_IDENTITY,
            Self::RequireSpaceManager => LEGACY_REQUIRE_SPACE_MANAGER_IDENTITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacySpaceAuthorizationProfileV1 {
    pub action: LegacySpaceAuthorizationActionV1,
    pub auth: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_SPACE_AUTHORIZATION_PROFILES: &[LegacySpaceAuthorizationProfileV1] = &[
    LegacySpaceAuthorizationProfileV1 {
        action: LegacySpaceAuthorizationActionV1::GetSpaceAccess,
        auth: "host_only_browser_session_actor",
        success: "space_access_object_or_null",
        validation: "exact_legacy_space_nanoid_and_strict_json_body",
        authorization: "live_active_tenant_and_tenant_scoped_immutable_space_alias",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "missing_foreign_deleted_or_tombstoned_space_projects_null",
    },
    LegacySpaceAuthorizationProfileV1 {
        action: LegacySpaceAuthorizationActionV1::RequireSpaceManager,
        auth: "host_only_browser_session_actor",
        success: "space_access_object_when_can_manage",
        validation: "exact_legacy_space_nanoid_and_strict_json_body",
        authorization: "organization_owner_or_admin_or_space_admin",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "exact_space_not_found_or_manager_required_error_message",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySpaceAuthorizationPrincipalV1 {
    pub actor_id: String,
    pub active_organization_id: String,
    pub active_legacy_organization_id: String,
}

impl LegacySpaceAuthorizationPrincipalV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_boundary_id(&self.actor_id)
            && valid_boundary_id(&self.active_organization_id)
            && valid_legacy_nanoid(&self.active_legacy_organization_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySpaceAuthorizationInputV1 {
    pub action: LegacySpaceAuthorizationActionV1,
    pub legacy_space_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationRoleV1 {
    Owner,
    Admin,
    Member,
}

impl LegacyOrganizationRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySpaceRoleV1 {
    Admin,
    Member,
}

impl LegacySpaceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
}

/// Raw D1 snapshot. Owner and creator identity comparisons intentionally use
/// Frame IDs inside the adapter; only lossless legacy aliases cross the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySpaceAuthorizationSnapshotV1 {
    pub legacy_space_id: String,
    pub legacy_organization_id: String,
    pub legacy_organization_owner_id: String,
    pub legacy_created_by_id: String,
    pub actor_is_organization_owner: bool,
    pub actor_is_space_creator: bool,
    pub organization_member_role: Option<String>,
    pub space_member_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySpaceAccessV1 {
    pub space_id: String,
    pub organization_id: String,
    pub organization_owner_id: String,
    pub created_by_id: String,
    pub organization_role: Option<LegacyOrganizationRoleV1>,
    pub space_role: Option<LegacySpaceRoleV1>,
    pub can_manage: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacySpaceAuthorizationResultV1 {
    GetSpaceAccess { access: Option<LegacySpaceAccessV1> },
    RequireSpaceManager { access: LegacySpaceAccessV1 },
    Thrown { message: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacySpaceAuthorizationPortErrorV1 {
    #[error("space authorization scope is no longer visible")]
    NotVisible,
    #[error("space authorization storage is unavailable")]
    Unavailable,
    #[error("space authorization projection is corrupt")]
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacySpaceAuthorizationServiceErrorV1 {
    #[error("space authorization input is invalid")]
    Invalid,
    #[error("space authorization storage is unavailable")]
    Unavailable,
    #[error("space authorization projection is corrupt")]
    Corrupt,
}

#[async_trait]
pub trait LegacySpaceAuthorizationPortV1: Send + Sync {
    async fn get_space_access(
        &self,
        principal: &LegacySpaceAuthorizationPrincipalV1,
        legacy_space_id: &str,
    ) -> Result<Option<LegacySpaceAuthorizationSnapshotV1>, LegacySpaceAuthorizationPortErrorV1>;
}

pub struct LegacySpaceAuthorizationServiceV1<'a, P> {
    port: &'a P,
}

impl<'a, P> LegacySpaceAuthorizationServiceV1<'a, P>
where
    P: LegacySpaceAuthorizationPortV1,
{
    #[must_use]
    pub const fn new(port: &'a P) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        principal: Option<&LegacySpaceAuthorizationPrincipalV1>,
        input: &LegacySpaceAuthorizationInputV1,
    ) -> Result<LegacySpaceAuthorizationResultV1, LegacySpaceAuthorizationServiceErrorV1> {
        if !valid_legacy_nanoid(&input.legacy_space_id) {
            return Err(LegacySpaceAuthorizationServiceErrorV1::Invalid);
        }
        let snapshot = if let Some(principal) = principal.filter(|value| value.valid()) {
            match self
                .port
                .get_space_access(principal, &input.legacy_space_id)
                .await
            {
                Ok(snapshot) => snapshot,
                Err(LegacySpaceAuthorizationPortErrorV1::NotVisible) => None,
                Err(LegacySpaceAuthorizationPortErrorV1::Unavailable) => {
                    return Err(LegacySpaceAuthorizationServiceErrorV1::Unavailable);
                }
                Err(LegacySpaceAuthorizationPortErrorV1::Corrupt) => {
                    return Err(LegacySpaceAuthorizationServiceErrorV1::Corrupt);
                }
            }
        } else {
            None
        };
        let access = snapshot
            .map(project_access)
            .transpose()?
            .filter(|access| access.space_id == input.legacy_space_id);
        Ok(match (input.action, access) {
            (LegacySpaceAuthorizationActionV1::GetSpaceAccess, access) => {
                LegacySpaceAuthorizationResultV1::GetSpaceAccess { access }
            }
            (LegacySpaceAuthorizationActionV1::RequireSpaceManager, None) => {
                LegacySpaceAuthorizationResultV1::Thrown {
                    message: LEGACY_SPACE_NOT_FOUND_MESSAGE,
                }
            }
            (LegacySpaceAuthorizationActionV1::RequireSpaceManager, Some(access))
                if !access.can_manage =>
            {
                LegacySpaceAuthorizationResultV1::Thrown {
                    message: LEGACY_SPACE_MANAGER_REQUIRED_MESSAGE,
                }
            }
            (LegacySpaceAuthorizationActionV1::RequireSpaceManager, Some(access)) => {
                LegacySpaceAuthorizationResultV1::RequireSpaceManager { access }
            }
        })
    }
}

fn project_access(
    snapshot: LegacySpaceAuthorizationSnapshotV1,
) -> Result<LegacySpaceAccessV1, LegacySpaceAuthorizationServiceErrorV1> {
    if ![
        snapshot.legacy_space_id.as_str(),
        snapshot.legacy_organization_id.as_str(),
        snapshot.legacy_organization_owner_id.as_str(),
        snapshot.legacy_created_by_id.as_str(),
    ]
    .into_iter()
    .all(valid_legacy_nanoid)
    {
        return Err(LegacySpaceAuthorizationServiceErrorV1::Corrupt);
    }
    let organization_role = effective_organization_role(
        snapshot.actor_is_organization_owner,
        snapshot.organization_member_role.as_deref(),
    );
    let space_role = effective_space_role(
        snapshot.actor_is_space_creator,
        snapshot.space_member_role.as_deref(),
    );
    let can_manage = can_manage_space(organization_role, space_role);
    Ok(LegacySpaceAccessV1 {
        space_id: snapshot.legacy_space_id,
        organization_id: snapshot.legacy_organization_id,
        organization_owner_id: snapshot.legacy_organization_owner_id,
        created_by_id: snapshot.legacy_created_by_id,
        organization_role,
        space_role,
        can_manage,
    })
}

#[must_use]
pub fn effective_organization_role(
    actor_is_owner: bool,
    member_role: Option<&str>,
) -> Option<LegacyOrganizationRoleV1> {
    if actor_is_owner {
        return Some(LegacyOrganizationRoleV1::Owner);
    }
    match member_role.map(str::to_ascii_lowercase).as_deref() {
        Some("owner") => Some(LegacyOrganizationRoleV1::Member),
        Some("admin") => Some(LegacyOrganizationRoleV1::Admin),
        Some("member") => Some(LegacyOrganizationRoleV1::Member),
        _ => None,
    }
}

#[must_use]
pub fn effective_space_role(
    actor_is_creator: bool,
    member_role: Option<&str>,
) -> Option<LegacySpaceRoleV1> {
    if actor_is_creator {
        return Some(LegacySpaceRoleV1::Admin);
    }
    match member_role.map(str::to_ascii_lowercase).as_deref() {
        Some("admin") => Some(LegacySpaceRoleV1::Admin),
        Some("member") => Some(LegacySpaceRoleV1::Member),
        _ => None,
    }
}

#[must_use]
pub const fn can_manage_space(
    organization_role: Option<LegacyOrganizationRoleV1>,
    space_role: Option<LegacySpaceRoleV1>,
) -> bool {
    matches!(
        organization_role,
        Some(LegacyOrganizationRoleV1::Owner | LegacyOrganizationRoleV1::Admin)
    ) || matches!(space_role, Some(LegacySpaceRoleV1::Admin))
}

fn valid_boundary_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

#[must_use]
pub fn valid_legacy_nanoid(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const SPACE_ID: &str = "0123456789abcdf";
    const ORGANIZATION_ID: &str = "0123456789abcdg";
    const OWNER_ID: &str = "0123456789abcdh";
    const CREATOR_ID: &str = "0123456789abcdj";

    #[derive(Debug)]
    struct FakePort {
        snapshot: Mutex<Option<LegacySpaceAuthorizationSnapshotV1>>,
        error: Option<LegacySpaceAuthorizationPortErrorV1>,
        calls: Mutex<usize>,
    }

    impl FakePort {
        fn with_snapshot(snapshot: Option<LegacySpaceAuthorizationSnapshotV1>) -> Self {
            Self {
                snapshot: Mutex::new(snapshot),
                error: None,
                calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl LegacySpaceAuthorizationPortV1 for FakePort {
        async fn get_space_access(
            &self,
            _principal: &LegacySpaceAuthorizationPrincipalV1,
            _legacy_space_id: &str,
        ) -> Result<Option<LegacySpaceAuthorizationSnapshotV1>, LegacySpaceAuthorizationPortErrorV1>
        {
            *self.calls.lock().expect("calls") += 1;
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(self.snapshot.lock().expect("snapshot").clone())
        }
    }

    fn principal() -> LegacySpaceAuthorizationPrincipalV1 {
        LegacySpaceAuthorizationPrincipalV1 {
            actor_id: "00000000-0000-4000-8000-000000000001".into(),
            active_organization_id: "00000000-0000-4000-8000-000000000002".into(),
            active_legacy_organization_id: ORGANIZATION_ID.into(),
        }
    }

    fn input(action: LegacySpaceAuthorizationActionV1) -> LegacySpaceAuthorizationInputV1 {
        LegacySpaceAuthorizationInputV1 {
            action,
            legacy_space_id: SPACE_ID.into(),
        }
    }

    fn snapshot(
        owner: bool,
        creator: bool,
        organization_role: Option<&str>,
        space_role: Option<&str>,
    ) -> LegacySpaceAuthorizationSnapshotV1 {
        LegacySpaceAuthorizationSnapshotV1 {
            legacy_space_id: SPACE_ID.into(),
            legacy_organization_id: ORGANIZATION_ID.into(),
            legacy_organization_owner_id: OWNER_ID.into(),
            legacy_created_by_id: CREATOR_ID.into(),
            actor_is_organization_owner: owner,
            actor_is_space_creator: creator,
            organization_member_role: organization_role.map(str::to_owned),
            space_member_role: space_role.map(str::to_owned),
        }
    }

    #[test]
    fn source_closure_and_profiles_pin_both_read_only_actions() {
        assert_eq!(LEGACY_GET_SPACE_ACCESS_SOURCES.len(), 2);
        assert_eq!(LEGACY_REQUIRE_SPACE_MANAGER_SOURCES.len(), 2);
        assert_eq!(LEGACY_SPACE_AUTHORIZATION_PROFILES.len(), 2);
        assert_eq!(
            LEGACY_SPACE_AUTHORIZATION_NO_PROTECTED_GATES,
            &[] as &[&str]
        );
        for action in [
            LegacySpaceAuthorizationActionV1::GetSpaceAccess,
            LegacySpaceAuthorizationActionV1::RequireSpaceManager,
        ] {
            assert_eq!(
                LegacySpaceAuthorizationActionV1::parse(action.operation_id()),
                Some(action)
            );
            assert!(action.identity().starts_with("action://"));
        }
        assert!(
            LEGACY_SPACE_AUTHORIZATION_PROFILES
                .iter()
                .all(|profile| profile.idempotency == "read_only_client_key_forbidden_retry_safe")
        );
    }

    #[test]
    fn exact_role_normalization_preserves_owner_creator_and_invalid_owner_semantics() {
        assert_eq!(
            effective_organization_role(true, Some("member")),
            Some(LegacyOrganizationRoleV1::Owner)
        );
        assert_eq!(
            effective_organization_role(false, Some("OWNER")),
            Some(LegacyOrganizationRoleV1::Member)
        );
        assert_eq!(effective_organization_role(false, Some("viewer")), None);
        assert_eq!(
            effective_space_role(true, Some("member")),
            Some(LegacySpaceRoleV1::Admin)
        );
        assert_eq!(effective_space_role(false, Some("contributor")), None);
    }

    #[test]
    fn can_manage_truth_table_matches_cap_roles() {
        assert!(can_manage_space(
            Some(LegacyOrganizationRoleV1::Owner),
            None
        ));
        assert!(can_manage_space(
            Some(LegacyOrganizationRoleV1::Admin),
            Some(LegacySpaceRoleV1::Member)
        ));
        assert!(can_manage_space(None, Some(LegacySpaceRoleV1::Admin)));
        assert!(!can_manage_space(
            Some(LegacyOrganizationRoleV1::Member),
            Some(LegacySpaceRoleV1::Member)
        ));
        assert!(!can_manage_space(None, None));
    }

    #[tokio::test]
    async fn access_projection_preserves_null_and_exact_require_errors() {
        let missing = FakePort::with_snapshot(None);
        let service = LegacySpaceAuthorizationServiceV1::new(&missing);
        assert_eq!(
            service
                .execute(
                    Some(&principal()),
                    &input(LegacySpaceAuthorizationActionV1::GetSpaceAccess)
                )
                .await,
            Ok(LegacySpaceAuthorizationResultV1::GetSpaceAccess { access: None })
        );
        assert_eq!(
            service
                .execute(
                    Some(&principal()),
                    &input(LegacySpaceAuthorizationActionV1::RequireSpaceManager)
                )
                .await,
            Ok(LegacySpaceAuthorizationResultV1::Thrown {
                message: LEGACY_SPACE_NOT_FOUND_MESSAGE
            })
        );

        let denied =
            FakePort::with_snapshot(Some(snapshot(false, false, Some("member"), Some("member"))));
        assert_eq!(
            LegacySpaceAuthorizationServiceV1::new(&denied)
                .execute(
                    Some(&principal()),
                    &input(LegacySpaceAuthorizationActionV1::RequireSpaceManager)
                )
                .await,
            Ok(LegacySpaceAuthorizationResultV1::Thrown {
                message: LEGACY_SPACE_MANAGER_REQUIRED_MESSAGE
            })
        );
    }

    #[tokio::test]
    async fn manager_success_returns_the_same_access_projection() {
        let port =
            FakePort::with_snapshot(Some(snapshot(false, true, Some("viewer"), Some("member"))));
        let result = LegacySpaceAuthorizationServiceV1::new(&port)
            .execute(
                Some(&principal()),
                &input(LegacySpaceAuthorizationActionV1::RequireSpaceManager),
            )
            .await
            .expect("success");
        let LegacySpaceAuthorizationResultV1::RequireSpaceManager { access } = result else {
            panic!("expected manager access");
        };
        assert_eq!(access.organization_role, None);
        assert_eq!(access.space_role, Some(LegacySpaceRoleV1::Admin));
        assert!(access.can_manage);
    }

    #[tokio::test]
    async fn invalid_alias_and_storage_corruption_fail_closed() {
        let port = FakePort::with_snapshot(Some(snapshot(false, false, None, None)));
        let mut invalid = input(LegacySpaceAuthorizationActionV1::GetSpaceAccess);
        invalid.legacy_space_id = "wrong".into();
        assert_eq!(
            LegacySpaceAuthorizationServiceV1::new(&port)
                .execute(Some(&principal()), &invalid)
                .await,
            Err(LegacySpaceAuthorizationServiceErrorV1::Invalid)
        );
        assert_eq!(*port.calls.lock().expect("calls"), 0);

        let corrupt = FakePort {
            snapshot: Mutex::new(None),
            error: Some(LegacySpaceAuthorizationPortErrorV1::Corrupt),
            calls: Mutex::new(0),
        };
        assert_eq!(
            LegacySpaceAuthorizationServiceV1::new(&corrupt)
                .execute(
                    Some(&principal()),
                    &input(LegacySpaceAuthorizationActionV1::GetSpaceAccess)
                )
                .await,
            Err(LegacySpaceAuthorizationServiceErrorV1::Corrupt)
        );
    }
}
