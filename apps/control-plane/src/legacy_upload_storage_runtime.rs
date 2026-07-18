//! D1 authority and replay adapter for Cap upload/storage compatibility.

use std::{collections::BTreeMap, fmt::Write as _};

use frame_application::{
    LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID, LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID,
    LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID, LEGACY_SHARE_CAP_OPERATION_ID,
    LEGACY_UPLOAD_STORAGE_CAPABILITY_TTL_SECONDS, LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_OPERATION_ID,
    LegacyCreateVideoUploadInputV1, LegacyCreateVideoUploadResultV1, LegacyUploadTargetV1,
    legacy_create_video_title, legacy_should_clear_edit_upload, legacy_upload_content_type,
    valid_cap_id,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

use crate::r2_direct_upload::R2DirectPutSigner;

const READ_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_upload_storage/read_authority.sql");
const PASSWORD_CANDIDATES_SQL: &str =
    include_str!("../queries/legacy_upload_storage/password_candidates.sql");
const OWNER_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_upload_storage/owner_authority.sql");
const CREATE_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_upload_storage/create_authority.sql");
const EXISTING_VIDEO_SQL: &str =
    include_str!("../queries/legacy_upload_storage/existing_video.sql");
const OPERATION_REPLAY_SQL: &str =
    include_str!("../queries/legacy_upload_storage/operation_replay.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/operation_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_upload_storage/operation_complete.sql");
const PROGRESS_UPSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/progress_upsert.sql");
const CREATE_VIDEO_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/create_video_insert.sql");
const CREATE_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/create_alias_insert.sql");
const CREATE_PROGRESS_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/create_progress_insert.sql");
const CAPABILITY_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/capability_insert.sql");
const DELETE_PROGRESS_SQL: &str =
    include_str!("../queries/legacy_upload_storage/delete_progress.sql");
const DELETE_INTENT_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/delete_intent_insert.sql");
const DELETE_INTENT_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_upload_storage/delete_intent_complete.sql");
const RECONCILE_DELETE_SQL: &str =
    include_str!("../queries/legacy_upload_storage/reconcile_delete.sql");
const SHARE_ORG_DELETE_SQL: &str =
    include_str!("../queries/legacy_upload_storage/share_org_delete.sql");
const SHARE_ORG_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/share_org_insert.sql");
const SHARE_SPACE_DELETE_SQL: &str =
    include_str!("../queries/legacy_upload_storage/share_space_delete.sql");
const SHARE_SPACE_INSERT_SQL: &str =
    include_str!("../queries/legacy_upload_storage/share_space_insert.sql");
const SHARE_PUBLIC_UPDATE_SQL: &str =
    include_str!("../queries/legacy_upload_storage/share_public_update.sql");

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacyUploadStorageFailureV1 {
    Invalid,
    Forbidden,
    PasswordNotProvided(String),
    PasswordWrong(String),
    NotFound,
    Conflict,
    UpgradeRequired,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyUploadStorageReadAuthorityV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) organization_id: String,
    pub(crate) object_prefix: String,
    pub(crate) source_type: String,
    pub(crate) title: String,
    pub(crate) legacy_is_screenshot: i64,
    legacy_public: i64,
    legacy_password_hash: Option<String>,
    actor_email: Option<String>,
    allowed_email_restriction: Option<String>,
    pub(crate) storage_integration_id: String,
    pub(crate) uploaded: Option<f64>,
    pub(crate) total: Option<f64>,
    pub(crate) started_at_ms: Option<i64>,
    pub(crate) upload_updated_at_ms: Option<i64>,
    pub(crate) phase: Option<String>,
    pub(crate) processing_progress: Option<f64>,
    pub(crate) processing_message: Option<String>,
    pub(crate) processing_error: Option<String>,
    pub(crate) raw_file_key: Option<String>,
    pub(crate) edit_source_key: Option<String>,
    explicit_view: i64,
    can_download: i64,
}

impl LegacyUploadStorageReadAuthorityV1 {
    fn valid(&self, legacy_video_id: &str) -> bool {
        valid_uuid(&self.mapped_video_id)
            && valid_cap_id(&self.legacy_video_id)
            && self.legacy_video_id == legacy_video_id
            && valid_uuid(&self.owner_id)
            && valid_uuid(&self.organization_id)
            && valid_uuid(&self.storage_integration_id)
            && valid_object_prefix(&self.object_prefix, legacy_video_id)
            && matches!(
                self.source_type.as_str(),
                "MediaConvert" | "local" | "desktopMP4" | "desktopSegments" | "webMP4"
            )
            && matches!(self.legacy_is_screenshot, 0 | 1)
            && matches!(self.legacy_public, 0 | 1)
            && matches!(self.explicit_view, 0 | 1)
            && matches!(self.can_download, 0 | 1)
            && self.title.len() <= 4_096
            && self
                .legacy_password_hash
                .as_deref()
                .is_none_or(valid_password_hash)
            && self
                .actor_email
                .as_deref()
                .is_none_or(|value| value.len() <= 4_096 && !value.chars().any(char::is_control))
            && self
                .allowed_email_restriction
                .as_deref()
                .is_none_or(|value| value.len() <= 4_096 && !value.chars().any(char::is_control))
            && self.raw_file_key.as_deref().is_none_or(valid_object_key)
            && self.edit_source_key.as_deref().is_none_or(valid_object_key)
            && upload_shape_valid(self)
    }

    pub(crate) fn downloadable(&self) -> bool {
        self.can_download == 1
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PasswordCandidateV1 {
    password_hash: String,
    ordinal: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyUploadStorageOwnerAuthorityV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) organization_id: String,
    pub(crate) object_prefix: String,
    pub(crate) source_type: String,
    pub(crate) title: String,
    pub(crate) legacy_is_screenshot: i64,
    pub(crate) storage_integration_id: String,
    pub(crate) phase: Option<String>,
    pub(crate) processing_progress: Option<f64>,
    pub(crate) upload_updated_at_ms: Option<i64>,
    pub(crate) raw_file_key: Option<String>,
    pub(crate) edit_source_key: Option<String>,
}

impl LegacyUploadStorageOwnerAuthorityV1 {
    fn valid(&self, actor_id: &str, legacy_video_id: &str) -> bool {
        valid_uuid(&self.mapped_video_id)
            && self.owner_id == actor_id
            && valid_uuid(&self.owner_id)
            && valid_uuid(&self.organization_id)
            && valid_uuid(&self.storage_integration_id)
            && self.legacy_video_id == legacy_video_id
            && valid_cap_id(&self.legacy_video_id)
            && valid_object_prefix(&self.object_prefix, legacy_video_id)
            && matches!(
                self.source_type.as_str(),
                "MediaConvert" | "local" | "desktopMP4" | "desktopSegments" | "webMP4"
            )
            && matches!(self.legacy_is_screenshot, 0 | 1)
            && self.title.len() <= 4_096
            && self.phase.as_deref().is_none_or(valid_phase)
            && self.processing_progress.is_none_or(valid_number)
            && self.upload_updated_at_ms.is_none_or(valid_safe_integer)
            && self.raw_file_key.as_deref().is_none_or(valid_object_key)
            && self.edit_source_key.as_deref().is_none_or(valid_object_key)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CreateAuthorityV1 {
    actor_id: String,
    legacy_actor_id: String,
    organization_id: String,
    legacy_organization_id: String,
    storage_integration_id: String,
    has_pro_seat: i64,
    folder_id: Option<String>,
    legacy_folder_id: Option<String>,
}

impl CreateAuthorityV1 {
    fn valid(&self, actor_id: &str, input: &LegacyCreateVideoUploadInputV1) -> bool {
        self.actor_id == actor_id
            && valid_uuid(&self.actor_id)
            && valid_cap_id(&self.legacy_actor_id)
            && valid_uuid(&self.organization_id)
            && self.legacy_organization_id == input.org_id
            && valid_cap_id(&self.legacy_organization_id)
            && valid_uuid(&self.storage_integration_id)
            && matches!(self.has_pro_seat, 0 | 1)
            && self.folder_id.as_deref().is_none_or(valid_uuid)
            && self.legacy_folder_id.as_deref() == input.folder_id.as_deref()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ExistingVideoV1 {
    mapped_video_id: String,
    legacy_video_id: String,
    owner_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OperationRowV1 {
    operation_id: String,
    mapped_video_id: String,
    legacy_video_id: String,
    request_digest: String,
    state: String,
    result_json: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyDeleteResultPlanV1 {
    pub(crate) operation_id: String,
    pub(crate) object_key: String,
    pub(crate) replayed_complete: bool,
}

pub(crate) struct D1LegacyUploadStorageV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyUploadStorageV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn read_authority(
        &self,
        actor_id: Option<&str>,
        legacy_video_id: &str,
        verified_password_hashes: &[String],
    ) -> Result<LegacyUploadStorageReadAuthorityV1, LegacyUploadStorageFailureV1> {
        let row = self.read_row(actor_id, legacy_video_id).await?;
        if actor_id == Some(row.owner_id.as_str()) {
            return Ok(row);
        }
        let public_email_allowed = row.legacy_public == 1
            && row
                .allowed_email_restriction
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none_or(|restriction| {
                    email_restriction_allows(row.actor_email.as_deref(), restriction)
                });
        if row.explicit_view != 1 && !public_email_allowed {
            return Err(LegacyUploadStorageFailureV1::Forbidden);
        }
        let candidates = self.password_candidates(&row.mapped_video_id).await?;
        if candidates.is_empty()
            || verified_password_hashes
                .iter()
                .any(|verified| candidates.iter().any(|candidate| candidate == verified))
        {
            return Ok(row);
        }
        Err(if verified_password_hashes.is_empty() {
            LegacyUploadStorageFailureV1::PasswordNotProvided(legacy_video_id.into())
        } else {
            LegacyUploadStorageFailureV1::PasswordWrong(legacy_video_id.into())
        })
    }

    pub(crate) async fn download_authority(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
    ) -> Result<LegacyUploadStorageReadAuthorityV1, LegacyUploadStorageFailureV1> {
        let row = self.read_row(Some(actor_id), legacy_video_id).await?;
        row.downloadable()
            .then_some(row)
            .ok_or(LegacyUploadStorageFailureV1::Forbidden)
    }

    async fn read_row(
        &self,
        actor_id: Option<&str>,
        legacy_video_id: &str,
    ) -> Result<LegacyUploadStorageReadAuthorityV1, LegacyUploadStorageFailureV1> {
        if actor_id.is_some_and(|value| !valid_uuid(value)) || !valid_cap_id(legacy_video_id) {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let rows = self
            .rows::<LegacyUploadStorageReadAuthorityV1>(
                READ_AUTHORITY_SQL,
                &[js_opt(actor_id), js(legacy_video_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        let row = rows
            .into_iter()
            .next()
            .ok_or(LegacyUploadStorageFailureV1::NotFound)?;
        if !row.valid(legacy_video_id) {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        Ok(row)
    }

    async fn password_candidates(
        &self,
        mapped_video_id: &str,
    ) -> Result<Vec<String>, LegacyUploadStorageFailureV1> {
        let rows = self
            .rows::<PasswordCandidateV1>(PASSWORD_CANDIDATES_SQL, &[js(mapped_video_id)])
            .await?;
        if rows.len() > 1_001
            || rows.iter().enumerate().any(|(index, row)| {
                row.ordinal != i64::try_from(index).unwrap_or(i64::MAX)
                    || !valid_password_hash(&row.password_hash)
            })
        {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        Ok(rows.into_iter().map(|row| row.password_hash).collect())
    }

    pub(crate) async fn owner_authority(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
    ) -> Result<LegacyUploadStorageOwnerAuthorityV1, LegacyUploadStorageFailureV1> {
        if !valid_uuid(actor_id) || !valid_cap_id(legacy_video_id) {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let rows = self
            .rows::<LegacyUploadStorageOwnerAuthorityV1>(
                OWNER_AUTHORITY_SQL,
                &[js(actor_id), js(legacy_video_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        let row = rows
            .into_iter()
            .next()
            .ok_or(LegacyUploadStorageFailureV1::NotFound)?;
        row.valid(actor_id, legacy_video_id)
            .then_some(row)
            .ok_or(LegacyUploadStorageFailureV1::Corrupt)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) async fn create_upload(
        &self,
        actor_id: &str,
        input: &LegacyCreateVideoUploadInputV1,
        idempotency_key: &str,
        default_public: bool,
        signer: &R2DirectPutSigner,
        now_ms: i64,
    ) -> Result<LegacyCreateVideoUploadResultV1, LegacyUploadStorageFailureV1> {
        if !valid_uuid(actor_id)
            || !input.valid()
            || !valid_idempotency(idempotency_key)
            || !valid_safe_integer(now_ms)
        {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let request_digest = request_digest(input)?;
        let idempotency_digest = digest(idempotency_key.as_bytes());
        if let Some(replay) = self
            .replay(
                LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID,
                actor_id,
                &idempotency_digest,
            )
            .await?
        {
            return replay_result(&replay, &request_digest);
        }
        let authority = self.create_authority(actor_id, input).await?;
        if authority.has_pro_seat == 0 && input.duration.is_some_and(|value| value > 300.0) {
            return Err(LegacyUploadStorageFailureV1::UpgradeRequired);
        }

        let existing = match input.video_id.as_deref() {
            Some(video_id) => self.existing_video(video_id).await?,
            None => None,
        };
        if existing
            .as_ref()
            .is_some_and(|video| video.owner_id != actor_id)
        {
            return Err(LegacyUploadStorageFailureV1::Forbidden);
        }
        let (
            mapped_video_id,
            legacy_video_id,
            object_prefix,
            integration_id,
            operation_organization_id,
            is_new,
        ) = if let Some(existing) = existing {
            let owner = self
                .owner_authority(actor_id, &existing.legacy_video_id)
                .await?;
            (
                existing.mapped_video_id,
                existing.legacy_video_id,
                owner.object_prefix,
                owner.storage_integration_id,
                owner.organization_id,
                false,
            )
        } else {
            let legacy_video_id = input.video_id.clone().map_or_else(random_cap_nanoid, Ok)?;
            (
                Uuid::now_v7().to_string(),
                legacy_video_id.clone(),
                format!("{}/{legacy_video_id}/", authority.legacy_actor_id),
                authority.storage_integration_id.clone(),
                authority.organization_id.clone(),
                true,
            )
        };
        let object_key = format!("{}{}", object_prefix, input.object_suffix());
        if !valid_object_key(&object_key) {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        let content_type = legacy_upload_content_type(&object_key);
        let mut headers = BTreeMap::from([
            ("content-type".into(), content_type.into()),
            (
                "x-amz-meta-userid".into(),
                authority.legacy_actor_id.clone(),
            ),
            (
                "x-amz-meta-duration".into(),
                input
                    .duration
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
            (
                "x-amz-meta-resolution".into(),
                input.resolution.clone().unwrap_or_default(),
            ),
            (
                "x-amz-meta-videocodec".into(),
                input.video_codec.clone().unwrap_or_default(),
            ),
            (
                "x-amz-meta-audiocodec".into(),
                input.audio_codec.clone().unwrap_or_default(),
            ),
        ]);
        // The signer normalizes names and adds the immutable write fence.
        let capability = signer
            .sign_legacy_storage_put(
                &object_key,
                &headers,
                u64::try_from(now_ms).map_err(|_| LegacyUploadStorageFailureV1::Invalid)?,
                LEGACY_UPLOAD_STORAGE_CAPABILITY_TTL_SECONDS,
            )
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
        headers.clear();
        headers.extend(capability.required_headers);
        let result = LegacyCreateVideoUploadResultV1 {
            id: legacy_video_id.clone(),
            presigned_post_data: None,
            upload_target: LegacyUploadTargetV1 {
                target_type: "put".into(),
                url: capability.url,
                headers,
            },
        };
        let result_json =
            serde_json::to_string(&result).map_err(|_| LegacyUploadStorageFailureV1::Corrupt)?;
        let operation_id = Uuid::now_v7().to_string();
        let mut statements = Vec::with_capacity(7);
        if is_new {
            let title = legacy_create_video_title(now_ms, input.is_screenshot, input.is_upload)
                .ok_or(LegacyUploadStorageFailureV1::Invalid)?;
            let duration_ms = input
                .duration
                .filter(|value| *value >= 0.0)
                .map(|value| (value * 1_000.0).round() as i64);
            statements.push(self.statement(
                CREATE_VIDEO_INSERT_SQL,
                &[
                    js(&mapped_video_id),
                    js(actor_id),
                    js(&title),
                    js(&object_key),
                    number_opt(duration_ms),
                    number(now_ms),
                    js(&authority.organization_id),
                    js_opt(authority.folder_id.as_deref()),
                    js(if default_public { "public" } else { "private" }),
                    js(&operation_id),
                    bool_number(default_public),
                    float_opt(input.duration),
                    bool_number(input.is_screenshot),
                ],
            )?);
            statements.push(self.statement(
                CREATE_ALIAS_INSERT_SQL,
                &[js(&legacy_video_id), js(&mapped_video_id), number(now_ms)],
            )?);
            if input.supports_upload_progress {
                statements.push(self.statement(
                    CREATE_PROGRESS_INSERT_SQL,
                    &[js(&mapped_video_id), number(now_ms)],
                )?);
            }
        }
        statements.push(self.operation_insert_statement(
            &operation_id,
            LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID,
            "create_upload",
            actor_id,
            &operation_organization_id,
            &mapped_video_id,
            &legacy_video_id,
            &idempotency_digest,
            &request_digest,
            "complete",
            Some(&result_json),
            now_ms,
            Some(now_ms),
        )?);
        statements.push(
            self.statement(
                CAPABILITY_INSERT_SQL,
                &[
                    js(&operation_id),
                    js(&integration_id),
                    js(&object_key),
                    js(content_type),
                    number(
                        i64::try_from(capability.expires_at_ms)
                            .map_err(|_| LegacyUploadStorageFailureV1::Invalid)?,
                    ),
                    number(now_ms),
                ],
            )?,
        );
        if let Err(failure) = self.batch(statements).await {
            if let Some(replay) = self
                .replay(
                    LEGACY_CREATE_VIDEO_UPLOAD_URL_OPERATION_ID,
                    actor_id,
                    &idempotency_digest,
                )
                .await?
            {
                return replay_result(&replay, &request_digest);
            }
            return Err(failure);
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn progress_update(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        uploaded: u64,
        total: u64,
        updated_at_ms: i64,
        rpc_request_id: &str,
        now_ms: i64,
    ) -> Result<bool, LegacyUploadStorageFailureV1> {
        if uploaded > MAX_SAFE_INTEGER as u64
            || total > MAX_SAFE_INTEGER as u64
            || !valid_safe_integer(updated_at_ms)
            || !valid_safe_integer(now_ms)
            || rpc_request_id.len() > 256
        {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let authority = self.owner_authority(actor_id, legacy_video_id).await?;
        let uploaded = i64::try_from(uploaded.min(total))
            .map_err(|_| LegacyUploadStorageFailureV1::Invalid)?;
        let total = i64::try_from(total).map_err(|_| LegacyUploadStorageFailureV1::Invalid)?;
        let request = json!({"videoId":legacy_video_id,"uploaded":uploaded,"total":total,"updatedAt":updated_at_ms});
        let request_digest = request_digest(&request)?;
        let idempotency_digest =
            digest(format!("effect-rpc:{rpc_request_id}:{request_digest}").as_bytes());
        if let Some(replay) = self
            .replay(
                LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_OPERATION_ID,
                actor_id,
                &idempotency_digest,
            )
            .await?
        {
            if replay.request_digest != request_digest || replay.legacy_video_id != legacy_video_id
            {
                return Err(LegacyUploadStorageFailureV1::Conflict);
            }
            return (replay.state == "complete")
                .then_some(true)
                .ok_or(LegacyUploadStorageFailureV1::Corrupt);
        }
        let operation_id = Uuid::now_v7().to_string();
        let result_json = "true";
        let batch = self
            .batch(vec![
                self.statement(
                    PROGRESS_UPSERT_SQL,
                    &[
                        js(&authority.mapped_video_id),
                        number(uploaded),
                        number(total),
                        number(updated_at_ms),
                    ],
                )?,
                self.operation_insert_statement(
                    &operation_id,
                    LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_OPERATION_ID,
                    "progress_update",
                    actor_id,
                    &authority.organization_id,
                    &authority.mapped_video_id,
                    legacy_video_id,
                    &idempotency_digest,
                    &request_digest,
                    "complete",
                    Some(result_json),
                    now_ms,
                    Some(now_ms),
                )?,
            ])
            .await;
        if let Err(failure) = batch {
            if let Some(replay) = self
                .replay(
                    LEGACY_VIDEO_UPLOAD_PROGRESS_UPDATE_OPERATION_ID,
                    actor_id,
                    &idempotency_digest,
                )
                .await?
            {
                if replay.request_digest != request_digest
                    || replay.legacy_video_id != legacy_video_id
                {
                    return Err(LegacyUploadStorageFailureV1::Conflict);
                }
                return (replay.state == "complete")
                    .then_some(true)
                    .ok_or(LegacyUploadStorageFailureV1::Corrupt);
            }
            return Err(failure);
        }
        Ok(true)
    }

    pub(crate) async fn begin_delete_result(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        idempotency_key: &str,
        now_ms: i64,
    ) -> Result<LegacyDeleteResultPlanV1, LegacyUploadStorageFailureV1> {
        if !valid_idempotency(idempotency_key) {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let authority = self.owner_authority(actor_id, legacy_video_id).await?;
        let request_digest = request_digest(&json!({"videoId":legacy_video_id}))?;
        let idempotency_digest = digest(idempotency_key.as_bytes());
        let object_key = format!("{}result.mp4", authority.object_prefix);
        if let Some(replay) = self
            .replay(
                LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID,
                actor_id,
                &idempotency_digest,
            )
            .await?
        {
            if replay.request_digest != request_digest || replay.legacy_video_id != legacy_video_id
            {
                return Err(LegacyUploadStorageFailureV1::Conflict);
            }
            if !matches!(replay.state.as_str(), "storage_pending" | "complete") {
                return Err(LegacyUploadStorageFailureV1::Corrupt);
            }
            return Ok(LegacyDeleteResultPlanV1 {
                operation_id: replay.operation_id,
                object_key,
                replayed_complete: replay.state == "complete",
            });
        }
        let operation_id = Uuid::now_v7().to_string();
        let batch = self
            .batch(vec![
                self.operation_insert_statement(
                    &operation_id,
                    LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID,
                    "delete_result",
                    actor_id,
                    &authority.organization_id,
                    &authority.mapped_video_id,
                    legacy_video_id,
                    &idempotency_digest,
                    &request_digest,
                    "storage_pending",
                    None,
                    now_ms,
                    None,
                )?,
                self.statement(DELETE_PROGRESS_SQL, &[js(&authority.mapped_video_id)])?,
                self.statement(
                    DELETE_INTENT_INSERT_SQL,
                    &[
                        js(&operation_id),
                        js(&authority.storage_integration_id),
                        js(&object_key),
                        number(now_ms),
                    ],
                )?,
            ])
            .await;
        if let Err(failure) = batch {
            if let Some(replay) = self
                .replay(
                    LEGACY_DELETE_VIDEO_RESULT_OPERATION_ID,
                    actor_id,
                    &idempotency_digest,
                )
                .await?
            {
                if replay.request_digest != request_digest
                    || replay.legacy_video_id != legacy_video_id
                {
                    return Err(LegacyUploadStorageFailureV1::Conflict);
                }
                if !matches!(replay.state.as_str(), "storage_pending" | "complete") {
                    return Err(LegacyUploadStorageFailureV1::Corrupt);
                }
                return Ok(LegacyDeleteResultPlanV1 {
                    operation_id: replay.operation_id,
                    object_key,
                    replayed_complete: replay.state == "complete",
                });
            }
            return Err(failure);
        }
        Ok(LegacyDeleteResultPlanV1 {
            operation_id,
            object_key,
            replayed_complete: false,
        })
    }

    pub(crate) async fn finish_delete_result(
        &self,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<(), LegacyUploadStorageFailureV1> {
        let result = r#"{"success":true}"#;
        self.batch(vec![
            self.statement(
                DELETE_INTENT_COMPLETE_SQL,
                &[js(operation_id), number(now_ms)],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[js(operation_id), js(result), number(now_ms)],
            )?,
        ])
        .await
    }

    pub(crate) async fn reconcile_edit(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        idempotency_key: &str,
        now_ms: i64,
    ) -> Result<bool, LegacyUploadStorageFailureV1> {
        if !valid_idempotency(idempotency_key) {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let authority = self.owner_authority(actor_id, legacy_video_id).await?;
        let request_digest = request_digest(&json!({"videoId":legacy_video_id}))?;
        let idempotency_digest = digest(idempotency_key.as_bytes());
        if let Some(replay) = self
            .replay(
                LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID,
                actor_id,
                &idempotency_digest,
            )
            .await?
        {
            if replay.request_digest != request_digest || replay.legacy_video_id != legacy_video_id
            {
                return Err(LegacyUploadStorageFailureV1::Conflict);
            }
            if replay.state != "complete" {
                return Err(LegacyUploadStorageFailureV1::Corrupt);
            }
            return replay
                .result_json
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
                .ok_or(LegacyUploadStorageFailureV1::Corrupt);
        }
        let expected_key = format!("{}source/original.mp4", authority.object_prefix);
        let clear = match (
            &authority.raw_file_key,
            &authority.phase,
            authority.processing_progress,
            authority.upload_updated_at_ms,
        ) {
            (Some(key), Some(phase), Some(progress), Some(updated_at)) if key == &expected_key => {
                legacy_should_clear_edit_upload(phase, progress, updated_at, now_ms)
            }
            _ => false,
        };
        let operation_id = Uuid::now_v7().to_string();
        let result_json = if clear { "true" } else { "false" };
        let mut statements = Vec::with_capacity(2);
        if clear {
            statements.push(self.statement(
                RECONCILE_DELETE_SQL,
                &[
                    js(&authority.mapped_video_id),
                    js(&expected_key),
                    number(authority.upload_updated_at_ms.expect("matched")),
                    js(authority.phase.as_deref().expect("matched")),
                    JsValue::from_f64(authority.processing_progress.expect("matched")),
                ],
            )?);
        }
        statements.push(self.operation_insert_statement(
            &operation_id,
            LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID,
            "reconcile_edit",
            actor_id,
            &authority.organization_id,
            &authority.mapped_video_id,
            legacy_video_id,
            &idempotency_digest,
            &request_digest,
            "complete",
            Some(result_json),
            now_ms,
            Some(now_ms),
        )?);
        if let Err(failure) = self.batch(statements).await {
            if let Some(replay) = self
                .replay(
                    LEGACY_RECONCILE_STALE_EDIT_UPLOAD_OPERATION_ID,
                    actor_id,
                    &idempotency_digest,
                )
                .await?
            {
                if replay.request_digest != request_digest
                    || replay.legacy_video_id != legacy_video_id
                {
                    return Err(LegacyUploadStorageFailureV1::Conflict);
                }
                if replay.state != "complete" {
                    return Err(LegacyUploadStorageFailureV1::Corrupt);
                }
                return replay
                    .result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .ok_or(LegacyUploadStorageFailureV1::Corrupt);
            }
            return Err(failure);
        }
        Ok(clear)
    }

    pub(crate) async fn share_cap(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        selected_ids: &[String],
        public: Option<bool>,
        idempotency_key: &str,
        now_ms: i64,
    ) -> Result<(), LegacyUploadStorageFailureV1> {
        if !valid_idempotency(idempotency_key)
            || selected_ids.len() > 1_000
            || selected_ids.iter().any(|value| !valid_cap_id(value))
        {
            return Err(LegacyUploadStorageFailureV1::Invalid);
        }
        let authority = self.owner_authority(actor_id, legacy_video_id).await?;
        let request = json!({"capId":legacy_video_id,"spaceIds":selected_ids,"public":public});
        let request_digest = request_digest(&request)?;
        let idempotency_digest = digest(idempotency_key.as_bytes());
        if let Some(replay) = self
            .replay(LEGACY_SHARE_CAP_OPERATION_ID, actor_id, &idempotency_digest)
            .await?
        {
            if replay.request_digest != request_digest || replay.legacy_video_id != legacy_video_id
            {
                return Err(LegacyUploadStorageFailureV1::Conflict);
            }
            return (replay.state == "complete")
                .then_some(())
                .ok_or(LegacyUploadStorageFailureV1::Corrupt);
        }
        let operation_id = Uuid::now_v7().to_string();
        let selected_json = serde_json::to_string(selected_ids)
            .map_err(|_| LegacyUploadStorageFailureV1::Invalid)?;
        let result_json = r#"{"success":true}"#;
        let batch = self
            .batch(vec![
                self.statement(SHARE_ORG_DELETE_SQL, &[js(&authority.mapped_video_id)])?,
                self.statement(
                    SHARE_ORG_INSERT_SQL,
                    &[
                        js(&authority.mapped_video_id),
                        js(actor_id),
                        js(&selected_json),
                        number(now_ms),
                        js(&operation_id),
                    ],
                )?,
                self.statement(SHARE_SPACE_DELETE_SQL, &[js(&authority.mapped_video_id)])?,
                self.statement(
                    SHARE_SPACE_INSERT_SQL,
                    &[
                        js(&authority.mapped_video_id),
                        js(actor_id),
                        js(&selected_json),
                        number(now_ms),
                        js(&operation_id),
                    ],
                )?,
                self.statement(
                    SHARE_PUBLIC_UPDATE_SQL,
                    &[
                        js(&authority.mapped_video_id),
                        public.map(bool_number).unwrap_or(JsValue::NULL),
                        number(now_ms),
                        js(&operation_id),
                    ],
                )?,
                self.operation_insert_statement(
                    &operation_id,
                    LEGACY_SHARE_CAP_OPERATION_ID,
                    "share_cap",
                    actor_id,
                    &authority.organization_id,
                    &authority.mapped_video_id,
                    legacy_video_id,
                    &idempotency_digest,
                    &request_digest,
                    "complete",
                    Some(result_json),
                    now_ms,
                    Some(now_ms),
                )?,
            ])
            .await;
        if let Err(failure) = batch {
            if let Some(replay) = self
                .replay(LEGACY_SHARE_CAP_OPERATION_ID, actor_id, &idempotency_digest)
                .await?
            {
                if replay.request_digest != request_digest
                    || replay.legacy_video_id != legacy_video_id
                {
                    return Err(LegacyUploadStorageFailureV1::Conflict);
                }
                return (replay.state == "complete")
                    .then_some(())
                    .ok_or(LegacyUploadStorageFailureV1::Corrupt);
            }
            return Err(failure);
        }
        Ok(())
    }

    async fn create_authority(
        &self,
        actor_id: &str,
        input: &LegacyCreateVideoUploadInputV1,
    ) -> Result<CreateAuthorityV1, LegacyUploadStorageFailureV1> {
        let rows = self
            .rows::<CreateAuthorityV1>(
                CREATE_AUTHORITY_SQL,
                &[
                    js(actor_id),
                    js(&input.org_id),
                    js_opt(input.folder_id.as_deref()),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        let row = rows
            .into_iter()
            .next()
            .ok_or(LegacyUploadStorageFailureV1::Forbidden)?;
        row.valid(actor_id, input)
            .then_some(row)
            .ok_or(LegacyUploadStorageFailureV1::Corrupt)
    }

    async fn existing_video(
        &self,
        legacy_video_id: &str,
    ) -> Result<Option<ExistingVideoV1>, LegacyUploadStorageFailureV1> {
        let rows = self
            .rows::<ExistingVideoV1>(EXISTING_VIDEO_SQL, &[js(legacy_video_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        let row = rows.into_iter().next();
        if row.as_ref().is_some_and(|row| {
            !valid_uuid(&row.mapped_video_id)
                || !valid_cap_id(&row.legacy_video_id)
                || !valid_uuid(&row.owner_id)
        }) {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        Ok(row)
    }

    async fn replay(
        &self,
        source: &str,
        actor: &str,
        idempotency_digest: &str,
    ) -> Result<Option<OperationRowV1>, LegacyUploadStorageFailureV1> {
        let rows = self
            .rows::<OperationRowV1>(
                OPERATION_REPLAY_SQL,
                &[js(source), js(actor), js(idempotency_digest)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        let row = rows.into_iter().next();
        if row.as_ref().is_some_and(|row| {
            !valid_uuid(&row.operation_id)
                || !valid_uuid(&row.mapped_video_id)
                || !valid_cap_id(&row.legacy_video_id)
                || row.request_digest.len() != 64
                || !matches!(
                    row.state.as_str(),
                    "claimed" | "storage_pending" | "complete"
                )
        }) {
            return Err(LegacyUploadStorageFailureV1::Corrupt);
        }
        Ok(row)
    }

    #[allow(clippy::too_many_arguments)]
    fn operation_insert_statement(
        &self,
        operation_id: &str,
        source: &str,
        kind: &str,
        actor: &str,
        organization: &str,
        video: &str,
        legacy_video: &str,
        idempotency_digest: &str,
        request_digest: &str,
        state: &str,
        result: Option<&str>,
        created_at: i64,
        completed_at: Option<i64>,
    ) -> Result<D1PreparedStatement, LegacyUploadStorageFailureV1> {
        self.statement(
            OPERATION_INSERT_SQL,
            &[
                js(operation_id),
                js(source),
                js(kind),
                js(actor),
                js(organization),
                js(video),
                js(legacy_video),
                js(idempotency_digest),
                js(request_digest),
                js(state),
                js_opt(result),
                number(created_at),
                completed_at.map(number).unwrap_or(JsValue::NULL),
            ],
        )
    }

    async fn rows<T: for<'de> Deserialize<'de>>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<Vec<T>, LegacyUploadStorageFailureV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?
            .results::<T>()
            .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<D1PreparedStatement, LegacyUploadStorageFailureV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyUploadStorageFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_error(&error.to_string()))?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyUploadStorageFailureV1::Unavailable);
        }
        Ok(())
    }
}

fn replay_result<T: for<'de> Deserialize<'de>>(
    row: &OperationRowV1,
    request_digest: &str,
) -> Result<T, LegacyUploadStorageFailureV1> {
    if row.request_digest != request_digest {
        return Err(LegacyUploadStorageFailureV1::Conflict);
    }
    if row.state != "complete" {
        return Err(LegacyUploadStorageFailureV1::Conflict);
    }
    serde_json::from_str(
        row.result_json
            .as_deref()
            .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
    )
    .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)
}

fn upload_shape_valid(row: &LegacyUploadStorageReadAuthorityV1) -> bool {
    let fields = [
        row.uploaded.is_some(),
        row.total.is_some(),
        row.started_at_ms.is_some(),
        row.upload_updated_at_ms.is_some(),
        row.phase.is_some(),
        row.processing_progress.is_some(),
    ];
    if fields.iter().all(|value| !value) {
        return true;
    }
    fields.iter().all(|value| *value)
        && row
            .uploaded
            .is_some_and(|value| value >= 0.0 && valid_number(value))
        && row
            .total
            .is_some_and(|value| value >= 0.0 && valid_number(value))
        && row.started_at_ms.is_some_and(valid_safe_integer)
        && row.upload_updated_at_ms.is_some_and(valid_safe_integer)
        && row.phase.as_deref().is_some_and(valid_phase)
        && row
            .processing_progress
            .is_some_and(|value| value >= 0.0 && valid_number(value))
}

fn map_d1_error(message: &str) -> LegacyUploadStorageFailureV1 {
    if message.contains("frame_legacy_upload_storage_assertion_v1") {
        LegacyUploadStorageFailureV1::Forbidden
    } else if message.contains("frame_legacy_upload_storage_")
        || message.contains("frame_business_")
        || message.contains("foreign key")
    {
        LegacyUploadStorageFailureV1::Corrupt
    } else {
        LegacyUploadStorageFailureV1::Unavailable
    }
}

fn request_digest<T: Serialize>(value: &T) -> Result<String, LegacyUploadStorageFailureV1> {
    serde_json::to_vec(value)
        .map(|bytes| digest(&bytes))
        .map_err(|_| LegacyUploadStorageFailureV1::Invalid)
}

fn digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("digest write");
    }
    output
}

fn random_cap_nanoid() -> Result<String, LegacyUploadStorageFailureV1> {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut random = [0_u8; 15];
    getrandom::fill(&mut random).map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
    Ok(random
        .into_iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect())
}

fn valid_phase(value: &str) -> bool {
    matches!(
        value,
        "uploading" | "processing" | "generating_thumbnail" | "complete" | "error"
    )
}
fn valid_number(value: f64) -> bool {
    value.is_finite()
}
fn valid_safe_integer(value: i64) -> bool {
    (0..=MAX_SAFE_INTEGER).contains(&value)
}
fn valid_uuid(value: &str) -> bool {
    value.len() == 36 && Uuid::parse_str(value).is_ok()
}
fn valid_idempotency(value: &str) -> bool {
    (1..=255).contains(&value.len()) && !value.chars().any(char::is_control)
}
fn valid_object_prefix(value: &str, legacy_video_id: &str) -> bool {
    valid_object_key(&format!("{value}x")) && value.ends_with(&format!("/{legacy_video_id}/"))
}
fn valid_object_key(value: &str) -> bool {
    (33..=512).contains(&value.len())
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.contains(['?', '#', '%'])
        && !value.chars().any(char::is_control)
}
fn valid_password_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}
fn email_restriction_allows(email: Option<&str>, restriction: &str) -> bool {
    let entries = restriction
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    entries.is_empty()
        || email.is_some_and(|email| {
            let email = email.to_lowercase();
            entries.into_iter().any(|entry| {
                let entry = entry.to_lowercase();
                if entry.contains('@') {
                    email == entry
                } else {
                    email.ends_with(&format!("@{entry}"))
                }
            })
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
    fn object_keys_remain_actor_video_scoped() {
        assert!(valid_object_prefix(
            "0123456789abcdf/0123456789abcde/",
            "0123456789abcde"
        ));
        assert!(!valid_object_key(
            "0123456789abcdf/0123456789abcde/../secret"
        ));
        assert!(email_restriction_allows(
            Some("Person@Example.com"),
            "other.test, example.com"
        ));
        assert!(email_restriction_allows(None, " , "));
        assert!(!email_restriction_allows(
            Some("person@badexample.com"),
            "example.com"
        ));
    }
}
