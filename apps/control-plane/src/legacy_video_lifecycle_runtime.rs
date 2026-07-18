//! D1 and R2 authority for Cap's provider-free video lifecycle RPCs and routes.

use frame_application::{
    LEGACY_VIDEO_LIFECYCLE_MAX_R2_PAGES, LegacyVideoLifecycleSurfaceV1,
    legacy_video_lifecycle_copy_key, legacy_video_lifecycle_object_prefix,
    legacy_video_lifecycle_valid_cap_id,
};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{
    Bucket, D1Database, D1PreparedStatement, D1Result, ResponseBody, send::IntoSendFuture,
};

const VIDEO_OWNER_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/video_owner_snapshot.sql");
const OG_SNAPSHOT_SQL: &str = include_str!("../queries/legacy_video_lifecycle/og_snapshot.sql");
const ORGANIZATION_ADMIN_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/organization_admin_snapshot.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/operation_by_key.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/operation_insert.sql");
const OPERATION_STORAGE_PENDING_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/operation_storage_pending.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/operation_complete.sql");
const DELETE_TOMBSTONE_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/delete_tombstone.sql");
const DELETE_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/delete_postcondition_assert.sql");
const DUPLICATE_VIDEO_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/duplicate_video_insert.sql");
const DUPLICATE_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/duplicate_alias_insert.sql");
const DUPLICATE_MEDIA_UPDATE_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/duplicate_media_update.sql");
const COPY_RECEIPT_EXISTS_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/copy_receipt_exists.sql");
const COPY_RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/copy_receipt_insert.sql");
const ORGANIZATION_ICON_UPDATE_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/organization_icon_update.sql");

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyVideoLifecycleFailureV1 {
    Invalid,
    NotFound,
    Forbidden,
    Conflict,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyVideoLifecycleVideoV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) organization_id: String,
    pub(crate) object_prefix: String,
    pub(crate) source_type: String,
    pub(crate) transcription_status: Option<String>,
    pub(crate) legacy_owner_id: String,
    title: String,
    state: String,
    duration_ms: Option<i64>,
    folder_id: Option<String>,
    privacy: String,
    metadata_json: Option<String>,
    legacy_public: i64,
    legacy_password_hash: Option<String>,
    legacy_settings_json: Option<String>,
    legacy_metadata_json: Option<String>,
    legacy_is_screenshot: i64,
    legacy_duration_seconds: Option<f64>,
    legacy_storage_width: Option<f64>,
    legacy_storage_height: Option<f64>,
    legacy_storage_fps: Option<f64>,
}

impl LegacyVideoLifecycleVideoV1 {
    fn valid(&self) -> bool {
        valid_uuid(&self.mapped_video_id)
            && legacy_video_lifecycle_valid_cap_id(&self.legacy_video_id)
            && valid_id(&self.owner_id)
            && valid_id(&self.organization_id)
            && legacy_video_lifecycle_valid_cap_id(&self.legacy_owner_id)
            && legacy_video_lifecycle_object_prefix(&self.legacy_owner_id, &self.legacy_video_id)
                .is_some()
            && valid_prefix(&self.object_prefix)
            && matches!(
                self.source_type.as_str(),
                "MediaConvert" | "local" | "desktopMP4" | "desktopSegments" | "webMP4"
            )
            && self.transcription_status.as_deref().is_none_or(|value| {
                matches!(
                    value,
                    "PROCESSING" | "COMPLETE" | "ERROR" | "SKIPPED" | "NO_AUDIO"
                )
            })
            && !self.title.is_empty()
            && matches!(
                self.state.as_str(),
                "pending" | "uploading" | "processing" | "ready" | "failed"
            )
            && self.duration_ms.is_none_or(valid_safe_integer)
            && self.folder_id.as_deref().is_none_or(valid_id)
            && matches!(
                self.privacy.as_str(),
                "private" | "organization" | "public" | "unlisted"
            )
            && self.metadata_json.as_deref().is_none_or(valid_json)
            && matches!(self.legacy_public, 0 | 1)
            && self
                .legacy_password_hash
                .as_deref()
                .is_none_or(|value| value.len() == 64)
            && self.legacy_settings_json.as_deref().is_none_or(valid_json)
            && self.legacy_metadata_json.as_deref().is_none_or(valid_json)
            && matches!(self.legacy_is_screenshot, 0 | 1)
            && [
                self.legacy_duration_seconds,
                self.legacy_storage_width,
                self.legacy_storage_height,
                self.legacy_storage_fps,
            ]
            .into_iter()
            .all(|value| value.is_none_or(f64::is_finite))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyVideoLifecycleOgV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) object_prefix: String,
    pub(crate) legacy_public: i64,
    pub(crate) privacy: String,
}

impl LegacyVideoLifecycleOgV1 {
    fn valid(&self) -> bool {
        valid_uuid(&self.mapped_video_id)
            && legacy_video_lifecycle_valid_cap_id(&self.legacy_video_id)
            && valid_id(&self.owner_id)
            && valid_prefix(&self.object_prefix)
            && matches!(self.legacy_public, 0 | 1)
            && matches!(
                self.privacy.as_str(),
                "private" | "organization" | "public" | "unlisted"
            )
    }

    #[must_use]
    pub(crate) fn public(&self) -> bool {
        self.legacy_public == 1
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyVideoLifecycleOrganizationV1 {
    pub(crate) organization_id: String,
    pub(crate) existing_icon_key: Option<String>,
}

impl LegacyVideoLifecycleOrganizationV1 {
    fn valid(&self) -> bool {
        valid_id(&self.organization_id)
            && self.existing_icon_key.as_deref().is_none_or(|value| {
                !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
            })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyVideoLifecycleOperationV1 {
    pub(crate) operation_id: String,
    pub(crate) source_operation_id: String,
    pub(crate) action: String,
    pub(crate) actor_id: String,
    pub(crate) organization_id: String,
    pub(crate) mapped_video_id: Option<String>,
    pub(crate) legacy_video_id: Option<String>,
    pub(crate) request_key_digest: String,
    pub(crate) request_digest: String,
    pub(crate) destination_mapped_video_id: Option<String>,
    pub(crate) destination_legacy_video_id: Option<String>,
    pub(crate) source_prefix: Option<String>,
    pub(crate) destination_prefix: Option<String>,
    pub(crate) result_json: Option<String>,
    pub(crate) state: String,
    pub(crate) failure_code: Option<String>,
    pub(crate) created_at_ms: i64,
    pub(crate) completed_at_ms: Option<i64>,
}

impl LegacyVideoLifecycleOperationV1 {
    fn valid(&self) -> bool {
        valid_uuid(&self.operation_id)
            && LegacyVideoLifecycleSurfaceV1::parse(&self.source_operation_id).is_some()
            && matches!(
                self.action.as_str(),
                "delete_route"
                    | "organisation_update"
                    | "video_delete"
                    | "video_duplicate"
                    | "video_instant_create"
            )
            && valid_id(&self.actor_id)
            && valid_id(&self.organization_id)
            && self.mapped_video_id.as_deref().is_none_or(valid_uuid)
            && self
                .legacy_video_id
                .as_deref()
                .is_none_or(legacy_video_lifecycle_valid_cap_id)
            && valid_digest(&self.request_key_digest)
            && valid_digest(&self.request_digest)
            && self
                .destination_mapped_video_id
                .as_deref()
                .is_none_or(valid_uuid)
            && self
                .destination_legacy_video_id
                .as_deref()
                .is_none_or(legacy_video_lifecycle_valid_cap_id)
            && self.source_prefix.as_deref().is_none_or(valid_prefix)
            && self.destination_prefix.as_deref().is_none_or(valid_prefix)
            && self.result_json.as_deref().is_none_or(valid_json)
            && matches!(
                self.state.as_str(),
                "claimed" | "storage_pending" | "complete" | "failed"
            )
            && self
                .failure_code
                .as_deref()
                .is_none_or(|value| !value.is_empty() && value.len() <= 64)
            && valid_safe_integer(self.created_at_ms)
            && self.completed_at_ms.is_none_or(valid_safe_integer)
    }
}

pub(crate) struct NewLegacyVideoLifecycleOperationV1<'a> {
    pub(crate) operation_id: &'a str,
    pub(crate) surface: LegacyVideoLifecycleSurfaceV1,
    pub(crate) action: &'a str,
    pub(crate) actor_id: &'a str,
    pub(crate) organization_id: &'a str,
    pub(crate) mapped_video_id: Option<&'a str>,
    pub(crate) legacy_video_id: Option<&'a str>,
    pub(crate) request_key_digest: &'a str,
    pub(crate) request_digest: &'a str,
    pub(crate) destination_mapped_video_id: Option<&'a str>,
    pub(crate) destination_legacy_video_id: Option<&'a str>,
    pub(crate) source_prefix: Option<&'a str>,
    pub(crate) destination_prefix: Option<&'a str>,
    pub(crate) result_json: Option<&'a str>,
    pub(crate) state: &'a str,
    pub(crate) now_ms: i64,
}

pub(crate) struct D1LegacyVideoLifecycleV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyVideoLifecycleV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn video_for_owner(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
    ) -> Result<LegacyVideoLifecycleVideoV1, LegacyVideoLifecycleFailureV1> {
        if !valid_id(actor_id) || !legacy_video_lifecycle_valid_cap_id(legacy_video_id) {
            return Err(LegacyVideoLifecycleFailureV1::Invalid);
        }
        let mut rows = self
            .rows::<LegacyVideoLifecycleVideoV1>(
                VIDEO_OWNER_SNAPSHOT_SQL,
                &[js(actor_id), js(legacy_video_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        let row = rows.pop().ok_or(LegacyVideoLifecycleFailureV1::NotFound)?;
        if !row.valid() {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        Ok(row)
    }

    pub(crate) async fn og(
        &self,
        legacy_video_id: &str,
    ) -> Result<Option<LegacyVideoLifecycleOgV1>, LegacyVideoLifecycleFailureV1> {
        if !legacy_video_lifecycle_valid_cap_id(legacy_video_id) {
            return Ok(None);
        }
        let mut rows = self
            .rows::<LegacyVideoLifecycleOgV1>(OG_SNAPSHOT_SQL, &[js(legacy_video_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        let row = rows.pop();
        if row.as_ref().is_some_and(|row| !row.valid()) {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        Ok(row)
    }

    pub(crate) async fn organization_for_admin(
        &self,
        actor_id: &str,
        requested_id: &str,
    ) -> Result<LegacyVideoLifecycleOrganizationV1, LegacyVideoLifecycleFailureV1> {
        if !valid_id(actor_id) || !valid_id(requested_id) {
            return Err(LegacyVideoLifecycleFailureV1::Invalid);
        }
        let mut rows = self
            .rows::<LegacyVideoLifecycleOrganizationV1>(
                ORGANIZATION_ADMIN_SNAPSHOT_SQL,
                &[js(actor_id), js(requested_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        let row = rows.pop().ok_or(LegacyVideoLifecycleFailureV1::NotFound)?;
        if !row.valid() {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        Ok(row)
    }

    pub(crate) async fn operation(
        &self,
        surface: LegacyVideoLifecycleSurfaceV1,
        actor_id: &str,
        request_key_digest: &str,
    ) -> Result<Option<LegacyVideoLifecycleOperationV1>, LegacyVideoLifecycleFailureV1> {
        if !valid_id(actor_id) || !valid_digest(request_key_digest) {
            return Err(LegacyVideoLifecycleFailureV1::Invalid);
        }
        let mut rows = self
            .rows::<LegacyVideoLifecycleOperationV1>(
                OPERATION_BY_KEY_SQL,
                &[
                    js(surface.operation_id()),
                    js(actor_id),
                    js(request_key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        let row = rows.pop();
        if row.as_ref().is_some_and(|row| !row.valid()) {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        Ok(row)
    }

    pub(crate) async fn begin_delete(
        &self,
        operation: &NewLegacyVideoLifecycleOperationV1<'_>,
        video: &LegacyVideoLifecycleVideoV1,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        self.validate_new(operation)?;
        if operation.mapped_video_id != Some(video.mapped_video_id.as_str())
            || operation.legacy_video_id != Some(video.legacy_video_id.as_str())
            || operation.source_prefix != Some(video.object_prefix.as_str())
            || operation.organization_id != video.organization_id
            || operation.actor_id != video.owner_id
            || operation.state != "claimed"
        {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        let statements = vec![
            self.operation_insert_statement(operation)?,
            self.statement(
                DELETE_TOMBSTONE_SQL,
                &[
                    js(&video.mapped_video_id),
                    js(&video.owner_id),
                    number(operation.now_ms),
                    js(operation.operation_id),
                ],
            )?,
            self.statement(
                DELETE_POSTCONDITION_ASSERT_SQL,
                &[
                    js(operation.operation_id),
                    js(&video.mapped_video_id),
                    js(&video.owner_id),
                ],
            )?,
            self.statement(OPERATION_STORAGE_PENDING_SQL, &[js(operation.operation_id)])?,
        ];
        self.batch(statements).await
    }

    pub(crate) async fn begin_duplicate(
        &self,
        operation: &NewLegacyVideoLifecycleOperationV1<'_>,
        video: &LegacyVideoLifecycleVideoV1,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        self.validate_new(operation)?;
        let destination_video = operation
            .destination_mapped_video_id
            .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
        let destination_alias = operation
            .destination_legacy_video_id
            .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
        let destination_prefix = operation
            .destination_prefix
            .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
        if operation.mapped_video_id != Some(video.mapped_video_id.as_str())
            || operation.legacy_video_id != Some(video.legacy_video_id.as_str())
            || operation.source_prefix != Some(video.object_prefix.as_str())
            || operation.organization_id != video.organization_id
            || operation.actor_id != video.owner_id
            || operation.state != "claimed"
            || legacy_video_lifecycle_object_prefix(&video.legacy_owner_id, destination_alias)
                .as_deref()
                != Some(destination_prefix)
        {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        let statements = vec![
            self.operation_insert_statement(operation)?,
            self.statement(
                DUPLICATE_VIDEO_INSERT_SQL,
                &[
                    js(&video.mapped_video_id),
                    js(destination_video),
                    number(operation.now_ms),
                    js(operation.operation_id),
                    js(&video.owner_id),
                ],
            )?,
            self.statement(
                DUPLICATE_ALIAS_INSERT_SQL,
                &[
                    js(destination_alias),
                    js(destination_video),
                    number(operation.now_ms),
                ],
            )?,
            self.statement(
                DUPLICATE_MEDIA_UPDATE_SQL,
                &[
                    js(destination_video),
                    js(destination_prefix),
                    js(&video.source_type),
                    js_opt(video.transcription_status.as_deref()),
                    number(operation.now_ms),
                ],
            )?,
            self.statement(OPERATION_STORAGE_PENDING_SQL, &[js(operation.operation_id)])?,
        ];
        self.batch(statements).await
    }

    pub(crate) async fn insert_operation(
        &self,
        operation: &NewLegacyVideoLifecycleOperationV1<'_>,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        self.validate_new(operation)?;
        self.batch(vec![self.operation_insert_statement(operation)?])
            .await
    }

    pub(crate) async fn update_icon_and_mark_pending(
        &self,
        operation_id: &str,
        actor_id: &str,
        organization_id: &str,
        icon_key: Option<&str>,
        now_ms: i64,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        if !valid_uuid(operation_id)
            || !valid_id(actor_id)
            || !valid_id(organization_id)
            || icon_key.is_some_and(|value| !valid_object_key(value))
            || !valid_safe_integer(now_ms)
        {
            return Err(LegacyVideoLifecycleFailureV1::Invalid);
        }
        self.batch(vec![
            self.statement(
                ORGANIZATION_ICON_UPDATE_SQL,
                &[
                    js(organization_id),
                    js(actor_id),
                    js_opt(icon_key),
                    js(operation_id),
                    number(now_ms),
                ],
            )?,
            self.statement(OPERATION_STORAGE_PENDING_SQL, &[js(operation_id)])?,
        ])
        .await
    }

    pub(crate) async fn complete(
        &self,
        operation_id: &str,
        result_json: &str,
        now_ms: i64,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        if !valid_uuid(operation_id) || !valid_json(result_json) || !valid_safe_integer(now_ms) {
            return Err(LegacyVideoLifecycleFailureV1::Invalid);
        }
        self.batch(vec![self.statement(
            OPERATION_COMPLETE_SQL,
            &[js(operation_id), js(result_json), number(now_ms)],
        )?])
        .await
    }

    async fn copy_receipt_exists(
        &self,
        operation_id: &str,
        source_key: &str,
    ) -> Result<bool, LegacyVideoLifecycleFailureV1> {
        #[derive(Deserialize)]
        struct CountRow {
            copied: i64,
        }
        let mut rows = self
            .rows::<CountRow>(COPY_RECEIPT_EXISTS_SQL, &[js(operation_id), js(source_key)])
            .await?;
        if rows.len() != 1 || !matches!(rows[0].copied, 0 | 1) {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        Ok(rows.pop().is_some_and(|row| row.copied == 1))
    }

    async fn insert_copy_receipt(
        &self,
        operation_id: &str,
        source_key: &str,
        destination_key: &str,
        source_version: &str,
        source_bytes: u64,
        now_ms: i64,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        let source_bytes =
            i64::try_from(source_bytes).map_err(|_| LegacyVideoLifecycleFailureV1::Corrupt)?;
        self.batch(vec![self.statement(
            COPY_RECEIPT_INSERT_SQL,
            &[
                js(operation_id),
                js(source_key),
                js(destination_key),
                js(source_version),
                number(source_bytes),
                number(now_ms),
            ],
        )?])
        .await
    }

    fn validate_new(
        &self,
        operation: &NewLegacyVideoLifecycleOperationV1<'_>,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        if !valid_uuid(operation.operation_id)
            || !valid_id(operation.actor_id)
            || !valid_id(operation.organization_id)
            || operation
                .mapped_video_id
                .is_some_and(|value| !valid_uuid(value))
            || operation
                .legacy_video_id
                .is_some_and(|value| !legacy_video_lifecycle_valid_cap_id(value))
            || !valid_digest(operation.request_key_digest)
            || !valid_digest(operation.request_digest)
            || operation
                .destination_mapped_video_id
                .is_some_and(|value| !valid_uuid(value))
            || operation
                .destination_legacy_video_id
                .is_some_and(|value| !legacy_video_lifecycle_valid_cap_id(value))
            || operation
                .source_prefix
                .is_some_and(|value| !valid_prefix(value))
            || operation
                .destination_prefix
                .is_some_and(|value| !valid_prefix(value))
            || operation
                .result_json
                .is_some_and(|value| !valid_json(value))
            || !matches!(operation.state, "claimed" | "storage_pending" | "complete")
            || !valid_safe_integer(operation.now_ms)
        {
            return Err(LegacyVideoLifecycleFailureV1::Invalid);
        }
        Ok(())
    }

    fn operation_insert_statement(
        &self,
        operation: &NewLegacyVideoLifecycleOperationV1<'_>,
    ) -> Result<D1PreparedStatement, LegacyVideoLifecycleFailureV1> {
        self.statement(
            OPERATION_INSERT_SQL,
            &[
                js(operation.operation_id),
                js(operation.surface.operation_id()),
                js(operation.action),
                js(operation.actor_id),
                js(operation.organization_id),
                js_opt(operation.mapped_video_id),
                js_opt(operation.legacy_video_id),
                js(operation.request_key_digest),
                js(operation.request_digest),
                js_opt(operation.destination_mapped_video_id),
                js_opt(operation.destination_legacy_video_id),
                js_opt(operation.source_prefix),
                js_opt(operation.destination_prefix),
                js_opt(operation.result_json),
                js(operation.state),
                number(operation.now_ms),
            ],
        )
    }

    async fn rows<T: for<'de> Deserialize<'de>>(
        &self,
        sql: &str,
        values: &[JsValue],
    ) -> Result<Vec<T>, LegacyVideoLifecycleFailureV1> {
        let statement = self.statement(sql, values)?;
        let result = statement
            .all()
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        result
            .results::<T>()
            .map_err(|_| LegacyVideoLifecycleFailureV1::Corrupt)
    }

    fn statement(
        &self,
        sql: &str,
        values: &[JsValue],
    ) -> Result<D1PreparedStatement, LegacyVideoLifecycleFailureV1> {
        self.database
            .prepare(sql)
            .bind(values)
            .map_err(|error| map_d1_message(&error.to_string()))
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyVideoLifecycleFailureV1> {
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.iter().any(|result: &D1Result| !result.success()) {
            return Err(LegacyVideoLifecycleFailureV1::Unavailable);
        }
        Ok(())
    }
}

pub(crate) async fn delete_r2_prefix(
    bucket: &Bucket,
    prefix: &str,
) -> Result<(), LegacyVideoLifecycleFailureV1> {
    if !valid_prefix(prefix) {
        return Err(LegacyVideoLifecycleFailureV1::Corrupt);
    }
    for _ in 0..LEGACY_VIDEO_LIFECYCLE_MAX_R2_PAGES {
        let listed = bucket
            .list()
            .prefix(prefix)
            .limit(1_000)
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
        let keys = listed
            .objects()
            .into_iter()
            .map(|object| object.key())
            .collect::<Vec<_>>();
        if keys.iter().any(|key| !key.starts_with(prefix)) {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
        if !keys.is_empty() {
            bucket
                .delete_multiple(keys)
                .into_send()
                .await
                .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
        }
        if !listed.truncated() {
            return Ok(());
        }
        // Deleting the current page changes the cursor domain; restart at page
        // one just like Cap's bounded cleanup adapters.
    }
    Err(LegacyVideoLifecycleFailureV1::Unavailable)
}

pub(crate) async fn copy_r2_prefix(
    authority: &D1LegacyVideoLifecycleV1<'_>,
    bucket: &Bucket,
    operation_id: &str,
    source_prefix: &str,
    destination_prefix: &str,
    now_ms: i64,
) -> Result<(), LegacyVideoLifecycleFailureV1> {
    if !valid_uuid(operation_id)
        || !valid_prefix(source_prefix)
        || !valid_prefix(destination_prefix)
        || source_prefix == destination_prefix
        || !valid_safe_integer(now_ms)
    {
        return Err(LegacyVideoLifecycleFailureV1::Corrupt);
    }
    let mut cursor = None;
    for _ in 0..LEGACY_VIDEO_LIFECYCLE_MAX_R2_PAGES {
        let mut list = bucket.list().prefix(source_prefix).limit(1_000);
        if let Some(value) = cursor.as_deref() {
            list = list.cursor(value);
        }
        let listed = list
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
        for listed_object in listed.objects() {
            let source_key = listed_object.key();
            let destination_key =
                legacy_video_lifecycle_copy_key(source_prefix, destination_prefix, &source_key)
                    .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
            if authority
                .copy_receipt_exists(operation_id, &source_key)
                .await?
            {
                continue;
            }
            let object = bucket
                .get(&source_key)
                .execute()
                .into_send()
                .await
                .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?
                .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
            let version = object.version();
            let bytes = object.size();
            let http_metadata = object.http_metadata();
            let custom_metadata = object
                .custom_metadata()
                .map_err(|_| LegacyVideoLifecycleFailureV1::Corrupt)?;
            let body = object
                .body()
                .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?
                .response_body()
                .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
            let ResponseBody::Stream(stream) = body else {
                return Err(LegacyVideoLifecycleFailureV1::Corrupt);
            };
            bucket
                .put(&destination_key, stream)
                .http_metadata(http_metadata)
                .custom_metadata(custom_metadata)
                .execute()
                .into_send()
                .await
                .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?
                .ok_or(LegacyVideoLifecycleFailureV1::Unavailable)?;
            authority
                .insert_copy_receipt(
                    operation_id,
                    &source_key,
                    &destination_key,
                    &version,
                    bytes,
                    now_ms,
                )
                .await?;
        }
        if !listed.truncated() {
            return Ok(());
        }
        cursor = listed.cursor();
        if cursor.is_none() {
            return Err(LegacyVideoLifecycleFailureV1::Corrupt);
        }
    }
    Err(LegacyVideoLifecycleFailureV1::Unavailable)
}

fn map_d1_message(message: &str) -> LegacyVideoLifecycleFailureV1 {
    if message.contains("frame_legacy_video_lifecycle_assertion_v1") {
        LegacyVideoLifecycleFailureV1::Forbidden
    } else if message.contains("UNIQUE constraint failed") {
        LegacyVideoLifecycleFailureV1::Conflict
    } else if message.contains("frame_legacy_video_lifecycle_")
        || message.contains("foreign key")
        || message.contains("CHECK constraint")
    {
        LegacyVideoLifecycleFailureV1::Corrupt
    } else {
        LegacyVideoLifecycleFailureV1::Unavailable
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_safe_integer(value: i64) -> bool {
    (0..=MAX_SAFE_INTEGER).contains(&value)
}

fn valid_json(value: &str) -> bool {
    value.len() <= 1_048_576 && serde_json::from_str::<serde_json::Value>(value).is_ok()
}

fn valid_prefix(value: &str) -> bool {
    value.len() >= 3
        && value.len() <= 512
        && value.ends_with('/')
        && !value.starts_with('/')
        && !value.contains("\\")
        && !value.contains("..")
        && !value.contains("//")
}

fn valid_object_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2_048
        && !value.starts_with('/')
        && !value.contains("\\")
        && !value.contains("..")
        && !value.contains("//")
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn js_opt(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_query_bundle_contains_authority_mutation_replay_and_storage_receipts() {
        let bundle = [
            VIDEO_OWNER_SNAPSHOT_SQL,
            OG_SNAPSHOT_SQL,
            ORGANIZATION_ADMIN_SNAPSHOT_SQL,
            OPERATION_BY_KEY_SQL,
            OPERATION_INSERT_SQL,
            OPERATION_STORAGE_PENDING_SQL,
            OPERATION_COMPLETE_SQL,
            DELETE_TOMBSTONE_SQL,
            DELETE_POSTCONDITION_ASSERT_SQL,
            DUPLICATE_VIDEO_INSERT_SQL,
            DUPLICATE_ALIAS_INSERT_SQL,
            DUPLICATE_MEDIA_UPDATE_SQL,
            COPY_RECEIPT_EXISTS_SQL,
            COPY_RECEIPT_INSERT_SQL,
            ORGANIZATION_ICON_UPDATE_SQL,
        ]
        .join("\n");
        for token in [
            "legacy_video_lifecycle_operations_v1",
            "request_key_digest",
            "legacy_video_lifecycle_copy_receipts_v1",
            "legacy_video_lifecycle_assertions_v1",
            "deleted_at_ms",
            "legacy_icon_key",
            "legacy_collaboration_video_aliases_v1",
        ] {
            assert!(bundle.contains(token), "missing checked SQL token {token}");
        }
    }

    #[test]
    fn prefixes_and_ids_are_strict() {
        assert!(valid_prefix("owner/video/"));
        assert!(!valid_prefix("../video/"));
        assert!(valid_uuid("018f47ef-f1da-7cc5-9d84-4ebf3c07ad0e"));
        assert!(!valid_uuid("not-a-uuid"));
    }
}
