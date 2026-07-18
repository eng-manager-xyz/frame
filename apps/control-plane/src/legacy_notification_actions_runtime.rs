//! Atomic D1 adapter for Cap's two retained notification actions.
//!
//! The adapter keeps organization-scoped notification reads separate from the
//! actor-global preference document, consumes one browser mutation grant per
//! attempt, and journals replay facts without retaining unrelated preferences.

use async_trait::async_trait;
use frame_application::{
    LegacyNotificationAtomicErrorV1, LegacyNotificationAtomicOutcomeV1,
    LegacyNotificationAtomicPortV1, LegacyNotificationAuthorityPostconditionV1,
    LegacyNotificationBrowserFenceV1, LegacyNotificationCommandV1,
    LegacyNotificationDiscoveredContextV1, LegacyNotificationMutationPostconditionV1,
    LegacyNotificationMutationReceiptV1, LegacyNotificationMutationResultV1,
    LegacyNotificationOrganizationAuthorityV1, LegacyNotificationPreferencesUpdateV1,
    LegacyNotificationPreservedPreferencesDigestV1,
};
use frame_domain::{SessionId, SessionMutationGrantId, TimestampMillis, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_notification_actions/clock_now.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_notification_actions/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_notification_actions/operation_claim.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_notification_actions/operation_complete.sql");
const MARK_AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_authority_snapshot.sql");
const MARK_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_authority_assert.sql");
const PREFERENCES_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/preferences_snapshot.sql");
const PREFERENCES_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/preferences_authority_assert.sql");
const MARK_MATCHING_COUNT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_matching_count.sql");
const MARK_PRECONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_precondition_assert.sql");
const MARK_UPDATE_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_update.sql");
const MARK_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_postcondition_assert.sql");
const MARK_OUT_OF_SCOPE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/mark_out_of_scope_assert.sql");
const PREFERENCES_UPDATE_SQL: &str =
    include_str!("../queries/legacy_notification_actions/preferences_update.sql");
const PREFERENCES_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/preferences_postcondition_assert.sql");
const PREFERENCES_OTHER_ACTOR_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/preferences_other_actor_assert.sql");
const BROWSER_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/browser_grant_assert.sql");
const BROWSER_GRANT_DELETE_RETURNING_SQL: &str =
    include_str!("../queries/legacy_notification_actions/browser_grant_delete_returning.sql");
const CHANGES_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/changes_assert.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/receipt_insert.sql");
const EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/effect_insert.sql");
const AUDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/audit_insert.sql");
const PROOF_INSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/proof_insert.sql");
const DURABLE_RECEIPT_ASSERT_SQL: &str =
    include_str!("../queries/legacy_notification_actions/durable_receipt_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_notification_actions/assertion_cleanup.sql");

const MARK_ACTION: &str = "legacy.notification.mark_as_read";
const PREFERENCES_ACTION: &str = "legacy.notification.update_preferences";
const MARK_EFFECT_JSON: &str = concat!(
    "{\"invalidatesNotificationList\":true,",
    "\"invalidatesNotificationPreferences\":false,",
    "\"revalidationPath\":\"/dashboard\",",
    "\"schema\":\"frame.legacy-notification-effect.v1\"}"
);
const PREFERENCES_EFFECT_JSON: &str = concat!(
    "{\"invalidatesNotificationList\":false,",
    "\"invalidatesNotificationPreferences\":true,",
    "\"revalidationPath\":\"/dashboard\",",
    "\"schema\":\"frame.legacy-notification-effect.v1\"}"
);
const AUTHORITY_SENTINEL: &str = "frame_legacy_notification_authority_v1";
const CONFLICT_SENTINEL: &str = "frame_legacy_notification_conflict_v1";
const CORRUPT_SENTINEL: &str = "frame_legacy_notification_corrupt_v1";

type AtomicResult<T> = Result<T, LegacyNotificationAtomicErrorV1>;

pub(crate) struct D1LegacyNotificationAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyNotificationAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyNotificationAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyNotificationAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)
    }

    async fn batch_results(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> AtomicResult<Vec<D1Result>> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyNotificationAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(results)
    }
}

fn map_d1_message(message: &str) -> LegacyNotificationAtomicErrorV1 {
    if message.contains(AUTHORITY_SENTINEL) {
        LegacyNotificationAtomicErrorV1::StaleAuthority
    } else if message.contains(CONFLICT_SENTINEL) {
        LegacyNotificationAtomicErrorV1::Conflict
    } else if message.contains(CORRUPT_SENTINEL) {
        LegacyNotificationAtomicErrorV1::Corrupt
    } else {
        LegacyNotificationAtomicErrorV1::Unavailable
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct MarkAuthorityRow {
    selection_revision: i64,
    organization_revision: i64,
    organization_authority_version: i64,
    membership_role: String,
    membership_revision: i64,
    membership_authority_version: i64,
}

impl MarkAuthorityRow {
    fn validate(&self) -> AtomicResult<()> {
        if [
            self.selection_revision,
            self.organization_revision,
            self.organization_authority_version,
            self.membership_revision,
            self.membership_authority_version,
        ]
        .into_iter()
        .any(|value| value < 0)
            || !matches!(
                self.membership_role.as_str(),
                "owner" | "admin" | "member" | "viewer"
            )
        {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn authority_class(&self) -> LegacyNotificationOrganizationAuthorityV1 {
        if self.membership_role == "owner" {
            LegacyNotificationOrganizationAuthorityV1::Owner
        } else {
            LegacyNotificationOrganizationAuthorityV1::Member
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PreferencesRow {
    preferences_json: Option<String>,
    notification_preferences_revision: i64,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    matching_count: i64,
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    selected_notification_id: Option<String>,
    matched_count: Option<i64>,
    read_at_ms: Option<i64>,
    notifications_json: Option<String>,
    preserved_before_sha256: Option<String>,
    preserved_after_sha256: Option<String>,
    matching_before: Option<i64>,
    updated_rows: Option<i64>,
    matching_after: Option<i64>,
    out_of_scope_updated_rows: Option<i64>,
    other_actor_rows_updated: Option<i64>,
    effect_json: Option<String>,
    audit_count: i64,
    proof_count: i64,
}

#[derive(Debug, Deserialize)]
struct ConsumedProofRow {
    mutation_grant_id: String,
    session_id: String,
    actor_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    MarkAsRead,
    UpdatePreferences,
}

impl Action {
    const fn journal_name(self) -> &'static str {
        match self {
            Self::MarkAsRead => MARK_ACTION,
            Self::UpdatePreferences => PREFERENCES_ACTION,
        }
    }

    const fn effect_json(self) -> &'static str {
        match self {
            Self::MarkAsRead => MARK_EFFECT_JSON,
            Self::UpdatePreferences => PREFERENCES_EFFECT_JSON,
        }
    }
}

#[derive(Debug, Clone)]
struct Scope {
    action: Action,
    tenant_kind: &'static str,
    tenant_id: String,
    organization_id: Option<String>,
    actor_id: String,
}

impl Scope {
    fn from_command(command: &LegacyNotificationCommandV1) -> AtomicResult<Self> {
        let actor_id = command.fence().authority().actor_id().to_string();
        match command {
            LegacyNotificationCommandV1::MarkAsRead { .. } => {
                let organization_id = command
                    .fence()
                    .authority()
                    .active_organization_id()
                    .ok_or(LegacyNotificationAtomicErrorV1::CrossTenant)?
                    .to_string();
                Ok(Self {
                    action: Action::MarkAsRead,
                    tenant_kind: "organization",
                    tenant_id: organization_id.clone(),
                    organization_id: Some(organization_id),
                    actor_id,
                })
            }
            LegacyNotificationCommandV1::UpdatePreferences { .. } => Ok(Self {
                action: Action::UpdatePreferences,
                tenant_kind: "actor",
                tenant_id: actor_id.clone(),
                organization_id: None,
                actor_id,
            }),
        }
    }

    fn rejected_for_actor(&self, actor_id: String) -> Self {
        if self.action == Action::UpdatePreferences {
            Self {
                tenant_id: actor_id.clone(),
                actor_id,
                ..self.clone()
            }
        } else {
            Self {
                actor_id,
                ..self.clone()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreferencesWire {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pause_anon_views: Option<bool>,
}

impl From<LegacyNotificationPreferencesUpdateV1> for PreferencesWire {
    fn from(value: LegacyNotificationPreferencesUpdateV1) -> Self {
        Self {
            pause_comments: value.pause_comments(),
            pause_replies: value.pause_replies(),
            pause_views: value.pause_views(),
            pause_reactions: value.pause_reactions(),
            pause_anon_views: value.pause_anon_views(),
        }
    }
}

impl From<PreferencesWire> for LegacyNotificationPreferencesUpdateV1 {
    fn from(value: PreferencesWire) -> Self {
        Self::new(
            value.pause_comments,
            value.pause_replies,
            value.pause_views,
            value.pause_reactions,
            value.pause_anon_views,
        )
    }
}

#[derive(Debug, Clone)]
struct ReceiptFacts {
    result_kind: &'static str,
    selected_notification_id: Option<String>,
    matched_count: Option<u32>,
    read_at_ms: Option<i64>,
    notifications: Option<LegacyNotificationPreferencesUpdateV1>,
    notifications_json: Option<String>,
    preserved_before: Option<[u8; 32]>,
    preserved_after: Option<[u8; 32]>,
    matching_before: u32,
    updated_rows: u32,
    matching_after: u32,
    out_of_scope_updated_rows: u32,
    other_actor_rows_updated: u32,
}

#[derive(Debug)]
struct PreferencePlan {
    merged_json: String,
    notifications_json: String,
    preserved_before: [u8; 32],
    preserved_after: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
struct ConsumedProof {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl D1LegacyNotificationAtomicPortV1<'_> {
    async fn clock_now(&self) -> AtomicResult<i64> {
        let mut rows = self.rows::<ClockRow>(CLOCK_NOW_SQL, Vec::new()).await?;
        if rows.len() != 1 {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        let now_ms = rows.remove(0).now_ms;
        TimestampMillis::new(now_ms).map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
        Ok(now_ms)
    }

    async fn mark_authority(
        &self,
        actor_id: &str,
        organization_id: &str,
    ) -> AtomicResult<MarkAuthorityRow> {
        let mut rows = self
            .rows::<MarkAuthorityRow>(
                MARK_AUTHORITY_SNAPSHOT_SQL,
                vec![js(actor_id), js(organization_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyNotificationAtomicErrorV1::StaleAuthority)?;
        row.validate()?;
        Ok(row)
    }

    async fn preferences_snapshot(&self, actor_id: &str) -> AtomicResult<PreferencesRow> {
        let mut rows = self
            .rows::<PreferencesRow>(PREFERENCES_SNAPSHOT_SQL, vec![js(actor_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyNotificationAtomicErrorV1::StaleAuthority)?;
        if row.notification_preferences_revision < 0 {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok(row)
    }

    async fn matching_count(
        &self,
        organization_id: &str,
        actor_id: &str,
        notification_id: Option<&str>,
    ) -> AtomicResult<u32> {
        let mut rows = self
            .rows::<CountRow>(
                MARK_MATCHING_COUNT_SQL,
                vec![js(organization_id), js(actor_id), js_opt(notification_id)],
            )
            .await?;
        if rows.len() != 1 {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        u32::try_from(rows.remove(0).matching_count)
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)
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
                    js(scope.tenant_kind),
                    js(&scope.tenant_id),
                    js(&scope.actor_id),
                    js(scope.action.journal_name()),
                    js(key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    fn mark_authority_assertion(
        &self,
        operation_id: &str,
        scope: &Scope,
        authority: &MarkAuthorityRow,
    ) -> AtomicResult<D1PreparedStatement> {
        let organization_id = scope
            .organization_id
            .as_deref()
            .ok_or(LegacyNotificationAtomicErrorV1::CrossTenant)?;
        self.statement(
            MARK_AUTHORITY_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&scope.actor_id),
                js(organization_id),
                number(authority.selection_revision),
                number(authority.organization_revision),
                number(authority.organization_authority_version),
                js(&authority.membership_role),
                number(authority.membership_revision),
                number(authority.membership_authority_version),
            ],
        )
    }

    fn preferences_authority_assertion(
        &self,
        operation_id: &str,
        scope: &Scope,
        snapshot: &PreferencesRow,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            PREFERENCES_AUTHORITY_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&scope.actor_id),
                number(snapshot.notification_preferences_revision),
                js_opt(snapshot.preferences_json.as_deref()),
            ],
        )
    }

    fn browser_grant_assertion(
        &self,
        assertion_id: &str,
        fence: LegacyNotificationBrowserFenceV1,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_ASSERT_SQL,
            vec![
                js(assertion_id),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
                number(now_ms),
            ],
        )
    }

    fn browser_grant_delete(
        &self,
        fence: LegacyNotificationBrowserFenceV1,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_DELETE_RETURNING_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
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

    fn proof_insert(
        &self,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            PROOF_INSERT_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
                js_opt(related_operation_id),
                js(scope.tenant_kind),
                js(&scope.tenant_id),
                js_opt(scope.organization_id.as_deref()),
                js(scope.action.journal_name()),
                js(request_digest),
                js(outcome),
                number(now_ms),
            ],
        )
    }

    fn cleanup(&self, operation_id: &str) -> AtomicResult<D1PreparedStatement> {
        self.statement(ASSERTION_CLEANUP_SQL, vec![js(operation_id)])
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn preferences(anon: Option<bool>) -> LegacyNotificationPreferencesUpdateV1 {
        LegacyNotificationPreferencesUpdateV1::new(true, false, true, false, anon)
    }

    #[test]
    fn preference_merge_preserves_every_sibling_and_optional_absence() {
        let source = r#"{"z":{"nested":[3,{"b":2,"a":1}]},"notifications":{"old":true},"trackedEvents":{"user_signed_up":true}}"#;
        let plan = preference_plan(Some(source), preferences(None)).expect("preference plan");
        let merged: Value = serde_json::from_str(&plan.merged_json).expect("merged JSON");
        assert_eq!(merged["z"]["nested"][1]["a"], 1);
        assert_eq!(merged["trackedEvents"]["user_signed_up"], true);
        assert_eq!(merged["notifications"]["pauseComments"], true);
        assert!(merged["notifications"].get("pauseAnonViews").is_none());
        assert_eq!(plan.preserved_before, plan.preserved_after);

        let explicit = preference_plan(Some(source), preferences(Some(false))).expect("explicit");
        let explicit: Value = serde_json::from_str(&explicit.notifications_json).expect("branch");
        assert_eq!(explicit["pauseAnonViews"], false);

        let json_null = preference_plan(Some("null"), preferences(None)).expect("JSON null");
        let json_null: Value = serde_json::from_str(&json_null.merged_json).expect("null merge");
        assert!(json_null.is_object());
    }

    #[test]
    fn canonical_sibling_digest_sorts_recursively_but_preserves_array_order() {
        let left: Value = serde_json::from_str(r#"{"b":2,"a":{"y":1,"x":[2,1]}}"#).expect("left");
        let right: Value = serde_json::from_str(r#"{"a":{"x":[2,1],"y":1},"b":2}"#).expect("right");
        let reordered_array: Value =
            serde_json::from_str(r#"{"a":{"x":[1,2],"y":1},"b":2}"#).expect("array");
        assert_eq!(canonical_digest(&left), canonical_digest(&right));
        assert_ne!(canonical_digest(&left), canonical_digest(&reordered_array));
    }

    #[test]
    fn operation_key_is_tenant_actor_action_and_secret_scoped() {
        let mark = Scope {
            action: Action::MarkAsRead,
            tenant_kind: "organization",
            tenant_id: "018f0000-0000-7000-8000-000000000001".into(),
            organization_id: Some("018f0000-0000-7000-8000-000000000001".into()),
            actor_id: "018f0000-0000-7000-8000-000000000002".into(),
        };
        let mut other_actor = mark.clone();
        other_actor.actor_id = "018f0000-0000-7000-8000-000000000003".into();
        let mut other_tenant = mark.clone();
        other_tenant.tenant_id = "018f0000-0000-7000-8000-000000000004".into();
        let first = operation_key_digest(&mark, "browser-key-0001");
        assert_eq!(first.len(), 64);
        assert!(!first.contains("browser-key-0001"));
        assert_ne!(
            first,
            operation_key_digest(&other_actor, "browser-key-0001")
        );
        assert_ne!(
            first,
            operation_key_digest(&other_tenant, "browser-key-0001")
        );
        assert_ne!(first, operation_key_digest(&mark, "browser-key-0002"));
    }

    #[test]
    fn sql_surface_is_bounded_scoped_and_proof_returning() {
        for token in [
            "operation.tenant_kind = ?1",
            "operation.tenant_id = ?2",
            "operation.actor_id = ?3",
            "LIMIT 2",
        ] {
            assert!(OPERATION_BY_KEY_SQL.contains(token));
        }
        for token in [
            "n.organization_id = ?1",
            "n.recipient_user_id = ?2",
            "(?3 IS NULL OR n.id = ?3)",
            "LIMIT 1",
        ] {
            assert!(MARK_MATCHING_COUNT_SQL.contains(token));
        }
        for token in [
            "notification_preferences_revision = ?3",
            "preferences_json IS ?4",
            "preferences_json = ?5",
        ] {
            assert!(PREFERENCES_UPDATE_SQL.contains(token));
        }
        assert!(BROWSER_GRANT_DELETE_RETURNING_SQL.contains("RETURNING id AS mutation_grant_id"));
        assert!(MARK_OUT_OF_SCOPE_ASSERT_SQL.contains("NOT ("));
        assert!(DURABLE_RECEIPT_ASSERT_SQL.contains("proof.mutation_grant_id = ?21"));
    }

    #[test]
    fn assertion_sentinels_map_to_fail_closed_typed_errors() {
        assert_eq!(
            map_d1_message(AUTHORITY_SENTINEL),
            LegacyNotificationAtomicErrorV1::StaleAuthority
        );
        assert_eq!(
            map_d1_message(CONFLICT_SENTINEL),
            LegacyNotificationAtomicErrorV1::Conflict
        );
        assert_eq!(
            map_d1_message(CORRUPT_SENTINEL),
            LegacyNotificationAtomicErrorV1::Corrupt
        );
        assert_eq!(
            map_d1_message("transport"),
            LegacyNotificationAtomicErrorV1::Unavailable
        );
    }

    #[test]
    fn receipt_facts_reject_partial_or_out_of_scope_results() {
        let mut mark = ReceiptFacts::mark(None, 4, 1_700_000_000_000);
        assert_eq!(mark.validate(), Ok(()));
        mark.updated_rows = 3;
        assert_eq!(
            mark.validate(),
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        );

        let plan = preference_plan(None, preferences(None)).expect("plan");
        let mut preference = ReceiptFacts::preferences(preferences(None), &plan);
        assert_eq!(preference.validate(), Ok(()));
        preference.other_actor_rows_updated = 1;
        assert_eq!(
            preference.validate(),
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        );
    }
}

impl OperationRow {
    fn validate_identity(&self) -> AtomicResult<()> {
        if Uuid::parse_str(&self.operation_id).is_err()
            || self.request_digest.len() != 64
            || self
                .request_digest
                .bytes()
                .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
            || self.audit_count < 0
            || self.proof_count < 0
        {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok(())
    }

    fn clean_claim(&self) -> bool {
        self.state == "claimed"
            && self.result_kind.is_none()
            && self.selected_notification_id.is_none()
            && self.matched_count.is_none()
            && self.read_at_ms.is_none()
            && self.notifications_json.is_none()
            && self.preserved_before_sha256.is_none()
            && self.preserved_after_sha256.is_none()
            && self.matching_before.is_none()
            && self.updated_rows.is_none()
            && self.matching_after.is_none()
            && self.out_of_scope_updated_rows.is_none()
            && self.other_actor_rows_updated.is_none()
            && self.effect_json.is_none()
            && self.audit_count == 0
            && self.proof_count == 0
    }
}

impl D1LegacyNotificationAtomicPortV1<'_> {
    async fn reject_and_return<T>(
        &self,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
        error: LegacyNotificationAtomicErrorV1,
    ) -> AtomicResult<T> {
        self.consume_only(fence, scope, related_operation_id, request_digest, outcome)
            .await?;
        Err(error)
    }

    async fn existing_outcome(
        &self,
        command: &LegacyNotificationCommandV1,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        operation: &OperationRow,
        request_digest: &str,
    ) -> AtomicResult<LegacyNotificationAtomicOutcomeV1> {
        if let Err(error) = operation.validate_identity() {
            let _ = self
                .consume_only(fence, scope, None, request_digest, "rejected")
                .await;
            return Err(error);
        }
        if operation.clean_claim() {
            return self
                .reject_and_return(
                    fence,
                    scope,
                    Some(&operation.operation_id),
                    request_digest,
                    "in_flight",
                    LegacyNotificationAtomicErrorV1::InFlight,
                )
                .await;
        }
        let facts = match ReceiptFacts::from_operation(command, operation) {
            Ok(facts) => facts,
            Err(error) => {
                let _ = self
                    .consume_only(
                        fence,
                        scope,
                        Some(&operation.operation_id),
                        request_digest,
                        "rejected",
                    )
                    .await;
                return Err(error);
            }
        };
        let (proof, authority) = match self
            .consume_replay(
                command,
                fence,
                scope,
                &operation.operation_id,
                request_digest,
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                let _ = self
                    .consume_only(
                        fence,
                        scope,
                        Some(&operation.operation_id),
                        request_digest,
                        "rejected",
                    )
                    .await;
                return Err(error);
            }
        };
        let receipt = build_receipt(command, &fence, proof, authority, &facts)?;
        Ok(LegacyNotificationAtomicOutcomeV1::Replay(receipt))
    }

    #[allow(clippy::too_many_arguments)]
    async fn reconcile(
        &self,
        command: &LegacyNotificationCommandV1,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        key_digest: &str,
        request_digest: &str,
        original_error: LegacyNotificationAtomicErrorV1,
    ) -> AtomicResult<LegacyNotificationAtomicOutcomeV1> {
        match self.operation(scope, key_digest).await {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                self.existing_outcome(command, fence, scope, &operation, request_digest)
                    .await
            }
            Ok(Some(operation)) => {
                if operation.validate_identity().is_err() {
                    let _ = self
                        .consume_only(fence, scope, None, request_digest, "rejected")
                        .await;
                    return Err(LegacyNotificationAtomicErrorV1::Corrupt);
                }
                self.reject_and_return(
                    fence,
                    scope,
                    Some(&operation.operation_id),
                    request_digest,
                    "conflict",
                    LegacyNotificationAtomicErrorV1::Conflict,
                )
                .await
            }
            Ok(None) => {
                self.reject_and_return(
                    fence,
                    scope,
                    None,
                    request_digest,
                    "rejected",
                    original_error,
                )
                .await
            }
            Err(_) => {
                let _ = self
                    .consume_only(fence, scope, None, request_digest, "rejected")
                    .await;
                Err(LegacyNotificationAtomicErrorV1::Unavailable)
            }
        }
    }
}

#[async_trait]
impl LegacyNotificationAtomicPortV1 for D1LegacyNotificationAtomicPortV1<'_> {
    async fn execute_atomic(
        &self,
        command: &LegacyNotificationCommandV1,
        browser_fence: &LegacyNotificationBrowserFenceV1,
    ) -> AtomicResult<LegacyNotificationAtomicOutcomeV1> {
        let scope = Scope::from_command(command)?;
        let fence = *browser_fence;
        let request_digest = lower_hex(command.fence().request_fingerprint());
        if fence.actor_id().to_string() != scope.actor_id {
            let rejected_scope = scope.rejected_for_actor(fence.actor_id().to_string());
            let _ = self
                .consume_only(fence, &rejected_scope, None, &request_digest, "rejected")
                .await;
            return Err(LegacyNotificationAtomicErrorV1::AccessDenied);
        }
        let key_digest = operation_key_digest(&scope, command.fence().idempotency_key().expose());

        match self.operation(&scope, &key_digest).await {
            Ok(Some(operation)) if operation.request_digest == request_digest => {
                return self
                    .existing_outcome(command, fence, &scope, &operation, &request_digest)
                    .await;
            }
            Ok(Some(operation)) => {
                if operation.validate_identity().is_err() {
                    let _ = self
                        .consume_only(fence, &scope, None, &request_digest, "rejected")
                        .await;
                    return Err(LegacyNotificationAtomicErrorV1::Corrupt);
                }
                return self
                    .reject_and_return(
                        fence,
                        &scope,
                        Some(&operation.operation_id),
                        &request_digest,
                        "conflict",
                        LegacyNotificationAtomicErrorV1::Conflict,
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = self
                    .consume_only(fence, &scope, None, &request_digest, "rejected")
                    .await;
                return Err(error);
            }
        }

        let fresh = match scope.action {
            Action::MarkAsRead => {
                self.execute_fresh_mark(command, fence, &scope, &key_digest, &request_digest)
                    .await
            }
            Action::UpdatePreferences => {
                self.execute_fresh_preferences(command, fence, &scope, &key_digest, &request_digest)
                    .await
            }
        };
        match fresh {
            Ok(receipt) => Ok(LegacyNotificationAtomicOutcomeV1::Applied(receipt)),
            Err(error) => {
                self.reconcile(command, fence, &scope, &key_digest, &request_digest, error)
                    .await
            }
        }
    }
}

fn operation_key_digest(scope: &Scope, raw_key: &str) -> String {
    digest_fields(
        b"frame.legacy-notification.operation-key.v1\0",
        &[
            scope.tenant_kind,
            &scope.tenant_id,
            &scope.actor_id,
            scope.action.journal_name(),
            raw_key,
        ],
    )
}

fn digest_fields(domain: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update(field.len().to_be_bytes());
        digest.update(field.as_bytes());
    }
    lower_hex(&digest.finalize())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
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

impl D1LegacyNotificationAtomicPortV1<'_> {
    async fn consume_only(
        &self,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        related_operation_id: Option<&str>,
        request_digest: &str,
        outcome: &str,
    ) -> AtomicResult<ConsumedProof> {
        let assertion_id = Uuid::now_v7().to_string();
        let now_ms = self.clock_now().await?;
        let mut statements = vec![self.browser_grant_assertion(&assertion_id, fence, now_ms)?];
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(&assertion_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            related_operation_id,
            request_digest,
            outcome,
            now_ms,
        )?);
        statements.push(self.changes_assertion(&assertion_id, "proof_journaled", 1)?);
        statements.push(self.cleanup(&assertion_id)?);
        let results = self.batch_results(statements).await?;
        decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
            fence,
        )
    }

    async fn consume_replay(
        &self,
        command: &LegacyNotificationCommandV1,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        operation_id: &str,
        request_digest: &str,
    ) -> AtomicResult<(
        ConsumedProof,
        Option<LegacyNotificationOrganizationAuthorityV1>,
    )> {
        let now_ms = self.clock_now().await?;
        let mut statements = Vec::new();
        let authority = match scope.action {
            Action::MarkAsRead => {
                let organization_id = scope
                    .organization_id
                    .as_deref()
                    .ok_or(LegacyNotificationAtomicErrorV1::CrossTenant)?;
                let authority = self
                    .mark_authority(&scope.actor_id, organization_id)
                    .await?;
                statements.push(self.mark_authority_assertion(operation_id, scope, &authority)?);
                Some(authority.authority_class())
            }
            Action::UpdatePreferences => {
                let snapshot = self.preferences_snapshot(&scope.actor_id).await?;
                statements.push(self.preferences_authority_assertion(
                    operation_id,
                    scope,
                    &snapshot,
                )?);
                None
            }
        };
        statements.push(self.browser_grant_assertion(operation_id, fence, now_ms)?);
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(operation_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            Some(operation_id),
            request_digest,
            "replay",
            now_ms,
        )?);
        statements.push(self.changes_assertion(operation_id, "proof_journaled", 1)?);
        statements.push(self.cleanup(operation_id)?);
        let results = self.batch_results(statements).await?;
        let proof = decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
            fence,
        )?;
        if proof.actor_id != command.fence().authority().actor_id() {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok((proof, authority))
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_fresh_mark(
        &self,
        command: &LegacyNotificationCommandV1,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        key_digest: &str,
        request_digest: &str,
    ) -> AtomicResult<LegacyNotificationMutationReceiptV1> {
        let organization_id = scope
            .organization_id
            .as_deref()
            .ok_or(LegacyNotificationAtomicErrorV1::CrossTenant)?;
        let authority = self
            .mark_authority(&scope.actor_id, organization_id)
            .await?;
        let selector = mark_selector(command)?;
        let matched_count = self
            .matching_count(organization_id, &scope.actor_id, selector.as_deref())
            .await?;
        let now_ms = self.clock_now().await?;
        let facts = ReceiptFacts::mark(selector, matched_count, now_ms);
        facts.validate()?;
        let operation_id = Uuid::now_v7().to_string();
        let audit_id = Uuid::now_v7().to_string();
        let principal_digest = digest_fields(
            b"frame.legacy-notification.principal.v1\0",
            &[&scope.actor_id],
        );
        let subject_digest = digest_fields(
            b"frame.legacy-notification.subject.v1\0",
            &[
                &scope.tenant_id,
                scope.action.journal_name(),
                request_digest,
            ],
        );
        let expected = i64::from(matched_count);

        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(scope.tenant_kind),
                    js(&scope.tenant_id),
                    js_opt(scope.organization_id.as_deref()),
                    js(&scope.actor_id),
                    js(scope.action.journal_name()),
                    js(key_digest),
                    js(request_digest),
                    number(now_ms),
                ],
            )?,
            self.mark_authority_assertion(&operation_id, scope, &authority)?,
            self.browser_grant_assertion(&operation_id, fence, now_ms)?,
            self.statement(
                MARK_PRECONDITION_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(&scope.actor_id),
                    js_opt(facts.selected_notification_id.as_deref()),
                    number(expected),
                ],
            )?,
            self.statement(
                MARK_UPDATE_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(&scope.actor_id),
                    js_opt(facts.selected_notification_id.as_deref()),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "mark_updated", expected)?,
            self.statement(
                MARK_POSTCONDITION_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(&scope.actor_id),
                    js_opt(facts.selected_notification_id.as_deref()),
                    number(now_ms),
                    number(expected),
                ],
            )?,
            self.statement(
                MARK_OUT_OF_SCOPE_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(organization_id),
                    js(&scope.actor_id),
                    js_opt(facts.selected_notification_id.as_deref()),
                ],
            )?,
            self.receipt_insert(&operation_id, &facts, now_ms)?,
            self.changes_assertion(&operation_id, "receipt_inserted", 1)?,
            self.statement(
                EFFECT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js(&scope.actor_id),
                    js(organization_id),
                    js(scope.action.journal_name()),
                    js(scope.action.effect_json()),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "effect_inserted", 1)?,
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    js(&audit_id),
                    js(&operation_id),
                    js(&scope.actor_id),
                    js(organization_id),
                    js(scope.action.journal_name()),
                    js(&principal_digest),
                    js(&subject_digest),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "audit_inserted", 1)?,
        ];
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(&operation_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            Some(&operation_id),
            request_digest,
            "applied",
            now_ms,
        )?);
        statements.push(self.changes_assertion(&operation_id, "proof_journaled", 1)?);
        statements.push(self.statement(
            OPERATION_COMPLETE_SQL,
            vec![js(&operation_id), number(now_ms)],
        )?);
        statements.push(self.changes_assertion(&operation_id, "operation_complete", 1)?);
        statements.push(self.durable_receipt_assertion(
            &operation_id,
            scope,
            request_digest,
            &facts,
            scope.action.effect_json(),
            fence,
            "applied",
        )?);
        statements.push(self.cleanup(&operation_id)?);

        let results = self.batch_results(statements).await?;
        let proof = decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
            fence,
        )?;
        build_receipt(
            command,
            &fence,
            proof,
            Some(authority.authority_class()),
            &facts,
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_fresh_preferences(
        &self,
        command: &LegacyNotificationCommandV1,
        fence: LegacyNotificationBrowserFenceV1,
        scope: &Scope,
        key_digest: &str,
        request_digest: &str,
    ) -> AtomicResult<LegacyNotificationMutationReceiptV1> {
        let notifications = command
            .notifications()
            .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?;
        let snapshot = self.preferences_snapshot(&scope.actor_id).await?;
        let plan = preference_plan(snapshot.preferences_json.as_deref(), notifications)?;
        let facts = ReceiptFacts::preferences(notifications, &plan);
        facts.validate()?;
        let now_ms = self.clock_now().await?;
        let operation_id = Uuid::now_v7().to_string();
        let audit_id = Uuid::now_v7().to_string();
        let principal_digest = digest_fields(
            b"frame.legacy-notification.principal.v1\0",
            &[&scope.actor_id],
        );
        let subject_digest = digest_fields(
            b"frame.legacy-notification.subject.v1\0",
            &[
                &scope.tenant_id,
                scope.action.journal_name(),
                request_digest,
            ],
        );

        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(scope.tenant_kind),
                    js(&scope.tenant_id),
                    js_opt(None),
                    js(&scope.actor_id),
                    js(scope.action.journal_name()),
                    js(key_digest),
                    js(request_digest),
                    number(now_ms),
                ],
            )?,
            self.preferences_authority_assertion(&operation_id, scope, &snapshot)?,
            self.browser_grant_assertion(&operation_id, fence, now_ms)?,
            self.statement(
                PREFERENCES_UPDATE_SQL,
                vec![
                    js(&operation_id),
                    js(&scope.actor_id),
                    number(snapshot.notification_preferences_revision),
                    js_opt(snapshot.preferences_json.as_deref()),
                    js(&plan.merged_json),
                ],
            )?,
            self.changes_assertion(&operation_id, "preferences_updated", 1)?,
            self.statement(
                PREFERENCES_POSTCONDITION_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(&scope.actor_id),
                    number(snapshot.notification_preferences_revision),
                    js(&plan.merged_json),
                    js(&plan.notifications_json),
                ],
            )?,
            self.statement(
                PREFERENCES_OTHER_ACTOR_ASSERT_SQL,
                vec![js(&operation_id), js(&scope.actor_id)],
            )?,
            self.receipt_insert(&operation_id, &facts, now_ms)?,
            self.changes_assertion(&operation_id, "receipt_inserted", 1)?,
            self.statement(
                EFFECT_INSERT_SQL,
                vec![
                    js(&operation_id),
                    js(&scope.actor_id),
                    js_opt(None),
                    js(scope.action.journal_name()),
                    js(scope.action.effect_json()),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "effect_inserted", 1)?,
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    js(&audit_id),
                    js(&operation_id),
                    js(&scope.actor_id),
                    js_opt(None),
                    js(scope.action.journal_name()),
                    js(&principal_digest),
                    js(&subject_digest),
                    number(now_ms),
                ],
            )?,
            self.changes_assertion(&operation_id, "audit_inserted", 1)?,
        ];
        let delete_index = statements.len();
        statements.push(self.browser_grant_delete(fence)?);
        statements.push(self.changes_assertion(&operation_id, "grant_consumed", 1)?);
        statements.push(self.proof_insert(
            fence,
            scope,
            Some(&operation_id),
            request_digest,
            "applied",
            now_ms,
        )?);
        statements.push(self.changes_assertion(&operation_id, "proof_journaled", 1)?);
        statements.push(self.statement(
            OPERATION_COMPLETE_SQL,
            vec![js(&operation_id), number(now_ms)],
        )?);
        statements.push(self.changes_assertion(&operation_id, "operation_complete", 1)?);
        statements.push(self.durable_receipt_assertion(
            &operation_id,
            scope,
            request_digest,
            &facts,
            scope.action.effect_json(),
            fence,
            "applied",
        )?);
        statements.push(self.cleanup(&operation_id)?);

        let results = self.batch_results(statements).await?;
        let proof = decode_consumed_proof(
            results
                .get(delete_index)
                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
            fence,
        )?;
        build_receipt(command, &fence, proof, None, &facts)
    }
}

fn decode_consumed_proof(
    result: &D1Result,
    fence: LegacyNotificationBrowserFenceV1,
) -> AtomicResult<ConsumedProof> {
    let mut rows = result
        .results::<ConsumedProofRow>()
        .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
    if rows.len() != 1 {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    }
    let row = rows.remove(0);
    let proof = ConsumedProof {
        mutation_grant_id: SessionMutationGrantId::parse(&row.mutation_grant_id)
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?,
        session_id: SessionId::parse(&row.session_id)
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?,
        actor_id: UserId::parse(&row.actor_id)
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?,
    };
    if proof.mutation_grant_id != fence.mutation_grant_id()
        || proof.session_id != fence.session_id()
        || proof.actor_id != fence.actor_id()
    {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    }
    Ok(proof)
}

fn mark_selector(command: &LegacyNotificationCommandV1) -> AtomicResult<Option<String>> {
    match command {
        LegacyNotificationCommandV1::MarkAsRead {
            notification_id, ..
        } => Ok(notification_id.map(|value| value.to_string())),
        LegacyNotificationCommandV1::UpdatePreferences { .. } => {
            Err(LegacyNotificationAtomicErrorV1::Corrupt)
        }
    }
}

fn preference_plan(
    source: Option<&str>,
    notifications: LegacyNotificationPreferencesUpdateV1,
) -> AtomicResult<PreferencePlan> {
    let source_value = match source {
        Some(value) => serde_json::from_str::<Value>(value)
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?,
        None => Value::Object(Map::new()),
    };
    let mut preferences = match source_value {
        Value::Null => Map::new(),
        Value::Object(preferences) => preferences,
        _ => return Err(LegacyNotificationAtomicErrorV1::Corrupt),
    };

    let mut preserved = preferences.clone();
    preserved.remove("notifications");
    let preserved_before = canonical_digest(&Value::Object(preserved))?;

    let wire = PreferencesWire::from(notifications);
    let notifications_json =
        serde_json::to_string(&wire).map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
    let notification_value =
        serde_json::to_value(wire).map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
    preferences.insert("notifications".into(), notification_value);
    let merged_value = Value::Object(preferences);
    let merged_json = serde_json::to_string(&merged_value)
        .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
    if merged_json.len() > 1_048_576 {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    }

    let Value::Object(mut after) = merged_value else {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    };
    after.remove("notifications");
    let preserved_after = canonical_digest(&Value::Object(after))?;
    if preserved_before != preserved_after {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    }
    Ok(PreferencePlan {
        merged_json,
        notifications_json,
        preserved_before,
        preserved_after,
    })
}

fn canonical_digest(value: &Value) -> AtomicResult<[u8; 32]> {
    let mut canonical = String::new();
    write_canonical_json(value, &mut canonical)?;
    Ok(Sha256::digest(canonical.as_bytes()).into())
}

fn write_canonical_json(value: &Value, output: &mut String) -> AtomicResult<()> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output.push_str(
            &serde_json::to_string(value).map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?,
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key)
                        .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?,
                );
                output.push(':');
                write_canonical_json(
                    values
                        .get(key)
                        .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
                    output,
                )?;
            }
            output.push('}');
        }
    }
    Ok(())
}

impl ReceiptFacts {
    fn mark(selector: Option<String>, matched_count: u32, read_at_ms: i64) -> Self {
        Self {
            result_kind: "marked_read",
            selected_notification_id: selector,
            matched_count: Some(matched_count),
            read_at_ms: Some(read_at_ms),
            notifications: None,
            notifications_json: None,
            preserved_before: None,
            preserved_after: None,
            matching_before: matched_count,
            updated_rows: matched_count,
            matching_after: matched_count,
            out_of_scope_updated_rows: 0,
            other_actor_rows_updated: 0,
        }
    }

    fn preferences(
        notifications: LegacyNotificationPreferencesUpdateV1,
        plan: &PreferencePlan,
    ) -> Self {
        Self {
            result_kind: "preferences_updated",
            selected_notification_id: None,
            matched_count: None,
            read_at_ms: None,
            notifications: Some(notifications),
            notifications_json: Some(plan.notifications_json.clone()),
            preserved_before: Some(plan.preserved_before),
            preserved_after: Some(plan.preserved_after),
            matching_before: 1,
            updated_rows: 1,
            matching_after: 1,
            out_of_scope_updated_rows: 0,
            other_actor_rows_updated: 0,
        }
    }

    fn from_operation(
        command: &LegacyNotificationCommandV1,
        row: &OperationRow,
    ) -> AtomicResult<Self> {
        if row.state != "complete"
            || row.audit_count != 1
            || row.proof_count < 1
            || row.effect_json.as_deref()
                != Some(Scope::from_command(command)?.action.effect_json())
        {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        let matching_before = required_u32(row.matching_before)?;
        let updated_rows = required_u32(row.updated_rows)?;
        let matching_after = required_u32(row.matching_after)?;
        let out_of_scope_updated_rows = required_u32(row.out_of_scope_updated_rows)?;
        let other_actor_rows_updated = required_u32(row.other_actor_rows_updated)?;

        match command {
            LegacyNotificationCommandV1::MarkAsRead {
                notification_id, ..
            } => {
                let expected_selector = notification_id.map(|value| value.to_string());
                if row.result_kind.as_deref() != Some("marked_read")
                    || row.selected_notification_id != expected_selector
                    || row.notifications_json.is_some()
                    || row.preserved_before_sha256.is_some()
                    || row.preserved_after_sha256.is_some()
                {
                    return Err(LegacyNotificationAtomicErrorV1::Corrupt);
                }
                let matched_count = required_u32(row.matched_count)?;
                let read_at_ms = row
                    .read_at_ms
                    .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?;
                TimestampMillis::new(read_at_ms)
                    .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
                let facts = Self {
                    result_kind: "marked_read",
                    selected_notification_id: expected_selector,
                    matched_count: Some(matched_count),
                    read_at_ms: Some(read_at_ms),
                    notifications: None,
                    notifications_json: None,
                    preserved_before: None,
                    preserved_after: None,
                    matching_before,
                    updated_rows,
                    matching_after,
                    out_of_scope_updated_rows,
                    other_actor_rows_updated,
                };
                facts.validate()?;
                Ok(facts)
            }
            LegacyNotificationCommandV1::UpdatePreferences { notifications, .. } => {
                if row.result_kind.as_deref() != Some("preferences_updated")
                    || row.selected_notification_id.is_some()
                    || row.matched_count.is_some()
                    || row.read_at_ms.is_some()
                {
                    return Err(LegacyNotificationAtomicErrorV1::Corrupt);
                }
                let source = row
                    .notifications_json
                    .as_deref()
                    .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?;
                let wire: PreferencesWire = serde_json::from_str(source)
                    .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
                if serde_json::to_string(&wire)
                    .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?
                    != source
                    || LegacyNotificationPreferencesUpdateV1::from(wire) != *notifications
                {
                    return Err(LegacyNotificationAtomicErrorV1::Corrupt);
                }
                let facts = Self {
                    result_kind: "preferences_updated",
                    selected_notification_id: None,
                    matched_count: None,
                    read_at_ms: None,
                    notifications: Some(*notifications),
                    notifications_json: Some(source.to_owned()),
                    preserved_before: Some(decode_sha256(
                        row.preserved_before_sha256
                            .as_deref()
                            .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
                    )?),
                    preserved_after: Some(decode_sha256(
                        row.preserved_after_sha256
                            .as_deref()
                            .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
                    )?),
                    matching_before,
                    updated_rows,
                    matching_after,
                    out_of_scope_updated_rows,
                    other_actor_rows_updated,
                };
                facts.validate()?;
                Ok(facts)
            }
        }
    }

    fn validate(&self) -> AtomicResult<()> {
        let valid = match self.result_kind {
            "marked_read" => {
                let Some(matched_count) = self.matched_count else {
                    return Err(LegacyNotificationAtomicErrorV1::Corrupt);
                };
                self.read_at_ms.is_some()
                    && self.notifications.is_none()
                    && self.notifications_json.is_none()
                    && self.preserved_before.is_none()
                    && self.preserved_after.is_none()
                    && self.matching_before == matched_count
                    && self.updated_rows == matched_count
                    && self.matching_after == matched_count
                    && self.out_of_scope_updated_rows == 0
                    && self.other_actor_rows_updated == 0
            }
            "preferences_updated" => {
                self.selected_notification_id.is_none()
                    && self.matched_count.is_none()
                    && self.read_at_ms.is_none()
                    && self.notifications.is_some()
                    && self.notifications_json.is_some()
                    && self.preserved_before.is_some()
                    && self.preserved_before == self.preserved_after
                    && (self.matching_before, self.updated_rows, self.matching_after) == (1, 1, 1)
                    && self.out_of_scope_updated_rows == 0
                    && self.other_actor_rows_updated == 0
            }
            _ => false,
        };
        if !valid {
            return Err(LegacyNotificationAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

fn required_u32(value: Option<i64>) -> AtomicResult<u32> {
    u32::try_from(value.ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?)
        .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)
}

fn decode_sha256(value: &str) -> AtomicResult<[u8; 32]> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    }
    let mut decoded = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let source =
            std::str::from_utf8(chunk).map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
        decoded[index] =
            u8::from_str_radix(source, 16).map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
    }
    Ok(decoded)
}

fn build_receipt(
    command: &LegacyNotificationCommandV1,
    browser_fence: &LegacyNotificationBrowserFenceV1,
    consumed: ConsumedProof,
    authority: Option<LegacyNotificationOrganizationAuthorityV1>,
    facts: &ReceiptFacts,
) -> AtomicResult<LegacyNotificationMutationReceiptV1> {
    facts.validate()?;
    if consumed.mutation_grant_id != browser_fence.mutation_grant_id()
        || consumed.session_id != browser_fence.session_id()
        || consumed.actor_id != browser_fence.actor_id()
    {
        return Err(LegacyNotificationAtomicErrorV1::Corrupt);
    }
    match command {
        LegacyNotificationCommandV1::MarkAsRead {
            notification_id, ..
        } => {
            let actor_id = command.fence().authority().actor_id();
            let organization_id = command
                .fence()
                .authority()
                .active_organization_id()
                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?;
            if facts.selected_notification_id
                != notification_id.map(|notification_id| notification_id.to_string())
            {
                return Err(LegacyNotificationAtomicErrorV1::Corrupt);
            }
            let matched_count = facts
                .matched_count
                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?;
            let read_at = TimestampMillis::new(
                facts
                    .read_at_ms
                    .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
            )
            .map_err(|_| LegacyNotificationAtomicErrorV1::Corrupt)?;
            LegacyNotificationMutationReceiptV1::new(
                command,
                browser_fence,
                LegacyNotificationMutationResultV1::MarkedRead {
                    matched_count,
                    read_at,
                },
                LegacyNotificationDiscoveredContextV1::MarkAsRead {
                    organization_id,
                    recipient_id: actor_id,
                    selected_notification_id: *notification_id,
                },
                LegacyNotificationAuthorityPostconditionV1::from_verified_mark_rows(
                    consumed.mutation_grant_id,
                    consumed.session_id,
                    consumed.actor_id,
                    organization_id,
                    authority.ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
                    actor_id,
                ),
                LegacyNotificationMutationPostconditionV1::MarkedRead {
                    matching_before: facts.matching_before,
                    updated_rows: facts.updated_rows,
                    matching_at_read_time_after: facts.matching_after,
                    out_of_scope_updated_rows: facts.out_of_scope_updated_rows,
                    read_at,
                },
            )
        }
        LegacyNotificationCommandV1::UpdatePreferences { notifications, .. } => {
            let actor_id = command.fence().authority().actor_id();
            if facts.notifications != Some(*notifications) || authority.is_some() {
                return Err(LegacyNotificationAtomicErrorV1::Corrupt);
            }
            LegacyNotificationMutationReceiptV1::new(
                command,
                browser_fence,
                LegacyNotificationMutationResultV1::PreferencesUpdated {
                    notifications: *notifications,
                },
                LegacyNotificationDiscoveredContextV1::UpdatePreferences { actor_id },
                LegacyNotificationAuthorityPostconditionV1::from_verified_preferences_row(
                    consumed.mutation_grant_id,
                    consumed.session_id,
                    consumed.actor_id,
                ),
                LegacyNotificationMutationPostconditionV1::PreferencesMerged {
                    matching_before: facts.matching_before,
                    updated_rows: facts.updated_rows,
                    matching_after: facts.matching_after,
                    other_actor_rows_updated: facts.other_actor_rows_updated,
                    stored_notifications: *notifications,
                    preserved_before:
                        LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256(
                            facts
                                .preserved_before
                                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
                        ),
                    preserved_after:
                        LegacyNotificationPreservedPreferencesDigestV1::from_canonical_sha256(
                            facts
                                .preserved_after
                                .ok_or(LegacyNotificationAtomicErrorV1::Corrupt)?,
                        ),
                },
            )
        }
    }
}

impl D1LegacyNotificationAtomicPortV1<'_> {
    fn receipt_insert(
        &self,
        operation_id: &str,
        facts: &ReceiptFacts,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            RECEIPT_INSERT_SQL,
            vec![
                js(operation_id),
                js(facts.result_kind),
                js_opt(facts.selected_notification_id.as_deref()),
                number_opt(facts.matched_count.map(i64::from)),
                number_opt(facts.read_at_ms),
                js_opt(facts.notifications_json.as_deref()),
                js_opt(
                    facts
                        .preserved_before
                        .as_ref()
                        .map(|value| lower_hex(value))
                        .as_deref(),
                ),
                js_opt(
                    facts
                        .preserved_after
                        .as_ref()
                        .map(|value| lower_hex(value))
                        .as_deref(),
                ),
                number(i64::from(facts.matching_before)),
                number(i64::from(facts.updated_rows)),
                number(i64::from(facts.matching_after)),
                number(i64::from(facts.out_of_scope_updated_rows)),
                number(i64::from(facts.other_actor_rows_updated)),
                number(now_ms),
            ],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn durable_receipt_assertion(
        &self,
        operation_id: &str,
        scope: &Scope,
        request_digest: &str,
        facts: &ReceiptFacts,
        effect_json: &str,
        fence: LegacyNotificationBrowserFenceV1,
        proof_outcome: &str,
    ) -> AtomicResult<D1PreparedStatement> {
        let before = facts
            .preserved_before
            .as_ref()
            .map(|value| lower_hex(value));
        let after = facts.preserved_after.as_ref().map(|value| lower_hex(value));
        self.statement(
            DURABLE_RECEIPT_ASSERT_SQL,
            vec![
                js(operation_id),
                js(scope.tenant_kind),
                js(&scope.tenant_id),
                js_opt(scope.organization_id.as_deref()),
                js(&scope.actor_id),
                js(scope.action.journal_name()),
                js(request_digest),
                js(facts.result_kind),
                js_opt(facts.selected_notification_id.as_deref()),
                number_opt(facts.matched_count.map(i64::from)),
                number_opt(facts.read_at_ms),
                js_opt(facts.notifications_json.as_deref()),
                js_opt(before.as_deref()),
                js_opt(after.as_deref()),
                number(i64::from(facts.matching_before)),
                number(i64::from(facts.updated_rows)),
                number(i64::from(facts.matching_after)),
                number(i64::from(facts.out_of_scope_updated_rows)),
                number(i64::from(facts.other_actor_rows_updated)),
                js(effect_json),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(proof_outcome),
            ],
        )
    }
}
