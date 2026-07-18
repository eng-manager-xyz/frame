//! Checked D1 projection and delete journal for Cap mobile bootstrap/cap reads.

use frame_application::{
    LegacyMobileBootstrapFolderV1, LegacyMobileCapProjectionV1, LegacyMobileUploadProgressV1,
    LegacyMobileVideoSourceV1,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const ACTOR_PROFILE_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/actor_profile.sql");
const ORGANIZATIONS_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/organizations.sql");
const ROOT_FOLDERS_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/root_folders.sql");
const CAPS_COUNT_SQL: &str = include_str!("../queries/legacy_mobile_bootstrap_caps/caps_count.sql");
const CAPS_ROWS_SQL: &str = include_str!("../queries/legacy_mobile_bootstrap_caps/caps_rows.sql");
const CAP_ROW_SQL: &str = include_str!("../queries/legacy_mobile_bootstrap_caps/cap_row.sql");
const COMMENTS_SQL: &str = include_str!("../queries/legacy_mobile_bootstrap_caps/comments.sql");
const DELETE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_snapshot.sql");
const DELETE_APPLY_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_apply.sql");
const DELETE_OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_operation_insert.sql");
const DELETE_AUDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_audit_insert.sql");
const DELETE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_assert.sql");
const DELETE_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_complete.sql");
const DELETE_CLEANUP_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_bootstrap_caps/delete_cleanup_assert.sql");

type RuntimeResult<T> = std::result::Result<T, LegacyMobileBootstrapCapsRuntimeFailureV1>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMobileBootstrapCapsRuntimeFailureV1 {
    NotFound,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyMobileBootstrapActorRowV1 {
    pub mapped_user_id: String,
    pub legacy_user_id: Option<String>,
    pub display_name: Option<String>,
    pub email: String,
    pub image_key: Option<String>,
    pub active_organization_id: Option<String>,
    pub active_legacy_organization_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyMobileBootstrapOrganizationRowV1 {
    pub mapped_organization_id: String,
    pub legacy_organization_id: String,
    pub name: String,
    pub icon_key: Option<String>,
    pub effective_role: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FolderRowV1 {
    legacy_folder_id: Option<String>,
    name: String,
    color: String,
    legacy_parent_id: Option<String>,
    video_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyMobileCapRowV1 {
    pub mapped_video_id: String,
    pub legacy_video_id: String,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub owner_name: String,
    pub duration_seconds: Option<f64>,
    pub legacy_folder_id: Option<String>,
    pub legacy_public: i64,
    pub protected: i64,
    pub view_count: f64,
    pub comment_count: i64,
    pub reaction_count: i64,
    pub upload_uploaded: Option<f64>,
    pub upload_total: Option<f64>,
    pub upload_phase: Option<String>,
    pub processing_progress: Option<f64>,
    pub processing_message: Option<String>,
    pub processing_error: Option<String>,
    pub metadata_json: Option<String>,
    pub transcription_status: Option<String>,
    pub object_prefix: Option<String>,
    pub source_type: Option<String>,
    pub raw_file_key: Option<String>,
    pub is_screenshot: i64,
}

impl LegacyMobileCapRowV1 {
    pub(crate) fn projection(&self) -> RuntimeResult<LegacyMobileCapProjectionV1> {
        let public = flag(self.legacy_public)?;
        let protected = flag(self.protected)?;
        let _ = flag(self.is_screenshot)?;
        if self.mapped_video_id.len() != 36
            || self.legacy_video_id.len() != 15
            || self.created_at_ms < 0
            || self.updated_at_ms < 0
            || self.comment_count < 0
            || self.reaction_count < 0
            || !self.view_count.is_finite()
            || self.view_count < 0.0
            || self
                .duration_seconds
                .is_some_and(|value| !value.is_finite())
        {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
        }
        let upload = match (
            self.upload_uploaded,
            self.upload_total,
            self.upload_phase.as_deref(),
            self.processing_progress,
        ) {
            (None, None, None, None) => None,
            (Some(uploaded), Some(total), Some(phase), Some(progress))
                if uploaded.is_finite()
                    && total.is_finite()
                    && progress.is_finite()
                    && matches!(
                        phase,
                        "uploading" | "processing" | "generating_thumbnail" | "complete" | "error"
                    ) =>
            {
                Some(LegacyMobileUploadProgressV1 {
                    uploaded,
                    total,
                    phase: phase.to_owned(),
                    processing_progress: progress,
                    processing_message: self.processing_message.clone(),
                    processing_error: self.processing_error.clone(),
                })
            }
            _ => return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt),
        };
        Ok(LegacyMobileCapProjectionV1 {
            legacy_video_id: self.legacy_video_id.clone(),
            title: self.title.clone(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            owner_name: self.owner_name.clone(),
            duration_seconds: self.duration_seconds,
            legacy_folder_id: self.legacy_folder_id.clone(),
            public,
            protected,
            view_count: self.view_count,
            comment_count: self.comment_count as f64,
            reaction_count: self.reaction_count as f64,
            upload,
        })
    }

    pub(crate) fn source(&self) -> RuntimeResult<LegacyMobileVideoSourceV1> {
        self.source_type
            .as_deref()
            .and_then(LegacyMobileVideoSourceV1::parse)
            .ok_or(LegacyMobileBootstrapCapsRuntimeFailureV1::NotFound)
    }

    pub(crate) fn screenshot(&self) -> RuntimeResult<bool> {
        flag(self.is_screenshot)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyMobileCapCommentRowV1 {
    pub legacy_comment_id: String,
    pub legacy_video_id: String,
    pub comment_kind: String,
    pub content: String,
    pub source_timestamp: Option<f64>,
    pub legacy_parent_comment_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub legacy_author_id: String,
    pub author_name: Option<String>,
    pub author_image: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyMobileCapsPageV1 {
    pub rows: Vec<LegacyMobileCapRowV1>,
    pub total: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyMobileDeleteContinuationV1 {
    pub operation_id: String,
    pub object_prefix: String,
}

#[derive(Debug, Deserialize)]
struct CountRowV1 {
    total: i64,
}

#[derive(Debug, Deserialize)]
struct DeleteSnapshotRowV1 {
    mapped_video_id: String,
    legacy_video_id: String,
    object_prefix: String,
}

pub(crate) struct D1LegacyMobileBootstrapCapsV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyMobileBootstrapCapsV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn actor(
        &self,
        actor_id: &str,
    ) -> RuntimeResult<Option<LegacyMobileBootstrapActorRowV1>> {
        self.one(ACTOR_PROFILE_SQL, vec![text(actor_id)]).await
    }

    pub(crate) async fn organizations(
        &self,
        actor_id: &str,
    ) -> RuntimeResult<Vec<LegacyMobileBootstrapOrganizationRowV1>> {
        let rows = self.rows(ORGANIZATIONS_SQL, vec![text(actor_id)]).await?;
        if rows
            .iter()
            .any(|row: &LegacyMobileBootstrapOrganizationRowV1| {
                row.mapped_organization_id.len() != 36
                    || row.legacy_organization_id.len() != 15
                    || !matches!(row.effective_role.as_str(), "owner" | "admin" | "member")
            })
        {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
        }
        Ok(rows)
    }

    pub(crate) async fn root_folders(
        &self,
        actor_id: &str,
        organization_id: &str,
    ) -> RuntimeResult<Vec<LegacyMobileBootstrapFolderV1>> {
        self.rows::<FolderRowV1>(
            ROOT_FOLDERS_SQL,
            vec![text(actor_id), text(organization_id)],
        )
        .await?
        .into_iter()
        .map(|row| {
            let id = row
                .legacy_folder_id
                .filter(|value| value.len() == 15)
                .ok_or(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt)?;
            if row.video_count < 0
                || !matches!(row.color.as_str(), "normal" | "blue" | "red" | "yellow")
                || row
                    .legacy_parent_id
                    .as_deref()
                    .is_some_and(|value| value.len() != 15)
            {
                return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
            }
            Ok(LegacyMobileBootstrapFolderV1 {
                id,
                name: row.name,
                color: row.color,
                parent_id: row.legacy_parent_id,
                video_count: row.video_count as f64,
            })
        })
        .collect()
    }

    pub(crate) async fn caps(
        &self,
        actor_id: &str,
        organization_id: &str,
        folder_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> RuntimeResult<LegacyMobileCapsPageV1> {
        let bindings = vec![text(actor_id), text(organization_id), optional(folder_id)];
        let count = self
            .one::<CountRowV1>(CAPS_COUNT_SQL, bindings.clone())
            .await?
            .ok_or(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt)?;
        if count.total < 0 {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
        }
        let rows = self
            .rows::<LegacyMobileCapRowV1>(
                CAPS_ROWS_SQL,
                vec![
                    text(actor_id),
                    text(organization_id),
                    optional(folder_id),
                    number(i64::from(limit)),
                    number(i64::from(offset)),
                ],
            )
            .await?;
        for row in &rows {
            row.projection()?;
        }
        Ok(LegacyMobileCapsPageV1 {
            rows,
            total: count.total,
        })
    }

    pub(crate) async fn cap(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
    ) -> RuntimeResult<LegacyMobileCapRowV1> {
        let row = self
            .one::<LegacyMobileCapRowV1>(CAP_ROW_SQL, vec![text(actor_id), text(legacy_video_id)])
            .await?
            .ok_or(LegacyMobileBootstrapCapsRuntimeFailureV1::NotFound)?;
        row.projection()?;
        Ok(row)
    }

    pub(crate) async fn comments(
        &self,
        legacy_video_id: &str,
    ) -> RuntimeResult<Vec<LegacyMobileCapCommentRowV1>> {
        let rows = self
            .rows::<LegacyMobileCapCommentRowV1>(COMMENTS_SQL, vec![text(legacy_video_id)])
            .await?;
        if rows.iter().any(|row| {
            row.legacy_comment_id.len() != 15
                || row.legacy_author_id.len() != 15
                || row.created_at_ms < 0
                || row.updated_at_ms < 0
                || !matches!(row.comment_kind.as_str(), "text" | "emoji")
                || row.source_timestamp.is_some_and(|value| !value.is_finite())
        }) {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
        }
        Ok(rows)
    }

    pub(crate) async fn begin_delete(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        now_ms: i64,
    ) -> RuntimeResult<LegacyMobileDeleteContinuationV1> {
        let snapshot = self
            .one::<DeleteSnapshotRowV1>(
                DELETE_SNAPSHOT_SQL,
                vec![text(actor_id), text(legacy_video_id)],
            )
            .await?
            .ok_or(LegacyMobileBootstrapCapsRuntimeFailureV1::NotFound)?;
        if snapshot.legacy_video_id != legacy_video_id
            || !valid_prefix(&snapshot.object_prefix)
            || !(0..=9_007_199_254_740_991).contains(&now_ms)
        {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
        }
        let operation_id = uuid::Uuid::now_v7().to_string();
        let audit_id = uuid::Uuid::now_v7().to_string();
        let statements = vec![
            self.statement(
                DELETE_APPLY_SQL,
                vec![
                    text(actor_id),
                    text(legacy_video_id),
                    text(&snapshot.mapped_video_id),
                    number(now_ms),
                ],
            )?,
            self.statement(
                DELETE_OPERATION_INSERT_SQL,
                vec![
                    text(&operation_id),
                    text(actor_id),
                    text(&snapshot.mapped_video_id),
                    text(legacy_video_id),
                    text(&snapshot.object_prefix),
                    number(now_ms),
                ],
            )?,
            self.statement(
                DELETE_AUDIT_INSERT_SQL,
                vec![
                    text(&audit_id),
                    text(&operation_id),
                    text(&digest(actor_id)),
                    text(&digest(legacy_video_id)),
                    number(now_ms),
                ],
            )?,
            self.statement(
                DELETE_ASSERT_SQL,
                vec![
                    text(&operation_id),
                    text("authority"),
                    text(&snapshot.mapped_video_id),
                    text(actor_id),
                    text(legacy_video_id),
                    number(now_ms),
                ],
            )?,
            self.statement(
                DELETE_ASSERT_SQL,
                vec![
                    text(&operation_id),
                    text("tombstone"),
                    text(&snapshot.mapped_video_id),
                    text(actor_id),
                    text(legacy_video_id),
                    number(now_ms),
                ],
            )?,
        ];
        self.batch(statements).await?;
        Ok(LegacyMobileDeleteContinuationV1 {
            operation_id,
            object_prefix: snapshot.object_prefix,
        })
    }

    pub(crate) async fn complete_delete(
        &self,
        operation_id: &str,
        now_ms: i64,
    ) -> RuntimeResult<()> {
        self.batch(vec![
            self.statement(
                DELETE_COMPLETE_SQL,
                vec![text(operation_id), number(now_ms)],
            )?,
            self.statement(
                DELETE_CLEANUP_ASSERT_SQL,
                vec![text(operation_id), number(now_ms)],
            )?,
        ])
        .await
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> RuntimeResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyMobileBootstrapCapsRuntimeFailureV1::Unavailable)
    }

    async fn rows<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> RuntimeResult<Vec<T>> {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyMobileBootstrapCapsRuntimeFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt)
    }

    async fn one<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> RuntimeResult<Option<T>> {
        let mut rows = self.rows(sql, bindings).await?;
        if rows.len() > 1 {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> RuntimeResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyMobileBootstrapCapsRuntimeFailureV1::Unavailable)?;
        if results.len() != expected {
            return Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1(failed.error().as_deref().unwrap_or_default()));
        }
        Ok(())
    }
}

fn flag(value: i64) -> RuntimeResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt),
    }
}

pub(crate) fn valid_prefix(value: &str) -> bool {
    (3..=512).contains(&value.len())
        && value.ends_with('/')
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.split('/').any(|part| matches!(part, "." | ".."))
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn map_d1(message: &str) -> LegacyMobileBootstrapCapsRuntimeFailureV1 {
    if message.contains("frame_legacy_mobile_cap_delete_assertion_v1") {
        LegacyMobileBootstrapCapsRuntimeFailureV1::NotFound
    } else if message.contains("frame_legacy_mobile_cap_delete_") || message.contains("foreign key")
    {
        LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt
    } else {
        LegacyMobileBootstrapCapsRuntimeFailureV1::Unavailable
    }
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn optional(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_queries_bind_owner_tombstone_and_provider_continuation() {
        assert!(CAP_ROW_SQL.contains("video.owner_id = ?1"));
        assert!(CAPS_ROWS_SQL.contains("video.organization_id = ?2"));
        assert!(DELETE_APPLY_SQL.contains("state = 'deleted'"));
        assert!(DELETE_OPERATION_INSERT_SQL.contains("'storage_pending'"));
        assert!(DELETE_CLEANUP_ASSERT_SQL.contains("state = 'complete'"));
    }

    #[test]
    fn private_prefix_validation_rejects_traversal_and_absolute_keys() {
        assert!(valid_prefix("owner/video/"));
        assert!(!valid_prefix("/owner/video/"));
        assert!(!valid_prefix("owner/../video/"));
        assert!(!valid_prefix("owner\\video/"));
    }
}
