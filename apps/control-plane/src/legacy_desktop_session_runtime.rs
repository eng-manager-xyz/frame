//! D1 credential mint for Cap's desktop session handoff.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, Error, Result};

const MINT_DESKTOP_KEY_SQL: &str =
    include_str!("../queries/legacy_desktop_session/mint_desktop_key.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyDesktopSessionRuntimeFailureV1 {
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct InsertedKeyRowV1 {
    id: String,
}

pub(crate) struct D1LegacyDesktopSessionV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyDesktopSessionV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    /// Return a one-time raw UUID while retaining only its SHA-256 digest.
    pub(crate) async fn mint_desktop_key(
        &self,
        actor_id: &str,
        now_ms: i64,
    ) -> Result<std::result::Result<String, LegacyDesktopSessionRuntimeFailureV1>> {
        if actor_id.is_empty()
            || actor_id.len() > 255
            || !actor_id.is_ascii()
            || actor_id.bytes().any(|byte| byte.is_ascii_control())
            || now_ms < 0
        {
            return Ok(Err(LegacyDesktopSessionRuntimeFailureV1::Unavailable));
        }
        let raw_key = uuid::Uuid::new_v4().to_string();
        let row_id = uuid::Uuid::now_v7().to_string();
        let digest = sha256_hex(raw_key.as_bytes());
        let row = self
            .database
            .prepare(MINT_DESKTOP_KEY_SQL)
            .bind(&[
                JsValue::from_str(&row_id),
                JsValue::from_str(actor_id),
                JsValue::from_str(&digest),
                JsValue::from_f64(now_ms as f64),
            ])?
            .first::<InsertedKeyRowV1>(None)
            .await
            .map_err(|_| Error::RustError("desktop API-key mint is unavailable".into()))?;
        match row {
            Some(row) if row.id == row_id => Ok(Ok(raw_key)),
            _ => Ok(Err(LegacyDesktopSessionRuntimeFailureV1::Unavailable)),
        }
    }
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
    fn checked_in_sql_is_digest_only_active_actor_scoped_and_returns_exact_row() {
        assert!(MINT_DESKTOP_KEY_SQL.contains("key_digest"));
        assert!(MINT_DESKTOP_KEY_SQL.contains("legacy_source"));
        assert!(MINT_DESKTOP_KEY_SQL.contains("'desktop'"));
        assert!(MINT_DESKTOP_KEY_SQL.contains("u.status = 'active'"));
        assert!(MINT_DESKTOP_KEY_SQL.contains("u.deleted_at_ms IS NULL"));
        assert!(MINT_DESKTOP_KEY_SQL.contains("RETURNING id"));
        assert!(!MINT_DESKTOP_KEY_SQL.contains("raw_key"));
        assert_eq!(sha256_hex(b"secret").len(), 64);
    }
}
