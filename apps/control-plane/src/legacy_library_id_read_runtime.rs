//! Tenant-scoped D1 adapter for Cap's library membership ID reads.

use async_trait::async_trait;
use frame_application::{
    LegacyLibraryIdReadPortErrorV1, LegacyLibraryIdReadPortV1, LegacyLibraryIdReadPrincipalV1,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const PRINCIPAL_SCOPE_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/principal_scope.sql");
const FOLDER_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/folder_authority.sql");
const FOLDER_ORGANIZATION_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/folder_video_ids_organization.sql");
const FOLDER_SPACE_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/folder_video_ids_space.sql");
const ORGANIZATION_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/organization_authority.sql");
const ORGANIZATION_VIDEO_IDS_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/organization_video_ids.sql");
const SPACE_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/space_authority.sql");
const SPACE_VIDEO_IDS_SQL: &str =
    include_str!("../queries/legacy_library_id_reads/space_video_ids.sql");

#[derive(Debug, Deserialize)]
struct PrincipalRow {
    actor_id: String,
    active_organization_id: String,
    active_legacy_organization_id: String,
}

#[derive(Debug, Deserialize)]
struct AuthorityRow {
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct VideoIdRow {
    legacy_video_id: Option<String>,
}

pub(crate) struct D1LegacyLibraryIdReadPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyLibraryIdReadPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn principal_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacyLibraryIdReadPrincipalV1, LegacyLibraryIdReadPortErrorV1> {
        if !valid_boundary_id(actor_id) {
            return Err(LegacyLibraryIdReadPortErrorV1::NotVisible);
        }
        let mut rows = self
            .query::<PrincipalRow>(PRINCIPAL_SCOPE_SQL, &[actor_id])
            .await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                LegacyLibraryIdReadPortErrorV1::NotVisible
            } else {
                LegacyLibraryIdReadPortErrorV1::Corrupt
            });
        }
        let row = rows.pop().expect("one principal row");
        let principal = LegacyLibraryIdReadPrincipalV1 {
            actor_id: row.actor_id,
            active_organization_id: row.active_organization_id,
            active_legacy_organization_id: row.active_legacy_organization_id,
        };
        principal
            .valid()
            .then_some(principal)
            .ok_or(LegacyLibraryIdReadPortErrorV1::Corrupt)
    }

    async fn reassert_principal(
        &self,
        expected: &LegacyLibraryIdReadPrincipalV1,
    ) -> Result<(), LegacyLibraryIdReadPortErrorV1> {
        let actual = self.principal_for_actor(&expected.actor_id).await?;
        (actual == *expected)
            .then_some(())
            .ok_or(LegacyLibraryIdReadPortErrorV1::NotVisible)
    }

    async fn require_authority(
        &self,
        sql: &str,
        binds: &[&str],
    ) -> Result<(), LegacyLibraryIdReadPortErrorV1> {
        let rows = self.query::<AuthorityRow>(sql, binds).await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                LegacyLibraryIdReadPortErrorV1::NotVisible
            } else {
                LegacyLibraryIdReadPortErrorV1::Corrupt
            });
        }
        // Reject a driver/schema mismatch that decoded an empty flattened row.
        (!rows[0].fields.is_empty())
            .then_some(())
            .ok_or(LegacyLibraryIdReadPortErrorV1::Corrupt)
    }

    async fn video_ids(
        &self,
        sql: &str,
        binds: &[&str],
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
        self.query::<VideoIdRow>(sql, binds)
            .await?
            .into_iter()
            .map(|row| {
                row.legacy_video_id
                    .filter(|value| valid_legacy_nanoid(value))
                    .ok_or(LegacyLibraryIdReadPortErrorV1::Corrupt)
            })
            .collect()
    }

    async fn query<T: DeserializeOwned>(
        &self,
        sql: &str,
        binds: &[&str],
    ) -> Result<Vec<T>, LegacyLibraryIdReadPortErrorV1> {
        let values = binds
            .iter()
            .map(|value| JsValue::from_str(value))
            .collect::<Vec<_>>();
        let result = self
            .database
            .prepare(sql)
            .bind(&values)
            .map_err(|_| LegacyLibraryIdReadPortErrorV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyLibraryIdReadPortErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyLibraryIdReadPortErrorV1::Unavailable);
        }
        result
            .results::<T>()
            .map_err(|_| LegacyLibraryIdReadPortErrorV1::Corrupt)
    }
}

#[async_trait]
impl LegacyLibraryIdReadPortV1 for D1LegacyLibraryIdReadPortV1<'_> {
    async fn folder_video_ids(
        &self,
        principal: &LegacyLibraryIdReadPrincipalV1,
        legacy_folder_id: &str,
        legacy_space_or_organization_id: &str,
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
        self.reassert_principal(principal).await?;
        self.require_authority(
            FOLDER_AUTHORITY_SQL,
            &[
                &principal.actor_id,
                &principal.active_organization_id,
                legacy_folder_id,
                legacy_space_or_organization_id,
                &principal.active_legacy_organization_id,
            ],
        )
        .await?;
        if legacy_space_or_organization_id == principal.active_legacy_organization_id {
            self.video_ids(
                FOLDER_ORGANIZATION_SQL,
                &[legacy_folder_id, &principal.active_organization_id],
            )
            .await
        } else {
            self.video_ids(
                FOLDER_SPACE_SQL,
                &[
                    legacy_folder_id,
                    legacy_space_or_organization_id,
                    &principal.active_organization_id,
                ],
            )
            .await
        }
    }

    async fn organization_video_ids(
        &self,
        principal: &LegacyLibraryIdReadPrincipalV1,
        legacy_organization_id: &str,
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
        self.reassert_principal(principal).await?;
        self.require_authority(
            ORGANIZATION_AUTHORITY_SQL,
            &[
                &principal.actor_id,
                legacy_organization_id,
                &principal.active_organization_id,
            ],
        )
        .await?;
        self.video_ids(
            ORGANIZATION_VIDEO_IDS_SQL,
            &[legacy_organization_id, &principal.active_organization_id],
        )
        .await
    }

    async fn space_video_ids(
        &self,
        principal: &LegacyLibraryIdReadPrincipalV1,
        legacy_space_or_organization_id: &str,
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
        self.reassert_principal(principal).await?;
        if legacy_space_or_organization_id == principal.active_legacy_organization_id {
            return self
                .organization_video_ids(principal, legacy_space_or_organization_id)
                .await;
        }
        self.require_authority(
            SPACE_AUTHORITY_SQL,
            &[
                &principal.actor_id,
                legacy_space_or_organization_id,
                &principal.active_organization_id,
            ],
        )
        .await?;
        self.video_ids(SPACE_VIDEO_IDS_SQL, &[legacy_space_or_organization_id])
            .await
    }
}

fn valid_boundary_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_legacy_nanoid(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_set_is_bounded_and_covers_each_authority_branch() {
        for query in [
            PRINCIPAL_SCOPE_SQL,
            FOLDER_AUTHORITY_SQL,
            FOLDER_ORGANIZATION_SQL,
            FOLDER_SPACE_SQL,
            ORGANIZATION_AUTHORITY_SQL,
            ORGANIZATION_VIDEO_IDS_SQL,
            SPACE_AUTHORITY_SQL,
            SPACE_VIDEO_IDS_SQL,
        ] {
            assert!(!query.is_empty());
            assert!(!query.contains("SELECT *"));
            assert!(!query.contains(';') || query.trim_end().ends_with(';'));
        }
        assert!(FOLDER_AUTHORITY_SQL.contains("space_membership.state = 'active'"));
        assert!(ORGANIZATION_AUTHORITY_SQL.contains("organization.id = ?3"));
        assert!(SPACE_AUTHORITY_SQL.contains("space.organization_id = ?3"));
    }

    #[test]
    fn source_aliases_are_strict_cap_nanoids() {
        assert!(valid_legacy_nanoid("0123456789abcde"));
        for value in ["", "0123456789abcdef", "0123456789abcil", "01234-6789abcde"] {
            assert!(!valid_legacy_nanoid(value), "{value}");
        }
    }
}
