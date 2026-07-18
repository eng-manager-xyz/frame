//! Tenant-scoped D1 adapter for Cap's space authorization reads.

use async_trait::async_trait;
use frame_application::{
    LegacySpaceAuthorizationPortErrorV1, LegacySpaceAuthorizationPortV1,
    LegacySpaceAuthorizationPrincipalV1, LegacySpaceAuthorizationSnapshotV1,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const PRINCIPAL_SCOPE_SQL: &str =
    include_str!("../queries/legacy_space_authorization/principal_scope.sql");
const ACCESS_READ_SQL: &str = include_str!("../queries/legacy_space_authorization/access_read.sql");
const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_space_authorization/clock_now.sql");
const USER_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_space_authorization/user_alias_insert.sql");
const USER_ALIAS_READ_SQL: &str =
    include_str!("../queries/legacy_space_authorization/user_alias_read.sql");
const NATIVE_ALIAS_ATTEMPTS: u8 = 8;
const LEGACY_NANOID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

#[derive(Debug, Deserialize)]
struct PrincipalRow {
    actor_id: String,
    active_organization_id: String,
    active_legacy_organization_id: String,
}

#[derive(Debug, Deserialize)]
struct AccessRow {
    legacy_space_id: String,
    legacy_organization_id: String,
    organization_owner_user_id: String,
    created_by_user_id: String,
    legacy_organization_owner_id: Option<String>,
    legacy_created_by_id: Option<String>,
    membership_organization_owner_id: Option<String>,
    membership_created_by_id: Option<String>,
    actor_is_organization_owner: i64,
    actor_is_space_creator: i64,
    organization_member_role: Option<String>,
    space_member_role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Deserialize)]
struct UserAliasRow {
    legacy_user_id: String,
}

pub(crate) struct D1LegacySpaceAuthorizationPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacySpaceAuthorizationPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn principal_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacySpaceAuthorizationPrincipalV1, LegacySpaceAuthorizationPortErrorV1> {
        if !valid_boundary_id(actor_id) {
            return Err(LegacySpaceAuthorizationPortErrorV1::NotVisible);
        }
        let mut rows = self
            .query::<PrincipalRow>(PRINCIPAL_SCOPE_SQL, &[actor_id])
            .await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                LegacySpaceAuthorizationPortErrorV1::NotVisible
            } else {
                LegacySpaceAuthorizationPortErrorV1::Corrupt
            });
        }
        let row = rows.pop().expect("one principal row");
        let principal = LegacySpaceAuthorizationPrincipalV1 {
            actor_id: row.actor_id,
            active_organization_id: row.active_organization_id,
            active_legacy_organization_id: row.active_legacy_organization_id,
        };
        principal
            .valid()
            .then_some(principal)
            .ok_or(LegacySpaceAuthorizationPortErrorV1::Corrupt)
    }

    async fn query<T: DeserializeOwned>(
        &self,
        sql: &str,
        binds: &[&str],
    ) -> Result<Vec<T>, LegacySpaceAuthorizationPortErrorV1> {
        let values = binds
            .iter()
            .map(|value| JsValue::from_str(value))
            .collect::<Vec<_>>();
        let result = self
            .database
            .prepare(sql)
            .bind(&values)
            .map_err(|_| LegacySpaceAuthorizationPortErrorV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacySpaceAuthorizationPortErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacySpaceAuthorizationPortErrorV1::Unavailable);
        }
        result
            .results::<T>()
            .map_err(|_| LegacySpaceAuthorizationPortErrorV1::Corrupt)
    }

    async fn access_row(
        &self,
        principal: &LegacySpaceAuthorizationPrincipalV1,
        legacy_space_id: &str,
    ) -> Result<Option<AccessRow>, LegacySpaceAuthorizationPortErrorV1> {
        let mut rows = self
            .query::<AccessRow>(
                ACCESS_READ_SQL,
                &[
                    &principal.actor_id,
                    &principal.active_organization_id,
                    &principal.active_legacy_organization_id,
                    legacy_space_id,
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacySpaceAuthorizationPortErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn clock_now(&self) -> Result<i64, LegacySpaceAuthorizationPortErrorV1> {
        let mut rows = self.query::<ClockRow>(CLOCK_NOW_SQL, &[]).await?;
        if rows.len() != 1 {
            return Err(LegacySpaceAuthorizationPortErrorV1::Corrupt);
        }
        let now_ms = rows.pop().expect("one clock row").now_ms;
        (0..=9_007_199_254_740_991)
            .contains(&now_ms)
            .then_some(now_ms)
            .ok_or(LegacySpaceAuthorizationPortErrorV1::Corrupt)
    }

    async fn persisted_alias(
        &self,
        user_id: &str,
    ) -> Result<Option<String>, LegacySpaceAuthorizationPortErrorV1> {
        let mut rows = self
            .query::<UserAliasRow>(USER_ALIAS_READ_SQL, &[user_id])
            .await?;
        if rows.len() > 1 {
            return Err(LegacySpaceAuthorizationPortErrorV1::Corrupt);
        }
        rows.pop()
            .map(|row| {
                frame_application::valid_legacy_nanoid(&row.legacy_user_id)
                    .then_some(row.legacy_user_id)
                    .ok_or(LegacySpaceAuthorizationPortErrorV1::Corrupt)
            })
            .transpose()
    }

    async fn insert_alias(
        &self,
        legacy_user_id: &str,
        user_id: &str,
        provenance: &str,
        now_ms: i64,
    ) -> Result<(), LegacySpaceAuthorizationPortErrorV1> {
        let values = [
            JsValue::from_str(legacy_user_id),
            JsValue::from_str(user_id),
            JsValue::from_str(provenance),
            JsValue::from_f64(now_ms as f64),
        ];
        let result = self
            .database
            .prepare(USER_ALIAS_INSERT_SQL)
            .bind(&values)
            .map_err(|_| LegacySpaceAuthorizationPortErrorV1::Unavailable)?
            .run()
            .into_send()
            .await
            .map_err(|_| LegacySpaceAuthorizationPortErrorV1::Unavailable)?;
        result
            .success()
            .then_some(())
            .ok_or(LegacySpaceAuthorizationPortErrorV1::Unavailable)
    }

    async fn ensure_persisted_alias(
        &self,
        user_id: &str,
        current: Option<&str>,
        membership: Option<&str>,
        now_ms: i64,
    ) -> Result<String, LegacySpaceAuthorizationPortErrorV1> {
        if !valid_boundary_id(user_id)
            || current.is_some_and(|value| !frame_application::valid_legacy_nanoid(value))
            || membership.is_some_and(|value| !frame_application::valid_legacy_nanoid(value))
            || current
                .zip(membership)
                .is_some_and(|(left, right)| left != right)
        {
            return Err(LegacySpaceAuthorizationPortErrorV1::Corrupt);
        }
        if let Some(current) = current {
            return Ok(current.to_owned());
        }
        if let Some(membership) = membership {
            self.insert_alias(membership, user_id, "membership_backfill", now_ms)
                .await?;
            return self
                .persisted_alias(user_id)
                .await?
                .filter(|persisted| persisted == membership)
                .ok_or(LegacySpaceAuthorizationPortErrorV1::Corrupt);
        }
        for attempt in 0..NATIVE_ALIAS_ATTEMPTS {
            let candidate = native_alias_candidate(user_id, attempt);
            self.insert_alias(&candidate, user_id, "native_generated", now_ms)
                .await?;
            if let Some(persisted) = self.persisted_alias(user_id).await? {
                return Ok(persisted);
            }
            // The globally unique candidate belonged to another user. The
            // next domain-separated digest is deterministic and retry-safe.
        }
        Err(LegacySpaceAuthorizationPortErrorV1::Corrupt)
    }
}

#[async_trait]
impl LegacySpaceAuthorizationPortV1 for D1LegacySpaceAuthorizationPortV1<'_> {
    async fn get_space_access(
        &self,
        principal: &LegacySpaceAuthorizationPrincipalV1,
        legacy_space_id: &str,
    ) -> Result<Option<LegacySpaceAuthorizationSnapshotV1>, LegacySpaceAuthorizationPortErrorV1>
    {
        if !principal.valid() || !frame_application::valid_legacy_nanoid(legacy_space_id) {
            return Err(LegacySpaceAuthorizationPortErrorV1::NotVisible);
        }
        // This query reasserts the complete principal snapshot, so an active
        // tenant or membership race becomes source-shaped non-visibility.
        let Some(row) = self.access_row(principal, legacy_space_id).await? else {
            return Ok(None);
        };
        if row.legacy_space_id != legacy_space_id
            || row.legacy_organization_id != principal.active_legacy_organization_id
        {
            return Err(LegacySpaceAuthorizationPortErrorV1::Corrupt);
        }
        let aliases_missing =
            row.legacy_organization_owner_id.is_none() || row.legacy_created_by_id.is_none();
        if aliases_missing {
            let now_ms = self.clock_now().await?;
            self.ensure_persisted_alias(
                &row.organization_owner_user_id,
                row.legacy_organization_owner_id.as_deref(),
                row.membership_organization_owner_id.as_deref(),
                now_ms,
            )
            .await?;
            self.ensure_persisted_alias(
                &row.created_by_user_id,
                row.legacy_created_by_id.as_deref(),
                row.membership_created_by_id.as_deref(),
                now_ms,
            )
            .await?;
        } else {
            // Even a pre-existing global alias must agree with imported member
            // aliases; otherwise the source user identity is ambiguous.
            self.ensure_persisted_alias(
                &row.organization_owner_user_id,
                row.legacy_organization_owner_id.as_deref(),
                row.membership_organization_owner_id.as_deref(),
                0,
            )
            .await?;
            self.ensure_persisted_alias(
                &row.created_by_user_id,
                row.legacy_created_by_id.as_deref(),
                row.membership_created_by_id.as_deref(),
                0,
            )
            .await?;
        }
        let row = if aliases_missing {
            self.access_row(principal, legacy_space_id)
                .await?
                .ok_or(LegacySpaceAuthorizationPortErrorV1::NotVisible)?
        } else {
            row
        };
        Ok(Some(LegacySpaceAuthorizationSnapshotV1 {
            legacy_space_id: row.legacy_space_id,
            legacy_organization_id: row.legacy_organization_id,
            legacy_organization_owner_id: row
                .legacy_organization_owner_id
                .ok_or(LegacySpaceAuthorizationPortErrorV1::Corrupt)?,
            legacy_created_by_id: row
                .legacy_created_by_id
                .ok_or(LegacySpaceAuthorizationPortErrorV1::Corrupt)?,
            actor_is_organization_owner: decode_bool(row.actor_is_organization_owner)?,
            actor_is_space_creator: decode_bool(row.actor_is_space_creator)?,
            organization_member_role: row.organization_member_role,
            space_member_role: row.space_member_role,
        }))
    }
}

fn native_alias_candidate(user_id: &str, attempt: u8) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-space-authorization-native-user-alias-v1\0");
    digest.update(user_id.as_bytes());
    digest.update([0, attempt]);
    let digest = digest.finalize();
    let mut encoded = String::with_capacity(15);
    let mut bit_offset = 0usize;
    for _ in 0..15 {
        let byte_index = bit_offset / 8;
        let shift = bit_offset % 8;
        let pair = u16::from(digest[byte_index]) << 8
            | u16::from(digest.get(byte_index + 1).copied().unwrap_or(0));
        let value = ((pair << shift) >> 11) as usize;
        encoded.push(char::from(LEGACY_NANOID_ALPHABET[value]));
        bit_offset += 5;
    }
    encoded
}

fn decode_bool(value: i64) -> Result<bool, LegacySpaceAuthorizationPortErrorV1> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(LegacySpaceAuthorizationPortErrorV1::Corrupt),
    }
}

fn valid_boundary_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_queries_reassert_active_tenant_alias_and_live_rows() {
        for query in [
            PRINCIPAL_SCOPE_SQL,
            ACCESS_READ_SQL,
            CLOCK_NOW_SQL,
            USER_ALIAS_READ_SQL,
        ] {
            assert!(!query.is_empty());
            assert!(!query.contains("SELECT *"));
            assert!(query.trim_end().ends_with("LIMIT 2;"));
        }
        assert!(USER_ALIAS_INSERT_SQL.contains("INSERT OR IGNORE"));
        assert!(USER_ALIAS_INSERT_SQL.contains("legacy_collaboration_user_aliases_v1"));
        for token in [
            "actor.active_organization_id",
            "organization.tombstoned_at_ms IS NULL",
            "legacy_library_space_aliases_v1",
            "space.organization_id = organization.id",
            "space.deleted_at_ms IS NULL",
            "organization_membership.state = 'active'",
            "space_membership.state = 'active'",
        ] {
            assert!(ACCESS_READ_SQL.contains(token), "missing {token}");
        }
    }

    #[test]
    fn frame_space_roles_map_back_to_the_legacy_role_vocabulary() {
        assert!(ACCESS_READ_SQL.contains("WHEN 'manager' THEN 'admin'"));
        assert!(ACCESS_READ_SQL.contains("WHEN 'viewer' THEN 'member'"));
        assert!(ACCESS_READ_SQL.contains("ELSE space_membership.role"));
    }

    #[test]
    fn boolean_decoder_rejects_driver_or_schema_drift() {
        assert_eq!(decode_bool(0), Ok(false));
        assert_eq!(decode_bool(1), Ok(true));
        assert_eq!(
            decode_bool(2),
            Err(LegacySpaceAuthorizationPortErrorV1::Corrupt)
        );
    }

    #[test]
    fn native_aliases_are_stable_valid_and_have_bounded_collision_candidates() {
        let user = "00000000-0000-4000-8000-000000000001";
        let first = native_alias_candidate(user, 0);
        assert_eq!(first, "bkeshjh7fd0reqq");
        assert_eq!(first, native_alias_candidate(user, 0));
        assert!(frame_application::valid_legacy_nanoid(&first));
        let candidates = (0..NATIVE_ALIAS_ATTEMPTS)
            .map(|attempt| native_alias_candidate(user, attempt))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(candidates.len(), usize::from(NATIVE_ALIAS_ATTEMPTS));
        assert_ne!(
            first,
            native_alias_candidate("00000000-0000-4000-8000-000000000002", 0)
        );
    }
}
