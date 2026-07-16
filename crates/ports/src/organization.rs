//! Provider-neutral organization repository capabilities.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    ActiveOrganizationSelection, AllowedDomain, AllowedDomainRecord, CollaborationName, FolderId,
    FolderRecord, IdempotencyKey, InviteState, MembershipState, OrganizationAction,
    OrganizationAuditDecision, OrganizationAuthorityFence, OrganizationGraphFinding,
    OrganizationId, OrganizationInviteId, OrganizationInviteRecord, OrganizationName,
    OrganizationOperationId, OrganizationRecord, OrganizationRepairPlan, OrganizationRevision,
    OrganizationRole, OrganizationScope, OrganizationSettings, SecretDigest, SpaceId,
    SpaceMembershipRecord, SpaceRecord, SpaceRole, TimestampMillis, UserId,
};
use thiserror::Error;

/// Stable repository failures. Missing and cross-tenant subjects intentionally share a variant.
#[derive(Clone, Copy, Error, PartialEq, Eq)]
pub enum OrganizationPortError {
    #[error("the operation is not permitted")]
    AccessDenied,
    #[error("the authority fence is stale")]
    StaleAuthority,
    #[error("the request conflicts with current state")]
    Conflict,
    #[error("the request is invalid")]
    Invalid,
    #[error("the retention policy prevents this operation")]
    RetentionLocked,
    #[error("the organization repository is unavailable")]
    Unavailable,
    #[error("the organization repository returned corrupt state")]
    Corrupt,
}

impl fmt::Debug for OrganizationPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AccessDenied => "AccessDenied",
            Self::StaleAuthority => "StaleAuthority",
            Self::Conflict => "Conflict",
            Self::Invalid => "Invalid",
            Self::RetentionLocked => "RetentionLocked",
            Self::Unavailable => "Unavailable",
            Self::Corrupt => "Corrupt",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizationWriteAuthority {
    pub operation_id: OrganizationOperationId,
    pub scope: OrganizationScope,
    pub actor_id: UserId,
    pub fence: OrganizationAuthorityFence,
    pub occurred_at: TimestampMillis,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OrganizationMutationContext {
    pub authority: OrganizationWriteAuthority,
    pub idempotency_key: IdempotencyKey,
    pub action: OrganizationAction,
}

impl fmt::Debug for OrganizationMutationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OrganizationMutationContext")
            .field("authority", &self.authority)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizationMutationResult {
    Created,
    Applied,
    Accepted,
    Revoked,
    Tombstoned,
    Recovered,
    Unchanged,
}

impl OrganizationMutationResult {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Applied => "applied",
            Self::Accepted => "accepted",
            Self::Revoked => "revoked",
            Self::Tombstoned => "tombstoned",
            Self::Recovered => "recovered",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationMutationReceipt {
    pub operation_id: OrganizationOperationId,
    pub result: OrganizationMutationResult,
    pub subject_id: String,
    pub committed_at: TimestampMillis,
    pub resulting_revision: OrganizationRevision,
    pub authority_version: OrganizationRevision,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizationReadRequest {
    pub scope: OrganizationScope,
    pub actor_id: UserId,
    pub identity_revision: OrganizationRevision,
    pub session_version: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationSnapshot {
    pub organization: OrganizationRecord,
    pub actor_membership: frame_domain::OrganizationMembershipRecord,
    pub active_selection: Option<ActiveOrganizationSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationCollectionRequest {
    pub authority: OrganizationReadRequest,
    /// Exclusive opaque UUID cursor. `None` starts at the first item.
    pub after_id: Option<String>,
    pub maximum_items: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationSpaceCollectionRequest {
    pub collection: OrganizationCollectionRequest,
    pub space_id: SpaceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// Safe invite projection: recipient and token digests never cross a read boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationInviteSummary {
    pub id: OrganizationInviteId,
    pub scope: OrganizationScope,
    pub invited_by_user_id: UserId,
    pub accepted_by_user_id: Option<UserId>,
    pub role: OrganizationRole,
    pub state: InviteState,
    pub created_at: TimestampMillis,
    pub expires_at: TimestampMillis,
    pub resolved_at: Option<TimestampMillis>,
    pub revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOrganizationCommand {
    pub context: OrganizationMutationContext,
    pub organization: OrganizationRecord,
    pub owner_membership: frame_domain::OrganizationMembershipRecord,
    pub primary_space: SpaceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetActiveOrganizationCommand {
    pub context: OrganizationMutationContext,
    pub default_organization_id: Option<OrganizationId>,
    pub active_organization_id: Option<OrganizationId>,
    pub expected_selection_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueOrganizationInviteCommand {
    pub context: OrganizationMutationContext,
    pub invite: OrganizationInviteRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeOrganizationInviteCommand {
    pub context: OrganizationMutationContext,
    pub invite_id: OrganizationInviteId,
    pub expected_invite_revision: OrganizationRevision,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AcceptOrganizationInviteCommand {
    pub context: OrganizationMutationContext,
    pub invite_id: OrganizationInviteId,
    pub presented_token_digest: SecretDigest,
    pub expected_invite_revision: OrganizationRevision,
}

impl fmt::Debug for AcceptOrganizationInviteCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcceptOrganizationInviteCommand")
            .field("context", &self.context)
            .field("invite_id", &self.invite_id)
            .field("expected_invite_revision", &self.expected_invite_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferOrganizationOwnershipCommand {
    pub context: OrganizationMutationContext,
    pub new_owner_id: UserId,
    pub expected_new_owner_membership_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeOrganizationMemberCommand {
    pub context: OrganizationMutationContext,
    pub subject_user_id: UserId,
    pub role: OrganizationRole,
    pub state: MembershipState,
    pub has_pro_seat: bool,
    pub expected_subject_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertAllowedDomainCommand {
    pub context: OrganizationMutationContext,
    pub domain: AllowedDomain,
    pub verified_at: Option<TimestampMillis>,
    pub expected_revision: Option<OrganizationRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOrganizationSettingsCommand {
    pub context: OrganizationMutationContext,
    pub name: OrganizationName,
    pub settings: OrganizationSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSpaceCommand {
    pub context: OrganizationMutationContext,
    pub space: SpaceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSpaceCommand {
    pub context: OrganizationMutationContext,
    pub space_id: SpaceId,
    pub name: CollaborationName,
    pub is_public: bool,
    pub settings: OrganizationSettings,
    pub expected_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSpaceRoleCommand {
    pub context: OrganizationMutationContext,
    pub space_id: SpaceId,
    pub subject_user_id: UserId,
    pub role: SpaceRole,
    pub state: MembershipState,
    pub expected_space_revision: OrganizationRevision,
    pub expected_revision: Option<OrganizationRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateFolderCommand {
    pub context: OrganizationMutationContext,
    pub folder: FolderRecord,
    pub expected_parent_revision: Option<OrganizationRevision>,
    pub expected_space_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveFolderCommand {
    pub context: OrganizationMutationContext,
    pub folder_id: FolderId,
    pub space_id: SpaceId,
    pub new_parent_id: Option<FolderId>,
    pub expected_folder_revision: OrganizationRevision,
    pub expected_space_revision: OrganizationRevision,
    pub expected_parent_revision: Option<OrganizationRevision>,
    pub expected_tree_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFolderCommand {
    pub context: OrganizationMutationContext,
    pub folder_id: FolderId,
    pub space_id: SpaceId,
    pub name: CollaborationName,
    pub is_public: bool,
    pub settings: OrganizationSettings,
    pub expected_folder_revision: OrganizationRevision,
    pub expected_space_revision: OrganizationRevision,
    pub expected_tree_revision: OrganizationRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TombstoneOrganizationCommand {
    pub context: OrganizationMutationContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverOrganizationCommand {
    pub context: OrganizationMutationContext,
    pub expected_tombstoned_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationGraphAuditRequest {
    pub scope: OrganizationScope,
    pub support_actor_id: UserId,
    pub support_ticket_digest: SecretDigest,
    pub identity_revision: OrganizationRevision,
    pub session_version: OrganizationRevision,
    pub occurred_at: TimestampMillis,
    pub maximum_findings: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationGraphAudit {
    pub findings: Vec<OrganizationGraphFinding>,
    pub generated_at: TimestampMillis,
    pub truncated: bool,
}

/// Mutation methods must atomically assert active user, identity/session, organization,
/// membership, role, and revision fences; commit one allow/deny audit; and persist a stable
/// idempotency receipt. Preflight reads are never mutation authority.
#[async_trait]
pub trait OrganizationRepository: Send + Sync {
    async fn snapshot(
        &self,
        request: OrganizationReadRequest,
    ) -> Result<OrganizationSnapshot, OrganizationPortError>;

    async fn list_members(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<frame_domain::OrganizationMembershipRecord>, OrganizationPortError>;

    async fn list_invites(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<OrganizationInviteSummary>, OrganizationPortError>;

    async fn list_allowed_domains(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<AllowedDomainRecord>, OrganizationPortError>;

    async fn list_spaces(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<SpaceRecord>, OrganizationPortError>;

    async fn list_space_members(
        &self,
        request: OrganizationSpaceCollectionRequest,
    ) -> Result<OrganizationPage<SpaceMembershipRecord>, OrganizationPortError>;

    async fn list_folders(
        &self,
        request: OrganizationSpaceCollectionRequest,
    ) -> Result<OrganizationPage<FolderRecord>, OrganizationPortError>;

    async fn create_organization(
        &self,
        command: CreateOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn set_active_organization(
        &self,
        command: SetActiveOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn issue_invite(
        &self,
        command: IssueOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn revoke_invite(
        &self,
        command: RevokeOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn accept_invite(
        &self,
        command: AcceptOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn transfer_ownership(
        &self,
        command: TransferOrganizationOwnershipCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn change_member(
        &self,
        command: ChangeOrganizationMemberCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn upsert_allowed_domain(
        &self,
        command: UpsertAllowedDomainCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn update_settings(
        &self,
        command: UpdateOrganizationSettingsCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn create_space(
        &self,
        command: CreateSpaceCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn update_space(
        &self,
        command: UpdateSpaceCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn change_space_role(
        &self,
        command: ChangeSpaceRoleCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn create_folder(
        &self,
        command: CreateFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn update_folder(
        &self,
        command: UpdateFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn move_folder(
        &self,
        command: MoveFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn tombstone_organization(
        &self,
        command: TombstoneOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn recover_organization(
        &self,
        command: RecoverOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError>;

    async fn audit_graph(
        &self,
        request: OrganizationGraphAuditRequest,
    ) -> Result<OrganizationGraphAudit, OrganizationPortError>;

    async fn plan_repair(
        &self,
        request: OrganizationGraphAuditRequest,
    ) -> Result<OrganizationRepairPlan, OrganizationPortError>;

    async fn audit_decision(
        &self,
        decision: OrganizationAuditDecision,
    ) -> Result<(), OrganizationPortError>;
}

// Compile-time checks keep all repository DTOs Send + Sync for native and Worker adapters.
const _: fn() = || {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OrganizationMutationContext>();
    send_sync::<CreateFolderCommand>();
    send_sync::<AcceptOrganizationInviteCommand>();
    send_sync::<OrganizationGraphAudit>();
    send_sync::<AllowedDomainRecord>();
    send_sync::<SpaceMembershipRecord>();
    send_sync::<CollaborationName>();
    send_sync::<InviteState>();
};
