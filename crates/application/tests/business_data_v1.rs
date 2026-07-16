use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use frame_application::{BusinessDataService, BusinessServiceError};
use frame_domain::{
    BusinessAction, BusinessActor, BusinessAuthorityFence, BusinessCommentRecord,
    BusinessOperationId, BusinessPolicyContext, BusinessRevision, BusinessScope,
    BusinessVideoRecord, ChecksumSha256, CommentAuthor, CommentBody, CommentId, CommentKind,
    DocumentCompatibility, DocumentKind, IdempotencyKey, ImportId, ImportProvider, ImportState,
    ImportedVideoRecord, OrderedEventResult, OrderedImportLifecycle, OrganizationId,
    OrganizationRevision, OrganizationRole, RedactedFailureClass, SecretDigest, TimestampMillis,
    UserId, VersionedBusinessDocument, VideoId, VideoPrivacy, business_initial_event_fingerprint,
    business_payload_checksum, business_semantic_fingerprint,
};
use frame_ports::{
    AdvanceImportCommand, AdvanceOutboxCommand, AdvanceUploadCommand,
    AppendCreditTransactionCommand, AppendUsageCommand, BusinessMutationContext,
    BusinessMutationReceipt, BusinessMutationResult, BusinessPortError, BusinessPrincipal,
    BusinessReadRequest, BusinessRepository, BusinessVideoSnapshot, DataHandlingCommand,
    DeleteCommentCommand, EnqueueNotificationCommand, MarkNotificationReadCommand,
    PlaceLegalHoldCommand, PutCommentCommand, PutDailyStorageSnapshotCommand,
    PutDerivativeJobCommand, PutDeveloperApiKeyCommand, PutDeveloperAppCommand,
    PutDeveloperDomainCommand, PutDeveloperVideoCommand, PutEditCommand, PutShareCommand,
    PutStorageIntegrationCommand, PutStorageObjectCommand, PutVideoCommand,
    ReleaseLegalHoldCommand, TenantDataExport, TenantExportManifest,
};

#[derive(Default)]
struct RecordingRepository {
    video_writes: AtomicUsize,
    comment_writes: AtomicUsize,
    import_advances: AtomicUsize,
}

fn receipt(context: &BusinessMutationContext, subject: String) -> BusinessMutationReceipt {
    BusinessMutationReceipt {
        operation_id: context.operation_id,
        scope: context.scope,
        principal_kind: context.principal.stable_kind().into(),
        principal_subject: context.principal.subject_for_receipt(),
        action: context.action.stable_code().into(),
        subject_id: subject,
        request_fingerprint: context.request_fingerprint.clone(),
        result: BusinessMutationResult::Applied,
        resulting_revision: BusinessRevision::new(1).expect("revision"),
        committed_at: context.occurred_at,
        replayed: false,
    }
}

#[async_trait]
impl BusinessRepository for RecordingRepository {
    async fn video_snapshot(
        &self,
        _request: BusinessReadRequest,
    ) -> Result<BusinessVideoSnapshot, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn operation_receipt(
        &self,
        _request: BusinessReadRequest,
        _idempotency_key: &IdempotencyKey,
    ) -> Result<Option<BusinessMutationReceipt>, BusinessPortError> {
        Ok(None)
    }

    async fn put_video(
        &self,
        command: PutVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.video_writes.fetch_add(1, Ordering::SeqCst);
        Ok(receipt(&command.context, command.video.id.to_string()))
    }

    async fn put_edit(
        &self,
        _command: PutEditCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_share(
        &self,
        _command: PutShareCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_comment(
        &self,
        command: PutCommentCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.comment_writes.fetch_add(1, Ordering::SeqCst);
        Ok(receipt(&command.context, command.comment.id.to_string()))
    }

    async fn list_comments(
        &self,
        _request: BusinessReadRequest,
    ) -> Result<Vec<BusinessCommentRecord>, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn delete_comment(
        &self,
        _command: DeleteCommentCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn enqueue_notification(
        &self,
        _command: EnqueueNotificationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn list_notifications(
        &self,
        _request: BusinessReadRequest,
    ) -> Result<Vec<frame_domain::NotificationRecord>, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn mark_notification_read(
        &self,
        _command: MarkNotificationReadCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn advance_outbox(
        &self,
        _command: AdvanceOutboxCommand,
    ) -> Result<(BusinessMutationReceipt, frame_domain::OrderedEventResult), BusinessPortError>
    {
        Err(BusinessPortError::Invalid)
    }

    async fn advance_upload(
        &self,
        _command: AdvanceUploadCommand,
    ) -> Result<(BusinessMutationReceipt, frame_domain::OrderedEventResult), BusinessPortError>
    {
        Err(BusinessPortError::Invalid)
    }

    async fn put_storage_object(
        &self,
        _command: PutStorageObjectCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_storage_integration(
        &self,
        _command: PutStorageIntegrationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_derivative_job(
        &self,
        _command: PutDerivativeJobCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn advance_import(
        &self,
        command: AdvanceImportCommand,
    ) -> Result<(BusinessMutationReceipt, frame_domain::OrderedEventResult), BusinessPortError>
    {
        self.import_advances.fetch_add(1, Ordering::SeqCst);
        Ok((
            receipt(&command.context, command.import.id.to_string()),
            OrderedEventResult::Applied,
        ))
    }

    async fn put_developer_api_key(
        &self,
        _command: PutDeveloperApiKeyCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_developer_app(
        &self,
        _command: PutDeveloperAppCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_developer_domain(
        &self,
        _command: PutDeveloperDomainCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_developer_video(
        &self,
        _command: PutDeveloperVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn append_credit_transaction(
        &self,
        _command: AppendCreditTransactionCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn credit_account(
        &self,
        _request: BusinessReadRequest,
        _account_id: frame_domain::CreditAccountId,
    ) -> Result<frame_domain::CreditAccountRecord, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn append_usage(
        &self,
        _command: AppendUsageCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn put_daily_storage_snapshot(
        &self,
        _command: PutDailyStorageSnapshotCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn handle_data(
        &self,
        _command: DataHandlingCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn list_legal_holds(
        &self,
        _request: BusinessReadRequest,
    ) -> Result<Vec<frame_domain::BusinessLegalHoldRecord>, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn place_legal_hold(
        &self,
        _command: PlaceLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn release_legal_hold(
        &self,
        _command: ReleaseLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn export_manifest(
        &self,
        _request: BusinessReadRequest,
    ) -> Result<TenantExportManifest, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }

    async fn export_tenant_data(
        &self,
        _request: BusinessReadRequest,
    ) -> Result<TenantDataExport, BusinessPortError> {
        Err(BusinessPortError::Invalid)
    }
}

struct Fixture {
    scope: BusinessScope,
    actor: UserId,
    video_id: VideoId,
    metadata: VersionedBusinessDocument,
}

fn fixture() -> Fixture {
    let organization_id = OrganizationId::new();
    Fixture {
        scope: BusinessScope::from_organization(organization_id).expect("scope"),
        actor: UserId::new(),
        video_id: VideoId::new(),
        metadata: VersionedBusinessDocument::parse(
            DocumentKind::VideoMetadata,
            r#"{"schema_version":1,"title":"Contract"}"#,
        )
        .expect("metadata"),
    }
}

fn fence() -> BusinessAuthorityFence {
    BusinessAuthorityFence {
        identity_revision: OrganizationRevision::INITIAL,
        session_version: OrganizationRevision::INITIAL,
        organization_revision: OrganizationRevision::INITIAL,
        organization_authority_version: OrganizationRevision::INITIAL,
        membership_revision: OrganizationRevision::INITIAL,
        membership_authority_version: OrganizationRevision::INITIAL,
        resource_revision: BusinessRevision::INITIAL,
    }
}

fn context(
    fixture: &Fixture,
    principal: BusinessPrincipal,
    action: BusinessAction,
    idempotency: &str,
    subject: &str,
    payload: &[&str],
) -> BusinessMutationContext {
    let key = IdempotencyKey::parse(idempotency).expect("idempotency");
    let scope = fixture.scope.tenant_id.to_string();
    let principal_subject = principal.subject_for_receipt();
    let base = [
        action.stable_code().as_bytes(),
        scope.as_bytes(),
        principal.stable_kind().as_bytes(),
        principal_subject.as_bytes(),
        key.expose().as_bytes(),
        subject.as_bytes(),
    ];
    let request_fingerprint = business_semantic_fingerprint(
        base.into_iter()
            .chain(payload.iter().map(|value| value.as_bytes())),
    );
    BusinessMutationContext {
        operation_id: BusinessOperationId::new(),
        scope: fixture.scope,
        principal,
        authority_fence: fence(),
        action,
        idempotency_key: key,
        request_fingerprint,
        occurred_at: TimestampMillis::new(1_700_000_000_000).expect("time"),
    }
}

fn owner_policy(fixture: &Fixture, privacy: VideoPrivacy) -> BusinessPolicyContext {
    BusinessPolicyContext {
        scope: fixture.scope,
        actor: BusinessActor::Authenticated {
            tenant_id: fixture.scope.tenant_id,
            user_id: fixture.actor,
            role: OrganizationRole::Owner,
        },
        privacy,
        resource_deleted: false,
        comments_enabled: true,
        owns_resource: true,
        owns_comment: false,
    }
}

#[tokio::test]
async fn semantic_fingerprint_is_recomputed_before_repository_io() {
    let fixture = fixture();
    let repository = RecordingRepository::default();
    let service = BusinessDataService::new(repository);
    let video = BusinessVideoRecord {
        id: fixture.video_id,
        scope: fixture.scope,
        owner_id: fixture.actor,
        privacy: VideoPrivacy::Private,
        metadata: fixture.metadata.clone(),
        comments_enabled: true,
        created_at: TimestampMillis::new(10).expect("time"),
        updated_at: TimestampMillis::new(10).expect("time"),
        deleted_at: None,
        revision: BusinessRevision::new(1).expect("revision"),
    };
    let subject = video.id.to_string();
    let payload = business_payload_checksum(&video).expect("payload checksum");
    let mut invalid = context(
        &fixture,
        BusinessPrincipal::Authenticated(fixture.actor),
        BusinessAction::ManageVideo,
        "video:write:1",
        &subject,
        &[payload.as_str()],
    );
    invalid.request_fingerprint = ChecksumSha256::digest_bytes(b"tampered");
    assert_eq!(
        service
            .put_video(
                PutVideoCommand {
                    context: invalid,
                    video: video.clone(),
                },
                owner_policy(&fixture, VideoPrivacy::Private),
            )
            .await,
        Err(BusinessServiceError::Conflict)
    );
    assert_eq!(service.into_inner().video_writes.load(Ordering::SeqCst), 0);

    let repository = RecordingRepository::default();
    let service = BusinessDataService::new(repository);
    let valid = context(
        &fixture,
        BusinessPrincipal::Authenticated(fixture.actor),
        BusinessAction::ManageVideo,
        "video:write:2",
        &subject,
        &[payload.as_str()],
    );
    service
        .put_video(
            PutVideoCommand {
                context: valid,
                video,
            },
            owner_policy(&fixture, VideoPrivacy::Private),
        )
        .await
        .expect("valid write");
    assert_eq!(service.into_inner().video_writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn caller_supplied_policy_actor_is_bound_to_the_mutation_principal() {
    let fixture = fixture();
    let video = BusinessVideoRecord {
        id: fixture.video_id,
        scope: fixture.scope,
        owner_id: fixture.actor,
        privacy: VideoPrivacy::Private,
        metadata: fixture.metadata.clone(),
        comments_enabled: true,
        created_at: TimestampMillis::new(10).expect("time"),
        updated_at: TimestampMillis::new(10).expect("time"),
        deleted_at: None,
        revision: BusinessRevision::new(1).expect("revision"),
    };
    let subject = video.id.to_string();
    let payload = business_payload_checksum(&video).expect("payload checksum");
    let command = PutVideoCommand {
        context: context(
            &fixture,
            BusinessPrincipal::Authenticated(fixture.actor),
            BusinessAction::ManageVideo,
            "video:policy:1",
            &subject,
            &[payload.as_str()],
        ),
        video,
    };
    let mut forged_policy = owner_policy(&fixture, VideoPrivacy::Private);
    forged_policy.actor = BusinessActor::Authenticated {
        tenant_id: fixture.scope.tenant_id,
        user_id: UserId::new(),
        role: OrganizationRole::Owner,
    };
    let service = BusinessDataService::new(RecordingRepository::default());
    assert_eq!(
        service.put_video(command, forged_policy).await,
        Err(BusinessServiceError::AccessDenied)
    );
    assert_eq!(service.into_inner().video_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn anonymous_comment_identity_cannot_be_substituted() {
    let fixture = fixture();
    let repository = RecordingRepository::default();
    let service = BusinessDataService::new(repository);
    let comment_id = CommentId::new();
    let principal = ChecksumSha256::digest_bytes(b"anonymous-a");
    let other = SecretDigest::parse_sha256(ChecksumSha256::digest_bytes(b"anonymous-b").as_str())
        .expect("digest");
    let body = CommentBody::parse("hello").expect("body");
    let subject = comment_id.to_string();
    let command = PutCommentCommand {
        context: context(
            &fixture,
            BusinessPrincipal::Anonymous(principal),
            BusinessAction::CreateComment,
            "comment:create:1",
            &subject,
            &[&fixture.video_id.to_string(), body.as_str()],
        ),
        comment: BusinessCommentRecord {
            id: comment_id,
            scope: fixture.scope,
            video_id: fixture.video_id,
            parent_comment_id: None,
            author: CommentAuthor::Anonymous(other),
            kind: CommentKind::Text,
            body,
            timeline_micros: None,
            created_at: TimestampMillis::new(10).expect("time"),
            updated_at: TimestampMillis::new(10).expect("time"),
            deleted_at: None,
            revision: BusinessRevision::new(1).expect("revision"),
        },
    };
    let policy = BusinessPolicyContext {
        scope: fixture.scope,
        actor: BusinessActor::Anonymous {
            actor_digest_present: true,
        },
        privacy: VideoPrivacy::Unlisted,
        resource_deleted: false,
        comments_enabled: true,
        owns_resource: false,
        owns_comment: false,
    };
    assert_eq!(
        service.put_comment(command, policy).await,
        Err(BusinessServiceError::AccessDenied)
    );
    assert_eq!(
        service.into_inner().comment_writes.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn failed_import_requires_one_redacted_failure_class() {
    let fixture = fixture();
    let service = BusinessDataService::new(RecordingRepository::default());
    let import = ImportedVideoRecord {
        id: ImportId::new(),
        scope: fixture.scope,
        video_id: Some(fixture.video_id),
        source: ImportProvider::Loom,
        external_id_digest: SecretDigest::parse_sha256(
            ChecksumSha256::digest_bytes(b"legacy-import").as_str(),
        )
        .expect("digest"),
        idempotency_key: IdempotencyKey::parse("import:application:one").expect("key"),
        lifecycle: OrderedImportLifecycle {
            state: ImportState::Queued,
            last_sequence: BusinessRevision::INITIAL,
            last_fingerprint: business_initial_event_fingerprint(),
        },
        error_class: None,
        created_at: TimestampMillis::new(10).expect("time"),
        updated_at: TimestampMillis::new(10).expect("time"),
    };
    let sequence = BusinessRevision::new(1).expect("sequence");
    let event_fingerprint = ChecksumSha256::digest_bytes(b"failed-import");
    let subject = import.id.to_string();
    let missing_error: Option<RedactedFailureClass> = None;
    let missing_payload = business_payload_checksum(&(
        &import,
        sequence,
        &event_fingerprint,
        ImportState::Failed,
        &missing_error,
    ))
    .expect("payload");
    let invalid = AdvanceImportCommand {
        context: context(
            &fixture,
            BusinessPrincipal::Authenticated(fixture.actor),
            BusinessAction::ManageImport,
            "import:application:missing-error",
            &subject,
            &[missing_payload.as_str()],
        ),
        import: import.clone(),
        event_sequence: sequence,
        event_fingerprint: event_fingerprint.clone(),
        target: ImportState::Failed,
        error_class: None,
    };
    assert_eq!(
        service.advance_import(invalid).await,
        Err(BusinessServiceError::Invalid)
    );

    let error_class = RedactedFailureClass::parse("provider_timeout").expect("failure");
    let valid_payload = business_payload_checksum(&(
        &import,
        sequence,
        &event_fingerprint,
        ImportState::Failed,
        &Some(error_class.clone()),
    ))
    .expect("payload");
    let valid = AdvanceImportCommand {
        context: context(
            &fixture,
            BusinessPrincipal::Authenticated(fixture.actor),
            BusinessAction::ManageImport,
            "import:application:valid-error",
            &subject,
            &[valid_payload.as_str()],
        ),
        import,
        event_sequence: sequence,
        event_fingerprint,
        target: ImportState::Failed,
        error_class: Some(error_class),
    };
    service.advance_import(valid).await.expect("valid failure");
    assert_eq!(
        service.into_inner().import_advances.load(Ordering::SeqCst),
        1
    );
}

#[test]
fn unknown_document_versions_are_preserved_but_never_rewritten() {
    let future = VersionedBusinessDocument::parse(
        DocumentKind::VideoEdit,
        r#"{"future_field":"opaque","schema_version":9}"#,
    )
    .expect("canonical future document");
    assert_eq!(
        future.compatibility(),
        DocumentCompatibility::ReadOnlyPreserve
    );
    assert!(future.require_writable().is_err());
    assert_eq!(
        future.canonical_json(),
        r#"{"future_field":"opaque","schema_version":9}"#
    );
}

#[test]
fn private_video_policy_denies_non_owner_member_without_leaking_state() {
    let fixture = fixture();
    let context = BusinessPolicyContext {
        scope: fixture.scope,
        actor: BusinessActor::Authenticated {
            tenant_id: fixture.scope.tenant_id,
            user_id: UserId::new(),
            role: OrganizationRole::Member,
        },
        privacy: VideoPrivacy::Private,
        resource_deleted: false,
        comments_enabled: true,
        owns_resource: false,
        owns_comment: false,
    };
    assert_eq!(
        frame_domain::BusinessAuthorizationPolicy::evaluate(context, BusinessAction::ReadVideo),
        frame_domain::BusinessAuthorizationDecision::AccessDenied
    );
}
