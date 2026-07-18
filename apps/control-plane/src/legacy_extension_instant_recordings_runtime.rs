//! D1 and R2 authority adapter for Cap's extension instant recordings.

use std::{collections::BTreeMap, fmt::Write as _};

use frame_application::{
    LEGACY_EXTENSION_INSTANT_UPLOAD_TTL_SECONDS, LegacyExtensionInstantCreateInputV1,
    LegacyExtensionInstantCreateSuccessV1, LegacyExtensionInstantProgressInputV1,
    LegacyExtensionInstantPutTargetV1, legacy_extension_instant_object_key,
    legacy_extension_instant_share_url, legacy_extension_instant_title,
    legacy_extension_instant_upload_headers, legacy_extension_instant_valid_wire_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{Bucket, D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

use crate::r2_direct_upload::R2DirectPutSigner;

const CREATE_AUTHORITY_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_authority.sql");
const CREATE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_authority_assert.sql");
const CREATE_VIDEO_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_video_insert.sql");
const CREATE_ALIAS_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_alias_insert.sql");
const CREATE_UPLOAD_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_upload_insert.sql");
const CREATE_RECORDING_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_recording_insert.sql");
const CREATE_OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_operation_insert.sql");
const CREATE_POSTCONDITION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/create_postcondition_assert.sql");
const PROGRESS_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/progress_snapshot.sql");
const PROGRESS_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/progress_authority_assert.sql");
const PROGRESS_UPLOAD_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/progress_upload_insert.sql");
const PROGRESS_RECORDING_CLAIM_UPLOAD_SQL: &str =
    include_str!("../queries/legacy_extension_instant/progress_recording_claim_upload.sql");
const PROGRESS_UPDATE_SQL: &str =
    include_str!("../queries/legacy_extension_instant/progress_update.sql");
const PROGRESS_OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/progress_operation_insert.sql");
const DELETE_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_snapshot.sql");
const DELETE_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_authority_assert.sql");
const DELETE_MARK_SQL: &str = include_str!("../queries/legacy_extension_instant/delete_mark.sql");
const DELETE_VIDEO_TOMBSTONE_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_video_tombstone.sql");
const DELETE_OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_operation_insert.sql");
const DELETE_CLEANUP_ASSERT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_cleanup_assert.sql");
const DELETE_UPLOAD_ABORT_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_upload_abort.sql");
const DELETE_FINALIZE_RECORDING_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_finalize_recording.sql");
const DELETE_FINALIZE_OPERATION_SQL: &str =
    include_str!("../queries/legacy_extension_instant/delete_finalize_operation.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_extension_instant/assertion_cleanup.sql");
const VIDEO_LIFECYCLE_OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/operation_insert.sql");
const VIDEO_LIFECYCLE_OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_video_lifecycle/operation_complete.sql");

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_R2_DELETE_PAGES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyExtensionInstantFailureV1 {
    Invalid,
    Forbidden,
    NotFound,
    Corrupt,
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct CreateAuthorityRowV1 {
    organization_id: String,
    storage_integration_id: String,
    folder_id: Option<String>,
    custom_domain: Option<String>,
    domain_verified: i64,
}

impl CreateAuthorityRowV1 {
    fn valid(&self, requested_org: &str, folder_requested: bool) -> bool {
        valid_id(&self.organization_id)
            && self.organization_id == requested_org
            && valid_id(&self.storage_integration_id)
            && self.folder_id.as_deref().is_none_or(valid_id)
            && (!folder_requested || self.folder_id.is_some())
            && matches!(self.domain_verified, 0 | 1)
            && self.custom_domain.as_deref().is_none_or(|value| {
                !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
            })
    }
}

#[derive(Debug, Deserialize)]
struct ProgressSnapshotRowV1 {
    mapped_video_id: String,
    organization_id: String,
    actor_id: String,
    storage_integration_id: String,
    upload_id: Option<String>,
    source_object_key: String,
    lifecycle_state: String,
    received_bytes: Option<i64>,
    expected_bytes: Option<i64>,
    updated_at_ms: Option<i64>,
}

impl ProgressSnapshotRowV1 {
    fn valid(&self, legacy_video_id: &str) -> bool {
        let object_key = legacy_extension_instant_object_key(&self.actor_id, legacy_video_id);
        let upload_absent = self.upload_id.is_none()
            && self.received_bytes.is_none()
            && self.expected_bytes.is_none()
            && self.updated_at_ms.is_none();
        let upload_present = self.upload_id.as_deref().is_some_and(valid_uuid)
            && [self.received_bytes, self.expected_bytes, self.updated_at_ms]
                .into_iter()
                .all(|value| value.is_some_and(valid_safe_integer));
        valid_uuid(&self.mapped_video_id)
            && valid_id(&self.organization_id)
            && valid_id(&self.actor_id)
            && valid_id(&self.storage_integration_id)
            && self.lifecycle_state == "active"
            && object_key.as_deref() == Some(self.source_object_key.as_str())
            && (upload_absent || upload_present)
    }
}

#[derive(Debug, Deserialize)]
struct DeleteSnapshotRowV1 {
    mapped_video_id: String,
    organization_id: String,
    actor_id: String,
    upload_id: Option<String>,
    storage_prefix: String,
    lifecycle_state: String,
    pending_operation_id: Option<String>,
}

impl DeleteSnapshotRowV1 {
    fn valid(&self, legacy_video_id: &str) -> bool {
        let expected_prefix = legacy_extension_instant_object_key(&self.actor_id, legacy_video_id)
            .and_then(|key| key.strip_suffix("result.mp4").map(str::to_owned));
        valid_uuid(&self.mapped_video_id)
            && valid_id(&self.organization_id)
            && valid_id(&self.actor_id)
            && self.upload_id.as_deref().is_none_or(valid_uuid)
            && expected_prefix.as_deref() == Some(self.storage_prefix.as_str())
            && matches!(self.lifecycle_state.as_str(), "active" | "deleting")
            && match self.lifecycle_state.as_str() {
                "active" => self.pending_operation_id.is_none(),
                "deleting" => self.pending_operation_id.as_deref().is_some_and(valid_uuid),
                _ => false,
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyExtensionInstantDeletePlanV1 {
    legacy_video_id: String,
    mapped_video_id: String,
    organization_id: String,
    actor_id: String,
    upload_id: Option<String>,
    storage_prefix: String,
    operation_id: String,
}

/// Optional receipt written in the same D1 batch as an Effect-RPC instant
/// recording. This closes the crash window between creating the recording and
/// making the RPC request ID replayable.
pub(crate) struct LegacyExtensionInstantLifecycleReceiptV1<'a> {
    pub(crate) operation_id: &'a str,
    pub(crate) source_operation_id: &'a str,
    pub(crate) request_key_digest: &'a str,
    pub(crate) request_digest: &'a str,
}

impl LegacyExtensionInstantLifecycleReceiptV1<'_> {
    fn valid(&self) -> bool {
        valid_uuid(self.operation_id)
            && self.source_operation_id == "cap-v1-7b4e8210491e549d"
            && valid_digest(self.request_key_digest)
            && valid_digest(self.request_digest)
    }
}

impl LegacyExtensionInstantDeletePlanV1 {
    #[must_use]
    pub(crate) fn storage_prefix(&self) -> &str {
        &self.storage_prefix
    }
}

pub(crate) struct D1LegacyExtensionInstantRecordingsV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyExtensionInstantRecordingsV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create(
        &self,
        actor_id: &str,
        input: &LegacyExtensionInstantCreateInputV1,
        web_origin: &Url,
        default_public: bool,
        signer: &R2DirectPutSigner,
        now_ms: i64,
        lifecycle_receipt: Option<&LegacyExtensionInstantLifecycleReceiptV1<'_>>,
    ) -> std::result::Result<LegacyExtensionInstantCreateSuccessV1, LegacyExtensionInstantFailureV1>
    {
        if !valid_id(actor_id)
            || !input.valid()
            || !valid_safe_integer(now_ms)
            || lifecycle_receipt.is_some_and(|receipt| !receipt.valid())
        {
            return Err(LegacyExtensionInstantFailureV1::Invalid);
        }
        let authority = self.create_authority(actor_id, input).await?;
        let operation_id = Uuid::now_v7().to_string();
        let mapped_video_id = Uuid::now_v7().to_string();
        let upload_id = Uuid::now_v7().to_string();
        let legacy_video_id = random_cap_nanoid()?;
        let source_object_key = legacy_extension_instant_object_key(actor_id, &legacy_video_id)
            .ok_or(LegacyExtensionInstantFailureV1::Invalid)?;
        let storage_prefix = source_object_key
            .strip_suffix("result.mp4")
            .ok_or(LegacyExtensionInstantFailureV1::Corrupt)?;
        let source_headers = legacy_extension_instant_upload_headers(actor_id, input);
        let capability = signer
            .sign_legacy_instant_put(
                &source_object_key,
                &source_headers,
                u64::try_from(now_ms).map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?,
                LEGACY_EXTENSION_INSTANT_UPLOAD_TTL_SECONDS,
            )
            .map_err(|_| LegacyExtensionInstantFailureV1::Unavailable)?;
        let headers = capability
            .required_headers
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let share_url = legacy_extension_instant_share_url(
            web_origin,
            authority.custom_domain.as_deref(),
            authority.domain_verified == 1,
            &legacy_video_id,
        )
        .ok_or(LegacyExtensionInstantFailureV1::Corrupt)?;
        let (year, month, day) = utc_date(now_ms)?;
        let title = legacy_extension_instant_title(day, month, year)
            .ok_or(LegacyExtensionInstantFailureV1::Corrupt)?;
        let supports_progress = input.supports_progress();
        let folder_id = js_opt(authority.folder_id.as_deref());
        let duration_ms = duration_ms(input.duration_seconds);
        let request_digest = request_digest(input)?;
        let privacy = if default_public { "public" } else { "private" };
        let success = LegacyExtensionInstantCreateSuccessV1 {
            id: legacy_video_id.clone(),
            share_url,
            upload: LegacyExtensionInstantPutTargetV1 {
                target_type: "put",
                url: capability.url,
                headers,
            },
        };
        let result_json = serde_json::to_string(&success)
            .map_err(|_| LegacyExtensionInstantFailureV1::Corrupt)?;
        let mut statements = vec![
            self.statement(
                CREATE_AUTHORITY_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    folder_id.clone(),
                    js(&authority.storage_integration_id),
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
                    folder_id,
                    js(privacy),
                    js(&operation_id),
                    bool_number(default_public),
                    float_opt(input.duration_seconds),
                    js_opt(input.resolution.as_deref()),
                    float_opt(input.width),
                    float_opt(input.height),
                    js_opt(input.video_codec.as_deref()),
                    js_opt(input.audio_codec.as_deref()),
                    bool_number(supports_progress),
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
                    js(&format!("legacy-extension-instant:{legacy_video_id}")),
                    js(&source_object_key),
                    number(now_ms),
                    js(&operation_id),
                    bool_number(supports_progress),
                ],
            )?,
            self.statement(
                CREATE_RECORDING_INSERT_SQL,
                &[
                    js(&legacy_video_id),
                    js(&mapped_video_id),
                    js(&upload_id),
                    js(&authority.organization_id),
                    js(actor_id),
                    js(&authority.storage_integration_id),
                    js(storage_prefix),
                    js(&source_object_key),
                    bool_number(supports_progress),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                CREATE_OPERATION_INSERT_SQL,
                &[
                    js(&operation_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&legacy_video_id),
                    js(&mapped_video_id),
                    js(&request_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                CREATE_POSTCONDITION_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&legacy_video_id),
                    js(&mapped_video_id),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&source_object_key),
                    bool_number(supports_progress),
                    js(&upload_id),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&operation_id)])?,
        ];
        if let Some(receipt) = lifecycle_receipt {
            statements.push(self.statement(
                VIDEO_LIFECYCLE_OPERATION_INSERT_SQL,
                &[
                    js(receipt.operation_id),
                    js(receipt.source_operation_id),
                    js("video_instant_create"),
                    js(actor_id),
                    js(&authority.organization_id),
                    js(&mapped_video_id),
                    js(&legacy_video_id),
                    js(receipt.request_key_digest),
                    js(receipt.request_digest),
                    JsValue::NULL,
                    JsValue::NULL,
                    js(storage_prefix),
                    JsValue::NULL,
                    js(&result_json),
                    js("claimed"),
                    number(now_ms),
                ],
            )?);
            statements.push(self.statement(
                VIDEO_LIFECYCLE_OPERATION_COMPLETE_SQL,
                &[js(receipt.operation_id), js(&result_json), number(now_ms)],
            )?);
        }
        self.batch(statements).await?;
        Ok(success)
    }

    pub(crate) async fn progress(
        &self,
        actor_id: &str,
        input: &LegacyExtensionInstantProgressInputV1,
        source_updated_at_ms: i64,
        now_ms: i64,
    ) -> std::result::Result<(), LegacyExtensionInstantFailureV1> {
        if !valid_id(actor_id)
            || !input.valid()
            || !valid_safe_integer(source_updated_at_ms)
            || !valid_safe_integer(now_ms)
        {
            return Err(LegacyExtensionInstantFailureV1::Invalid);
        }
        let rows = self
            .rows::<ProgressSnapshotRowV1>(PROGRESS_SNAPSHOT_SQL, &[js(&input.video_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        let snapshot = rows
            .into_iter()
            .next()
            .ok_or(LegacyExtensionInstantFailureV1::NotFound)?;
        if !snapshot.valid(&input.video_id) {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        if snapshot.actor_id != actor_id {
            return Err(LegacyExtensionInstantFailureV1::NotFound);
        }
        let operation_id = Uuid::now_v7().to_string();
        let generated_upload_id = Uuid::now_v7().to_string();
        let upload_id = snapshot
            .upload_id
            .clone()
            .unwrap_or_else(|| generated_upload_id.clone());
        let insert_upload = snapshot.upload_id.is_none();
        let uploaded = i64::try_from(input.clamped_uploaded())
            .map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?;
        let total =
            i64::try_from(input.total).map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?;
        let digest = request_digest(input)?;
        let statements = vec![
            self.statement(
                PROGRESS_AUTHORITY_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(&input.video_id),
                    js(&snapshot.mapped_video_id),
                    js(actor_id),
                    js(&snapshot.organization_id),
                ],
            )?,
            self.statement(
                PROGRESS_UPLOAD_INSERT_SQL,
                &[
                    js(&generated_upload_id),
                    js(&snapshot.organization_id),
                    js(&snapshot.mapped_video_id),
                    number(total),
                    js(&format!("legacy-extension-progress:{}", input.video_id)),
                    js(&snapshot.source_object_key),
                    number(now_ms),
                    number(source_updated_at_ms),
                    js(&operation_id),
                    bool_number(insert_upload),
                ],
            )?,
            self.statement(
                PROGRESS_RECORDING_CLAIM_UPLOAD_SQL,
                &[
                    js(&input.video_id),
                    js(&snapshot.mapped_video_id),
                    js(&upload_id),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                PROGRESS_UPDATE_SQL,
                &[
                    js(&input.video_id),
                    js(&snapshot.mapped_video_id),
                    js(actor_id),
                    number(uploaded),
                    number(total),
                    number(source_updated_at_ms),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                PROGRESS_OPERATION_INSERT_SQL,
                &[
                    js(&operation_id),
                    js(actor_id),
                    js(&snapshot.organization_id),
                    js(&input.video_id),
                    js(&snapshot.mapped_video_id),
                    js(&digest),
                    number(uploaded),
                    number(total),
                    number(source_updated_at_ms),
                    number(now_ms),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&operation_id)])?,
        ];
        self.batch(statements).await
    }

    pub(crate) async fn begin_delete(
        &self,
        actor_id: &str,
        legacy_video_id: &str,
        now_ms: i64,
    ) -> std::result::Result<LegacyExtensionInstantDeletePlanV1, LegacyExtensionInstantFailureV1>
    {
        if !valid_id(actor_id)
            || !legacy_extension_instant_valid_wire_id(legacy_video_id)
            || !valid_safe_integer(now_ms)
        {
            return Err(LegacyExtensionInstantFailureV1::Invalid);
        }
        let rows = self
            .rows::<DeleteSnapshotRowV1>(DELETE_SNAPSHOT_SQL, &[js(legacy_video_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        let snapshot = rows
            .into_iter()
            .next()
            .ok_or(LegacyExtensionInstantFailureV1::NotFound)?;
        if !snapshot.valid(legacy_video_id) {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        if snapshot.actor_id != actor_id {
            return Err(LegacyExtensionInstantFailureV1::NotFound);
        }
        if snapshot.lifecycle_state == "deleting" {
            return Ok(LegacyExtensionInstantDeletePlanV1 {
                legacy_video_id: legacy_video_id.into(),
                mapped_video_id: snapshot.mapped_video_id,
                organization_id: snapshot.organization_id,
                actor_id: snapshot.actor_id,
                upload_id: snapshot.upload_id,
                storage_prefix: snapshot.storage_prefix,
                operation_id: snapshot
                    .pending_operation_id
                    .ok_or(LegacyExtensionInstantFailureV1::Corrupt)?,
            });
        }
        let operation_id = Uuid::now_v7().to_string();
        let digest = sha256_hex(legacy_video_id.as_bytes());
        let statements = vec![
            self.statement(
                DELETE_AUTHORITY_ASSERT_SQL,
                &[
                    js(&operation_id),
                    js(legacy_video_id),
                    js(&snapshot.mapped_video_id),
                    js(actor_id),
                    js(&snapshot.organization_id),
                ],
            )?,
            self.statement(
                DELETE_MARK_SQL,
                &[
                    js(legacy_video_id),
                    js(&snapshot.mapped_video_id),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                DELETE_VIDEO_TOMBSTONE_SQL,
                &[
                    js(&snapshot.mapped_video_id),
                    js(actor_id),
                    number(now_ms),
                    js(&operation_id),
                ],
            )?,
            self.statement(
                DELETE_OPERATION_INSERT_SQL,
                &[
                    js(&operation_id),
                    js(actor_id),
                    js(&snapshot.organization_id),
                    js(legacy_video_id),
                    js(&snapshot.mapped_video_id),
                    js(&digest),
                    number(now_ms),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&operation_id)])?,
        ];
        self.batch(statements).await?;
        Ok(LegacyExtensionInstantDeletePlanV1 {
            legacy_video_id: legacy_video_id.into(),
            mapped_video_id: snapshot.mapped_video_id,
            organization_id: snapshot.organization_id,
            actor_id: snapshot.actor_id,
            upload_id: snapshot.upload_id,
            storage_prefix: snapshot.storage_prefix,
            operation_id,
        })
    }

    pub(crate) async fn finalize_delete(
        &self,
        plan: &LegacyExtensionInstantDeletePlanV1,
        now_ms: i64,
    ) -> std::result::Result<(), LegacyExtensionInstantFailureV1> {
        if !valid_safe_integer(now_ms) {
            return Err(LegacyExtensionInstantFailureV1::Invalid);
        }
        let fingerprint = sha256_hex(
            format!("frame-extension-instant-abort-v1\0{}", plan.operation_id).as_bytes(),
        );
        let mut statements = Vec::with_capacity(6);
        if let Some(upload_id) = &plan.upload_id {
            statements.push(self.statement(
                DELETE_UPLOAD_ABORT_SQL,
                &[
                    js(upload_id),
                    js(&plan.organization_id),
                    number(now_ms),
                    js(&fingerprint),
                    js(&plan.operation_id),
                ],
            )?);
        }
        statements.extend([
            self.statement(
                DELETE_CLEANUP_ASSERT_SQL,
                &[
                    js(&plan.operation_id),
                    js(&plan.legacy_video_id),
                    js(&plan.mapped_video_id),
                    js(&plan.actor_id),
                ],
            )?,
            self.statement(
                DELETE_FINALIZE_RECORDING_SQL,
                &[
                    js(&plan.legacy_video_id),
                    js(&plan.mapped_video_id),
                    number(now_ms),
                    js(&plan.operation_id),
                ],
            )?,
            self.statement(
                DELETE_FINALIZE_OPERATION_SQL,
                &[js(&plan.operation_id), number(now_ms)],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, &[js(&plan.operation_id)])?,
        ]);
        self.batch(statements).await
    }

    async fn create_authority(
        &self,
        actor_id: &str,
        input: &LegacyExtensionInstantCreateInputV1,
    ) -> std::result::Result<CreateAuthorityRowV1, LegacyExtensionInstantFailureV1> {
        let rows = self
            .rows::<CreateAuthorityRowV1>(
                CREATE_AUTHORITY_SQL,
                &[
                    js(actor_id),
                    js(&input.org_id),
                    js_opt(input.folder_id.as_deref()),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        let row = rows
            .into_iter()
            .next()
            .ok_or(LegacyExtensionInstantFailureV1::Forbidden)?;
        if !row.valid(&input.org_id, input.folder_id.is_some()) {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        Ok(row)
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> std::result::Result<D1PreparedStatement, LegacyExtensionInstantFailureV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyExtensionInstantFailureV1::Unavailable)
    }

    async fn rows<T>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> std::result::Result<Vec<T>, LegacyExtensionInstantFailureV1>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyExtensionInstantFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyExtensionInstantFailureV1::Corrupt)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> std::result::Result<(), LegacyExtensionInstantFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyExtensionInstantFailureV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }
}

pub(crate) async fn delete_r2_prefix(
    bucket: &Bucket,
    prefix: &str,
) -> std::result::Result<(), LegacyExtensionInstantFailureV1> {
    if prefix.is_empty()
        || prefix.len() > 512
        || !prefix.ends_with('/')
        || prefix.contains("..")
        || prefix.contains('\\')
    {
        return Err(LegacyExtensionInstantFailureV1::Corrupt);
    }
    for _ in 0..MAX_R2_DELETE_PAGES {
        let listed = bucket
            .list()
            .limit(1_000)
            .prefix(prefix)
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyExtensionInstantFailureV1::Unavailable)?;
        let keys = listed
            .objects()
            .into_iter()
            .map(|object| object.key())
            .collect::<Vec<_>>();
        if keys.iter().any(|key| !key.starts_with(prefix)) {
            return Err(LegacyExtensionInstantFailureV1::Corrupt);
        }
        if !keys.is_empty() {
            bucket
                .delete_multiple(keys)
                .into_send()
                .await
                .map_err(|_| LegacyExtensionInstantFailureV1::Unavailable)?;
        }
        if !listed.truncated() {
            return Ok(());
        }
    }
    Err(LegacyExtensionInstantFailureV1::Unavailable)
}

fn map_d1_message(message: &str) -> LegacyExtensionInstantFailureV1 {
    if message.contains("frame_legacy_extension_instant_assertion_failed_v1") {
        LegacyExtensionInstantFailureV1::Forbidden
    } else if message.contains("frame_legacy_extension_instant_")
        || message.contains("frame_business_")
        || message.contains("foreign key")
    {
        LegacyExtensionInstantFailureV1::Corrupt
    } else {
        LegacyExtensionInstantFailureV1::Unavailable
    }
}

fn random_cap_nanoid() -> std::result::Result<String, LegacyExtensionInstantFailureV1> {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut random = [0_u8; 15];
    getrandom::fill(&mut random).map_err(|_| LegacyExtensionInstantFailureV1::Unavailable)?;
    let value = random
        .into_iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect::<String>();
    legacy_extension_instant_valid_wire_id(&value)
        .then_some(value)
        .ok_or(LegacyExtensionInstantFailureV1::Corrupt)
}

fn request_digest<T: Serialize>(
    value: &T,
) -> std::result::Result<String, LegacyExtensionInstantFailureV1> {
    let encoded =
        serde_json::to_vec(value).map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?;
    Ok(sha256_hex(&encoded))
}

fn sha256_hex(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(value) {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

fn duration_ms(value: Option<f64>) -> Option<i64> {
    let value = value?;
    let milliseconds = value * 1_000.0;
    (value >= 0.0 && milliseconds <= MAX_SAFE_INTEGER as f64).then(|| milliseconds.round() as i64)
}

fn utc_date(now_ms: i64) -> std::result::Result<(i32, u8, u8), LegacyExtensionInstantFailureV1> {
    let days = now_ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_prime = (5 * doy + 2) / 153;
    let day = doy - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Ok((
        i32::try_from(year).map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?,
        u8::try_from(month).map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?,
        u8::try_from(day).map_err(|_| LegacyExtensionInstantFailureV1::Invalid)?,
    ))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn valid_safe_integer(value: i64) -> bool {
    value >= 0 && value <= MAX_SAFE_INTEGER
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

fn bool_number(value: bool) -> JsValue {
    JsValue::from_f64(if value { 1.0 } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_sql_fences_tenant_progress_and_two_phase_cleanup() {
        assert!(CREATE_AUTHORITY_SQL.contains("actor.active_organization_id = organization.id"));
        assert!(CREATE_AUTHORITY_SQL.contains("storage.provider = 'r2'"));
        assert!(CREATE_POSTCONDITION_ASSERT_SQL.contains("instant.source_object_key = ?6"));
        assert!(PROGRESS_UPDATE_SQL.contains("updated_at_ms <= ?6"));
        assert!(PROGRESS_UPDATE_SQL.contains("received_bytes <= ?4"));
        assert!(PROGRESS_UPDATE_SQL.contains("expected_bytes <= ?5"));
        assert!(PROGRESS_OPERATION_INSERT_SQL.contains("changes() = 1"));
        assert!(DELETE_MARK_SQL.contains("storage_cleanup_state = 'pending'"));
        assert!(DELETE_FINALIZE_RECORDING_SQL.contains("storage_cleanup_state = 'complete'"));
        assert!(VIDEO_LIFECYCLE_OPERATION_INSERT_SQL.contains("request_key_digest"));
        assert!(VIDEO_LIFECYCLE_OPERATION_COMPLETE_SQL.contains("state = 'complete'"));
    }

    #[test]
    fn aliases_dates_digests_and_duration_projection_are_deterministic() {
        assert_eq!(utc_date(1_721_174_400_000), Ok((2024, 7, 17)));
        assert_eq!(duration_ms(Some(12.3456)), Some(12_346));
        assert_eq!(duration_ms(Some(-1.0)), None);
        assert_eq!(sha256_hex(b"frame").len(), 64);
        for _ in 0..64 {
            let alias = random_cap_nanoid().expect("alias");
            assert!(legacy_extension_instant_valid_wire_id(&alias));
        }
    }

    #[test]
    fn d1_error_projection_fails_closed() {
        assert_eq!(
            map_d1_message("frame_legacy_extension_instant_assertion_failed_v1"),
            LegacyExtensionInstantFailureV1::Forbidden
        );
        assert_eq!(
            map_d1_message("frame_legacy_extension_instant_progress_regression_v1"),
            LegacyExtensionInstantFailureV1::Corrupt
        );
        assert_eq!(
            map_d1_message("network"),
            LegacyExtensionInstantFailureV1::Unavailable
        );
    }

    #[test]
    fn effect_rpc_receipt_requires_the_exact_operation_and_digests() {
        let receipt = LegacyExtensionInstantLifecycleReceiptV1 {
            operation_id: "00000000-0000-7000-8000-000000000001",
            source_operation_id: "cap-v1-7b4e8210491e549d",
            request_key_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            request_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        };
        assert!(receipt.valid());
        assert!(
            !LegacyExtensionInstantLifecycleReceiptV1 {
                source_operation_id: "cap-v1-e6a882aeeffaa4f6",
                ..receipt
            }
            .valid()
        );
    }
}
