//! Runtime-neutral organization workflows.

use std::fmt;

use frame_domain::{
    AllowedDomainRecord, AuthorizationPolicy, OrganizationAction,
    OrganizationAuthorizationDecision, OrganizationDenialReason, OrganizationPolicyContext,
    OrganizationRepairPlan, SecretDigest,
};
use frame_ports::{
    AcceptOrganizationInviteCommand, ChangeOrganizationMemberCommand, ChangeSpaceRoleCommand,
    CreateFolderCommand, CreateOrganizationCommand, CreateSpaceCommand,
    IssueOrganizationInviteCommand, MoveFolderCommand, OrganizationCollectionRequest,
    OrganizationGraphAudit, OrganizationGraphAuditRequest, OrganizationInviteSummary,
    OrganizationMutationReceipt, OrganizationPage, OrganizationPortError, OrganizationRepository,
    OrganizationSpaceCollectionRequest, RecoverOrganizationCommand,
    RevokeOrganizationInviteCommand, SetActiveOrganizationCommand, TombstoneOrganizationCommand,
    TransferOrganizationOwnershipCommand, UpdateFolderCommand, UpdateOrganizationSettingsCommand,
    UpdateSpaceCommand, UpsertAllowedDomainCommand,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Error, PartialEq, Eq)]
pub enum OrganizationApplicationError {
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
    #[error("the organization service is temporarily unavailable")]
    Unavailable,
    #[error("the organization service encountered an internal failure")]
    Internal,
}

impl fmt::Debug for OrganizationApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AccessDenied => "AccessDenied",
            Self::StaleAuthority => "StaleAuthority",
            Self::Conflict => "Conflict",
            Self::Invalid => "Invalid",
            Self::RetentionLocked => "RetentionLocked",
            Self::Unavailable => "Unavailable",
            Self::Internal => "Internal",
        })
    }
}

impl From<OrganizationPortError> for OrganizationApplicationError {
    fn from(value: OrganizationPortError) -> Self {
        match value {
            OrganizationPortError::AccessDenied => Self::AccessDenied,
            OrganizationPortError::StaleAuthority => Self::StaleAuthority,
            OrganizationPortError::Conflict => Self::Conflict,
            OrganizationPortError::Invalid => Self::Invalid,
            OrganizationPortError::RetentionLocked => Self::RetentionLocked,
            OrganizationPortError::Unavailable => Self::Unavailable,
            OrganizationPortError::Corrupt => Self::Internal,
        }
    }
}

impl From<OrganizationDenialReason> for OrganizationApplicationError {
    fn from(value: OrganizationDenialReason) -> Self {
        match value {
            OrganizationDenialReason::AccessDenied
            | OrganizationDenialReason::InactiveAuthority
            | OrganizationDenialReason::StateDenied => Self::AccessDenied,
            OrganizationDenialReason::StaleAuthority => Self::StaleAuthority,
            OrganizationDenialReason::RetentionLocked => Self::RetentionLocked,
        }
    }
}

pub struct OrganizationService<'repository, Repository> {
    repository: &'repository Repository,
}

impl<'repository, Repository> OrganizationService<'repository, Repository>
where
    Repository: OrganizationRepository,
{
    #[must_use]
    pub const fn new(repository: &'repository Repository) -> Self {
        Self { repository }
    }

    pub async fn create_organization(
        &self,
        policy: OrganizationPolicyContext,
        command: CreateOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::CreateOrganization)?;
        self.repository
            .create_organization(command)
            .await
            .map_err(Into::into)
    }

    pub async fn list_members(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationCollectionRequest,
    ) -> Result<
        OrganizationPage<frame_domain::OrganizationMembershipRecord>,
        OrganizationApplicationError,
    > {
        self.require(policy, OrganizationAction::ReadMembers)?;
        self.repository
            .list_members(request)
            .await
            .map_err(Into::into)
    }

    pub async fn list_invites(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<OrganizationInviteSummary>, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::IssueInvite)?;
        self.repository
            .list_invites(request)
            .await
            .map_err(Into::into)
    }

    pub async fn list_allowed_domains(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<AllowedDomainRecord>, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ManageAllowedDomain)?;
        self.repository
            .list_allowed_domains(request)
            .await
            .map_err(Into::into)
    }

    pub async fn list_spaces(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<frame_domain::SpaceRecord>, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ReadSpace)?;
        self.repository
            .list_spaces(request)
            .await
            .map_err(Into::into)
    }

    pub async fn list_space_members(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationSpaceCollectionRequest,
    ) -> Result<OrganizationPage<frame_domain::SpaceMembershipRecord>, OrganizationApplicationError>
    {
        self.require(policy, OrganizationAction::ReadSpace)?;
        self.repository
            .list_space_members(request)
            .await
            .map_err(Into::into)
    }

    pub async fn list_folders(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationSpaceCollectionRequest,
    ) -> Result<OrganizationPage<frame_domain::FolderRecord>, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ReadFolder)?;
        self.repository
            .list_folders(request)
            .await
            .map_err(Into::into)
    }

    pub async fn set_active_organization(
        &self,
        policy: OrganizationPolicyContext,
        command: SetActiveOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ReadOrganization)?;
        self.repository
            .set_active_organization(command)
            .await
            .map_err(Into::into)
    }

    pub async fn issue_invite(
        &self,
        policy: OrganizationPolicyContext,
        command: IssueOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::IssueInvite)?;
        self.repository
            .issue_invite(command)
            .await
            .map_err(Into::into)
    }

    pub async fn revoke_invite(
        &self,
        policy: OrganizationPolicyContext,
        command: RevokeOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::RevokeInvite)?;
        self.repository
            .revoke_invite(command)
            .await
            .map_err(Into::into)
    }

    pub async fn accept_invite(
        &self,
        policy: OrganizationPolicyContext,
        command: AcceptOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::AcceptInvite)?;
        self.repository
            .accept_invite(command)
            .await
            .map_err(Into::into)
    }

    pub async fn transfer_ownership(
        &self,
        policy: OrganizationPolicyContext,
        command: TransferOrganizationOwnershipCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::TransferOwnership)?;
        self.repository
            .transfer_ownership(command)
            .await
            .map_err(Into::into)
    }

    pub async fn change_member(
        &self,
        policy: OrganizationPolicyContext,
        command: ChangeOrganizationMemberCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        let action = if command.state == frame_domain::MembershipState::Removed {
            OrganizationAction::RemoveMember
        } else {
            OrganizationAction::ChangeMemberRole
        };
        self.require(policy, action)?;
        self.repository
            .change_member(command)
            .await
            .map_err(Into::into)
    }

    pub async fn upsert_allowed_domain(
        &self,
        policy: OrganizationPolicyContext,
        command: UpsertAllowedDomainCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ManageAllowedDomain)?;
        self.repository
            .upsert_allowed_domain(command)
            .await
            .map_err(Into::into)
    }

    pub async fn update_settings(
        &self,
        policy: OrganizationPolicyContext,
        command: UpdateOrganizationSettingsCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ManageSettings)?;
        self.repository
            .update_settings(command)
            .await
            .map_err(Into::into)
    }

    pub async fn create_space(
        &self,
        policy: OrganizationPolicyContext,
        command: CreateSpaceCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::CreateSpace)?;
        self.repository
            .create_space(command)
            .await
            .map_err(Into::into)
    }

    pub async fn update_space(
        &self,
        policy: OrganizationPolicyContext,
        command: UpdateSpaceCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ManageSpace)?;
        self.repository
            .update_space(command)
            .await
            .map_err(Into::into)
    }

    pub async fn change_space_role(
        &self,
        policy: OrganizationPolicyContext,
        command: ChangeSpaceRoleCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ChangeSpaceRole)?;
        self.repository
            .change_space_role(command)
            .await
            .map_err(Into::into)
    }

    pub async fn create_folder(
        &self,
        policy: OrganizationPolicyContext,
        command: CreateFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::CreateFolder)?;
        self.repository
            .create_folder(command)
            .await
            .map_err(Into::into)
    }

    pub async fn update_folder(
        &self,
        policy: OrganizationPolicyContext,
        command: UpdateFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::ManageFolder)?;
        self.repository
            .update_folder(command)
            .await
            .map_err(Into::into)
    }

    pub async fn move_folder(
        &self,
        policy: OrganizationPolicyContext,
        command: MoveFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::MoveFolder)?;
        self.repository
            .move_folder(command)
            .await
            .map_err(Into::into)
    }

    pub async fn tombstone_organization(
        &self,
        policy: OrganizationPolicyContext,
        command: TombstoneOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::TombstoneOrganization)?;
        self.repository
            .tombstone_organization(command)
            .await
            .map_err(Into::into)
    }

    pub async fn recover_organization(
        &self,
        policy: OrganizationPolicyContext,
        command: RecoverOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::RecoverOrganization)?;
        self.repository
            .recover_organization(command)
            .await
            .map_err(Into::into)
    }

    pub async fn audit_graph(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationGraphAuditRequest,
    ) -> Result<OrganizationGraphAudit, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::AuditGraph)?;
        if request.maximum_findings == 0 || request.maximum_findings > 10_000 {
            return Err(OrganizationApplicationError::Invalid);
        }
        self.repository
            .audit_graph(request)
            .await
            .map_err(Into::into)
    }

    pub async fn plan_repair(
        &self,
        policy: OrganizationPolicyContext,
        request: OrganizationGraphAuditRequest,
    ) -> Result<OrganizationRepairPlan, OrganizationApplicationError> {
        self.require(policy, OrganizationAction::PlanRepair)?;
        if request.maximum_findings == 0 || request.maximum_findings > 10_000 {
            return Err(OrganizationApplicationError::Invalid);
        }
        let plan = self.repository.plan_repair(request).await?;
        if !plan.dry_run {
            return Err(OrganizationApplicationError::Internal);
        }
        Ok(plan)
    }

    fn require(
        &self,
        policy: OrganizationPolicyContext,
        action: OrganizationAction,
    ) -> Result<(), OrganizationApplicationError> {
        match AuthorizationPolicy::evaluate_organization(policy, action) {
            OrganizationAuthorizationDecision::Allow => Ok(()),
            OrganizationAuthorizationDecision::Deny(reason) => Err(reason.into()),
        }
    }
}

/// Hash a bearer invite at the service boundary; raw tokens never enter repository DTOs.
pub fn digest_organization_invite(
    secret: &frame_domain::OrganizationInviteSecret,
) -> Result<SecretDigest, OrganizationApplicationError> {
    let digest = Sha256::digest(secret.expose_for_hashing());
    SecretDigest::parse_sha256(format!("{digest:x}"))
        .map_err(|_| OrganizationApplicationError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_hash_is_stable_and_redacted() {
        let invite =
            frame_domain::OrganizationInviteSecret::parse(vec![b'a'; 32]).expect("invite secret");
        let digest = digest_organization_invite(&invite).expect("digest");
        assert_eq!(digest.expose_for_verification().len(), 64);
        assert!(!format!("{digest:?}").contains(&"a".repeat(32)));
    }

    #[test]
    fn adapter_details_never_cross_the_application_boundary() {
        let error = OrganizationApplicationError::from(OrganizationPortError::Corrupt);
        assert_eq!(format!("{error:?}"), "Internal");
        assert_eq!(error, OrganizationApplicationError::Internal);
    }
}
