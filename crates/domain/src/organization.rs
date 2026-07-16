//! Organization, collaboration, and centralized authorization contracts.
//!
//! `AuthorizationPolicy` remains the single policy type for the workspace. The
//! original video-oriented evaluator lives in `contracts`; this module extends
//! the same type with the complete organization/object matrix used by new
//! collaboration workflows.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    AuthorizationPolicy, CorrelationId, DurationMillis, FolderId, IdempotencyKey, MembershipState,
    OrganizationId, OrganizationRole, SecretDigest, SpaceId, SpaceRole, TenantId, TimestampMillis,
    UserId, VersionedSecretDigest,
};

pub const MAX_ORGANIZATION_NAME_BYTES: usize = 160;
pub const MAX_FOLDER_NAME_BYTES: usize = 255;
pub const MAX_SETTINGS_BYTES: usize = 16 * 1024;
pub const MAX_SETTINGS_DEPTH: usize = 8;
pub const MAX_FOLDER_DEPTH: u8 = 32;
pub const MAX_ALLOWED_DOMAINS: u16 = 256;
pub const MAX_ORGANIZATION_MEMBERS: u32 = 100_000;

macro_rules! organization_uuid {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(value: &str) -> Result<Self, OrganizationContractError> {
                let parsed = Uuid::parse_str(value)
                    .ok()
                    .filter(|candidate| !candidate.is_nil())
                    .ok_or(OrganizationContractError::InvalidIdentifier($kind))?;
                Ok(Self(parsed))
            }

            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = OrganizationContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

organization_uuid!(OrganizationInviteId, "organization invite");
organization_uuid!(OrganizationOperationId, "organization operation");
organization_uuid!(OrganizationAuditId, "organization audit");
organization_uuid!(OrganizationRepairPlanId, "organization repair plan");

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OrganizationContractError {
    #[error("invalid {0} identifier")]
    InvalidIdentifier(&'static str),
    #[error("organization and tenant scopes do not match")]
    ScopeMismatch,
    #[error("name is outside the supported bounds")]
    InvalidName,
    #[error("settings are outside the supported bounds")]
    InvalidSettings,
    #[error("domain is invalid")]
    InvalidDomain,
    #[error("revision is outside the supported range")]
    InvalidRevision,
    #[error("folder depth is outside the supported range")]
    InvalidFolderDepth,
    #[error("invite secret is invalid")]
    InvalidInviteSecret,
    #[error("time range is invalid")]
    InvalidTimeRange,
    #[error("tombstone policy is invalid")]
    InvalidTombstonePolicy,
}

/// Current authority invariant: an organization is the tenant boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrganizationScope {
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
}

impl OrganizationScope {
    pub fn new(
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> Result<Self, OrganizationContractError> {
        if tenant_id.as_uuid() != organization_id.as_uuid() {
            return Err(OrganizationContractError::ScopeMismatch);
        }
        Ok(Self {
            tenant_id,
            organization_id,
        })
    }

    pub fn from_organization(
        organization_id: OrganizationId,
    ) -> Result<Self, OrganizationContractError> {
        let tenant_id = TenantId::parse(&organization_id.to_string())
            .map_err(|_| OrganizationContractError::ScopeMismatch)?;
        Self::new(tenant_id, organization_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrganizationRevision(u64);

impl OrganizationRevision {
    pub const INITIAL: Self = Self(0);

    pub fn new(value: u64) -> Result<Self, OrganizationContractError> {
        if value > crate::MAX_WIRE_INTEGER {
            return Err(OrganizationContractError::InvalidRevision);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, OrganizationContractError> {
        self.0
            .checked_add(1)
            .ok_or(OrganizationContractError::InvalidRevision)
            .and_then(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrganizationAuthorityFence {
    pub identity_revision: OrganizationRevision,
    pub session_version: OrganizationRevision,
    pub organization_revision: OrganizationRevision,
    pub organization_authority_version: OrganizationRevision,
    pub membership_revision: OrganizationRevision,
    pub membership_authority_version: OrganizationRevision,
    pub space_membership_revision: Option<OrganizationRevision>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrganizationName(String);

impl OrganizationName {
    pub fn parse(value: impl Into<String>) -> Result<Self, OrganizationContractError> {
        bounded_name(value.into(), MAX_ORGANIZATION_NAME_BYTES).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OrganizationName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OrganizationName")
            .field(&self.0)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CollaborationName(String);

impl CollaborationName {
    pub fn parse(value: impl Into<String>) -> Result<Self, OrganizationContractError> {
        bounded_name(value.into(), MAX_FOLDER_NAME_BYTES).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CollaborationName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CollaborationName")
            .field(&self.0)
            .finish()
    }
}

fn bounded_name(value: String, maximum: usize) -> Result<String, OrganizationContractError> {
    if value.is_empty()
        || value.len() > maximum
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(OrganizationContractError::InvalidName);
    }
    Ok(value)
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrganizationSettings(String);

impl OrganizationSettings {
    pub fn parse(value: impl Into<String>) -> Result<Self, OrganizationContractError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_SETTINGS_BYTES {
            return Err(OrganizationContractError::InvalidSettings);
        }
        let parsed: Value =
            serde_json::from_str(&value).map_err(|_| OrganizationContractError::InvalidSettings)?;
        if !parsed.is_object() || !bounded_json(&parsed, 1) {
            return Err(OrganizationContractError::InvalidSettings);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_json(&self) -> &str {
        &self.0
    }
}

impl Default for OrganizationSettings {
    fn default() -> Self {
        Self("{}".into())
    }
}

impl fmt::Debug for OrganizationSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OrganizationSettings")
            .field("bytes", &self.0.len())
            .finish()
    }
}

fn bounded_json(value: &Value, depth: usize) -> bool {
    if depth > MAX_SETTINGS_DEPTH {
        return false;
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(value) => value.len() <= 4_096 && !value.chars().any(char::is_control),
        Value::Array(values) => {
            values.len() <= 128 && values.iter().all(|value| bounded_json(value, depth + 1))
        }
        Value::Object(values) => {
            values.len() <= 128
                && values.iter().all(|(key, value)| {
                    !key.is_empty()
                        && key.len() <= 128
                        && !key.chars().any(char::is_control)
                        && bounded_json(value, depth + 1)
                })
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AllowedDomain(String);

impl AllowedDomain {
    pub fn parse(value: impl Into<String>) -> Result<Self, OrganizationContractError> {
        let value = value.into().to_ascii_lowercase();
        if value.is_empty()
            || value.len() > 253
            || value.ends_with('.')
            || value.parse::<std::net::IpAddr>().is_ok()
            || !value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
        {
            return Err(OrganizationContractError::InvalidDomain);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AllowedDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AllowedDomain([redacted])")
    }
}

/// A one-use bearer secret. It deliberately has no `Clone`, `Serialize`, or display method.
pub struct OrganizationInviteSecret(Vec<u8>);

impl OrganizationInviteSecret {
    pub fn parse(value: impl Into<Vec<u8>>) -> Result<Self, OrganizationContractError> {
        let value = value.into();
        if !(32..=128).contains(&value.len())
            || !value.iter().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
        {
            return Err(OrganizationContractError::InvalidInviteSecret);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_hashing(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for OrganizationInviteSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OrganizationInviteSecret([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationStatus {
    Active,
    Tombstoned,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationObjectState {
    Active,
    Tombstoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalState {
    Active,
    Suspended,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteState {
    Pending,
    Accepted,
    Declined,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationObjectKind {
    Organization,
    Membership,
    Invite,
    AllowedDomain,
    Settings,
    Seat,
    Space,
    Folder,
    Tombstone,
    RepairPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationAction {
    CreateOrganization,
    ReadOrganization,
    UpdateOrganization,
    ManageSettings,
    ManageBilling,
    ManageSeat,
    IssueInvite,
    RevokeInvite,
    AcceptInvite,
    ReadMembers,
    ChangeMemberRole,
    RemoveMember,
    TransferOwnership,
    ManageAllowedDomain,
    ReadSpace,
    CreateSpace,
    ManageSpace,
    ChangeSpaceRole,
    ReadFolder,
    CreateFolder,
    ManageFolder,
    MoveFolder,
    TombstoneOrganization,
    RecoverOrganization,
    AuditGraph,
    PlanRepair,
}

impl OrganizationAction {
    pub const ALL: [Self; 26] = [
        Self::CreateOrganization,
        Self::ReadOrganization,
        Self::UpdateOrganization,
        Self::ManageSettings,
        Self::ManageBilling,
        Self::ManageSeat,
        Self::IssueInvite,
        Self::RevokeInvite,
        Self::AcceptInvite,
        Self::ReadMembers,
        Self::ChangeMemberRole,
        Self::RemoveMember,
        Self::TransferOwnership,
        Self::ManageAllowedDomain,
        Self::ReadSpace,
        Self::CreateSpace,
        Self::ManageSpace,
        Self::ChangeSpaceRole,
        Self::ReadFolder,
        Self::CreateFolder,
        Self::ManageFolder,
        Self::MoveFolder,
        Self::TombstoneOrganization,
        Self::RecoverOrganization,
        Self::AuditGraph,
        Self::PlanRepair,
    ];

    #[must_use]
    pub const fn object_kind(self) -> OrganizationObjectKind {
        match self {
            Self::CreateOrganization | Self::ReadOrganization | Self::UpdateOrganization => {
                OrganizationObjectKind::Organization
            }
            Self::ManageSettings => OrganizationObjectKind::Settings,
            Self::ManageBilling | Self::ManageSeat => OrganizationObjectKind::Seat,
            Self::IssueInvite | Self::RevokeInvite | Self::AcceptInvite => {
                OrganizationObjectKind::Invite
            }
            Self::ReadMembers
            | Self::ChangeMemberRole
            | Self::RemoveMember
            | Self::TransferOwnership => OrganizationObjectKind::Membership,
            Self::ManageAllowedDomain => OrganizationObjectKind::AllowedDomain,
            Self::ReadSpace | Self::CreateSpace | Self::ManageSpace | Self::ChangeSpaceRole => {
                OrganizationObjectKind::Space
            }
            Self::ReadFolder | Self::CreateFolder | Self::ManageFolder | Self::MoveFolder => {
                OrganizationObjectKind::Folder
            }
            Self::TombstoneOrganization | Self::RecoverOrganization => {
                OrganizationObjectKind::Tombstone
            }
            Self::AuditGraph | Self::PlanRepair => OrganizationObjectKind::RepairPlan,
        }
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::CreateOrganization => "organization_create",
            Self::ReadOrganization => "organization_read",
            Self::UpdateOrganization => "organization_update",
            Self::ManageSettings => "settings_manage",
            Self::ManageBilling => "billing_manage",
            Self::ManageSeat => "seat_manage",
            Self::IssueInvite => "invite_issue",
            Self::RevokeInvite => "invite_revoke",
            Self::AcceptInvite => "invite_accept",
            Self::ReadMembers => "members_read",
            Self::ChangeMemberRole => "member_role_change",
            Self::RemoveMember => "member_remove",
            Self::TransferOwnership => "ownership_transfer",
            Self::ManageAllowedDomain => "allowed_domain_manage",
            Self::ReadSpace => "space_read",
            Self::CreateSpace => "space_create",
            Self::ManageSpace => "space_manage",
            Self::ChangeSpaceRole => "space_role_change",
            Self::ReadFolder => "folder_read",
            Self::CreateFolder => "folder_create",
            Self::ManageFolder => "folder_manage",
            Self::MoveFolder => "folder_move",
            Self::TombstoneOrganization => "organization_tombstone",
            Self::RecoverOrganization => "organization_recover",
            Self::AuditGraph => "graph_audit",
            Self::PlanRepair => "repair_plan",
        }
    }
}

/// Public denials deliberately collapse missing IDs and tenant mismatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationDenialReason {
    AccessDenied,
    InactiveAuthority,
    StateDenied,
    StaleAuthority,
    RetentionLocked,
}

impl OrganizationDenialReason {
    #[must_use]
    pub const fn public_code(self) -> &'static str {
        match self {
            Self::AccessDenied => "organization_access_denied",
            Self::InactiveAuthority => "organization_authority_inactive",
            Self::StateDenied => "organization_state_denied",
            Self::StaleAuthority => "organization_authority_stale",
            Self::RetentionLocked => "organization_retention_locked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrganizationAuthorizationDecision {
    Allow,
    Deny(OrganizationDenialReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizationPolicyMembership {
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
    pub user_id: UserId,
    pub role: OrganizationRole,
    pub state: MembershipState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizationPolicyContext {
    pub actor_tenant_id: TenantId,
    pub resource_tenant_id: TenantId,
    pub organization_id: OrganizationId,
    pub actor_id: UserId,
    pub principal_state: PrincipalState,
    pub organization_status: OrganizationStatus,
    pub membership: Option<OrganizationPolicyMembership>,
    pub space_role: Option<SpaceRole>,
    pub object_kind: OrganizationObjectKind,
    pub object_state: CollaborationObjectState,
    pub owns_object: bool,
    /// True only after token preflight. D1 reasserts the token and binds the
    /// invitation's versioned identifier digest to the authenticated actor.
    pub invite_grant: bool,
    pub authenticated_support: bool,
}

impl AuthorizationPolicy {
    /// Evaluate organization authorization without network or database access.
    #[must_use]
    pub fn evaluate_organization(
        context: OrganizationPolicyContext,
        action: OrganizationAction,
    ) -> OrganizationAuthorizationDecision {
        if action.object_kind() != context.object_kind {
            return OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied);
        }
        let scope_matches = context.actor_tenant_id == context.resource_tenant_id
            && context.actor_tenant_id.as_uuid() == context.organization_id.as_uuid();
        if !scope_matches {
            return OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied);
        }
        if context.principal_state != PrincipalState::Active {
            return OrganizationAuthorizationDecision::Deny(
                OrganizationDenialReason::InactiveAuthority,
            );
        }

        if context.object_state != CollaborationObjectState::Active
            && !matches!(
                action,
                OrganizationAction::AuditGraph | OrganizationAction::PlanRepair
            )
        {
            return OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::StateDenied);
        }

        match context.organization_status {
            OrganizationStatus::Deleted => {
                return OrganizationAuthorizationDecision::Deny(
                    OrganizationDenialReason::StateDenied,
                );
            }
            OrganizationStatus::Tombstoned
                if !matches!(
                    action,
                    OrganizationAction::ReadOrganization
                        | OrganizationAction::RecoverOrganization
                        | OrganizationAction::AuditGraph
                        | OrganizationAction::PlanRepair
                ) =>
            {
                return OrganizationAuthorizationDecision::Deny(
                    OrganizationDenialReason::StateDenied,
                );
            }
            OrganizationStatus::Active if action == OrganizationAction::RecoverOrganization => {
                return OrganizationAuthorizationDecision::Deny(
                    OrganizationDenialReason::StateDenied,
                );
            }
            OrganizationStatus::Active | OrganizationStatus::Tombstoned => {}
        }

        if matches!(
            action,
            OrganizationAction::AuditGraph | OrganizationAction::PlanRepair
        ) {
            return if context.authenticated_support {
                OrganizationAuthorizationDecision::Allow
            } else {
                OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
            };
        }

        if action == OrganizationAction::CreateOrganization {
            return if context.membership.is_none() {
                OrganizationAuthorizationDecision::Allow
            } else {
                OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
            };
        }

        if action == OrganizationAction::AcceptInvite {
            return if context.membership.is_none() && context.invite_grant {
                OrganizationAuthorizationDecision::Allow
            } else {
                OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
            };
        }

        let Some(membership) = context.membership else {
            return OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied);
        };
        if membership.tenant_id != context.actor_tenant_id
            || membership.organization_id != context.organization_id
            || membership.user_id != context.actor_id
        {
            return OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied);
        }
        if membership.state != MembershipState::Active {
            return OrganizationAuthorizationDecision::Deny(
                OrganizationDenialReason::InactiveAuthority,
            );
        }

        let allowed = match membership.role {
            OrganizationRole::Owner => true,
            OrganizationRole::Admin => !matches!(
                action,
                OrganizationAction::TransferOwnership
                    | OrganizationAction::ManageBilling
                    | OrganizationAction::TombstoneOrganization
                    | OrganizationAction::RecoverOrganization
            ),
            OrganizationRole::Member => member_action_allowed(context, action),
            OrganizationRole::Viewer => matches!(
                action,
                OrganizationAction::ReadOrganization
                    | OrganizationAction::ReadMembers
                    | OrganizationAction::ReadSpace
                    | OrganizationAction::ReadFolder
            ),
        };
        if allowed {
            OrganizationAuthorizationDecision::Allow
        } else {
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        }
    }
}

const fn member_action_allowed(
    context: OrganizationPolicyContext,
    action: OrganizationAction,
) -> bool {
    match action {
        OrganizationAction::ReadOrganization
        | OrganizationAction::ReadMembers
        | OrganizationAction::ReadSpace
        | OrganizationAction::ReadFolder
        | OrganizationAction::CreateSpace => true,
        OrganizationAction::ManageSpace | OrganizationAction::ChangeSpaceRole => {
            matches!(context.space_role, Some(SpaceRole::Manager))
        }
        OrganizationAction::CreateFolder => matches!(
            context.space_role,
            Some(SpaceRole::Manager | SpaceRole::Contributor)
        ),
        OrganizationAction::ManageFolder | OrganizationAction::MoveFolder => {
            matches!(context.space_role, Some(SpaceRole::Manager))
                || (matches!(context.space_role, Some(SpaceRole::Contributor))
                    && context.owns_object)
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationRecord {
    pub scope: OrganizationScope,
    pub owner_id: UserId,
    pub name: OrganizationName,
    pub status: OrganizationStatus,
    pub settings: OrganizationSettings,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub tombstoned_at: Option<TimestampMillis>,
    pub retention_until: Option<TimestampMillis>,
    pub revision: OrganizationRevision,
    pub authority_version: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationMembershipRecord {
    pub scope: OrganizationScope,
    pub user_id: UserId,
    pub role: OrganizationRole,
    pub state: MembershipState,
    pub has_pro_seat: bool,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub revision: OrganizationRevision,
    pub authority_version: OrganizationRevision,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationInviteRecord {
    pub id: OrganizationInviteId,
    pub scope: OrganizationScope,
    pub invited_identifier_digest: VersionedSecretDigest,
    pub invited_by_user_id: UserId,
    pub accepted_by_user_id: Option<UserId>,
    pub role: OrganizationRole,
    pub state: InviteState,
    pub token_digest: SecretDigest,
    pub created_at: TimestampMillis,
    pub expires_at: TimestampMillis,
    pub resolved_at: Option<TimestampMillis>,
    pub revision: OrganizationRevision,
}

impl fmt::Debug for OrganizationInviteRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OrganizationInviteRecord")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("invited_by_user_id", &self.invited_by_user_id)
            .field("accepted_by_user_id", &self.accepted_by_user_id)
            .field("role", &self.role)
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("resolved_at", &self.resolved_at)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowedDomainRecord {
    pub scope: OrganizationScope,
    pub domain: AllowedDomain,
    pub verified_at: Option<TimestampMillis>,
    pub created_at: TimestampMillis,
    pub revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceRecord {
    pub id: SpaceId,
    pub scope: OrganizationScope,
    pub created_by_user_id: UserId,
    pub name: CollaborationName,
    pub is_primary: bool,
    pub is_public: bool,
    pub settings: OrganizationSettings,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceMembershipRecord {
    pub scope: OrganizationScope,
    pub space_id: SpaceId,
    pub user_id: UserId,
    pub role: SpaceRole,
    pub state: MembershipState,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderRecord {
    pub id: FolderId,
    pub scope: OrganizationScope,
    pub space_id: SpaceId,
    pub parent_id: Option<FolderId>,
    pub created_by_user_id: UserId,
    pub name: CollaborationName,
    pub is_public: bool,
    pub settings: OrganizationSettings,
    pub depth: u8,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub revision: OrganizationRevision,
    pub tree_revision: OrganizationRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveOrganizationSelection {
    pub user_id: UserId,
    pub default_organization_id: Option<OrganizationId>,
    pub active_organization_id: Option<OrganizationId>,
    pub revision: OrganizationRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstonePolicy {
    pub minimum_retention_ms: u64,
    pub maximum_recovery_ms: u64,
}

impl TombstonePolicy {
    pub fn new(
        minimum_retention_ms: u64,
        maximum_recovery_ms: u64,
    ) -> Result<Self, OrganizationContractError> {
        const MAXIMUM: u64 = 36500 * 24 * 60 * 60 * 1_000;
        if minimum_retention_ms == 0
            || maximum_recovery_ms < minimum_retention_ms
            || maximum_recovery_ms > MAXIMUM
        {
            return Err(OrganizationContractError::InvalidTombstonePolicy);
        }
        Ok(Self {
            minimum_retention_ms,
            maximum_recovery_ms,
        })
    }

    /// Derive the only recovery deadline accepted by a repository configured
    /// with this policy. Callers never supply a retention timestamp directly.
    pub fn recovery_deadline(
        self,
        tombstoned_at: TimestampMillis,
    ) -> Result<TimestampMillis, OrganizationContractError> {
        let duration = DurationMillis::new(self.maximum_recovery_ms)
            .map_err(|_| OrganizationContractError::InvalidTombstonePolicy)?;
        tombstoned_at
            .checked_add(duration)
            .map_err(|_| OrganizationContractError::InvalidTimeRange)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationGraphFindingKind {
    MissingOwnerMembership,
    MultipleActiveOwners,
    OwnerPointerMismatch,
    MembershipWithoutUser,
    ActiveSelectionWithoutMembership,
    SpaceMembershipWithoutOrganizationMembership,
    FolderWithoutSpace,
    FolderCrossesSpace,
    FolderCycle,
    FolderDepthMismatch,
    DeletedAncestor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationGraphFinding {
    pub kind: OrganizationGraphFindingKind,
    pub scope: OrganizationScope,
    pub subject_id: String,
    pub observed_revision: OrganizationRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationRepairActionKind {
    RestoreOwnerMembership,
    AlignOwnerPointer,
    SuspendOrphanMembership,
    ClearActiveSelection,
    SuspendSpaceMembership,
    ReparentFolderToRoot,
    RecomputeFolderDepth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationRepairStep {
    pub action: OrganizationRepairActionKind,
    pub subject_id: String,
    pub expected_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationRepairPlan {
    pub id: OrganizationRepairPlanId,
    pub scope: OrganizationScope,
    pub generated_by: UserId,
    pub generated_at: TimestampMillis,
    pub dry_run: bool,
    pub steps: Vec<OrganizationRepairStep>,
}

impl OrganizationRepairPlan {
    pub fn new_dry_run(
        scope: OrganizationScope,
        generated_by: UserId,
        generated_at: TimestampMillis,
        steps: Vec<OrganizationRepairStep>,
    ) -> Result<Self, OrganizationContractError> {
        if steps.len() > 10_000 {
            return Err(OrganizationContractError::InvalidSettings);
        }
        Ok(Self {
            id: OrganizationRepairPlanId::new(),
            scope,
            generated_by,
            generated_at,
            dry_run: true,
            steps,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationAuditDecision {
    pub id: OrganizationAuditId,
    pub operation_id: OrganizationOperationId,
    pub correlation_id: CorrelationId,
    pub scope: OrganizationScope,
    pub actor_id: UserId,
    pub action: OrganizationAction,
    pub allowed: bool,
    pub denial: Option<OrganizationDenialReason>,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationCommandEnvelope<T> {
    pub operation_id: OrganizationOperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub scope: OrganizationScope,
    pub actor_id: UserId,
    pub fence: OrganizationAuthorityFence,
    pub issued_at: TimestampMillis,
    pub command: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("timestamp")
    }

    fn context(
        role: OrganizationRole,
        action: OrganizationAction,
        space_role: Option<SpaceRole>,
        owns_object: bool,
    ) -> OrganizationPolicyContext {
        let organization_id = OrganizationId::new();
        let tenant_id = TenantId::parse(&organization_id.to_string()).expect("tenant");
        let actor_id = UserId::new();
        OrganizationPolicyContext {
            actor_tenant_id: tenant_id,
            resource_tenant_id: tenant_id,
            organization_id,
            actor_id,
            principal_state: PrincipalState::Active,
            organization_status: OrganizationStatus::Active,
            membership: Some(OrganizationPolicyMembership {
                tenant_id,
                organization_id,
                user_id: actor_id,
                role,
                state: MembershipState::Active,
            }),
            space_role,
            object_kind: action.object_kind(),
            object_state: CollaborationObjectState::Active,
            owns_object,
            invite_grant: false,
            authenticated_support: false,
        }
    }

    fn expected(
        role: OrganizationRole,
        action: OrganizationAction,
        space_role: Option<SpaceRole>,
        owns_object: bool,
    ) -> bool {
        if matches!(
            action,
            OrganizationAction::AuditGraph | OrganizationAction::PlanRepair
        ) {
            return false;
        }
        if action == OrganizationAction::CreateOrganization {
            return false;
        }
        if action == OrganizationAction::AcceptInvite {
            return false;
        }
        match role {
            OrganizationRole::Owner => action != OrganizationAction::RecoverOrganization,
            OrganizationRole::Admin => !matches!(
                action,
                OrganizationAction::TransferOwnership
                    | OrganizationAction::ManageBilling
                    | OrganizationAction::TombstoneOrganization
                    | OrganizationAction::RecoverOrganization
            ),
            OrganizationRole::Member => match action {
                OrganizationAction::ReadOrganization
                | OrganizationAction::ReadMembers
                | OrganizationAction::ReadSpace
                | OrganizationAction::ReadFolder
                | OrganizationAction::CreateSpace => true,
                OrganizationAction::ManageSpace | OrganizationAction::ChangeSpaceRole => {
                    space_role == Some(SpaceRole::Manager)
                }
                OrganizationAction::CreateFolder => matches!(
                    space_role,
                    Some(SpaceRole::Manager | SpaceRole::Contributor)
                ),
                OrganizationAction::ManageFolder | OrganizationAction::MoveFolder => {
                    space_role == Some(SpaceRole::Manager)
                        || (space_role == Some(SpaceRole::Contributor) && owns_object)
                }
                _ => false,
            },
            OrganizationRole::Viewer => matches!(
                action,
                OrganizationAction::ReadOrganization
                    | OrganizationAction::ReadMembers
                    | OrganizationAction::ReadSpace
                    | OrganizationAction::ReadFolder
            ),
        }
    }

    #[test]
    fn policy_table_covers_every_role_action_space_role_and_ownership_combination() {
        let roles = [
            OrganizationRole::Owner,
            OrganizationRole::Admin,
            OrganizationRole::Member,
            OrganizationRole::Viewer,
        ];
        let space_roles = [
            None,
            Some(SpaceRole::Manager),
            Some(SpaceRole::Contributor),
            Some(SpaceRole::Viewer),
        ];
        let mut evaluated = 0;
        for role in roles {
            for action in OrganizationAction::ALL {
                for space_role in space_roles {
                    for owns_object in [false, true] {
                        let decision = AuthorizationPolicy::evaluate_organization(
                            context(role, action, space_role, owns_object),
                            action,
                        );
                        assert_eq!(
                            decision == OrganizationAuthorizationDecision::Allow,
                            expected(role, action, space_role, owns_object),
                            "{role:?}/{action:?}/{space_role:?}/{owns_object}"
                        );
                        evaluated += 1;
                    }
                }
            }
        }
        assert_eq!(evaluated, 4 * 26 * 4 * 2);
    }

    #[test]
    fn organization_creation_requires_an_active_nonmember_principal() {
        let action = OrganizationAction::CreateOrganization;
        let mut create = context(OrganizationRole::Owner, action, None, false);
        create.membership = None;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(create, action),
            OrganizationAuthorizationDecision::Allow
        );

        let existing_member = context(OrganizationRole::Owner, action, None, false);
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(existing_member, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        );

        create.principal_state = PrincipalState::Suspended;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(create, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::InactiveAuthority)
        );
    }

    #[test]
    fn unknown_and_cross_tenant_resources_are_indistinguishable() {
        let action = OrganizationAction::ReadOrganization;
        let mut absent = context(OrganizationRole::Owner, action, None, false);
        absent.membership = None;
        let mut cross_tenant = context(OrganizationRole::Owner, action, None, false);
        cross_tenant.resource_tenant_id = TenantId::new();
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(absent, action),
            AuthorizationPolicy::evaluate_organization(cross_tenant, action)
        );
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(absent, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        );
    }

    #[test]
    fn inactive_and_exceptional_states_fail_closed() {
        let action = OrganizationAction::UpdateOrganization;
        let mut suspended = context(OrganizationRole::Owner, action, None, false);
        suspended.principal_state = PrincipalState::Suspended;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(suspended, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::InactiveAuthority)
        );
        let mut tombstoned = context(OrganizationRole::Owner, action, None, false);
        tombstoned.organization_status = OrganizationStatus::Tombstoned;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(tombstoned, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::StateDenied)
        );
        let mut wrong_object = context(OrganizationRole::Owner, action, None, false);
        wrong_object.object_kind = OrganizationObjectKind::Folder;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(wrong_object, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        );

        let recover = OrganizationAction::RecoverOrganization;
        let mut tombstoned_owner = context(OrganizationRole::Owner, recover, None, false);
        tombstoned_owner.organization_status = OrganizationStatus::Tombstoned;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(tombstoned_owner, recover),
            OrganizationAuthorizationDecision::Allow
        );
    }

    #[test]
    fn explicit_support_authority_is_read_only_and_tenant_bound() {
        let action = OrganizationAction::PlanRepair;
        let mut support = context(OrganizationRole::Viewer, action, None, false);
        support.membership = None;
        support.authenticated_support = true;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(support, action),
            OrganizationAuthorizationDecision::Allow
        );
        support.resource_tenant_id = TenantId::new();
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(support, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        );
        let owner_without_support = context(OrganizationRole::Owner, action, None, false);
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(owner_without_support, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        );
    }

    #[test]
    fn invite_grants_require_no_existing_membership_and_active_state() {
        let action = OrganizationAction::AcceptInvite;
        let mut invite = context(OrganizationRole::Viewer, action, None, false);
        invite.membership = None;
        invite.invite_grant = true;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(invite, action),
            OrganizationAuthorizationDecision::Allow
        );

        let mut existing_member = invite;
        existing_member.membership = Some(OrganizationPolicyMembership {
            tenant_id: invite.actor_tenant_id,
            organization_id: invite.organization_id,
            user_id: invite.actor_id,
            role: OrganizationRole::Viewer,
            state: MembershipState::Active,
        });
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(existing_member, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::AccessDenied)
        );

        let mut tombstoned = invite;
        tombstoned.organization_status = OrganizationStatus::Tombstoned;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(tombstoned, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::StateDenied)
        );

        let mut object_tombstoned = invite;
        object_tombstoned.object_state = CollaborationObjectState::Tombstoned;
        assert_eq!(
            AuthorizationPolicy::evaluate_organization(object_tombstoned, action),
            OrganizationAuthorizationDecision::Deny(OrganizationDenialReason::StateDenied)
        );
    }

    #[test]
    fn bounded_values_and_sensitive_debug_are_safe() {
        assert!(OrganizationName::parse(" Acme").is_err());
        assert!(OrganizationName::parse("a".repeat(161)).is_err());
        assert!(OrganizationSettings::parse("[]").is_err());
        assert!(OrganizationSettings::parse("{\"nested\":{\"enabled\":true}}").is_ok());
        assert!(AllowedDomain::parse("127.0.0.1").is_err());
        assert!(AllowedDomain::parse("Example.COM").is_ok());
        let invite_material =
            OrganizationInviteSecret::parse(vec![b'a'; 32]).expect("invite material");
        assert_eq!(
            format!("{invite_material:?}"),
            "OrganizationInviteSecret([redacted])"
        );
        let digest = SecretDigest::parse_sha256("a".repeat(64)).expect("digest");
        let record = OrganizationInviteRecord {
            id: OrganizationInviteId::new(),
            scope: OrganizationScope::from_organization(OrganizationId::new()).expect("scope"),
            invited_identifier_digest: VersionedSecretDigest::new(
                crate::HashKeyVersion::new(1).expect("key version"),
                digest.clone(),
            ),
            invited_by_user_id: UserId::new(),
            accepted_by_user_id: None,
            role: OrganizationRole::Member,
            state: InviteState::Pending,
            token_digest: digest,
            created_at: timestamp(1),
            expires_at: timestamp(2),
            resolved_at: None,
            revision: OrganizationRevision::INITIAL,
        };
        assert!(!format!("{record:?}").contains(&"a".repeat(64)));
    }

    #[test]
    fn revision_scope_and_tombstone_contracts_are_bounded() {
        let organization_id = OrganizationId::new();
        assert!(OrganizationScope::from_organization(organization_id).is_ok());
        assert!(OrganizationScope::new(TenantId::new(), organization_id).is_err());
        assert!(OrganizationRevision::new(crate::MAX_WIRE_INTEGER).is_ok());
        assert!(OrganizationRevision::new(crate::MAX_WIRE_INTEGER + 1).is_err());
        assert!(TombstonePolicy::new(1, 1).is_ok());
        assert!(TombstonePolicy::new(0, 1).is_err());
        let policy = TombstonePolicy::new(10, 20).expect("policy");
        assert_eq!(
            policy.recovery_deadline(timestamp(100)).expect("deadline"),
            timestamp(120)
        );
    }
}
