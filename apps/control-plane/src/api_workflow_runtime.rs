//! D1 authority for signed-webhook replay claims.
//!
//! Provider parsing and HMAC verification stay in `frame-application`. This
//! adapter performs the single atomic insert-if-absent required before a
//! verified callback can reach a business authority.

use async_trait::async_trait;
use frame_application::{ReplayClaimV1, WebhookReplayStoreV1, WebhookStoreErrorV1};
use frame_domain::{ChecksumSha256, TimestampMillis};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const CLAIM_SQL: &str = include_str!("../queries/api_workflow/webhook_replay_claim.sql");
const PRUNE_SQL: &str = include_str!("../queries/api_workflow/webhook_replay_prune.sql");
const MAX_CLAIM_LIFETIME_MS: i64 = 30 * 60 * 1_000;
const MAX_PRUNE_ROWS: u16 = 1_000;

#[derive(Deserialize)]
struct ReplayDigestRow {
    replay_digest: String,
}

pub struct D1WebhookReplayStoreV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1WebhookReplayStoreV1<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    /// Delete only claims that expired before the trusted current time. The
    /// batch is bounded so cleanup cannot monopolize a Worker invocation.
    pub async fn prune_expired(
        &self,
        now_ms: i64,
        limit: u16,
    ) -> Result<usize, WebhookStoreErrorV1> {
        if TimestampMillis::new(now_ms).is_err() || !(1..=MAX_PRUNE_ROWS).contains(&limit) {
            return Err(WebhookStoreErrorV1);
        }
        let statement = self
            .database
            .prepare(PRUNE_SQL)
            .bind(&[
                JsValue::from_f64(now_ms as f64),
                JsValue::from_f64(f64::from(limit)),
            ])
            .map_err(|_| WebhookStoreErrorV1)?;
        let result = statement
            .all()
            .into_send()
            .await
            .map_err(|_| WebhookStoreErrorV1)?;
        if !result.success() {
            return Err(WebhookStoreErrorV1);
        }
        result
            .results::<ReplayDigestRow>()
            .map(|rows| rows.len())
            .map_err(|_| WebhookStoreErrorV1)
    }
}

#[async_trait]
impl WebhookReplayStoreV1 for D1WebhookReplayStoreV1<'_> {
    async fn claim_once(
        &self,
        digest: &ChecksumSha256,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<ReplayClaimV1, WebhookStoreErrorV1> {
        if TimestampMillis::new(now_ms).is_err()
            || TimestampMillis::new(expires_at_ms).is_err()
            || expires_at_ms <= now_ms
            || expires_at_ms - now_ms > MAX_CLAIM_LIFETIME_MS
        {
            return Err(WebhookStoreErrorV1);
        }
        let statement = self
            .database
            .prepare(CLAIM_SQL)
            .bind(&[
                JsValue::from_str(digest.as_str()),
                JsValue::from_f64(now_ms as f64),
                JsValue::from_f64(expires_at_ms as f64),
            ])
            .map_err(|_| WebhookStoreErrorV1)?;
        let claimed = statement
            .first::<ReplayDigestRow>(None)
            .into_send()
            .await
            .map_err(|_| WebhookStoreErrorV1)?;
        match claimed {
            Some(row) if row.replay_digest == digest.as_str() => Ok(ReplayClaimV1::Claimed),
            Some(_) => Err(WebhookStoreErrorV1),
            None => Ok(ReplayClaimV1::Duplicate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_bound_atomic_and_bounded() {
        assert!(CLAIM_SQL.starts_with("INSERT OR IGNORE"));
        assert!(CLAIM_SQL.contains("VALUES (?1, ?2, ?3)"));
        assert!(CLAIM_SQL.contains("RETURNING replay_digest"));
        assert!(!CLAIM_SQL.contains("DELETE"));
        assert!(PRUNE_SQL.contains("expires_at_ms < ?1"));
        assert!(PRUNE_SQL.contains("LIMIT ?2"));
        assert!(PRUNE_SQL.contains("ORDER BY expires_at_ms, replay_digest"));
    }

    #[test]
    fn adapter_limits_match_the_domain_verifier_ceiling() {
        assert_eq!(MAX_CLAIM_LIFETIME_MS, 1_800_000);
        assert_eq!(MAX_PRUNE_ROWS, 1_000);
    }
}
