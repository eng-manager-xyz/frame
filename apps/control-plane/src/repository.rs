//! Aggregate-oriented D1 access for the Worker boundary.
//!
//! SQL lives in checked-in query files so runtime access, query-plan evidence,
//! and the local D1 conformance suite cannot silently drift apart. Every
//! externally supplied value is a bound parameter. The only generated SQL is
//! a bounded list of positional placeholders for bulk reads.

use std::{cmp::Ordering, collections::BTreeSet, future::Future, ops::Range, time::Duration};

use frame_domain::{business_initial_event_fingerprint, business_payload_checksum};
use futures::{future::Either, future::select, pin_mut};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Delay, Error};

use crate::{
    commands::{
        COMMAND_TTL_MS, MediaJobStatusResponse, StoredCommandRow, UploadStatusResponse,
        VideoResponse, WorkerJobResponse, request_digest,
    },
    contracts::{
        API_SCHEMA_VERSION, MAX_SAFE_INTEGER, valid_content_type, valid_idempotency_key, valid_uuid,
    },
};

pub const MAX_PAGE_SIZE: usize = 100;
pub const DEFAULT_PAGE_SIZE: usize = 25;
pub const MAX_D1_BOUND_PARAMETERS: usize = 100;
pub const MAX_BULK_IDENTIFIERS: usize = 1_000;
pub const DEFAULT_QUERY_TIMEOUT_MS: u64 = 5_000;

const CURSOR_VERSION: u8 = 1;
const CURSOR_BYTES: usize = 1 + 8 + 16;
const VIDEO_FOR_MUTATION_SQL: &str = include_str!("../queries/video_for_mutation.sql");
const UPLOAD_BY_ID_SQL: &str = include_str!("../queries/upload_by_id.sql");
const MEDIA_JOB_BY_ID_SQL: &str = include_str!("../queries/media_job_by_id.sql");
const NATIVE_WORKER_JOB_BY_ID_SQL: &str = include_str!("../queries/native_worker_job_by_id.sql");
const ORGANIZATION_SNAPSHOT_SQL: &str = include_str!("../queries/organization_snapshot.sql");
const VIDEO_PAGE_SQL: &str = include_str!("../queries/video_page.sql");
const VIDEO_PAGE_AFTER_SQL: &str = include_str!("../queries/video_page_after.sql");
const VIDEO_TITLE_COMMAND_SQL: &str = include_str!("../queries/video_title_command.sql");
const VIDEO_TITLE_APPLY_SQL: &str = include_str!("../queries/video_title_apply.sql");
const VIDEO_TITLE_COMMAND_TYPE: &str = "repository_video_title_v1";
const VIDEO_TITLE_BATCH_META_CHANGES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryFailure {
    InvalidRequest,
    Conflict,
    Timeout,
    Unavailable,
    CorruptResult,
}

impl RepositoryFailure {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "repository_invalid_request",
            Self::Conflict => "repository_conflict",
            Self::Timeout => "repository_timeout",
            Self::Unavailable => "repository_unavailable",
            Self::CorruptResult => "repository_corrupt_result",
        }
    }

    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Conflict | Self::Timeout | Self::Unavailable)
    }

    pub(crate) fn into_worker_error(self) -> Error {
        // This deliberately excludes the provider error, SQL, bindings, and
        // tenant identifiers. The fixed code is enough for internal routing;
        // public handlers retain their existing stable API failures.
        Error::RustError(format!("database repository failure: {}", self.code()))
    }
}

pub type RepositoryResult<T> = std::result::Result<T, RepositoryFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryClass {
    VideoAuthorization,
    VideoPage,
    VideoBulk,
    UploadAggregate,
    MediaJobAggregate,
    NativeWorkerJobAggregate,
    OrganizationSnapshot,
    VideoTitleCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryTelemetry {
    pub event: &'static str,
    pub query_class: QueryClass,
    pub duration_ms: u64,
    pub rows: usize,
    pub retries: u8,
    pub bookmark_use: &'static str,
    pub outcome: &'static str,
}

impl QueryTelemetry {
    fn completed(query_class: QueryClass, started_at_ms: f64, rows: usize) -> Self {
        Self {
            event: "d1_repository_query",
            query_class,
            duration_ms: elapsed_ms(started_at_ms),
            rows,
            retries: 0,
            // The Workers D1 binding used here does not expose session
            // bookmarks. Cross-request bookmark evidence is therefore not
            // claimed by this adapter or its local harness.
            bookmark_use: "unavailable_in_workers_binding",
            outcome: "ok",
        }
    }

    fn failed(query_class: QueryClass, started_at_ms: f64, failure: RepositoryFailure) -> Self {
        Self {
            event: "d1_repository_query",
            query_class,
            duration_ms: elapsed_ms(started_at_ms),
            rows: 0,
            retries: 0,
            bookmark_use: "unavailable_in_workers_binding",
            outcome: failure.code(),
        }
    }

    #[must_use]
    pub fn safe_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            "{\"event\":\"d1_repository_query\",\"outcome\":\"telemetry_encoding_failed\"}".into()
        })
    }
}

fn elapsed_ms(started_at_ms: f64) -> u64 {
    let elapsed = js_sys::Date::now() - started_at_ms;
    if elapsed.is_finite() && elapsed > 0.0 {
        elapsed.min(MAX_SAFE_INTEGER as f64).floor() as u64
    } else {
        0
    }
}

fn emit_telemetry(telemetry: &QueryTelemetry) {
    worker::console_log!("{}", telemetry.safe_json());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCursor {
    created_at_ms: u64,
    id: Uuid,
}

impl PageCursor {
    pub fn new(created_at_ms: u64, id: Uuid) -> RepositoryResult<Self> {
        if created_at_ms > MAX_SAFE_INTEGER || id.is_nil() {
            return Err(RepositoryFailure::InvalidRequest);
        }
        Ok(Self { created_at_ms, id })
    }

    #[must_use]
    pub const fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    #[must_use]
    pub fn encode(&self) -> String {
        let mut bytes = [0_u8; CURSOR_BYTES];
        bytes[0] = CURSOR_VERSION;
        bytes[1..9].copy_from_slice(&self.created_at_ms.to_be_bytes());
        bytes[9..].copy_from_slice(self.id.as_bytes());
        lower_hex(&bytes)
    }

    pub fn decode(value: &str) -> RepositoryResult<Self> {
        if value.len() != CURSOR_BYTES * 2
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(RepositoryFailure::InvalidRequest);
        }
        let mut bytes = [0_u8; CURSOR_BYTES];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        if bytes[0] != CURSOR_VERSION {
            return Err(RepositoryFailure::InvalidRequest);
        }
        let timestamp = u64::from_be_bytes(
            bytes[1..9]
                .try_into()
                .map_err(|_| RepositoryFailure::InvalidRequest)?,
        );
        if timestamp > MAX_SAFE_INTEGER {
            return Err(RepositoryFailure::InvalidRequest);
        }
        let id = Uuid::from_slice(&bytes[9..]).map_err(|_| RepositoryFailure::InvalidRequest)?;
        if id.is_nil() {
            return Err(RepositoryFailure::InvalidRequest);
        }
        Self::new(timestamp, id)
    }
}

fn hex_nibble(byte: u8) -> RepositoryResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(RepositoryFailure::InvalidRequest),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPageRequest {
    pub limit: usize,
    pub cursor: Option<PageCursor>,
}

impl VideoPageRequest {
    pub fn new(limit: Option<usize>, cursor: Option<&str>) -> RepositoryResult<Self> {
        let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(RepositoryFailure::InvalidRequest);
        }
        let cursor = cursor.map(PageCursor::decode).transpose()?;
        Ok(Self { limit, cursor })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoPageItem {
    pub id: String,
    pub title: String,
    pub state: String,
    pub privacy: String,
    pub revision: u64,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoPage {
    pub items: Vec<VideoPageItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VideoPageRow {
    id: String,
    title: String,
    state: String,
    privacy: String,
    revision: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl TryFrom<VideoPageRow> for VideoPageItem {
    type Error = RepositoryFailure;

    fn try_from(row: VideoPageRow) -> RepositoryResult<Self> {
        if !valid_uuid(&row.id)
            || row.title.trim().is_empty()
            || row.title.trim() != row.title
            || row.title.chars().count() > 160
            || row.title.chars().any(char::is_control)
            || !matches!(
                row.state.as_str(),
                "pending" | "uploading" | "processing" | "ready" | "failed"
            )
            || !matches!(
                row.privacy.as_str(),
                "private" | "organization" | "public" | "unlisted"
            )
        {
            return Err(RepositoryFailure::CorruptResult);
        }
        Ok(Self {
            id: row.id,
            title: row.title,
            state: row.state,
            privacy: row.privacy,
            revision: safe_u64(row.revision)?,
            created_at_ms: safe_u64(row.created_at_ms)?,
            updated_at_ms: safe_u64(row.updated_at_ms)?,
        })
    }
}

fn safe_u64(value: i64) -> RepositoryResult<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value <= MAX_SAFE_INTEGER)
        .ok_or(RepositoryFailure::CorruptResult)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrganizationSnapshot {
    pub id: String,
    pub name: String,
    pub status: String,
    pub revision: u64,
    pub active_members: u64,
    pub active_videos: u64,
    pub active_uploads: u64,
    pub active_media_jobs: u64,
}

#[derive(Debug, Deserialize)]
struct OrganizationSnapshotRow {
    id: String,
    name: String,
    status: String,
    revision: i64,
    active_members: i64,
    active_videos: i64,
    active_uploads: i64,
    active_media_jobs: i64,
}

impl TryFrom<OrganizationSnapshotRow> for OrganizationSnapshot {
    type Error = RepositoryFailure;

    fn try_from(row: OrganizationSnapshotRow) -> RepositoryResult<Self> {
        if !valid_uuid(&row.id)
            || row.name.trim().is_empty()
            || row.name.chars().count() > 160
            || !matches!(row.status.as_str(), "active" | "tombstoned")
        {
            return Err(RepositoryFailure::CorruptResult);
        }
        Ok(Self {
            id: row.id,
            name: row.name,
            status: row.status,
            revision: safe_u64(row.revision)?,
            active_members: safe_u64(row.active_members)?,
            active_videos: safe_u64(row.active_videos)?,
            active_uploads: safe_u64(row.active_uploads)?,
            active_media_jobs: safe_u64(row.active_media_jobs)?,
        })
    }
}

#[derive(Clone, Deserialize)]
pub struct VideoMutationRow {
    pub id: String,
    pub owner_id: String,
    pub state: String,
    pub privacy: String,
    pub revision: i64,
    pub actor_role: String,
    pub actor_manages_space: i64,
    #[serde(skip)]
    queried_actor_id: String,
}

impl VideoMutationRow {
    fn validated(mut self, video_id: &str, actor_id: &str) -> RepositoryResult<VideoMutationRow> {
        if self.id != video_id
            || !valid_uuid(&self.id)
            || !valid_uuid(&self.owner_id)
            || !valid_uuid(actor_id)
            || !matches!(
                self.state.as_str(),
                "pending" | "uploading" | "processing" | "ready" | "failed"
            )
            || !matches!(
                self.privacy.as_str(),
                "private" | "organization" | "public" | "unlisted"
            )
            || !matches!(
                self.actor_role.as_str(),
                "owner" | "admin" | "member" | "viewer"
            )
            || !matches!(self.actor_manages_space, 0 | 1)
            || safe_u64(self.revision).is_err()
        {
            return Err(RepositoryFailure::CorruptResult);
        }
        self.queried_actor_id = actor_id.to_owned();
        Ok(self)
    }

    #[must_use]
    pub fn actor_can_update(&self) -> bool {
        matches!(self.actor_role.as_str(), "owner" | "admin")
            || (self.actor_role == "member"
                && (self.owner_id == self.queried_actor_id || self.actor_manages_space == 1))
    }

    #[must_use]
    pub fn public_response(&self) -> Option<VideoResponse> {
        let revision = u64::try_from(self.revision).ok()?;
        Some(VideoResponse {
            schema_version: API_SCHEMA_VERSION,
            video_id: self.id.clone(),
            state: self.state.clone(),
            privacy: self.privacy.clone(),
            revision,
            upload_intents_path: "/api/v1/uploads/intents".into(),
            public_share_path: (self.privacy == "public")
                .then(|| format!("/api/v1/public/shares/{}", self.id)),
        })
    }
}

#[derive(Deserialize)]
pub struct UploadRow {
    pub id: String,
    pub organization_id: String,
    pub video_id: String,
    pub state: String,
    pub expected_bytes: i64,
    pub received_bytes: i64,
    pub source_object_key: String,
    pub source_version: i64,
    pub content_type: String,
    pub checksum_sha256: Option<String>,
    pub transfer_mode: String,
    pub direct_staging_key: Option<String>,
    pub direct_checksum_sha256: Option<String>,
    pub direct_expires_at_ms: Option<i64>,
}

impl UploadRow {
    fn validated(self, tenant_id: &str, upload_id: &str) -> RepositoryResult<UploadRow> {
        if self.id != upload_id
            || self.organization_id != tenant_id
            || !valid_uuid(&self.id)
            || !valid_uuid(&self.organization_id)
            || !valid_uuid(&self.video_id)
            || !matches!(
                self.state.as_str(),
                "initiated" | "uploading" | "finalizing" | "complete" | "failed" | "aborted"
            )
            || !(0..=i64::try_from(MAX_SAFE_INTEGER).unwrap_or(i64::MAX))
                .contains(&self.expected_bytes)
            || !(0..=self.expected_bytes).contains(&self.received_bytes)
            || self.source_version <= 0
            || u64::try_from(self.source_version)
                .ok()
                .is_none_or(|version| version > MAX_SAFE_INTEGER)
            || !valid_content_type(&self.content_type)
            || self
                .checksum_sha256
                .as_deref()
                .is_some_and(|digest| !valid_digest(digest))
            || match self.transfer_mode.as_str() {
                "brokered" => {
                    self.direct_staging_key.is_some()
                        || self.direct_checksum_sha256.is_some()
                        || self.direct_expires_at_ms.is_some()
                }
                "direct" => {
                    self.direct_staging_key
                        .as_deref()
                        .is_none_or(|key| !valid_direct_staging_key(key))
                        || self
                            .direct_checksum_sha256
                            .as_deref()
                            .is_none_or(|digest| !valid_digest(digest))
                        || self.direct_expires_at_ms.is_none_or(|expiry| expiry <= 0)
                }
                _ => true,
            }
            || !valid_private_object_key(&self.source_object_key, tenant_id, &self.video_id)
        {
            return Err(RepositoryFailure::CorruptResult);
        }
        Ok(self)
    }

    #[must_use]
    pub fn public_status(&self) -> Option<UploadStatusResponse> {
        Some(UploadStatusResponse {
            schema_version: API_SCHEMA_VERSION,
            upload_id: self.id.clone(),
            state: self.state.clone(),
            expected_bytes: u64::try_from(self.expected_bytes).ok()?,
            received_bytes: u64::try_from(self.received_bytes).ok()?,
            content_type: self.content_type.clone(),
        })
    }
}

fn valid_direct_staging_key(value: &str) -> bool {
    let segments = value.split('/').collect::<Vec<_>>();
    let object = segments.get(3).and_then(|value| value.rsplit_once('.'));
    (64..=1_024).contains(&value.len())
        && segments.len() == 4
        && segments[0] == "uploads"
        && segments[1].len() == 64
        && segments[1]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && segments[2] == "staging"
        && object.is_some_and(|(upload_id, extension)| {
            valid_uuid(upload_id) && matches!(extension, "mp4" | "webm" | "mov" | "mkv")
        })
        && !segments[3].is_empty()
        && segments[3]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Deserialize)]
pub struct MediaJobRow {
    pub id: String,
    pub state: String,
    pub profile: String,
    pub selected_executor: Option<String>,
    pub progress_basis_points: Option<i64>,
    pub attempt: i64,
    pub cancel_requested: i64,
    pub error_class: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl MediaJobRow {
    fn validated(self, job_id: &str) -> RepositoryResult<MediaJobRow> {
        if self.id != job_id
            || !valid_uuid(&self.id)
            || !valid_media_job_state(&self.state)
            || !valid_media_profile(&self.profile)
            || self.selected_executor.as_deref().is_some_and(|executor| {
                !matches!(executor, "cloudflare_media" | "native_gstreamer")
            })
            || self
                .progress_basis_points
                .is_some_and(|progress| !(0..=10_000).contains(&progress))
            || u32::try_from(self.attempt).is_err()
            || !matches!(self.cancel_requested, 0 | 1)
            || self
                .error_class
                .as_deref()
                .is_some_and(|error_class| !valid_media_error_class(error_class))
            || safe_u64(self.created_at_ms).is_err()
            || safe_u64(self.updated_at_ms).is_err()
            || self.updated_at_ms < self.created_at_ms
        {
            return Err(RepositoryFailure::CorruptResult);
        }
        Ok(self)
    }

    #[must_use]
    pub fn public_status(&self) -> Option<MediaJobStatusResponse> {
        let progress_basis_points = self
            .progress_basis_points
            .map(u16::try_from)
            .transpose()
            .ok()?;
        Some(MediaJobStatusResponse {
            schema_version: API_SCHEMA_VERSION,
            job_id: self.id.clone(),
            state: self.state.clone(),
            profile: self.profile.clone(),
            executor: self.selected_executor.clone(),
            progress_basis_points,
            attempt: u32::try_from(self.attempt).ok()?,
            cancel_requested: match self.cancel_requested {
                0 => false,
                1 => true,
                _ => return None,
            },
            error_class: self.error_class.clone(),
            created_at_ms: u64::try_from(self.created_at_ms).ok()?,
            updated_at_ms: u64::try_from(self.updated_at_ms).ok()?,
        })
    }
}

#[derive(Clone, Deserialize)]
pub struct WorkerJobRow {
    pub id: String,
    pub video_id: String,
    pub state: String,
    pub revision: i64,
    pub attempt: i64,
    pub profile: String,
    pub source_version: i64,
    pub output_object_key: String,
    pub worker_id: Option<String>,
    pub lease_token_digest: Option<String>,
    pub lease_expires_at_ms: Option<i64>,
    pub progress_basis_points: Option<i64>,
    pub cancel_requested: i64,
}

impl WorkerJobRow {
    fn validated(self, tenant_id: &str, job_id: &str) -> RepositoryResult<WorkerJobRow> {
        if self.id != job_id
            || !valid_uuid(&self.id)
            || !valid_uuid(&self.video_id)
            || !valid_media_job_state(&self.state)
            || !valid_media_profile(&self.profile)
            || safe_u64(self.revision).is_err()
            || u32::try_from(self.attempt).is_err()
            || self.source_version <= 0
            || !valid_derivative_object_key(
                &self.output_object_key,
                tenant_id,
                &self.video_id,
                &self.profile,
            )
            || self
                .worker_id
                .as_deref()
                .is_some_and(|worker_id| !valid_uuid(worker_id))
            || self
                .lease_token_digest
                .as_deref()
                .is_some_and(|digest| !valid_digest(digest))
            || self
                .lease_expires_at_ms
                .is_some_and(|expires_at| safe_u64(expires_at).is_err())
            || self
                .progress_basis_points
                .is_some_and(|progress| !(0..=10_000).contains(&progress))
            || !matches!(self.cancel_requested, 0 | 1)
        {
            return Err(RepositoryFailure::CorruptResult);
        }
        Ok(self)
    }

    #[must_use]
    pub fn private_response(&self, retry_scheduled: bool) -> Option<WorkerJobResponse> {
        Some(WorkerJobResponse {
            schema_version: API_SCHEMA_VERSION,
            job_id: self.id.clone(),
            state: self.state.clone(),
            attempt: u32::try_from(self.attempt).ok()?,
            revision: u64::try_from(self.revision).ok()?,
            progress_basis_points: self
                .progress_basis_points
                .map(u16::try_from)
                .transpose()
                .ok()?,
            cancel_requested: match self.cancel_requested {
                0 => false,
                1 => true,
                _ => return None,
            },
            lease_expires_at_ms: self
                .lease_expires_at_ms
                .map(u64::try_from)
                .transpose()
                .ok()?,
            retry_scheduled,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoTitleCommand {
    pub tenant_id: String,
    pub video_id: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub expected_revision: u64,
    pub title: String,
    pub now_ms: u64,
    pub reservation_id: String,
    pub outbox_id: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoTitleResult {
    pub schema_version: u16,
    pub video_id: String,
    pub title: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoTitleWriteOutcome {
    Applied {
        result: VideoTitleResult,
        batch_meta_changes: usize,
    },
    Replay(VideoTitleResult),
    Conflict,
}

#[derive(Serialize)]
struct VideoTitleDigest<'command> {
    tenant_id: &'command str,
    video_id: &'command str,
    actor_id: &'command str,
    expected_revision: u64,
    title: &'command str,
}

pub struct AggregateRepository<'database> {
    database: &'database D1Database,
    query_timeout_ms: u64,
}

impl<'database> AggregateRepository<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self {
            database,
            query_timeout_ms: DEFAULT_QUERY_TIMEOUT_MS,
        }
    }

    #[must_use]
    pub const fn with_query_timeout_ms(
        database: &'database D1Database,
        query_timeout_ms: u64,
    ) -> Self {
        Self {
            database,
            query_timeout_ms,
        }
    }

    pub async fn video_for_mutation(
        &self,
        tenant_id: &str,
        video_id: &str,
        actor_id: &str,
    ) -> RepositoryResult<Option<VideoMutationRow>> {
        validate_ids(&[tenant_id, video_id, actor_id])?;
        let statement = self.database.prepare(VIDEO_FOR_MUTATION_SQL).bind(&[
            JsValue::from_str(video_id),
            JsValue::from_str(tenant_id),
            JsValue::from_str(actor_id),
        ]);
        self.first(QueryClass::VideoAuthorization, statement, |row| {
            VideoMutationRow::validated(row, video_id, actor_id)
        })
        .await
    }

    pub async fn upload(
        &self,
        tenant_id: &str,
        upload_id: &str,
    ) -> RepositoryResult<Option<UploadRow>> {
        validate_ids(&[tenant_id, upload_id])?;
        let statement = self
            .database
            .prepare(UPLOAD_BY_ID_SQL)
            .bind(&[JsValue::from_str(upload_id), JsValue::from_str(tenant_id)]);
        self.first(QueryClass::UploadAggregate, statement, |row| {
            UploadRow::validated(row, tenant_id, upload_id)
        })
        .await
    }

    pub async fn media_job(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> RepositoryResult<Option<MediaJobRow>> {
        validate_ids(&[tenant_id, job_id])?;
        let statement = self
            .database
            .prepare(MEDIA_JOB_BY_ID_SQL)
            .bind(&[JsValue::from_str(job_id), JsValue::from_str(tenant_id)]);
        self.first(QueryClass::MediaJobAggregate, statement, |row| {
            MediaJobRow::validated(row, job_id)
        })
        .await
    }

    pub async fn native_worker_job(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> RepositoryResult<Option<WorkerJobRow>> {
        validate_ids(&[tenant_id, job_id])?;
        let statement = self
            .database
            .prepare(NATIVE_WORKER_JOB_BY_ID_SQL)
            .bind(&[JsValue::from_str(job_id), JsValue::from_str(tenant_id)]);
        self.first(QueryClass::NativeWorkerJobAggregate, statement, |row| {
            WorkerJobRow::validated(row, tenant_id, job_id)
        })
        .await
    }

    pub async fn organization_snapshot(
        &self,
        tenant_id: &str,
    ) -> RepositoryResult<Option<OrganizationSnapshot>> {
        validate_ids(&[tenant_id])?;
        let statement = self
            .database
            .prepare(ORGANIZATION_SNAPSHOT_SQL)
            .bind(&[JsValue::from_str(tenant_id)]);
        self.first(
            QueryClass::OrganizationSnapshot,
            statement,
            |row: OrganizationSnapshotRow| row.try_into(),
        )
        .await
    }

    pub async fn video_page(
        &self,
        tenant_id: &str,
        request: &VideoPageRequest,
    ) -> RepositoryResult<VideoPage> {
        validate_ids(&[tenant_id])?;
        if !(1..=MAX_PAGE_SIZE).contains(&request.limit) {
            return Err(RepositoryFailure::InvalidRequest);
        }
        let fetch_limit = request
            .limit
            .checked_add(1)
            .ok_or(RepositoryFailure::InvalidRequest)?;
        let statement = request.cursor.as_ref().map_or_else(
            || {
                self.database.prepare(VIDEO_PAGE_SQL).bind(&[
                    JsValue::from_str(tenant_id),
                    JsValue::from_f64(fetch_limit as f64),
                ])
            },
            |cursor| {
                self.database.prepare(VIDEO_PAGE_AFTER_SQL).bind(&[
                    JsValue::from_str(tenant_id),
                    JsValue::from_f64(cursor.created_at_ms as f64),
                    JsValue::from_str(&cursor.id.to_string()),
                    JsValue::from_f64(fetch_limit as f64),
                ])
            },
        );
        let mut items = self
            .video_items(QueryClass::VideoPage, statement, fetch_limit)
            .await?;
        let has_more = items.len() > request.limit;
        items.truncate(request.limit);
        let next_cursor = if has_more {
            items
                .last()
                .map(|item| {
                    Ok(PageCursor::new(
                        item.created_at_ms,
                        Uuid::parse_str(&item.id).map_err(|_| RepositoryFailure::CorruptResult)?,
                    )?
                    .encode())
                })
                .transpose()?
        } else {
            None
        };
        Ok(VideoPage { items, next_cursor })
    }

    pub async fn videos_by_id(
        &self,
        tenant_id: &str,
        ids: &[String],
    ) -> RepositoryResult<Vec<VideoPageItem>> {
        validate_ids(&[tenant_id])?;
        if ids.len() > MAX_BULK_IDENTIFIERS || ids.iter().any(|id| !valid_uuid(id)) {
            return Err(RepositoryFailure::InvalidRequest);
        }
        let unique = ids.iter().cloned().collect::<BTreeSet<_>>();
        let unique = unique.into_iter().collect::<Vec<_>>();
        let mut items = Vec::with_capacity(unique.len());
        for range in parameter_chunk_ranges(unique.len(), 1)? {
            let chunk = &unique[range];
            let sql = video_bulk_sql(chunk.len())?;
            let mut bindings = Vec::with_capacity(chunk.len() + 1);
            bindings.push(JsValue::from_str(tenant_id));
            bindings.extend(chunk.iter().map(|id| JsValue::from_str(id)));
            let statement = self.database.prepare(&sql).bind(&bindings);
            items.extend(
                self.video_items(QueryClass::VideoBulk, statement, chunk.len())
                    .await?,
            );
        }
        items.sort_by(
            |left, right| match right.created_at_ms.cmp(&left.created_at_ms) {
                Ordering::Equal => right.id.cmp(&left.id),
                ordering => ordering,
            },
        );
        Ok(items)
    }

    pub async fn update_video_title(
        &self,
        command: &VideoTitleCommand,
    ) -> RepositoryResult<VideoTitleWriteOutcome> {
        validate_video_title_command(command)?;
        let started_at_ms = js_sys::Date::now();
        let digest = request_digest(
            VIDEO_TITLE_COMMAND_TYPE,
            &VideoTitleDigest {
                tenant_id: &command.tenant_id,
                video_id: &command.video_id,
                actor_id: &command.actor_id,
                expected_revision: command.expected_revision,
                title: &command.title,
            },
        )
        .map_err(|()| RepositoryFailure::InvalidRequest)?;

        if let Some(stored) = self
            .stored_video_title_command(&command.tenant_id, &command.idempotency_key)
            .await?
        {
            let outcome = classify_video_title_replay(stored, command, &digest);
            match &outcome {
                Ok(VideoTitleWriteOutcome::Replay(_)) => emit_telemetry(
                    &QueryTelemetry::completed(QueryClass::VideoTitleCommand, started_at_ms, 0),
                ),
                Ok(VideoTitleWriteOutcome::Conflict) => emit_telemetry(&QueryTelemetry::failed(
                    QueryClass::VideoTitleCommand,
                    started_at_ms,
                    RepositoryFailure::Conflict,
                )),
                Err(failure) => emit_telemetry(&QueryTelemetry::failed(
                    QueryClass::VideoTitleCommand,
                    started_at_ms,
                    *failure,
                )),
                Ok(VideoTitleWriteOutcome::Applied { .. }) => {}
            }
            return outcome;
        }

        let Some(video) = self
            .video_for_mutation(&command.tenant_id, &command.video_id, &command.actor_id)
            .await?
        else {
            emit_telemetry(&QueryTelemetry::failed(
                QueryClass::VideoTitleCommand,
                started_at_ms,
                RepositoryFailure::Conflict,
            ));
            return Ok(VideoTitleWriteOutcome::Conflict);
        };
        if !video.actor_can_update()
            || u64::try_from(video.revision).ok() != Some(command.expected_revision)
        {
            emit_telemetry(&QueryTelemetry::failed(
                QueryClass::VideoTitleCommand,
                started_at_ms,
                RepositoryFailure::Conflict,
            ));
            return Ok(VideoTitleWriteOutcome::Conflict);
        }

        let next_revision = command
            .expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_SAFE_INTEGER)
            .ok_or(RepositoryFailure::InvalidRequest)?;
        let expires_at_ms = command
            .now_ms
            .checked_add(u64::try_from(COMMAND_TTL_MS).unwrap_or(0))
            .filter(|expires_at| *expires_at <= MAX_SAFE_INTEGER)
            .ok_or(RepositoryFailure::InvalidRequest)?;
        let result = VideoTitleResult {
            schema_version: API_SCHEMA_VERSION,
            video_id: command.video_id.clone(),
            title: command.title.clone(),
            revision: next_revision,
        };
        let response_json =
            serde_json::to_string(&result).map_err(|_| RepositoryFailure::InvalidRequest)?;
        let payload_json = response_json.clone();
        let payload_checksum =
            business_payload_checksum(&result).map_err(|_| RepositoryFailure::InvalidRequest)?;
        let event_fingerprint = business_initial_event_fingerprint();
        let deduplication_key = format!(
            "repository-video-title:{}:{}",
            command.tenant_id, command.idempotency_key
        );

        let operation = self.database.prepare(VIDEO_TITLE_APPLY_SQL).bind(&[
            JsValue::from_str(&command.operation_id),
            JsValue::from_str(&command.tenant_id),
            JsValue::from_str(&command.video_id),
            JsValue::from_str(&command.actor_id),
            JsValue::from_str(&command.idempotency_key),
            JsValue::from_str(&digest),
            JsValue::from_str(&command.reservation_id),
            JsValue::from_str(&command.outbox_id),
            JsValue::from_str(&deduplication_key),
            JsValue::from_f64(command.expected_revision as f64),
            JsValue::from_str(&command.title),
            JsValue::from_str(&response_json),
            JsValue::from_str(&payload_json),
            JsValue::from_f64(command.now_ms as f64),
            JsValue::from_f64(expires_at_ms as f64),
            JsValue::from_str(payload_checksum.as_str()),
            JsValue::from_str(event_fingerprint.as_str()),
        ]);
        let statements = match [operation].into_iter().collect::<worker::Result<Vec<_>>>() {
            Ok(statements) => statements,
            Err(_) => {
                let failure = RepositoryFailure::Unavailable;
                emit_telemetry(&QueryTelemetry::failed(
                    QueryClass::VideoTitleCommand,
                    started_at_ms,
                    failure,
                ));
                return Err(failure);
            }
        };

        match self.await_d1(self.database.batch(statements)).await {
            Ok(results) if results.len() == 1 && results.iter().all(worker::D1Result::success) => {
                let batch_meta_changes = results[0]
                    .meta()
                    .ok()
                    .flatten()
                    .and_then(|meta| meta.changes);
                let Some(batch_meta_changes) = batch_meta_changes else {
                    let failure = RepositoryFailure::CorruptResult;
                    emit_telemetry(&QueryTelemetry::failed(
                        QueryClass::VideoTitleCommand,
                        started_at_ms,
                        failure,
                    ));
                    return Err(failure);
                };
                if batch_meta_changes != VIDEO_TITLE_BATCH_META_CHANGES {
                    let failure = RepositoryFailure::CorruptResult;
                    emit_telemetry(&QueryTelemetry::failed(
                        QueryClass::VideoTitleCommand,
                        started_at_ms,
                        failure,
                    ));
                    return Err(failure);
                }
                emit_telemetry(&QueryTelemetry::completed(
                    QueryClass::VideoTitleCommand,
                    started_at_ms,
                    1,
                ));
                Ok(VideoTitleWriteOutcome::Applied {
                    result,
                    batch_meta_changes,
                })
            }
            Err(RepositoryFailure::Timeout) => {
                let failure = RepositoryFailure::Timeout;
                emit_telemetry(&QueryTelemetry::failed(
                    QueryClass::VideoTitleCommand,
                    started_at_ms,
                    failure,
                ));
                Err(failure)
            }
            Ok(_) | Err(RepositoryFailure::Unavailable) => {
                let outcome = self
                    .classify_video_title_batch_failure(command, &digest)
                    .await;
                match &outcome {
                    Ok(VideoTitleWriteOutcome::Replay(_)) => emit_telemetry(
                        &QueryTelemetry::completed(QueryClass::VideoTitleCommand, started_at_ms, 0),
                    ),
                    Ok(VideoTitleWriteOutcome::Conflict) => {
                        emit_telemetry(&QueryTelemetry::failed(
                            QueryClass::VideoTitleCommand,
                            started_at_ms,
                            RepositoryFailure::Conflict,
                        ))
                    }
                    Ok(VideoTitleWriteOutcome::Applied { .. }) => {
                        emit_telemetry(&QueryTelemetry::failed(
                            QueryClass::VideoTitleCommand,
                            started_at_ms,
                            RepositoryFailure::CorruptResult,
                        ))
                    }
                    Err(failure) => emit_telemetry(&QueryTelemetry::failed(
                        QueryClass::VideoTitleCommand,
                        started_at_ms,
                        *failure,
                    )),
                }
                outcome
            }
            Err(failure) => Err(failure),
        }
    }

    async fn stored_video_title_command(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
    ) -> RepositoryResult<Option<StoredCommandRow>> {
        let statement = self.database.prepare(VIDEO_TITLE_COMMAND_SQL).bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(idempotency_key),
        ]);
        self.first(
            QueryClass::VideoTitleCommand,
            statement,
            |row: StoredCommandRow| {
                if row.command_type.is_empty()
                    || row.command_type.len() > 64
                    || !row.command_type.is_ascii()
                    || !valid_digest(&row.request_digest)
                    || safe_u64(row.expires_at_ms).is_err()
                    || row.response_status.is_some() != row.response_json.is_some()
                {
                    return Err(RepositoryFailure::CorruptResult);
                }
                Ok(row)
            },
        )
        .await
    }

    async fn classify_video_title_batch_failure(
        &self,
        command: &VideoTitleCommand,
        digest: &str,
    ) -> RepositoryResult<VideoTitleWriteOutcome> {
        if let Some(stored) = self
            .stored_video_title_command(&command.tenant_id, &command.idempotency_key)
            .await?
        {
            return classify_video_title_replay(stored, command, digest);
        }
        match self
            .video_for_mutation(&command.tenant_id, &command.video_id, &command.actor_id)
            .await
        {
            Ok(Some(video))
                if video.actor_can_update()
                    && u64::try_from(video.revision).ok() != Some(command.expected_revision) =>
            {
                Ok(VideoTitleWriteOutcome::Conflict)
            }
            Ok(None) => Ok(VideoTitleWriteOutcome::Conflict),
            Ok(Some(_)) => Err(RepositoryFailure::Unavailable),
            Err(failure) => Err(failure),
        }
    }

    async fn await_d1<T>(
        &self,
        future: impl Future<Output = worker::Result<T>>,
    ) -> RepositoryResult<T> {
        if self.query_timeout_ms == 0 {
            return Err(RepositoryFailure::Timeout);
        }
        let deadline = Delay::from(Duration::from_millis(self.query_timeout_ms));
        pin_mut!(future);
        pin_mut!(deadline);
        match select(future, deadline).await {
            Either::Left((result, _)) => result.map_err(|_| RepositoryFailure::Unavailable),
            Either::Right(((), _)) => Err(RepositoryFailure::Timeout),
        }
    }

    async fn first<T, U, Validate>(
        &self,
        query_class: QueryClass,
        statement: worker::Result<D1PreparedStatement>,
        validate: Validate,
    ) -> RepositoryResult<Option<U>>
    where
        T: DeserializeOwned,
        Validate: FnOnce(T) -> RepositoryResult<U>,
    {
        let started_at_ms = js_sys::Date::now();
        let statement = match statement {
            Ok(statement) => statement,
            Err(_) => {
                let failure = RepositoryFailure::Unavailable;
                emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                return Err(failure);
            }
        };
        match self
            .await_d1(statement.first::<serde_json::Value>(None))
            .await
        {
            Ok(Some(value)) => match decode_row(value).and_then(validate) {
                Ok(value) => {
                    emit_telemetry(&QueryTelemetry::completed(query_class, started_at_ms, 1));
                    Ok(Some(value))
                }
                Err(failure) => {
                    emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                    Err(failure)
                }
            },
            Ok(None) => {
                emit_telemetry(&QueryTelemetry::completed(query_class, started_at_ms, 0));
                Ok(None)
            }
            Err(failure) => {
                emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                Err(failure)
            }
        }
    }

    async fn video_items(
        &self,
        query_class: QueryClass,
        statement: worker::Result<D1PreparedStatement>,
        max_rows: usize,
    ) -> RepositoryResult<Vec<VideoPageItem>> {
        let started_at_ms = js_sys::Date::now();
        let statement = match statement {
            Ok(statement) => statement,
            Err(_) => {
                let failure = RepositoryFailure::Unavailable;
                emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                return Err(failure);
            }
        };
        let raw_rows = match self.await_d1(statement.raw::<serde_json::Value>()).await {
            Ok(rows) => rows,
            Err(failure) => {
                emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                return Err(failure);
            }
        };
        let items = raw_rows
            .into_iter()
            .map(|values| parse_video_page_row(values).and_then(VideoPageItem::try_from))
            .collect::<RepositoryResult<Vec<_>>>();
        let items = match items {
            Ok(items) if items.len() <= max_rows => items,
            Ok(_) => {
                let failure = RepositoryFailure::CorruptResult;
                emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                return Err(failure);
            }
            Err(failure) => {
                emit_telemetry(&QueryTelemetry::failed(query_class, started_at_ms, failure));
                return Err(failure);
            }
        };
        emit_telemetry(&QueryTelemetry::completed(
            query_class,
            started_at_ms,
            items.len(),
        ));
        Ok(items)
    }
}

fn validate_video_title_command(command: &VideoTitleCommand) -> RepositoryResult<()> {
    validate_ids(&[
        &command.tenant_id,
        &command.video_id,
        &command.actor_id,
        &command.reservation_id,
        &command.outbox_id,
        &command.operation_id,
    ])?;
    let infrastructure_ids = [
        command.reservation_id.as_str(),
        command.outbox_id.as_str(),
        command.operation_id.as_str(),
    ];
    if !valid_idempotency_key(&command.idempotency_key)
        || command.title.trim().is_empty()
        || command.title.trim() != command.title
        || command.title.chars().count() > 160
        || command.title.chars().any(char::is_control)
        || command.expected_revision > MAX_SAFE_INTEGER
        || command.now_ms > MAX_SAFE_INTEGER
        || infrastructure_ids.iter().collect::<BTreeSet<_>>().len() != infrastructure_ids.len()
    {
        return Err(RepositoryFailure::InvalidRequest);
    }
    command
        .expected_revision
        .checked_add(1)
        .filter(|revision| *revision <= MAX_SAFE_INTEGER)
        .ok_or(RepositoryFailure::InvalidRequest)?;
    command
        .now_ms
        .checked_add(u64::try_from(COMMAND_TTL_MS).unwrap_or(0))
        .filter(|expires_at| *expires_at <= MAX_SAFE_INTEGER)
        .ok_or(RepositoryFailure::InvalidRequest)?;
    Ok(())
}

fn classify_video_title_replay(
    stored: StoredCommandRow,
    command: &VideoTitleCommand,
    digest: &str,
) -> RepositoryResult<VideoTitleWriteOutcome> {
    if stored.command_type != VIDEO_TITLE_COMMAND_TYPE
        || stored.request_digest != digest
        || u64::try_from(stored.expires_at_ms)
            .ok()
            .is_none_or(|expires_at| expires_at > MAX_SAFE_INTEGER || expires_at < command.now_ms)
    {
        return Ok(VideoTitleWriteOutcome::Conflict);
    }
    match (stored.response_status, stored.response_json) {
        (Some(200), Some(json)) => {
            let response = serde_json::from_str::<VideoTitleResult>(&json)
                .map_err(|_| RepositoryFailure::CorruptResult)?;
            let expected_revision = command
                .expected_revision
                .checked_add(1)
                .ok_or(RepositoryFailure::CorruptResult)?;
            if response.schema_version != API_SCHEMA_VERSION
                || response.video_id != command.video_id
                || response.title != command.title
                || response.revision != expected_revision
            {
                return Err(RepositoryFailure::CorruptResult);
            }
            Ok(VideoTitleWriteOutcome::Replay(response))
        }
        (None, None) => Ok(VideoTitleWriteOutcome::Conflict),
        _ => Err(RepositoryFailure::CorruptResult),
    }
}

fn decode_row<T: DeserializeOwned>(value: serde_json::Value) -> RepositoryResult<T> {
    serde_json::from_value(value).map_err(|_| RepositoryFailure::CorruptResult)
}

fn parse_video_page_row(values: Vec<serde_json::Value>) -> RepositoryResult<VideoPageRow> {
    if values.len() != 7 {
        return Err(RepositoryFailure::CorruptResult);
    }
    let mut values = values.into_iter();
    Ok(VideoPageRow {
        id: raw_string(values.next())?,
        title: raw_string(values.next())?,
        state: raw_string(values.next())?,
        privacy: raw_string(values.next())?,
        revision: raw_i64(values.next())?,
        created_at_ms: raw_i64(values.next())?,
        updated_at_ms: raw_i64(values.next())?,
    })
}

fn raw_string(value: Option<serde_json::Value>) -> RepositoryResult<String> {
    value
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(RepositoryFailure::CorruptResult)
}

fn raw_i64(value: Option<serde_json::Value>) -> RepositoryResult<i64> {
    value
        .and_then(|value| value.as_i64())
        .ok_or(RepositoryFailure::CorruptResult)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_private_object_key(value: &str, tenant_id: &str, video_id: &str) -> bool {
    value.len() <= 1_024
        && value.starts_with(&format!("tenants/{tenant_id}/videos/{video_id}/"))
        && !value.contains("..")
        && !value.contains(['\\', '?', '#', '%'])
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn valid_derivative_object_key(
    value: &str,
    tenant_id: &str,
    video_id: &str,
    profile: &str,
) -> bool {
    let prefix = format!("tenants/{tenant_id}/videos/{video_id}/derivatives/{profile}/");
    value.strip_prefix(&prefix).is_some_and(valid_digest)
        && valid_private_object_key(value, tenant_id, video_id)
}

fn valid_media_job_state(value: &str) -> bool {
    matches!(
        value,
        "queued" | "leased" | "running" | "succeeded" | "failed" | "cancelled"
    )
}

fn valid_media_profile(value: &str) -> bool {
    matches!(
        value,
        "optimized_clip_v1"
            | "thumbnail_v1"
            | "spritesheet_v1"
            | "audio_extract_v1"
            | "probe_v1"
            | "audio_presence_v1"
            | "distribution_master_v1"
            | "animated_preview_v1"
            | "audio_normalize_v1"
            | "remux_repair_v1"
            | "segment_mux_v1"
            | "waveform_v1"
            | "composition_v1"
            | "normalize_v1"
            | "transcription_v1"
            | "ai_cleanup_v1"
            | "preview_v1"
            | "audio_v1"
    )
}

fn valid_media_error_class(value: &str) -> bool {
    matches!(
        value,
        "input_invalid"
            | "unsupported_media"
            | "pipeline_timeout"
            | "pipeline_failure"
            | "resource_limit"
            | "output_invalid"
            | "cancelled"
            | "transport_failure"
            | "fake_executor_failure"
            | "lease_expired"
    )
}

fn validate_ids(values: &[&str]) -> RepositoryResult<()> {
    if values.iter().all(|value| valid_uuid(value)) {
        Ok(())
    } else {
        Err(RepositoryFailure::InvalidRequest)
    }
}

pub fn parameter_chunk_ranges(
    item_count: usize,
    fixed_parameters: usize,
) -> RepositoryResult<Vec<Range<usize>>> {
    if item_count > MAX_BULK_IDENTIFIERS || fixed_parameters >= MAX_D1_BOUND_PARAMETERS {
        return Err(RepositoryFailure::InvalidRequest);
    }
    let chunk_size = MAX_D1_BOUND_PARAMETERS - fixed_parameters;
    Ok((0..item_count)
        .step_by(chunk_size)
        .map(|start| start..item_count.min(start + chunk_size))
        .collect())
}

pub fn video_bulk_sql(identifier_count: usize) -> RepositoryResult<String> {
    if identifier_count == 0 || identifier_count + 1 > MAX_D1_BOUND_PARAMETERS {
        return Err(RepositoryFailure::InvalidRequest);
    }
    let placeholders = (0..identifier_count)
        .map(|index| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "SELECT id, title, state, privacy, revision, created_at_ms, updated_at_ms \
         FROM videos WHERE organization_id = ?1 AND deleted_at_ms IS NULL \
         AND id IN ({placeholders}) ORDER BY created_at_ms DESC, id DESC"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900104";

    #[test]
    fn cursor_is_versioned_canonical_and_bounded() {
        let cursor = PageCursor::new(
            1_700_000_000_123,
            Uuid::parse_str(ID).expect("fixture uuid"),
        )
        .expect("cursor");
        let encoded = cursor.encode();
        assert_eq!(encoded.len(), CURSOR_BYTES * 2);
        assert_eq!(PageCursor::decode(&encoded), Ok(cursor.clone()));

        let mut uppercase = encoded.clone();
        uppercase.replace_range(0..2, "0A");
        assert_eq!(
            PageCursor::decode(&uppercase),
            Err(RepositoryFailure::InvalidRequest)
        );
        assert_eq!(
            PageCursor::decode("00"),
            Err(RepositoryFailure::InvalidRequest)
        );
        assert!(PageCursor::new(MAX_SAFE_INTEGER + 1, cursor.id()).is_err());
        assert!(PageCursor::new(1, Uuid::nil()).is_err());
        assert!(VideoPageRequest::new(Some(0), None).is_err());
        assert!(VideoPageRequest::new(Some(MAX_PAGE_SIZE + 1), None).is_err());
    }

    #[test]
    fn parameter_chunks_never_cross_the_d1_binding_limit() {
        let chunks = parameter_chunk_ranges(MAX_BULK_IDENTIFIERS, 1).expect("chunks");
        assert_eq!(chunks.len(), 11);
        assert_eq!(chunks.first(), Some(&(0..99)));
        assert_eq!(chunks.last(), Some(&(990..1_000)));
        assert!(
            chunks
                .iter()
                .all(|range| range.len() < MAX_D1_BOUND_PARAMETERS)
        );
        assert!(parameter_chunk_ranges(MAX_BULK_IDENTIFIERS + 1, 1).is_err());
        assert!(parameter_chunk_ranges(1, MAX_D1_BOUND_PARAMETERS).is_err());
    }

    #[test]
    fn generated_bulk_sql_contains_only_positional_placeholders() {
        let marker = "018f47a6'; DELETE FROM videos; --";
        let sql = video_bulk_sql(3).expect("query");
        assert!(sql.contains("id IN (?2, ?3, ?4)"));
        assert!(!sql.contains(marker));
        assert_eq!(sql.matches('?').count(), 4);
        assert!(video_bulk_sql(0).is_err());
        assert!(video_bulk_sql(MAX_D1_BOUND_PARAMETERS).is_err());
    }

    #[test]
    fn raw_page_rows_fail_closed_without_panicking() {
        let valid = vec![
            serde_json::Value::String(ID.into()),
            serde_json::Value::String("Bounded title".into()),
            serde_json::Value::String("ready".into()),
            serde_json::Value::String("private".into()),
            serde_json::json!(4),
            serde_json::json!(1_700_000_000_000_i64),
            serde_json::json!(1_700_000_000_001_i64),
        ];
        assert_eq!(
            parse_video_page_row(valid).expect("row").created_at_ms,
            1_700_000_000_000
        );
        assert!(matches!(
            parse_video_page_row(vec![serde_json::Value::String(ID.into())]),
            Err(RepositoryFailure::CorruptResult)
        ));
    }

    #[test]
    fn typed_rows_distinguish_persisted_corruption_from_provider_failure() {
        let malformed = serde_json::json!({
            "id": ID,
            "organization_id": "not-a-uuid",
            "video_id": ID,
            "state": "complete",
            "expected_bytes": "forty-two",
            "received_bytes": 42,
            "source_object_key": "redacted",
            "source_version": 1,
            "content_type": "video/webm",
            "checksum_sha256": null
        });
        assert!(matches!(
            decode_row::<UploadRow>(malformed),
            Err(RepositoryFailure::CorruptResult)
        ));
    }

    #[test]
    fn persisted_job_attempts_must_fit_the_public_u32_contract() {
        let oversized_attempt = i64::from(u32::MAX) + 1;
        let media = MediaJobRow {
            id: ID.into(),
            state: "queued".into(),
            profile: "thumbnail_v1".into(),
            selected_executor: Some("native_gstreamer".into()),
            progress_basis_points: Some(0),
            attempt: oversized_attempt,
            cancel_requested: 0,
            error_class: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        };
        assert!(matches!(
            media.validated(ID),
            Err(RepositoryFailure::CorruptResult)
        ));

        let tenant_id = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
        let worker = WorkerJobRow {
            id: ID.into(),
            video_id: ID.into(),
            state: "queued".into(),
            revision: 0,
            attempt: oversized_attempt,
            profile: "thumbnail_v1".into(),
            source_version: 1,
            output_object_key: format!(
                "tenants/{tenant_id}/videos/{ID}/derivatives/thumbnail_v1/{}",
                "0".repeat(64)
            ),
            worker_id: None,
            lease_token_digest: None,
            lease_expires_at_ms: None,
            progress_basis_points: Some(0),
            cancel_requested: 0,
        };
        assert!(matches!(
            worker.validated(tenant_id, ID),
            Err(RepositoryFailure::CorruptResult)
        ));
    }

    #[test]
    fn telemetry_and_errors_are_stable_and_redacted() {
        let telemetry = QueryTelemetry {
            event: "d1_repository_query",
            query_class: QueryClass::VideoPage,
            duration_ms: 17,
            rows: 2,
            retries: 0,
            bookmark_use: "unavailable_in_workers_binding",
            outcome: RepositoryFailure::Timeout.code(),
        };
        let json = telemetry.safe_json();
        assert!(json.contains("\"query_class\":\"video_page\""));
        assert!(json.contains("\"outcome\":\"repository_timeout\""));
        for forbidden in [ID, "SELECT", "tenant_id", "frame-local"] {
            assert!(!json.contains(forbidden));
        }
        assert!(RepositoryFailure::Timeout.retryable());
        assert!(!RepositoryFailure::InvalidRequest.retryable());
    }

    #[test]
    fn checked_in_queries_are_parameterized_and_tenant_scoped() {
        for sql in [
            VIDEO_FOR_MUTATION_SQL,
            UPLOAD_BY_ID_SQL,
            MEDIA_JOB_BY_ID_SQL,
            NATIVE_WORKER_JOB_BY_ID_SQL,
            ORGANIZATION_SNAPSHOT_SQL,
            VIDEO_PAGE_SQL,
            VIDEO_PAGE_AFTER_SQL,
            VIDEO_TITLE_COMMAND_SQL,
            VIDEO_TITLE_APPLY_SQL,
        ] {
            assert!(sql.contains("?1"));
            assert!(!sql.contains(ID));
            assert!(!sql.contains(';'));
        }
        for sql in [
            VIDEO_FOR_MUTATION_SQL,
            UPLOAD_BY_ID_SQL,
            MEDIA_JOB_BY_ID_SQL,
            NATIVE_WORKER_JOB_BY_ID_SQL,
            VIDEO_PAGE_SQL,
            VIDEO_PAGE_AFTER_SQL,
            VIDEO_TITLE_COMMAND_SQL,
            VIDEO_TITLE_APPLY_SQL,
        ] {
            assert!(sql.contains("organization_id"));
        }
    }

    #[test]
    fn video_title_commands_bind_actor_into_validation_digest_and_replay() {
        let command = VideoTitleCommand {
            tenant_id: "018f47a6-7b1c-7f55-8f39-8f8a86900102".into(),
            video_id: ID.into(),
            actor_id: "018f47a6-7b1c-7f55-8f39-8f8a86900101".into(),
            idempotency_key: "repository-command-0001".into(),
            expected_revision: 4,
            title: "Repository \"Applied\" – O'Brien".into(),
            now_ms: 1_700_100_000_000,
            reservation_id: "018f47a6-7b1c-7f55-8f39-8f8a86904101".into(),
            outbox_id: "018f47a6-7b1c-7f55-8f39-8f8a86904102".into(),
            operation_id: "018f47a6-7b1c-7f55-8f39-8f8a86904103".into(),
        };
        assert_eq!(validate_video_title_command(&command), Ok(()));
        let mut padded_title = command.clone();
        padded_title.title = format!(" {} ", command.title);
        assert_eq!(
            validate_video_title_command(&padded_title),
            Err(RepositoryFailure::InvalidRequest)
        );
        let digest = request_digest(
            VIDEO_TITLE_COMMAND_TYPE,
            &VideoTitleDigest {
                tenant_id: &command.tenant_id,
                video_id: &command.video_id,
                actor_id: &command.actor_id,
                expected_revision: command.expected_revision,
                title: &command.title,
            },
        )
        .expect("digest");
        let other_actor_digest = request_digest(
            VIDEO_TITLE_COMMAND_TYPE,
            &VideoTitleDigest {
                tenant_id: &command.tenant_id,
                video_id: &command.video_id,
                actor_id: "018f47a6-7b1c-7f55-8f39-8f8a86900999",
                expected_revision: command.expected_revision,
                title: &command.title,
            },
        )
        .expect("digest");
        assert_ne!(digest, other_actor_digest);

        let response = VideoTitleResult {
            schema_version: API_SCHEMA_VERSION,
            video_id: command.video_id.clone(),
            title: command.title.clone(),
            revision: 5,
        };
        let stored = StoredCommandRow {
            command_type: VIDEO_TITLE_COMMAND_TYPE.into(),
            request_digest: digest.clone(),
            response_status: Some(200),
            response_json: Some(serde_json::to_string(&response).expect("response")),
            expires_at_ms: 1_700_186_400_000,
        };
        assert_eq!(
            classify_video_title_replay(stored, &command, &digest),
            Ok(VideoTitleWriteOutcome::Replay(response))
        );
    }

    #[test]
    fn migration_trigger_guards_every_video_title_mutation() {
        let migration = include_str!("../migrations/0008_repository_query_indexes.sql");
        assert!(migration.contains("CREATE TRIGGER repository_video_title_apply"));
        assert!(!migration.contains("repository_video_title_cleanup"));
        assert_eq!(migration.matches("changes() = 1").count(), 4);
        assert!(migration.contains("RAISE(ABORT"));
        assert!(migration.contains("SELECT RAISE(IGNORE);"));
        assert!(migration.contains("repository_video_title_operations"));
    }

    #[test]
    fn video_update_policy_requires_ownership_or_space_management_for_members() {
        let actor = "018f47a6-7b1c-7f55-8f39-8f8a86900101";
        let mut row = VideoMutationRow {
            id: ID.into(),
            owner_id: "018f47a6-7b1c-7f55-8f39-8f8a86900109".into(),
            state: "ready".into(),
            privacy: "private".into(),
            revision: 3,
            actor_role: "member".into(),
            actor_manages_space: 0,
            queried_actor_id: actor.into(),
        };
        assert!(!row.actor_can_update());
        row.owner_id = actor.into();
        assert!(row.actor_can_update());
        row.owner_id = "018f47a6-7b1c-7f55-8f39-8f8a86900109".into();
        row.actor_manages_space = 1;
        assert!(row.actor_can_update());
        row.actor_role = "viewer".into();
        assert!(!row.actor_can_update());
        row.actor_role = "admin".into();
        assert!(row.actor_can_update());
    }

    #[test]
    fn aggregate_rows_reject_cross_tenant_keys_and_unbounded_diagnostics() {
        let tenant = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
        let upload_id = "018f47a6-7b1c-7f55-8f39-8f8a86900111";
        let upload = || UploadRow {
            id: upload_id.into(),
            organization_id: tenant.into(),
            video_id: ID.into(),
            state: "complete".into(),
            expected_bytes: 42,
            received_bytes: 42,
            source_object_key: format!("tenants/{tenant}/videos/{ID}/source/v1/payload"),
            source_version: 1,
            content_type: "video/webm".into(),
            checksum_sha256: Some("a".repeat(64)),
            transfer_mode: "brokered".into(),
            direct_staging_key: None,
            direct_checksum_sha256: None,
            direct_expires_at_ms: None,
        };
        assert!(upload().validated(tenant, upload_id).is_ok());

        let mut foreign_key = upload();
        foreign_key.source_object_key =
            format!("tenants/018f47a6-7b1c-7f55-8f39-8f8a86900999/videos/{ID}/source/v1/payload");
        assert_eq!(
            foreign_key.validated(tenant, upload_id).err(),
            Some(RepositoryFailure::CorruptResult)
        );

        let job = MediaJobRow {
            id: "018f47a6-7b1c-7f55-8f39-8f8a86900112".into(),
            state: "failed".into(),
            profile: "thumbnail_v1".into(),
            selected_executor: Some("native_gstreamer".into()),
            progress_basis_points: Some(4_200),
            attempt: 1,
            cancel_requested: 0,
            error_class: Some("internal stack trace".into()),
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_001,
        };
        assert_eq!(
            job.validated("018f47a6-7b1c-7f55-8f39-8f8a86900112").err(),
            Some(RepositoryFailure::CorruptResult)
        );
    }
}
