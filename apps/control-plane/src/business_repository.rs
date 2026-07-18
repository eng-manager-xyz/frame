//! Race-safe D1 repository for Issue-15 business metadata.

use async_trait::async_trait;
use frame_domain::{
    BusinessDataClass, BusinessLegalHoldRecord, BusinessObjectRole, BusinessOperationId,
    BusinessRevision, BusinessScope, BusinessVideoRecord, ChecksumSha256, CommentAuthor,
    CreditAccountRecord, CreditAccountState, CreditTransactionKind, DeliveryState,
    DerivativeExecutor, DerivativeState, DocumentKind, ImportState, OrderedDeliveryLifecycle,
    OrderedEventResult, OrderedImportLifecycle, OrderedUploadLifecycle, ShareMode,
    StorageObjectState, TimestampMillis, UploadState, UsageKind, VersionedBusinessDocument,
    VideoId, VideoPrivacy, business_payload_checksum, business_semantic_fingerprint,
    tenant_scoped_idempotency_digest,
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
    ReleaseLegalHoldCommand, TenantDataExport, TenantExportManifest, TenantExportRow,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, send::IntoSendFuture};

const AUTHORITY_ASSERT_SQL: &str = include_str!("../queries/business/authority_assert.sql");
const READ_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/business/read_authority_assert.sql");
const EXPORT_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/business/export_authority_assert.sql");
const ASSERTION_CLEANUP_SQL: &str = include_str!("../queries/business/assertion_cleanup.sql");
const OPERATION_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/business/operation_absent_assert.sql");
const OPERATION_BY_IDEMPOTENCY_SQL: &str =
    include_str!("../queries/business/operation_by_idempotency.sql");
const OPERATION_INSERT_SQL: &str = include_str!("../queries/business/operation_insert.sql");
const OPERATION_MATCH_ASSERT_SQL: &str =
    include_str!("../queries/business/operation_match_assert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/business/audit_insert.sql");
const VIDEO_SNAPSHOT_SQL: &str = include_str!("../queries/business/video_snapshot.sql");
const LATEST_EDIT_SQL: &str = include_str!("../queries/business/latest_edit.sql");
const ACTIVE_SHARES_SQL: &str = include_str!("../queries/business/active_shares.sql");
const VIDEO_UPSERT_SQL: &str = include_str!("../queries/business/video_upsert.sql");
const EDIT_UPSERT_SQL: &str = include_str!("../queries/business/edit_upsert.sql");
const SHARE_UPSERT_SQL: &str = include_str!("../queries/business/share_upsert.sql");
const SHARE_EXACT_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/share_exact_postcondition.sql");
const COMMENT_INSERT_SQL: &str = include_str!("../queries/business/comment_insert.sql");
const COMMENT_LIST_SQL: &str = include_str!("../queries/business/comment_list.sql");
const COMMENT_DELETE_SQL: &str = include_str!("../queries/business/comment_delete.sql");
const COMMENT_DELETE_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/comment_delete_postcondition.sql");
const NOTIFICATION_INSERT_SQL: &str = include_str!("../queries/business/notification_insert.sql");
const NOTIFICATION_LIST_SQL: &str = include_str!("../queries/business/notification_list.sql");
const NOTIFICATION_MARK_READ_SQL: &str =
    include_str!("../queries/business/notification_mark_read.sql");
const NOTIFICATION_MARK_READ_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/notification_mark_read_postcondition.sql");
const OUTBOX_INSERT_SQL: &str = include_str!("../queries/business/outbox_insert.sql");
const EVENT_INBOX_INSERT_SQL: &str = include_str!("../queries/business/event_inbox_insert.sql");
const OUTBOX_ADVANCE_SQL: &str = include_str!("../queries/business/outbox_advance.sql");
const OUTBOX_LIFECYCLE_SQL: &str = include_str!("../queries/business/outbox_lifecycle.sql");
const IMPORT_LIFECYCLE_SQL: &str = include_str!("../queries/business/import_lifecycle.sql");
const UPLOAD_LIFECYCLE_SQL: &str = include_str!("../queries/business/upload_lifecycle.sql");
const UPLOAD_INSERT_SQL: &str = include_str!("../queries/business/upload_insert.sql");
const UPLOAD_ADVANCE_SQL: &str = include_str!("../queries/business/upload_advance.sql");
const UPLOAD_IMMUTABLE_ASSERT_SQL: &str =
    include_str!("../queries/business/upload_immutable_assert.sql");
const UPLOAD_EXACT_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/upload_exact_postcondition.sql");
const STORAGE_OBJECT_UPSERT_SQL: &str =
    include_str!("../queries/business/storage_object_upsert.sql");
const STORAGE_OBJECT_EXACT_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/storage_object_exact_postcondition.sql");
const STORAGE_INTEGRATION_UPSERT_SQL: &str =
    include_str!("../queries/business/storage_integration_upsert.sql");
const DERIVATIVE_MANIFEST_UPSERT_SQL: &str =
    include_str!("../queries/business/derivative_manifest_upsert.sql");
const DERIVATIVE_EXACT_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/derivative_exact_postcondition.sql");
const IMPORT_UPSERT_SQL: &str = include_str!("../queries/business/import_upsert.sql");
const IMPORT_ADVANCE_SQL: &str = include_str!("../queries/business/import_advance.sql");
const IMPORT_IMMUTABLE_ASSERT_SQL: &str =
    include_str!("../queries/business/import_immutable_assert.sql");
const DEVELOPER_KEY_INSERT_SQL: &str = include_str!("../queries/business/developer_key_insert.sql");
const DEVELOPER_APP_UPSERT_SQL: &str = include_str!("../queries/business/developer_app_upsert.sql");
const DEVELOPER_DOMAIN_UPSERT_SQL: &str =
    include_str!("../queries/business/developer_domain_upsert.sql");
const DEVELOPER_VIDEO_UPSERT_SQL: &str =
    include_str!("../queries/business/developer_video_upsert.sql");
const CREDIT_TRANSACTION_INSERT_SQL: &str =
    include_str!("../queries/business/credit_transaction_insert.sql");
const CREDIT_ACCOUNT_SQL: &str = include_str!("../queries/business/credit_account.sql");
const USAGE_INSERT_SQL: &str = include_str!("../queries/business/usage_insert.sql");
const DAILY_SNAPSHOT_UPSERT_SQL: &str =
    include_str!("../queries/business/daily_snapshot_upsert.sql");
const RETENTION_ASSERT_SQL: &str = include_str!("../queries/business/retention_assert.sql");
const DATA_SUBJECT_ASSERT_SQL: &str = include_str!("../queries/business/data_subject_assert.sql");
const DATA_REQUEST_INSERT_SQL: &str = include_str!("../queries/business/data_request_insert.sql");
const DELETE_VIDEO_SQL: &str = include_str!("../queries/business/delete_video.sql");
const DELETE_EDIT_SQL: &str = include_str!("../queries/business/delete_edit.sql");
const DELETE_SHARE_SQL: &str = include_str!("../queries/business/delete_share.sql");
const DELETE_COMMENT_DATA_SQL: &str = include_str!("../queries/business/delete_comment_data.sql");
const DELETE_NOTIFICATION_SQL: &str = include_str!("../queries/business/delete_notification.sql");
const DELETE_OUTBOX_SQL: &str = include_str!("../queries/business/delete_outbox.sql");
const DELETE_STORAGE_INTEGRATION_SQL: &str =
    include_str!("../queries/business/delete_storage_integration.sql");
const DELETE_STORAGE_OBJECT_SQL: &str =
    include_str!("../queries/business/delete_storage_object.sql");
const DELETE_DERIVATIVE_SQL: &str = include_str!("../queries/business/delete_derivative.sql");
const DELETE_UPLOAD_SQL: &str = include_str!("../queries/business/delete_upload.sql");
const DELETE_IMPORT_SQL: &str = include_str!("../queries/business/delete_import.sql");
const DELETE_DEVELOPER_APP_SQL: &str = include_str!("../queries/business/delete_developer_app.sql");
const DELETE_DEVELOPER_DOMAIN_SQL: &str =
    include_str!("../queries/business/delete_developer_domain.sql");
const DELETE_DEVELOPER_KEY_SQL: &str = include_str!("../queries/business/delete_developer_key.sql");
const DELETE_DEVELOPER_VIDEO_SQL: &str =
    include_str!("../queries/business/delete_developer_video.sql");
const DELETE_MESSENGER_QUARANTINE_SQL: &str =
    include_str!("../queries/business/messenger_quarantine_purge.sql");
const DELETE_MESSENGER_CONVERSATION_SQL: &str =
    include_str!("../queries/business/messenger_quarantine_delete_conversation.sql");
const DELETE_MESSENGER_MESSAGE_SQL: &str =
    include_str!("../queries/business/messenger_quarantine_delete_message.sql");
const DELETE_MESSENGER_SUPPORT_EMAIL_SQL: &str =
    include_str!("../queries/business/messenger_quarantine_delete_support_email.sql");
const DATA_DELETE_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/data_delete_postcondition.sql");
const LEDGER_COMPENSATION_ASSERT_SQL: &str =
    include_str!("../queries/business/ledger_compensation_assert.sql");
const EXPORT_COUNTS_SQL: &str = include_str!("../queries/business/export_counts.sql");
const EXPORT_REVISION_SQL: &str = include_str!("../queries/business/export_revision.sql");
const EXPORT_ROWS_SQL: &str = include_str!("../queries/business/export_rows.sql");
const EXPORT_PAGE_ROWS: u64 = 1_000;
const LEGAL_HOLD_LIST_SQL: &str = include_str!("../queries/business/legal_hold_list.sql");
const LEGAL_HOLD_INSERT_SQL: &str = include_str!("../queries/business/legal_hold_insert.sql");
const LEGAL_HOLD_RELEASE_SQL: &str = include_str!("../queries/business/legal_hold_release.sql");
const LEGAL_HOLD_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/legal_hold_postcondition.sql");
const LEGAL_HOLD_RELEASE_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/legal_hold_release_postcondition.sql");
const RESOURCE_POSTCONDITION_SQL: &str =
    include_str!("../queries/business/resource_postcondition.sql");

const AUTHORITY_SENTINEL: &str = "frame_business_authority_conflict_v1";
const REPLAY_SENTINEL: &str = "frame_business_semantic_replay_conflict_v1";
const EVENT_SENTINEL: &str = "frame_business_event_order_conflict_v1";
const ACCOUNTING_SENTINEL: &str = "frame_business_accounting_conflict_v1";
const RETENTION_SENTINEL: &str = "frame_business_retention_locked_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterFailure {
    AccessDenied,
    Stale,
    Conflict,
    Invalid,
    Retention,
    Unavailable,
    Corrupt,
}

impl AdapterFailure {
    const fn into_port(self) -> BusinessPortError {
        match self {
            Self::AccessDenied => BusinessPortError::AccessDenied,
            Self::Stale => BusinessPortError::StaleAuthority,
            Self::Conflict => BusinessPortError::Conflict,
            Self::Invalid => BusinessPortError::Invalid,
            Self::Retention => BusinessPortError::RetentionLocked,
            Self::Unavailable => BusinessPortError::Unavailable,
            Self::Corrupt => BusinessPortError::Corrupt,
        }
    }
}

type AdapterResult<T> = Result<T, AdapterFailure>;

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    organization_id: String,
    principal_kind: String,
    principal_subject: String,
    idempotency_key: String,
    action: String,
    subject_id: String,
    request_fingerprint: String,
    result_code: String,
    resulting_revision: i64,
    committed_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct VideoRow {
    id: String,
    owner_id: String,
    organization_id: String,
    privacy: String,
    metadata_json: Option<String>,
    metadata_schema_version: i64,
    metadata_checksum: Option<String>,
    comments_enabled: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    deleted_at_ms: Option<i64>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct EditRow {
    id: String,
    video_id: String,
    document_version: i64,
    edit_spec_json: String,
    document_checksum: Option<String>,
    created_by_user_id: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct ShareRow {
    id: String,
    video_id: String,
    organization_id: String,
    folder_id: Option<String>,
    shared_by_user_id: String,
    sharing_mode: String,
    shared_at_ms: i64,
    revoked_at_ms: Option<i64>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct CommentRow {
    id: String,
    video_id: String,
    parent_comment_id: Option<String>,
    author_user_id: Option<String>,
    anonymous_author_digest: Option<String>,
    body: String,
    comment_kind: String,
    timeline_micros: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
    deleted_at_ms: Option<i64>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct NotificationRow {
    id: String,
    recipient_user_id: String,
    #[serde(rename = "type")]
    kind: String,
    deduplication_key: String,
    data_json: String,
    payload_schema_version: i64,
    payload_checksum: Option<String>,
    created_at_ms: i64,
    read_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreditAccountRow {
    id: String,
    app_id: String,
    balance_microcredits: i64,
    auto_top_up_enabled: i64,
    auto_top_up_threshold_microcredits: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: i64,
    ledger_sequence: i64,
}

#[derive(Debug, Deserialize)]
struct LegalHoldRow {
    id: String,
    data_class: String,
    subject_id: String,
    reason_code: String,
    placed_by_user_id: String,
    placed_at_ms: i64,
    released_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ExportRow {
    data_class: String,
    subject_id: String,
    export_json: String,
}

#[derive(Debug, Deserialize)]
struct LifecycleRow {
    state: String,
    event_sequence: i64,
    event_fingerprint: Option<String>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    data_class: String,
    item_count: i64,
}

#[derive(Debug, Deserialize)]
struct RevisionRow {
    source_revision: i64,
}

/// The D1 adapter never exposes D1/JavaScript values through its public API.
pub struct D1BusinessRepository<'database> {
    database: &'database D1Database,
}

#[path = "business_runtime.rs"]
pub mod runtime;

impl<'database> D1BusinessRepository<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> AdapterResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| AdapterFailure::Unavailable)
    }

    async fn batch_results(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> AdapterResult<Vec<worker::D1Result>> {
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| classify_provider_error(&error.to_string()))?;
        results
            .iter()
            .all(worker::D1Result::success)
            .then_some(results)
            .ok_or(AdapterFailure::Unavailable)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AdapterResult<()> {
        self.batch_results(statements).await.map(|_| ())
    }

    fn result_rows<T: DeserializeOwned>(result: &worker::D1Result) -> AdapterResult<Vec<T>> {
        result
            .results::<serde_json::Value>()
            .map_err(|_| AdapterFailure::Unavailable)?
            .into_iter()
            .map(|row| serde_json::from_value(row).map_err(|_| AdapterFailure::Corrupt))
            .collect()
    }

    fn read_authority_statement(
        &self,
        request: &BusinessReadRequest,
        operation_id: BusinessOperationId,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            READ_AUTHORITY_ASSERT_SQL,
            &[
                string(format!("{operation_id}:read")),
                string(request.scope.organization_id),
                JsValue::from_str(request.principal.stable_kind()),
                JsValue::from_str(&request.principal.subject_for_receipt()),
                number(request.identity_revision.get())?,
                number(request.session_version.get())?,
                request
                    .video_id
                    .map_or_else(|| JsValue::from_str(""), string),
            ],
        )
    }

    fn export_authority_statement(
        &self,
        request: &BusinessReadRequest,
        operation_id: BusinessOperationId,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            EXPORT_AUTHORITY_ASSERT_SQL,
            &[
                string(format!("{operation_id}:export")),
                string(request.scope.organization_id),
                JsValue::from_str(request.principal.stable_kind()),
                JsValue::from_str(&request.principal.subject_for_receipt()),
                number(request.identity_revision.get())?,
                number(request.session_version.get())?,
            ],
        )
    }

    fn authority_statement(
        &self,
        context: &BusinessMutationContext,
        role_class: &'static str,
        resource_video_id: Option<VideoId>,
    ) -> AdapterResult<D1PreparedStatement> {
        let fence = context.authority_fence;
        self.statement(
            AUTHORITY_ASSERT_SQL,
            &[
                string(format!("{}:authority", context.operation_id)),
                string(context.scope.organization_id),
                JsValue::from_str(context.principal.stable_kind()),
                JsValue::from_str(&context.principal.subject_for_receipt()),
                number(fence.identity_revision.get())?,
                number(fence.session_version.get())?,
                number(fence.organization_revision.get())?,
                number(fence.organization_authority_version.get())?,
                number(fence.membership_revision.get())?,
                number(fence.membership_authority_version.get())?,
                JsValue::from_str(role_class),
                JsValue::from_str(context.action.stable_code()),
                resource_video_id.map_or_else(|| JsValue::from_str(""), string),
            ],
        )
    }

    async fn replay(
        &self,
        context: &BusinessMutationContext,
        subject_id: &str,
        role_class: &'static str,
        resource_video_id: Option<VideoId>,
    ) -> AdapterResult<Option<BusinessMutationReceipt>> {
        let authority = self.authority_statement(context, role_class, resource_video_id)?;
        let operation = self.statement(
            OPERATION_BY_IDEMPOTENCY_SQL,
            &[
                string(context.scope.organization_id),
                JsValue::from_str(context.principal.stable_kind()),
                JsValue::from_str(&context.principal.subject_for_receipt()),
                JsValue::from_str(context.idempotency_key.expose()),
            ],
        )?;
        let cleanup = self.statement(ASSERTION_CLEANUP_SQL, &[string(context.operation_id)])?;
        let results = self
            .batch_results(vec![authority, operation, cleanup])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let mut rows = Self::result_rows::<OperationRow>(&results[1])?;
        if rows.len() > 1 {
            return Err(AdapterFailure::Corrupt);
        }
        let Some(row) = rows.pop() else {
            return Ok(None);
        };
        if row.organization_id != context.scope.organization_id.to_string()
            || row.principal_kind != context.principal.stable_kind()
            || row.principal_subject != context.principal.subject_for_receipt()
            || row.idempotency_key != context.idempotency_key.expose()
            || row.action != context.action.stable_code()
            || row.subject_id != subject_id
            || row.request_fingerprint != context.request_fingerprint.as_str()
        {
            return Err(AdapterFailure::Conflict);
        }
        decode_receipt(row, true).map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit(
        &self,
        context: &BusinessMutationContext,
        subject_id: String,
        result: BusinessMutationResult,
        resulting_revision: BusinessRevision,
        role_class: &'static str,
        resource_video_id: Option<VideoId>,
        mut statements: Vec<D1PreparedStatement>,
    ) -> AdapterResult<BusinessMutationReceipt> {
        if let Some(receipt) = self
            .replay(context, &subject_id, role_class, resource_video_id)
            .await?
        {
            return Ok(receipt);
        }
        statements.insert(
            0,
            self.authority_statement(context, role_class, resource_video_id)?,
        );
        statements.insert(
            1,
            self.statement(
                OPERATION_ABSENT_ASSERT_SQL,
                &[
                    string(format!("{}:operation", context.operation_id)),
                    string(context.operation_id),
                    string(context.scope.organization_id),
                    JsValue::from_str(context.principal.stable_kind()),
                    JsValue::from_str(&context.principal.subject_for_receipt()),
                    JsValue::from_str(context.idempotency_key.expose()),
                ],
            )?,
        );
        statements.push(self.operation_statement(
            context,
            &subject_id,
            result,
            resulting_revision,
        )?);
        statements.push(self.audit_statement(context, &subject_id, "allow")?);
        statements.push(self.statement(ASSERTION_CLEANUP_SQL, &[string(context.operation_id)])?);
        match self.batch(statements).await {
            Ok(()) => Ok(BusinessMutationReceipt {
                operation_id: context.operation_id,
                scope: context.scope,
                principal_kind: context.principal.stable_kind().into(),
                principal_subject: context.principal.subject_for_receipt(),
                action: context.action.stable_code().into(),
                subject_id,
                request_fingerprint: context.request_fingerprint.clone(),
                result,
                resulting_revision,
                committed_at: context.occurred_at,
                replayed: false,
            }),
            Err(failure) => {
                if let Some(receipt) = self
                    .replay(context, &subject_id, role_class, resource_video_id)
                    .await?
                {
                    return Ok(receipt);
                }
                Err(failure)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_ordered(
        &self,
        context: &BusinessMutationContext,
        subject_id: String,
        result: BusinessMutationResult,
        resulting_revision: BusinessRevision,
        role_class: &'static str,
        resource_video_id: Option<VideoId>,
        mut statements: Vec<D1PreparedStatement>,
    ) -> AdapterResult<BusinessMutationReceipt> {
        if let Some(receipt) = self
            .replay(context, &subject_id, role_class, resource_video_id)
            .await?
        {
            if receipt.result == BusinessMutationResult::Accepted
                && result == BusinessMutationResult::Applied
            {
                statements.insert(
                    0,
                    self.authority_statement(context, role_class, resource_video_id)?,
                );
                statements.insert(
                    1,
                    self.statement(
                        OPERATION_MATCH_ASSERT_SQL,
                        &[
                            string(format!("{}:accepted-replay", context.operation_id)),
                            string(context.scope.organization_id),
                            JsValue::from_str(context.principal.stable_kind()),
                            JsValue::from_str(&context.principal.subject_for_receipt()),
                            JsValue::from_str(context.idempotency_key.expose()),
                            JsValue::from_str(context.action.stable_code()),
                            JsValue::from_str(&subject_id),
                            JsValue::from_str(context.request_fingerprint.as_str()),
                        ],
                    )?,
                );
                statements.push(self.audit_statement(context, &subject_id, "allow")?);
                statements
                    .push(self.statement(ASSERTION_CLEANUP_SQL, &[string(context.operation_id)])?);
                self.batch(statements).await?;
            }
            return Ok(receipt);
        }
        self.commit(
            context,
            subject_id,
            result,
            resulting_revision,
            role_class,
            resource_video_id,
            statements,
        )
        .await
    }

    fn operation_statement(
        &self,
        context: &BusinessMutationContext,
        subject_id: &str,
        result: BusinessMutationResult,
        resulting_revision: BusinessRevision,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            OPERATION_INSERT_SQL,
            &[
                string(context.operation_id),
                string(context.scope.organization_id),
                JsValue::from_str(context.principal.stable_kind()),
                JsValue::from_str(&context.principal.subject_for_receipt()),
                JsValue::from_str(context.idempotency_key.expose()),
                JsValue::from_str(context.action.stable_code()),
                JsValue::from_str(subject_id),
                JsValue::from_str(context.request_fingerprint.as_str()),
                JsValue::from_str(result.stable_code()),
                number(resulting_revision.get())?,
                timestamp(context.occurred_at),
            ],
        )
    }

    fn audit_statement(
        &self,
        context: &BusinessMutationContext,
        subject_id: &str,
        outcome: &'static str,
    ) -> AdapterResult<D1PreparedStatement> {
        let principal_digest =
            ChecksumSha256::digest_bytes(context.principal.subject_for_receipt().as_bytes());
        let subject_digest = ChecksumSha256::digest_bytes(subject_id.as_bytes());
        self.statement(
            AUDIT_INSERT_SQL,
            &[
                string(BusinessOperationId::new()),
                string(context.operation_id),
                string(context.scope.organization_id),
                JsValue::from_str(context.principal.stable_kind()),
                JsValue::from_str(principal_digest.as_str()),
                JsValue::from_str(context.action.stable_code()),
                JsValue::from_str(subject_digest.as_str()),
                JsValue::from_str(outcome),
                timestamp(context.occurred_at),
            ],
        )
    }

    fn postcondition_statement(
        &self,
        context: &BusinessMutationContext,
        kind: &'static str,
        subject_id: &str,
        revision: BusinessRevision,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            RESOURCE_POSTCONDITION_SQL,
            &[
                string(format!("{}:post:{kind}", context.operation_id)),
                JsValue::from_str(kind),
                JsValue::from_str(subject_id),
                string(context.scope.organization_id),
                number(revision.get())?,
                string(context.operation_id),
            ],
        )
    }

    async fn lifecycle_row(
        &self,
        context: &BusinessMutationContext,
        resource_video_id: Option<VideoId>,
        sql: &str,
        subject_id: &str,
    ) -> AdapterResult<LifecycleRow> {
        let authority =
            self.authority_statement(context, role_class(context), resource_video_id)?;
        let data = self.statement(
            sql,
            &[
                JsValue::from_str(subject_id),
                string(context.scope.organization_id),
            ],
        )?;
        let cleanup = self.statement(ASSERTION_CLEANUP_SQL, &[string(context.operation_id)])?;
        let results = self.batch_results(vec![authority, data, cleanup]).await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let mut rows = Self::result_rows::<LifecycleRow>(&results[1])?;
        if rows.len() != 1 {
            return Err(AdapterFailure::AccessDenied);
        }
        rows.pop().ok_or(AdapterFailure::Corrupt)
    }
}

#[async_trait]
impl BusinessRepository for D1BusinessRepository<'_> {
    async fn video_snapshot(
        &self,
        request: BusinessReadRequest,
    ) -> Result<BusinessVideoSnapshot, BusinessPortError> {
        self.video_snapshot_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn operation_receipt(
        &self,
        request: BusinessReadRequest,
        idempotency_key: &frame_domain::IdempotencyKey,
    ) -> Result<Option<BusinessMutationReceipt>, BusinessPortError> {
        self.operation_receipt_inner(request, idempotency_key)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_video(
        &self,
        command: PutVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_video_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_edit(
        &self,
        command: PutEditCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_edit_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_share(
        &self,
        command: PutShareCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_share_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_comment(
        &self,
        command: PutCommentCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_comment_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_comments(
        &self,
        request: BusinessReadRequest,
    ) -> Result<Vec<frame_domain::BusinessCommentRecord>, BusinessPortError> {
        self.list_comments_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn delete_comment(
        &self,
        command: DeleteCommentCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.delete_comment_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn enqueue_notification(
        &self,
        command: EnqueueNotificationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.enqueue_notification_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_notifications(
        &self,
        request: BusinessReadRequest,
    ) -> Result<Vec<frame_domain::NotificationRecord>, BusinessPortError> {
        self.list_notifications_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn mark_notification_read(
        &self,
        command: MarkNotificationReadCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.mark_notification_read_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn advance_outbox(
        &self,
        command: AdvanceOutboxCommand,
    ) -> Result<(BusinessMutationReceipt, OrderedEventResult), BusinessPortError> {
        self.advance_outbox_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn advance_upload(
        &self,
        command: AdvanceUploadCommand,
    ) -> Result<(BusinessMutationReceipt, OrderedEventResult), BusinessPortError> {
        self.advance_upload_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_storage_object(
        &self,
        command: PutStorageObjectCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_storage_object_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_storage_integration(
        &self,
        command: PutStorageIntegrationCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_storage_integration_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_derivative_job(
        &self,
        command: PutDerivativeJobCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_derivative_job_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn advance_import(
        &self,
        command: AdvanceImportCommand,
    ) -> Result<(BusinessMutationReceipt, OrderedEventResult), BusinessPortError> {
        self.advance_import_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_developer_api_key(
        &self,
        command: PutDeveloperApiKeyCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_developer_key_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_developer_app(
        &self,
        command: PutDeveloperAppCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_developer_app_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_developer_domain(
        &self,
        command: PutDeveloperDomainCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_developer_domain_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_developer_video(
        &self,
        command: PutDeveloperVideoCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_developer_video_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn append_credit_transaction(
        &self,
        command: AppendCreditTransactionCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.append_credit_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn credit_account(
        &self,
        request: BusinessReadRequest,
        account_id: frame_domain::CreditAccountId,
    ) -> Result<CreditAccountRecord, BusinessPortError> {
        self.credit_account_inner(request, account_id)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn append_usage(
        &self,
        command: AppendUsageCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.append_usage_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn put_daily_storage_snapshot(
        &self,
        command: PutDailyStorageSnapshotCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.put_snapshot_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn handle_data(
        &self,
        command: DataHandlingCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.handle_data_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_legal_holds(
        &self,
        request: BusinessReadRequest,
    ) -> Result<Vec<BusinessLegalHoldRecord>, BusinessPortError> {
        self.list_legal_holds_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn place_legal_hold(
        &self,
        command: PlaceLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.place_legal_hold_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn release_legal_hold(
        &self,
        command: ReleaseLegalHoldCommand,
    ) -> Result<BusinessMutationReceipt, BusinessPortError> {
        self.release_legal_hold_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn export_manifest(
        &self,
        request: BusinessReadRequest,
    ) -> Result<TenantExportManifest, BusinessPortError> {
        self.export_manifest_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn export_tenant_data(
        &self,
        request: BusinessReadRequest,
    ) -> Result<TenantDataExport, BusinessPortError> {
        self.export_tenant_data_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }
}

impl D1BusinessRepository<'_> {
    async fn video_snapshot_inner(
        &self,
        request: BusinessReadRequest,
    ) -> AdapterResult<BusinessVideoSnapshot> {
        let video_id = request.video_id.ok_or(AdapterFailure::Invalid)?;
        let operation_id = BusinessOperationId::new();
        let statements = vec![
            self.read_authority_statement(&request, operation_id)?,
            self.statement(
                VIDEO_SNAPSHOT_SQL,
                &[string(video_id), string(request.scope.organization_id)],
            )?,
            self.statement(LATEST_EDIT_SQL, &[string(video_id)])?,
            self.statement(
                ACTIVE_SHARES_SQL,
                &[string(video_id), string(request.scope.organization_id)],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
        ];
        let results = match self.batch_results(statements).await {
            Err(AdapterFailure::Stale) => return Err(AdapterFailure::AccessDenied),
            result => result?,
        };
        if results.len() != 5 {
            return Err(AdapterFailure::Corrupt);
        }
        let mut videos = Self::result_rows::<VideoRow>(&results[1])?;
        if videos.len() != 1 {
            return Err(AdapterFailure::AccessDenied);
        }
        let video = decode_video(videos.pop().ok_or(AdapterFailure::Corrupt)?, request.scope)?;
        let edits = Self::result_rows::<EditRow>(&results[2])?;
        let latest_edit = edits
            .into_iter()
            .next()
            .map(|row| decode_edit(row, request.scope))
            .transpose()?;
        let shares = Self::result_rows::<ShareRow>(&results[3])?;
        if shares.len() > 100 {
            return Err(AdapterFailure::Corrupt);
        }
        let active_shares = shares
            .into_iter()
            .map(|row| decode_share(row, request.scope))
            .collect::<AdapterResult<Vec<_>>>()?;
        Ok(BusinessVideoSnapshot {
            video,
            latest_edit,
            active_shares,
        })
    }

    async fn operation_receipt_inner(
        &self,
        request: BusinessReadRequest,
        idempotency_key: &frame_domain::IdempotencyKey,
    ) -> AdapterResult<Option<BusinessMutationReceipt>> {
        let operation_id = BusinessOperationId::new();
        let statement = self.statement(
            OPERATION_BY_IDEMPOTENCY_SQL,
            &[
                string(request.scope.organization_id),
                JsValue::from_str(request.principal.stable_kind()),
                JsValue::from_str(&request.principal.subject_for_receipt()),
                JsValue::from_str(idempotency_key.expose()),
            ],
        )?;
        let results = self
            .batch_results(vec![
                self.read_authority_statement(&request, operation_id)?,
                statement,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let mut rows = Self::result_rows::<OperationRow>(&results[1])?;
        if rows.len() > 1 {
            return Err(AdapterFailure::Corrupt);
        }
        rows.pop().map(|row| decode_receipt(row, true)).transpose()
    }

    async fn put_video_inner(
        &self,
        command: PutVideoCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let video = &command.video;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        if video.scope != context.scope || video.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        let subject = video.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageVideo,
            &subject,
            video,
        )?;
        let statements = vec![
            self.statement(
                VIDEO_UPSERT_SQL,
                &[
                    string(video.id),
                    string(video.owner_id),
                    string(video.scope.organization_id),
                    JsValue::from_str(privacy_code(video.privacy)),
                    JsValue::from_str(video.metadata.canonical_json()),
                    number(u64::from(video.metadata.schema_version()))?,
                    JsValue::from_str(video.metadata.checksum().as_str()),
                    boolean(video.comments_enabled),
                    timestamp(video.created_at),
                    timestamp(video.updated_at),
                    number(resulting.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                ],
            )?,
            self.postcondition_statement(context, "video", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            Some(video.id),
            statements,
        )
        .await
    }

    async fn put_edit_inner(
        &self,
        command: PutEditCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let edit = &command.edit;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        if edit.scope != context.scope || edit.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        if !matches!(
            context.principal,
            BusinessPrincipal::Authenticated(actor) if actor == edit.created_by_user_id
        ) {
            return Err(AdapterFailure::AccessDenied);
        }
        let subject = edit.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageEdit,
            &subject,
            edit,
        )?;
        let statements = vec![
            self.statement(
                EDIT_UPSERT_SQL,
                &[
                    string(edit.id),
                    string(edit.video_id),
                    number(u64::from(edit.document.schema_version()))?,
                    JsValue::from_str(edit.document.canonical_json()),
                    string(edit.created_by_user_id),
                    timestamp(edit.created_at),
                    timestamp(edit.updated_at),
                    number(resulting.get())?,
                    JsValue::from_str(edit.document.checksum().as_str()),
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                ],
            )?,
            self.postcondition_statement(context, "edit", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            Some(edit.video_id),
            statements,
        )
        .await
    }

    async fn put_share_inner(
        &self,
        command: PutShareCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let share = &command.share;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        if share.scope != context.scope || share.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        share.validate().map_err(|_| AdapterFailure::Invalid)?;
        if !matches!(
            context.principal,
            BusinessPrincipal::Authenticated(actor) if actor == share.shared_by_user_id
        ) {
            return Err(AdapterFailure::AccessDenied);
        }
        let subject = share.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageShare,
            &subject,
            share,
        )?;
        let statements = vec![
            self.statement(
                SHARE_UPSERT_SQL,
                &[
                    string(share.id),
                    string(share.video_id),
                    string(share.scope.organization_id),
                    optional_string(share.folder_id),
                    string(share.shared_by_user_id),
                    JsValue::from_str(share.mode.stable_code()),
                    timestamp(share.shared_at),
                    optional_timestamp(share.revoked_at),
                    number(resulting.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                SHARE_EXACT_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:share", context.operation_id)),
                    string(share.id),
                    string(share.video_id),
                    string(share.scope.organization_id),
                    optional_string(share.folder_id),
                    string(share.shared_by_user_id),
                    JsValue::from_str(share.mode.stable_code()),
                    timestamp(share.shared_at),
                    optional_timestamp(share.revoked_at),
                    number(resulting.get())?,
                    string(context.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            Some(share.video_id),
            statements,
        )
        .await
    }

    async fn put_comment_inner(
        &self,
        command: PutCommentCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let comment = &command.comment;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        if comment.scope != context.scope || comment.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        comment.validate().map_err(|_| AdapterFailure::Invalid)?;
        let author_matches = match (&context.principal, &comment.author) {
            (BusinessPrincipal::Authenticated(principal), CommentAuthor::User(author)) => {
                principal == author
            }
            (BusinessPrincipal::Anonymous(principal), CommentAuthor::Anonymous(author)) => {
                principal.as_str() == author.expose_for_verification()
            }
            _ => false,
        };
        if !author_matches {
            return Err(AdapterFailure::AccessDenied);
        }
        let (author_user, anonymous_digest) = match &comment.author {
            CommentAuthor::User(user_id) => (string(*user_id), JsValue::NULL),
            CommentAuthor::Anonymous(digest) => (
                JsValue::NULL,
                JsValue::from_str(digest.expose_for_verification()),
            ),
        };
        let subject = comment.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::CreateComment,
            &subject,
            comment,
        )?;
        let statements = vec![
            self.statement(
                COMMENT_INSERT_SQL,
                &[
                    string(comment.id),
                    string(comment.video_id),
                    optional_string(comment.parent_comment_id),
                    author_user,
                    anonymous_digest,
                    JsValue::from_str(comment.body.as_str()),
                    timestamp(comment.created_at),
                    timestamp(comment.updated_at),
                    optional_timestamp(comment.deleted_at),
                    number(resulting.get())?,
                    string(comment.scope.organization_id),
                    string(context.operation_id),
                    JsValue::from_str(comment.kind.stable_code()),
                    comment
                        .timeline_micros
                        .map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64)),
                ],
            )?,
            self.postcondition_statement(context, "comment", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Created,
            resulting,
            role_class(context),
            Some(comment.video_id),
            statements,
        )
        .await
    }

    async fn list_comments_inner(
        &self,
        request: BusinessReadRequest,
    ) -> AdapterResult<Vec<frame_domain::BusinessCommentRecord>> {
        let video_id = request.video_id.ok_or(AdapterFailure::Invalid)?;
        let operation_id = BusinessOperationId::new();
        let results = self
            .batch_results(vec![
                self.read_authority_statement(&request, operation_id)?,
                self.statement(
                    COMMENT_LIST_SQL,
                    &[string(request.scope.organization_id), string(video_id)],
                )?,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let rows = Self::result_rows::<CommentRow>(&results[1])?;
        if rows.len() > 1000 {
            return Err(AdapterFailure::Corrupt);
        }
        rows.into_iter()
            .map(|row| decode_comment(row, request.scope))
            .collect()
    }

    async fn delete_comment_inner(
        &self,
        command: DeleteCommentCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        if command.expected_revision != context.authority_fence.resource_revision {
            return Err(AdapterFailure::Invalid);
        }
        let subject = command.comment_id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::DeleteComment,
            &subject,
            &(
                command.comment_id,
                command.video_id,
                command.deleted_at,
                command.expected_revision,
            ),
        )?;
        let resulting = command
            .expected_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        let statements = vec![
            self.statement(
                COMMENT_DELETE_SQL,
                &[
                    string(command.comment_id),
                    string(context.scope.organization_id),
                    string(command.video_id),
                    timestamp(command.deleted_at),
                    number(command.expected_revision.get())?,
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                COMMENT_DELETE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:comment-delete", context.operation_id)),
                    string(command.comment_id),
                    string(context.scope.organization_id),
                    string(command.video_id),
                    timestamp(command.deleted_at),
                    number(resulting.get())?,
                    string(context.operation_id),
                ],
            )?,
        ];
        let authority_resource = subject
            .parse::<VideoId>()
            .map_err(|_| AdapterFailure::Invalid)?;
        self.commit(
            context,
            subject,
            BusinessMutationResult::Tombstoned,
            resulting,
            role_class(context),
            Some(authority_resource),
            statements,
        )
        .await
    }

    async fn enqueue_notification_inner(
        &self,
        command: EnqueueNotificationCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let notification = &command.notification;
        let outbox = &command.outbox;
        if notification.scope != context.scope || outbox.scope != context.scope {
            return Err(AdapterFailure::Invalid);
        }
        notification
            .validate()
            .map_err(|_| AdapterFailure::Invalid)?;
        outbox.validate().map_err(|_| AdapterFailure::Invalid)?;
        let subject = notification.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageNotification,
            &subject,
            &(notification, outbox),
        )?;
        let statements = vec![
            self.statement(
                NOTIFICATION_INSERT_SQL,
                &[
                    string(notification.id),
                    string(notification.scope.organization_id),
                    string(notification.recipient_user_id),
                    JsValue::from_str(notification.kind.stable_code()),
                    JsValue::from_str(notification.deduplication_key.expose()),
                    JsValue::from_str(notification.payload.canonical_json()),
                    timestamp(notification.created_at),
                    optional_timestamp(notification.read_at),
                    number(u64::from(notification.payload.schema_version()))?,
                    JsValue::from_str(notification.payload.checksum().as_str()),
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                OUTBOX_INSERT_SQL,
                &[
                    string(outbox.id),
                    string(outbox.scope.organization_id),
                    JsValue::from_str(outbox.aggregate_kind.as_str()),
                    JsValue::from_str(&outbox.aggregate_id),
                    JsValue::from_str(outbox.event_type.as_str()),
                    JsValue::from_str(
                        tenant_scoped_idempotency_digest(
                            outbox.scope,
                            "outbox",
                            &outbox.deduplication_key,
                        )
                        .as_str(),
                    ),
                    JsValue::from_str(outbox.payload.canonical_json()),
                    JsValue::from_str(delivery_code(outbox.lifecycle.state)),
                    timestamp(outbox.available_at),
                    timestamp(outbox.created_at),
                    number(outbox.lifecycle.last_sequence.get())?,
                    JsValue::from_str(outbox.lifecycle.last_fingerprint.as_str()),
                    number(u64::from(outbox.payload.schema_version()))?,
                    JsValue::from_str(outbox.payload.checksum().as_str()),
                    number(0)?,
                    string(context.operation_id),
                ],
            )?,
            self.postcondition_statement(
                context,
                "notification",
                &subject,
                BusinessRevision::INITIAL,
            )?,
            self.postcondition_statement(
                context,
                "outbox",
                &outbox.id.to_string(),
                BusinessRevision::INITIAL,
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Created,
            BusinessRevision::INITIAL,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn list_notifications_inner(
        &self,
        request: BusinessReadRequest,
    ) -> AdapterResult<Vec<frame_domain::NotificationRecord>> {
        let recipient = match &request.principal {
            BusinessPrincipal::Authenticated(user_id) => *user_id,
            BusinessPrincipal::Anonymous(_) => return Err(AdapterFailure::AccessDenied),
        };
        let operation_id = BusinessOperationId::new();
        let results = self
            .batch_results(vec![
                self.read_authority_statement(&request, operation_id)?,
                self.statement(
                    NOTIFICATION_LIST_SQL,
                    &[string(request.scope.organization_id), string(recipient)],
                )?,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let rows = Self::result_rows::<NotificationRow>(&results[1])?;
        if rows.len() > 1000 {
            return Err(AdapterFailure::Corrupt);
        }
        rows.into_iter()
            .map(|row| decode_notification(row, request.scope))
            .collect()
    }

    async fn mark_notification_read_inner(
        &self,
        command: MarkNotificationReadCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let recipient = match &context.principal {
            BusinessPrincipal::Authenticated(user_id) => *user_id,
            BusinessPrincipal::Anonymous(_) => return Err(AdapterFailure::AccessDenied),
        };
        let subject = command.notification_id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ReadNotification,
            &subject,
            &(command.notification_id, command.read_at),
        )?;
        let statements = vec![
            self.statement(
                NOTIFICATION_MARK_READ_SQL,
                &[
                    string(command.notification_id),
                    string(context.scope.organization_id),
                    string(recipient),
                    timestamp(command.read_at),
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                NOTIFICATION_MARK_READ_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:notification-read", context.operation_id)),
                    string(command.notification_id),
                    string(context.scope.organization_id),
                    string(recipient),
                    timestamp(command.read_at),
                    string(context.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            BusinessRevision::INITIAL,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn advance_outbox_inner(
        &self,
        command: AdvanceOutboxCommand,
    ) -> AdapterResult<(BusinessMutationReceipt, OrderedEventResult)> {
        let context = &command.context;
        let subject = command.outbox_id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageNotification,
            &subject,
            &(
                command.event_sequence,
                &command.event_fingerprint,
                command.target,
            ),
        )?;
        let row = self
            .lifecycle_row(context, None, OUTBOX_LIFECYCLE_SQL, &subject)
            .await?;
        let mut lifecycle = OrderedDeliveryLifecycle {
            state: parse_delivery(&row.state)?,
            last_sequence: safe_revision(row.event_sequence)?,
            last_fingerprint: optional_checksum(row.event_fingerprint)?,
        };
        let disposition = lifecycle
            .apply(
                command.event_sequence,
                command.event_fingerprint.clone(),
                command.target,
            )
            .map_err(|error| match error {
                frame_domain::BusinessContractError::ConflictingReplay => AdapterFailure::Conflict,
                _ => AdapterFailure::Invalid,
            })?;
        let resulting = if disposition == OrderedEventResult::Applied {
            safe_revision(row.revision)?
                .next()
                .map_err(|_| AdapterFailure::Invalid)?
        } else {
            safe_revision(row.revision)?
        };
        let result = event_result(disposition);
        let lease_expires = context
            .occurred_at
            .get()
            .checked_add(60_000)
            .ok_or(AdapterFailure::Invalid)?;
        let mut statements = vec![
            self.statement(
                EVENT_INBOX_INSERT_SQL,
                &[
                    string(context.scope.organization_id),
                    JsValue::from_str("outbox"),
                    JsValue::from_str(&subject),
                    number(command.event_sequence.get())?,
                    JsValue::from_str(command.event_fingerprint.as_str()),
                    JsValue::from_str(delivery_code(command.target)),
                    number(safe_revision(row.event_sequence)?.get())?,
                    timestamp(context.occurred_at),
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                OUTBOX_ADVANCE_SQL,
                &[
                    string(command.outbox_id),
                    string(context.scope.organization_id),
                    number(command.event_sequence.get())?,
                    JsValue::from_str(delivery_code(command.target)),
                    JsValue::from_str(command.event_fingerprint.as_str()),
                    if command.target == DeliveryState::Leased {
                        JsValue::from_f64(lease_expires as f64)
                    } else {
                        JsValue::NULL
                    },
                    timestamp(context.occurred_at),
                    string(context.operation_id),
                ],
            )?,
        ];
        if disposition == OrderedEventResult::Applied {
            statements.push(self.postcondition_statement(context, "outbox", &subject, resulting)?);
        }
        let receipt = self
            .commit_ordered(
                context,
                subject,
                result,
                resulting,
                role_class(context),
                None,
                statements,
            )
            .await?;
        Ok((receipt, disposition))
    }

    async fn advance_upload_inner(
        &self,
        command: AdvanceUploadCommand,
    ) -> AdapterResult<(BusinessMutationReceipt, OrderedEventResult)> {
        let context = &command.context;
        let upload = &command.upload;
        if upload.scope != context.scope
            || command.received_bytes > upload.expected_bytes
            || (command.target == UploadState::Complete) != command.checksum.is_some()
        {
            return Err(AdapterFailure::Invalid);
        }
        upload
            .validate_initial()
            .map_err(|_| AdapterFailure::Invalid)?;
        let subject = upload.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageUpload,
            &subject,
            &(
                upload,
                command.event_sequence,
                &command.event_fingerprint,
                command.target,
                command.received_bytes,
                &command.checksum,
            ),
        )?;
        let existing = self
            .lifecycle_row(
                context,
                Some(upload.video_id),
                UPLOAD_LIFECYCLE_SQL,
                &subject,
            )
            .await;
        let (current, current_revision, include_insert) = match existing {
            Ok(row) => (
                OrderedUploadLifecycle {
                    state: parse_upload(&row.state)?,
                    last_sequence: safe_revision(row.event_sequence)?,
                    last_fingerprint: optional_checksum(row.event_fingerprint)?,
                },
                safe_revision(row.revision)?,
                false,
            ),
            Err(AdapterFailure::AccessDenied) => {
                (upload.lifecycle.clone(), BusinessRevision::INITIAL, true)
            }
            Err(error) => return Err(error),
        };
        let current_sequence = current.last_sequence;
        let mut lifecycle = current;
        let disposition = lifecycle
            .apply(
                command.event_sequence,
                command.event_fingerprint.clone(),
                command.target,
            )
            .map_err(|error| match error {
                frame_domain::BusinessContractError::ConflictingReplay => AdapterFailure::Conflict,
                _ => AdapterFailure::Invalid,
            })?;
        let resulting = if disposition == OrderedEventResult::Applied {
            current_revision
                .next()
                .map_err(|_| AdapterFailure::Invalid)?
        } else {
            current_revision
        };
        let scoped_key =
            tenant_scoped_idempotency_digest(upload.scope, "upload", &upload.idempotency_key);
        let mut statements = Vec::new();
        if include_insert {
            statements.push(self.statement(
                UPLOAD_INSERT_SQL,
                &[
                    string(upload.id),
                    string(upload.scope.organization_id),
                    string(upload.video_id),
                    number(upload.expected_bytes.get())?,
                    JsValue::from_str(scoped_key.as_str()),
                    JsValue::from_str(upload.source_object_key.as_str()),
                    number(upload.source_version.get())?,
                    JsValue::from_str(upload.content_type.as_str()),
                    timestamp(upload.created_at),
                    string(context.operation_id),
                ],
            )?);
        }
        statements.push(self.statement(
            UPLOAD_IMMUTABLE_ASSERT_SQL,
            &[
                string(format!("{}:upload-immutable", context.operation_id)),
                string(upload.id),
                string(upload.scope.organization_id),
                string(upload.video_id),
                number(upload.expected_bytes.get())?,
                JsValue::from_str(scoped_key.as_str()),
                JsValue::from_str(upload.source_object_key.as_str()),
                number(upload.source_version.get())?,
                JsValue::from_str(upload.content_type.as_str()),
                timestamp(upload.created_at),
            ],
        )?);
        statements.push(self.statement(
            EVENT_INBOX_INSERT_SQL,
            &[
                string(context.scope.organization_id),
                JsValue::from_str("upload"),
                JsValue::from_str(&subject),
                number(command.event_sequence.get())?,
                JsValue::from_str(command.event_fingerprint.as_str()),
                JsValue::from_str(upload_code(command.target)),
                number(current_sequence.get())?,
                timestamp(context.occurred_at),
                string(context.operation_id),
            ],
        )?);
        statements.push(self.statement(
            UPLOAD_ADVANCE_SQL,
            &[
                string(upload.id),
                string(upload.scope.organization_id),
                number(command.event_sequence.get())?,
                JsValue::from_str(upload_code(command.target)),
                number(command.received_bytes.get())?,
                optional_checksum_value(command.checksum.as_ref()),
                JsValue::from_str(command.event_fingerprint.as_str()),
                timestamp(upload.updated_at),
                string(context.operation_id),
            ],
        )?);
        if disposition == OrderedEventResult::Applied {
            statements.push(self.statement(
                UPLOAD_EXACT_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:upload", context.operation_id)),
                    string(upload.id),
                    string(upload.scope.organization_id),
                    JsValue::from_str(upload_code(command.target)),
                    number(command.received_bytes.get())?,
                    optional_checksum_value(command.checksum.as_ref()),
                    number(command.event_sequence.get())?,
                    JsValue::from_str(command.event_fingerprint.as_str()),
                    timestamp(upload.updated_at),
                    number(resulting.get())?,
                    string(context.operation_id),
                ],
            )?);
        }
        let receipt = self
            .commit_ordered(
                context,
                subject,
                event_result(disposition),
                resulting,
                role_class(context),
                Some(upload.video_id),
                statements,
            )
            .await?;
        Ok((receipt, disposition))
    }

    async fn put_storage_object_inner(
        &self,
        command: PutStorageObjectCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let object = &command.object;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        object.validate().map_err(|_| AdapterFailure::Invalid)?;
        if object.scope != context.scope {
            return Err(AdapterFailure::Invalid);
        }
        let subject = object.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageStorage,
            &subject,
            object,
        )?;
        let statements = vec![
            self.statement(
                STORAGE_OBJECT_UPSERT_SQL,
                &[
                    string(object.id),
                    string(object.scope.organization_id),
                    string(object.integration_id),
                    optional_string(object.video_id),
                    JsValue::from_str(object.object_key.as_str()),
                    JsValue::from_str(object_role_code(object.role)),
                    number(object.object_version.get())?,
                    JsValue::from_str(storage_state_code(object.state)),
                    number(object.bytes.get())?,
                    JsValue::from_str(object.content_type.as_str()),
                    JsValue::from_str(object.checksum.as_str()),
                    timestamp(object.created_at),
                    if object.state == StorageObjectState::Deleted {
                        timestamp(object.updated_at)
                    } else {
                        JsValue::NULL
                    },
                    timestamp(object.updated_at),
                    number(resulting.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                STORAGE_OBJECT_EXACT_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:storage", context.operation_id)),
                    string(object.id),
                    string(object.scope.organization_id),
                    string(object.integration_id),
                    optional_string(object.video_id),
                    JsValue::from_str(object.object_key.as_str()),
                    JsValue::from_str(object_role_code(object.role)),
                    number(object.object_version.get())?,
                    JsValue::from_str(storage_state_code(object.state)),
                    number(object.bytes.get())?,
                    JsValue::from_str(object.content_type.as_str()),
                    JsValue::from_str(object.checksum.as_str()),
                    timestamp(object.created_at),
                    if object.state == StorageObjectState::Deleted {
                        timestamp(object.updated_at)
                    } else {
                        JsValue::NULL
                    },
                    timestamp(object.updated_at),
                    number(resulting.get())?,
                    string(context.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            object.video_id,
            statements,
        )
        .await
    }

    async fn put_storage_integration_inner(
        &self,
        command: PutStorageIntegrationCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let integration = &command.integration;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        integration
            .validate()
            .map_err(|_| AdapterFailure::Invalid)?;
        if integration.scope != context.scope
            || integration.revision != resulting
            || integration.authority_version != resulting
        {
            return Err(AdapterFailure::Invalid);
        }
        let subject = integration.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageStorage,
            &subject,
            integration,
        )?;
        let statements = vec![
            self.statement(
                STORAGE_INTEGRATION_UPSERT_SQL,
                &[
                    string(integration.id),
                    string(integration.scope.organization_id),
                    optional_string(integration.owner_user_id),
                    JsValue::from_str(integration.provider.stable_code()),
                    JsValue::from_str(integration.state.stable_code()),
                    JsValue::from_str(integration.capabilities.canonical_json()),
                    JsValue::from_str(integration.encrypted_config.expose_to_provider_adapter()),
                    timestamp(integration.created_at),
                    timestamp(integration.updated_at),
                    number(integration.revision.get())?,
                    number(integration.authority_version.get())?,
                    string(context.operation_id),
                    number(u64::from(integration.capabilities.schema_version()))?,
                    JsValue::from_str(integration.capabilities.checksum().as_str()),
                    number(context.authority_fence.resource_revision.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                ],
            )?,
            self.postcondition_statement(context, "storage_integration", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn put_derivative_job_inner(
        &self,
        command: PutDerivativeJobCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let job = &command.job;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        job.validate().map_err(|_| AdapterFailure::Invalid)?;
        if job.scope != context.scope || job.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        let subject = job.job_id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageStorage,
            &subject,
            job,
        )?;
        let statements = vec![
            self.statement(
                DERIVATIVE_MANIFEST_UPSERT_SQL,
                &[
                    string(job.job_id),
                    string(job.scope.organization_id),
                    JsValue::from_str(executor_code(job.executor)),
                    string(job.source_object_id),
                    number(job.source_version.get())?,
                    JsValue::from_str(&job.transform_profile),
                    number(job.profile_version.get())?,
                    JsValue::from_str(object_role_code(job.output_role)),
                    optional_string(job.output_object_id),
                    JsValue::from_str(job.output_key.as_str()),
                    optional_checksum_value(job.output_checksum.as_ref()),
                    JsValue::from_str(job.output_content_type.as_str()),
                    JsValue::from_str(derivative_state_code(job.state)),
                    number(job.usage_units)?,
                    number(job.cost_microcredits)?,
                    job.failure_class.as_ref().map_or(JsValue::NULL, |failure| {
                        JsValue::from_str(failure.stable_code())
                    }),
                    number(resulting.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                ],
            )?,
            self.statement(
                DERIVATIVE_EXACT_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:derivative", context.operation_id)),
                    string(job.job_id),
                    string(job.scope.organization_id),
                    JsValue::from_str(executor_code(job.executor)),
                    string(job.source_object_id),
                    number(job.source_version.get())?,
                    JsValue::from_str(&job.transform_profile),
                    number(job.profile_version.get())?,
                    JsValue::from_str(object_role_code(job.output_role)),
                    optional_string(job.output_object_id),
                    JsValue::from_str(job.output_key.as_str()),
                    optional_checksum_value(job.output_checksum.as_ref()),
                    JsValue::from_str(job.output_content_type.as_str()),
                    JsValue::from_str(derivative_state_code(job.state)),
                    number(job.usage_units)?,
                    number(job.cost_microcredits)?,
                    job.failure_class.as_ref().map_or(JsValue::NULL, |failure| {
                        JsValue::from_str(failure.stable_code())
                    }),
                    number(resulting.get())?,
                    string(context.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn advance_import_inner(
        &self,
        command: AdvanceImportCommand,
    ) -> AdapterResult<(BusinessMutationReceipt, OrderedEventResult)> {
        let context = &command.context;
        let import = &command.import;
        if import.scope != context.scope
            || (command.target == ImportState::Failed) != command.error_class.is_some()
        {
            return Err(AdapterFailure::Invalid);
        }
        import
            .validate_initial()
            .map_err(|_| AdapterFailure::Invalid)?;
        let subject = import.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageImport,
            &subject,
            &(
                import,
                command.event_sequence,
                &command.event_fingerprint,
                command.target,
                &command.error_class,
            ),
        )?;
        let existing = self
            .lifecycle_row(context, import.video_id, IMPORT_LIFECYCLE_SQL, &subject)
            .await;
        let (current, current_revision, include_insert) = match existing {
            Ok(row) => (
                OrderedImportLifecycle {
                    state: parse_import(&row.state)?,
                    last_sequence: safe_revision(row.event_sequence)?,
                    last_fingerprint: optional_checksum(row.event_fingerprint)?,
                },
                safe_revision(row.revision)?,
                false,
            ),
            Err(AdapterFailure::AccessDenied) => {
                (import.lifecycle.clone(), BusinessRevision::INITIAL, true)
            }
            Err(error) => return Err(error),
        };
        let current_sequence = current.last_sequence;
        let mut lifecycle = current;
        let disposition = lifecycle
            .apply(
                command.event_sequence,
                command.event_fingerprint.clone(),
                command.target,
            )
            .map_err(|error| match error {
                frame_domain::BusinessContractError::ConflictingReplay => AdapterFailure::Conflict,
                _ => AdapterFailure::Invalid,
            })?;
        let resulting = if disposition == OrderedEventResult::Applied {
            current_revision
                .next()
                .map_err(|_| AdapterFailure::Invalid)?
        } else {
            current_revision
        };
        let mut statements = Vec::new();
        if include_insert {
            statements.push(self.statement(
                IMPORT_UPSERT_SQL,
                &[
                    string(import.id),
                    string(import.scope.organization_id),
                    optional_string(import.video_id),
                    JsValue::from_str(import.source.stable_code()),
                    JsValue::from_str(import.external_id_digest.expose_for_verification()),
                    JsValue::from_str(import_code(ImportState::Queued)),
                    JsValue::from_str(import.idempotency_key.expose()),
                    JsValue::NULL,
                    timestamp(import.created_at),
                    timestamp(import.updated_at),
                    number(BusinessRevision::INITIAL.get())?,
                    JsValue::from_str(frame_domain::business_initial_event_fingerprint().as_str()),
                    number(0)?,
                    string(context.operation_id),
                ],
            )?);
        }
        statements.push(self.statement(
            IMPORT_IMMUTABLE_ASSERT_SQL,
            &[
                string(format!("{}:import-immutable", context.operation_id)),
                string(import.id),
                string(import.scope.organization_id),
                optional_string(import.video_id),
                JsValue::from_str(import.source.stable_code()),
                JsValue::from_str(import.external_id_digest.expose_for_verification()),
                JsValue::from_str(import.idempotency_key.expose()),
                timestamp(import.created_at),
            ],
        )?);
        statements.push(self.statement(
            EVENT_INBOX_INSERT_SQL,
            &[
                string(context.scope.organization_id),
                JsValue::from_str("import"),
                JsValue::from_str(&subject),
                number(command.event_sequence.get())?,
                JsValue::from_str(command.event_fingerprint.as_str()),
                JsValue::from_str(import_code(command.target)),
                number(current_sequence.get())?,
                timestamp(context.occurred_at),
                string(context.operation_id),
            ],
        )?);
        statements.push(
            self.statement(
                IMPORT_ADVANCE_SQL,
                &[
                    string(import.id),
                    string(context.scope.organization_id),
                    number(command.event_sequence.get())?,
                    JsValue::from_str(import_code(command.target)),
                    JsValue::from_str(command.event_fingerprint.as_str()),
                    timestamp(import.updated_at),
                    command
                        .error_class
                        .as_ref()
                        .map_or(JsValue::NULL, |failure| {
                            JsValue::from_str(failure.stable_code())
                        }),
                    string(context.operation_id),
                ],
            )?,
        );
        if disposition == OrderedEventResult::Applied {
            statements.push(self.postcondition_statement(context, "import", &subject, resulting)?);
        }
        let receipt = self
            .commit_ordered(
                context,
                subject,
                event_result(disposition),
                resulting,
                role_class(context),
                import.video_id,
                statements,
            )
            .await?;
        Ok((receipt, disposition))
    }

    async fn put_developer_key_inner(
        &self,
        command: PutDeveloperApiKeyCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let key = &command.key;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        key.validate().map_err(|_| AdapterFailure::Invalid)?;
        if key.scope != context.scope {
            return Err(AdapterFailure::Invalid);
        }
        let subject = key.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageDeveloper,
            &subject,
            key,
        )?;
        let statements = vec![
            self.statement(
                DEVELOPER_KEY_INSERT_SQL,
                &[
                    string(key.id),
                    string(key.app_id),
                    JsValue::from_str(key.key_digest.expose_for_verification()),
                    JsValue::from_str(developer_key_code(key.kind)),
                    JsValue::from_str(&key.display_prefix),
                    timestamp(key.created_at),
                    optional_timestamp(key.last_used_at),
                    optional_timestamp(key.revoked_at),
                    JsValue::from_str(&key.display_prefix),
                    number(resulting.get())?,
                    string(context.operation_id),
                    string(context.scope.organization_id),
                ],
            )?,
            self.postcondition_statement(context, "developer_key", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Created,
            resulting,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn put_developer_app_inner(
        &self,
        command: PutDeveloperAppCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let app = &command.app;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        app.validate().map_err(|_| AdapterFailure::Invalid)?;
        if app.scope != context.scope
            || app.revision != resulting
            || app.authority_version != resulting
        {
            return Err(AdapterFailure::Invalid);
        }
        let subject = app.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageDeveloper,
            &subject,
            app,
        )?;
        let statements = vec![
            self.statement(
                DEVELOPER_APP_UPSERT_SQL,
                &[
                    string(app.id),
                    string(app.owner_user_id),
                    string(app.scope.organization_id),
                    JsValue::from_str(app.name.as_str()),
                    JsValue::from_str(app.environment.stable_code()),
                    JsValue::from_str(app.state.stable_code()),
                    timestamp(app.created_at),
                    timestamp(app.updated_at),
                    optional_timestamp(app.deleted_at),
                    number(app.revision.get())?,
                    number(app.authority_version.get())?,
                    string(context.operation_id),
                    number(context.authority_fence.resource_revision.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                ],
            )?,
            self.postcondition_statement(context, "developer_app", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn put_developer_domain_inner(
        &self,
        command: PutDeveloperDomainCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let domain = &command.domain;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        domain.validate().map_err(|_| AdapterFailure::Invalid)?;
        if domain.scope != context.scope || domain.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        let subject = format!("{}:{}", domain.app_id, domain.domain.as_str());
        validate_command(
            context,
            frame_domain::BusinessAction::ManageDeveloper,
            &subject,
            domain,
        )?;
        let statements = vec![
            self.statement(
                DEVELOPER_DOMAIN_UPSERT_SQL,
                &[
                    string(domain.app_id),
                    JsValue::from_str(domain.domain.as_str()),
                    timestamp(domain.created_at),
                    optional_timestamp(domain.verified_at),
                    number(domain.revision.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                    string(context.scope.organization_id),
                ],
            )?,
            self.postcondition_statement(context, "developer_domain", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn put_developer_video_inner(
        &self,
        command: PutDeveloperVideoCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let video = &command.video;
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        video.validate().map_err(|_| AdapterFailure::Invalid)?;
        if video.scope != context.scope || video.revision != resulting {
            return Err(AdapterFailure::Invalid);
        }
        let subject = video.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageDeveloper,
            &subject,
            video,
        )?;
        let metadata_json = video.metadata.as_ref().map_or(JsValue::NULL, |metadata| {
            JsValue::from_str(metadata.canonical_json())
        });
        let metadata_version = video
            .metadata
            .as_ref()
            .map_or(1_u64, |metadata| u64::from(metadata.schema_version()));
        let metadata_checksum = video.metadata.as_ref().map_or(JsValue::NULL, |metadata| {
            JsValue::from_str(metadata.checksum().as_str())
        });
        let statements = vec![
            self.statement(
                DEVELOPER_VIDEO_UPSERT_SQL,
                &[
                    string(video.id),
                    string(video.app_id),
                    optional_string(video.video_id),
                    JsValue::from_str(video.external_user_digest.expose_for_verification()),
                    metadata_json,
                    timestamp(video.created_at),
                    timestamp(video.updated_at),
                    optional_timestamp(video.deleted_at),
                    number(metadata_version)?,
                    metadata_checksum,
                    number(video.revision.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                    string(context.scope.organization_id),
                ],
            )?,
            self.postcondition_statement(context, "developer_video", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            video.video_id,
            statements,
        )
        .await
    }

    async fn append_credit_inner(
        &self,
        command: AppendCreditTransactionCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let transaction = &command.transaction;
        let mut account = command.expected_account;
        account
            .apply(transaction)
            .map_err(|_| AdapterFailure::Invalid)?;
        if transaction.scope != context.scope {
            return Err(AdapterFailure::Invalid);
        }
        let subject = transaction.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageLedger,
            &subject,
            transaction,
        )?;
        let statements = vec![
            self.statement(
                CREDIT_TRANSACTION_INSERT_SQL,
                &[
                    string(transaction.id),
                    string(transaction.account_id),
                    JsValue::from_str(credit_kind_code(transaction.kind)),
                    JsValue::from_f64(transaction.amount_microcredits as f64),
                    number(transaction.balance_after_microcredits)?,
                    JsValue::from_str(&transaction.reference_kind),
                    JsValue::from_str(transaction.reference_digest.as_str()),
                    JsValue::from_str(transaction.idempotency_key.expose()),
                    timestamp(transaction.occurred_at),
                    number(transaction.sequence.get())?,
                    JsValue::from_str(transaction.reference_digest.as_str()),
                    string(context.operation_id),
                    JsValue::from_str(context.request_fingerprint.as_str()),
                ],
            )?,
            self.postcondition_statement(
                context,
                "credit_transaction",
                &subject,
                transaction.sequence,
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            transaction.sequence,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn credit_account_inner(
        &self,
        request: BusinessReadRequest,
        account_id: frame_domain::CreditAccountId,
    ) -> AdapterResult<CreditAccountRecord> {
        let operation_id = BusinessOperationId::new();
        let results = self
            .batch_results(vec![
                self.export_authority_statement(&request, operation_id)?,
                self.statement(
                    CREDIT_ACCOUNT_SQL,
                    &[string(account_id), string(request.scope.organization_id)],
                )?,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let mut rows = Self::result_rows::<CreditAccountRow>(&results[1])?;
        if rows.len() != 1 {
            return Err(AdapterFailure::AccessDenied);
        }
        decode_credit_account(rows.pop().ok_or(AdapterFailure::Corrupt)?, request.scope)
    }

    async fn append_usage_inner(
        &self,
        command: AppendUsageCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let usage = &command.usage;
        usage.validate().map_err(|_| AdapterFailure::Invalid)?;
        if usage.scope != context.scope {
            return Err(AdapterFailure::Invalid);
        }
        let subject = usage.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageLedger,
            &subject,
            usage,
        )?;
        let statements = vec![
            self.statement(
                USAGE_INSERT_SQL,
                &[
                    string(usage.id),
                    string(usage.scope.organization_id),
                    optional_string(usage.app_id),
                    optional_string(usage.video_id),
                    optional_string(usage.media_job_id),
                    JsValue::from_str(usage_kind_code(usage.kind)),
                    number(usage.quantity)?,
                    number(usage.microcredits_charged)?,
                    JsValue::from_str(
                        tenant_scoped_idempotency_digest(
                            usage.scope,
                            "usage",
                            &usage.idempotency_key,
                        )
                        .as_str(),
                    ),
                    timestamp(usage.occurred_at),
                    timestamp(usage.recorded_at),
                    string(context.operation_id),
                    JsValue::from_str(context.request_fingerprint.as_str()),
                ],
            )?,
            self.postcondition_statement(context, "usage", &subject, BusinessRevision::INITIAL)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            BusinessRevision::INITIAL,
            role_class(context),
            usage.video_id,
            statements,
        )
        .await
    }

    async fn put_snapshot_inner(
        &self,
        command: PutDailyStorageSnapshotCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let snapshot = &command.snapshot;
        snapshot.validate().map_err(|_| AdapterFailure::Invalid)?;
        if snapshot.scope != context.scope {
            return Err(AdapterFailure::Invalid);
        }
        let resulting = context
            .authority_fence
            .resource_revision
            .next()
            .map_err(|_| AdapterFailure::Invalid)?;
        let subject = snapshot.app_id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageLedger,
            &subject,
            snapshot,
        )?;
        let statements = vec![
            self.statement(
                DAILY_SNAPSHOT_UPSERT_SQL,
                &[
                    string(snapshot.app_id),
                    JsValue::from_str(&snapshot.snapshot_day),
                    number(snapshot.total_bytes.get())?,
                    number(snapshot.microcredits_charged)?,
                    JsValue::from_str(snapshot.source_checksum.as_str()),
                    optional_timestamp(snapshot.processed_at),
                    timestamp(snapshot.created_at),
                    number(resulting.get())?,
                    number(context.authority_fence.resource_revision.get())?,
                    string(context.operation_id),
                    string(context.scope.organization_id),
                ],
            )?,
            self.postcondition_statement(context, "daily_snapshot", &subject, resulting)?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Applied,
            resulting,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn list_legal_holds_inner(
        &self,
        request: BusinessReadRequest,
    ) -> AdapterResult<Vec<BusinessLegalHoldRecord>> {
        let operation_id = BusinessOperationId::new();
        let results = self
            .batch_results(vec![
                self.export_authority_statement(&request, operation_id)?,
                self.statement(
                    LEGAL_HOLD_LIST_SQL,
                    &[string(request.scope.organization_id)],
                )?,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let rows = Self::result_rows::<LegalHoldRow>(&results[1])?;
        if rows.len() > 1000 {
            return Err(AdapterFailure::Corrupt);
        }
        rows.into_iter()
            .map(|row| decode_legal_hold(row, request.scope))
            .collect()
    }

    async fn place_legal_hold_inner(
        &self,
        command: PlaceLegalHoldCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let hold = &command.hold;
        if hold.scope != context.scope
            || !matches!(
                context.principal,
                BusinessPrincipal::Authenticated(user_id) if user_id == hold.placed_by_user_id
            )
        {
            return Err(AdapterFailure::AccessDenied);
        }
        hold.validate().map_err(|_| AdapterFailure::Invalid)?;
        let subject = hold.id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageLegalHold,
            &subject,
            hold,
        )?;
        let data_class = data_class_code(hold.data_class);
        let statements = vec![
            self.statement(
                DATA_SUBJECT_ASSERT_SQL,
                &[
                    string(format!("{}:hold-subject", context.operation_id)),
                    string(context.scope.organization_id),
                    JsValue::from_str(data_class),
                    JsValue::from_str(&hold.subject_id),
                ],
            )?,
            self.statement(
                LEGAL_HOLD_INSERT_SQL,
                &[
                    string(hold.id),
                    string(hold.scope.organization_id),
                    JsValue::from_str(data_class),
                    JsValue::from_str(&hold.subject_id),
                    JsValue::from_str(hold.reason_code.as_str()),
                    string(hold.placed_by_user_id),
                    timestamp(hold.placed_at),
                ],
            )?,
            self.statement(
                LEGAL_HOLD_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:legal-hold", context.operation_id)),
                    string(hold.id),
                    string(hold.scope.organization_id),
                    JsValue::from_str(data_class),
                    JsValue::from_str(&hold.subject_id),
                    JsValue::from_str(hold.reason_code.as_str()),
                    string(hold.placed_by_user_id),
                    timestamp(hold.placed_at),
                    JsValue::NULL,
                ],
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Created,
            BusinessRevision::INITIAL,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn release_legal_hold_inner(
        &self,
        command: ReleaseLegalHoldCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let subject = command.hold_id.to_string();
        validate_command(
            context,
            frame_domain::BusinessAction::ManageLegalHold,
            &subject,
            &(command.hold_id, command.released_at),
        )?;
        let statements = vec![
            self.statement(
                LEGAL_HOLD_RELEASE_SQL,
                &[
                    string(command.hold_id),
                    string(context.scope.organization_id),
                    timestamp(command.released_at),
                ],
            )?,
            self.statement(
                LEGAL_HOLD_RELEASE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post:legal-hold-release", context.operation_id)),
                    string(command.hold_id),
                    string(context.scope.organization_id),
                    timestamp(command.released_at),
                ],
            )?,
        ];
        self.commit(
            context,
            subject,
            BusinessMutationResult::Revoked,
            BusinessRevision::INITIAL,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn handle_data_inner(
        &self,
        command: DataHandlingCommand,
    ) -> AdapterResult<BusinessMutationReceipt> {
        let context = &command.context;
        let request_kind = match context.action {
            frame_domain::BusinessAction::ExportData => "export",
            frame_domain::BusinessAction::DeleteData => "delete",
            _ => return Err(AdapterFailure::Invalid),
        };
        let disposition = match command.decision {
            frame_domain::RetentionDecision::Export => "scheduled",
            frame_domain::RetentionDecision::Delete(
                frame_domain::DeletionMode::AppendCompensatingEntry,
            ) => "compensated",
            frame_domain::RetentionDecision::Delete(
                frame_domain::DeletionMode::ExcludedQuarantine,
            ) => "quarantined",
            frame_domain::RetentionDecision::Delete(_) => "completed",
            frame_domain::RetentionDecision::Denied => return Err(AdapterFailure::Retention),
        };
        let data_class = data_class_code(command.data_class);
        let subject = command.subject_id.clone();
        let needs_compensation = matches!(
            command.decision,
            frame_domain::RetentionDecision::Delete(
                frame_domain::DeletionMode::AppendCompensatingEntry
            )
        );
        if needs_compensation != command.compensation.is_some() {
            return Err(AdapterFailure::Invalid);
        }
        let compensation_checksum = command
            .compensation
            .as_ref()
            .map(|compensation| {
                business_payload_checksum(&(
                    compensation.expected_account,
                    &compensation.transaction,
                ))
                .map_err(|_| AdapterFailure::Invalid)
            })
            .transpose()?;
        validate_command(
            context,
            context.action,
            &subject,
            &(
                command.data_class,
                command.decision,
                command.subject_id.as_str(),
                compensation_checksum.as_ref().map(ChecksumSha256::as_str),
            ),
        )?;
        let mut statements = vec![
            self.statement(
                DATA_SUBJECT_ASSERT_SQL,
                &[
                    string(format!("{}:subject", context.operation_id)),
                    string(context.scope.organization_id),
                    JsValue::from_str(data_class),
                    JsValue::from_str(&subject),
                ],
            )?,
            self.statement(
                RETENTION_ASSERT_SQL,
                &[
                    string(format!("{}:retention", context.operation_id)),
                    string(context.scope.organization_id),
                    JsValue::from_str(data_class),
                    JsValue::from_str(request_kind),
                    JsValue::from_str(&subject),
                ],
            )?,
        ];
        let result = match command.decision {
            frame_domain::RetentionDecision::Export => BusinessMutationResult::Accepted,
            frame_domain::RetentionDecision::Delete(
                frame_domain::DeletionMode::AppendCompensatingEntry,
            ) => {
                let compensation = command
                    .compensation
                    .as_ref()
                    .ok_or(AdapterFailure::Invalid)?;
                let transaction = &compensation.transaction;
                let mut expected = compensation.expected_account;
                expected
                    .apply(transaction)
                    .map_err(|_| AdapterFailure::Invalid)?;
                if transaction.scope != context.scope
                    || transaction.kind != CreditTransactionKind::Adjustment
                    || transaction.reference_kind != "data_deletion_compensation"
                    || transaction.reference_digest
                        != frame_domain::deletion_compensation_reference(
                            command.data_class,
                            &subject,
                        )
                {
                    return Err(AdapterFailure::Invalid);
                }
                statements.push(self.statement(
                    LEDGER_COMPENSATION_ASSERT_SQL,
                    &[
                        string(format!("{}:compensation", context.operation_id)),
                        string(context.scope.organization_id),
                        JsValue::from_str(data_class),
                        JsValue::from_str(&subject),
                        string(transaction.id),
                        string(transaction.account_id),
                        number(transaction.sequence.get())?,
                        JsValue::from_f64(transaction.amount_microcredits as f64),
                        number(transaction.balance_after_microcredits)?,
                        JsValue::from_str(credit_kind_code(transaction.kind)),
                        JsValue::from_str(&transaction.reference_kind),
                        JsValue::from_str(transaction.reference_digest.as_str()),
                    ],
                )?);
                statements.push(self.statement(
                    CREDIT_TRANSACTION_INSERT_SQL,
                    &[
                        string(transaction.id),
                        string(transaction.account_id),
                        JsValue::from_str(credit_kind_code(transaction.kind)),
                        JsValue::from_f64(transaction.amount_microcredits as f64),
                        number(transaction.balance_after_microcredits)?,
                        JsValue::from_str(&transaction.reference_kind),
                        JsValue::from_str(transaction.reference_digest.as_str()),
                        JsValue::from_str(transaction.idempotency_key.expose()),
                        timestamp(transaction.occurred_at),
                        number(transaction.sequence.get())?,
                        JsValue::from_str(transaction.reference_digest.as_str()),
                        string(context.operation_id),
                        JsValue::from_str(context.request_fingerprint.as_str()),
                    ],
                )?);
                statements.push(self.postcondition_statement(
                    context,
                    "credit_transaction",
                    &transaction.id.to_string(),
                    transaction.sequence,
                )?);
                BusinessMutationResult::Applied
            }
            frame_domain::RetentionDecision::Delete(mode) => {
                let delete_sql = data_delete_sql(command.data_class);
                if command.data_class == BusinessDataClass::MessengerLegacy {
                    let messenger_parameters = [
                        JsValue::from_str(&subject),
                        string(context.scope.organization_id),
                        timestamp(context.occurred_at),
                    ];
                    statements.push(
                        self.statement(DELETE_MESSENGER_SUPPORT_EMAIL_SQL, &messenger_parameters)?,
                    );
                    statements
                        .push(self.statement(DELETE_MESSENGER_MESSAGE_SQL, &messenger_parameters)?);
                    statements.push(
                        self.statement(DELETE_MESSENGER_CONVERSATION_SQL, &messenger_parameters)?,
                    );
                }
                if let Some(sql) = delete_sql {
                    statements.push(self.statement(
                        sql,
                        &[
                            JsValue::from_str(&subject),
                            string(context.scope.organization_id),
                            timestamp(context.occurred_at),
                            string(context.operation_id),
                        ],
                    )?);
                } else if !matches!(
                    mode,
                    frame_domain::DeletionMode::RetainAuditOnly
                        | frame_domain::DeletionMode::ExcludedQuarantine
                ) {
                    return Err(AdapterFailure::Invalid);
                }
                if mode != frame_domain::DeletionMode::ExcludedQuarantine || delete_sql.is_some() {
                    statements.push(self.statement(
                        DATA_DELETE_POSTCONDITION_SQL,
                        &[
                            string(format!("{}:post:data-delete", context.operation_id)),
                            string(context.scope.organization_id),
                            JsValue::from_str(data_class),
                            JsValue::from_str(&subject),
                            timestamp(context.occurred_at),
                            string(context.operation_id),
                        ],
                    )?);
                }
                match mode {
                    frame_domain::DeletionMode::TombstoneThenPurge => {
                        BusinessMutationResult::Tombstoned
                    }
                    frame_domain::DeletionMode::CryptographicEraseThenPurge => {
                        BusinessMutationResult::Purged
                    }
                    frame_domain::DeletionMode::RetainAuditOnly => {
                        BusinessMutationResult::Unchanged
                    }
                    frame_domain::DeletionMode::ExcludedQuarantine => {
                        BusinessMutationResult::Accepted
                    }
                    frame_domain::DeletionMode::AppendCompensatingEntry => {
                        return Err(AdapterFailure::Invalid);
                    }
                }
            }
            frame_domain::RetentionDecision::Denied => return Err(AdapterFailure::Retention),
        };
        statements.push(self.statement(
            DATA_REQUEST_INSERT_SQL,
            &[
                string(context.operation_id),
                string(context.scope.organization_id),
                JsValue::from_str(data_class),
                JsValue::from_str(&subject),
                JsValue::from_str(request_kind),
                JsValue::from_str(disposition),
                JsValue::NULL,
                timestamp(context.occurred_at),
                if request_kind == "delete" {
                    timestamp(context.occurred_at)
                } else {
                    JsValue::NULL
                },
                string(context.operation_id),
            ],
        )?);
        statements.push(self.postcondition_statement(
            context,
            "data_request",
            &context.operation_id.to_string(),
            BusinessRevision::INITIAL,
        )?);
        self.commit(
            context,
            subject,
            result,
            BusinessRevision::INITIAL,
            role_class(context),
            None,
            statements,
        )
        .await
    }

    async fn export_manifest_inner(
        &self,
        request: BusinessReadRequest,
    ) -> AdapterResult<TenantExportManifest> {
        self.export_tenant_data_inner(request)
            .await
            .map(|export| export.manifest)
    }

    async fn export_snapshot_state(
        &self,
        request: &BusinessReadRequest,
    ) -> AdapterResult<(Vec<(BusinessDataClass, u64)>, BusinessRevision)> {
        let operation_id = BusinessOperationId::new();
        let results = self
            .batch_results(vec![
                self.export_authority_statement(request, operation_id)?,
                self.statement(EXPORT_COUNTS_SQL, &[string(request.scope.organization_id)])?,
                self.statement(
                    EXPORT_REVISION_SQL,
                    &[string(request.scope.organization_id)],
                )?,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 4 {
            return Err(AdapterFailure::Corrupt);
        }
        let count_rows = Self::result_rows::<CountRow>(&results[1])?;
        if count_rows.len() != frame_domain::BusinessDataClass::ALL.len() {
            return Err(AdapterFailure::Corrupt);
        }
        let mut class_counts = Vec::with_capacity(count_rows.len());
        for row in count_rows {
            let class = parse_data_class(&row.data_class)?;
            let count = u64::try_from(row.item_count).map_err(|_| AdapterFailure::Corrupt)?;
            class_counts.push((class, count));
        }
        let mut revisions = Self::result_rows::<RevisionRow>(&results[2])?;
        if revisions.len() != 1 {
            return Err(AdapterFailure::Corrupt);
        }
        let source_revision = safe_revision(
            revisions
                .pop()
                .ok_or(AdapterFailure::Corrupt)?
                .source_revision,
        )?;
        Ok((class_counts, source_revision))
    }

    async fn export_page(
        &self,
        request: &BusinessReadRequest,
        cursor_class: &str,
        cursor_subject: &str,
    ) -> AdapterResult<Vec<ExportRow>> {
        let operation_id = BusinessOperationId::new();
        let results = self
            .batch_results(vec![
                self.export_authority_statement(request, operation_id)?,
                self.statement(
                    EXPORT_ROWS_SQL,
                    &[
                        string(request.scope.organization_id),
                        JsValue::from_str(cursor_class),
                        JsValue::from_str(cursor_subject),
                        number(EXPORT_PAGE_ROWS)?,
                    ],
                )?,
                self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            ])
            .await?;
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        let rows = Self::result_rows::<ExportRow>(&results[1])?;
        if rows.len() > usize::try_from(EXPORT_PAGE_ROWS).unwrap_or(usize::MAX) {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(rows)
    }

    async fn export_tenant_data_inner(
        &self,
        request: BusinessReadRequest,
    ) -> AdapterResult<TenantDataExport> {
        let (class_counts, source_revision) = self.export_snapshot_state(&request).await?;
        let expected_rows = class_counts
            .iter()
            .filter(|(class, _)| frame_domain::data_handling_rule(*class).exportable)
            .try_fold(0_u64, |total, (_, count)| total.checked_add(*count))
            .ok_or(AdapterFailure::Corrupt)?;
        let mut rows = Vec::new();
        let mut cursor_class = String::new();
        let mut cursor_subject = String::new();
        loop {
            let raw_rows = self
                .export_page(&request, &cursor_class, &cursor_subject)
                .await?;
            let page_len = raw_rows.len();
            for row in raw_rows {
                if (row.data_class.as_str(), row.subject_id.as_str())
                    <= (cursor_class.as_str(), cursor_subject.as_str())
                {
                    return Err(AdapterFailure::Corrupt);
                }
                serde_json::from_str::<serde_json::Value>(&row.export_json)
                    .map_err(|_| AdapterFailure::Corrupt)?;
                let data_class = parse_data_class(&row.data_class)?;
                if !frame_domain::data_handling_rule(data_class).exportable
                    || row.subject_id.is_empty()
                {
                    return Err(AdapterFailure::Corrupt);
                }
                cursor_class.clone_from(&row.data_class);
                cursor_subject.clone_from(&row.subject_id);
                rows.push(TenantExportRow {
                    data_class,
                    subject_id: row.subject_id,
                    checksum: ChecksumSha256::digest_bytes(row.export_json.as_bytes()),
                    export_json: row.export_json,
                });
                if u64::try_from(rows.len()).map_err(|_| AdapterFailure::Corrupt)? > expected_rows {
                    return Err(AdapterFailure::Conflict);
                }
            }
            if page_len < usize::try_from(EXPORT_PAGE_ROWS).unwrap_or(usize::MAX) {
                break;
            }
        }
        let (final_counts, final_revision) = self.export_snapshot_state(&request).await?;
        if final_counts != class_counts || final_revision != source_revision {
            return Err(AdapterFailure::Conflict);
        }
        if expected_rows != u64::try_from(rows.len()).map_err(|_| AdapterFailure::Corrupt)? {
            return Err(AdapterFailure::Conflict);
        }
        let content_checksum =
            tenant_export_content_checksum(&class_counts, &rows, source_revision);
        Ok(TenantDataExport {
            manifest: TenantExportManifest {
                scope: request.scope,
                generated_at: request.observed_at,
                source_revision,
                class_counts,
                content_checksum,
                excludes_secrets: true,
            },
            rows,
        })
    }
}

fn decode_video(row: VideoRow, scope: BusinessScope) -> AdapterResult<BusinessVideoRecord> {
    if row.organization_id != scope.organization_id.to_string()
        || !matches!(row.comments_enabled, 0 | 1)
        || row.metadata_schema_version <= 0
        || row.metadata_checksum.is_none()
    {
        return Err(AdapterFailure::Corrupt);
    }
    let metadata = VersionedBusinessDocument::parse(
        DocumentKind::VideoMetadata,
        row.metadata_json
            .as_deref()
            .ok_or(AdapterFailure::Corrupt)?,
    )
    .map_err(|_| AdapterFailure::Corrupt)?;
    if metadata.schema_version() as i64 != row.metadata_schema_version
        || row.metadata_checksum.as_deref() != Some(metadata.checksum().as_str())
    {
        return Err(AdapterFailure::Corrupt);
    }
    Ok(BusinessVideoRecord {
        id: row.id.parse().map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        owner_id: frame_domain::UserId::parse(&row.owner_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        privacy: parse_privacy(&row.privacy)?,
        metadata,
        comments_enabled: row.comments_enabled == 1,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        deleted_at: row.deleted_at_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_edit(row: EditRow, scope: BusinessScope) -> AdapterResult<frame_domain::VideoEditRecord> {
    let document = VersionedBusinessDocument::parse(DocumentKind::VideoEdit, &row.edit_spec_json)
        .map_err(|_| AdapterFailure::Corrupt)?;
    if row.document_version != i64::from(document.schema_version())
        || row.document_checksum.as_deref() != Some(document.checksum().as_str())
    {
        return Err(AdapterFailure::Corrupt);
    }
    Ok(frame_domain::VideoEditRecord {
        id: frame_domain::VideoEditId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        video_id: row.video_id.parse().map_err(|_| AdapterFailure::Corrupt)?,
        document,
        created_by_user_id: frame_domain::UserId::parse(&row.created_by_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_share(
    row: ShareRow,
    scope: BusinessScope,
) -> AdapterResult<frame_domain::VideoShareRecord> {
    if row.organization_id != scope.organization_id.to_string() {
        return Err(AdapterFailure::Corrupt);
    }
    Ok(frame_domain::VideoShareRecord {
        id: frame_domain::VideoShareId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        video_id: row.video_id.parse().map_err(|_| AdapterFailure::Corrupt)?,
        folder_id: row
            .folder_id
            .as_deref()
            .map(frame_domain::FolderId::parse)
            .transpose()
            .map_err(|_| AdapterFailure::Corrupt)?,
        shared_by_user_id: frame_domain::UserId::parse(&row.shared_by_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        mode: parse_share(&row.sharing_mode)?,
        shared_at: safe_timestamp(row.shared_at_ms)?,
        revoked_at: row.revoked_at_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_comment(
    row: CommentRow,
    scope: BusinessScope,
) -> AdapterResult<frame_domain::BusinessCommentRecord> {
    let author = match (row.author_user_id, row.anonymous_author_digest) {
        (Some(user_id), None) => CommentAuthor::User(
            frame_domain::UserId::parse(&user_id).map_err(|_| AdapterFailure::Corrupt)?,
        ),
        (None, Some(digest)) => CommentAuthor::Anonymous(
            frame_domain::SecretDigest::parse_sha256(digest)
                .map_err(|_| AdapterFailure::Corrupt)?,
        ),
        _ => return Err(AdapterFailure::Corrupt),
    };
    let record = frame_domain::BusinessCommentRecord {
        id: frame_domain::CommentId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        video_id: row.video_id.parse().map_err(|_| AdapterFailure::Corrupt)?,
        parent_comment_id: row
            .parent_comment_id
            .as_deref()
            .map(frame_domain::CommentId::parse)
            .transpose()
            .map_err(|_| AdapterFailure::Corrupt)?,
        author,
        kind: parse_comment_kind(&row.comment_kind)?,
        body: frame_domain::CommentBody::parse(row.body).map_err(|_| AdapterFailure::Corrupt)?,
        timeline_micros: row
            .timeline_micros
            .map(u64::try_from)
            .transpose()
            .map_err(|_| AdapterFailure::Corrupt)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        deleted_at: row.deleted_at_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
    };
    record.validate().map_err(|_| AdapterFailure::Corrupt)?;
    Ok(record)
}

fn decode_notification(
    row: NotificationRow,
    scope: BusinessScope,
) -> AdapterResult<frame_domain::NotificationRecord> {
    let payload =
        VersionedBusinessDocument::parse(DocumentKind::NotificationPayload, &row.data_json)
            .map_err(|_| AdapterFailure::Corrupt)?;
    if i64::from(payload.schema_version()) != row.payload_schema_version
        || row.payload_checksum.as_deref() != Some(payload.checksum().as_str())
    {
        return Err(AdapterFailure::Corrupt);
    }
    let record = frame_domain::NotificationRecord {
        id: frame_domain::NotificationId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        recipient_user_id: frame_domain::UserId::parse(&row.recipient_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        kind: parse_notification_kind(&row.kind)?,
        deduplication_key: frame_domain::IdempotencyKey::parse(row.deduplication_key)
            .map_err(|_| AdapterFailure::Corrupt)?,
        payload,
        created_at: safe_timestamp(row.created_at_ms)?,
        read_at: row.read_at_ms.map(safe_timestamp).transpose()?,
    };
    record.validate().map_err(|_| AdapterFailure::Corrupt)?;
    Ok(record)
}

fn decode_credit_account(
    row: CreditAccountRow,
    scope: BusinessScope,
) -> AdapterResult<CreditAccountRecord> {
    if !matches!(row.auto_top_up_enabled, 0 | 1) {
        return Err(AdapterFailure::Corrupt);
    }
    let record = CreditAccountRecord {
        id: frame_domain::CreditAccountId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        app_id: frame_domain::DeveloperAppId::parse(&row.app_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        state: CreditAccountState {
            balance_microcredits: u64::try_from(row.balance_microcredits)
                .map_err(|_| AdapterFailure::Corrupt)?,
            last_sequence: safe_revision(row.ledger_sequence)?,
        },
        auto_top_up_enabled: row.auto_top_up_enabled == 1,
        auto_top_up_threshold_microcredits: row
            .auto_top_up_threshold_microcredits
            .map(u64::try_from)
            .transpose()
            .map_err(|_| AdapterFailure::Corrupt)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        revision: safe_revision(row.revision)?,
    };
    record.validate().map_err(|_| AdapterFailure::Corrupt)?;
    Ok(record)
}

fn decode_legal_hold(
    row: LegalHoldRow,
    scope: BusinessScope,
) -> AdapterResult<BusinessLegalHoldRecord> {
    let record = BusinessLegalHoldRecord {
        id: frame_domain::LegalHoldId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        data_class: parse_data_class(&row.data_class)?,
        subject_id: row.subject_id,
        reason_code: frame_domain::StableEventCode::parse(row.reason_code)
            .map_err(|_| AdapterFailure::Corrupt)?,
        placed_by_user_id: frame_domain::UserId::parse(&row.placed_by_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        placed_at: safe_timestamp(row.placed_at_ms)?,
        released_at: row.released_at_ms.map(safe_timestamp).transpose()?,
    };
    record.validate().map_err(|_| AdapterFailure::Corrupt)?;
    Ok(record)
}

fn decode_receipt(row: OperationRow, replayed: bool) -> AdapterResult<BusinessMutationReceipt> {
    let organization_id = frame_domain::OrganizationId::parse(&row.organization_id)
        .map_err(|_| AdapterFailure::Corrupt)?;
    let scope =
        BusinessScope::from_organization(organization_id).map_err(|_| AdapterFailure::Corrupt)?;
    let result = match row.result_code.as_str() {
        "created" => BusinessMutationResult::Created,
        "applied" => BusinessMutationResult::Applied,
        "accepted" => BusinessMutationResult::Accepted,
        "revoked" => BusinessMutationResult::Revoked,
        "tombstoned" => BusinessMutationResult::Tombstoned,
        "purged" => BusinessMutationResult::Purged,
        "unchanged" => BusinessMutationResult::Unchanged,
        _ => return Err(AdapterFailure::Corrupt),
    };
    Ok(BusinessMutationReceipt {
        operation_id: BusinessOperationId::parse(&row.operation_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        principal_kind: row.principal_kind,
        principal_subject: row.principal_subject,
        action: row.action,
        subject_id: row.subject_id,
        request_fingerprint: ChecksumSha256::parse(row.request_fingerprint)
            .map_err(|_| AdapterFailure::Corrupt)?,
        result,
        resulting_revision: safe_revision(row.resulting_revision)?,
        committed_at: safe_timestamp(row.committed_at_ms)?,
        replayed,
    })
}

fn validate_command<T: Serialize>(
    context: &BusinessMutationContext,
    expected_action: frame_domain::BusinessAction,
    subject_id: &str,
    payload: &T,
) -> AdapterResult<()> {
    if context.action != expected_action {
        return Err(AdapterFailure::Invalid);
    }
    let payload_checksum =
        business_payload_checksum(payload).map_err(|_| AdapterFailure::Invalid)?;
    let scope = context.scope.tenant_id.to_string();
    let principal = context.principal.subject_for_receipt();
    let expected = business_semantic_fingerprint([
        context.action.stable_code().as_bytes(),
        scope.as_bytes(),
        context.principal.stable_kind().as_bytes(),
        principal.as_bytes(),
        context.idempotency_key.expose().as_bytes(),
        subject_id.as_bytes(),
        payload_checksum.as_str().as_bytes(),
    ]);
    if expected == context.request_fingerprint {
        Ok(())
    } else {
        Err(AdapterFailure::Conflict)
    }
}

fn classify_provider_error(message: &str) -> AdapterFailure {
    if exact_d1_trigger_error(message, RETENTION_SENTINEL) {
        AdapterFailure::Retention
    } else if exact_d1_trigger_error(message, REPLAY_SENTINEL)
        || exact_d1_trigger_error(message, EVENT_SENTINEL)
        || exact_d1_trigger_error(message, ACCOUNTING_SENTINEL)
    {
        AdapterFailure::Conflict
    } else if exact_d1_trigger_error(message, AUTHORITY_SENTINEL) {
        AdapterFailure::Stale
    } else {
        AdapterFailure::Unavailable
    }
}

fn exact_d1_trigger_error(message: &str, sentinel: &str) -> bool {
    let constraint = format!("{sentinel}: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)");
    let lines = message.lines().collect::<Vec<_>>();
    if lines.len() < 6
        || lines[0] != "D1: D1Error {"
        || lines[1] != format!("    cause: JsValue(Error: {constraint}")
        || lines[2] != format!("    Error: {constraint}")
        || lines.last() != Some(&"}")
    {
        return false;
    }
    let stack = &lines[3..lines.len() - 1];
    stack.iter().all(|line| line.starts_with("        at "))
        && stack.last().is_some_and(|line| line.ends_with("),"))
}

fn safe_timestamp(value: i64) -> AdapterResult<TimestampMillis> {
    TimestampMillis::new(value).map_err(|_| AdapterFailure::Corrupt)
}

fn safe_revision(value: i64) -> AdapterResult<BusinessRevision> {
    u64::try_from(value)
        .map_err(|_| AdapterFailure::Corrupt)
        .and_then(|value| BusinessRevision::new(value).map_err(|_| AdapterFailure::Corrupt))
}

fn optional_checksum(value: Option<String>) -> AdapterResult<ChecksumSha256> {
    Ok(value
        .map(ChecksumSha256::parse)
        .transpose()
        .map_err(|_| AdapterFailure::Corrupt)?
        .unwrap_or_else(frame_domain::business_initial_event_fingerprint))
}

fn number(value: u64) -> AdapterResult<JsValue> {
    if value > frame_domain::MAX_WIRE_INTEGER {
        return Err(AdapterFailure::Invalid);
    }
    Ok(JsValue::from_f64(value as f64))
}

fn timestamp(value: TimestampMillis) -> JsValue {
    JsValue::from_f64(value.get() as f64)
}

fn optional_timestamp(value: Option<TimestampMillis>) -> JsValue {
    value.map_or(JsValue::NULL, timestamp)
}

fn boolean(value: bool) -> JsValue {
    JsValue::from_f64(if value { 1.0 } else { 0.0 })
}

fn string(value: impl ToString) -> JsValue {
    JsValue::from_str(&value.to_string())
}

fn optional_string(value: Option<impl ToString>) -> JsValue {
    value.map_or(JsValue::NULL, string)
}

fn optional_checksum_value(value: Option<&ChecksumSha256>) -> JsValue {
    value.map_or(JsValue::NULL, |checksum| {
        JsValue::from_str(checksum.as_str())
    })
}

fn role_class(context: &BusinessMutationContext) -> &'static str {
    use frame_domain::BusinessAction;
    match context.action {
        BusinessAction::ManageLedger
        | BusinessAction::ManageLegalHold
        | BusinessAction::ExportData
        | BusinessAction::DeleteData => "owner",
        BusinessAction::ManageUpload
        | BusinessAction::ManageStorage
        | BusinessAction::ManageImport
        | BusinessAction::ManageDeveloper => "admin",
        BusinessAction::ManageNotification => "admin",
        BusinessAction::ReadNotification => "member",
        BusinessAction::CreateComment
            if matches!(context.principal, BusinessPrincipal::Anonymous(_)) =>
        {
            "member"
        }
        _ => "write",
    }
}

fn privacy_code(value: VideoPrivacy) -> &'static str {
    match value {
        VideoPrivacy::Private => "private",
        VideoPrivacy::Organization => "organization",
        VideoPrivacy::Unlisted => "unlisted",
        VideoPrivacy::Public => "public",
    }
}

fn parse_privacy(value: &str) -> AdapterResult<VideoPrivacy> {
    match value {
        "private" => Ok(VideoPrivacy::Private),
        "organization" => Ok(VideoPrivacy::Organization),
        "unlisted" => Ok(VideoPrivacy::Unlisted),
        "public" => Ok(VideoPrivacy::Public),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_share(value: &str) -> AdapterResult<ShareMode> {
    match value {
        "organization" => Ok(ShareMode::Organization),
        "space" => Ok(ShareMode::Space),
        "public_link" => Ok(ShareMode::PublicLink),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_comment_kind(value: &str) -> AdapterResult<frame_domain::CommentKind> {
    match value {
        "text" => Ok(frame_domain::CommentKind::Text),
        "emoji" => Ok(frame_domain::CommentKind::Emoji),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_notification_kind(value: &str) -> AdapterResult<frame_domain::NotificationKind> {
    match value {
        "view" => Ok(frame_domain::NotificationKind::View),
        "comment" => Ok(frame_domain::NotificationKind::Comment),
        "reply" => Ok(frame_domain::NotificationKind::Reply),
        "reaction" => Ok(frame_domain::NotificationKind::Reaction),
        "anon_view" => Ok(frame_domain::NotificationKind::AnonymousView),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn delivery_code(value: DeliveryState) -> &'static str {
    match value {
        DeliveryState::Pending => "pending",
        DeliveryState::Leased => "leased",
        DeliveryState::Delivered => "delivered",
        DeliveryState::DeadLetter => "dead_letter",
    }
}

fn parse_delivery(value: &str) -> AdapterResult<DeliveryState> {
    match value {
        "pending" => Ok(DeliveryState::Pending),
        "leased" => Ok(DeliveryState::Leased),
        "delivered" => Ok(DeliveryState::Delivered),
        "dead_letter" => Ok(DeliveryState::DeadLetter),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn import_code(value: ImportState) -> &'static str {
    match value {
        ImportState::Queued => "queued",
        ImportState::Running => "running",
        ImportState::Complete => "complete",
        ImportState::Failed => "failed",
        ImportState::Cancelled => "cancelled",
    }
}

fn parse_import(value: &str) -> AdapterResult<ImportState> {
    match value {
        "queued" => Ok(ImportState::Queued),
        "running" => Ok(ImportState::Running),
        "complete" => Ok(ImportState::Complete),
        "failed" => Ok(ImportState::Failed),
        "cancelled" => Ok(ImportState::Cancelled),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn upload_code(value: UploadState) -> &'static str {
    match value {
        UploadState::Initiated => "initiated",
        UploadState::Uploading => "uploading",
        UploadState::Finalizing => "finalizing",
        UploadState::Complete => "complete",
        UploadState::Failed => "failed",
        UploadState::Aborted => "aborted",
    }
}

fn parse_upload(value: &str) -> AdapterResult<UploadState> {
    match value {
        "initiated" => Ok(UploadState::Initiated),
        "uploading" => Ok(UploadState::Uploading),
        "finalizing" => Ok(UploadState::Finalizing),
        "complete" => Ok(UploadState::Complete),
        "failed" => Ok(UploadState::Failed),
        "aborted" => Ok(UploadState::Aborted),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn object_role_code(value: BusinessObjectRole) -> &'static str {
    match value {
        BusinessObjectRole::Source => "source",
        BusinessObjectRole::Segment => "segment",
        BusinessObjectRole::Thumbnail => "thumbnail",
        BusinessObjectRole::Preview => "preview",
        BusinessObjectRole::Spritesheet => "spritesheet",
        BusinessObjectRole::Audio => "audio",
        BusinessObjectRole::Export => "export",
        BusinessObjectRole::Manifest => "manifest",
    }
}

fn storage_state_code(value: StorageObjectState) -> &'static str {
    match value {
        StorageObjectState::Pending => "pending",
        StorageObjectState::Available => "available",
        StorageObjectState::Quarantined => "quarantined",
        StorageObjectState::Deleting => "deleting",
        StorageObjectState::Deleted => "deleted",
        StorageObjectState::Missing => "missing",
    }
}

fn executor_code(value: DerivativeExecutor) -> &'static str {
    match value {
        DerivativeExecutor::CloudflareMedia => "cloudflare_media",
        DerivativeExecutor::NativeGstreamer => "native_gstreamer",
    }
}

fn derivative_state_code(value: DerivativeState) -> &'static str {
    match value {
        DerivativeState::Queued => "queued",
        DerivativeState::Running => "running",
        DerivativeState::Succeeded => "succeeded",
        DerivativeState::Failed => "failed",
        DerivativeState::Cancelled => "cancelled",
    }
}

fn developer_key_code(value: frame_domain::DeveloperKeyKind) -> &'static str {
    match value {
        frame_domain::DeveloperKeyKind::Publishable => "publishable",
        frame_domain::DeveloperKeyKind::Secret => "secret",
    }
}

fn credit_kind_code(value: CreditTransactionKind) -> &'static str {
    match value {
        CreditTransactionKind::Purchase => "purchase",
        CreditTransactionKind::Usage => "usage",
        CreditTransactionKind::Refund => "refund",
        CreditTransactionKind::Adjustment => "adjustment",
    }
}

fn usage_kind_code(value: UsageKind) -> &'static str {
    match value {
        UsageKind::StorageByteDay => "storage_byte_day",
        UsageKind::UploadByte => "upload_byte",
        UsageKind::DownloadByte => "download_byte",
        UsageKind::TransformUnit => "transform_unit",
        UsageKind::ComputeMillisecond => "compute_millisecond",
    }
}

fn event_result(value: OrderedEventResult) -> BusinessMutationResult {
    match value {
        OrderedEventResult::Applied => BusinessMutationResult::Applied,
        OrderedEventResult::DeferredGap => BusinessMutationResult::Accepted,
        OrderedEventResult::Duplicate | OrderedEventResult::StaleIgnored => {
            BusinessMutationResult::Unchanged
        }
    }
}

fn data_class_code(value: BusinessDataClass) -> &'static str {
    value.stable_code()
}

fn data_delete_sql(value: BusinessDataClass) -> Option<&'static str> {
    match value {
        BusinessDataClass::VideoMetadata => Some(DELETE_VIDEO_SQL),
        BusinessDataClass::VideoEdit => Some(DELETE_EDIT_SQL),
        BusinessDataClass::Share => Some(DELETE_SHARE_SQL),
        BusinessDataClass::Comment => Some(DELETE_COMMENT_DATA_SQL),
        BusinessDataClass::Notification => Some(DELETE_NOTIFICATION_SQL),
        BusinessDataClass::Outbox => Some(DELETE_OUTBOX_SQL),
        BusinessDataClass::StorageIntegration => Some(DELETE_STORAGE_INTEGRATION_SQL),
        BusinessDataClass::StorageObject => Some(DELETE_STORAGE_OBJECT_SQL),
        BusinessDataClass::DerivativeJob => Some(DELETE_DERIVATIVE_SQL),
        BusinessDataClass::Upload => Some(DELETE_UPLOAD_SQL),
        BusinessDataClass::Import => Some(DELETE_IMPORT_SQL),
        BusinessDataClass::DeveloperApp => Some(DELETE_DEVELOPER_APP_SQL),
        BusinessDataClass::DeveloperDomain => Some(DELETE_DEVELOPER_DOMAIN_SQL),
        BusinessDataClass::DeveloperApiKey => Some(DELETE_DEVELOPER_KEY_SQL),
        BusinessDataClass::DeveloperVideo => Some(DELETE_DEVELOPER_VIDEO_SQL),
        BusinessDataClass::CreditAccount
        | BusinessDataClass::CreditTransaction
        | BusinessDataClass::UsageLedger
        | BusinessDataClass::DailyStorageSnapshot => None,
        BusinessDataClass::MessengerLegacy => Some(DELETE_MESSENGER_QUARANTINE_SQL),
    }
}

fn tenant_export_content_checksum(
    class_counts: &[(BusinessDataClass, u64)],
    rows: &[TenantExportRow],
    source_revision: BusinessRevision,
) -> ChecksumSha256 {
    let mut material = Vec::new();
    for (class, count) in class_counts {
        material.extend_from_slice(class.stable_code().as_bytes());
        material.extend_from_slice(&count.to_be_bytes());
    }
    for row in rows {
        material.extend_from_slice(row.data_class.stable_code().as_bytes());
        material.extend_from_slice(row.subject_id.as_bytes());
        material.extend_from_slice(row.export_json.as_bytes());
    }
    material.extend_from_slice(&source_revision.get().to_be_bytes());
    ChecksumSha256::digest_bytes(&material)
}

fn parse_data_class(value: &str) -> AdapterResult<BusinessDataClass> {
    frame_domain::BusinessDataClass::ALL
        .into_iter()
        .find(|candidate| data_class_code(*candidate) == value)
        .ok_or(AdapterFailure::Corrupt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger_envelope(sentinel: &str) -> String {
        let constraint =
            format!("{sentinel}: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)");
        format!(
            "D1: D1Error {{\n    cause: JsValue(Error: {constraint}\n    Error: {constraint}\n        at batch (worker.js:1:1),\n        at async write (worker.js:2:2),\n}}"
        )
    }

    #[test]
    fn trigger_classification_requires_the_exact_d1_envelope() {
        let authority = trigger_envelope(AUTHORITY_SENTINEL);
        assert_eq!(classify_provider_error(&authority), AdapterFailure::Stale);
        assert_eq!(
            classify_provider_error(&trigger_envelope(REPLAY_SENTINEL)),
            AdapterFailure::Conflict
        );
        assert_eq!(
            classify_provider_error(&trigger_envelope(RETENTION_SENTINEL)),
            AdapterFailure::Retention
        );
        for spoof in [
            AUTHORITY_SENTINEL.to_owned(),
            format!("provider prefix: {authority}"),
            authority.replace("SQLITE_CONSTRAINT_TRIGGER", "SQLITE_CONSTRAINT_CHECK"),
            format!("{authority}\ntrailing"),
        ] {
            assert_eq!(classify_provider_error(&spoof), AdapterFailure::Unavailable);
        }
    }

    #[test]
    fn tenant_export_checksum_covers_manifest_rows_and_revision() {
        let counts = BusinessDataClass::ALL
            .into_iter()
            .map(|class| (class, 0))
            .collect::<Vec<_>>();
        let revision = BusinessRevision::INITIAL;
        let empty = tenant_export_content_checksum(&counts, &[], revision);
        assert_eq!(
            empty.as_str(),
            "88edd832bddc940755501b809d975856160f4fd1555b220f24bfacfd7884770e"
        );

        let json = r#"{"id":"one"}"#.to_owned();
        let rows = vec![TenantExportRow {
            data_class: BusinessDataClass::VideoMetadata,
            subject_id: "one".to_owned(),
            checksum: ChecksumSha256::digest_bytes(json.as_bytes()),
            export_json: json,
        }];
        assert_ne!(
            empty,
            tenant_export_content_checksum(&counts, &rows, revision)
        );
        assert_ne!(
            empty,
            tenant_export_content_checksum(
                &counts,
                &[],
                BusinessRevision::new(1).expect("revision")
            )
        );
        let mut changed_counts = counts;
        changed_counts[0].1 = 1;
        assert_ne!(
            empty,
            tenant_export_content_checksum(&changed_counts, &[], revision)
        );
    }
}
