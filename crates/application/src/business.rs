//! Application orchestration for collaboration and business metadata.

#[cfg(test)]
use frame_domain::BusinessDataClass;
use frame_domain::{
    BusinessAction, BusinessActor, BusinessAuthorizationDecision, BusinessAuthorizationPolicy,
    BusinessContractError, BusinessPolicyContext, ChecksumSha256, CreditTransactionKind,
    DeletionMode, DocumentKind, RetentionDecision, business_payload_checksum,
    business_semantic_fingerprint, deletion_compensation_reference, retention_decision,
};
use frame_ports::{
    AdvanceImportCommand, AdvanceOutboxCommand, AdvanceUploadCommand,
    AppendCreditTransactionCommand, AppendUsageCommand, BusinessMutationContext,
    BusinessMutationReceipt, BusinessPortError, BusinessReadRequest, BusinessRepository,
    DataHandlingCommand, DeleteCommentCommand, EnqueueNotificationCommand,
    MarkNotificationReadCommand, PlaceLegalHoldCommand, PutCommentCommand,
    PutDailyStorageSnapshotCommand, PutDerivativeJobCommand, PutDeveloperApiKeyCommand,
    PutDeveloperAppCommand, PutDeveloperDomainCommand, PutDeveloperVideoCommand, PutEditCommand,
    PutShareCommand, PutStorageIntegrationCommand, PutStorageObjectCommand, PutVideoCommand,
    ReleaseLegalHoldCommand,
};
use thiserror::Error;

#[derive(Clone, Copy, Error, PartialEq, Eq)]
pub enum BusinessServiceError {
    #[error("the operation is not permitted")]
    AccessDenied,
    #[error("the authority fence is stale")]
    StaleAuthority,
    #[error("the request conflicts with current state")]
    Conflict,
    #[error("the request is invalid")]
    Invalid,
    #[error("retention or a legal hold prevents the operation")]
    RetentionLocked,
    #[error("the service is temporarily unavailable")]
    Unavailable,
    #[error("the repository returned corrupt state")]
    Corrupt,
}

impl std::fmt::Debug for BusinessServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl From<BusinessPortError> for BusinessServiceError {
    fn from(error: BusinessPortError) -> Self {
        match error {
            BusinessPortError::AccessDenied => Self::AccessDenied,
            BusinessPortError::StaleAuthority => Self::StaleAuthority,
            BusinessPortError::Conflict => Self::Conflict,
            BusinessPortError::Invalid => Self::Invalid,
            BusinessPortError::RetentionLocked => Self::RetentionLocked,
            BusinessPortError::Unavailable => Self::Unavailable,
            BusinessPortError::Corrupt => Self::Corrupt,
        }
    }
}

impl From<BusinessContractError> for BusinessServiceError {
    fn from(error: BusinessContractError) -> Self {
        match error {
            BusinessContractError::RetentionLocked => Self::RetentionLocked,
            BusinessContractError::ConflictingReplay => Self::Conflict,
            BusinessContractError::InvalidIdentifier(_)
            | BusinessContractError::ScopeMismatch
            | BusinessContractError::InvalidDocument
            | BusinessContractError::ReadOnlyDocument
            | BusinessContractError::InvalidSequence
            | BusinessContractError::InvalidTransition
            | BusinessContractError::InvalidRecord
            | BusinessContractError::AccountingInvariant => Self::Invalid,
        }
    }
}

pub struct BusinessDataService<R> {
    repository: R,
}

impl<R> BusinessDataService<R>
where
    R: BusinessRepository,
{
    #[must_use]
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    pub fn into_inner(self) -> R {
        self.repository
    }

    pub async fn put_video(
        &self,
        command: PutVideoCommand,
        policy: BusinessPolicyContext,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        validate_policy(policy, &command.context, BusinessAction::ManageVideo)?;
        require_action(&command.context, BusinessAction::ManageVideo)?;
        if command.video.scope != command.context.scope
            || command.video.metadata.kind() != DocumentKind::VideoMetadata
        {
            return Err(BusinessServiceError::Invalid);
        }
        command.video.metadata.require_writable()?;
        let payload = business_payload_checksum(&command.video)?;
        validate_fingerprint(
            &command.context,
            &command.video.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository.put_video(command).await.map_err(Into::into)
    }

    pub async fn put_edit(
        &self,
        command: PutEditCommand,
        policy: BusinessPolicyContext,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        validate_policy(policy, &command.context, BusinessAction::ManageEdit)?;
        require_action(&command.context, BusinessAction::ManageEdit)?;
        if command.edit.scope != command.context.scope
            || command.edit.document.kind() != DocumentKind::VideoEdit
        {
            return Err(BusinessServiceError::Invalid);
        }
        command.edit.document.require_writable()?;
        let payload = business_payload_checksum(&command.edit)?;
        validate_fingerprint(
            &command.context,
            &command.edit.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository.put_edit(command).await.map_err(Into::into)
    }

    pub async fn put_share(
        &self,
        command: PutShareCommand,
        policy: BusinessPolicyContext,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        validate_policy(policy, &command.context, BusinessAction::ManageShare)?;
        require_action(&command.context, BusinessAction::ManageShare)?;
        if command.share.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.share.validate()?;
        let payload = business_payload_checksum(&command.share)?;
        validate_fingerprint(
            &command.context,
            &command.share.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository.put_share(command).await.map_err(Into::into)
    }

    pub async fn put_comment(
        &self,
        command: PutCommentCommand,
        policy: BusinessPolicyContext,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        validate_policy(policy, &command.context, BusinessAction::CreateComment)?;
        require_action(&command.context, BusinessAction::CreateComment)?;
        if command.comment.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        let author_matches = match (&command.context.principal, &command.comment.author) {
            (
                frame_ports::BusinessPrincipal::Authenticated(principal),
                frame_domain::CommentAuthor::User(author),
            ) => principal == author,
            (
                frame_ports::BusinessPrincipal::Anonymous(principal),
                frame_domain::CommentAuthor::Anonymous(author),
            ) => principal.as_str() == author.expose_for_verification(),
            _ => false,
        };
        if !author_matches {
            return Err(BusinessServiceError::AccessDenied);
        }
        command.comment.validate()?;
        let payload = business_payload_checksum(&command.comment)?;
        validate_fingerprint(
            &command.context,
            &command.comment.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_comment(command)
            .await
            .map_err(Into::into)
    }

    pub async fn list_comments(
        &self,
        request: BusinessReadRequest,
        policy: BusinessPolicyContext,
    ) -> Result<Vec<frame_domain::BusinessCommentRecord>, BusinessServiceError> {
        validate_read_policy(policy, &request, BusinessAction::ReadComment)?;
        self.repository
            .list_comments(request)
            .await
            .map_err(Into::into)
    }

    pub async fn delete_comment(
        &self,
        command: DeleteCommentCommand,
        policy: BusinessPolicyContext,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        validate_policy(policy, &command.context, BusinessAction::DeleteComment)?;
        require_action(&command.context, BusinessAction::DeleteComment)?;
        let payload = business_payload_checksum(&(
            command.comment_id,
            command.video_id,
            command.deleted_at,
            command.expected_revision,
        ))?;
        validate_fingerprint(
            &command.context,
            &command.comment_id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .delete_comment(command)
            .await
            .map_err(Into::into)
    }

    pub async fn enqueue_notification(
        &self,
        command: EnqueueNotificationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageNotification)?;
        if command.notification.scope != command.context.scope
            || command.outbox.scope != command.context.scope
            || command.notification.payload.kind() != DocumentKind::NotificationPayload
            || command.outbox.payload.kind() != DocumentKind::OutboxPayload
        {
            return Err(BusinessServiceError::Invalid);
        }
        command.notification.payload.require_writable()?;
        command.outbox.payload.require_writable()?;
        command.notification.validate()?;
        command.outbox.validate()?;
        let payload = business_payload_checksum(&(&command.notification, &command.outbox))?;
        validate_fingerprint(
            &command.context,
            &command.notification.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .enqueue_notification(command)
            .await
            .map_err(Into::into)
    }

    pub async fn list_notifications(
        &self,
        request: BusinessReadRequest,
        policy: BusinessPolicyContext,
    ) -> Result<Vec<frame_domain::NotificationRecord>, BusinessServiceError> {
        validate_read_policy(policy, &request, BusinessAction::ReadNotification)?;
        self.repository
            .list_notifications(request)
            .await
            .map_err(Into::into)
    }

    pub async fn mark_notification_read(
        &self,
        command: MarkNotificationReadCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ReadNotification)?;
        if !matches!(
            command.context.principal,
            frame_ports::BusinessPrincipal::Authenticated(_)
        ) {
            return Err(BusinessServiceError::AccessDenied);
        }
        let payload = business_payload_checksum(&(command.notification_id, command.read_at))?;
        validate_fingerprint(
            &command.context,
            &command.notification_id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .mark_notification_read(command)
            .await
            .map_err(Into::into)
    }

    pub async fn advance_outbox(
        &self,
        command: AdvanceOutboxCommand,
    ) -> Result<(BusinessMutationReceipt, frame_domain::OrderedEventResult), BusinessServiceError>
    {
        require_action(&command.context, BusinessAction::ManageNotification)?;
        let payload = business_payload_checksum(&(
            command.event_sequence,
            &command.event_fingerprint,
            command.target,
        ))?;
        validate_fingerprint(
            &command.context,
            &command.outbox_id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .advance_outbox(command)
            .await
            .map_err(Into::into)
    }

    pub async fn advance_upload(
        &self,
        command: AdvanceUploadCommand,
    ) -> Result<(BusinessMutationReceipt, frame_domain::OrderedEventResult), BusinessServiceError>
    {
        require_action(&command.context, BusinessAction::ManageUpload)?;
        if command.upload.scope != command.context.scope
            || command.received_bytes > command.upload.expected_bytes
            || (command.target == frame_domain::UploadState::Complete) != command.checksum.is_some()
        {
            return Err(BusinessServiceError::Invalid);
        }
        command.upload.validate_initial()?;
        let payload = business_payload_checksum(&(
            &command.upload,
            command.event_sequence,
            &command.event_fingerprint,
            command.target,
            command.received_bytes,
            &command.checksum,
        ))?;
        validate_fingerprint(
            &command.context,
            &command.upload.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .advance_upload(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_storage_object(
        &self,
        command: PutStorageObjectCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageStorage)?;
        if command.object.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.object.validate()?;
        let payload = business_payload_checksum(&command.object)?;
        validate_fingerprint(
            &command.context,
            &command.object.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_storage_object(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_storage_integration(
        &self,
        command: PutStorageIntegrationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageStorage)?;
        if command.integration.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.integration.validate()?;
        let payload = business_payload_checksum(&command.integration)?;
        validate_fingerprint(
            &command.context,
            &command.integration.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_storage_integration(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_derivative_job(
        &self,
        command: PutDerivativeJobCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageStorage)?;
        if command.job.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.job.validate()?;
        let payload = business_payload_checksum(&command.job)?;
        validate_fingerprint(
            &command.context,
            &command.job.job_id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_derivative_job(command)
            .await
            .map_err(Into::into)
    }

    pub async fn advance_import(
        &self,
        command: AdvanceImportCommand,
    ) -> Result<(BusinessMutationReceipt, frame_domain::OrderedEventResult), BusinessServiceError>
    {
        require_action(&command.context, BusinessAction::ManageImport)?;
        if command.import.scope != command.context.scope
            || (command.target == frame_domain::ImportState::Failed)
                != command.error_class.is_some()
        {
            return Err(BusinessServiceError::Invalid);
        }
        command.import.validate_initial()?;
        let payload = business_payload_checksum(&(
            &command.import,
            command.event_sequence,
            &command.event_fingerprint,
            command.target,
            &command.error_class,
        ))?;
        validate_fingerprint(
            &command.context,
            &command.import.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .advance_import(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_developer_api_key(
        &self,
        command: PutDeveloperApiKeyCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageDeveloper)?;
        if command.key.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.key.validate()?;
        let payload = business_payload_checksum(&command.key)?;
        validate_fingerprint(
            &command.context,
            &command.key.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_developer_api_key(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_developer_app(
        &self,
        command: PutDeveloperAppCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageDeveloper)?;
        if command.app.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.app.validate()?;
        let payload = business_payload_checksum(&command.app)?;
        validate_fingerprint(
            &command.context,
            &command.app.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_developer_app(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_developer_domain(
        &self,
        command: PutDeveloperDomainCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageDeveloper)?;
        if command.domain.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.domain.validate()?;
        let payload = business_payload_checksum(&command.domain)?;
        let subject = format!(
            "{}:{}",
            command.domain.app_id,
            command.domain.domain.as_str()
        );
        validate_fingerprint(&command.context, &subject, &[payload.as_str()])?;
        self.repository
            .put_developer_domain(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_developer_video(
        &self,
        command: PutDeveloperVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageDeveloper)?;
        if command.video.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.video.validate()?;
        let payload = business_payload_checksum(&command.video)?;
        validate_fingerprint(
            &command.context,
            &command.video.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_developer_video(command)
            .await
            .map_err(Into::into)
    }

    pub async fn append_credit_transaction(
        &self,
        command: AppendCreditTransactionCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageLedger)?;
        if command.transaction.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        let mut expected = command.expected_account;
        expected.apply(&command.transaction)?;
        let payload = business_payload_checksum(&command.transaction)?;
        validate_fingerprint(
            &command.context,
            &command.transaction.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .append_credit_transaction(command)
            .await
            .map_err(Into::into)
    }

    pub async fn credit_account(
        &self,
        request: BusinessReadRequest,
        account_id: frame_domain::CreditAccountId,
        policy: BusinessPolicyContext,
    ) -> Result<frame_domain::CreditAccountRecord, BusinessServiceError> {
        validate_read_policy(policy, &request, BusinessAction::ManageLedger)?;
        self.repository
            .credit_account(request, account_id)
            .await
            .map_err(Into::into)
    }

    pub async fn append_usage(
        &self,
        command: AppendUsageCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageLedger)?;
        if command.usage.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.usage.validate()?;
        let payload = business_payload_checksum(&command.usage)?;
        validate_fingerprint(
            &command.context,
            &command.usage.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .append_usage(command)
            .await
            .map_err(Into::into)
    }

    pub async fn put_daily_storage_snapshot(
        &self,
        command: PutDailyStorageSnapshotCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageLedger)?;
        if command.snapshot.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.snapshot.validate()?;
        let payload = business_payload_checksum(&command.snapshot)?;
        validate_fingerprint(
            &command.context,
            &command.snapshot.app_id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .put_daily_storage_snapshot(command)
            .await
            .map_err(Into::into)
    }

    pub async fn handle_data(
        &self,
        command: DataHandlingCommand,
        active_legal_hold: bool,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        let export = command.context.action == BusinessAction::ExportData;
        if !matches!(
            command.context.action,
            BusinessAction::ExportData | BusinessAction::DeleteData
        ) || command.decision
            != retention_decision(command.data_class, export, active_legal_hold)
            || command.decision == RetentionDecision::Denied
        {
            return Err(BusinessServiceError::RetentionLocked);
        }
        let needs_compensation = matches!(
            command.decision,
            RetentionDecision::Delete(DeletionMode::AppendCompensatingEntry)
        );
        if needs_compensation != command.compensation.is_some() {
            return Err(BusinessServiceError::Invalid);
        }
        let compensation_checksum = if let Some(compensation) = &command.compensation {
            if compensation.transaction.scope != command.context.scope
                || compensation.transaction.kind != CreditTransactionKind::Adjustment
                || compensation.transaction.reference_kind != "data_deletion_compensation"
                || compensation.transaction.reference_digest
                    != deletion_compensation_reference(command.data_class, &command.subject_id)
            {
                return Err(BusinessServiceError::Invalid);
            }
            let mut expected = compensation.expected_account;
            expected.apply(&compensation.transaction)?;
            Some(business_payload_checksum(&(
                compensation.expected_account,
                &compensation.transaction,
            ))?)
        } else {
            None
        };
        let payload = business_payload_checksum(&(
            command.data_class,
            command.decision,
            command.subject_id.as_str(),
            compensation_checksum.as_ref().map(ChecksumSha256::as_str),
        ))?;
        validate_fingerprint(&command.context, &command.subject_id, &[payload.as_str()])?;
        self.repository
            .handle_data(command)
            .await
            .map_err(Into::into)
    }

    pub async fn list_legal_holds(
        &self,
        request: BusinessReadRequest,
        policy: BusinessPolicyContext,
    ) -> Result<Vec<frame_domain::BusinessLegalHoldRecord>, BusinessServiceError> {
        validate_read_policy(policy, &request, BusinessAction::ManageLegalHold)?;
        self.repository
            .list_legal_holds(request)
            .await
            .map_err(Into::into)
    }

    pub async fn place_legal_hold(
        &self,
        command: PlaceLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageLegalHold)?;
        if command.hold.scope != command.context.scope {
            return Err(BusinessServiceError::Invalid);
        }
        command.hold.validate()?;
        let payload = business_payload_checksum(&command.hold)?;
        validate_fingerprint(
            &command.context,
            &command.hold.id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .place_legal_hold(command)
            .await
            .map_err(Into::into)
    }

    pub async fn release_legal_hold(
        &self,
        command: ReleaseLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessServiceError> {
        require_action(&command.context, BusinessAction::ManageLegalHold)?;
        let payload = business_payload_checksum(&(command.hold_id, command.released_at))?;
        validate_fingerprint(
            &command.context,
            &command.hold_id.to_string(),
            &[payload.as_str()],
        )?;
        self.repository
            .release_legal_hold(command)
            .await
            .map_err(Into::into)
    }

    pub async fn export_tenant_data(
        &self,
        request: BusinessReadRequest,
        policy: BusinessPolicyContext,
    ) -> Result<frame_ports::TenantDataExport, BusinessServiceError> {
        validate_read_policy(policy, &request, BusinessAction::ExportData)?;
        self.repository
            .export_tenant_data(request)
            .await
            .map_err(Into::into)
    }
}

fn validate_policy(
    context: BusinessPolicyContext,
    mutation: &BusinessMutationContext,
    action: BusinessAction,
) -> Result<(), BusinessServiceError> {
    if context.scope != mutation.scope {
        return Err(BusinessServiceError::AccessDenied);
    }
    let actor_matches = match (context.actor, &mutation.principal) {
        (
            BusinessActor::Authenticated {
                tenant_id, user_id, ..
            },
            frame_ports::BusinessPrincipal::Authenticated(principal),
        ) => tenant_id == mutation.scope.tenant_id && user_id == *principal,
        (
            BusinessActor::Anonymous {
                actor_digest_present: true,
            },
            frame_ports::BusinessPrincipal::Anonymous(_),
        ) => true,
        _ => false,
    };
    if !actor_matches {
        return Err(BusinessServiceError::AccessDenied);
    }
    match BusinessAuthorizationPolicy::evaluate(context, action) {
        BusinessAuthorizationDecision::Allow => Ok(()),
        BusinessAuthorizationDecision::AccessDenied | BusinessAuthorizationDecision::Deleted => {
            Err(BusinessServiceError::AccessDenied)
        }
    }
}

fn validate_read_policy(
    context: BusinessPolicyContext,
    request: &BusinessReadRequest,
    action: BusinessAction,
) -> Result<(), BusinessServiceError> {
    if context.scope != request.scope {
        return Err(BusinessServiceError::AccessDenied);
    }
    let actor_matches = match (context.actor, &request.principal) {
        (
            BusinessActor::Authenticated {
                tenant_id, user_id, ..
            },
            frame_ports::BusinessPrincipal::Authenticated(principal),
        ) => tenant_id == request.scope.tenant_id && user_id == *principal,
        (
            BusinessActor::Anonymous {
                actor_digest_present: true,
            },
            frame_ports::BusinessPrincipal::Anonymous(_),
        ) => true,
        _ => false,
    };
    if !actor_matches {
        return Err(BusinessServiceError::AccessDenied);
    }
    match BusinessAuthorizationPolicy::evaluate(context, action) {
        BusinessAuthorizationDecision::Allow => Ok(()),
        BusinessAuthorizationDecision::AccessDenied | BusinessAuthorizationDecision::Deleted => {
            Err(BusinessServiceError::AccessDenied)
        }
    }
}

fn require_action(
    context: &BusinessMutationContext,
    expected: BusinessAction,
) -> Result<(), BusinessServiceError> {
    if context.action == expected {
        Ok(())
    } else {
        Err(BusinessServiceError::Invalid)
    }
}

fn validate_fingerprint(
    context: &BusinessMutationContext,
    subject_id: &str,
    payload_components: &[&str],
) -> Result<(), BusinessServiceError> {
    let scope = context.scope.tenant_id.to_string();
    let principal = context.principal.subject_for_receipt();
    let components = [
        context.action.stable_code().as_bytes(),
        scope.as_bytes(),
        context.principal.stable_kind().as_bytes(),
        principal.as_bytes(),
        context.idempotency_key.expose().as_bytes(),
        subject_id.as_bytes(),
    ];
    let expected = business_semantic_fingerprint(
        components
            .into_iter()
            .chain(payload_components.iter().map(|value| value.as_bytes())),
    );
    if constant_time_checksum_eq(&expected, &context.request_fingerprint) {
        Ok(())
    } else {
        Err(BusinessServiceError::Conflict)
    }
}

fn constant_time_checksum_eq(left: &ChecksumSha256, right: &ChecksumSha256) -> bool {
    let left = left.as_str().as_bytes();
    let right = right.as_str().as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
const fn data_class_code(class: BusinessDataClass) -> &'static str {
    match class {
        BusinessDataClass::VideoMetadata => "video_metadata",
        BusinessDataClass::VideoEdit => "video_edit",
        BusinessDataClass::Share => "share",
        BusinessDataClass::Comment => "comment",
        BusinessDataClass::Notification => "notification",
        BusinessDataClass::Outbox => "outbox",
        BusinessDataClass::StorageIntegration => "storage_integration",
        BusinessDataClass::StorageObject => "storage_object",
        BusinessDataClass::DerivativeJob => "derivative_job",
        BusinessDataClass::Upload => "upload",
        BusinessDataClass::Import => "import",
        BusinessDataClass::DeveloperApp => "developer_app",
        BusinessDataClass::DeveloperDomain => "developer_domain",
        BusinessDataClass::DeveloperApiKey => "developer_api_key",
        BusinessDataClass::DeveloperVideo => "developer_video",
        BusinessDataClass::CreditAccount => "credit_account",
        BusinessDataClass::CreditTransaction => "credit_transaction",
        BusinessDataClass::UsageLedger => "usage_ledger",
        BusinessDataClass::DailyStorageSnapshot => "daily_storage_snapshot",
        BusinessDataClass::MessengerLegacy => "messenger_legacy",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_errors_are_stable_and_redacted() {
        assert_eq!(
            BusinessServiceError::from(BusinessPortError::Unavailable),
            BusinessServiceError::Unavailable
        );
        assert_eq!(format!("{:?}", BusinessServiceError::Corrupt), "Corrupt");
    }

    #[test]
    fn checksum_comparison_covers_all_bytes() {
        let first = ChecksumSha256::digest_bytes(b"first");
        let same = ChecksumSha256::digest_bytes(b"first");
        let other = ChecksumSha256::digest_bytes(b"other");
        assert!(constant_time_checksum_eq(&first, &same));
        assert!(!constant_time_checksum_eq(&first, &other));
    }

    #[test]
    fn every_data_class_has_a_stable_code() {
        for class in BusinessDataClass::ALL {
            assert!(!data_class_code(class).is_empty());
        }
    }
}
