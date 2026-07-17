//! Race-safe D1 organization repository.

use async_trait::async_trait;
use frame_domain::{
    ActiveOrganizationSelection, AllowedDomain, AllowedDomainRecord, CollaborationName, FolderId,
    FolderRecord, InviteState, MembershipState, OrganizationAction, OrganizationAuditDecision,
    OrganizationAuditId, OrganizationGraphFinding, OrganizationGraphFindingKind, OrganizationId,
    OrganizationMembershipRecord, OrganizationOperationId, OrganizationRecord,
    OrganizationRepairActionKind, OrganizationRepairPlan, OrganizationRepairStep,
    OrganizationRevision, OrganizationRole, OrganizationScope, OrganizationSettings,
    OrganizationStatus, SecretDigest, SpaceId, SpaceMembershipRecord, SpaceRecord, SpaceRole,
    TimestampMillis, TombstonePolicy, UserId,
};
use frame_ports::{
    AcceptOrganizationInviteCommand, ChangeOrganizationMemberCommand, ChangeSpaceRoleCommand,
    CreateFolderCommand, CreateOrganizationCommand, CreateSpaceCommand,
    IssueOrganizationInviteCommand, LegacyOrganizationSelectionRepositoryV1,
    LegacySetActiveOrganizationCommandV1, MoveFolderCommand, OrganizationCollectionRequest,
    OrganizationGraphAudit, OrganizationGraphAuditRequest, OrganizationInviteSummary,
    OrganizationMutationContext, OrganizationMutationReceipt, OrganizationMutationResult,
    OrganizationPage, OrganizationPortError, OrganizationReadRequest, OrganizationRepository,
    OrganizationSnapshot, OrganizationSpaceCollectionRequest, RecoverOrganizationCommand,
    RevokeOrganizationInviteCommand, SetActiveOrganizationCommand, TombstoneOrganizationCommand,
    TransferOrganizationOwnershipCommand, UpdateFolderCommand, UpdateOrganizationSettingsCommand,
    UpdateSpaceCommand, UpsertAllowedDomainCommand,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, send::IntoSendFuture};

const OPERATION_BY_IDEMPOTENCY_SQL: &str =
    include_str!("../queries/organization/operation_by_idempotency.sql");
const OPERATION_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/organization/operation_absent_assert.sql");
const OPERATION_INSERT_SQL: &str = include_str!("../queries/organization/operation_insert.sql");
const AUTHORITY_ASSERT_SQL: &str = include_str!("../queries/organization/authority_assert.sql");
const PRINCIPAL_ASSERT_SQL: &str = include_str!("../queries/organization/principal_assert.sql");
const SPACE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/organization/space_authority_assert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/organization/audit_insert.sql");
const SNAPSHOT_SQL: &str = include_str!("../queries/organization/snapshot.sql");
const READ_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/organization/read_authority_assert.sql");
const MEMBERS_LIST_SQL: &str = include_str!("../queries/organization/members_list.sql");
const INVITES_LIST_SQL: &str = include_str!("../queries/organization/invites_list.sql");
const DOMAINS_LIST_SQL: &str = include_str!("../queries/organization/domains_list.sql");
const SPACES_LIST_SQL: &str = include_str!("../queries/organization/spaces_list.sql");
const SPACE_MEMBERS_LIST_SQL: &str = include_str!("../queries/organization/space_members_list.sql");
const FOLDERS_LIST_SQL: &str = include_str!("../queries/organization/folders_list.sql");
const ORGANIZATION_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/organization/organization_absent_assert.sql");
const ORGANIZATION_INSERT_SQL: &str =
    include_str!("../queries/organization/organization_insert.sql");
const OWNER_MEMBERSHIP_INSERT_SQL: &str =
    include_str!("../queries/organization/owner_membership_insert.sql");
const ORGANIZATION_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/organization_postcondition.sql");
const SELECTION_UPSERT_SQL: &str = include_str!("../queries/organization/selection_upsert.sql");
const SELECTION_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/selection_postcondition.sql");
const LEGACY_SELECTION_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/organization/legacy_selection_authority_assert.sql");
const LEGACY_SELECTION_ACTIVE_UPDATE_SQL: &str =
    include_str!("../queries/organization/legacy_selection_active_update.sql");
const LEGACY_SELECTION_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/legacy_selection_postcondition.sql");
const LEGACY_SELECTION_OPERATION_INSERT_SQL: &str =
    include_str!("../queries/organization/legacy_selection_operation_insert.sql");
const LEGACY_SELECTION_RECEIPT_SQL: &str =
    include_str!("../queries/organization/legacy_selection_receipt.sql");
const INVITE_INSERT_SQL: &str = include_str!("../queries/organization/invite_insert.sql");
const INVITE_REVOKE_SQL: &str = include_str!("../queries/organization/invite_revoke.sql");
const INVITE_ACCEPT_SQL: &str = include_str!("../queries/organization/invite_accept.sql");
const INVITE_ACCEPT_MEMBERSHIP_SQL: &str =
    include_str!("../queries/organization/invite_accept_membership.sql");
const INVITE_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/invite_postcondition.sql");
const INVITE_ACCEPT_ELIGIBILITY_ASSERT_SQL: &str =
    include_str!("../queries/organization/invite_accept_eligibility_assert.sql");
const OWNERSHIP_TARGET_ASSERT_SQL: &str =
    include_str!("../queries/organization/ownership_target_assert.sql");
const OWNERSHIP_OLD_DEMOTE_SQL: &str =
    include_str!("../queries/organization/ownership_old_demote.sql");
const OWNERSHIP_POINTER_UPDATE_SQL: &str =
    include_str!("../queries/organization/ownership_pointer_update.sql");
const OWNERSHIP_NEW_PROMOTE_SQL: &str =
    include_str!("../queries/organization/ownership_new_promote.sql");
const OWNERSHIP_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/ownership_postcondition.sql");
const MEMBER_CHANGE_SQL: &str = include_str!("../queries/organization/member_change.sql");
const MEMBER_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/member_postcondition.sql");
const MEMBER_TARGET_ASSERT_SQL: &str =
    include_str!("../queries/organization/member_target_assert.sql");
const PRINCIPAL_AUTHORITY_BUMP_SQL: &str =
    include_str!("../queries/organization/principal_authority_bump.sql");
const PRINCIPAL_GRANTS_REVOKE_SQL: &str =
    include_str!("../queries/organization/principal_grants_revoke.sql");
const PRINCIPAL_BUMP_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/principal_bump_postcondition.sql");
const DOMAIN_UPSERT_SQL: &str = include_str!("../queries/organization/domain_upsert.sql");
const DOMAIN_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/domain_postcondition.sql");
const DOMAIN_CAPACITY_ASSERT_SQL: &str =
    include_str!("../queries/organization/domain_capacity_assert.sql");
const SETTINGS_UPDATE_SQL: &str = include_str!("../queries/organization/settings_update.sql");
const SPACE_INSERT_SQL: &str = include_str!("../queries/organization/space_insert.sql");
const SPACE_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/space_postcondition.sql");
const SPACE_UPDATE_SQL: &str = include_str!("../queries/organization/space_update.sql");
const SPACE_UPDATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/space_update_postcondition.sql");
const SPACE_ROLE_UPSERT_SQL: &str = include_str!("../queries/organization/space_role_upsert.sql");
const SPACE_ROLE_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/space_role_postcondition.sql");
const FOLDER_INSERT_SQL: &str = include_str!("../queries/organization/folder_insert.sql");
const FOLDER_UPDATE_SQL: &str = include_str!("../queries/organization/folder_update.sql");
const FOLDER_MANAGE_ASSERT_SQL: &str =
    include_str!("../queries/organization/folder_manage_assert.sql");
const FOLDER_UPDATE_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/folder_update_postcondition.sql");
const FOLDER_CLOSURE_INSERT_SQL: &str =
    include_str!("../queries/organization/folder_closure_insert.sql");
const FOLDER_MOVE_ASSERT_SQL: &str = include_str!("../queries/organization/folder_move_assert.sql");
const FOLDER_CLOSURE_DELETE_SUBTREE_SQL: &str =
    include_str!("../queries/organization/folder_closure_delete_subtree.sql");
const FOLDER_MOVE_SQL: &str = include_str!("../queries/organization/folder_move.sql");
const FOLDER_DESCENDANT_DEPTH_UPDATE_SQL: &str =
    include_str!("../queries/organization/folder_descendant_depth_update.sql");
const FOLDER_CLOSURE_MOVE_INSERT_SQL: &str =
    include_str!("../queries/organization/folder_closure_move_insert.sql");
const FOLDER_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/folder_postcondition.sql");
const FOLDER_TREE_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/folder_tree_postcondition.sql");
const INVITE_MEMBERSHIP_POSTCONDITION_SQL: &str =
    include_str!("../queries/organization/invite_membership_postcondition.sql");
const TOMBSTONE_SQL: &str = include_str!("../queries/organization/tombstone.sql");
const RECOVER_SQL: &str = include_str!("../queries/organization/recover.sql");
const TOMBSTONE_EVENT_INSERT_SQL: &str =
    include_str!("../queries/organization/tombstone_event_insert.sql");
const SUPPORT_ASSERT_SQL: &str = include_str!("../queries/organization/support_assert.sql");
const GRAPH_AUDIT_SQL: &str = include_str!("../queries/organization/graph_audit.sql");
const GRAPH_AUDIT_SELECTIONS_SQL: &str =
    include_str!("../queries/organization/graph_audit_selections.sql");
const GRAPH_AUDIT_FOLDERS_SQL: &str =
    include_str!("../queries/organization/graph_audit_folders.sql");
const REPAIR_PLAN_INSERT_SQL: &str = include_str!("../queries/organization/repair_plan_insert.sql");
const ASSERTION_CLEANUP_SQL: &str = include_str!("../queries/organization/assertion_cleanup.sql");
const RETENTION_ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/organization/retention_assertion_cleanup.sql");
const RECOVERY_RETENTION_ASSERT_SQL: &str =
    include_str!("../queries/organization/recovery_retention_assert.sql");

const ORGANIZATION_CAS_SENTINEL: &str = "frame_organization_cas_conflict_v1";
const ORGANIZATION_RETENTION_SENTINEL: &str = "frame_organization_retention_locked_v1";

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
    const fn into_port(self) -> OrganizationPortError {
        match self {
            Self::AccessDenied => OrganizationPortError::AccessDenied,
            Self::Stale => OrganizationPortError::StaleAuthority,
            Self::Conflict => OrganizationPortError::Conflict,
            Self::Invalid => OrganizationPortError::Invalid,
            Self::Retention => OrganizationPortError::RetentionLocked,
            Self::Unavailable => OrganizationPortError::Unavailable,
            Self::Corrupt => OrganizationPortError::Corrupt,
        }
    }
}

type AdapterResult<T> = Result<T, AdapterFailure>;

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    organization_id: String,
    idempotency_key: String,
    operation_kind: String,
    subject_id: String,
    request_fingerprint: String,
    result_code: String,
    resulting_revision: i64,
    authority_version: i64,
    committed_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SnapshotRow {
    id: String,
    owner_id: String,
    name: String,
    status: String,
    settings_json: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    tombstoned_at_ms: Option<i64>,
    retention_until_ms: Option<i64>,
    revision: i64,
    authority_version: i64,
    actor_role: String,
    actor_state: String,
    has_pro_seat: i64,
    member_created_at_ms: i64,
    member_updated_at_ms: i64,
    member_revision: i64,
    member_authority_version: i64,
    default_organization_id: Option<String>,
    active_organization_id: Option<String>,
    organization_preference_revision: i64,
}

#[derive(Debug, Deserialize)]
struct MembershipRow {
    organization_id: String,
    user_id: String,
    role: String,
    state: String,
    has_pro_seat: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: i64,
    authority_version: i64,
}

#[derive(Debug, Deserialize)]
struct InviteSummaryRow {
    id: String,
    organization_id: String,
    invited_by_user_id: String,
    accepted_by_user_id: Option<String>,
    role: String,
    status: String,
    created_at_ms: i64,
    expires_at_ms: i64,
    resolved_at_ms: Option<i64>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct DomainRow {
    organization_id: String,
    domain_ascii: String,
    verified_at_ms: Option<i64>,
    created_at_ms: i64,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct SpaceRow {
    id: String,
    organization_id: String,
    created_by_user_id: String,
    name: String,
    is_primary: i64,
    is_public: i64,
    settings_json: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    deleted_at_ms: Option<i64>,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct SpaceMembershipRow {
    space_id: String,
    organization_id: String,
    user_id: String,
    role: String,
    state: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct FolderRow {
    id: String,
    organization_id: String,
    space_id: String,
    parent_id: Option<String>,
    created_by_user_id: String,
    name: String,
    is_public: i64,
    settings_json: String,
    depth: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    deleted_at_ms: Option<i64>,
    revision: i64,
    tree_revision: i64,
}

#[derive(Debug, Deserialize)]
struct GraphFindingRow {
    finding_kind: String,
    subject_id: String,
    observed_revision: i64,
}

#[derive(Debug, Serialize)]
pub struct OrganizationRepositoryTelemetry {
    pub event: &'static str,
    pub operation: &'static str,
    pub outcome: &'static str,
    pub rows: u32,
}

impl OrganizationRepositoryTelemetry {
    fn emit(operation: &'static str, outcome: &'static str, rows: u32) {
        let event = Self {
            event: "d1_organization_repository",
            operation,
            outcome,
            rows,
        };
        if let Ok(payload) = serde_json::to_string(&event) {
            worker::console_log!("{payload}");
        }
    }
}

/// D1 adapter. Mutation promises always settle; local deadlines never abandon a maybe-commit.
pub struct D1OrganizationRepository<'database> {
    database: &'database D1Database,
    tombstone_policy: TombstonePolicy,
}

impl<'database> D1OrganizationRepository<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database, tombstone_policy: TombstonePolicy) -> Self {
        Self {
            database,
            tombstone_policy,
        }
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> AdapterResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| AdapterFailure::Unavailable)
    }

    async fn rows<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> AdapterResult<Vec<T>> {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| AdapterFailure::Unavailable)?;
        if !result.success() {
            return Err(AdapterFailure::Unavailable);
        }
        result
            .results::<serde_json::Value>()
            .map_err(|_| AdapterFailure::Unavailable)?
            .into_iter()
            .map(|row| serde_json::from_value(row).map_err(|_| AdapterFailure::Corrupt))
            .collect()
    }

    async fn one<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> AdapterResult<Option<T>> {
        let mut rows = self.rows(sql, bindings).await?;
        if rows.len() > 1 {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(rows.pop())
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
            .map_err(|error| {
                let message = error.to_string();
                if exact_d1_trigger_error(&message, ORGANIZATION_CAS_SENTINEL) {
                    AdapterFailure::Stale
                } else if exact_d1_trigger_error(&message, ORGANIZATION_RETENTION_SENTINEL) {
                    AdapterFailure::Retention
                } else {
                    AdapterFailure::Unavailable
                }
            })?;
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

    async fn authorized_rows<T: DeserializeOwned>(
        &self,
        authority: OrganizationReadRequest,
        role_class: &'static str,
        data: D1PreparedStatement,
    ) -> AdapterResult<Vec<T>> {
        let operation_id = OrganizationOperationId::new();
        let authority_statement = self.statement(
            READ_AUTHORITY_ASSERT_SQL,
            &[
                string(format!("{operation_id}:read")),
                string(authority.scope.organization_id),
                string(authority.actor_id),
                integer(authority.identity_revision)?,
                integer(authority.session_version)?,
                JsValue::from_str(role_class),
            ],
        )?;
        let cleanup = self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?;
        let results = match self
            .batch_results(vec![authority_statement, data, cleanup])
            .await
        {
            Err(AdapterFailure::Stale) => return Err(AdapterFailure::AccessDenied),
            result => result?,
        };
        if results.len() != 3 {
            return Err(AdapterFailure::Corrupt);
        }
        Self::result_rows(&results[1])
    }

    fn authority_statement(
        &self,
        context: &OrganizationMutationContext,
        organization_status: &'static str,
        role_class: &'static str,
    ) -> AdapterResult<D1PreparedStatement> {
        let fence = context.authority.fence;
        self.statement(
            AUTHORITY_ASSERT_SQL,
            &[
                string(format!("{}:authority", context.authority.operation_id)),
                string(context.authority.scope.organization_id),
                string(context.authority.actor_id),
                integer(fence.identity_revision)?,
                integer(fence.session_version)?,
                JsValue::from_str(organization_status),
                integer(fence.organization_revision)?,
                integer(fence.organization_authority_version)?,
                integer(fence.membership_revision)?,
                integer(fence.membership_authority_version)?,
                JsValue::from_str(role_class),
            ],
        )
    }

    fn principal_statement(
        &self,
        context: &OrganizationMutationContext,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            PRINCIPAL_ASSERT_SQL,
            &[
                string(format!("{}:principal", context.authority.operation_id)),
                string(context.authority.actor_id),
                integer(context.authority.fence.identity_revision)?,
                integer(context.authority.fence.session_version)?,
            ],
        )
    }

    async fn replay(
        &self,
        context: &OrganizationMutationContext,
        operation_kind: &'static str,
        subject_id: &str,
        request_fingerprint: &SecretDigest,
    ) -> AdapterResult<Option<OrganizationMutationReceipt>> {
        let row = self
            .one::<OperationRow>(
                OPERATION_BY_IDEMPOTENCY_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(context.idempotency_key.expose()),
                    string(context.authority.actor_id),
                    integer(context.authority.fence.identity_revision)?,
                    integer(context.authority.fence.session_version)?,
                ],
            )
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.organization_id != context.authority.scope.organization_id.to_string()
            || row.idempotency_key != context.idempotency_key.expose()
            || row.operation_id != context.authority.operation_id.to_string()
            || row.operation_kind != operation_kind
            || row.subject_id != subject_id
            || row.request_fingerprint != request_fingerprint.expose_for_verification()
        {
            return Err(AdapterFailure::Conflict);
        }
        Ok(Some(decode_receipt(row, true)?))
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit(
        &self,
        context: &OrganizationMutationContext,
        operation_kind: &'static str,
        subject_id: String,
        result: OrganizationMutationResult,
        resulting_revision: OrganizationRevision,
        authority_version: OrganizationRevision,
        request_fingerprint: SecretDigest,
        mut statements: Vec<D1PreparedStatement>,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        if let Some(receipt) = self
            .replay(context, operation_kind, &subject_id, &request_fingerprint)
            .await?
        {
            OrganizationRepositoryTelemetry::emit(operation_kind, "replay", 1);
            return Ok(receipt);
        }
        statements.insert(
            0,
            self.statement(
                OPERATION_ABSENT_ASSERT_SQL,
                &[
                    string(format!("{}:operation", context.authority.operation_id)),
                    string(context.authority.operation_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(context.idempotency_key.expose()),
                ],
            )?,
        );
        statements.push(self.operation_statement(
            context,
            operation_kind,
            &subject_id,
            result,
            resulting_revision,
            authority_version,
            &request_fingerprint,
        )?);
        statements.push(self.statement(
            ASSERTION_CLEANUP_SQL,
            &[string(context.authority.operation_id)],
        )?);
        statements.push(self.statement(
            RETENTION_ASSERTION_CLEANUP_SQL,
            &[string(context.authority.operation_id)],
        )?);
        statements.push(self.allow_audit_statement(context, &subject_id)?);

        match self.batch(statements).await {
            Ok(()) => {
                OrganizationRepositoryTelemetry::emit(operation_kind, "allow", 1);
                Ok(OrganizationMutationReceipt {
                    operation_id: context.authority.operation_id,
                    result,
                    subject_id,
                    committed_at: context.authority.occurred_at,
                    resulting_revision,
                    authority_version,
                    replayed: false,
                })
            }
            Err(failure) => {
                if let Some(receipt) = self
                    .replay(context, operation_kind, &subject_id, &request_fingerprint)
                    .await?
                {
                    OrganizationRepositoryTelemetry::emit(operation_kind, "reconciled", 1);
                    return Ok(receipt);
                }
                OrganizationRepositoryTelemetry::emit(
                    operation_kind,
                    match failure {
                        AdapterFailure::Stale => "stale",
                        AdapterFailure::Retention => "retention_locked",
                        AdapterFailure::Unavailable => "unavailable",
                        AdapterFailure::AccessDenied
                        | AdapterFailure::Conflict
                        | AdapterFailure::Invalid
                        | AdapterFailure::Corrupt => "deny",
                    },
                    0,
                );
                Err(failure)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn operation_statement(
        &self,
        context: &OrganizationMutationContext,
        operation_kind: &'static str,
        subject_id: &str,
        result: OrganizationMutationResult,
        resulting_revision: OrganizationRevision,
        authority_version: OrganizationRevision,
        request_fingerprint: &SecretDigest,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            OPERATION_INSERT_SQL,
            &[
                string(context.authority.operation_id),
                string(context.authority.scope.organization_id),
                JsValue::from_str(context.idempotency_key.expose()),
                JsValue::from_str(operation_kind),
                JsValue::from_str(subject_id),
                JsValue::from_str(request_fingerprint.expose_for_verification()),
                JsValue::from_str(result.stable_code()),
                integer(resulting_revision)?,
                integer(authority_version)?,
                timestamp_value(context.authority.occurred_at),
            ],
        )
    }

    fn allow_audit_statement(
        &self,
        context: &OrganizationMutationContext,
        subject_id: &str,
    ) -> AdapterResult<D1PreparedStatement> {
        self.audit_statement(context, subject_id, "allow", None)
    }

    fn audit_statement(
        &self,
        context: &OrganizationMutationContext,
        subject_id: &str,
        outcome: &'static str,
        denial: Option<&str>,
    ) -> AdapterResult<D1PreparedStatement> {
        let digest = hex_sha256(subject_id.as_bytes());
        self.statement(
            AUDIT_INSERT_SQL,
            &[
                string(OrganizationAuditId::new()),
                string(context.authority.operation_id),
                string(context.authority.scope.organization_id),
                string(context.authority.actor_id),
                JsValue::from_str(context.action.stable_code()),
                JsValue::from_str(object_kind(context.action)),
                JsValue::from_str(&digest),
                JsValue::from_str(outcome),
                denial.map_or(JsValue::NULL, JsValue::from_str),
                timestamp_value(context.authority.occurred_at),
            ],
        )
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

fn decode_receipt(row: OperationRow, replayed: bool) -> AdapterResult<OrganizationMutationReceipt> {
    let result = match row.result_code.as_str() {
        "created" => OrganizationMutationResult::Created,
        "applied" => OrganizationMutationResult::Applied,
        "accepted" => OrganizationMutationResult::Accepted,
        "revoked" => OrganizationMutationResult::Revoked,
        "tombstoned" => OrganizationMutationResult::Tombstoned,
        "recovered" => OrganizationMutationResult::Recovered,
        "unchanged" => OrganizationMutationResult::Unchanged,
        _ => return Err(AdapterFailure::Corrupt),
    };
    Ok(OrganizationMutationReceipt {
        operation_id: OrganizationOperationId::parse(&row.operation_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        result,
        subject_id: row.subject_id,
        committed_at: safe_timestamp(row.committed_at_ms)?,
        resulting_revision: safe_revision(row.resulting_revision)?,
        authority_version: safe_revision(row.authority_version)?,
        replayed,
    })
}

fn safe_timestamp(value: i64) -> AdapterResult<TimestampMillis> {
    TimestampMillis::new(value).map_err(|_| AdapterFailure::Corrupt)
}

fn safe_revision(value: i64) -> AdapterResult<OrganizationRevision> {
    u64::try_from(value)
        .map_err(|_| AdapterFailure::Corrupt)
        .and_then(|value| OrganizationRevision::new(value).map_err(|_| AdapterFailure::Corrupt))
}

fn integer(value: OrganizationRevision) -> AdapterResult<JsValue> {
    if value.get() > frame_domain::MAX_WIRE_INTEGER {
        return Err(AdapterFailure::Invalid);
    }
    Ok(JsValue::from_f64(value.get() as f64))
}

fn timestamp_value(value: TimestampMillis) -> JsValue {
    JsValue::from_f64(value.get() as f64)
}

fn boolean_value(value: bool) -> JsValue {
    JsValue::from_f64(if value { 1.0 } else { 0.0 })
}

fn string(value: impl ToString) -> JsValue {
    JsValue::from_str(&value.to_string())
}

fn optional_string(value: Option<impl ToString>) -> JsValue {
    value.map_or(JsValue::NULL, string)
}

fn optional_timestamp(value: Option<TimestampMillis>) -> JsValue {
    value.map_or(JsValue::NULL, timestamp_value)
}

fn optional_revision(value: Option<OrganizationRevision>) -> AdapterResult<JsValue> {
    value.map_or(Ok(JsValue::NULL), integer)
}

fn hex_sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn semantic_fingerprint(
    operation_kind: &'static str,
    actor_id: UserId,
    payload: &serde_json::Value,
) -> AdapterResult<SecretDigest> {
    let payload =
        serde_json::to_vec(&canonical_json(payload)).map_err(|_| AdapterFailure::Invalid)?;
    if payload.len() > 1_048_576 {
        return Err(AdapterFailure::Invalid);
    }
    let actor_id = actor_id.to_string();
    let mut hash = Sha256::new();
    hash.update(b"frame/organization/semantic-request/v2\0");
    for part in [operation_kind.as_bytes(), actor_id.as_bytes(), &payload] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    SecretDigest::parse_sha256(format!("{:x}", hash.finalize()))
        .map_err(|_| AdapterFailure::Corrupt)
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json).collect())
        }
        serde_json::Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(left, _)| *left);
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        scalar => scalar.clone(),
    }
}

const fn object_kind(action: OrganizationAction) -> &'static str {
    match action.object_kind() {
        frame_domain::OrganizationObjectKind::Organization => "organization",
        frame_domain::OrganizationObjectKind::Membership => "membership",
        frame_domain::OrganizationObjectKind::Invite => "invite",
        frame_domain::OrganizationObjectKind::AllowedDomain => "allowed_domain",
        frame_domain::OrganizationObjectKind::Settings => "settings",
        frame_domain::OrganizationObjectKind::Seat => "seat",
        frame_domain::OrganizationObjectKind::Space => "space",
        frame_domain::OrganizationObjectKind::Folder => "folder",
        frame_domain::OrganizationObjectKind::Tombstone => "tombstone",
        frame_domain::OrganizationObjectKind::RepairPlan => "repair_plan",
    }
}

fn role(value: OrganizationRole) -> &'static str {
    match value {
        OrganizationRole::Owner => "owner",
        OrganizationRole::Admin => "admin",
        OrganizationRole::Member => "member",
        OrganizationRole::Viewer => "viewer",
    }
}

fn membership_state(value: MembershipState) -> &'static str {
    match value {
        MembershipState::Active => "active",
        MembershipState::Suspended => "suspended",
        MembershipState::Removed => "removed",
    }
}

fn space_role(value: SpaceRole) -> &'static str {
    match value {
        SpaceRole::Manager => "manager",
        SpaceRole::Contributor => "contributor",
        SpaceRole::Viewer => "viewer",
    }
}

fn parse_role(value: &str) -> AdapterResult<OrganizationRole> {
    match value {
        "owner" => Ok(OrganizationRole::Owner),
        "admin" => Ok(OrganizationRole::Admin),
        "member" => Ok(OrganizationRole::Member),
        "viewer" => Ok(OrganizationRole::Viewer),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_membership_state(value: &str) -> AdapterResult<MembershipState> {
    match value {
        "active" => Ok(MembershipState::Active),
        "suspended" => Ok(MembershipState::Suspended),
        "removed" => Ok(MembershipState::Removed),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_status(value: &str) -> AdapterResult<OrganizationStatus> {
    match value {
        "active" => Ok(OrganizationStatus::Active),
        "tombstoned" => Ok(OrganizationStatus::Tombstoned),
        "deleted" => Ok(OrganizationStatus::Deleted),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn expected_next(value: OrganizationRevision) -> AdapterResult<OrganizationRevision> {
    value.next().map_err(|_| AdapterFailure::Invalid)
}

fn collection_limit(request: &OrganizationCollectionRequest) -> AdapterResult<u16> {
    if request.maximum_items == 0
        || request.maximum_items > 500
        || request.after_id.as_ref().is_some_and(|cursor| {
            cursor.is_empty() || cursor.len() > 255 || cursor.chars().any(char::is_control)
        })
    {
        return Err(AdapterFailure::Invalid);
    }
    request
        .maximum_items
        .checked_add(1)
        .ok_or(AdapterFailure::Invalid)
}

fn into_page<Row, Item>(
    mut rows: Vec<Row>,
    maximum_items: u16,
    cursor: impl Fn(&Row) -> String,
    decode: impl FnMut(Row) -> AdapterResult<Item>,
) -> AdapterResult<OrganizationPage<Item>> {
    let maximum = usize::from(maximum_items);
    let truncated = rows.len() > maximum;
    rows.truncate(maximum);
    let next_cursor = truncated.then(|| rows.last().map(&cursor)).flatten();
    Ok(OrganizationPage {
        items: rows
            .into_iter()
            .map(decode)
            .collect::<AdapterResult<Vec<_>>>()?,
        next_cursor,
    })
}

fn safe_bool(value: i64) -> AdapterResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_invite_state(value: &str) -> AdapterResult<InviteState> {
    match value {
        "pending" => Ok(InviteState::Pending),
        "accepted" => Ok(InviteState::Accepted),
        "declined" => Ok(InviteState::Declined),
        "revoked" => Ok(InviteState::Revoked),
        "expired" => Ok(InviteState::Expired),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn parse_space_role(value: &str) -> AdapterResult<SpaceRole> {
    match value {
        "manager" => Ok(SpaceRole::Manager),
        "contributor" => Ok(SpaceRole::Contributor),
        "viewer" => Ok(SpaceRole::Viewer),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn row_scope(organization_id: &str, expected: OrganizationScope) -> AdapterResult<()> {
    let parsed = OrganizationId::parse(organization_id).map_err(|_| AdapterFailure::Corrupt)?;
    (parsed == expected.organization_id)
        .then_some(())
        .ok_or(AdapterFailure::Corrupt)
}

fn decode_membership_row(
    row: MembershipRow,
    scope: OrganizationScope,
) -> AdapterResult<OrganizationMembershipRecord> {
    row_scope(&row.organization_id, scope)?;
    Ok(OrganizationMembershipRecord {
        scope,
        user_id: UserId::parse(&row.user_id).map_err(|_| AdapterFailure::Corrupt)?,
        role: parse_role(&row.role)?,
        state: parse_membership_state(&row.state)?,
        has_pro_seat: safe_bool(row.has_pro_seat)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        revision: safe_revision(row.revision)?,
        authority_version: safe_revision(row.authority_version)?,
    })
}

fn decode_invite_summary_row(
    row: InviteSummaryRow,
    scope: OrganizationScope,
) -> AdapterResult<OrganizationInviteSummary> {
    row_scope(&row.organization_id, scope)?;
    Ok(OrganizationInviteSummary {
        id: frame_domain::OrganizationInviteId::parse(&row.id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        invited_by_user_id: UserId::parse(&row.invited_by_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        accepted_by_user_id: row
            .accepted_by_user_id
            .map(|value| UserId::parse(&value).map_err(|_| AdapterFailure::Corrupt))
            .transpose()?,
        role: parse_role(&row.role)?,
        state: parse_invite_state(&row.status)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        expires_at: safe_timestamp(row.expires_at_ms)?,
        resolved_at: row.resolved_at_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_domain_row(
    row: DomainRow,
    scope: OrganizationScope,
) -> AdapterResult<AllowedDomainRecord> {
    row_scope(&row.organization_id, scope)?;
    Ok(AllowedDomainRecord {
        scope,
        domain: AllowedDomain::parse(row.domain_ascii).map_err(|_| AdapterFailure::Corrupt)?,
        verified_at: row.verified_at_ms.map(safe_timestamp).transpose()?,
        created_at: safe_timestamp(row.created_at_ms)?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_space_row(row: SpaceRow, scope: OrganizationScope) -> AdapterResult<SpaceRecord> {
    row_scope(&row.organization_id, scope)?;
    Ok(SpaceRecord {
        id: SpaceId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        created_by_user_id: UserId::parse(&row.created_by_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        name: CollaborationName::parse(row.name).map_err(|_| AdapterFailure::Corrupt)?,
        is_primary: safe_bool(row.is_primary)?,
        is_public: safe_bool(row.is_public)?,
        settings: OrganizationSettings::parse(row.settings_json)
            .map_err(|_| AdapterFailure::Corrupt)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        deleted_at: row.deleted_at_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_space_membership_row(
    row: SpaceMembershipRow,
    scope: OrganizationScope,
    expected_space_id: SpaceId,
) -> AdapterResult<SpaceMembershipRecord> {
    row_scope(&row.organization_id, scope)?;
    let space_id = SpaceId::parse(&row.space_id).map_err(|_| AdapterFailure::Corrupt)?;
    if space_id != expected_space_id {
        return Err(AdapterFailure::Corrupt);
    }
    Ok(SpaceMembershipRecord {
        scope,
        space_id,
        user_id: UserId::parse(&row.user_id).map_err(|_| AdapterFailure::Corrupt)?,
        role: parse_space_role(&row.role)?,
        state: parse_membership_state(&row.state)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        revision: safe_revision(row.revision)?,
    })
}

fn decode_folder_row(
    row: FolderRow,
    scope: OrganizationScope,
    expected_space_id: SpaceId,
) -> AdapterResult<FolderRecord> {
    row_scope(&row.organization_id, scope)?;
    let space_id = SpaceId::parse(&row.space_id).map_err(|_| AdapterFailure::Corrupt)?;
    if space_id != expected_space_id {
        return Err(AdapterFailure::Corrupt);
    }
    Ok(FolderRecord {
        id: FolderId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?,
        scope,
        space_id,
        parent_id: row
            .parent_id
            .map(|value| FolderId::parse(&value).map_err(|_| AdapterFailure::Corrupt))
            .transpose()?,
        created_by_user_id: UserId::parse(&row.created_by_user_id)
            .map_err(|_| AdapterFailure::Corrupt)?,
        name: CollaborationName::parse(row.name).map_err(|_| AdapterFailure::Corrupt)?,
        is_public: safe_bool(row.is_public)?,
        settings: OrganizationSettings::parse(row.settings_json)
            .map_err(|_| AdapterFailure::Corrupt)?,
        depth: u8::try_from(row.depth).map_err(|_| AdapterFailure::Corrupt)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        deleted_at: row.deleted_at_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
        tree_revision: safe_revision(row.tree_revision)?,
    })
}

#[async_trait]
impl OrganizationRepository for D1OrganizationRepository<'_> {
    async fn snapshot(
        &self,
        request: OrganizationReadRequest,
    ) -> Result<OrganizationSnapshot, OrganizationPortError> {
        self.snapshot_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_members(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<OrganizationMembershipRecord>, OrganizationPortError> {
        self.list_members_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_invites(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<OrganizationInviteSummary>, OrganizationPortError> {
        self.list_invites_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_allowed_domains(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<AllowedDomainRecord>, OrganizationPortError> {
        self.list_allowed_domains_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_spaces(
        &self,
        request: OrganizationCollectionRequest,
    ) -> Result<OrganizationPage<SpaceRecord>, OrganizationPortError> {
        self.list_spaces_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_space_members(
        &self,
        request: OrganizationSpaceCollectionRequest,
    ) -> Result<OrganizationPage<SpaceMembershipRecord>, OrganizationPortError> {
        self.list_space_members_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn list_folders(
        &self,
        request: OrganizationSpaceCollectionRequest,
    ) -> Result<OrganizationPage<FolderRecord>, OrganizationPortError> {
        self.list_folders_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn create_organization(
        &self,
        command: CreateOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.create_organization_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn set_active_organization(
        &self,
        command: SetActiveOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.set_active_organization_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn issue_invite(
        &self,
        command: IssueOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.issue_invite_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn revoke_invite(
        &self,
        command: RevokeOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.revoke_invite_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn accept_invite(
        &self,
        command: AcceptOrganizationInviteCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.accept_invite_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn transfer_ownership(
        &self,
        command: TransferOrganizationOwnershipCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.transfer_ownership_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn change_member(
        &self,
        command: ChangeOrganizationMemberCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.change_member_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn upsert_allowed_domain(
        &self,
        command: UpsertAllowedDomainCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.upsert_allowed_domain_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn update_settings(
        &self,
        command: UpdateOrganizationSettingsCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.update_settings_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn create_space(
        &self,
        command: CreateSpaceCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.create_space_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn update_space(
        &self,
        command: UpdateSpaceCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.update_space_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn change_space_role(
        &self,
        command: ChangeSpaceRoleCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.change_space_role_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn create_folder(
        &self,
        command: CreateFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.create_folder_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn update_folder(
        &self,
        command: UpdateFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.update_folder_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn move_folder(
        &self,
        command: MoveFolderCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.move_folder_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn tombstone_organization(
        &self,
        command: TombstoneOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.tombstone_organization_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn recover_organization(
        &self,
        command: RecoverOrganizationCommand,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.recover_organization_inner(command)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn audit_graph(
        &self,
        request: OrganizationGraphAuditRequest,
    ) -> Result<OrganizationGraphAudit, OrganizationPortError> {
        self.audit_graph_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn plan_repair(
        &self,
        request: OrganizationGraphAuditRequest,
    ) -> Result<OrganizationRepairPlan, OrganizationPortError> {
        self.plan_repair_inner(request)
            .await
            .map_err(AdapterFailure::into_port)
    }

    async fn audit_decision(
        &self,
        decision: OrganizationAuditDecision,
    ) -> Result<(), OrganizationPortError> {
        self.audit_decision_inner(decision)
            .await
            .map_err(AdapterFailure::into_port)
    }
}

#[async_trait]
impl LegacyOrganizationSelectionRepositoryV1 for D1OrganizationRepository<'_> {
    async fn legacy_set_active_organization(
        &self,
        command: LegacySetActiveOrganizationCommandV1,
    ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
        self.legacy_set_active_organization_inner(command)
            .await
            .map_err(|failure| match failure {
                // A failed compatibility predicate is a non-disclosing target
                // denial, not a caller-visible optimistic concurrency error.
                AdapterFailure::Stale => OrganizationPortError::AccessDenied,
                other => other.into_port(),
            })
    }
}

impl D1OrganizationRepository<'_> {
    async fn snapshot_inner(
        &self,
        request: OrganizationReadRequest,
    ) -> AdapterResult<OrganizationSnapshot> {
        let row = self
            .one::<SnapshotRow>(
                SNAPSHOT_SQL,
                &[
                    string(request.scope.organization_id),
                    string(request.actor_id),
                    integer(request.identity_revision)?,
                    integer(request.session_version)?,
                ],
            )
            .await?
            .ok_or_else(|| {
                OrganizationRepositoryTelemetry::emit("snapshot", "deny", 0);
                AdapterFailure::AccessDenied
            })?;
        let snapshot = decode_snapshot(row, request)?;
        OrganizationRepositoryTelemetry::emit("snapshot", "allow", 1);
        Ok(snapshot)
    }

    async fn list_members_inner(
        &self,
        request: OrganizationCollectionRequest,
    ) -> AdapterResult<OrganizationPage<OrganizationMembershipRecord>> {
        let limit = collection_limit(&request)?;
        let scope = request.authority.scope;
        let rows = self
            .authorized_rows::<MembershipRow>(
                request.authority,
                "any",
                self.statement(
                    MEMBERS_LIST_SQL,
                    &[
                        string(scope.organization_id),
                        optional_string(request.after_id.clone()),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )?,
            )
            .await?;
        into_page(
            rows,
            request.maximum_items,
            |row| row.user_id.clone(),
            |row| decode_membership_row(row, scope),
        )
    }

    async fn list_invites_inner(
        &self,
        request: OrganizationCollectionRequest,
    ) -> AdapterResult<OrganizationPage<OrganizationInviteSummary>> {
        let limit = collection_limit(&request)?;
        let scope = request.authority.scope;
        let rows = self
            .authorized_rows::<InviteSummaryRow>(
                request.authority,
                "admin",
                self.statement(
                    INVITES_LIST_SQL,
                    &[
                        string(scope.organization_id),
                        optional_string(request.after_id.clone()),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )?,
            )
            .await?;
        into_page(
            rows,
            request.maximum_items,
            |row| row.id.clone(),
            |row| decode_invite_summary_row(row, scope),
        )
    }

    async fn list_allowed_domains_inner(
        &self,
        request: OrganizationCollectionRequest,
    ) -> AdapterResult<OrganizationPage<AllowedDomainRecord>> {
        let limit = collection_limit(&request)?;
        let scope = request.authority.scope;
        let rows = self
            .authorized_rows::<DomainRow>(
                request.authority,
                "admin",
                self.statement(
                    DOMAINS_LIST_SQL,
                    &[
                        string(scope.organization_id),
                        optional_string(request.after_id.clone()),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )?,
            )
            .await?;
        into_page(
            rows,
            request.maximum_items,
            |row| row.domain_ascii.clone(),
            |row| decode_domain_row(row, scope),
        )
    }

    async fn list_spaces_inner(
        &self,
        request: OrganizationCollectionRequest,
    ) -> AdapterResult<OrganizationPage<SpaceRecord>> {
        let limit = collection_limit(&request)?;
        let scope = request.authority.scope;
        let rows = self
            .authorized_rows::<SpaceRow>(
                request.authority,
                "any",
                self.statement(
                    SPACES_LIST_SQL,
                    &[
                        string(scope.organization_id),
                        optional_string(request.after_id.clone()),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )?,
            )
            .await?;
        into_page(
            rows,
            request.maximum_items,
            |row| row.id.clone(),
            |row| decode_space_row(row, scope),
        )
    }

    async fn list_space_members_inner(
        &self,
        request: OrganizationSpaceCollectionRequest,
    ) -> AdapterResult<OrganizationPage<SpaceMembershipRecord>> {
        let limit = collection_limit(&request.collection)?;
        let scope = request.collection.authority.scope;
        let rows = self
            .authorized_rows::<SpaceMembershipRow>(
                request.collection.authority,
                "any",
                self.statement(
                    SPACE_MEMBERS_LIST_SQL,
                    &[
                        string(scope.organization_id),
                        string(request.space_id),
                        optional_string(request.collection.after_id.clone()),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )?,
            )
            .await?;
        into_page(
            rows,
            request.collection.maximum_items,
            |row| row.user_id.clone(),
            |row| decode_space_membership_row(row, scope, request.space_id),
        )
    }

    async fn list_folders_inner(
        &self,
        request: OrganizationSpaceCollectionRequest,
    ) -> AdapterResult<OrganizationPage<FolderRecord>> {
        let limit = collection_limit(&request.collection)?;
        let scope = request.collection.authority.scope;
        let rows = self
            .authorized_rows::<FolderRow>(
                request.collection.authority,
                "any",
                self.statement(
                    FOLDERS_LIST_SQL,
                    &[
                        string(scope.organization_id),
                        string(request.space_id),
                        optional_string(request.collection.after_id.clone()),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )?,
            )
            .await?;
        into_page(
            rows,
            request.collection.maximum_items,
            |row| row.id.clone(),
            |row| decode_folder_row(row, scope, request.space_id),
        )
    }

    async fn create_organization_inner(
        &self,
        command: CreateOrganizationCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        let organization = &command.organization;
        let owner = &command.owner_membership;
        let space = &command.primary_space;
        if context.action != OrganizationAction::CreateOrganization
            || organization.scope != context.authority.scope
            || owner.scope != context.authority.scope
            || space.scope != context.authority.scope
            || organization.owner_id != context.authority.actor_id
            || owner.user_id != context.authority.actor_id
            || owner.role != OrganizationRole::Owner
            || owner.state != MembershipState::Active
            || organization.status != OrganizationStatus::Active
            || organization.revision != OrganizationRevision::INITIAL
            || organization.authority_version != OrganizationRevision::INITIAL
            || owner.revision != OrganizationRevision::INITIAL
            || owner.authority_version != OrganizationRevision::INITIAL
            || space.created_by_user_id != context.authority.actor_id
            || !space.is_primary
            || space.name.as_str().len() > 160
            || space.deleted_at.is_some()
            || space.revision != OrganizationRevision::INITIAL
        {
            return Err(AdapterFailure::Invalid);
        }
        let organization_id = organization.scope.organization_id.to_string();
        let fingerprint = semantic_fingerprint(
            "organization_create",
            context.authority.actor_id,
            &serde_json::json!({
                "organization": organization,
                "owner_membership": owner,
                "primary_space": space,
            }),
        )?;
        let statements = vec![
            self.principal_statement(context)?,
            self.statement(
                ORGANIZATION_ABSENT_ASSERT_SQL,
                &[
                    string(format!(
                        "{}:organization_absent",
                        context.authority.operation_id
                    )),
                    JsValue::from_str(&organization_id),
                ],
            )?,
            self.statement(
                ORGANIZATION_INSERT_SQL,
                &[
                    JsValue::from_str(&organization_id),
                    string(organization.owner_id),
                    JsValue::from_str(organization.name.as_str()),
                    JsValue::from_str(organization.settings.as_json()),
                    timestamp_value(organization.created_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                OWNER_MEMBERSHIP_INSERT_SQL,
                &[
                    JsValue::from_str(&organization_id),
                    string(owner.user_id),
                    boolean_value(owner.has_pro_seat),
                    timestamp_value(owner.created_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.space_insert_statement(context, space)?,
            self.statement(
                ORGANIZATION_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    JsValue::from_str(&organization_id),
                    JsValue::from_str("active"),
                    JsValue::from_f64(0.0),
                    JsValue::from_f64(0.0),
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "organization_create",
            organization_id,
            OrganizationMutationResult::Created,
            OrganizationRevision::INITIAL,
            OrganizationRevision::INITIAL,
            fingerprint,
            statements,
        )
        .await
    }

    async fn set_active_organization_inner(
        &self,
        command: SetActiveOrganizationCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::ReadOrganization {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_selection_revision)?;
        let subject = context.authority.actor_id.to_string();
        let fingerprint = semantic_fingerprint(
            "active_organization_set",
            context.authority.actor_id,
            &serde_json::json!({
                "default_organization_id": command.default_organization_id,
                "active_organization_id": command.active_organization_id,
                "expected_selection_revision": command.expected_selection_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "any")?,
            self.statement(
                SELECTION_UPSERT_SQL,
                &[
                    string(context.authority.actor_id),
                    integer(command.expected_selection_revision)?,
                    optional_string(command.default_organization_id),
                    optional_string(command.active_organization_id),
                    string(context.authority.operation_id),
                    timestamp_value(context.authority.occurred_at),
                ],
            )?,
            self.statement(
                SELECTION_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.actor_id),
                    integer(next)?,
                    optional_string(command.default_organization_id),
                    optional_string(command.active_organization_id),
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "active_organization_set",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn legacy_set_active_organization_inner(
        &self,
        command: LegacySetActiveOrganizationCommandV1,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let operation_id = OrganizationOperationId::new();
        let operation_id_value = operation_id.to_string();
        // The source operations accepted no idempotency key. A fresh server
        // key intentionally makes every retry execute and preserves their
        // last-write-wins behavior.
        let idempotency_key = format!("legacy-auto-{operation_id_value}");
        let actor_id = command.actor_id.to_string();
        let organization_id = command.active_organization_id.to_string();
        let fingerprint = semantic_fingerprint(
            "active_organization_set",
            command.actor_id,
            &serde_json::json!({
                "active_organization_id": command.active_organization_id,
                "authorization": command.authorization.stable_code(),
                "lifecycle": command.lifecycle.stable_code(),
                "source": "legacy_active_only_v1",
            }),
        )?;
        let statements = vec![
            self.statement(
                LEGACY_SELECTION_AUTHORITY_ASSERT_SQL,
                &[
                    JsValue::from_str(&format!("{operation_id_value}:legacy_authority")),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&organization_id),
                    JsValue::from_str(command.lifecycle.stable_code()),
                    JsValue::from_str(command.authorization.stable_code()),
                ],
            )?,
            self.statement(
                LEGACY_SELECTION_ACTIVE_UPDATE_SQL,
                &[
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&organization_id),
                    JsValue::from_str(&operation_id_value),
                    timestamp_value(command.occurred_at),
                ],
            )?,
            self.statement(
                LEGACY_SELECTION_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&format!("{operation_id_value}:legacy_post")),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(&organization_id),
                    JsValue::from_str(&operation_id_value),
                ],
            )?,
            self.statement(
                LEGACY_SELECTION_OPERATION_INSERT_SQL,
                &[
                    JsValue::from_str(&operation_id_value),
                    JsValue::from_str(&organization_id),
                    JsValue::from_str(&idempotency_key),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str(fingerprint.expose_for_verification()),
                    timestamp_value(command.occurred_at),
                ],
            )?,
            self.statement(
                ASSERTION_CLEANUP_SQL,
                &[JsValue::from_str(&operation_id_value)],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                &[
                    string(OrganizationAuditId::new()),
                    JsValue::from_str(&operation_id_value),
                    JsValue::from_str(&organization_id),
                    JsValue::from_str(&actor_id),
                    JsValue::from_str("active_organization_set"),
                    JsValue::from_str("organization"),
                    JsValue::from_str(&hex_sha256(actor_id.as_bytes())),
                    JsValue::from_str("allow"),
                    JsValue::NULL,
                    timestamp_value(command.occurred_at),
                ],
            )?,
        ];
        match self.batch(statements).await {
            Ok(()) => {
                OrganizationRepositoryTelemetry::emit("legacy_active_organization_set", "allow", 1)
            }
            Err(failure) => {
                OrganizationRepositoryTelemetry::emit(
                    "legacy_active_organization_set",
                    if failure == AdapterFailure::Stale {
                        "deny"
                    } else {
                        "unavailable"
                    },
                    0,
                );
                return Err(failure);
            }
        }
        let row = self
            .one::<OperationRow>(
                LEGACY_SELECTION_RECEIPT_SQL,
                &[JsValue::from_str(&operation_id_value)],
            )
            .await?
            .ok_or(AdapterFailure::Corrupt)?;
        if row.organization_id != organization_id
            || row.idempotency_key != idempotency_key
            || row.operation_id != operation_id_value
            || row.operation_kind != "active_organization_set"
            || row.subject_id != actor_id
            || row.request_fingerprint != fingerprint.expose_for_verification()
        {
            return Err(AdapterFailure::Corrupt);
        }
        decode_receipt(row, false)
    }

    async fn issue_invite_inner(
        &self,
        command: IssueOrganizationInviteCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        let invite = &command.invite;
        if context.action != OrganizationAction::IssueInvite
            || invite.scope != context.authority.scope
            || invite.invited_by_user_id != context.authority.actor_id
            || invite.accepted_by_user_id.is_some()
            || invite.state != InviteState::Pending
            || invite.role == OrganizationRole::Owner
            || invite.revision != OrganizationRevision::INITIAL
            || invite.created_at > context.authority.occurred_at
            || invite.expires_at <= context.authority.occurred_at
            || invite.resolved_at.is_some()
        {
            return Err(AdapterFailure::Invalid);
        }
        let subject = invite.id.to_string();
        let fingerprint = semantic_fingerprint(
            "invite_issue",
            context.authority.actor_id,
            &serde_json::json!({ "invite": invite }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "admin")?,
            self.statement(
                INVITE_INSERT_SQL,
                &[
                    string(invite.id),
                    string(invite.scope.organization_id),
                    JsValue::from_f64(f64::from(
                        invite.invited_identifier_digest.key_version.get(),
                    )),
                    JsValue::from_str(
                        invite
                            .invited_identifier_digest
                            .digest
                            .expose_for_verification(),
                    ),
                    string(invite.invited_by_user_id),
                    JsValue::from_str(role(invite.role)),
                    JsValue::from_str(invite.token_digest.expose_for_verification()),
                    timestamp_value(invite.created_at),
                    timestamp_value(invite.expires_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                INVITE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(invite.id),
                    string(invite.scope.organization_id),
                    JsValue::from_str("pending"),
                    JsValue::from_f64(0.0),
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "invite_issue",
            subject,
            OrganizationMutationResult::Created,
            OrganizationRevision::INITIAL,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn revoke_invite_inner(
        &self,
        command: RevokeOrganizationInviteCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::RevokeInvite {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_invite_revision)?;
        let subject = command.invite_id.to_string();
        let fingerprint = semantic_fingerprint(
            "invite_revoke",
            context.authority.actor_id,
            &serde_json::json!({
                "invite_id": command.invite_id,
                "expected_invite_revision": command.expected_invite_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "admin")?,
            self.statement(
                INVITE_REVOKE_SQL,
                &[
                    string(command.invite_id),
                    string(context.authority.scope.organization_id),
                    integer(command.expected_invite_revision)?,
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                INVITE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(command.invite_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str("revoked"),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "invite_revoke",
            subject,
            OrganizationMutationResult::Revoked,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn accept_invite_inner(
        &self,
        command: AcceptOrganizationInviteCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::AcceptInvite {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_invite_revision)?;
        let subject = command.invite_id.to_string();
        let fingerprint = semantic_fingerprint(
            "invite_accept",
            context.authority.actor_id,
            &serde_json::json!({
                "invite_id": command.invite_id,
                "presented_token_digest": command.presented_token_digest,
                "expected_invite_revision": command.expected_invite_revision,
            }),
        )?;
        let statements = vec![
            self.statement(
                INVITE_ACCEPT_ELIGIBILITY_ASSERT_SQL,
                &[
                    string(format!("{}:eligibility", context.authority.operation_id)),
                    string(command.invite_id),
                    string(context.authority.scope.organization_id),
                    integer(command.expected_invite_revision)?,
                    string(context.authority.actor_id),
                    JsValue::from_str(command.presented_token_digest.expose_for_verification()),
                    integer(context.authority.fence.identity_revision)?,
                    integer(context.authority.fence.session_version)?,
                ],
            )?,
            self.statement(
                INVITE_ACCEPT_SQL,
                &[
                    string(command.invite_id),
                    string(context.authority.scope.organization_id),
                    integer(command.expected_invite_revision)?,
                    JsValue::from_str(command.presented_token_digest.expose_for_verification()),
                    string(context.authority.actor_id),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                INVITE_ACCEPT_MEMBERSHIP_SQL,
                &[
                    string(command.invite_id),
                    string(context.authority.actor_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                INVITE_MEMBERSHIP_POSTCONDITION_SQL,
                &[
                    string(format!(
                        "{}:membership_post",
                        context.authority.operation_id
                    )),
                    string(command.invite_id),
                    string(context.authority.scope.organization_id),
                    string(context.authority.actor_id),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                INVITE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(command.invite_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str("accepted"),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "invite_accept",
            subject,
            OrganizationMutationResult::Accepted,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn transfer_ownership_inner(
        &self,
        command: TransferOrganizationOwnershipCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::TransferOwnership
            || command.new_owner_id == context.authority.actor_id
        {
            return Err(AdapterFailure::Invalid);
        }
        let next_org = expected_next(context.authority.fence.organization_revision)?;
        let next_authority = expected_next(context.authority.fence.organization_authority_version)?;
        let subject = command.new_owner_id.to_string();
        let fingerprint = semantic_fingerprint(
            "ownership_transfer",
            context.authority.actor_id,
            &serde_json::json!({
                "new_owner_id": command.new_owner_id,
                "expected_new_owner_membership_revision": command.expected_new_owner_membership_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "owner")?,
            self.statement(
                OWNERSHIP_TARGET_ASSERT_SQL,
                &[
                    string(format!("{}:target", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    string(command.new_owner_id),
                    integer(command.expected_new_owner_membership_revision)?,
                ],
            )?,
            self.statement(
                OWNERSHIP_OLD_DEMOTE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(context.authority.actor_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                OWNERSHIP_POINTER_UPDATE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(command.new_owner_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                OWNERSHIP_NEW_PROMOTE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(command.new_owner_id),
                    integer(command.expected_new_owner_membership_revision)?,
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                PRINCIPAL_AUTHORITY_BUMP_SQL,
                &[
                    string(context.authority.actor_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                PRINCIPAL_GRANTS_REVOKE_SQL,
                &[string(context.authority.actor_id)],
            )?,
            self.statement(
                PRINCIPAL_AUTHORITY_BUMP_SQL,
                &[
                    string(command.new_owner_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(PRINCIPAL_GRANTS_REVOKE_SQL, &[string(command.new_owner_id)])?,
            self.statement(
                PRINCIPAL_BUMP_POSTCONDITION_SQL,
                &[
                    string(format!(
                        "{}:old_principal_post",
                        context.authority.operation_id
                    )),
                    string(context.authority.actor_id),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                PRINCIPAL_BUMP_POSTCONDITION_SQL,
                &[
                    string(format!(
                        "{}:new_principal_post",
                        context.authority.operation_id
                    )),
                    string(command.new_owner_id),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                OWNERSHIP_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    string(command.new_owner_id),
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "ownership_transfer",
            subject,
            OrganizationMutationResult::Applied,
            next_org,
            next_authority,
            fingerprint,
            statements,
        )
        .await
    }

    async fn change_member_inner(
        &self,
        command: ChangeOrganizationMemberCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if !matches!(
            context.action,
            OrganizationAction::ChangeMemberRole | OrganizationAction::RemoveMember
        ) || command.role == OrganizationRole::Owner
        {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_subject_revision)?;
        let subject = command.subject_user_id.to_string();
        let fingerprint = semantic_fingerprint(
            "member_change",
            context.authority.actor_id,
            &serde_json::json!({
                "subject_user_id": command.subject_user_id,
                "role": command.role,
                "state": command.state,
                "has_pro_seat": command.has_pro_seat,
                "expected_subject_revision": command.expected_subject_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "admin")?,
            self.statement(
                MEMBER_TARGET_ASSERT_SQL,
                &[
                    string(format!("{}:target", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    string(command.subject_user_id),
                    integer(command.expected_subject_revision)?,
                ],
            )?,
            self.statement(
                MEMBER_CHANGE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(command.subject_user_id),
                    integer(command.expected_subject_revision)?,
                    JsValue::from_str(role(command.role)),
                    JsValue::from_str(membership_state(command.state)),
                    boolean_value(command.has_pro_seat),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                PRINCIPAL_AUTHORITY_BUMP_SQL,
                &[
                    string(command.subject_user_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                PRINCIPAL_GRANTS_REVOKE_SQL,
                &[string(command.subject_user_id)],
            )?,
            self.statement(
                PRINCIPAL_BUMP_POSTCONDITION_SQL,
                &[
                    string(format!("{}:principal_post", context.authority.operation_id)),
                    string(command.subject_user_id),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                MEMBER_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    string(command.subject_user_id),
                    JsValue::from_str(role(command.role)),
                    JsValue::from_str(membership_state(command.state)),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "member_change",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn upsert_allowed_domain_inner(
        &self,
        command: UpsertAllowedDomainCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::ManageAllowedDomain {
            return Err(AdapterFailure::Invalid);
        }
        let (expected_binding, next) = match command.expected_revision {
            Some(expected) => (integer(expected)?, expected_next(expected)?),
            None => (JsValue::from_f64(-1.0), OrganizationRevision::INITIAL),
        };
        let subject = command.domain.as_str().to_owned();
        let fingerprint = semantic_fingerprint(
            "allowed_domain_upsert",
            context.authority.actor_id,
            &serde_json::json!({
                "domain": command.domain,
                "verified_at": command.verified_at,
                "expected_revision": command.expected_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "admin")?,
            self.statement(
                DOMAIN_CAPACITY_ASSERT_SQL,
                &[
                    string(format!("{}:capacity", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(command.domain.as_str()),
                ],
            )?,
            self.statement(
                DOMAIN_UPSERT_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(command.domain.as_str()),
                    optional_timestamp(command.verified_at),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                    expected_binding,
                ],
            )?,
            self.statement(
                DOMAIN_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(command.domain.as_str()),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "allowed_domain_upsert",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn update_settings_inner(
        &self,
        command: UpdateOrganizationSettingsCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::ManageSettings {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(context.authority.fence.organization_revision)?;
        let subject = context.authority.scope.organization_id.to_string();
        let fingerprint = semantic_fingerprint(
            "settings_update",
            context.authority.actor_id,
            &serde_json::json!({
                "name": command.name,
                "settings": command.settings,
                "expected_organization_revision": context.authority.fence.organization_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "admin")?,
            self.statement(
                SETTINGS_UPDATE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(command.name.as_str()),
                    JsValue::from_str(command.settings.as_json()),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                ORGANIZATION_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str("active"),
                    integer(next)?,
                    integer(context.authority.fence.organization_authority_version)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "settings_update",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn create_space_inner(
        &self,
        command: CreateSpaceCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        let space = &command.space;
        if context.action != OrganizationAction::CreateSpace
            || space.scope != context.authority.scope
            || space.created_by_user_id != context.authority.actor_id
            || space.is_primary
            || space.name.as_str().len() > 160
            || space.deleted_at.is_some()
            || space.revision != OrganizationRevision::INITIAL
        {
            return Err(AdapterFailure::Invalid);
        }
        let subject = space.id.to_string();
        let fingerprint = semantic_fingerprint(
            "space_create",
            context.authority.actor_id,
            &serde_json::json!({ "space": space }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "write")?,
            self.space_insert_statement(context, space)?,
            self.statement(
                SPACE_ROLE_UPSERT_SQL,
                &[
                    string(space.id),
                    string(context.authority.actor_id),
                    JsValue::from_str("manager"),
                    JsValue::from_str("active"),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_f64(-1.0),
                ],
            )?,
            self.statement(
                SPACE_ROLE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:creator_post", context.authority.operation_id)),
                    string(space.id),
                    string(context.authority.actor_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str("manager"),
                    JsValue::from_str("active"),
                    JsValue::from_f64(0.0),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                SPACE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(space.id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_f64(0.0),
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "space_create",
            subject,
            OrganizationMutationResult::Created,
            OrganizationRevision::INITIAL,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn update_space_inner(
        &self,
        command: UpdateSpaceCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::ManageSpace || command.name.as_str().len() > 160 {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_revision)?;
        let subject = command.space_id.to_string();
        let fingerprint = semantic_fingerprint(
            "space_update",
            context.authority.actor_id,
            &serde_json::json!({
                "space_id": command.space_id,
                "name": command.name,
                "is_public": command.is_public,
                "settings": command.settings,
                "expected_revision": command.expected_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "write")?,
            self.space_authority_statement(
                context,
                command.space_id,
                command.expected_revision,
                "manager",
            )?,
            self.statement(
                SPACE_UPDATE_SQL,
                &[
                    string(command.space_id),
                    string(context.authority.scope.organization_id),
                    integer(command.expected_revision)?,
                    JsValue::from_str(command.name.as_str()),
                    boolean_value(command.is_public),
                    JsValue::from_str(command.settings.as_json()),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                SPACE_UPDATE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(command.space_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(command.name.as_str()),
                    boolean_value(command.is_public),
                    JsValue::from_str(command.settings.as_json()),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "space_update",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    fn space_insert_statement(
        &self,
        context: &OrganizationMutationContext,
        space: &frame_domain::SpaceRecord,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            SPACE_INSERT_SQL,
            &[
                string(space.id),
                string(space.scope.organization_id),
                string(space.created_by_user_id),
                JsValue::from_str(space.name.as_str()),
                boolean_value(space.is_primary),
                boolean_value(space.is_public),
                JsValue::from_str(space.settings.as_json()),
                timestamp_value(space.created_at),
                string(context.authority.operation_id),
            ],
        )
    }

    async fn change_space_role_inner(
        &self,
        command: ChangeSpaceRoleCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::ChangeSpaceRole {
            return Err(AdapterFailure::Invalid);
        }
        let (expected_binding, next) = match command.expected_revision {
            Some(expected) => (integer(expected)?, expected_next(expected)?),
            None => (JsValue::from_f64(-1.0), OrganizationRevision::INITIAL),
        };
        let subject = format!("{}:{}", command.space_id, command.subject_user_id);
        let fingerprint = semantic_fingerprint(
            "space_role_change",
            context.authority.actor_id,
            &serde_json::json!({
                "space_id": command.space_id,
                "subject_user_id": command.subject_user_id,
                "role": command.role,
                "state": command.state,
                "expected_space_revision": command.expected_space_revision,
                "expected_revision": command.expected_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "write")?,
            self.space_authority_statement(
                context,
                command.space_id,
                command.expected_space_revision,
                "manager",
            )?,
            self.statement(
                SPACE_ROLE_UPSERT_SQL,
                &[
                    string(command.space_id),
                    string(command.subject_user_id),
                    JsValue::from_str(space_role(command.role)),
                    JsValue::from_str(membership_state(command.state)),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                    string(context.authority.scope.organization_id),
                    expected_binding,
                ],
            )?,
            self.statement(
                PRINCIPAL_AUTHORITY_BUMP_SQL,
                &[
                    string(command.subject_user_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                PRINCIPAL_GRANTS_REVOKE_SQL,
                &[string(command.subject_user_id)],
            )?,
            self.statement(
                PRINCIPAL_BUMP_POSTCONDITION_SQL,
                &[
                    string(format!("{}:principal_post", context.authority.operation_id)),
                    string(command.subject_user_id),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                SPACE_ROLE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(command.space_id),
                    string(command.subject_user_id),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str(space_role(command.role)),
                    JsValue::from_str(membership_state(command.state)),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "space_role_change",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    fn space_authority_statement(
        &self,
        context: &OrganizationMutationContext,
        space_id: frame_domain::SpaceId,
        space_revision: OrganizationRevision,
        role_class: &'static str,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            SPACE_AUTHORITY_ASSERT_SQL,
            &[
                string(format!("{}:space", context.authority.operation_id)),
                string(space_id),
                string(context.authority.actor_id),
                string(context.authority.scope.organization_id),
                integer(space_revision)?,
                optional_revision(context.authority.fence.space_membership_revision)?,
                JsValue::from_str(role_class),
            ],
        )
    }

    async fn create_folder_inner(
        &self,
        command: CreateFolderCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        let folder = &command.folder;
        if context.action != OrganizationAction::CreateFolder
            || folder.scope != context.authority.scope
            || folder.created_by_user_id != context.authority.actor_id
            || folder.deleted_at.is_some()
            || folder.revision != OrganizationRevision::INITIAL
            || folder.tree_revision != OrganizationRevision::INITIAL
            || folder.depth > frame_domain::MAX_FOLDER_DEPTH
        {
            return Err(AdapterFailure::Invalid);
        }
        let subject = folder.id.to_string();
        let fingerprint = semantic_fingerprint(
            "folder_create",
            context.authority.actor_id,
            &serde_json::json!({
                "folder": folder,
                "expected_parent_revision": command.expected_parent_revision,
                "expected_space_revision": command.expected_space_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "write")?,
            self.space_authority_statement(
                context,
                folder.space_id,
                command.expected_space_revision,
                "write",
            )?,
            self.statement(
                FOLDER_INSERT_SQL,
                &[
                    string(folder.id),
                    string(folder.scope.organization_id),
                    string(folder.space_id),
                    optional_string(folder.parent_id),
                    string(folder.created_by_user_id),
                    JsValue::from_str(folder.name.as_str()),
                    boolean_value(folder.is_public),
                    JsValue::from_str(folder.settings.as_json()),
                    timestamp_value(folder.created_at),
                    integer(folder.tree_revision)?,
                    string(context.authority.operation_id),
                    optional_revision(command.expected_parent_revision)?,
                    integer(command.expected_space_revision)?,
                ],
            )?,
            self.statement(
                FOLDER_CLOSURE_INSERT_SQL,
                &[
                    string(folder.id),
                    string(folder.scope.organization_id),
                    string(folder.space_id),
                    optional_string(folder.parent_id),
                ],
            )?,
            self.statement(
                FOLDER_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(folder.id),
                    string(folder.scope.organization_id),
                    string(folder.space_id),
                    optional_string(folder.parent_id),
                    JsValue::from_f64(0.0),
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "folder_create",
            subject,
            OrganizationMutationResult::Created,
            OrganizationRevision::INITIAL,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn update_folder_inner(
        &self,
        command: UpdateFolderCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::ManageFolder {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_folder_revision)?;
        let subject = command.folder_id.to_string();
        let fingerprint = semantic_fingerprint(
            "folder_update",
            context.authority.actor_id,
            &serde_json::json!({
                "folder_id": command.folder_id,
                "space_id": command.space_id,
                "name": command.name,
                "is_public": command.is_public,
                "settings": command.settings,
                "expected_folder_revision": command.expected_folder_revision,
                "expected_space_revision": command.expected_space_revision,
                "expected_tree_revision": command.expected_tree_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "write")?,
            self.space_authority_statement(
                context,
                command.space_id,
                command.expected_space_revision,
                "write",
            )?,
            self.statement(
                FOLDER_MANAGE_ASSERT_SQL,
                &[
                    string(format!("{}:manage", context.authority.operation_id)),
                    string(command.folder_id),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    integer(command.expected_folder_revision)?,
                    integer(command.expected_tree_revision)?,
                    string(context.authority.actor_id),
                ],
            )?,
            self.statement(
                FOLDER_UPDATE_SQL,
                &[
                    string(command.folder_id),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    integer(command.expected_folder_revision)?,
                    JsValue::from_str(command.name.as_str()),
                    boolean_value(command.is_public),
                    JsValue::from_str(command.settings.as_json()),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                FOLDER_UPDATE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(command.folder_id),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    JsValue::from_str(command.name.as_str()),
                    boolean_value(command.is_public),
                    JsValue::from_str(command.settings.as_json()),
                    integer(next)?,
                    integer(command.expected_tree_revision)?,
                    string(context.authority.operation_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "folder_update",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn move_folder_inner(
        &self,
        command: MoveFolderCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::MoveFolder
            || command.new_parent_id == Some(command.folder_id)
        {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(command.expected_folder_revision)?;
        let subject = command.folder_id.to_string();
        let fingerprint = semantic_fingerprint(
            "folder_move",
            context.authority.actor_id,
            &serde_json::json!({
                "folder_id": command.folder_id,
                "space_id": command.space_id,
                "new_parent_id": command.new_parent_id,
                "expected_folder_revision": command.expected_folder_revision,
                "expected_space_revision": command.expected_space_revision,
                "expected_parent_revision": command.expected_parent_revision,
                "expected_tree_revision": command.expected_tree_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "write")?,
            self.space_authority_statement(
                context,
                command.space_id,
                command.expected_space_revision,
                "write",
            )?,
            self.statement(
                FOLDER_MOVE_ASSERT_SQL,
                &[
                    string(format!("{}:move", context.authority.operation_id)),
                    string(command.folder_id),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    integer(command.expected_folder_revision)?,
                    integer(command.expected_tree_revision)?,
                    optional_string(command.new_parent_id),
                    optional_revision(command.expected_parent_revision)?,
                    string(context.authority.actor_id),
                ],
            )?,
            self.statement(
                FOLDER_DESCENDANT_DEPTH_UPDATE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    string(command.folder_id),
                    optional_string(command.new_parent_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                FOLDER_CLOSURE_DELETE_SUBTREE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    string(command.folder_id),
                ],
            )?,
            self.statement(
                FOLDER_MOVE_SQL,
                &[
                    string(command.folder_id),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    optional_string(command.new_parent_id),
                    timestamp_value(context.authority.occurred_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                FOLDER_CLOSURE_MOVE_INSERT_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    string(command.folder_id),
                    optional_string(command.new_parent_id),
                ],
            )?,
            self.statement(
                FOLDER_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(command.folder_id),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    optional_string(command.new_parent_id),
                    integer(next)?,
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                FOLDER_TREE_POSTCONDITION_SQL,
                &[
                    string(format!("{}:tree_post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    string(command.space_id),
                    string(command.folder_id),
                ],
            )?,
        ];
        self.commit(
            context,
            "folder_move",
            subject,
            OrganizationMutationResult::Applied,
            next,
            context.authority.fence.organization_authority_version,
            fingerprint,
            statements,
        )
        .await
    }

    async fn tombstone_organization_inner(
        &self,
        command: TombstoneOrganizationCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::TombstoneOrganization {
            return Err(AdapterFailure::Invalid);
        }
        self.tombstone_policy
            .recovery_deadline(context.authority.occurred_at)
            .map_err(|_| AdapterFailure::Invalid)?;
        let next = expected_next(context.authority.fence.organization_revision)?;
        let next_authority = expected_next(context.authority.fence.organization_authority_version)?;
        let subject = context.authority.scope.organization_id.to_string();
        let fingerprint = semantic_fingerprint(
            "organization_tombstone",
            context.authority.actor_id,
            &serde_json::json!({
                "minimum_retention_ms": self.tombstone_policy.minimum_retention_ms,
                "maximum_recovery_ms": self.tombstone_policy.maximum_recovery_ms,
                "expected_organization_revision": context.authority.fence.organization_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "active", "owner")?,
            self.statement(
                TOMBSTONE_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    JsValue::from_f64(self.tombstone_policy.maximum_recovery_ms as f64),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                ORGANIZATION_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str("tombstoned"),
                    integer(next)?,
                    integer(next_authority)?,
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                TOMBSTONE_EVENT_INSERT_SQL,
                &[
                    string(context.authority.operation_id),
                    string(context.authority.scope.organization_id),
                    string(context.authority.actor_id),
                    JsValue::from_str("tombstoned"),
                ],
            )?,
        ];
        self.commit(
            context,
            "organization_tombstone",
            subject,
            OrganizationMutationResult::Tombstoned,
            next,
            next_authority,
            fingerprint,
            statements,
        )
        .await
    }

    async fn recover_organization_inner(
        &self,
        command: RecoverOrganizationCommand,
    ) -> AdapterResult<OrganizationMutationReceipt> {
        let context = &command.context;
        if context.action != OrganizationAction::RecoverOrganization {
            return Err(AdapterFailure::Invalid);
        }
        let next = expected_next(context.authority.fence.organization_revision)?;
        let next_authority = expected_next(context.authority.fence.organization_authority_version)?;
        let subject = context.authority.scope.organization_id.to_string();
        let fingerprint = semantic_fingerprint(
            "organization_recover",
            context.authority.actor_id,
            &serde_json::json!({
                "expected_tombstoned_at": command.expected_tombstoned_at,
                "expected_organization_revision": context.authority.fence.organization_revision,
            }),
        )?;
        let statements = vec![
            self.authority_statement(context, "tombstoned", "owner")?,
            self.statement(
                RECOVERY_RETENTION_ASSERT_SQL,
                &[
                    string(format!("{}:retention", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    timestamp_value(command.expected_tombstoned_at),
                ],
            )?,
            self.statement(
                RECOVER_SQL,
                &[
                    string(context.authority.scope.organization_id),
                    timestamp_value(command.expected_tombstoned_at),
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                ORGANIZATION_POSTCONDITION_SQL,
                &[
                    string(format!("{}:post", context.authority.operation_id)),
                    string(context.authority.scope.organization_id),
                    JsValue::from_str("active"),
                    integer(next)?,
                    integer(next_authority)?,
                    string(context.authority.operation_id),
                ],
            )?,
            self.statement(
                TOMBSTONE_EVENT_INSERT_SQL,
                &[
                    string(context.authority.operation_id),
                    string(context.authority.scope.organization_id),
                    string(context.authority.actor_id),
                    JsValue::from_str("recovered"),
                ],
            )?,
        ];
        self.commit(
            context,
            "organization_recover",
            subject,
            OrganizationMutationResult::Recovered,
            next,
            next_authority,
            fingerprint,
            statements,
        )
        .await
    }

    async fn authorized_graph_findings(
        &self,
        request: &OrganizationGraphAuditRequest,
        action: OrganizationAction,
        append_allow_audit: bool,
    ) -> AdapterResult<OrganizationGraphAudit> {
        if request.maximum_findings == 0 || request.maximum_findings > 10_000 {
            return Err(AdapterFailure::Invalid);
        }
        let limit = i64::from(request.maximum_findings) + 1;
        let bindings = [
            string(request.scope.organization_id),
            JsValue::from_f64(limit as f64),
        ];
        let operation_id = OrganizationOperationId::new();
        let mut statements = vec![
            self.support_statement(request, operation_id, "authority")?,
            self.statement(GRAPH_AUDIT_SQL, &bindings)?,
            self.statement(GRAPH_AUDIT_SELECTIONS_SQL, &bindings)?,
            self.statement(GRAPH_AUDIT_FOLDERS_SQL, &bindings)?,
            self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
        ];
        if append_allow_audit {
            statements.push(self.support_audit_statement(request, action, operation_id)?);
        }
        let results = match self.batch_results(statements).await {
            Err(AdapterFailure::Stale) => {
                self.record_support_denial(action);
                return Err(AdapterFailure::AccessDenied);
            }
            result => result?,
        };
        let expected_results = if append_allow_audit { 6 } else { 5 };
        if results.len() != expected_results {
            return Err(AdapterFailure::Corrupt);
        }
        let mut rows = Self::result_rows::<GraphFindingRow>(&results[1])?;
        rows.extend(Self::result_rows::<GraphFindingRow>(&results[2])?);
        rows.extend(Self::result_rows::<GraphFindingRow>(&results[3])?);
        rows.sort_unstable_by(|left, right| {
            left.finding_kind
                .cmp(&right.finding_kind)
                .then_with(|| left.subject_id.cmp(&right.subject_id))
        });
        rows.dedup_by(|right, left| {
            right.finding_kind == left.finding_kind && right.subject_id == left.subject_id
        });
        let truncated = rows.len() > usize::from(request.maximum_findings);
        let findings = rows
            .into_iter()
            .take(usize::from(request.maximum_findings))
            .map(|row| decode_finding(row, request.scope))
            .collect::<AdapterResult<Vec<_>>>()?;
        Ok(OrganizationGraphAudit {
            findings,
            generated_at: request.occurred_at,
            truncated,
        })
    }

    async fn audit_graph_inner(
        &self,
        request: OrganizationGraphAuditRequest,
    ) -> AdapterResult<OrganizationGraphAudit> {
        let audit = self
            .authorized_graph_findings(&request, OrganizationAction::AuditGraph, true)
            .await?;
        OrganizationRepositoryTelemetry::emit(
            "graph_audit",
            "allow",
            u32::try_from(audit.findings.len()).unwrap_or(u32::MAX),
        );
        Ok(audit)
    }

    async fn plan_repair_inner(
        &self,
        request: OrganizationGraphAuditRequest,
    ) -> AdapterResult<OrganizationRepairPlan> {
        let audit = self
            .authorized_graph_findings(&request, OrganizationAction::PlanRepair, false)
            .await?;
        let steps = audit
            .findings
            .iter()
            .filter_map(repair_step)
            .collect::<Vec<_>>();
        let plan = OrganizationRepairPlan::new_dry_run(
            request.scope,
            request.support_actor_id,
            request.occurred_at,
            steps,
        )
        .map_err(|_| AdapterFailure::Invalid)?;
        let findings_json =
            serde_json::to_string(&audit.findings).map_err(|_| AdapterFailure::Corrupt)?;
        let steps_json = serde_json::to_string(&plan.steps).map_err(|_| AdapterFailure::Corrupt)?;
        if findings_json.len() > 1_048_576 || steps_json.len() > 1_048_576 {
            return Err(AdapterFailure::Invalid);
        }
        let operation_id = OrganizationOperationId::new();
        let statements = vec![
            self.support_statement(&request, operation_id, "plan_insert")?,
            self.statement(
                REPAIR_PLAN_INSERT_SQL,
                &[
                    string(plan.id),
                    string(request.scope.organization_id),
                    string(request.support_actor_id),
                    JsValue::from_str(&hex_sha256(
                        request
                            .support_ticket_digest
                            .expose_for_verification()
                            .as_bytes(),
                    )),
                    JsValue::from_str(&findings_json),
                    JsValue::from_str(&steps_json),
                    timestamp_value(request.occurred_at),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[string(operation_id)])?,
            self.support_audit_statement(&request, OrganizationAction::PlanRepair, operation_id)?,
        ];
        match self.batch(statements).await {
            Err(AdapterFailure::Stale) => {
                self.record_support_denial(OrganizationAction::PlanRepair);
                return Err(AdapterFailure::AccessDenied);
            }
            result => result?,
        }
        OrganizationRepositoryTelemetry::emit(
            "repair_plan",
            "allow",
            u32::try_from(plan.steps.len()).unwrap_or(u32::MAX),
        );
        Ok(plan)
    }

    fn support_statement(
        &self,
        request: &OrganizationGraphAuditRequest,
        operation_id: OrganizationOperationId,
        suffix: &str,
    ) -> AdapterResult<D1PreparedStatement> {
        self.statement(
            SUPPORT_ASSERT_SQL,
            &[
                string(format!("{operation_id}:{suffix}")),
                string(request.support_actor_id),
                string(request.scope.organization_id),
                JsValue::from_str(request.support_ticket_digest.expose_for_verification()),
                integer(request.identity_revision)?,
                integer(request.session_version)?,
            ],
        )
    }

    fn support_audit_statement(
        &self,
        request: &OrganizationGraphAuditRequest,
        action: OrganizationAction,
        operation_id: OrganizationOperationId,
    ) -> AdapterResult<D1PreparedStatement> {
        let subject = request.scope.organization_id.to_string();
        let digest = hex_sha256(subject.as_bytes());
        self.statement(
            AUDIT_INSERT_SQL,
            &[
                string(OrganizationAuditId::new()),
                string(operation_id),
                string(request.scope.organization_id),
                string(request.support_actor_id),
                JsValue::from_str(action.stable_code()),
                JsValue::from_str("repair_plan"),
                JsValue::from_str(&digest),
                JsValue::from_str("allow"),
                JsValue::NULL,
                timestamp_value(request.occurred_at),
            ],
        )
    }

    fn record_support_denial(&self, action: OrganizationAction) {
        OrganizationRepositoryTelemetry::emit(action.stable_code(), "deny", 0);
    }

    async fn audit_decision_inner(&self, decision: OrganizationAuditDecision) -> AdapterResult<()> {
        if decision.allowed == decision.denial.is_some() {
            return Err(AdapterFailure::Invalid);
        }
        let outcome = if decision.allowed { "allow" } else { "deny" };
        OrganizationRepositoryTelemetry::emit(decision.action.stable_code(), outcome, 0);
        Ok(())
    }
}

fn decode_finding(
    row: GraphFindingRow,
    scope: OrganizationScope,
) -> AdapterResult<OrganizationGraphFinding> {
    let kind = match row.finding_kind.as_str() {
        "missing_owner_membership" => OrganizationGraphFindingKind::MissingOwnerMembership,
        "multiple_active_owners" => OrganizationGraphFindingKind::MultipleActiveOwners,
        "owner_pointer_mismatch" => OrganizationGraphFindingKind::OwnerPointerMismatch,
        "membership_without_user" => OrganizationGraphFindingKind::MembershipWithoutUser,
        "active_selection_without_membership" => {
            OrganizationGraphFindingKind::ActiveSelectionWithoutMembership
        }
        "space_membership_without_organization_membership" => {
            OrganizationGraphFindingKind::SpaceMembershipWithoutOrganizationMembership
        }
        "folder_without_space" => OrganizationGraphFindingKind::FolderWithoutSpace,
        "folder_crosses_space" => OrganizationGraphFindingKind::FolderCrossesSpace,
        "folder_cycle" => OrganizationGraphFindingKind::FolderCycle,
        "folder_depth_mismatch" => OrganizationGraphFindingKind::FolderDepthMismatch,
        "deleted_ancestor" => OrganizationGraphFindingKind::DeletedAncestor,
        _ => return Err(AdapterFailure::Corrupt),
    };
    Ok(OrganizationGraphFinding {
        kind,
        scope,
        subject_id: row.subject_id,
        observed_revision: safe_revision(row.observed_revision)?,
    })
}

fn repair_step(finding: &OrganizationGraphFinding) -> Option<OrganizationRepairStep> {
    let action = match finding.kind {
        OrganizationGraphFindingKind::MissingOwnerMembership => {
            OrganizationRepairActionKind::RestoreOwnerMembership
        }
        OrganizationGraphFindingKind::MultipleActiveOwners
        | OrganizationGraphFindingKind::OwnerPointerMismatch => {
            OrganizationRepairActionKind::AlignOwnerPointer
        }
        OrganizationGraphFindingKind::MembershipWithoutUser => {
            OrganizationRepairActionKind::SuspendOrphanMembership
        }
        OrganizationGraphFindingKind::ActiveSelectionWithoutMembership => {
            OrganizationRepairActionKind::ClearActiveSelection
        }
        OrganizationGraphFindingKind::SpaceMembershipWithoutOrganizationMembership => {
            OrganizationRepairActionKind::SuspendSpaceMembership
        }
        OrganizationGraphFindingKind::FolderWithoutSpace
        | OrganizationGraphFindingKind::FolderCrossesSpace
        | OrganizationGraphFindingKind::FolderCycle
        | OrganizationGraphFindingKind::DeletedAncestor => {
            OrganizationRepairActionKind::ReparentFolderToRoot
        }
        OrganizationGraphFindingKind::FolderDepthMismatch => {
            OrganizationRepairActionKind::RecomputeFolderDepth
        }
    };
    Some(OrganizationRepairStep {
        action,
        subject_id: finding.subject_id.clone(),
        expected_revision: finding.observed_revision,
    })
}

fn decode_snapshot(
    row: SnapshotRow,
    request: OrganizationReadRequest,
) -> AdapterResult<OrganizationSnapshot> {
    let organization_id = OrganizationId::parse(&row.id).map_err(|_| AdapterFailure::Corrupt)?;
    if organization_id != request.scope.organization_id || !matches!(row.has_pro_seat, 0 | 1) {
        return Err(AdapterFailure::Corrupt);
    }
    let scope = OrganizationScope::new(request.scope.tenant_id, organization_id)
        .map_err(|_| AdapterFailure::Corrupt)?;
    let organization = OrganizationRecord {
        scope,
        owner_id: UserId::parse(&row.owner_id).map_err(|_| AdapterFailure::Corrupt)?,
        name: frame_domain::OrganizationName::parse(row.name)
            .map_err(|_| AdapterFailure::Corrupt)?,
        status: parse_status(&row.status)?,
        settings: OrganizationSettings::parse(row.settings_json)
            .map_err(|_| AdapterFailure::Corrupt)?,
        created_at: safe_timestamp(row.created_at_ms)?,
        updated_at: safe_timestamp(row.updated_at_ms)?,
        tombstoned_at: row.tombstoned_at_ms.map(safe_timestamp).transpose()?,
        retention_until: row.retention_until_ms.map(safe_timestamp).transpose()?,
        revision: safe_revision(row.revision)?,
        authority_version: safe_revision(row.authority_version)?,
    };
    let actor_membership = OrganizationMembershipRecord {
        scope,
        user_id: request.actor_id,
        role: parse_role(&row.actor_role)?,
        state: parse_membership_state(&row.actor_state)?,
        has_pro_seat: row.has_pro_seat == 1,
        created_at: safe_timestamp(row.member_created_at_ms)?,
        updated_at: safe_timestamp(row.member_updated_at_ms)?,
        revision: safe_revision(row.member_revision)?,
        authority_version: safe_revision(row.member_authority_version)?,
    };
    let selection = ActiveOrganizationSelection {
        user_id: request.actor_id,
        default_organization_id: row
            .default_organization_id
            .map(|value| OrganizationId::parse(&value).map_err(|_| AdapterFailure::Corrupt))
            .transpose()?,
        active_organization_id: row
            .active_organization_id
            .map(|value| OrganizationId::parse(&value).map_err(|_| AdapterFailure::Corrupt))
            .transpose()?,
        revision: safe_revision(row.organization_preference_revision)?,
    };
    Ok(OrganizationSnapshot {
        organization,
        actor_membership,
        active_selection: Some(selection),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_parser_requires_exact_wranger_envelope() {
        let exact = "D1: D1Error {\n    cause: JsValue(Error: frame_organization_cas_conflict_v1: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)\n    Error: frame_organization_cas_conflict_v1: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)\n        at D1DatabaseSessionAlwaysPrimary._sendOrThrow (cloudflare-internal:d1-api:147:24)\n        at async cloudflare-internal:d1-api:97:27),\n}";
        assert!(exact_d1_trigger_error(exact, ORGANIZATION_CAS_SENTINEL));
        assert!(!exact_d1_trigger_error(
            &format!("provider {exact}"),
            ORGANIZATION_CAS_SENTINEL
        ));
        assert!(!exact_d1_trigger_error(
            &exact.replace("SQLITE_CONSTRAINT_TRIGGER", "SQLITE_CONSTRAINT_UNIQUE"),
            ORGANIZATION_CAS_SENTINEL
        ));
    }

    #[test]
    fn telemetry_and_failures_never_include_provider_or_subject_data() {
        let telemetry = OrganizationRepositoryTelemetry {
            event: "d1_organization_repository",
            operation: "folder_move",
            outcome: "deny",
            rows: 0,
        };
        let encoded = serde_json::to_string(&telemetry).expect("telemetry");
        for forbidden in ["SELECT", "token_digest", "example.test", "subject_id"] {
            assert!(!encoded.contains(forbidden));
        }
        assert_eq!(
            AdapterFailure::Unavailable.into_port(),
            OrganizationPortError::Unavailable
        );
    }

    #[test]
    fn semantic_fingerprint_is_server_derived_and_payload_bound() {
        let actor = UserId::parse("018f47a6-7b1c-7f55-8f39-8f8a8690a305").expect("actor");
        let other_actor =
            UserId::parse("018f47a6-7b1c-7f55-8f39-8f8a8690a306").expect("other actor");
        let exact_payload = serde_json::json!({
            "invite_id": "018f47a6-7b1c-7f55-8f39-8f8a8690f301",
            "presented_token_digest": format!("{:064x}", 21),
            "expected_invite_revision": 0,
        });
        let changed_payload = serde_json::json!({
            "invite_id": "018f47a6-7b1c-7f55-8f39-8f8a8690f301",
            "presented_token_digest": format!("{:064x}", 22),
            "expected_invite_revision": 0,
        });
        let reordered_payload = serde_json::json!({
            "expected_invite_revision": 0,
            "presented_token_digest": format!("{:064x}", 21),
            "invite_id": "018f47a6-7b1c-7f55-8f39-8f8a8690f301",
        });

        let exact = semantic_fingerprint("invite_accept", actor, &exact_payload)
            .expect("exact fingerprint");
        let replay = semantic_fingerprint("invite_accept", actor, &exact_payload)
            .expect("replay fingerprint");
        let reordered = semantic_fingerprint("invite_accept", actor, &reordered_payload)
            .expect("canonical fingerprint");
        let changed = semantic_fingerprint("invite_accept", actor, &changed_payload)
            .expect("changed fingerprint");
        let changed_actor = semantic_fingerprint("invite_accept", other_actor, &exact_payload)
            .expect("actor-bound fingerprint");
        let changed_kind = semantic_fingerprint("invite_revoke", actor, &exact_payload)
            .expect("operation-bound fingerprint");

        assert_eq!(
            exact.expose_for_verification(),
            replay.expose_for_verification()
        );
        assert_eq!(
            exact.expose_for_verification(),
            reordered.expose_for_verification()
        );
        assert_eq!(
            exact.expose_for_verification(),
            "8ebd0facc77e48b7cd952d998518355b8587d2d64c757f8f9cc1fe2ac89f3a9a"
        );
        for mismatch in [&changed, &changed_actor, &changed_kind] {
            assert_ne!(
                exact.expose_for_verification(),
                mismatch.expose_for_verification()
            );
        }
    }
}
