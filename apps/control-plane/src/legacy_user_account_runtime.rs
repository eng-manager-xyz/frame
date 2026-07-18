//! Atomic D1 authority for the provider-free branches of Cap's eight
//! user/account identities. Web ingress and R2 execution are separate layers.

use async_trait::async_trait;
use frame_application::{
    LegacyNullableTextPatchV1, LegacyOnboardingStepResultV1, LegacyOrganizationIdHintV1,
    LegacyUserAccountAtomicErrorV1, LegacyUserAccountAtomicOutcomeV1,
    LegacyUserAccountAtomicPortV1, LegacyUserAccountBrowserFenceV1, LegacyUserAccountCommandV1,
    LegacyUserAccountMutationResultV1, LegacyUserAccountMutationV1,
    LegacyUserAccountProviderEffectV1, LegacyUserAccountSurfaceV1,
};
use frame_domain::LegacyCapNanoId;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_user_account/clock_now.sql");
const AUTHORITY_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_user_account/authority_snapshot.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_user_account/operation_by_key.sql");
const ORGANIZATION_EXISTS_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_exists.sql");
const ORGANIZATION_MAPPING_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_mapping.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_user_account/operation_claim.sql");
const AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/authority_assert.sql");
const ORGANIZATION_ACCESS_ASSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_access_assert.sql");
const NAME_UPDATE_SQL: &str = include_str!("../queries/legacy_user_account/name_update.sql");
const WELCOME_USER_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/welcome_user_update.sql");
const WELCOME_ORGANIZATION_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/welcome_organization_update.sql");
const ORGANIZATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_insert.sql");
const ORGANIZATION_OWNER_INSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_owner_insert.sql");
const ORGANIZATION_MAPPING_INSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_mapping_insert.sql");
const ORGANIZATION_PROJECTION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_projection_assert.sql");
const ORGANIZATION_NAME_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_name_update.sql");
const ORGANIZATION_SETUP_USER_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/organization_setup_user_update.sql");
const CUSTOM_DOMAIN_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/custom_domain_update.sql");
const INVITE_TEAM_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/invite_team_update.sql");
const SKIP_USER_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/skip_user_update.sql");
const PATCH_ACCOUNT_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/patch_account_update.sql");
const SIGN_OUT_USER_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_user_update.sql");
const SIGN_OUT_IDENTITY_UPDATE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_identity_update.sql");
const SIGN_OUT_LEGACY_SESSIONS_DELETE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_legacy_sessions_delete.sql");
const SIGN_OUT_LEGACY_API_KEYS_DELETE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_legacy_api_keys_delete.sql");
const SIGN_OUT_V2_SESSION_CREDENTIALS_REVOKE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_v2_session_credentials_revoke.sql");
const SIGN_OUT_V2_MUTATION_GRANTS_DELETE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_v2_mutation_grants_delete.sql");
const SIGN_OUT_V2_SESSIONS_REVOKE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_v2_sessions_revoke.sql");
const SIGN_OUT_V2_API_KEYS_REVOKE_SQL: &str =
    include_str!("../queries/legacy_user_account/sign_out_v2_api_keys_revoke.sql");
const PROMOTE_TO_PRO_SQL: &str = include_str!("../queries/legacy_user_account/promote_to_pro.sql");
const DEMOTE_FROM_PRO_SQL: &str =
    include_str!("../queries/legacy_user_account/demote_from_pro.sql");
const RESTART_ONBOARDING_SQL: &str =
    include_str!("../queries/legacy_user_account/restart_onboarding.sql");
const MUTATION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_user_account/mutation_assert.sql");
const RECEIPT_INSERT_SQL: &str = include_str!("../queries/legacy_user_account/receipt_insert.sql");
const EFFECT_INSERT_SQL: &str = include_str!("../queries/legacy_user_account/effect_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_user_account/audit_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_user_account/operation_complete.sql");
const DURABLE_POSTCONDITION_SQL: &str =
    include_str!("../queries/legacy_user_account/durable_postcondition.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_user_account/assertion_cleanup.sql");
const BROWSER_GRANT_ASSERT_SQL: &str =
    include_str!("../queries/auth/browser_mutation_grant_assert.sql");
const BROWSER_GRANT_DELETE_SQL: &str =
    include_str!("../queries/auth/browser_mutation_grant_delete_by_proof.sql");
const BROWSER_ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_user_account/browser_assertion_cleanup.sql");

const AUTHORITY_SENTINEL: &str = "frame_legacy_user_account_authority_v1";
const FORBIDDEN_SENTINEL: &str = "frame_legacy_user_account_forbidden_v1";
const PROJECTION_SENTINEL: &str = "frame_legacy_user_account_projection_v1";
const CORRUPT_SENTINEL: &str = "frame_legacy_user_account_corrupt_v1";
const IMMUTABLE_SENTINEL: &str = "frame_legacy_user_account_evidence_immutable_v1";
const OPERATION_IMMUTABLE_SENTINEL: &str = "frame_legacy_user_account_operation_immutable_v1";
const OPERATION_UNIQUE_SENTINEL: &str =
    "UNIQUE constraint failed: legacy_user_account_operations_v1";

type AtomicResult<T> = Result<T, LegacyUserAccountAtomicErrorV1>;

pub(crate) struct D1LegacyUserAccountAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyUserAccountAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyUserAccountAtomicErrorV1::Unavailable)
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
            .map_err(|_| LegacyUserAccountAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyUserAccountAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyUserAccountAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }

    fn browser_grant_assertion(
        &self,
        operation: &str,
        fence: LegacyUserAccountBrowserFenceV1,
        now_ms: i64,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_ASSERT_SQL,
            vec![
                js(operation),
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
                number(now_ms),
            ],
        )
    }

    fn browser_grant_delete(
        &self,
        fence: LegacyUserAccountBrowserFenceV1,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            BROWSER_GRANT_DELETE_SQL,
            vec![
                js(&fence.mutation_grant_id().to_string()),
                js(&fence.session_id().to_string()),
                js(&fence.actor_id().to_string()),
            ],
        )
    }

    fn browser_assertion_cleanup(&self, operation: &str) -> AtomicResult<D1PreparedStatement> {
        self.statement(BROWSER_ASSERTION_CLEANUP_SQL, vec![js(operation)])
    }

    async fn consume_browser_fence(
        &self,
        fence: LegacyUserAccountBrowserFenceV1,
    ) -> AtomicResult<()> {
        let operation = Uuid::now_v7().to_string();
        let now_ms = self.now_ms().await?;
        self.batch(vec![
            self.browser_grant_assertion(&operation, fence, now_ms)?,
            self.browser_grant_delete(fence)?,
            self.browser_assertion_cleanup(&operation)?,
        ])
        .await
    }
}

fn map_d1_message(message: &str) -> LegacyUserAccountAtomicErrorV1 {
    if message.contains(AUTHORITY_SENTINEL) {
        LegacyUserAccountAtomicErrorV1::StaleAuthority
    } else if message.contains(FORBIDDEN_SENTINEL) {
        LegacyUserAccountAtomicErrorV1::Forbidden
    } else if message.contains(PROJECTION_SENTINEL) {
        LegacyUserAccountAtomicErrorV1::ProjectionUnavailable
    } else if message.contains(OPERATION_UNIQUE_SENTINEL) {
        LegacyUserAccountAtomicErrorV1::Conflict
    } else if message.contains(CORRUPT_SENTINEL)
        || message.contains(IMMUTABLE_SENTINEL)
        || message.contains(OPERATION_IMMUTABLE_SENTINEL)
    {
        LegacyUserAccountAtomicErrorV1::Corrupt
    } else {
        LegacyUserAccountAtomicErrorV1::Unavailable
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthorityRow {
    id: String,
    display_name: Option<String>,
    legacy_onboarding_steps_json: Option<String>,
    active_organization_id: Option<String>,
    default_organization_id: Option<String>,
    session_version: i64,
    legacy_user_account_revision: i64,
    legacy_user_account_authority_version: i64,
}

impl AuthorityRow {
    fn validate(&self, actor_id: &str) -> AtomicResult<()> {
        if self.id != actor_id
            || Uuid::parse_str(&self.id).is_err()
            || self.session_version < 0
            || self.legacy_user_account_revision < 0
            || self.legacy_user_account_authority_version < 0
            || self
                .active_organization_id
                .as_deref()
                .is_some_and(|v| Uuid::parse_str(v).is_err())
            || self
                .default_organization_id
                .as_deref()
                .is_some_and(|v| Uuid::parse_str(v).is_err())
            || self
                .legacy_onboarding_steps_json
                .as_deref()
                .is_some_and(|v| serde_json::from_str::<Value>(v).is_err())
        {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    onboarding_step: Option<String>,
    result_legacy_organization_id: Option<String>,
    provider_effect: Option<String>,
    receipt_count: i64,
    effect_count: i64,
    audit_count: i64,
}

#[derive(Debug, Deserialize)]
struct IdRow {
    id: String,
}
#[derive(Debug, Deserialize)]
struct MappingRow {
    legacy_organization_id: String,
}

struct MutationPlan {
    statements: Vec<D1PreparedStatement>,
    result: LegacyUserAccountMutationResultV1,
    result_kind: &'static str,
    onboarding_step: Option<&'static str>,
    result_legacy_organization_id: Option<String>,
    provider_effect: LegacyUserAccountProviderEffectV1,
    revision_delta: i64,
}

impl D1LegacyUserAccountAtomicPortV1<'_> {
    async fn now_ms(&self) -> AtomicResult<i64> {
        let rows: Vec<ClockRow> = self.rows(CLOCK_NOW_SQL, vec![]).await?;
        let now = rows
            .first()
            .filter(|_| rows.len() == 1)
            .map(|row| row.now_ms)
            .ok_or(LegacyUserAccountAtomicErrorV1::Corrupt)?;
        if !(0..=9_007_199_254_740_991).contains(&now) {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        Ok(now)
    }

    async fn authority(&self, actor: &str) -> AtomicResult<AuthorityRow> {
        let rows: Vec<AuthorityRow> = self.rows(AUTHORITY_SNAPSHOT_SQL, vec![js(actor)]).await?;
        let row = rows
            .first()
            .filter(|_| rows.len() == 1)
            .cloned()
            .ok_or(LegacyUserAccountAtomicErrorV1::StaleAuthority)?;
        row.validate(actor)?;
        Ok(row)
    }

    async fn operation(
        &self,
        actor: &str,
        action: &str,
        key: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let rows: Vec<OperationRow> = self
            .rows(OPERATION_BY_KEY_SQL, vec![js(actor), js(action), js(key)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        Ok(rows.into_iter().next())
    }

    async fn organization_exists(&self, id: &str) -> AtomicResult<bool> {
        let rows: Vec<IdRow> = self.rows(ORGANIZATION_EXISTS_SQL, vec![js(id)]).await?;
        if rows.len() > 1 || rows.first().is_some_and(|row| row.id != id) {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        Ok(rows.len() == 1)
    }

    async fn organization_mapping(&self, id: &str) -> AtomicResult<Option<LegacyCapNanoId>> {
        let rows: Vec<MappingRow> = self.rows(ORGANIZATION_MAPPING_SQL, vec![js(id)]).await?;
        if rows.len() > 1 {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        rows.into_iter()
            .next()
            .map(|row| {
                LegacyCapNanoId::parse(row.legacy_organization_id)
                    .map_err(|_| LegacyUserAccountAtomicErrorV1::Corrupt)
            })
            .transpose()
    }

    async fn resolve_existing_legacy_id(
        &self,
        id: &str,
        active: Option<&LegacyOrganizationIdHintV1>,
        default: Option<&LegacyOrganizationIdHintV1>,
    ) -> AtomicResult<(LegacyCapNanoId, bool)> {
        let durable = self.organization_mapping(id).await?;
        let hinted = [active, default]
            .into_iter()
            .flatten()
            .find(|hint| hint.organization_id.to_string() == id)
            .map(|hint| hint.legacy_id.clone());
        match (durable, hinted) {
            (Some(a), Some(b)) if a != b => Err(LegacyUserAccountAtomicErrorV1::Corrupt),
            (Some(value), _) => Ok((value, false)),
            (None, Some(value)) => Ok((value, true)),
            (None, None) => Err(LegacyUserAccountAtomicErrorV1::ProjectionUnavailable),
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn mutation_plan(
        &self,
        command: &LegacyUserAccountCommandV1,
        authority: &AuthorityRow,
        operation: &str,
        now: i64,
    ) -> AtomicResult<MutationPlan> {
        let actor = command.actor_id().to_string();
        let mut sql = Vec::new();
        let plan = match command.mutation() {
            LegacyUserAccountMutationV1::NameRoute {
                first_name,
                last_name,
            } => {
                let (fm, fv) = patch_binding(first_name);
                let (lm, lv) = patch_binding(last_name);
                sql.push(self.statement(
                    NAME_UPDATE_SQL,
                    vec![
                        js(&actor),
                        number(fm),
                        fv,
                        number(lm),
                        lv,
                        number(now),
                        js(operation),
                    ],
                )?);
                MutationPlan {
                    statements: sql,
                    result: LegacyUserAccountMutationResultV1::JsonTrue,
                    result_kind: "json_true",
                    onboarding_step: None,
                    result_legacy_organization_id: None,
                    provider_effect: LegacyUserAccountProviderEffectV1::NotRequested,
                    revision_delta: 1,
                }
            }
            LegacyUserAccountMutationV1::Welcome {
                first_name,
                last_name,
            } => {
                sql.push(self.statement(
                    WELCOME_USER_UPDATE_SQL,
                    vec![
                        js(&actor),
                        js(first_name),
                        js(last_name),
                        number(now),
                        js(operation),
                    ],
                )?);
                let active = authority
                    .active_organization_id
                    .as_deref()
                    .or(authority.default_organization_id.as_deref());
                if let Some(org) = active.filter(|_| !first_name.is_empty()) {
                    let name = format!("{first_name}'s Organization");
                    sql.push(self.statement(
                        WELCOME_ORGANIZATION_UPDATE_SQL,
                        vec![
                            js(org),
                            js(safe_organization_name(&name)),
                            js(&name),
                            number(now),
                            js(operation),
                        ],
                    )?);
                }
                onboarding_plan(
                    sql,
                    LegacyOnboardingStepResultV1::Welcome,
                    LegacyUserAccountProviderEffectV1::NotRequested,
                    1,
                )
            }
            LegacyUserAccountMutationV1::OrganizationSetup {
                organization_name,
                fallback_legacy_id,
                fallback_organization_id,
                active_hint,
                default_hint,
                organization_icon,
            } => {
                let candidate = authority
                    .active_organization_id
                    .as_deref()
                    .or(authority.default_organization_id.as_deref());
                let exists = match candidate {
                    Some(id) => self.organization_exists(id).await?,
                    None => false,
                };
                let (resolved, legacy, create, record) = if exists {
                    let resolved = candidate.ok_or(LegacyUserAccountAtomicErrorV1::Corrupt)?;
                    let (legacy, record) = self
                        .resolve_existing_legacy_id(
                            resolved,
                            active_hint.as_ref(),
                            default_hint.as_ref(),
                        )
                        .await?;
                    (resolved.to_owned(), legacy, false, record)
                } else {
                    (
                        fallback_organization_id.to_string(),
                        fallback_legacy_id.clone(),
                        true,
                        true,
                    )
                };
                if create {
                    sql.push(self.statement(
                        ORGANIZATION_INSERT_SQL,
                        vec![
                            js(&resolved),
                            js(&actor),
                            js(safe_organization_name(organization_name)),
                            js(organization_name),
                            number(now),
                            js(operation),
                        ],
                    )?);
                    sql.push(self.statement(
                        ORGANIZATION_OWNER_INSERT_SQL,
                        vec![js(&resolved), js(&actor), number(now), js(operation)],
                    )?);
                } else {
                    sql.push(self.statement(
                        ORGANIZATION_NAME_UPDATE_SQL,
                        vec![
                            js(&resolved),
                            js(safe_organization_name(organization_name)),
                            js(organization_name),
                            number(now),
                            js(operation),
                        ],
                    )?);
                }
                if record {
                    sql.push(self.statement(
                        ORGANIZATION_MAPPING_INSERT_SQL,
                        vec![
                            js(&resolved),
                            js(legacy.as_str()),
                            number(now),
                            js(operation),
                        ],
                    )?);
                    sql.push(self.statement(
                        ORGANIZATION_PROJECTION_ASSERT_SQL,
                        vec![js(operation), js(&resolved), js(legacy.as_str())],
                    )?);
                }
                sql.push(self.statement(
                    ORGANIZATION_SETUP_USER_UPDATE_SQL,
                    vec![js(&actor), js(&resolved), js(operation), number(now)],
                )?);
                onboarding_plan(
                    sql,
                    LegacyOnboardingStepResultV1::OrganizationSetup {
                        legacy_organization_id: legacy,
                    },
                    if organization_icon.is_some() {
                        LegacyUserAccountProviderEffectV1::BestEffortProtectedGate
                    } else {
                        LegacyUserAccountProviderEffectV1::NotRequested
                    },
                    1,
                )
            }
            LegacyUserAccountMutationV1::CustomDomain => {
                sql.push(self.statement(
                    CUSTOM_DOMAIN_UPDATE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                onboarding_plan(
                    sql,
                    LegacyOnboardingStepResultV1::CustomDomain,
                    LegacyUserAccountProviderEffectV1::NotRequested,
                    1,
                )
            }
            LegacyUserAccountMutationV1::InviteTeam => {
                sql.push(self.statement(
                    INVITE_TEAM_UPDATE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                onboarding_plan(
                    sql,
                    LegacyOnboardingStepResultV1::InviteTeam,
                    LegacyUserAccountProviderEffectV1::NotRequested,
                    1,
                )
            }
            LegacyUserAccountMutationV1::SkipToDashboard {
                fallback_legacy_id,
                fallback_organization_id,
                active_hint,
            } => {
                let session_active = active_hint
                    .as_ref()
                    .map(|hint| hint.organization_id.to_string());
                let exists = match session_active.as_deref() {
                    Some(id) => self.organization_exists(id).await?,
                    None => false,
                };
                let create = !exists;
                let placeholder =
                    !onboarding_welcome_truthy(authority.legacy_onboarding_steps_json.as_deref())?;
                let user_name = if placeholder {
                    Some("Your name".to_owned())
                } else {
                    authority.display_name.clone()
                };
                if create {
                    let org_name = if placeholder {
                        "Your Organization".to_owned()
                    } else {
                        format!(
                            "{}'s organization",
                            authority.display_name.as_deref().unwrap_or("null")
                        )
                    };
                    let resolved = fallback_organization_id.to_string();
                    sql.push(self.statement(
                        ORGANIZATION_INSERT_SQL,
                        vec![
                            js(&resolved),
                            js(&actor),
                            js(safe_organization_name(&org_name)),
                            js(&org_name),
                            number(now),
                            js(operation),
                        ],
                    )?);
                    sql.push(self.statement(
                        ORGANIZATION_OWNER_INSERT_SQL,
                        vec![js(&resolved), js(&actor), number(now), js(operation)],
                    )?);
                    sql.push(self.statement(
                        ORGANIZATION_MAPPING_INSERT_SQL,
                        vec![
                            js(&resolved),
                            js(fallback_legacy_id.as_str()),
                            number(now),
                            js(operation),
                        ],
                    )?);
                    sql.push(self.statement(
                        ORGANIZATION_PROJECTION_ASSERT_SQL,
                        vec![
                            js(operation),
                            js(&resolved),
                            js(fallback_legacy_id.as_str()),
                        ],
                    )?);
                }
                sql.push(self.statement(
                    SKIP_USER_UPDATE_SQL,
                    vec![
                        js(&actor),
                        js_opt(user_name.as_deref()),
                        number(i64::from(create)),
                        js(&fallback_organization_id.to_string()),
                        js(operation),
                        number(now),
                    ],
                )?);
                onboarding_plan(
                    sql,
                    LegacyOnboardingStepResultV1::SkipToDashboard,
                    LegacyUserAccountProviderEffectV1::NotRequested,
                    1,
                )
            }
            LegacyUserAccountMutationV1::UserImageAbsent => MutationPlan {
                statements: sql,
                result: LegacyUserAccountMutationResultV1::RpcVoid,
                result_kind: "rpc_void",
                onboarding_step: None,
                result_legacy_organization_id: None,
                provider_effect: LegacyUserAccountProviderEffectV1::NotRequested,
                revision_delta: 0,
            },
            LegacyUserAccountMutationV1::UserImageClear
            | LegacyUserAccountMutationV1::UserImageSome(_) => {
                return Err(LegacyUserAccountAtomicErrorV1::ProviderRequired);
            }
            LegacyUserAccountMutationV1::PatchAccountSettings {
                first_name,
                last_name,
                default_organization_id,
            } => {
                if let Some(org) = default_organization_id {
                    sql.push(self.statement(
                        ORGANIZATION_ACCESS_ASSERT_SQL,
                        vec![js(operation), js(&org.to_string()), js(&actor)],
                    )?);
                }
                let (fm, fv) = patch_binding(first_name);
                let (lm, lv) = patch_binding(last_name);
                let default_string = default_organization_id.map(|id| id.to_string());
                sql.push(self.statement(
                    PATCH_ACCOUNT_UPDATE_SQL,
                    vec![
                        js(&actor),
                        number(fm),
                        fv,
                        number(lm),
                        lv,
                        number(i64::from(default_organization_id.is_some())),
                        js_opt(default_string.as_deref()),
                        js(operation),
                        number(now),
                    ],
                )?);
                server_void_plan(sql, 1)
            }
            LegacyUserAccountMutationV1::SignOutAllDevices => {
                sql.push(self.statement(
                    SIGN_OUT_USER_UPDATE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                sql.push(self.statement(
                    SIGN_OUT_IDENTITY_UPDATE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                sql.push(self.statement(SIGN_OUT_LEGACY_SESSIONS_DELETE_SQL, vec![js(&actor)])?);
                sql.push(self.statement(SIGN_OUT_LEGACY_API_KEYS_DELETE_SQL, vec![js(&actor)])?);
                sql.push(self.statement(
                    SIGN_OUT_V2_SESSION_CREDENTIALS_REVOKE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                sql.push(self.statement(SIGN_OUT_V2_MUTATION_GRANTS_DELETE_SQL, vec![js(&actor)])?);
                sql.push(self.statement(
                    SIGN_OUT_V2_SESSIONS_REVOKE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                sql.push(self.statement(
                    SIGN_OUT_V2_API_KEYS_REVOKE_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                server_void_plan(sql, 1)
            }
            LegacyUserAccountMutationV1::PromoteToPro => {
                sql.push(self.statement(
                    PROMOTE_TO_PRO_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                server_void_plan(sql, 1)
            }
            LegacyUserAccountMutationV1::DemoteFromPro => {
                sql.push(self.statement(
                    DEMOTE_FROM_PRO_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                server_void_plan(sql, 1)
            }
            LegacyUserAccountMutationV1::RestartOnboarding => {
                sql.push(self.statement(
                    RESTART_ONBOARDING_SQL,
                    vec![js(&actor), number(now), js(operation)],
                )?);
                server_void_plan(sql, 1)
            }
        };
        Ok(plan)
    }

    fn replay(
        &self,
        command: &LegacyUserAccountCommandV1,
        row: &OperationRow,
    ) -> AtomicResult<LegacyUserAccountAtomicOutcomeV1> {
        if Uuid::parse_str(&row.operation_id).is_err() {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        if row.state == "pending" {
            return Err(LegacyUserAccountAtomicErrorV1::Conflict);
        }
        if row.state != "applied"
            || row.receipt_count != 1
            || row.effect_count != 1
            || row.audit_count != 1
        {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        let provider = parse_provider_effect(
            row.provider_effect
                .as_deref()
                .ok_or(LegacyUserAccountAtomicErrorV1::Corrupt)?,
        )?;
        let result = match row.result_kind.as_deref() {
            Some("json_true") if command.surface() == LegacyUserAccountSurfaceV1::NameRoute => {
                LegacyUserAccountMutationResultV1::JsonTrue
            }
            Some("rpc_void") if command.surface() == LegacyUserAccountSurfaceV1::UserUpdate => {
                LegacyUserAccountMutationResultV1::RpcVoid
            }
            Some("server_action_void")
                if matches!(
                    command.surface(),
                    LegacyUserAccountSurfaceV1::PatchAccountSettings
                        | LegacyUserAccountSurfaceV1::SignOutAllDevices
                        | LegacyUserAccountSurfaceV1::DemoteFromPro
                        | LegacyUserAccountSurfaceV1::PromoteToPro
                        | LegacyUserAccountSurfaceV1::RestartOnboarding
                ) =>
            {
                LegacyUserAccountMutationResultV1::ServerActionVoid
            }
            Some("onboarding")
                if command.surface() == LegacyUserAccountSurfaceV1::CompleteOnboardingStep =>
            {
                LegacyUserAccountMutationResultV1::Onboarding {
                    step: parse_onboarding_result(
                        row.onboarding_step
                            .as_deref()
                            .ok_or(LegacyUserAccountAtomicErrorV1::Corrupt)?,
                        row.result_legacy_organization_id.as_deref(),
                    )?,
                }
            }
            _ => return Err(LegacyUserAccountAtomicErrorV1::Corrupt),
        };
        Ok(LegacyUserAccountAtomicOutcomeV1 {
            result,
            provider_effect: provider,
            replayed: true,
        })
    }

    async fn reconcile(
        &self,
        command: &LegacyUserAccountCommandV1,
        original: LegacyUserAccountAtomicErrorV1,
    ) -> AtomicResult<LegacyUserAccountAtomicOutcomeV1> {
        let actor = command.actor_id().to_string();
        match self
            .operation(
                &actor,
                action(command.surface()),
                &command.idempotency_key_digest_hex(),
            )
            .await
        {
            Ok(Some(row)) if row.request_digest == command.request_digest_hex() => {
                self.replay(command, &row)
            }
            Ok(Some(_)) => Err(LegacyUserAccountAtomicErrorV1::IdempotencyConflict),
            Ok(None) => Err(original),
            Err(_) => Err(LegacyUserAccountAtomicErrorV1::Unavailable),
        }
    }
}

#[async_trait]
impl LegacyUserAccountAtomicPortV1 for D1LegacyUserAccountAtomicPortV1<'_> {
    async fn execute(
        &self,
        command: LegacyUserAccountCommandV1,
    ) -> AtomicResult<LegacyUserAccountAtomicOutcomeV1> {
        if matches!(
            command.mutation(),
            LegacyUserAccountMutationV1::UserImageClear
                | LegacyUserAccountMutationV1::UserImageSome(_)
        ) {
            return Err(LegacyUserAccountAtomicErrorV1::ProviderRequired);
        }
        let browser_fence = command.browser_fence();
        if is_server_action(command.surface()) {
            let fence = browser_fence.ok_or(LegacyUserAccountAtomicErrorV1::StaleAuthority)?;
            if fence.actor_id() != command.actor_id() {
                return Err(LegacyUserAccountAtomicErrorV1::StaleAuthority);
            }
        } else if browser_fence.is_some() {
            return Err(LegacyUserAccountAtomicErrorV1::Corrupt);
        }
        let actor = command.actor_id().to_string();
        let action = action(command.surface());
        let key = command.idempotency_key_digest_hex();
        let request = command.request_digest_hex();
        if let Some(row) = self.operation(&actor, action, &key).await? {
            if row.request_digest == request {
                if let Some(fence) = browser_fence {
                    self.consume_browser_fence(fence).await?;
                }
                return self.replay(&command, &row);
            }
            if let Some(fence) = browser_fence {
                self.consume_browser_fence(fence).await?;
            }
            return Err(LegacyUserAccountAtomicErrorV1::IdempotencyConflict);
        }
        let authority = self.authority(&actor).await?;
        let now = self.now_ms().await?;
        let operation = command.operation_id().to_string();
        let plan = self
            .mutation_plan(&command, &authority, &operation, now)
            .await?;
        let mut statements = Vec::new();
        if let Some(fence) = browser_fence {
            statements.push(self.browser_grant_assertion(&operation, fence, now)?);
            statements.push(self.browser_grant_delete(fence)?);
        }
        statements.extend([
            self.statement(
                AUTHORITY_ASSERT_SQL,
                vec![
                    js(&operation),
                    js(&actor),
                    number(authority.legacy_user_account_revision),
                    number(authority.legacy_user_account_authority_version),
                ],
            )?,
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation),
                    js(&actor),
                    js(action),
                    js(&key),
                    js(&request),
                    number(now),
                ],
            )?,
        ]);
        statements.extend(plan.statements);
        statements.push(self.statement(
            MUTATION_ASSERT_SQL,
            vec![
                js(&operation),
                js(&actor),
                number(plan.revision_delta),
                number(authority.legacy_user_account_revision),
            ],
        )?);
        let revision = authority
            .legacy_user_account_revision
            .checked_add(plan.revision_delta)
            .ok_or(LegacyUserAccountAtomicErrorV1::Corrupt)?;
        let provider = plan.provider_effect.stable_code();
        let effect = json!({"action": action, "providerEffect": provider}).to_string();
        let principal = digest_fields(b"frame.legacy-user-account.principal.v1\0", &[&actor]);
        let subject = digest_fields(
            b"frame.legacy-user-account.subject.v1\0",
            &[action, &request],
        );
        statements.push(self.statement(
            RECEIPT_INSERT_SQL,
            vec![
                js(&operation),
                js(&actor),
                js(action),
                js(plan.result_kind),
                js_opt(plan.onboarding_step),
                js_opt(plan.result_legacy_organization_id.as_deref()),
                js(provider),
                number(revision),
                number(now),
            ],
        )?);
        statements.push(self.statement(
            EFFECT_INSERT_SQL,
            vec![
                js(&operation),
                js(&actor),
                js(action),
                js(provider),
                js(&effect),
                number(now),
            ],
        )?);
        statements.push(self.statement(
            AUDIT_INSERT_SQL,
            vec![
                js(&Uuid::now_v7().to_string()),
                js(&operation),
                js(&actor),
                js(action),
                js(&principal),
                js(&subject),
                number(now),
            ],
        )?);
        statements.push(self.statement(
            OPERATION_COMPLETE_SQL,
            vec![
                js(&operation),
                js(plan.result_kind),
                js_opt(plan.onboarding_step),
                js_opt(plan.result_legacy_organization_id.as_deref()),
                js(provider),
                number(now),
            ],
        )?);
        statements.push(self.statement(DURABLE_POSTCONDITION_SQL, vec![js(&operation)])?);
        statements.push(self.statement(ASSERTION_CLEANUP_SQL, vec![js(&operation)])?);
        if browser_fence.is_some() {
            statements.push(self.browser_assertion_cleanup(&operation)?);
        }
        let result = plan.result.clone();
        let provider_effect = plan.provider_effect;
        match self.batch(statements).await {
            Ok(()) => Ok(LegacyUserAccountAtomicOutcomeV1 {
                result,
                provider_effect,
                replayed: false,
            }),
            Err(error) => self.reconcile(&command, error).await,
        }
    }
}

fn onboarding_plan(
    statements: Vec<D1PreparedStatement>,
    step: LegacyOnboardingStepResultV1,
    provider_effect: LegacyUserAccountProviderEffectV1,
    revision_delta: i64,
) -> MutationPlan {
    let legacy = match &step {
        LegacyOnboardingStepResultV1::OrganizationSetup {
            legacy_organization_id,
        } => Some(legacy_organization_id.as_str().to_owned()),
        _ => None,
    };
    let onboarding_step = Some(step.stable_code());
    MutationPlan {
        statements,
        result: LegacyUserAccountMutationResultV1::Onboarding { step },
        result_kind: "onboarding",
        onboarding_step,
        result_legacy_organization_id: legacy,
        provider_effect,
        revision_delta,
    }
}

fn server_void_plan(statements: Vec<D1PreparedStatement>, revision_delta: i64) -> MutationPlan {
    MutationPlan {
        statements,
        result: LegacyUserAccountMutationResultV1::ServerActionVoid,
        result_kind: "server_action_void",
        onboarding_step: None,
        result_legacy_organization_id: None,
        provider_effect: LegacyUserAccountProviderEffectV1::NotRequested,
        revision_delta,
    }
}

fn parse_provider_effect(value: &str) -> AtomicResult<LegacyUserAccountProviderEffectV1> {
    match value {
        "not_requested" => Ok(LegacyUserAccountProviderEffectV1::NotRequested),
        "applied" => Ok(LegacyUserAccountProviderEffectV1::Applied),
        "best_effort_failed" => Ok(LegacyUserAccountProviderEffectV1::BestEffortFailed),
        "best_effort_protected_gate" => {
            Ok(LegacyUserAccountProviderEffectV1::BestEffortProtectedGate)
        }
        _ => Err(LegacyUserAccountAtomicErrorV1::Corrupt),
    }
}

fn parse_onboarding_result(
    step: &str,
    org: Option<&str>,
) -> AtomicResult<LegacyOnboardingStepResultV1> {
    match (step, org) {
        ("welcome", None) => Ok(LegacyOnboardingStepResultV1::Welcome),
        ("organizationSetup", Some(value)) => Ok(LegacyOnboardingStepResultV1::OrganizationSetup {
            legacy_organization_id: LegacyCapNanoId::parse(value.to_owned())
                .map_err(|_| LegacyUserAccountAtomicErrorV1::Corrupt)?,
        }),
        ("customDomain", None) => Ok(LegacyOnboardingStepResultV1::CustomDomain),
        ("inviteTeam", None) => Ok(LegacyOnboardingStepResultV1::InviteTeam),
        ("skipToDashboard", None) => Ok(LegacyOnboardingStepResultV1::SkipToDashboard),
        _ => Err(LegacyUserAccountAtomicErrorV1::Corrupt),
    }
}

fn action(surface: LegacyUserAccountSurfaceV1) -> &'static str {
    match surface {
        LegacyUserAccountSurfaceV1::NameRoute => "legacy.user.name",
        LegacyUserAccountSurfaceV1::CompleteOnboardingStep => "legacy.user.complete_onboarding",
        LegacyUserAccountSurfaceV1::UserUpdate => "legacy.user.update",
        LegacyUserAccountSurfaceV1::PatchAccountSettings => "legacy.account.patch",
        LegacyUserAccountSurfaceV1::SignOutAllDevices => "legacy.account.sign_out_all",
        LegacyUserAccountSurfaceV1::DemoteFromPro => "legacy.devtool.demote_from_pro",
        LegacyUserAccountSurfaceV1::PromoteToPro => "legacy.devtool.promote_to_pro",
        LegacyUserAccountSurfaceV1::RestartOnboarding => "legacy.devtool.restart_onboarding",
    }
}

const fn is_server_action(surface: LegacyUserAccountSurfaceV1) -> bool {
    matches!(
        surface,
        LegacyUserAccountSurfaceV1::PatchAccountSettings
            | LegacyUserAccountSurfaceV1::SignOutAllDevices
            | LegacyUserAccountSurfaceV1::DemoteFromPro
            | LegacyUserAccountSurfaceV1::PromoteToPro
            | LegacyUserAccountSurfaceV1::RestartOnboarding
    )
}

fn safe_organization_name(value: &str) -> &str {
    if (1..=160).contains(&value.chars().count()) {
        value
    } else {
        "Legacy organization"
    }
}

fn onboarding_welcome_truthy(value: Option<&str>) -> AtomicResult<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    let value: Value =
        serde_json::from_str(value).map_err(|_| LegacyUserAccountAtomicErrorV1::Corrupt)?;
    let Some(welcome) = value.get("welcome") else {
        return Ok(false);
    };
    Ok(match welcome {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64().is_some_and(|v| v != 0.0),
        Value::String(v) => !v.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    })
}

fn patch_binding(value: &LegacyNullableTextPatchV1) -> (i64, JsValue) {
    match value {
        LegacyNullableTextPatchV1::Absent => (0, JsValue::NULL),
        LegacyNullableTextPatchV1::Null => (1, JsValue::NULL),
        LegacyNullableTextPatchV1::Value(value) => (2, js(value)),
    }
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

fn lower_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_names_cover_all_eight_inventory_rows() {
        let actions = [
            LegacyUserAccountSurfaceV1::NameRoute,
            LegacyUserAccountSurfaceV1::CompleteOnboardingStep,
            LegacyUserAccountSurfaceV1::UserUpdate,
            LegacyUserAccountSurfaceV1::PatchAccountSettings,
            LegacyUserAccountSurfaceV1::SignOutAllDevices,
            LegacyUserAccountSurfaceV1::DemoteFromPro,
            LegacyUserAccountSurfaceV1::PromoteToPro,
            LegacyUserAccountSurfaceV1::RestartOnboarding,
        ]
        .map(action);
        assert_eq!(actions.len(), 8);
        assert_eq!(actions[4], "legacy.account.sign_out_all");
    }

    #[test]
    fn retained_organization_name_is_safe_without_losing_cap_projection() {
        assert_eq!(safe_organization_name("Cap"), "Cap");
        assert_eq!(safe_organization_name(""), "Legacy organization");
        assert_eq!(
            safe_organization_name(&"x".repeat(161)),
            "Legacy organization"
        );
    }

    #[test]
    fn skip_truthiness_matches_javascript_for_imported_json_values() {
        assert!(!onboarding_welcome_truthy(None).expect("none"));
        assert!(!onboarding_welcome_truthy(Some(r#"{"welcome":false}"#)).expect("false"));
        assert!(onboarding_welcome_truthy(Some(r#"{"welcome":"yes"}"#)).expect("string"));
        assert!(onboarding_welcome_truthy(Some(r#"{"welcome":{}}"#)).expect("object"));
    }

    #[test]
    fn provider_effect_parser_preserves_best_effort_failure_states() {
        assert_eq!(
            parse_provider_effect("best_effort_failed").expect("effect"),
            LegacyUserAccountProviderEffectV1::BestEffortFailed
        );
        assert!(parse_provider_effect("pretend_applied").is_err());
    }
}
