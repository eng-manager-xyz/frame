//! Tenant-scoped D1 adapter for Cap's user-video and dashboard-search reads.

use async_trait::async_trait;
use frame_application::{
    LegacyDashboardVideoSearchProjectionV1, LegacyLibraryDetailPortErrorV1,
    LegacyLibraryDetailPrincipalV1, LegacyLibraryDetailReadPortV1, LegacyUserVideoProjectionV1,
    NormalizedDashboardVideoQueryV1,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;
use worker::{D1Database, send::IntoSendFuture};

const PRINCIPAL_SCOPE_SQL: &str =
    include_str!("../queries/legacy_library_detail_reads/principal_scope.sql");
const SCOPE_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_library_detail_reads/scope_authority.sql");
const GET_USER_VIDEOS_ORGANIZATION_SQL: &str =
    include_str!("../queries/legacy_library_detail_reads/get_user_videos_organization.sql");
const GET_USER_VIDEOS_SPACE_SQL: &str =
    include_str!("../queries/legacy_library_detail_reads/get_user_videos_space.sql");
const SEARCH_DASHBOARD_VIDEOS_SQL: &str =
    include_str!("../queries/legacy_library_detail_reads/search_dashboard_videos.sql");

#[derive(Debug, Deserialize)]
struct PrincipalRow {
    actor_id: String,
    active_organization_id: String,
    active_legacy_organization_id: String,
}

#[derive(Debug, Deserialize)]
struct AuthorityRow {
    scope_kind: String,
}

#[derive(Debug, Deserialize)]
struct UserVideoRow {
    legacy_video_id: Option<String>,
    legacy_owner_id: Option<String>,
    video_name: String,
    created_at_ms: i64,
    metadata_json: Option<String>,
    is_screenshot: i64,
    total_comments: i64,
    total_reactions: i64,
    owner_name: String,
    folder_name: Option<String>,
    folder_color: Option<String>,
    has_active_upload: i64,
    effective_created_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SearchVideoRow {
    legacy_video_id: Option<String>,
    video_name: String,
    owner_name: Option<String>,
    created_at_ms: i64,
    duration_seconds: Option<f64>,
    is_screenshot: i64,
    effective_created_at_ms: i64,
}

pub(crate) struct D1LegacyLibraryDetailReadPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyLibraryDetailReadPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn principal_for_actor(
        &self,
        actor_id: &str,
    ) -> Result<LegacyLibraryDetailPrincipalV1, LegacyLibraryDetailPortErrorV1> {
        if !valid_boundary_id(actor_id) {
            return Err(LegacyLibraryDetailPortErrorV1::NotVisible);
        }
        let mut rows = self
            .query::<PrincipalRow>(PRINCIPAL_SCOPE_SQL, &[actor_id])
            .await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                LegacyLibraryDetailPortErrorV1::NotVisible
            } else {
                LegacyLibraryDetailPortErrorV1::Corrupt
            });
        }
        let row = rows.pop().expect("one principal row");
        let principal = LegacyLibraryDetailPrincipalV1 {
            actor_id: row.actor_id,
            active_organization_id: row.active_organization_id,
            active_legacy_organization_id: row.active_legacy_organization_id,
        };
        principal
            .valid()
            .then_some(principal)
            .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)
    }

    async fn reassert_principal(
        &self,
        expected: &LegacyLibraryDetailPrincipalV1,
    ) -> Result<(), LegacyLibraryDetailPortErrorV1> {
        let actual = self.principal_for_actor(&expected.actor_id).await?;
        (actual == *expected)
            .then_some(())
            .ok_or(LegacyLibraryDetailPortErrorV1::NotVisible)
    }

    async fn require_scope(
        &self,
        principal: &LegacyLibraryDetailPrincipalV1,
        legacy_scope_id: &str,
    ) -> Result<(), LegacyLibraryDetailPortErrorV1> {
        let rows = self
            .query::<AuthorityRow>(
                SCOPE_AUTHORITY_SQL,
                &[
                    &principal.actor_id,
                    &principal.active_organization_id,
                    &principal.active_legacy_organization_id,
                    legacy_scope_id,
                ],
            )
            .await?;
        if rows.len() != 1 {
            return Err(if rows.is_empty() {
                LegacyLibraryDetailPortErrorV1::NotVisible
            } else {
                LegacyLibraryDetailPortErrorV1::Corrupt
            });
        }
        let expected = if legacy_scope_id == principal.active_legacy_organization_id {
            "organization"
        } else {
            "space"
        };
        (rows[0].scope_kind == expected)
            .then_some(())
            .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)
    }

    async fn user_videos(
        &self,
        sql: &str,
        binds: &[&str],
    ) -> Result<Vec<LegacyUserVideoProjectionV1>, LegacyLibraryDetailPortErrorV1> {
        let rows = self.query::<UserVideoRow>(sql, binds).await?;
        let mut previous_effective = None;
        rows.into_iter()
            .map(|row| {
                if previous_effective.is_some_and(|previous| previous < row.effective_created_at_ms)
                {
                    return Err(LegacyLibraryDetailPortErrorV1::Corrupt);
                }
                previous_effective = Some(row.effective_created_at_ms);
                let metadata = row
                    .metadata_json
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(|_| LegacyLibraryDetailPortErrorV1::Corrupt)?;
                let projection = LegacyUserVideoProjectionV1 {
                    id: row
                        .legacy_video_id
                        .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)?,
                    owner_id: row
                        .legacy_owner_id
                        .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)?,
                    name: row.video_name,
                    created_at_ms: row.created_at_ms,
                    metadata,
                    is_screenshot: decode_bool(row.is_screenshot)?,
                    total_comments: u64::try_from(row.total_comments)
                        .map_err(|_| LegacyLibraryDetailPortErrorV1::Corrupt)?,
                    total_reactions: u64::try_from(row.total_reactions)
                        .map_err(|_| LegacyLibraryDetailPortErrorV1::Corrupt)?,
                    owner_name: row.owner_name,
                    folder_name: row.folder_name,
                    folder_color: row.folder_color,
                    has_active_upload: decode_bool(row.has_active_upload)?,
                    effective_created_at_ms: row.effective_created_at_ms,
                };
                projection
                    .valid()
                    .then_some(projection)
                    .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)
            })
            .collect()
    }

    async fn search_videos(
        &self,
        principal: &LegacyLibraryDetailPrincipalV1,
        query: &NormalizedDashboardVideoQueryV1,
    ) -> Result<Vec<LegacyDashboardVideoSearchProjectionV1>, LegacyLibraryDetailPortErrorV1> {
        let rows = self
            .query::<SearchVideoRow>(
                SEARCH_DASHBOARD_VIDEOS_SQL,
                &[
                    &principal.actor_id,
                    &principal.active_organization_id,
                    &query.contains_pattern,
                    &query.starts_with_pattern,
                ],
            )
            .await?;
        rows.into_iter()
            .map(|row| {
                let projection = LegacyDashboardVideoSearchProjectionV1 {
                    id: row
                        .legacy_video_id
                        .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)?,
                    name: row.video_name,
                    owner_name: row.owner_name,
                    created_at_ms: row.created_at_ms,
                    duration_seconds: row.duration_seconds,
                    is_screenshot: decode_bool(row.is_screenshot)?,
                    effective_created_at_ms: row.effective_created_at_ms,
                };
                projection
                    .valid()
                    .then_some(projection)
                    .ok_or(LegacyLibraryDetailPortErrorV1::Corrupt)
            })
            .collect()
    }

    async fn query<T: DeserializeOwned>(
        &self,
        sql: &str,
        binds: &[&str],
    ) -> Result<Vec<T>, LegacyLibraryDetailPortErrorV1> {
        let values = binds
            .iter()
            .map(|value| JsValue::from_str(value))
            .collect::<Vec<_>>();
        let result = self
            .database
            .prepare(sql)
            .bind(&values)
            .map_err(|_| LegacyLibraryDetailPortErrorV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyLibraryDetailPortErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyLibraryDetailPortErrorV1::Unavailable);
        }
        result
            .results::<T>()
            .map_err(|_| LegacyLibraryDetailPortErrorV1::Corrupt)
    }
}

#[async_trait]
impl LegacyLibraryDetailReadPortV1 for D1LegacyLibraryDetailReadPortV1<'_> {
    async fn get_user_videos(
        &self,
        principal: &LegacyLibraryDetailPrincipalV1,
        legacy_scope_id: &str,
    ) -> Result<Vec<LegacyUserVideoProjectionV1>, LegacyLibraryDetailPortErrorV1> {
        self.reassert_principal(principal).await?;
        self.require_scope(principal, legacy_scope_id).await?;
        if legacy_scope_id == principal.active_legacy_organization_id {
            self.user_videos(
                GET_USER_VIDEOS_ORGANIZATION_SQL,
                &[&principal.actor_id, &principal.active_organization_id],
            )
            .await
        } else {
            self.user_videos(
                GET_USER_VIDEOS_SPACE_SQL,
                &[
                    &principal.actor_id,
                    legacy_scope_id,
                    &principal.active_organization_id,
                ],
            )
            .await
        }
    }

    async fn search_dashboard_videos(
        &self,
        principal: &LegacyLibraryDetailPrincipalV1,
        query: &NormalizedDashboardVideoQueryV1,
    ) -> Result<Vec<LegacyDashboardVideoSearchProjectionV1>, LegacyLibraryDetailPortErrorV1> {
        self.reassert_principal(principal).await?;
        self.search_videos(principal, query).await
    }
}

fn decode_bool(value: i64) -> Result<bool, LegacyLibraryDetailPortErrorV1> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(LegacyLibraryDetailPortErrorV1::Corrupt),
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
    fn query_set_is_bounded_source_ordered_and_tenant_scoped() {
        for query in [
            PRINCIPAL_SCOPE_SQL,
            SCOPE_AUTHORITY_SQL,
            GET_USER_VIDEOS_ORGANIZATION_SQL,
            GET_USER_VIDEOS_SPACE_SQL,
            SEARCH_DASHBOARD_VIDEOS_SQL,
        ] {
            assert!(!query.is_empty());
            assert!(!query.contains("SELECT *"));
            assert!(query.trim_end().ends_with(';'));
        }
        assert!(SCOPE_AUTHORITY_SQL.contains("space.organization_id = ?2"));
        assert!(
            GET_USER_VIDEOS_ORGANIZATION_SQL
                .contains("ORDER BY video.legacy_effective_created_at_us DESC")
        );
        assert!(GET_USER_VIDEOS_SPACE_SQL.contains("video.organization_id"));
        assert!(SEARCH_DASHBOARD_VIDEOS_SQL.contains("ESCAPE '!'"));
        assert!(SEARCH_DASHBOARD_VIDEOS_SQL.contains("LIMIT 8"));
    }

    #[test]
    fn boundary_boolean_decoder_fails_closed() {
        assert_eq!(decode_bool(0), Ok(false));
        assert_eq!(decode_bool(1), Ok(true));
        assert_eq!(decode_bool(2), Err(LegacyLibraryDetailPortErrorV1::Corrupt));
    }
}
