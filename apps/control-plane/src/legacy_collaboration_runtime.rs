//! Atomic D1 adapter for Cap's retained comment mutations.
//!
//! Core writes and action notification cleanup share one D1 batch. Create
//! notification handoff happens only after commit and failures are swallowed.

use async_trait::async_trait;
use frame_application::{
    LegacyCollaborationAtomicErrorV1, LegacyCollaborationAtomicOutcomeV1,
    LegacyCollaborationAtomicPortV1, LegacyCollaborationCommandV1,
    LegacyCollaborationMutationResultV1, LegacyCollaborationMutationV1,
    LegacyCollaborationNotificationKindV1, LegacyCollaborationSurfaceV1, LegacyCommentKindV1,
    LegacyCreatedCommentV1, LegacyDeleteCommentResultV1, LegacyDeleteNotificationSelectorV1,
};
use frame_domain::{LegacyCapNanoId, TimestampMillis};
use serde::Deserialize;
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_collaboration/clock_now.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_collaboration/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_collaboration/operation_claim.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_collaboration/operation_complete.sql");
const TENANT_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_collaboration/tenant_authority_snapshot.sql");
const TENANT_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/tenant_authority_assert.sql");
const USER_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/user_alias_insert.sql");
const USER_ALIAS_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/user_alias_assert.sql");
const VIDEO_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_collaboration/video_authority_snapshot.sql");
const VIDEO_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/video_authority_assert.sql");
const COMMENT_INSERT_SQL: &str = include_str!("../queries/legacy_collaboration/comment_insert.sql");
const CHANGES_ASSERT_SQL: &str = include_str!("../queries/legacy_collaboration/changes_assert.sql");
const CREATE_RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/create_receipt_insert.sql");
const DELETE_TARGETS_INSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/delete_targets_insert.sql");
const AUTHORED_TARGET_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/authored_target_assert.sql");
const DELETE_BOUND_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/delete_bound_assert.sql");
const NOTIFICATION_TARGETS_INSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/notification_targets_insert.sql");
const NOTIFICATION_BOUND_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/notification_bound_assert.sql");
const COMMENTS_DELETE_SQL: &str =
    include_str!("../queries/legacy_collaboration/comments_delete.sql");
const COMMENTS_DELETED_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/comments_deleted_assert.sql");
const NOTIFICATIONS_DELETE_SQL: &str =
    include_str!("../queries/legacy_collaboration/notifications_delete.sql");
const NOTIFICATIONS_DELETED_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/notifications_deleted_assert.sql");
const DELETE_RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/delete_receipt_insert.sql");
const EFFECT_INSERT_SQL: &str = include_str!("../queries/legacy_collaboration/effect_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_collaboration/audit_insert.sql");
const DURABLE_RECEIPT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/durable_receipt_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_collaboration/assertion_cleanup.sql");
const NOTIFICATION_ATTEMPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_collaboration/notification_attempt_insert.sql");

const AUTHORITY_SENTINEL: &str = "frame_legacy_collaboration_authority_v1";
const TARGET_SENTINEL: &str = "frame_legacy_collaboration_target_v1";
const CORRUPT_SENTINEL: &str = "frame_legacy_collaboration_corrupt_v1";
const MAX_FRESH_BATCH_STATEMENTS: usize = 30;
type AtomicResult<T> = Result<T, LegacyCollaborationAtomicErrorV1>;

pub(crate) struct D1LegacyCollaborationAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyCollaborationAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyCollaborationAtomicErrorV1::Unavailable)
    }

    async fn rows<T>(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyCollaborationAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<Vec<D1Result>> {
        if statements.len() > MAX_FRESH_BATCH_STATEMENTS {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyCollaborationAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(results)
    }
}

fn map_d1_message(message: &str) -> LegacyCollaborationAtomicErrorV1 {
    if message.contains(AUTHORITY_SENTINEL) {
        LegacyCollaborationAtomicErrorV1::StaleAuthority
    } else if message.contains(TARGET_SENTINEL) {
        LegacyCollaborationAtomicErrorV1::TargetMissing
    } else if message.contains(CORRUPT_SENTINEL)
        || message.contains("frame_legacy_collaboration_receipt_immutable_v1")
        || message.contains("frame_legacy_collaboration_comment_immutable_v1")
        || message.contains("frame_legacy_collaboration_operation_immutable_v1")
    {
        LegacyCollaborationAtomicErrorV1::Corrupt
    } else if message.contains("UNIQUE constraint failed") {
        LegacyCollaborationAtomicErrorV1::Conflict
    } else {
        LegacyCollaborationAtomicErrorV1::Unavailable
    }
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}
fn js_opt(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}
#[allow(clippy::cast_precision_loss)]
fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}
fn number_opt(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, number)
}
fn float_opt(value: Option<f64>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_f64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    MobileCreateComment,
    MobileCreateReaction,
    MobileDeleteComment,
    WebDeleteCommentRoute,
    WebDeleteCarrier,
    WebCreateCarrier,
}
impl Action {
    const fn from_surface(surface: LegacyCollaborationSurfaceV1) -> Self {
        match surface {
            LegacyCollaborationSurfaceV1::MobileCreateComment => Self::MobileCreateComment,
            LegacyCollaborationSurfaceV1::MobileCreateReaction => Self::MobileCreateReaction,
            LegacyCollaborationSurfaceV1::MobileDeleteComment => Self::MobileDeleteComment,
            LegacyCollaborationSurfaceV1::WebDeleteCommentRoute => Self::WebDeleteCommentRoute,
            LegacyCollaborationSurfaceV1::WebDeleteCommentAction => Self::WebDeleteCarrier,
            LegacyCollaborationSurfaceV1::WebNewCommentAction => Self::WebCreateCarrier,
        }
    }
    const fn journal_name(self) -> &'static str {
        match self {
            Self::MobileCreateComment => "legacy.collaboration.mobile_create_comment",
            Self::MobileCreateReaction => "legacy.collaboration.mobile_create_reaction",
            Self::MobileDeleteComment => "legacy.collaboration.mobile_delete_comment",
            Self::WebDeleteCommentRoute => "legacy.collaboration.web_delete_comment_route",
            Self::WebDeleteCarrier => "legacy.collaboration.web_delete_comment_action",
            Self::WebCreateCarrier => "legacy.collaboration.web_new_comment_action",
        }
    }
}

#[derive(Debug, Clone)]
struct Scope {
    organization_id: String,
    actor_id: String,
    action: Action,
}
impl Scope {
    fn from_command(command: &LegacyCollaborationCommandV1) -> Self {
        Self {
            organization_id: command.active_organization_id().to_string(),
            actor_id: command.actor_id().to_string(),
            action: Action::from_surface(command.surface()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}
#[derive(Debug, Clone, Deserialize)]
struct TenantAuthorityRow {
    selection_revision: i64,
    user_updated_at_ms: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    membership_role: Option<String>,
    membership_state: Option<String>,
    membership_revision: Option<i64>,
    membership_authority_version: Option<i64>,
    author_name: Option<String>,
    legacy_author_id: String,
    author_image: Option<String>,
    alias_provenance: String,
}
impl TenantAuthorityRow {
    fn validate(&self) -> AtomicResult<()> {
        if [
            self.selection_revision,
            self.user_updated_at_ms,
            self.organization_revision,
            self.organization_authority_version,
        ]
        .into_iter()
        .any(|value| value < 0)
            || LegacyCapNanoId::parse(self.legacy_author_id.clone()).is_err()
            || !matches!(
                self.alias_provenance.as_str(),
                "cap_backfill" | "membership_backfill" | "native_generated"
            )
            || self
                .author_image
                .as_ref()
                .is_some_and(|image| image.len() > 262_144)
        {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        let absent = self.membership_role.is_none()
            && self.membership_state.is_none()
            && self.membership_revision.is_none()
            && self.membership_authority_version.is_none();
        let present = self.membership_role.is_some()
            && self.membership_state.is_some()
            && self.membership_revision.is_some()
            && self.membership_authority_version.is_some();
        if !(absent || present)
            || self
                .membership_role
                .as_deref()
                .is_some_and(|role| !matches!(role, "owner" | "admin" | "member" | "viewer"))
            || self
                .membership_state
                .as_deref()
                .is_some_and(|state| !matches!(state, "active" | "suspended" | "removed"))
            || self.membership_revision.is_some_and(|value| value < 0)
            || self
                .membership_authority_version
                .is_some_and(|value| value < 0)
        {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct VideoAuthorityRow {
    mapped_video_id: String,
    owner_id: String,
    video_revision: i64,
    shared_revision: i64,
    authority_kind: String,
}
impl VideoAuthorityRow {
    fn validate(&self, scope: &Scope) -> AtomicResult<()> {
        if Uuid::parse_str(&self.mapped_video_id).is_err()
            || Uuid::parse_str(&self.owner_id).is_err()
            || self.video_revision < 0
            || self.shared_revision < -1
            || !matches!(
                self.authority_kind.as_str(),
                "owner" | "active_organization_share"
            )
            || (self.authority_kind == "owner" && self.owner_id != scope.actor_id)
            || (self.authority_kind == "active_organization_share"
                && (self.owner_id == scope.actor_id || self.shared_revision < 0))
        {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    organization_id: String,
    actor_id: String,
    action: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    legacy_comment_id: Option<String>,
    legacy_video_id: Option<String>,
    legacy_author_id: Option<String>,
    author_name: Option<String>,
    author_image: Option<String>,
    comment_kind: Option<String>,
    content: Option<String>,
    source_timestamp: Option<f64>,
    legacy_parent_comment_id: Option<String>,
    created_comment_at_ms: Option<i64>,
    updated_comment_at_ms: Option<i64>,
    notification_kind: Option<String>,
    deleted_comment_count: Option<i64>,
    deleted_notification_count: Option<i64>,
    notification_selector: Option<String>,
    revalidation_path: Option<String>,
    notification_timing: Option<String>,
    notification_failure_rolls_back_core: Option<i64>,
    effect_revalidation_path: Option<String>,
    audit_count: i64,
}
impl OperationRow {
    fn validate_identity(&self, scope: &Scope) -> AtomicResult<()> {
        if Uuid::parse_str(&self.operation_id).is_err()
            || self.organization_id != scope.organization_id
            || self.actor_id != scope.actor_id
            || self.action != scope.action.journal_name()
            || !is_lower_sha256(&self.request_digest)
            || self.audit_count < 0
        {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
    fn is_clean_claim(&self) -> bool {
        self.state == "claimed"
            && self.result_kind.is_none()
            && self.legacy_comment_id.is_none()
            && self.legacy_video_id.is_none()
            && self.legacy_author_id.is_none()
            && self.author_name.is_none()
            && self.author_image.is_none()
            && self.comment_kind.is_none()
            && self.content.is_none()
            && self.source_timestamp.is_none()
            && self.legacy_parent_comment_id.is_none()
            && self.created_comment_at_ms.is_none()
            && self.updated_comment_at_ms.is_none()
            && self.notification_kind.is_none()
            && self.deleted_comment_count.is_none()
            && self.deleted_notification_count.is_none()
            && self.notification_selector.is_none()
            && self.revalidation_path.is_none()
            && self.notification_timing.is_none()
            && self.notification_failure_rolls_back_core.is_none()
            && self.effect_revalidation_path.is_none()
            && self.audit_count == 0
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl D1LegacyCollaborationAtomicPortV1<'_> {
    async fn clock_now(&self) -> AtomicResult<i64> {
        let mut rows = self.rows::<ClockRow>(CLOCK_NOW_SQL, Vec::new()).await?;
        if rows.len() != 1 {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        let now_ms = rows.remove(0).now_ms;
        TimestampMillis::new(now_ms).map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
        Ok(now_ms)
    }
    async fn tenant_authority(&self, scope: &Scope) -> AtomicResult<TenantAuthorityRow> {
        let mut rows = self
            .rows::<TenantAuthorityRow>(
                TENANT_AUTHORITY_SNAPSHOT_SQL,
                vec![js(&scope.actor_id), js(&scope.organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyCollaborationAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        Ok(row)
    }
    async fn video_authority(
        &self,
        scope: &Scope,
        legacy_video_id: &str,
    ) -> AtomicResult<VideoAuthorityRow> {
        let mut rows = self
            .rows::<VideoAuthorityRow>(
                VIDEO_AUTHORITY_SNAPSHOT_SQL,
                vec![
                    js(legacy_video_id),
                    js(&scope.actor_id),
                    js(&scope.organization_id),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyCollaborationAtomicErrorV1::TargetMissing)?;
        row.validate(scope)?;
        Ok(row)
    }
    async fn operation(
        &self,
        scope: &Scope,
        key_digest: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let mut rows = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![
                    js(&scope.organization_id),
                    js(&scope.actor_id),
                    js(scope.action.journal_name()),
                    js(key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }
    fn operation_claim(
        &self,
        command: &LegacyCollaborationCommandV1,
        scope: &Scope,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            OPERATION_CLAIM_SQL,
            vec![
                js(&command.operation_id().to_string()),
                js(&scope.organization_id),
                js(&scope.actor_id),
                js(scope.action.journal_name()),
                js(&command.idempotency_key_digest_hex()),
                js(&command.request_digest_hex()),
                number(now_ms),
            ],
        )
    }
    fn tenant_authority_assertion(
        &self,
        operation_id: &str,
        scope: &Scope,
        authority: &TenantAuthorityRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            TENANT_AUTHORITY_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&scope.actor_id),
                js(&scope.organization_id),
                number(authority.selection_revision),
                number(authority.user_updated_at_ms),
                number(authority.organization_revision),
                number(authority.organization_authority_version),
                js_opt(authority.membership_role.as_deref()),
                js_opt(authority.membership_state.as_deref()),
                number_opt(authority.membership_revision),
                number_opt(authority.membership_authority_version),
                js_opt(authority.author_name.as_deref()),
                js(&authority.legacy_author_id),
                js_opt(authority.author_image.as_deref()),
            ],
        )
    }
    fn changes_assertion(
        &self,
        operation_id: &str,
        kind: &str,
        expected: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            CHANGES_ASSERT_SQL,
            vec![js(operation_id), js(kind), number(expected)],
        )
    }
    fn audit_insert(
        &self,
        command: &LegacyCollaborationCommandV1,
        scope: &Scope,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            AUDIT_INSERT_SQL,
            vec![
                js(&Uuid::now_v7().to_string()),
                js(&command.operation_id().to_string()),
                js(&scope.organization_id),
                js(&scope.actor_id),
                js(scope.action.journal_name()),
                js(&command.request_digest_hex()),
                number(now_ms),
            ],
        )
    }

    async fn execute_create(
        &self,
        command: &LegacyCollaborationCommandV1,
        scope: &Scope,
        authority: &TenantAuthorityRow,
        now_ms: i64,
    ) -> AtomicResult<()> {
        let LegacyCollaborationMutationV1::Create(create) = command.mutation() else {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        };
        let video_authority = if create.requires_video_authority() {
            Some(
                self.video_authority(scope, create.legacy_video_id())
                    .await?,
            )
        } else {
            None
        };
        let operation_id = command.operation_id().to_string();
        let revalidation_path =
            if command.surface() == LegacyCollaborationSurfaceV1::WebNewCommentAction {
                format!("/s/{}", create.legacy_video_id())
            } else {
                String::new()
            };
        let response_author_image =
            if command.surface() == LegacyCollaborationSurfaceV1::WebNewCommentAction {
                create.author_image()
            } else {
                authority.author_image.as_deref()
            };
        let mut statements = vec![
            self.operation_claim(command, scope, now_ms)?,
            self.tenant_authority_assertion(&operation_id, scope, authority)?,
            self.statement(
                USER_ALIAS_INSERT_SQL,
                vec![
                    js(&authority.legacy_author_id),
                    js(&scope.actor_id),
                    js_opt(authority.author_image.as_deref()),
                    js(&authority.alias_provenance),
                    number(now_ms),
                ],
            )?,
            self.statement(
                USER_ALIAS_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(&authority.legacy_author_id),
                    js(&scope.actor_id),
                ],
            )?,
        ];
        if let Some(video) = &video_authority {
            statements.push(self.statement(
                VIDEO_AUTHORITY_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(create.legacy_video_id()),
                    js(&video.mapped_video_id),
                    js(&video.owner_id),
                    js(&scope.actor_id),
                    js(&scope.organization_id),
                    number(video.video_revision),
                    number(video.shared_revision),
                    js(&video.authority_kind),
                ],
            )?);
        }
        statements.extend([
            self.statement(
                COMMENT_INSERT_SQL,
                vec![
                    js(create.legacy_comment_id()),
                    js(create.mapped_comment_id()),
                    js(create.legacy_video_id()),
                    js_opt(
                        video_authority
                            .as_ref()
                            .map(|video| video.mapped_video_id.as_str()),
                    ),
                    js(&scope.actor_id),
                    js(&authority.legacy_author_id),
                    js(create.kind().as_str()),
                    js(create.content()),
                    float_opt(create.timestamp()),
                    js_opt(create.legacy_parent_comment_id()),
                    js(create.notification_kind().as_str()),
                    number(now_ms),
                    js(scope.action.journal_name()),
                    js(&operation_id),
                ],
            )?,
            self.changes_assertion(&operation_id, "comment_inserted", 1)?,
            self.statement(
                CREATE_RECEIPT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js_opt(authority.author_name.as_deref()),
                    js_opt(response_author_image),
                    js(&revalidation_path),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "receipt_inserted", 1)?,
            self.statement(
                EFFECT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js("after_insert_best_effort"),
                    number(0),
                    js(&revalidation_path),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "effect_inserted", 1)?,
            self.audit_insert(command, scope, now_ms)?,
            self.changes_assertion(&operation_id, "audit_inserted", 1)?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![js(&operation_id), number(now_ms)],
            )?,
            self.changes_assertion(&operation_id, "operation_complete", 1)?,
            self.statement(DURABLE_RECEIPT_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(ASSERTION_CLEANUP_SQL, vec![js(&operation_id)])?,
        ]);
        self.batch(statements).await?;
        self.best_effort_notification_attempt(
            &operation_id,
            create.legacy_comment_id(),
            create.notification_kind(),
            now_ms,
        )
        .await;
        Ok(())
    }

    async fn best_effort_notification_attempt(
        &self,
        operation_id: &str,
        legacy_comment_id: &str,
        kind: LegacyCollaborationNotificationKindV1,
        now_ms: i64,
    ) {
        let Ok(statement) = self.statement(
            NOTIFICATION_ATTEMPT_INSERT_SQL,
            vec![
                js(operation_id),
                js(legacy_comment_id),
                js(kind.as_str()),
                number(now_ms),
            ],
        ) else {
            return;
        };
        let _ = statement.run().into_send().await;
    }

    async fn execute_delete(
        &self,
        command: &LegacyCollaborationCommandV1,
        scope: &Scope,
        authority: &TenantAuthorityRow,
        now_ms: i64,
    ) -> AtomicResult<()> {
        let LegacyCollaborationMutationV1::Delete {
            legacy_comment_id,
            caller_parent_id,
            caller_video_id,
        } = command.mutation()
        else {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        };
        let operation_id = command.operation_id().to_string();
        let route_mode = if command.surface() == LegacyCollaborationSurfaceV1::WebDeleteCommentRoute
        {
            "route"
        } else {
            "exact"
        };
        let selector = if command.surface() == LegacyCollaborationSurfaceV1::WebDeleteCommentAction
        {
            Some(
                if caller_parent_id
                    .as_deref()
                    .is_some_and(|parent| !parent.is_empty())
                {
                    "reply_by_comment_id"
                } else {
                    "root_comment_and_replies_by_parent_id"
                },
            )
        } else {
            None
        };
        let revalidation_path = caller_video_id
            .as_deref()
            .map_or_else(String::new, |video_id| format!("/s/{video_id}"));
        let notification_timing = if selector.is_some() {
            "same_delete_transaction"
        } else {
            "none"
        };
        let statements = vec![
            self.operation_claim(command, scope, now_ms)?,
            self.tenant_authority_assertion(&operation_id, scope, authority)?,
            self.statement(
                DELETE_TARGETS_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js(&scope.actor_id),
                    js(legacy_comment_id),
                    js(route_mode),
                ],
            )?,
            self.statement(AUTHORED_TARGET_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(DELETE_BOUND_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(
                NOTIFICATION_TARGETS_INSERT_SQL,
                vec![js(&operation_id), js_opt(selector), js(legacy_comment_id)],
            )?,
            self.statement(NOTIFICATION_BOUND_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(COMMENTS_DELETE_SQL, vec![js(&operation_id)])?,
            self.statement(COMMENTS_DELETED_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(NOTIFICATIONS_DELETE_SQL, vec![js(&operation_id)])?,
            self.statement(NOTIFICATIONS_DELETED_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(
                DELETE_RECEIPT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js(legacy_comment_id),
                    js_opt(selector),
                    js(&revalidation_path),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "receipt_inserted", 1)?,
            self.statement(
                EFFECT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js(notification_timing),
                    number(i64::from(selector.is_some())),
                    js(&revalidation_path),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "effect_inserted", 1)?,
            self.audit_insert(command, scope, now_ms)?,
            self.changes_assertion(&operation_id, "audit_inserted", 1)?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![js(&operation_id), number(now_ms)],
            )?,
            self.changes_assertion(&operation_id, "operation_complete", 1)?,
            self.statement(DURABLE_RECEIPT_ASSERT_SQL, vec![js(&operation_id)])?,
            self.statement(ASSERTION_CLEANUP_SQL, vec![js(&operation_id)])?,
        ];
        self.batch(statements).await?;
        Ok(())
    }

    async fn execute_fresh(
        &self,
        command: &LegacyCollaborationCommandV1,
        scope: &Scope,
    ) -> AtomicResult<()> {
        let authority = self.tenant_authority(scope).await?;
        let now_ms = self.clock_now().await?;
        match command.mutation() {
            LegacyCollaborationMutationV1::Create(_) => {
                self.execute_create(command, scope, &authority, now_ms)
                    .await
            }
            LegacyCollaborationMutationV1::Delete { .. } => {
                self.execute_delete(command, scope, &authority, now_ms)
                    .await
            }
        }
    }
}

fn decode_existing(
    row: OperationRow,
    command: &LegacyCollaborationCommandV1,
    scope: &Scope,
    replayed: bool,
) -> AtomicResult<LegacyCollaborationAtomicOutcomeV1> {
    row.validate_identity(scope)?;
    if row.request_digest != command.request_digest_hex() {
        return Err(LegacyCollaborationAtomicErrorV1::Conflict);
    }
    if row.state == "claimed" {
        return if row.is_clean_claim() {
            Err(LegacyCollaborationAtomicErrorV1::InFlight)
        } else {
            Err(LegacyCollaborationAtomicErrorV1::Corrupt)
        };
    }
    if row.state != "complete" || row.audit_count != 1 {
        return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
    }
    let receipt_path = row
        .revalidation_path
        .as_deref()
        .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
    if row.effect_revalidation_path.as_deref() != Some(receipt_path) {
        return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
    }
    let result = match row.result_kind.as_deref() {
        Some("created") => decode_created(&row, command)?,
        Some("deleted") => decode_deleted(&row, command)?,
        _ => return Err(LegacyCollaborationAtomicErrorV1::Corrupt),
    };
    Ok(LegacyCollaborationAtomicOutcomeV1 {
        request_digest: *command.request_digest(),
        result,
        replayed,
    })
}

fn decode_created(
    row: &OperationRow,
    command: &LegacyCollaborationCommandV1,
) -> AtomicResult<LegacyCollaborationMutationResultV1> {
    if !command.surface().creates_comment()
        || row.deleted_comment_count != Some(0)
        || row.deleted_notification_count != Some(0)
        || row.notification_selector.is_some()
        || row.notification_timing.as_deref() != Some("after_insert_best_effort")
        || row.notification_failure_rolls_back_core != Some(0)
    {
        return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
    }
    let kind = match row.comment_kind.as_deref() {
        Some("text") => LegacyCommentKindV1::Text,
        Some("emoji") => LegacyCommentKindV1::Emoji,
        _ => return Err(LegacyCollaborationAtomicErrorV1::Corrupt),
    };
    let notification_kind = match row.notification_kind.as_deref() {
        Some("comment") => LegacyCollaborationNotificationKindV1::Comment,
        Some("reply") => LegacyCollaborationNotificationKindV1::Reply,
        Some("reaction") => LegacyCollaborationNotificationKindV1::Reaction,
        _ => return Err(LegacyCollaborationAtomicErrorV1::Corrupt),
    };
    let expected_path = if command.surface() == LegacyCollaborationSurfaceV1::WebNewCommentAction {
        let LegacyCollaborationMutationV1::Create(create) = command.mutation() else {
            return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
        };
        format!("/s/{}", create.legacy_video_id())
    } else {
        String::new()
    };
    if row.revalidation_path.as_deref() != Some(expected_path.as_str()) {
        return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
    }
    let legacy_comment_id = row
        .legacy_comment_id
        .clone()
        .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
    let legacy_author_id = row
        .legacy_author_id
        .clone()
        .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
    LegacyCapNanoId::parse(legacy_comment_id.clone())
        .map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
    LegacyCapNanoId::parse(legacy_author_id.clone())
        .map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
    let created_at_ms = row
        .created_comment_at_ms
        .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
    let updated_at_ms = row
        .updated_comment_at_ms
        .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
    TimestampMillis::new(created_at_ms).map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
    TimestampMillis::new(updated_at_ms).map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
    Ok(LegacyCollaborationMutationResultV1::Created(
        LegacyCreatedCommentV1 {
            legacy_comment_id,
            legacy_video_id: row
                .legacy_video_id
                .clone()
                .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?,
            legacy_author_id,
            author_name: row.author_name.clone(),
            author_image: row.author_image.clone(),
            kind,
            content: row
                .content
                .clone()
                .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?,
            timestamp: row.source_timestamp,
            legacy_parent_comment_id: row.legacy_parent_comment_id.clone(),
            created_at_ms,
            updated_at_ms,
            notification_kind,
        },
    ))
}

fn decode_deleted(
    row: &OperationRow,
    command: &LegacyCollaborationCommandV1,
) -> AtomicResult<LegacyCollaborationMutationResultV1> {
    if command.surface().creates_comment()
        || row.legacy_video_id.is_some()
        || row.legacy_author_id.is_some()
        || row.comment_kind.is_some()
        || row.content.is_some()
        || row.created_comment_at_ms.is_some()
        || row.updated_comment_at_ms.is_some()
        || row.notification_kind.is_some()
    {
        return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
    }
    let deleted_comment_count = u32::try_from(
        row.deleted_comment_count
            .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
    let deleted_notification_count = u32::try_from(
        row.deleted_notification_count
            .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyCollaborationAtomicErrorV1::Corrupt)?;
    let (selector, expected_path, timing, rollback) = match command.surface() {
        LegacyCollaborationSurfaceV1::MobileDeleteComment => {
            if deleted_comment_count != 1 || deleted_notification_count != 0 {
                return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
            }
            (None, String::new(), "none", 0)
        }
        LegacyCollaborationSurfaceV1::WebDeleteCommentRoute => {
            if !(1..=100_000).contains(&deleted_comment_count) || deleted_notification_count != 0 {
                return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
            }
            (None, String::new(), "none", 0)
        }
        LegacyCollaborationSurfaceV1::WebDeleteCommentAction => {
            if deleted_comment_count != 1 {
                return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
            }
            let LegacyCollaborationMutationV1::Delete {
                caller_parent_id,
                caller_video_id,
                ..
            } = command.mutation()
            else {
                return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
            };
            let selector = if caller_parent_id
                .as_deref()
                .is_some_and(|parent| !parent.is_empty())
            {
                LegacyDeleteNotificationSelectorV1::ReplyByCommentId
            } else {
                LegacyDeleteNotificationSelectorV1::RootCommentAndRepliesByParentId
            };
            let video_id = caller_video_id
                .as_deref()
                .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
            (
                Some(selector),
                format!("/s/{video_id}"),
                "same_delete_transaction",
                1,
            )
        }
        _ => return Err(LegacyCollaborationAtomicErrorV1::Corrupt),
    };
    let selector_code = selector.map(|value| match value {
        LegacyDeleteNotificationSelectorV1::ReplyByCommentId => "reply_by_comment_id",
        LegacyDeleteNotificationSelectorV1::RootCommentAndRepliesByParentId => {
            "root_comment_and_replies_by_parent_id"
        }
    });
    if row.notification_selector.as_deref() != selector_code
        || row.revalidation_path.as_deref() != Some(expected_path.as_str())
        || row.notification_timing.as_deref() != Some(timing)
        || row.notification_failure_rolls_back_core != Some(rollback)
    {
        return Err(LegacyCollaborationAtomicErrorV1::Corrupt);
    }
    Ok(LegacyCollaborationMutationResultV1::Deleted(
        LegacyDeleteCommentResultV1 {
            deleted_comment_count,
            deleted_notification_count,
            notification_selector: selector,
        },
    ))
}

#[async_trait]
impl LegacyCollaborationAtomicPortV1 for D1LegacyCollaborationAtomicPortV1<'_> {
    async fn execute(
        &self,
        command: &LegacyCollaborationCommandV1,
    ) -> AtomicResult<LegacyCollaborationAtomicOutcomeV1> {
        let scope = Scope::from_command(command);
        let key_digest = command.idempotency_key_digest_hex();
        if let Some(existing) = self.operation(&scope, &key_digest).await? {
            return decode_existing(existing, command, &scope, true);
        }
        match self.execute_fresh(command, &scope).await {
            Ok(()) => {}
            Err(LegacyCollaborationAtomicErrorV1::Conflict) => {
                if let Some(existing) = self.operation(&scope, &key_digest).await? {
                    return decode_existing(existing, command, &scope, true);
                }
                return Err(LegacyCollaborationAtomicErrorV1::Conflict);
            }
            Err(error) => return Err(error),
        }
        let completed = self
            .operation(&scope, &key_digest)
            .await?
            .ok_or(LegacyCollaborationAtomicErrorV1::Corrupt)?;
        decode_existing(completed, command, &scope, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surfaces_have_six_distinct_journal_actions() {
        let actions = [
            LegacyCollaborationSurfaceV1::MobileCreateComment,
            LegacyCollaborationSurfaceV1::MobileCreateReaction,
            LegacyCollaborationSurfaceV1::MobileDeleteComment,
            LegacyCollaborationSurfaceV1::WebDeleteCommentRoute,
            LegacyCollaborationSurfaceV1::WebDeleteCommentAction,
            LegacyCollaborationSurfaceV1::WebNewCommentAction,
        ]
        .map(|surface| Action::from_surface(surface).journal_name());
        assert_eq!(
            actions
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            6
        );
    }
    #[test]
    fn d1_error_mapping_is_stable() {
        assert_eq!(
            map_d1_message("frame_legacy_collaboration_authority_v1 secret"),
            LegacyCollaborationAtomicErrorV1::StaleAuthority
        );
        assert_eq!(
            map_d1_message("frame_legacy_collaboration_target_v1 secret"),
            LegacyCollaborationAtomicErrorV1::TargetMissing
        );
        assert_eq!(
            map_d1_message("UNIQUE constraint failed: hidden"),
            LegacyCollaborationAtomicErrorV1::Conflict
        );
        assert_eq!(
            map_d1_message("provider token=hidden"),
            LegacyCollaborationAtomicErrorV1::Unavailable
        );
    }
    #[test]
    fn lower_sha256_validation_rejects_uppercase_and_non_hex() {
        assert!(is_lower_sha256(&"a".repeat(64)));
        assert!(!is_lower_sha256(&"A".repeat(64)));
        assert!(!is_lower_sha256(&"z".repeat(64)));
        assert!(!is_lower_sha256(&"a".repeat(63)));
    }
}
