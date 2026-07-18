//! D1/R2 authority adapter for Cap's released mobile upload lifecycle.

use std::{collections::BTreeMap, fmt::Write as _};

use frame_application::{
    LEGACY_MOBILE_UPLOAD_COMPLETE_OPERATION_ID, LEGACY_MOBILE_UPLOAD_CREATE_OPERATION_ID,
    LEGACY_MOBILE_UPLOAD_PROGRESS_OPERATION_ID, LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER,
    LEGACY_MOBILE_UPLOADS_UPLOAD_TTL_SECONDS, LegacyMobileCapSummaryV1,
    LegacyMobileUploadCompleteInputV1, LegacyMobileUploadCreateInputV1,
    LegacyMobileUploadCreateResponseV1, LegacyMobileUploadProgressInputV1,
    LegacyMobileUploadProgressV1, LegacyMobileUploadPutTargetV1, legacy_mobile_iso_from_millis,
    legacy_mobile_upload_extension, legacy_mobile_upload_raw_key, legacy_mobile_upload_title,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

use crate::r2_direct_upload::R2DirectPutSigner;

const CREATE_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_authority.sql");
const CREATE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_authority_assert.sql");
const CREATE_VIDEO_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_video_insert.sql");
const CREATE_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_alias_insert.sql");
const CREATE_UPLOAD_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_upload_insert.sql");
const CREATE_RECORD_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_record_insert.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/operation_insert.sql");
const CREATE_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/create_postcondition_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/assertion_cleanup.sql");
const PROGRESS_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/progress_snapshot.sql");
const PROGRESS_UPLOAD_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/progress_upload_insert.sql");
const PROGRESS_UPDATE_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/progress_update.sql");
const PROGRESS_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/progress_authority_assert.sql");
const PROGRESS_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/progress_postcondition_assert.sql");
const COMPLETE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/complete_snapshot.sql");
const COMPLETE_UPLOAD_BYTES_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/complete_upload_bytes.sql");
const COMPLETE_RECORD_PENDING_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/complete_record_pending.sql");
const COMPLETE_INTENT_INSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/complete_intent_insert.sql");
const COMPLETE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/complete_authority_assert.sql");
const COMPLETE_PENDING_ASSERT_SQL: &str =
    include_str!("../queries/legacy_mobile_uploads/complete_pending_assert.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMobileUploadsFailureV1 {
    Invalid,
    Forbidden,
    NotFound,
    Conflict,
    ProviderGated,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateAuthorityRowV1 {
    actor_id: String,
    legacy_actor_id: String,
    owner_name: String,
    organization_id: String,
    legacy_organization_id: String,
    storage_integration_id: String,
    folder_id: Option<String>,
    legacy_folder_id: Option<String>,
}

impl CreateAuthorityRowV1 {
    fn valid(
        &self,
        actor_id: &str,
        requested_organization_id: Option<&str>,
        requested_folder_id: Option<&str>,
    ) -> bool {
        self.actor_id == actor_id
            && valid_id(&self.actor_id)
            && valid_cap_id(&self.legacy_actor_id)
            && valid_id(&self.organization_id)
            && valid_cap_id(&self.legacy_organization_id)
            && requested_organization_id
                .is_none_or(|requested| requested == self.legacy_organization_id)
            && valid_id(&self.storage_integration_id)
            && self.folder_id.as_deref().is_none_or(valid_id)
            && self.legacy_folder_id.as_deref().is_none_or(valid_cap_id)
            && match requested_folder_id {
                Some(requested) => {
                    self.folder_id.is_some() && self.legacy_folder_id.as_deref() == Some(requested)
                }
                None => self.folder_id.is_none() && self.legacy_folder_id.is_none(),
            }
            && self.owner_name.len() <= 255
            && !self.owner_name.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ProgressSnapshotRowV1 {
    mapped_video_id: String,
    organization_id: String,
    legacy_video_id: String,
    legacy_actor_id: String,
    upload_id: Option<String>,
    received_bytes: Option<i64>,
    expected_bytes: Option<i64>,
    upload_updated_at_ms: Option<i64>,
}

impl ProgressSnapshotRowV1 {
    fn valid(&self, legacy_video_id: &str) -> bool {
        let upload_absent = self.upload_id.is_none()
            && self.received_bytes.is_none()
            && self.expected_bytes.is_none()
            && self.upload_updated_at_ms.is_none();
        let upload_present = self.upload_id.as_deref().is_some_and(valid_uuid)
            && [
                self.received_bytes,
                self.expected_bytes,
                self.upload_updated_at_ms,
            ]
            .into_iter()
            .all(|value| value.is_some_and(valid_safe_i64));
        valid_uuid(&self.mapped_video_id)
            && valid_id(&self.organization_id)
            && self.legacy_video_id == legacy_video_id
            && valid_cap_id(&self.legacy_video_id)
            && valid_cap_id(&self.legacy_actor_id)
            && (upload_absent || upload_present)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyMobileUploadCompletionSnapshotV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) actor_id: String,
    pub(crate) organization_id: String,
    pub(crate) upload_id: String,
    pub(crate) raw_file_key: String,
    content_type: String,
    lifecycle_state: String,
    intent_operation_id: Option<String>,
    intent_observed_bytes: Option<i64>,
    intent_requested_content_length: Option<i64>,
    intent_state: Option<String>,
}

impl LegacyMobileUploadCompletionSnapshotV1 {
    fn valid(&self, actor_id: &str, legacy_video_id: &str) -> bool {
        let no_intent = self.intent_operation_id.is_none()
            && self.intent_observed_bytes.is_none()
            && self.intent_requested_content_length.is_none()
            && self.intent_state.is_none()
            && self.lifecycle_state == "uploading";
        let pending_intent = self.intent_operation_id.as_deref().is_some_and(valid_uuid)
            && self
                .intent_observed_bytes
                .is_some_and(valid_positive_safe_i64)
            && self
                .intent_requested_content_length
                .is_none_or(valid_safe_i64)
            && matches!(
                self.intent_state.as_deref(),
                Some("provider_pending" | "submitted" | "complete" | "failed")
            )
            && matches!(
                self.lifecycle_state.as_str(),
                "provider_pending" | "processing" | "complete" | "error"
            );
        valid_uuid(&self.mapped_video_id)
            && self.legacy_video_id == legacy_video_id
            && valid_cap_id(&self.legacy_video_id)
            && self.actor_id == actor_id
            && valid_id(&self.actor_id)
            && valid_id(&self.organization_id)
            && valid_uuid(&self.upload_id)
            && valid_object_key(&self.raw_file_key)
            && self.content_type.starts_with("video/")
            && self.content_type.len() <= 127
            && (no_intent || pending_intent)
    }

    pub(crate) fn admits_input(&self, input: &LegacyMobileUploadCompleteInputV1) -> bool {
        input.valid() && input.raw_file_key == self.raw_file_key
    }

    pub(crate) fn pending_disposition(
        &self,
        input: &LegacyMobileUploadCompleteInputV1,
    ) -> Option<Result<(), LegacyMobileUploadsFailureV1>> {
        let state = self.intent_state.as_deref()?;
        let observed = u64::try_from(self.intent_observed_bytes?).ok()?;
        if input
            .normalized_content_length()
            .is_some_and(|length| length != observed)
        {
            return Some(Err(LegacyMobileUploadsFailureV1::Conflict));
        }
        Some(match state {
            "provider_pending" => Err(LegacyMobileUploadsFailureV1::ProviderGated),
            "submitted" | "complete" => Ok(()),
            "failed" => Err(LegacyMobileUploadsFailureV1::Unavailable),
            _ => Err(LegacyMobileUploadsFailureV1::Corrupt),
        })
    }
}

pub(crate) struct D1LegacyMobileUploadsV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyMobileUploadsV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) async fn create(
        &self,
        actor_id: &str,
        input: &LegacyMobileUploadCreateInputV1,
        web_url: &str,
        default_public: bool,
        signer: &R2DirectPutSigner,
        now_ms: i64,
    ) -> Result<LegacyMobileUploadCreateResponseV1, LegacyMobileUploadsFailureV1> {
        if !valid_id(actor_id)
            || !input.valid()
            || !valid_safe_i64(now_ms)
            || !valid_web_url(web_url)
        {
            return Err(LegacyMobileUploadsFailureV1::Invalid);
        }
        let authority = self.create_authority(actor_id, input).await?;
        let operation_id = Uuid::now_v7().to_string();
        let mapped_video_id = Uuid::now_v7().to_string();
        let upload_id = Uuid::now_v7().to_string();
        let legacy_video_id = random_cap_nanoid()?;
        let extension = legacy_mobile_upload_extension(&input.file_name, &input.content_type);
        let raw_file_key =
            legacy_mobile_upload_raw_key(&authority.legacy_actor_id, &legacy_video_id, &extension)
                .ok_or(LegacyMobileUploadsFailureV1::Invalid)?;
        let source_object_key = format!(
            "{}/{}/result.mp4",
            authority.legacy_actor_id, legacy_video_id
        );
        let capability = signer
            .sign_legacy_storage_put(
                &raw_file_key,
                &BTreeMap::from([("Content-Type".into(), input.content_type.clone())]),
                u64::try_from(now_ms).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?,
                LEGACY_MOBILE_UPLOADS_UPLOAD_TTL_SECONDS,
            )
            .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
        let title = legacy_mobile_upload_title(&input.file_name);
        let expected_bytes = input.normalized_content_length().unwrap_or(0);
        let expected_bytes_i64 =
            i64::try_from(expected_bytes).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
        let duration_ms = duration_ms(input.duration_seconds);
        let privacy = if default_public { "public" } else { "private" };
        let request_digest = request_digest(input)?;
        let folder_id = js_opt(authority.folder_id.as_deref());
        let statements = vec![
            self.statement(
                CREATE_AUTHORITY_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(actor_id),
                    js(&authority.legacy_actor_id),
                    js(&authority.organization_id),
                    js(&authority.storage_integration_id),
                    folder_id.clone(),
                ],
            )?,
            self.statement(
                CREATE_VIDEO_INSERT_SQL,
                &[
                    js(&mapped_video_id),
                    js(actor_id),
                    js(&title),
                    number_opt(duration_ms),
                    number(now_ms),
                    js(&authority.organization_id),
                    folder_id.clone(),
                    js(privacy),
                    js(&operation_id),
                    bool_number(default_public),
                    float_opt(input.duration_seconds),
                    float_opt(input.width),
                    float_opt(input.height),
                    js(&source_object_key),
                ],
            )?,
            self.statement(
                CREATE_ALIAS_INSERT_SQL,
                &[js(&legacy_video_id), js(&mapped_video_id), number(now_ms)],
            )?,
            self.statement(
                CREATE_UPLOAD_INSERT_SQL,
                &[
                    js(&upload_id),
                    js(&authority.organization_id),
                    js(&mapped_video_id),
                    number(expected_bytes_i64),
                    js(&format!("legacy-mobile-upload:{legacy_video_id}")),
                    js(&raw_file_key),
                    js(&input.content_type),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                CREATE_RECORD_INSERT_SQL,
                &[
                    js(&mapped_video_id),
                    js(&legacy_video_id),
                    js(actor_id),
                    js(&authority.legacy_actor_id),
                    js(&authority.organization_id),
                    js(&authority.storage_integration_id),
                    js(&upload_id),
                    folder_id,
                    js(&raw_file_key),
                    js(&input.file_name),
                    js(&input.content_type),
                    input
                        .content_length
                        .map(|_| number(expected_bytes_i64))
                        .unwrap_or(JsValue::NULL),
                    float_opt(input.duration_seconds),
                    float_opt(input.width),
                    float_opt(input.height),
                    float_opt(input.fps),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.operation_statement(
                &operation_id,
                LEGACY_MOBILE_UPLOAD_CREATE_OPERATION_ID,
                "create",
                actor_id,
                &authority.organization_id,
                &mapped_video_id,
                &legacy_video_id,
                &request_digest,
                "complete",
                now_ms,
            )?,
            self.statement(
                CREATE_POSTCONDITION_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&mapped_video_id),
                    js(&legacy_video_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&raw_file_key),
                    js(&upload_id),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&operation_id)])?,
        ];
        self.batch(statements).await?;

        let created_at =
            legacy_mobile_iso_from_millis(now_ms).ok_or(LegacyMobileUploadsFailureV1::Corrupt)?;
        let share_url = format!("{}/s/{legacy_video_id}", web_url.trim_end_matches('/'));
        let cap = LegacyMobileCapSummaryV1 {
            id: legacy_video_id.clone(),
            share_url: share_url.clone(),
            title,
            created_at: created_at.clone(),
            updated_at: created_at,
            owner_name: authority.owner_name,
            duration_seconds: input.duration_seconds,
            thumbnail_url: None,
            folder_id: authority.legacy_folder_id,
            public: default_public,
            protected: false,
            view_count: 0.0,
            comment_count: 0.0,
            reaction_count: 0.0,
            upload: Some(LegacyMobileUploadProgressV1 {
                uploaded: 0.0,
                total: expected_bytes as f64,
                phase: "uploading".into(),
                processing_progress: 0.0,
                processing_message: None,
                processing_error: None,
            }),
        };
        let headers = capability.required_headers.into_iter().collect();
        Ok(LegacyMobileUploadCreateResponseV1 {
            id: legacy_video_id,
            share_url,
            raw_file_key,
            upload: LegacyMobileUploadPutTargetV1 {
                target_type: "put",
                url: capability.url,
                headers,
            },
            cap,
        })
    }

    pub(crate) async fn progress(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        input: &LegacyMobileUploadProgressInputV1,
        now_ms: i64,
    ) -> Result<(), LegacyMobileUploadsFailureV1> {
        let (uploaded, total) = input
            .normalized()
            .ok_or(LegacyMobileUploadsFailureV1::Invalid)?;
        if !valid_id(actor_id) || !valid_cap_id(legacy_video_id) || !valid_safe_i64(now_ms) {
            return Err(LegacyMobileUploadsFailureV1::Invalid);
        }
        let snapshot = self
            .one::<ProgressSnapshotRowV1>(
                PROGRESS_SNAPSHOT_SQL,
                &[js(actor_id), js(legacy_video_id)],
            )
            .await?
            .ok_or(LegacyMobileUploadsFailureV1::NotFound)?;
        if !snapshot.valid(legacy_video_id) {
            return Err(LegacyMobileUploadsFailureV1::Corrupt);
        }
        let operation_id = Uuid::now_v7().to_string();
        let upload_id = snapshot
            .upload_id
            .clone()
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let request_digest = request_digest(input)?;
        let uploaded =
            i64::try_from(uploaded).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
        let total = i64::try_from(total).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
        let mut statements = Vec::with_capacity(7);
        if snapshot.upload_id.is_none() {
            let fallback_key = format!(
                "{}/{}/raw-upload.mp4",
                snapshot.legacy_actor_id, snapshot.legacy_video_id
            );
            statements.push(self.statement(
                PROGRESS_UPLOAD_INSERT_SQL,
                &[
                    js(&upload_id),
                    js(&snapshot.organization_id),
                    js(&snapshot.mapped_video_id),
                    js(&format!("legacy-mobile-progress:{legacy_video_id}")),
                    js(&fallback_key),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?);
        }
        statements.extend([
            self.statement(
                PROGRESS_AUTHORITY_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&snapshot.mapped_video_id),
                    js(actor_id),
                    js(legacy_video_id),
                    js(&upload_id),
                ],
            )?,
            self.statement(
                PROGRESS_UPDATE_SQL,
                &[
                    js(&snapshot.mapped_video_id),
                    js(&upload_id),
                    number(uploaded),
                    number(total),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.operation_statement(
                &operation_id,
                LEGACY_MOBILE_UPLOAD_PROGRESS_OPERATION_ID,
                "progress",
                actor_id,
                &snapshot.organization_id,
                &snapshot.mapped_video_id,
                legacy_video_id,
                &request_digest,
                "complete",
                now_ms,
            )?,
            self.statement(
                PROGRESS_POSTCONDITION_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&upload_id),
                    js(&snapshot.mapped_video_id),
                    number(uploaded),
                    number(total),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&operation_id)])?,
        ]);
        self.batch(statements).await
    }

    pub(crate) async fn completion_snapshot(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        input: &LegacyMobileUploadCompleteInputV1,
    ) -> Result<LegacyMobileUploadCompletionSnapshotV1, LegacyMobileUploadsFailureV1> {
        if !valid_id(actor_id) || !valid_cap_id(legacy_video_id) || !input.valid() {
            return Err(LegacyMobileUploadsFailureV1::Invalid);
        }
        let row = self
            .one::<LegacyMobileUploadCompletionSnapshotV1>(
                COMPLETE_SNAPSHOT_SQL,
                &[js(actor_id), js(legacy_video_id)],
            )
            .await?
            .ok_or(LegacyMobileUploadsFailureV1::NotFound)?;
        if !row.valid(actor_id, legacy_video_id) {
            return Err(LegacyMobileUploadsFailureV1::Corrupt);
        }
        if !row.admits_input(input) {
            return Err(LegacyMobileUploadsFailureV1::Invalid);
        }
        Ok(row)
    }

    pub(crate) async fn begin_completion(
        &self,
        snapshot: &LegacyMobileUploadCompletionSnapshotV1,
        input: &LegacyMobileUploadCompleteInputV1,
        observed_bytes: u64,
        now_ms: i64,
    ) -> Result<(), LegacyMobileUploadsFailureV1> {
        if !snapshot.admits_input(input)
            || observed_bytes == 0
            || observed_bytes > LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER
            || !valid_safe_i64(now_ms)
        {
            return Err(LegacyMobileUploadsFailureV1::Invalid);
        }
        if let Some(disposition) = snapshot.pending_disposition(input) {
            return disposition;
        }
        if input
            .normalized_content_length()
            .is_some_and(|length| length != observed_bytes)
        {
            return Err(LegacyMobileUploadsFailureV1::Conflict);
        }
        let observed_bytes =
            i64::try_from(observed_bytes).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
        let requested_content_length = input
            .normalized_content_length()
            .map(i64::try_from)
            .transpose()
            .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
        let operation_id = Uuid::now_v7().to_string();
        let request_digest = request_digest(input)?;
        self.batch(vec![
            self.statement(
                COMPLETE_AUTHORITY_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&snapshot.mapped_video_id),
                    js(&snapshot.legacy_video_id),
                    js(&snapshot.actor_id),
                    js(&snapshot.organization_id),
                    js(&snapshot.raw_file_key),
                ],
            )?,
            self.operation_statement(
                &operation_id,
                LEGACY_MOBILE_UPLOAD_COMPLETE_OPERATION_ID,
                "complete",
                &snapshot.actor_id,
                &snapshot.organization_id,
                &snapshot.mapped_video_id,
                &snapshot.legacy_video_id,
                &request_digest,
                "provider_pending",
                now_ms,
            )?,
            self.statement(
                COMPLETE_UPLOAD_BYTES_SQL,
                &[
                    js(&snapshot.upload_id),
                    js(&snapshot.mapped_video_id),
                    js(&snapshot.organization_id),
                    number(observed_bytes),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                COMPLETE_RECORD_PENDING_SQL,
                &[
                    js(&snapshot.mapped_video_id),
                    js(&snapshot.actor_id),
                    js(&operation_id),
                    number(now_ms),
                ],
            )?,
            self.statement(
                COMPLETE_INTENT_INSERT_SQL,
                &[
                    js(&snapshot.mapped_video_id),
                    js(&operation_id),
                    js(&snapshot.actor_id),
                    js(&snapshot.organization_id),
                    js(&snapshot.raw_file_key),
                    number(observed_bytes),
                    requested_content_length
                        .map(number)
                        .unwrap_or(JsValue::NULL),
                    number(now_ms),
                ],
            )?,
            self.statement(
                COMPLETE_PENDING_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&snapshot.mapped_video_id),
                    number(observed_bytes),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&operation_id)])?,
        ])
        .await?;
        Err(LegacyMobileUploadsFailureV1::ProviderGated)
    }

    async fn create_authority(
        &self,
        actor_id: &str,
        input: &LegacyMobileUploadCreateInputV1,
    ) -> Result<CreateAuthorityRowV1, LegacyMobileUploadsFailureV1> {
        let rows = self
            .rows::<CreateAuthorityRowV1>(
                CREATE_AUTHORITY_SQL,
                &[
                    js(actor_id),
                    js_opt(input.organization_id.as_deref()),
                    js_opt(input.folder_id.as_deref()),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyMobileUploadsFailureV1::Corrupt);
        }
        if let Some(row) = rows.into_iter().next() {
            return row
                .valid(
                    actor_id,
                    input.organization_id.as_deref(),
                    input.folder_id.as_deref(),
                )
                .then_some(row)
                .ok_or(LegacyMobileUploadsFailureV1::Corrupt);
        }
        if input.folder_id.is_some() {
            let without_folder = self
                .rows::<CreateAuthorityRowV1>(
                    CREATE_AUTHORITY_SQL,
                    &[
                        js(actor_id),
                        js_opt(input.organization_id.as_deref()),
                        JsValue::NULL,
                    ],
                )
                .await?;
            if without_folder.len() > 1 {
                return Err(LegacyMobileUploadsFailureV1::Corrupt);
            }
            if without_folder
                .into_iter()
                .next()
                .is_some_and(|row| row.valid(actor_id, input.organization_id.as_deref(), None))
            {
                return Err(LegacyMobileUploadsFailureV1::NotFound);
            }
        }
        Err(LegacyMobileUploadsFailureV1::Forbidden)
    }

    #[allow(clippy::too_many_arguments)]
    fn operation_statement(
        &self,
        operation_id: &str,
        source_operation_id: &str,
        kind: &str,
        actor_id: &str,
        organization_id: &str,
        mapped_video_id: &str,
        legacy_video_id: &str,
        request_digest: &str,
        state: &str,
        now_ms: i64,
    ) -> Result<D1PreparedStatement, LegacyMobileUploadsFailureV1> {
        self.statement(
            OPERATION_INSERT_SQL,
            &[
                js(operation_id),
                js(source_operation_id),
                js(kind),
                js(actor_id),
                js(organization_id),
                js(mapped_video_id),
                js(legacy_video_id),
                js(request_digest),
                js(state),
                number(now_ms),
            ],
        )
    }

    async fn one<T: for<'de> Deserialize<'de>>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<Option<T>, LegacyMobileUploadsFailureV1> {
        let rows = self.rows(sql, bindings).await?;
        if rows.len() > 1 {
            return Err(LegacyMobileUploadsFailureV1::Corrupt);
        }
        Ok(rows.into_iter().next())
    }

    async fn rows<T: for<'de> Deserialize<'de>>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<Vec<T>, LegacyMobileUploadsFailureV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyMobileUploadsFailureV1::Corrupt)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?
            .results::<T>()
            .map_err(|_| LegacyMobileUploadsFailureV1::Corrupt)
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<D1PreparedStatement, LegacyMobileUploadsFailureV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyMobileUploadsFailureV1::Corrupt)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyMobileUploadsFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyMobileUploadsFailureV1::Unavailable);
        }
        Ok(())
    }
}

fn map_d1_message(message: &str) -> LegacyMobileUploadsFailureV1 {
    if message.contains("frame_legacy_mobile_upload_assertion_v1") {
        LegacyMobileUploadsFailureV1::Forbidden
    } else if message.contains("frame_legacy_mobile_upload_")
        || message.contains("frame_business_")
        || message.contains("foreign key")
    {
        LegacyMobileUploadsFailureV1::Corrupt
    } else {
        LegacyMobileUploadsFailureV1::Unavailable
    }
}

fn random_cap_nanoid() -> Result<String, LegacyMobileUploadsFailureV1> {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut random = [0_u8; 15];
    getrandom::fill(&mut random).map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
    let value = random
        .into_iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect::<String>();
    valid_cap_id(&value)
        .then_some(value)
        .ok_or(LegacyMobileUploadsFailureV1::Corrupt)
}

fn request_digest<T: Serialize>(value: &T) -> Result<String, LegacyMobileUploadsFailureV1> {
    let encoded = serde_json::to_vec(value).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(encoded) {
        write!(&mut output, "{byte:02x}").expect("write digest");
    }
    Ok(output)
}

fn duration_ms(value: Option<f64>) -> Option<i64> {
    let value = value?;
    let milliseconds = value * 1_000.0;
    (value >= 0.0 && milliseconds <= LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER as f64)
        .then(|| milliseconds.round() as i64)
}

fn valid_cap_id(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
        && !value.chars().any(char::is_control)
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36 && Uuid::parse_str(value).is_ok()
}

fn valid_safe_i64(value: i64) -> bool {
    (0..=LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER as i64).contains(&value)
}

fn valid_positive_safe_i64(value: i64) -> bool {
    (1..=LEGACY_MOBILE_UPLOADS_MAX_SAFE_INTEGER as i64).contains(&value)
}

fn valid_object_key(value: &str) -> bool {
    value.len() >= 35
        && value.len() <= 512
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.contains(['?', '#', '%'])
        && !value.chars().any(char::is_control)
}

fn valid_web_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn js_opt(value: Option<&str>) -> JsValue {
    value.map(js).unwrap_or(JsValue::NULL)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn number_opt(value: Option<i64>) -> JsValue {
    value.map(number).unwrap_or(JsValue::NULL)
}

fn float_opt(value: Option<f64>) -> JsValue {
    value.map(JsValue::from_f64).unwrap_or(JsValue::NULL)
}

fn bool_number(value: bool) -> JsValue {
    JsValue::from_f64(f64::from(u8::from(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_never_treats_provider_pending_as_success() {
        let snapshot = LegacyMobileUploadCompletionSnapshotV1 {
            mapped_video_id: Uuid::now_v7().to_string(),
            legacy_video_id: "0123456789abcde".into(),
            actor_id: Uuid::now_v7().to_string(),
            organization_id: Uuid::now_v7().to_string(),
            upload_id: Uuid::now_v7().to_string(),
            raw_file_key: "0123456789abcdf/0123456789abcde/raw-upload.mp4".into(),
            content_type: "video/mp4".into(),
            lifecycle_state: "provider_pending".into(),
            intent_operation_id: Some(Uuid::now_v7().to_string()),
            intent_observed_bytes: Some(42),
            intent_requested_content_length: Some(42),
            intent_state: Some("provider_pending".into()),
        };
        let input = LegacyMobileUploadCompleteInputV1 {
            raw_file_key: snapshot.raw_file_key.clone(),
            content_length: Some(42.0),
        };
        assert_eq!(
            snapshot.pending_disposition(&input),
            Some(Err(LegacyMobileUploadsFailureV1::ProviderGated))
        );
    }
}
