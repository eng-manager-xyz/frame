//! D1/R2 authority adapter for Cap's retained storage and upload routes.
//!
//! The adapter intentionally separates durable intent from provider effects.
//! Multipart completion and recording finalization persist their complete
//! evidence package, then fail closed at `provider_execution` until the
//! provider worker is independently admitted.

use std::{collections::BTreeMap, fmt::Write as _};

use frame_application::{
    LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS, LEGACY_CORE_STORAGE_FREE_PLAN_COMPLETION_SECONDS,
    LEGACY_CORE_STORAGE_MULTIPART_TTL_MS, LEGACY_MULTIPART_ABORT_OPERATION_ID,
    LEGACY_MULTIPART_COMPLETE_OPERATION_ID, LEGACY_MULTIPART_INITIATE_OPERATION_ID,
    LEGACY_MULTIPART_PRESIGN_PART_OPERATION_ID, LEGACY_RECORDING_COMPLETE_OPERATION_ID,
    LEGACY_SIGNED_UPLOAD_BATCH_OPERATION_ID, LEGACY_SIGNED_UPLOAD_OPERATION_ID,
    LegacyMultipartAbortInputV1, LegacyMultipartCompleteInputV1, LegacyMultipartInitiateInputV1,
    LegacyMultipartPresignPartInputV1, LegacyNumberInputV1, LegacyRecordingCompleteInputV1,
    LegacySignedMethodV1, LegacySignedUploadBatchInputV1, LegacySignedUploadInputV1,
    legacy_content_type_for_subpath,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{
    Bucket, D1Database, D1PreparedStatement, D1Result, HttpMetadata, send::IntoSendFuture,
};

use crate::r2_direct_upload::R2DirectPutSigner;

const OWNER_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_core_storage/owner_authority.sql");
const READ_AUTHORITY_SQL: &str = include_str!("../queries/legacy_core_storage/read_authority.sql");
const OPERATION_REPLAY_SQL: &str =
    include_str!("../queries/legacy_core_storage/operation_replay.sql");
const OPERATION_BY_ID_SQL: &str =
    include_str!("../queries/legacy_core_storage/operation_by_id.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_core_storage/operation_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_core_storage/operation_complete.sql");
const OPERATION_EFFECT_PENDING_SQL: &str =
    include_str!("../queries/legacy_core_storage/operation_effect_pending.sql");
const MULTIPART_INSERT_SQL: &str =
    include_str!("../queries/legacy_core_storage/multipart_insert.sql");
const MULTIPART_SELECT_SQL: &str =
    include_str!("../queries/legacy_core_storage/multipart_select.sql");
const MULTIPART_MARK_ABORT_SQL: &str =
    include_str!("../queries/legacy_core_storage/multipart_mark_abort.sql");
const MULTIPART_FINISH_ABORT_SQL: &str =
    include_str!("../queries/legacy_core_storage/multipart_finish_abort.sql");
const MULTIPART_MARK_COMPLETION_SQL: &str =
    include_str!("../queries/legacy_core_storage/multipart_mark_completion.sql");
const MULTIPART_PART_INSERT_SQL: &str =
    include_str!("../queries/legacy_core_storage/multipart_part_insert.sql");
const OBJECT_INTENT_INSERT_SQL: &str =
    include_str!("../queries/legacy_core_storage/object_intent_insert.sql");
const FINALIZE_INTENT_INSERT_SQL: &str =
    include_str!("../queries/legacy_core_storage/finalize_intent_insert.sql");
const FINALIZE_INTENT_SELECT_SQL: &str =
    include_str!("../queries/legacy_core_storage/finalize_intent_select.sql");
const VIDEO_METADATA_UPDATE_SQL: &str =
    include_str!("../queries/legacy_core_storage/video_metadata_update.sql");
const ASSERT_CHANGES_SQL: &str = include_str!("../queries/legacy_core_storage/assert_changes.sql");
const ASSERT_CLEANUP_SQL: &str = include_str!("../queries/legacy_core_storage/assert_cleanup.sql");

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyCoreStorageFailureV1 {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    RangeNotSatisfiable,
    ProviderGated,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyCoreStorageOwnerAuthorityV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) organization_id: String,
    pub(crate) object_prefix: String,
    pub(crate) source_type: String,
    pub(crate) storage_integration_id: String,
    organization_owner_has_pro_seat: i64,
    supports_single_put: i64,
    supports_multipart: i64,
}

impl LegacyCoreStorageOwnerAuthorityV1 {
    fn valid(&self, actor_id: &str, legacy_video_id: &str) -> bool {
        valid_id(&self.mapped_video_id)
            && valid_id(&self.organization_id)
            && self.owner_id == actor_id
            && self.legacy_video_id == legacy_video_id
            && valid_id(&self.storage_integration_id)
            && matches!(self.organization_owner_has_pro_seat, 0 | 1)
            && valid_object_prefix(&self.object_prefix, legacy_video_id)
            && matches!(
                self.source_type.as_str(),
                "MediaConvert" | "local" | "desktopMP4" | "desktopSegments" | "webMP4"
            )
            && matches!(self.supports_single_put, 0 | 1)
            && matches!(self.supports_multipart, 0 | 1)
    }

    fn object_key(&self, subpath: &str) -> Option<String> {
        valid_subpath(subpath).then(|| format!("{}{subpath}", self.object_prefix))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyCoreStorageReadAuthorityV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) organization_id: String,
    pub(crate) object_prefix: String,
    pub(crate) source_type: String,
    pub(crate) raw_file_key: Option<String>,
    pub(crate) storage_integration_id: String,
    privacy: String,
    legacy_public: i64,
    legacy_password_hash: Option<String>,
    supports_single_put: i64,
    supports_multipart: i64,
}

impl LegacyCoreStorageReadAuthorityV1 {
    fn valid(&self, legacy_video_id: &str) -> bool {
        valid_id(&self.mapped_video_id)
            && valid_id(&self.owner_id)
            && valid_id(&self.organization_id)
            && self.legacy_video_id == legacy_video_id
            && valid_object_prefix(&self.object_prefix, legacy_video_id)
            && valid_id(&self.storage_integration_id)
            && matches!(
                self.source_type.as_str(),
                "MediaConvert" | "local" | "desktopMP4" | "desktopSegments" | "webMP4"
            )
            && self.raw_file_key.as_deref().is_none_or(|key| {
                key.starts_with(&self.object_prefix)
                    && key.len() > self.object_prefix.len()
                    && valid_subpath(&key[self.object_prefix.len()..])
            })
            && matches!(self.privacy.as_str(), "public" | "private" | "organization")
            && matches!(self.legacy_public, 0 | 1)
            && self
                .legacy_password_hash
                .as_deref()
                .is_none_or(|value| value.len() <= 512 && !value.chars().any(char::is_control))
            && self.supports_single_put == 1
            && matches!(self.supports_multipart, 0 | 1)
    }

    pub(crate) fn admits_key(&self, key: &str) -> bool {
        key.starts_with(&self.object_prefix)
            && key.len() > self.object_prefix.len()
            && valid_subpath(&key[self.object_prefix.len()..])
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OperationRowV1 {
    operation_id: String,
    request_digest: String,
    result_binding_json: Option<String>,
    state: String,
    client_idempotency: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct FinalizeIntentRowV1 {
    operation_id: String,
    state: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MultipartRowV1 {
    external_upload_id: String,
    provider_upload_id: String,
    initiate_operation_id: String,
    completion_operation_id: Option<String>,
    abort_operation_id: Option<String>,
    actor_id: String,
    organization_id: String,
    mapped_video_id: String,
    legacy_video_id: String,
    storage_integration_id: String,
    object_prefix: String,
    subpath: String,
    object_key: String,
    content_type: String,
    state: String,
    expected_bytes: Option<i64>,
    parts_digest: Option<String>,
    created_at_ms: i64,
    expires_at_ms: i64,
    terminal_at_ms: Option<i64>,
}

impl MultipartRowV1 {
    fn valid(&self) -> bool {
        valid_uuid(&self.external_upload_id)
            && !self.provider_upload_id.is_empty()
            && self.provider_upload_id.len() <= 1_024
            && valid_uuid(&self.initiate_operation_id)
            && self
                .completion_operation_id
                .as_deref()
                .is_none_or(valid_uuid)
            && self.abort_operation_id.as_deref().is_none_or(valid_uuid)
            && valid_id(&self.actor_id)
            && valid_id(&self.organization_id)
            && valid_id(&self.mapped_video_id)
            && valid_id(&self.storage_integration_id)
            && valid_object_prefix(&self.object_prefix, &self.legacy_video_id)
            && valid_subpath(&self.subpath)
            && self.object_key == format!("{}{}", self.object_prefix, self.subpath)
            && self.content_type.len() <= 127
            && matches!(
                self.state.as_str(),
                "open" | "completion_pending" | "complete" | "abort_pending" | "aborted"
            )
            && self.expected_bytes.is_none_or(valid_positive_safe_integer)
            && self.parts_digest.as_deref().is_none_or(valid_digest)
            && valid_safe_integer(self.created_at_ms)
            && self.expires_at_ms > self.created_at_ms
            && self.expires_at_ms <= MAX_SAFE_INTEGER
            && self.terminal_at_ms.is_none_or(valid_safe_integer)
    }
}

enum OperationClaimV1 {
    New { operation_id: String },
    Replay(OperationRowV1),
}

struct SignedPersistenceV1<'a> {
    operation_id: &'a str,
    request_digest: &'a str,
    authority: &'a LegacyCoreStorageOwnerAuthorityV1,
    object_key: &'a str,
    content_type: &'a str,
    requested_method: &'a str,
    role: &'a str,
    result: &'a Value,
    metadata: (Option<f64>, Option<f64>, Option<f64>, Option<f64>),
    now_ms: i64,
}

pub(crate) struct D1LegacyCoreStorageV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyCoreStorageV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn owner_authority(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
    ) -> Result<LegacyCoreStorageOwnerAuthorityV1, LegacyCoreStorageFailureV1> {
        if !valid_id(actor_id) || !valid_id(legacy_video_id) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let rows = self
            .rows::<LegacyCoreStorageOwnerAuthorityV1>(
                OWNER_AUTHORITY_SQL,
                &[js(actor_id), js(legacy_video_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let authority = rows
            .into_iter()
            .next()
            .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        authority
            .valid(actor_id, legacy_video_id)
            .then_some(authority)
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)
    }

    pub(crate) async fn read_authority(
        &self,
        actor_id: Option<&str>,
        legacy_video_id: &str,
        exact_storage_token: bool,
        now_ms: i64,
    ) -> Result<LegacyCoreStorageReadAuthorityV1, LegacyCoreStorageFailureV1> {
        if actor_id.is_some_and(|value| !valid_id(value))
            || !valid_id(legacy_video_id)
            || !valid_safe_integer(now_ms)
        {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let rows = self
            .rows::<LegacyCoreStorageReadAuthorityV1>(
                READ_AUTHORITY_SQL,
                &[
                    js(legacy_video_id),
                    js_opt(actor_id),
                    number(i64::from(exact_storage_token)),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let authority = rows
            .into_iter()
            .next()
            .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        authority
            .valid(legacy_video_id)
            .then_some(authority)
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn initiate(
        &self,
        actor_id: &str,
        input: &LegacyMultipartInitiateInputV1,
        idempotency_key: Option<&str>,
        bucket: &Bucket,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let target = input
            .target
            .normalized(actor_id, "result.mp4")
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let authority = self.owner_authority(actor_id, &target.video_id).await?;
        if authority.supports_multipart != 1 {
            return Err(LegacyCoreStorageFailureV1::Forbidden);
        }
        let object_key = authority
            .object_key(&target.subpath)
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
        let request_digest = request_digest(input)?;
        let claim = self
            .claim_operation(
                LEGACY_MULTIPART_INITIATE_OPERATION_ID,
                "multipart_initiate",
                actor_id,
                &authority,
                idempotency_key,
                &request_digest,
                now_ms,
            )
            .await?;
        let operation_id = match claim {
            OperationClaimV1::Replay(row) => return replay_complete(&row),
            OperationClaimV1::New { operation_id } => operation_id,
        };
        let metadata = HttpMetadata {
            content_type: Some(input.content_type().into()),
            content_disposition: Some("inline".into()),
            cache_control: Some("public, max-age=31536000, immutable".into()),
            ..HttpMetadata::default()
        };
        let legacy_owner_id = authority
            .object_prefix
            .split('/')
            .next()
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
        let upload = bucket
            .create_multipart_upload(&object_key)
            .http_metadata(metadata)
            .custom_metadata(std::collections::HashMap::from([
                ("frame-operation-id".into(), operation_id.clone()),
                ("frame-actor-id".into(), actor_id.into()),
                ("userId".into(), legacy_owner_id.into()),
                ("frame-source".into(), "cap-multipart-upload".into()),
            ]))
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
        let provider_upload_id = upload.upload_id().into_send().await;
        if provider_upload_id.is_empty() || provider_upload_id.len() > 1_024 {
            let _ = upload.abort().into_send().await;
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let external_upload_id = Uuid::now_v7().to_string();
        let expires_at_ms = now_ms
            .checked_add(LEGACY_CORE_STORAGE_MULTIPART_TTL_MS)
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let result = json!({"uploadId": external_upload_id, "provider": "s3"});
        let result_json = compact_json(&result)?;
        let statements = vec![
            self.statement(
                MULTIPART_INSERT_SQL,
                &[
                    js(&external_upload_id),
                    js(&provider_upload_id),
                    js(&operation_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&authority.mapped_video_id),
                    js(&authority.legacy_video_id),
                    js(&authority.storage_integration_id),
                    js(&authority.object_prefix),
                    js(&target.subpath),
                    js(&object_key),
                    js(input.content_type()),
                    number(now_ms),
                    number(expires_at_ms),
                ],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[
                    js(&operation_id),
                    js(&request_digest),
                    js(&result_json),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("terminal"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(&operation_id)])?,
        ];
        if let Err(failure) = self.batch(statements).await {
            let _ = upload.abort().into_send().await;
            return Err(failure);
        }
        Ok(result)
    }

    pub(crate) async fn presign_part(
        &self,
        actor_id: &str,
        input: &LegacyMultipartPresignPartInputV1,
        idempotency_key: Option<&str>,
        signer: &R2DirectPutSigner,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let target = input
            .target
            .normalized(actor_id, "result.mp4")
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let authority = self.owner_authority(actor_id, &target.video_id).await?;
        let session = self.multipart(&input.upload_id, actor_id).await?;
        bind_session(&session, &authority, &target.subpath, now_ms, "open")?;
        let request_digest = request_digest(input)?;
        let claim = self
            .claim_operation(
                LEGACY_MULTIPART_PRESIGN_PART_OPERATION_ID,
                "multipart_presign_part",
                actor_id,
                &authority,
                idempotency_key,
                &request_digest,
                now_ms,
            )
            .await?;
        let operation_id = match claim {
            OperationClaimV1::Replay(row) => return replay_complete(&row),
            OperationClaimV1::New { operation_id } => operation_id,
        };
        let capability = signer
            .sign_legacy_upload_part(
                &session.object_key,
                &session.provider_upload_id,
                input.part_number,
                input.md5_sum.as_deref(),
                u64::try_from(now_ms).map_err(|_| LegacyCoreStorageFailureV1::Invalid)?,
                LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS,
            )
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
        let result = json!({"presignedUrl": capability.url, "provider": "s3"});
        self.complete_operation(&operation_id, &request_digest, &result, now_ms)
            .await?;
        Ok(result)
    }

    pub(crate) async fn abort(
        &self,
        actor_id: &str,
        input: &LegacyMultipartAbortInputV1,
        idempotency_key: Option<&str>,
        bucket: &Bucket,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let target = input
            .target
            .normalized(actor_id, "result.mp4")
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let authority = self.owner_authority(actor_id, &target.video_id).await?;
        let session = self.multipart(&input.upload_id, actor_id).await?;
        ensure_session_binding(&session, &authority, &target.subpath)?;
        if session.state == "aborted" {
            return Ok(json!({"success": true}));
        }
        if !matches!(session.state.as_str(), "open" | "abort_pending") {
            return Err(LegacyCoreStorageFailureV1::Conflict);
        }
        let request_digest = request_digest(input)?;
        let operation_id = if session.state == "abort_pending" {
            let operation_id = session
                .abort_operation_id
                .as_deref()
                .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
            let row = self
                .operation_by_id(operation_id, actor_id, LEGACY_MULTIPART_ABORT_OPERATION_ID)
                .await?;
            if row.request_digest != request_digest {
                return Err(LegacyCoreStorageFailureV1::Conflict);
            }
            match row.state.as_str() {
                "effect_pending" => row.operation_id,
                "complete" => return replay_complete(&row),
                _ => return Err(LegacyCoreStorageFailureV1::Corrupt),
            }
        } else {
            if session.expires_at_ms <= now_ms {
                return Err(LegacyCoreStorageFailureV1::Conflict);
            }
            let claim = self
                .claim_operation(
                    LEGACY_MULTIPART_ABORT_OPERATION_ID,
                    "multipart_abort",
                    actor_id,
                    &authority,
                    idempotency_key,
                    &request_digest,
                    now_ms,
                )
                .await?;
            match claim {
                OperationClaimV1::Replay(row) if row.state == "complete" => {
                    return replay_complete(&row);
                }
                OperationClaimV1::Replay(row) if row.state == "effect_pending" => row.operation_id,
                OperationClaimV1::Replay(_) => {
                    return Err(LegacyCoreStorageFailureV1::Unavailable);
                }
                OperationClaimV1::New { operation_id } => {
                    let pending = json!({"providerEffect": "abort"});
                    self.batch(vec![
                        self.statement(
                            MULTIPART_MARK_ABORT_SQL,
                            &[
                                js(&session.external_upload_id),
                                js(actor_id),
                                js(&operation_id),
                                number(now_ms),
                            ],
                        )?,
                        self.statement(
                            ASSERT_CHANGES_SQL,
                            &[js(&operation_id), js("multipart_binding"), number(1)],
                        )?,
                        self.statement(
                            OPERATION_EFFECT_PENDING_SQL,
                            &[
                                js(&operation_id),
                                js(&request_digest),
                                js(&compact_json(&pending)?),
                            ],
                        )?,
                        self.statement(
                            ASSERT_CHANGES_SQL,
                            &[js(&operation_id), js("provider_pending"), number(1)],
                        )?,
                        self.statement(ASSERT_CLEANUP_SQL, &[js(&operation_id)])?,
                    ])
                    .await?;
                    operation_id
                }
            }
        };
        let upload = bucket
            .resume_multipart_upload(&session.object_key, &session.provider_upload_id)
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
        upload
            .abort()
            .into_send()
            .await
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
        let result = json!({"success": true});
        let result_json = compact_json(&result)?;
        self.batch(vec![
            self.statement(
                MULTIPART_FINISH_ABORT_SQL,
                &[
                    js(&session.external_upload_id),
                    js(actor_id),
                    js(&operation_id),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("terminal"), number(1)],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[
                    js(&operation_id),
                    js(&request_digest),
                    js(&result_json),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("claim"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(&operation_id)])?,
        ])
        .await?;
        Ok(result)
    }

    pub(crate) async fn complete(
        &self,
        actor_id: &str,
        input: &LegacyMultipartCompleteInputV1,
        idempotency_key: Option<&str>,
        bucket: &Bucket,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let target = input
            .target
            .normalized(actor_id, "result.mp4")
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let authority = self.owner_authority(actor_id, &target.video_id).await?;
        let session = self.multipart(&input.upload_id, actor_id).await?;
        ensure_session_binding(&session, &authority, &target.subpath)?;
        let request_digest = request_digest(input)?;
        if matches!(session.state.as_str(), "completion_pending" | "complete") {
            let operation_id = session
                .completion_operation_id
                .as_deref()
                .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
            let row = self
                .operation_by_id(
                    operation_id,
                    actor_id,
                    LEGACY_MULTIPART_COMPLETE_OPERATION_ID,
                )
                .await?;
            if row.request_digest != request_digest {
                return Err(LegacyCoreStorageFailureV1::Conflict);
            }
            return match row.state.as_str() {
                "effect_pending" if session.state == "completion_pending" => {
                    Err(LegacyCoreStorageFailureV1::ProviderGated)
                }
                "complete" if session.state == "complete" => replay_complete(&row),
                _ => Err(LegacyCoreStorageFailureV1::Corrupt),
            };
        }
        if session.state != "open" {
            return Err(LegacyCoreStorageFailureV1::Conflict);
        }
        if session.expires_at_ms <= now_ms {
            return Err(LegacyCoreStorageFailureV1::Conflict);
        }
        let reported_duration = input
            .duration_in_secs
            .as_ref()
            .and_then(LegacyNumberInputV1::finite);
        let raw_recorder_upload =
            input.target.file_key.is_none() && target.subpath.starts_with("raw-upload.");
        if !free_plan_completion_allowed(
            authority.organization_owner_has_pro_seat == 1,
            raw_recorder_upload,
            reported_duration,
        ) {
            let cleanup = LegacyMultipartAbortInputV1 {
                upload_id: input.upload_id.clone(),
                target: input.target.clone(),
            };
            let _ = self.abort(actor_id, &cleanup, None, bucket, now_ms).await;
            return Err(LegacyCoreStorageFailureV1::Forbidden);
        }
        let claim = self
            .claim_operation(
                LEGACY_MULTIPART_COMPLETE_OPERATION_ID,
                "multipart_complete",
                actor_id,
                &authority,
                idempotency_key,
                &request_digest,
                now_ms,
            )
            .await?;
        let operation_id = match claim {
            OperationClaimV1::Replay(row) if row.state == "effect_pending" => {
                return Err(LegacyCoreStorageFailureV1::ProviderGated);
            }
            OperationClaimV1::Replay(row) => return replay_complete(&row),
            OperationClaimV1::New { operation_id } => operation_id,
        };
        let total = input
            .total_size()
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let parts_digest = sha256_domain(
            "parts",
            &serde_json::to_vec(&input.parts).map_err(|_| LegacyCoreStorageFailureV1::Invalid)?,
        );
        let pending = json!({
            "providerGate": "provider_execution",
            "uploadId": session.external_upload_id,
            "objectKeyDigest": sha256_domain("object-key", session.object_key.as_bytes()),
            "partsDigest": parts_digest,
            "expectedBytes": total,
        });
        let mut statements = Vec::with_capacity(input.parts.len() + 6);
        for part in &input.parts {
            statements.push(
                self.statement(
                    MULTIPART_PART_INSERT_SQL,
                    &[
                        js(&session.external_upload_id),
                        number(i64::from(part.part_number)),
                        js(&part.etag),
                        number(
                            i64::try_from(part.size)
                                .map_err(|_| LegacyCoreStorageFailureV1::Invalid)?,
                        ),
                        js(&operation_id),
                        number(now_ms),
                    ],
                )?,
            );
        }
        statements.extend([
            self.statement(
                MULTIPART_MARK_COMPLETION_SQL,
                &[
                    js(&session.external_upload_id),
                    js(actor_id),
                    js(&operation_id),
                    number(total),
                    js(&parts_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("multipart_binding"), number(1)],
            )?,
            self.statement(
                OPERATION_EFFECT_PENDING_SQL,
                &[
                    js(&operation_id),
                    js(&request_digest),
                    js(&compact_json(&pending)?),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("provider_pending"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(&operation_id)])?,
        ]);
        self.batch(statements).await?;
        Err(LegacyCoreStorageFailureV1::ProviderGated)
    }

    pub(crate) async fn signed(
        &self,
        actor_id: &str,
        input: &LegacySignedUploadInputV1,
        idempotency_key: Option<&str>,
        signer: &R2DirectPutSigner,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let target = input
            .target
            .normalized(actor_id, "result.mp4")
            .ok_or(LegacyCoreStorageFailureV1::Invalid)?;
        let authority = self.owner_authority(actor_id, &target.video_id).await?;
        if authority.supports_single_put != 1 {
            return Err(LegacyCoreStorageFailureV1::Forbidden);
        }
        let object_key = authority
            .object_key(&target.subpath)
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
        let request_digest = request_digest(input)?;
        let claim = self
            .claim_operation(
                LEGACY_SIGNED_UPLOAD_OPERATION_ID,
                "signed",
                actor_id,
                &authority,
                idempotency_key,
                &request_digest,
                now_ms,
            )
            .await?;
        let operation_id = match claim {
            OperationClaimV1::Replay(row) => return replay_complete(&row),
            OperationClaimV1::New { operation_id } => operation_id,
        };
        let content_type = match legacy_content_type_for_subpath(&target.subpath) {
            "application/octet-stream" => "video/mp2t",
            value => value,
        };
        let legacy_owner_id = authority
            .object_prefix
            .split('/')
            .next()
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
        let headers = BTreeMap::from([
            ("content-type".into(), content_type.into()),
            ("x-amz-meta-userid".into(), legacy_owner_id.into()),
            (
                "x-amz-meta-duration".into(),
                input
                    .duration_in_secs
                    .as_ref()
                    .and_then(LegacyNumberInputV1::finite)
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
        ]);
        let capability = signer
            .sign_legacy_storage_put(
                &object_key,
                &headers,
                u64::try_from(now_ms).map_err(|_| LegacyCoreStorageFailureV1::Invalid)?,
                LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS,
            )
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
        let response_headers = capability
            .required_headers
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let result = json!({
            "presignedPutData": {
                "url": capability.url,
                "fields": {},
                "headers": response_headers,
                "type": "put"
            }
        });
        let requested_method = match input.method {
            LegacySignedMethodV1::Post => "post",
            LegacySignedMethodV1::Put => "put",
        };
        self.persist_signed(SignedPersistenceV1 {
            operation_id: &operation_id,
            request_digest: &request_digest,
            authority: &authority,
            object_key: &object_key,
            content_type,
            requested_method,
            role: object_role(&target.subpath),
            result: &result,
            metadata: metadata_values(input),
            now_ms,
        })
        .await?;
        Ok(result)
    }

    pub(crate) async fn signed_batch(
        &self,
        actor_id: &str,
        input: &LegacySignedUploadBatchInputV1,
        idempotency_key: Option<&str>,
        signer: &R2DirectPutSigner,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let authority = self.owner_authority(actor_id, &input.video_id).await?;
        if authority.supports_single_put != 1 {
            return Err(LegacyCoreStorageFailureV1::Forbidden);
        }
        let request_digest = request_digest(input)?;
        let claim = self
            .claim_operation(
                LEGACY_SIGNED_UPLOAD_BATCH_OPERATION_ID,
                "signed_batch",
                actor_id,
                &authority,
                idempotency_key,
                &request_digest,
                now_ms,
            )
            .await?;
        let operation_id = match claim {
            OperationClaimV1::Replay(row) => return replay_complete(&row),
            OperationClaimV1::New { operation_id } => operation_id,
        };
        let mut uploads = serde_json::Map::new();
        let mut urls = serde_json::Map::new();
        let mut persisted = Vec::with_capacity(input.subpaths.len());
        for subpath in &input.subpaths {
            let object_key = authority
                .object_key(subpath)
                .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
            let content_type = legacy_content_type_for_subpath(subpath);
            let headers = BTreeMap::from([("content-type".into(), content_type.into())]);
            let capability = signer
                .sign_legacy_storage_put(
                    &object_key,
                    &headers,
                    u64::try_from(now_ms).map_err(|_| LegacyCoreStorageFailureV1::Invalid)?,
                    LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS,
                )
                .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
            let url = capability.url;
            uploads.insert(
                subpath.clone(),
                json!({
                    "url": url,
                    "headers": capability.required_headers.into_iter().collect::<BTreeMap<_, _>>(),
                    "type": "put"
                }),
            );
            urls.insert(subpath.clone(), Value::String(url));
            persisted.push((object_key, content_type, object_role(subpath)));
        }
        let result = Value::Object(serde_json::Map::from_iter([
            ("uploads".into(), Value::Object(uploads)),
            ("urls".into(), Value::Object(urls)),
        ]));
        let mut statements = Vec::with_capacity(persisted.len() * 2 + 3);
        for (object_key, content_type, role) in persisted {
            statements.push(self.statement(
                OBJECT_INTENT_INSERT_SQL,
                &[
                    js(&Uuid::now_v7().to_string()),
                    js(&object_key),
                    js(&operation_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&authority.mapped_video_id),
                    js(&authority.legacy_video_id),
                    js(&authority.storage_integration_id),
                    js(content_type),
                    js(role),
                    js("put"),
                    number(now_ms),
                ],
            )?);
        }
        statements.extend([
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[
                    js(&operation_id),
                    js(&request_digest),
                    js(&compact_json(&result)?),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("terminal"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(&operation_id)])?,
        ]);
        self.batch(statements).await?;
        Ok(result)
    }

    pub(crate) async fn recording_complete(
        &self,
        actor_id: &str,
        input: &LegacyRecordingCompleteInputV1,
        idempotency_key: Option<&str>,
        now_ms: i64,
    ) -> Result<Value, LegacyCoreStorageFailureV1> {
        if !input.valid() || !valid_safe_integer(now_ms) {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        let authority = self.owner_authority(actor_id, &input.video_id).await?;
        let request_digest = request_digest(input)?;
        if authority.source_type == "desktopMP4" {
            let claim = self
                .claim_operation(
                    LEGACY_RECORDING_COMPLETE_OPERATION_ID,
                    "recording_complete",
                    actor_id,
                    &authority,
                    idempotency_key,
                    &request_digest,
                    now_ms,
                )
                .await?;
            let operation_id = match claim {
                OperationClaimV1::Replay(row) => return replay_complete(&row),
                OperationClaimV1::New { operation_id } => operation_id,
            };
            let result = json!({"success": true, "status": "already-complete"});
            self.complete_operation(&operation_id, &request_digest, &result, now_ms)
                .await?;
            return Ok(result);
        }
        if authority.source_type != "desktopSegments" {
            return Err(LegacyCoreStorageFailureV1::Invalid);
        }
        if let Some(intent) = self.finalize_intent(&authority).await? {
            let row = self
                .operation_by_id(
                    &intent.operation_id,
                    actor_id,
                    LEGACY_RECORDING_COMPLETE_OPERATION_ID,
                )
                .await?;
            if row.request_digest != request_digest {
                return Err(LegacyCoreStorageFailureV1::Conflict);
            }
            return match (intent.state.as_str(), row.state.as_str()) {
                ("provider_pending", "effect_pending") => {
                    Err(LegacyCoreStorageFailureV1::ProviderGated)
                }
                ("complete", "complete") => replay_complete(&row),
                ("failed", "complete") => Err(LegacyCoreStorageFailureV1::Unavailable),
                _ => Err(LegacyCoreStorageFailureV1::Corrupt),
            };
        }
        let claim = self
            .claim_operation(
                LEGACY_RECORDING_COMPLETE_OPERATION_ID,
                "recording_complete",
                actor_id,
                &authority,
                idempotency_key,
                &request_digest,
                now_ms,
            )
            .await?;
        let operation_id = match claim {
            OperationClaimV1::Replay(row) if row.state == "effect_pending" => {
                return Err(LegacyCoreStorageFailureV1::ProviderGated);
            }
            OperationClaimV1::Replay(row) => return replay_complete(&row),
            OperationClaimV1::New { operation_id } => operation_id,
        };
        let pending =
            json!({"providerGate": "provider_execution", "workflow": "recording-complete"});
        self.batch(vec![
            self.statement(
                FINALIZE_INTENT_INSERT_SQL,
                &[
                    js(&authority.mapped_video_id),
                    js(&authority.legacy_video_id),
                    js(&operation_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("provider_pending"), number(1)],
            )?,
            self.statement(
                OPERATION_EFFECT_PENDING_SQL,
                &[
                    js(&operation_id),
                    js(&request_digest),
                    js(&compact_json(&pending)?),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(&operation_id), js("claim"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(&operation_id)])?,
        ])
        .await?;
        Err(LegacyCoreStorageFailureV1::ProviderGated)
    }

    async fn persist_signed(
        &self,
        persistence: SignedPersistenceV1<'_>,
    ) -> Result<(), LegacyCoreStorageFailureV1> {
        let SignedPersistenceV1 {
            operation_id,
            request_digest,
            authority,
            object_key,
            content_type,
            requested_method,
            role,
            result,
            metadata: (duration, width, height, fps),
            now_ms,
        } = persistence;
        let duration_ms = duration
            .map(|value| value * 1_000.0)
            .filter(|value| (0.0..=MAX_SAFE_INTEGER as f64).contains(value))
            .map(|value| value.round() as i64);
        self.batch(vec![
            self.statement(
                OBJECT_INTENT_INSERT_SQL,
                &[
                    js(&Uuid::now_v7().to_string()),
                    js(object_key),
                    js(operation_id),
                    js(&authority.owner_id),
                    js(&authority.organization_id),
                    js(&authority.mapped_video_id),
                    js(&authority.legacy_video_id),
                    js(&authority.storage_integration_id),
                    js(content_type),
                    js(role),
                    js(requested_method),
                    number(now_ms),
                ],
            )?,
            self.statement(
                VIDEO_METADATA_UPDATE_SQL,
                &[
                    js(&authority.mapped_video_id),
                    js(&authority.owner_id),
                    float_opt(duration),
                    number_opt(duration_ms),
                    float_opt(width),
                    float_opt(height),
                    float_opt(fps),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(operation_id), js("authority"), number(1)],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[
                    js(operation_id),
                    js(request_digest),
                    js(&compact_json(result)?),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(operation_id), js("terminal"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(operation_id)])?,
        ])
        .await
    }

    async fn complete_operation(
        &self,
        operation_id: &str,
        request_digest: &str,
        result: &Value,
        now_ms: i64,
    ) -> Result<(), LegacyCoreStorageFailureV1> {
        self.batch(vec![
            self.statement(
                OPERATION_COMPLETE_SQL,
                &[
                    js(operation_id),
                    js(request_digest),
                    js(&compact_json(result)?),
                    number(now_ms),
                ],
            )?,
            self.statement(
                ASSERT_CHANGES_SQL,
                &[js(operation_id), js("terminal"), number(1)],
            )?,
            self.statement(ASSERT_CLEANUP_SQL, &[js(operation_id)])?,
        ])
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn claim_operation(
        &self,
        source_operation_id: &str,
        operation_kind: &str,
        actor_id: &str,
        authority: &LegacyCoreStorageOwnerAuthorityV1,
        idempotency_key: Option<&str>,
        request_digest: &str,
        now_ms: i64,
    ) -> Result<OperationClaimV1, LegacyCoreStorageFailureV1> {
        let (idempotency_digest, client_idempotency) = match idempotency_key {
            Some(value) if valid_idempotency_key(value) => {
                (sha256_domain("idempotency", value.as_bytes()), true)
            }
            Some(_) => return Err(LegacyCoreStorageFailureV1::Invalid),
            None => (
                sha256_domain("generated-idempotency", Uuid::now_v7().as_bytes()),
                false,
            ),
        };
        if client_idempotency
            && let Some(row) = self
                .operation_replay(source_operation_id, actor_id, &idempotency_digest)
                .await?
        {
            return exact_client_replay(row, request_digest);
        }
        let operation_id = Uuid::now_v7().to_string();
        let inserted = self
            .execute(
                OPERATION_INSERT_SQL,
                &[
                    js(&operation_id),
                    js(source_operation_id),
                    js(operation_kind),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&authority.mapped_video_id),
                    js(&authority.legacy_video_id),
                    js(&idempotency_digest),
                    number(i64::from(client_idempotency)),
                    js(request_digest),
                    JsValue::NULL,
                    number(now_ms),
                ],
            )
            .await;
        if let Err(failure) = inserted {
            if client_idempotency && failure == LegacyCoreStorageFailureV1::Conflict {
                let row = self
                    .operation_replay(source_operation_id, actor_id, &idempotency_digest)
                    .await?
                    .ok_or(LegacyCoreStorageFailureV1::Conflict)?;
                return exact_client_replay(row, request_digest);
            }
            return Err(failure);
        }
        Ok(OperationClaimV1::New { operation_id })
    }

    async fn operation_replay(
        &self,
        source_operation_id: &str,
        actor_id: &str,
        idempotency_digest: &str,
    ) -> Result<Option<OperationRowV1>, LegacyCoreStorageFailureV1> {
        let mut rows = self
            .rows::<OperationRowV1>(
                OPERATION_REPLAY_SQL,
                &[
                    js(source_operation_id),
                    js(actor_id),
                    js(idempotency_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let row = rows.pop();
        if row.as_ref().is_some_and(|row| {
            !valid_uuid(&row.operation_id)
                || !valid_digest(&row.request_digest)
                || !matches!(
                    row.state.as_str(),
                    "claimed" | "effect_pending" | "complete"
                )
                || !matches!(row.client_idempotency, 0 | 1)
        }) {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        Ok(row)
    }

    async fn operation_by_id(
        &self,
        operation_id: &str,
        actor_id: &str,
        source_operation_id: &str,
    ) -> Result<OperationRowV1, LegacyCoreStorageFailureV1> {
        let mut rows = self
            .rows::<OperationRowV1>(
                OPERATION_BY_ID_SQL,
                &[js(operation_id), js(actor_id), js(source_operation_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let row = rows.pop().ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        if !valid_uuid(&row.operation_id)
            || !valid_digest(&row.request_digest)
            || !matches!(
                row.state.as_str(),
                "claimed" | "effect_pending" | "complete"
            )
            || !matches!(row.client_idempotency, 0 | 1)
        {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        Ok(row)
    }

    async fn finalize_intent(
        &self,
        authority: &LegacyCoreStorageOwnerAuthorityV1,
    ) -> Result<Option<FinalizeIntentRowV1>, LegacyCoreStorageFailureV1> {
        let mut rows = self
            .rows::<FinalizeIntentRowV1>(
                FINALIZE_INTENT_SELECT_SQL,
                &[
                    js(&authority.mapped_video_id),
                    js(&authority.legacy_video_id),
                    js(&authority.owner_id),
                    js(&authority.organization_id),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let row = rows.pop();
        if row.as_ref().is_some_and(|row| {
            !valid_uuid(&row.operation_id)
                || !matches!(
                    row.state.as_str(),
                    "provider_pending" | "complete" | "failed"
                )
        }) {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        Ok(row)
    }

    async fn multipart(
        &self,
        external_upload_id: &str,
        actor_id: &str,
    ) -> Result<MultipartRowV1, LegacyCoreStorageFailureV1> {
        let rows = self
            .rows::<MultipartRowV1>(
                MULTIPART_SELECT_SQL,
                &[js(external_upload_id), js(actor_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let row = rows
            .into_iter()
            .next()
            .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        row.valid()
            .then_some(row)
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<D1PreparedStatement, LegacyCoreStorageFailureV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)
    }

    async fn execute(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<(), LegacyCoreStorageFailureV1> {
        let result = self
            .statement(sql, bindings)?
            .run()
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if result.success() {
            Ok(())
        } else {
            Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ))
        }
    }

    async fn rows<T>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<Vec<T>, LegacyCoreStorageFailureV1>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyCoreStorageFailureV1::Corrupt)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyCoreStorageFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyCoreStorageFailureV1::Unavailable);
        }
        if let Some(result) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }
}

fn bind_session(
    session: &MultipartRowV1,
    authority: &LegacyCoreStorageOwnerAuthorityV1,
    subpath: &str,
    now_ms: i64,
    expected_state: &str,
) -> Result<(), LegacyCoreStorageFailureV1> {
    ensure_session_binding(session, authority, subpath)?;
    if session.state != expected_state {
        return Err(LegacyCoreStorageFailureV1::NotFound);
    }
    if session.expires_at_ms <= now_ms {
        return Err(LegacyCoreStorageFailureV1::Conflict);
    }
    Ok(())
}

fn ensure_session_binding(
    session: &MultipartRowV1,
    authority: &LegacyCoreStorageOwnerAuthorityV1,
    subpath: &str,
) -> Result<(), LegacyCoreStorageFailureV1> {
    if session.actor_id != authority.owner_id
        || session.organization_id != authority.organization_id
        || session.mapped_video_id != authority.mapped_video_id
        || session.legacy_video_id != authority.legacy_video_id
        || session.storage_integration_id != authority.storage_integration_id
        || session.object_prefix != authority.object_prefix
        || session.subpath != subpath
    {
        return Err(LegacyCoreStorageFailureV1::NotFound);
    }
    Ok(())
}

fn replay_complete(row: &OperationRowV1) -> Result<Value, LegacyCoreStorageFailureV1> {
    if row.state != "complete" {
        return Err(if row.state == "effect_pending" {
            LegacyCoreStorageFailureV1::ProviderGated
        } else {
            LegacyCoreStorageFailureV1::Unavailable
        });
    }
    let json = row
        .result_binding_json
        .as_deref()
        .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
    serde_json::from_str(json).map_err(|_| LegacyCoreStorageFailureV1::Corrupt)
}

fn exact_client_replay(
    row: OperationRowV1,
    request_digest: &str,
) -> Result<OperationClaimV1, LegacyCoreStorageFailureV1> {
    if row.request_digest != request_digest || row.client_idempotency != 1 {
        return Err(LegacyCoreStorageFailureV1::Conflict);
    }
    Ok(OperationClaimV1::Replay(row))
}

fn free_plan_completion_allowed(
    organization_owner_is_pro: bool,
    raw_recorder_upload: bool,
    reported_duration: Option<f64>,
) -> bool {
    organization_owner_is_pro
        || ((!raw_recorder_upload || reported_duration.is_some())
            && reported_duration.is_none_or(|duration| {
                duration <= LEGACY_CORE_STORAGE_FREE_PLAN_COMPLETION_SECONDS
            }))
}

fn metadata_values(
    input: &LegacySignedUploadInputV1,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    (
        input
            .duration_in_secs
            .as_ref()
            .and_then(LegacyNumberInputV1::finite),
        input.width.as_ref().and_then(LegacyNumberInputV1::finite),
        input.height.as_ref().and_then(LegacyNumberInputV1::finite),
        input.fps.as_ref().and_then(LegacyNumberInputV1::finite),
    )
}

fn object_role(subpath: &str) -> &'static str {
    if subpath.contains("segment") || subpath.ends_with(".m4s") || subpath.ends_with(".ts") {
        "segment"
    } else if subpath.ends_with(".m3u8") || subpath.ends_with("manifest.json") {
        "manifest"
    } else if subpath.ends_with(".jpg") || subpath.ends_with(".jpeg") || subpath.ends_with(".png") {
        "thumbnail"
    } else if subpath.ends_with(".aac") || subpath.ends_with(".mp3") || subpath.ends_with(".webm") {
        "audio"
    } else if subpath.contains("preview") {
        "preview"
    } else {
        "source"
    }
}

fn request_digest<T: Serialize>(value: &T) -> Result<String, LegacyCoreStorageFailureV1> {
    let bytes = serde_json::to_vec(value).map_err(|_| LegacyCoreStorageFailureV1::Invalid)?;
    Ok(sha256_domain("request", &bytes))
}

fn compact_json(value: &Value) -> Result<String, LegacyCoreStorageFailureV1> {
    let json = serde_json::to_string(value).map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
    (json.len() <= 1_048_576)
        .then_some(json)
        .ok_or(LegacyCoreStorageFailureV1::Invalid)
}

fn sha256_domain(domain: &str, value: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame.legacy-core-storage.v1\0");
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(value);
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

fn map_d1_message(message: &str) -> LegacyCoreStorageFailureV1 {
    if message.contains("UNIQUE constraint failed") {
        LegacyCoreStorageFailureV1::Conflict
    } else if message.contains("frame_legacy_core_storage_assertion_v1") {
        LegacyCoreStorageFailureV1::Forbidden
    } else if message.contains("frame_legacy_core_storage_")
        || message.contains("frame_business_")
        || message.contains("foreign key")
        || message.contains("CHECK constraint")
    {
        LegacyCoreStorageFailureV1::Corrupt
    } else {
        LegacyCoreStorageFailureV1::Unavailable
    }
}

fn valid_object_prefix(value: &str, legacy_video_id: &str) -> bool {
    value.ends_with(&format!("/{legacy_video_id}/"))
        && value.len() <= 1_279
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains(['\\', '?', '#', '%'])
        && value.split('/').filter(|part| !part.is_empty()).count() == 2
}

fn valid_subpath(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 768
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.contains(['\\', '?', '#', '%'])
        && !value.chars().any(char::is_control)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1_020
        && !value.contains(['/', '\\', '?', '#', '%'])
        && !value.contains("..")
        && !value.chars().any(char::is_control)
}

fn valid_idempotency_key(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'\\')
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

const fn valid_safe_integer(value: i64) -> bool {
    value >= 0 && value <= MAX_SAFE_INTEGER
}

const fn valid_positive_safe_integer(value: i64) -> bool {
    value > 0 && value <= MAX_SAFE_INTEGER
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn js_opt(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

#[allow(clippy::cast_precision_loss)]
fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn number_opt(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, number)
}

fn float_opt(value: Option<f64>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_sql_keeps_provider_handles_private_and_effects_two_phase() {
        assert!(MULTIPART_INSERT_SQL.contains("provider_upload_id"));
        assert!(!OPERATION_COMPLETE_SQL.contains("provider_upload_id"));
        assert!(MULTIPART_MARK_COMPLETION_SQL.contains("completion_pending"));
        assert!(OPERATION_EFFECT_PENDING_SQL.contains("effect_pending"));
        assert!(FINALIZE_INTENT_INSERT_SQL.contains("provider_pending"));
    }

    #[test]
    fn migrated_prefixes_and_roles_are_deterministic() {
        assert!(valid_object_prefix(
            "legacy-owner/1234567890abcde/",
            "1234567890abcde"
        ));
        assert!(!valid_object_prefix("actor/other/", "1234567890abcde"));
        assert_eq!(object_role("segments/video/segment-1.m4s"), "segment");
        assert_eq!(object_role("stream.m3u8"), "manifest");
        assert_eq!(object_role("result.mp4"), "source");
    }

    #[test]
    fn error_mapping_fails_closed() {
        assert_eq!(
            map_d1_message("UNIQUE constraint failed"),
            LegacyCoreStorageFailureV1::Conflict
        );
        assert_eq!(
            map_d1_message("frame_legacy_core_storage_assertion_v1"),
            LegacyCoreStorageFailureV1::Forbidden
        );
        assert_eq!(
            map_d1_message("network"),
            LegacyCoreStorageFailureV1::Unavailable
        );
    }

    #[test]
    fn digests_never_store_raw_idempotency_material() {
        let digest = sha256_domain("idempotency", b"client-secret-key");
        assert!(valid_digest(&digest));
        assert!(!digest.contains("client-secret-key"));
    }

    #[test]
    fn idempotency_race_replays_only_the_exact_client_request() {
        let request_digest = sha256_domain("request", b"same-request");
        let row = OperationRowV1 {
            operation_id: Uuid::now_v7().to_string(),
            request_digest: request_digest.clone(),
            result_binding_json: None,
            state: "claimed".into(),
            client_idempotency: 1,
        };
        assert!(matches!(
            exact_client_replay(row.clone(), &request_digest),
            Ok(OperationClaimV1::Replay(_))
        ));
        assert_eq!(
            exact_client_replay(row.clone(), &sha256_domain("request", b"other-request")).err(),
            Some(LegacyCoreStorageFailureV1::Conflict)
        );
        assert_eq!(
            exact_client_replay(
                OperationRowV1 {
                    client_idempotency: 0,
                    ..row
                },
                &request_digest,
            )
            .err(),
            Some(LegacyCoreStorageFailureV1::Conflict)
        );
    }

    #[test]
    fn free_plan_completion_fence_matches_cap_duration_backstop() {
        assert!(!free_plan_completion_allowed(false, true, None));
        assert!(free_plan_completion_allowed(false, true, Some(330.0)));
        assert!(!free_plan_completion_allowed(false, false, Some(330.001)));
        assert!(free_plan_completion_allowed(true, true, None));
        assert!(free_plan_completion_allowed(true, false, Some(10_000.0)));
    }
}
