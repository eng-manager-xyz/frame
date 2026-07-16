//! Provider-neutral persistence capabilities for business metadata.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    BusinessAction, BusinessAuthorityFence, BusinessCommentRecord, BusinessDataClass,
    BusinessLegalHoldRecord, BusinessOperationId, BusinessRevision, BusinessScope,
    BusinessVideoRecord, ChecksumSha256, CommentId, CreditAccountId, CreditAccountRecord,
    CreditAccountState, CreditTransactionRecord, DailyStorageSnapshot, DerivativeJobManifest,
    DeveloperApiKeyRecord, DeveloperAppRecord, DeveloperDomainRecord, DeveloperVideoRecord,
    IdempotencyKey, ImportedVideoRecord, LegalHoldId, NotificationId, NotificationRecord,
    OrderedEventResult, OutboxRecord, RetentionDecision, StorageIntegrationRecord,
    StorageObjectManifest, TimestampMillis, UploadState, UsageLedgerRecord, UserId,
    VideoEditRecord, VideoId, VideoShareRecord, VideoUploadRecord,
};
use thiserror::Error;

#[derive(Clone, Copy, Error, PartialEq, Eq)]
pub enum BusinessPortError {
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
    #[error("the business repository is unavailable")]
    Unavailable,
    #[error("the business repository returned corrupt state")]
    Corrupt,
}

impl fmt::Debug for BusinessPortError {
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

#[derive(Clone, PartialEq, Eq)]
pub enum BusinessPrincipal {
    Authenticated(UserId),
    /// Only a keyed digest crosses the repository boundary. Raw visitor IDs,
    /// IP addresses, and cookies are forbidden here.
    Anonymous(ChecksumSha256),
}

impl fmt::Debug for BusinessPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authenticated(user_id) => formatter
                .debug_tuple("Authenticated")
                .field(user_id)
                .finish(),
            Self::Anonymous(_) => formatter.write_str("Anonymous([redacted])"),
        }
    }
}

impl BusinessPrincipal {
    #[must_use]
    pub fn stable_kind(&self) -> &'static str {
        match self {
            Self::Authenticated(_) => "user",
            Self::Anonymous(_) => "anonymous",
        }
    }

    #[must_use]
    pub fn subject_for_receipt(&self) -> String {
        match self {
            Self::Authenticated(user_id) => user_id.to_string(),
            Self::Anonymous(digest) => digest.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessReadRequest {
    pub scope: BusinessScope,
    pub principal: BusinessPrincipal,
    pub identity_revision: BusinessRevision,
    pub session_version: BusinessRevision,
    pub video_id: Option<VideoId>,
    /// Caller-supplied clock used for deterministic export/audit timestamps.
    pub observed_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessMutationContext {
    pub operation_id: BusinessOperationId,
    pub scope: BusinessScope,
    pub principal: BusinessPrincipal,
    pub authority_fence: BusinessAuthorityFence,
    pub action: BusinessAction,
    pub idempotency_key: IdempotencyKey,
    pub request_fingerprint: ChecksumSha256,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusinessMutationResult {
    Created,
    Applied,
    Accepted,
    Revoked,
    Tombstoned,
    Purged,
    Unchanged,
}

impl BusinessMutationResult {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Applied => "applied",
            Self::Accepted => "accepted",
            Self::Revoked => "revoked",
            Self::Tombstoned => "tombstoned",
            Self::Purged => "purged",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessMutationReceipt {
    pub operation_id: BusinessOperationId,
    pub scope: BusinessScope,
    pub principal_kind: String,
    pub principal_subject: String,
    pub action: String,
    pub subject_id: String,
    pub request_fingerprint: ChecksumSha256,
    pub result: BusinessMutationResult,
    pub resulting_revision: BusinessRevision,
    pub committed_at: TimestampMillis,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessVideoSnapshot {
    pub video: BusinessVideoRecord,
    pub latest_edit: Option<VideoEditRecord>,
    pub active_shares: Vec<VideoShareRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutVideoCommand {
    pub context: BusinessMutationContext,
    pub video: BusinessVideoRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutEditCommand {
    pub context: BusinessMutationContext,
    pub edit: VideoEditRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutShareCommand {
    pub context: BusinessMutationContext,
    pub share: VideoShareRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutCommentCommand {
    pub context: BusinessMutationContext,
    pub comment: BusinessCommentRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteCommentCommand {
    pub context: BusinessMutationContext,
    pub comment_id: CommentId,
    pub video_id: VideoId,
    pub deleted_at: TimestampMillis,
    pub expected_revision: BusinessRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueNotificationCommand {
    pub context: BusinessMutationContext,
    pub notification: NotificationRecord,
    pub outbox: OutboxRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkNotificationReadCommand {
    pub context: BusinessMutationContext,
    pub notification_id: NotificationId,
    pub read_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceOutboxCommand {
    pub context: BusinessMutationContext,
    pub outbox_id: frame_domain::OutboxEventId,
    pub event_sequence: BusinessRevision,
    pub event_fingerprint: ChecksumSha256,
    pub target: frame_domain::DeliveryState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceUploadCommand {
    pub context: BusinessMutationContext,
    pub upload: VideoUploadRecord,
    pub event_sequence: BusinessRevision,
    pub event_fingerprint: ChecksumSha256,
    pub target: UploadState,
    pub received_bytes: frame_domain::ByteSize,
    pub checksum: Option<ChecksumSha256>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutStorageObjectCommand {
    pub context: BusinessMutationContext,
    pub object: StorageObjectManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutStorageIntegrationCommand {
    pub context: BusinessMutationContext,
    pub integration: StorageIntegrationRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDerivativeJobCommand {
    pub context: BusinessMutationContext,
    pub job: DerivativeJobManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceImportCommand {
    pub context: BusinessMutationContext,
    pub import: ImportedVideoRecord,
    pub event_sequence: BusinessRevision,
    pub event_fingerprint: ChecksumSha256,
    pub target: frame_domain::ImportState,
    pub error_class: Option<frame_domain::RedactedFailureClass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDeveloperApiKeyCommand {
    pub context: BusinessMutationContext,
    pub key: DeveloperApiKeyRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDeveloperAppCommand {
    pub context: BusinessMutationContext,
    pub app: DeveloperAppRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDeveloperDomainCommand {
    pub context: BusinessMutationContext,
    pub domain: DeveloperDomainRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDeveloperVideoCommand {
    pub context: BusinessMutationContext,
    pub video: DeveloperVideoRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendCreditTransactionCommand {
    pub context: BusinessMutationContext,
    pub expected_account: CreditAccountState,
    pub transaction: CreditTransactionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendUsageCommand {
    pub context: BusinessMutationContext,
    pub usage: UsageLedgerRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDailyStorageSnapshotCommand {
    pub context: BusinessMutationContext,
    pub snapshot: DailyStorageSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataHandlingCommand {
    pub context: BusinessMutationContext,
    pub data_class: BusinessDataClass,
    pub decision: RetentionDecision,
    pub subject_id: String,
    pub compensation: Option<LedgerCompensation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerCompensation {
    pub expected_account: CreditAccountState,
    pub transaction: CreditTransactionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceLegalHoldCommand {
    pub context: BusinessMutationContext,
    pub hold: BusinessLegalHoldRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseLegalHoldCommand {
    pub context: BusinessMutationContext,
    pub hold_id: LegalHoldId,
    pub released_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantExportManifest {
    pub scope: BusinessScope,
    pub generated_at: TimestampMillis,
    pub source_revision: BusinessRevision,
    pub class_counts: Vec<(BusinessDataClass, u64)>,
    pub content_checksum: ChecksumSha256,
    pub excludes_secrets: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantExportRow {
    pub data_class: BusinessDataClass,
    pub subject_id: String,
    pub export_json: String,
    pub checksum: ChecksumSha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantDataExport {
    pub manifest: TenantExportManifest,
    pub rows: Vec<TenantExportRow>,
}

/// Repository contract for all retained Issue-15 aggregates.
///
/// Every implementation must execute its authorization assertion as the first
/// database statement. It must retrieve replay receipts only for the current
/// principal and must leave tenant state unchanged on every denial.
#[async_trait]
pub trait BusinessRepository: Send + Sync {
    async fn video_snapshot(
        &self,
        request: BusinessReadRequest,
    ) -> Result<BusinessVideoSnapshot, BusinessPortError>;

    async fn operation_receipt(
        &self,
        request: BusinessReadRequest,
        idempotency_key: &IdempotencyKey,
    ) -> Result<Option<BusinessMutationReceipt>, BusinessPortError>;

    async fn put_video(
        &self,
        command: PutVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_edit(
        &self,
        command: PutEditCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_share(
        &self,
        command: PutShareCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_comment(
        &self,
        command: PutCommentCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn list_comments(
        &self,
        request: BusinessReadRequest,
    ) -> Result<Vec<BusinessCommentRecord>, BusinessPortError>;

    async fn delete_comment(
        &self,
        command: DeleteCommentCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn enqueue_notification(
        &self,
        command: EnqueueNotificationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn list_notifications(
        &self,
        request: BusinessReadRequest,
    ) -> Result<Vec<NotificationRecord>, BusinessPortError>;

    async fn mark_notification_read(
        &self,
        command: MarkNotificationReadCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn advance_outbox(
        &self,
        command: AdvanceOutboxCommand,
    ) -> Result<(BusinessMutationReceipt, OrderedEventResult), BusinessPortError>;

    async fn advance_upload(
        &self,
        command: AdvanceUploadCommand,
    ) -> Result<(BusinessMutationReceipt, OrderedEventResult), BusinessPortError>;

    async fn put_storage_object(
        &self,
        command: PutStorageObjectCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_storage_integration(
        &self,
        command: PutStorageIntegrationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_derivative_job(
        &self,
        command: PutDerivativeJobCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn advance_import(
        &self,
        command: AdvanceImportCommand,
    ) -> Result<(BusinessMutationReceipt, OrderedEventResult), BusinessPortError>;

    async fn put_developer_api_key(
        &self,
        command: PutDeveloperApiKeyCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_developer_app(
        &self,
        command: PutDeveloperAppCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_developer_domain(
        &self,
        command: PutDeveloperDomainCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_developer_video(
        &self,
        command: PutDeveloperVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn append_credit_transaction(
        &self,
        command: AppendCreditTransactionCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn credit_account(
        &self,
        request: BusinessReadRequest,
        account_id: CreditAccountId,
    ) -> Result<CreditAccountRecord, BusinessPortError>;

    async fn append_usage(
        &self,
        command: AppendUsageCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn put_daily_storage_snapshot(
        &self,
        command: PutDailyStorageSnapshotCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn handle_data(
        &self,
        command: DataHandlingCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn list_legal_holds(
        &self,
        request: BusinessReadRequest,
    ) -> Result<Vec<BusinessLegalHoldRecord>, BusinessPortError>;

    async fn place_legal_hold(
        &self,
        command: PlaceLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn release_legal_hold(
        &self,
        command: ReleaseLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError>;

    async fn export_manifest(
        &self,
        request: BusinessReadRequest,
    ) -> Result<TenantExportManifest, BusinessPortError>;

    async fn export_tenant_data(
        &self,
        request: BusinessReadRequest,
    ) -> Result<TenantDataExport, BusinessPortError>;
}
