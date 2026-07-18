//! D1 adapter for Cap's extension credential and bootstrap service.

use frame_application::{
    LEGACY_EXTENSION_AUTH_KEY_MINT_LIMIT, LegacyExtensionBootstrapPlanV1,
    legacy_extension_user_is_pro,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, Error, Result, send::IntoSendFuture};

const SESSION_USER_SQL: &str = include_str!("../queries/legacy_extension_auth/session_user.sql");
const API_KEY_ACTOR_SQL: &str = include_str!("../queries/legacy_extension_auth/api_key_actor.sql");
const MINT_INSERT_SQL: &str = include_str!("../queries/legacy_extension_auth/mint_insert.sql");
const MINT_OVERFLOW_DELETE_SQL: &str =
    include_str!("../queries/legacy_extension_auth/mint_overflow_delete.sql");
const MINT_ASSERT_SQL: &str = include_str!("../queries/legacy_extension_auth/mint_assert.sql");
const MINT_RECENT_COUNT_SQL: &str =
    include_str!("../queries/legacy_extension_auth/mint_recent_count.sql");
const ASSERTION_DELETE_SQL: &str =
    include_str!("../queries/legacy_extension_auth/assertion_delete.sql");
const REVOKE_OWNED_SQL: &str = include_str!("../queries/legacy_extension_auth/revoke_owned.sql");
const BOOTSTRAP_RESOLVE_SQL: &str =
    include_str!("../queries/legacy_extension_auth/bootstrap_resolve.sql");
const BOOTSTRAP_REPAIR_SQL: &str =
    include_str!("../queries/legacy_extension_auth/bootstrap_repair.sql");
const BOOTSTRAP_REPAIR_ASSERT_SQL: &str =
    include_str!("../queries/legacy_extension_auth/bootstrap_repair_assert.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyExtensionRuntimeFailureV1 {
    RateLimited,
    MissingOrganization,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct LegacyExtensionActorV1 {
    pub id: String,
    pub email: String,
}

impl LegacyExtensionActorV1 {
    fn valid(&self) -> bool {
        valid_id(&self.id)
            && !self.email.is_empty()
            && self.email.len() <= 255
            && !self.email.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LegacyExtensionBootstrapV1 {
    pub user: LegacyExtensionBootstrapUserV1,
    pub organization: LegacyExtensionBootstrapOrganizationV1,
    pub plan: LegacyExtensionBootstrapPlanV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LegacyExtensionBootstrapUserV1 {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LegacyExtensionBootstrapOrganizationV1 {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct BootstrapRowV1 {
    id: String,
    name: String,
    stripe_subscription_status: Option<String>,
    third_party_stripe_subscription_id: Option<String>,
    active_organization_id: Option<String>,
}

impl BootstrapRowV1 {
    fn valid(&self) -> bool {
        valid_id(&self.id)
            && self.name.len() <= 255
            && !self.name.chars().any(char::is_control)
            && self
                .stripe_subscription_status
                .as_deref()
                .is_none_or(|value| value.len() <= 255 && !value.chars().any(char::is_control))
            && self
                .third_party_stripe_subscription_id
                .as_deref()
                .is_none_or(|value| value.len() <= 255 && !value.chars().any(char::is_control))
            && self.active_organization_id.as_deref().is_none_or(valid_id)
    }
}

#[derive(Debug, Deserialize)]
struct RecentCountRowV1 {
    recent_count: i64,
}

pub(crate) struct D1LegacyExtensionAuthV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyExtensionAuthV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn session_actor(
        &self,
        actor_id: &str,
    ) -> Result<std::result::Result<Option<LegacyExtensionActorV1>, LegacyExtensionRuntimeFailureV1>>
    {
        let row = self
            .first::<LegacyExtensionActorV1>(SESSION_USER_SQL, &[JsValue::from_str(actor_id)])
            .await?;
        Ok(match row {
            Some(row) if row.valid() => Ok(Some(row)),
            Some(_) => Err(LegacyExtensionRuntimeFailureV1::Corrupt),
            None => Ok(None),
        })
    }

    pub(crate) async fn api_key_actor(
        &self,
        auth_api_key: &str,
        now_ms: i64,
    ) -> Result<std::result::Result<Option<LegacyExtensionActorV1>, LegacyExtensionRuntimeFailureV1>>
    {
        let digest = sha256_hex(auth_api_key.as_bytes());
        let row = self
            .first::<LegacyExtensionActorV1>(
                API_KEY_ACTOR_SQL,
                &[JsValue::from_str(&digest), JsValue::from_f64(now_ms as f64)],
            )
            .await?;
        Ok(match row {
            Some(row) if row.valid() => Ok(Some(row)),
            Some(_) => Err(LegacyExtensionRuntimeFailureV1::Corrupt),
            None => Ok(None),
        })
    }

    pub(crate) async fn mint_auth_key(
        &self,
        actor_id: &str,
        now_ms: i64,
    ) -> Result<std::result::Result<String, LegacyExtensionRuntimeFailureV1>> {
        let auth_api_key = uuid::Uuid::new_v4().to_string();
        let key_row_id = uuid::Uuid::now_v7().to_string();
        let operation_id = uuid::Uuid::now_v7().to_string();
        let key_digest = sha256_hex(auth_api_key.as_bytes());
        let statements = vec![
            self.statement(
                MINT_INSERT_SQL,
                &[
                    JsValue::from_str(&key_row_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&key_digest),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
            self.statement(
                MINT_OVERFLOW_DELETE_SQL,
                &[
                    JsValue::from_str(&key_row_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
            self.statement(
                MINT_ASSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&key_row_id),
                    JsValue::from_str(actor_id),
                    JsValue::from_str(&key_digest),
                ],
            )?,
            self.statement(ASSERTION_DELETE_SQL, &[JsValue::from_str(&operation_id)])?,
        ];
        if self.batch(statements).await.is_err() {
            let count = self.recent_count(actor_id, now_ms).await?;
            return Ok(Err(
                if count >= LEGACY_EXTENSION_AUTH_KEY_MINT_LIMIT as i64 {
                    LegacyExtensionRuntimeFailureV1::RateLimited
                } else {
                    LegacyExtensionRuntimeFailureV1::Unavailable
                },
            ));
        }
        Ok(Ok(auth_api_key))
    }

    pub(crate) async fn revoke_owned_key(
        &self,
        actor_id: &str,
        auth_api_key: &str,
    ) -> Result<std::result::Result<(), LegacyExtensionRuntimeFailureV1>> {
        let digest = sha256_hex(auth_api_key.as_bytes());
        let result = self
            .statement(
                REVOKE_OWNED_SQL,
                &[JsValue::from_str(actor_id), JsValue::from_str(&digest)],
            )?
            .run()
            .into_send()
            .await
            .map_err(|_| Error::RustError("extension key revocation is unavailable".into()))?;
        if !result.success() {
            return Ok(Err(LegacyExtensionRuntimeFailureV1::Unavailable));
        }
        Ok(Ok(()))
    }

    pub(crate) async fn bootstrap(
        &self,
        actor: &LegacyExtensionActorV1,
        now_ms: i64,
        is_cap_hosted: bool,
    ) -> Result<std::result::Result<LegacyExtensionBootstrapV1, LegacyExtensionRuntimeFailureV1>>
    {
        let Some(row) = self
            .first::<BootstrapRowV1>(BOOTSTRAP_RESOLVE_SQL, &[JsValue::from_str(&actor.id)])
            .await?
        else {
            return Ok(Err(LegacyExtensionRuntimeFailureV1::MissingOrganization));
        };
        if !row.valid() {
            return Ok(Err(LegacyExtensionRuntimeFailureV1::Corrupt));
        }
        if row.active_organization_id.as_deref() != Some(row.id.as_str()) {
            let operation_id = uuid::Uuid::now_v7().to_string();
            let statements = vec![
                self.statement(
                    BOOTSTRAP_REPAIR_SQL,
                    &[
                        JsValue::from_str(&actor.id),
                        JsValue::from_str(&row.id),
                        JsValue::from_str(&operation_id),
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?,
                self.statement(
                    BOOTSTRAP_REPAIR_ASSERT_SQL,
                    &[
                        JsValue::from_str(&operation_id),
                        JsValue::from_str(&actor.id),
                        JsValue::from_str(&row.id),
                    ],
                )?,
                self.statement(ASSERTION_DELETE_SQL, &[JsValue::from_str(&operation_id)])?,
            ];
            if self.batch(statements).await.is_err() {
                return Ok(Err(LegacyExtensionRuntimeFailureV1::Unavailable));
            }
        }
        let is_pro = legacy_extension_user_is_pro(
            is_cap_hosted,
            row.stripe_subscription_status.as_deref(),
            row.third_party_stripe_subscription_id.as_deref(),
        );
        Ok(Ok(LegacyExtensionBootstrapV1 {
            user: LegacyExtensionBootstrapUserV1 {
                id: actor.id.clone(),
                email: actor.email.clone(),
            },
            organization: LegacyExtensionBootstrapOrganizationV1 {
                id: row.id,
                name: row.name,
            },
            plan: LegacyExtensionBootstrapPlanV1::from_pro(is_pro),
        }))
    }

    async fn recent_count(&self, actor_id: &str, now_ms: i64) -> Result<i64> {
        let row = self
            .first::<RecentCountRowV1>(
                MINT_RECENT_COUNT_SQL,
                &[
                    JsValue::from_str(actor_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )
            .await?;
        Ok(row.map_or(0, |row| row.recent_count))
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> Result<D1PreparedStatement> {
        self.database.prepare(sql).bind(bindings)
    }

    async fn first<T>(&self, sql: &str, bindings: &[JsValue]) -> Result<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.statement(sql, bindings)?.first::<T>(None).await
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> std::result::Result<(), LegacyExtensionRuntimeFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyExtensionRuntimeFailureV1::Unavailable)?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyExtensionRuntimeFailureV1::Unavailable);
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_sql_retains_atomic_mint_and_deterministic_bootstrap_contracts() {
        assert!(MINT_INSERT_SQL.contains("legacy_source"));
        assert!(MINT_OVERFLOW_DELETE_SQL.contains(") > 10"));
        assert!(MINT_OVERFLOW_DELETE_SQL.contains("created_at_ms > ?3 - 3600000"));
        assert!(MINT_ASSERT_SQL.contains("mint_within_hourly_limit"));
        assert!(REVOKE_OWNED_SQL.contains("user_id = ?1"));
        assert!(REVOKE_OWNED_SQL.contains("key_digest = ?2"));
        assert!(BOOTSTRAP_RESOLVE_SQL.contains("0 AS priority"));
        assert!(BOOTSTRAP_RESOLVE_SQL.contains("1 AS priority"));
        assert!(BOOTSTRAP_RESOLVE_SQL.contains("2 AS priority"));
        assert!(BOOTSTRAP_RESOLVE_SQL.contains("ORDER BY priority, membership_created_at_ms, id"));
        assert!(BOOTSTRAP_REPAIR_SQL.contains("organization_preference_revision + 1"));
    }

    #[test]
    fn secrets_are_digest_only_and_bootstrap_json_is_exact() {
        assert_eq!(sha256_hex(b"secret").len(), 64);
        assert!(MINT_INSERT_SQL.contains("key_digest"));
        assert!(!MINT_INSERT_SQL.contains("authApiKey"));
        let response = LegacyExtensionBootstrapV1 {
            user: LegacyExtensionBootstrapUserV1 {
                id: "user".into(),
                email: "user@example.com".into(),
            },
            organization: LegacyExtensionBootstrapOrganizationV1 {
                id: "org".into(),
                name: "Org".into(),
            },
            plan: LegacyExtensionBootstrapPlanV1::from_pro(false),
        };
        assert_eq!(
            serde_json::to_value(response).expect("json"),
            serde_json::json!({
                "user": {"id": "user", "email": "user@example.com"},
                "organization": {"id": "org", "name": "Org"},
                "plan": {"isPro": false, "maxRecordingSeconds": 300}
            })
        );
    }
}
