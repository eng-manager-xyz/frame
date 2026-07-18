//! D1/R2 authority for Cap's public developer API, recorder SDK, and daily
//! storage billing cron. D1 claims every SDK/REST mutation before provider
//! effects; completion/abort can safely continue from `effect_pending`, while
//! a lost multipart-create bind fails closed for operator reconciliation.

use async_trait::async_trait;
use frame_application::{
    LEGACY_DEVELOPER_MAX_DURATION_SECONDS, LEGACY_DEVELOPER_MICROCREDITS_PER_MINUTE,
    LEGACY_DEVELOPER_MIN_BALANCE_MICROCREDITS, LEGACY_DEVELOPER_MULTIPART_URL_TTL_SECONDS,
    LEGACY_DEVELOPER_STORAGE_RATE_DENOMINATOR, LEGACY_DEVELOPER_STORAGE_RATE_NUMERATOR,
    LegacyDeveloperApiCommandV1, LegacyDeveloperApiInputV1, LegacyDeveloperApiOutcomeV1,
    LegacyDeveloperApiPortErrorV1, LegacyDeveloperApiPortV1, LegacyDeveloperApiResultV1,
    LegacyDeveloperPartV1, LegacyDeveloperUsageV1, LegacyDeveloperVideoStatusV1,
    LegacyDeveloperVideoV1,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{
    Bucket, D1Database, D1PreparedStatement, D1Result, HttpMetadata, UploadedPart,
    send::IntoSendFuture,
};

use crate::r2_direct_upload::R2DirectPutSigner;

const AUTH_KEY_SQL: &str = include_str!("../queries/legacy_developer_api/auth_key.sql");
const AUTH_ORIGIN_SQL: &str = include_str!("../queries/legacy_developer_api/auth_origin.sql");
const AUTH_TOUCH_SQL: &str = include_str!("../queries/legacy_developer_api/auth_touch.sql");
const USAGE_READ_SQL: &str = include_str!("../queries/legacy_developer_api/usage_read.sql");
const VIDEOS_LIST_SQL: &str = include_str!("../queries/legacy_developer_api/videos_list.sql");
const VIDEO_READ_SQL: &str = include_str!("../queries/legacy_developer_api/video_read.sql");
const ACCOUNT_READ_SQL: &str = include_str!("../queries/legacy_developer_api/account_read.sql");
const OPERATION_READ_SQL: &str = include_str!("../queries/legacy_developer_api/operation_read.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_developer_api/operation_claim.sql");
const OPERATION_EFFECT_PENDING_SQL: &str =
    include_str!("../queries/legacy_developer_api/operation_effect_pending.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_developer_api/operation_complete.sql");
const RECEIPT_INSERT_SQL: &str = include_str!("../queries/legacy_developer_api/receipt_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_developer_api/audit_insert.sql");
const VIDEO_CREATE_SQL: &str = include_str!("../queries/legacy_developer_api/video_create.sql");
const VIDEO_DELETE_SQL: &str = include_str!("../queries/legacy_developer_api/video_delete.sql");
const MULTIPART_SESSION_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_api/multipart_session_insert.sql");
const MULTIPART_SESSION_READ_SQL: &str =
    include_str!("../queries/legacy_developer_api/multipart_session_read.sql");
const MULTIPART_STATE_SQL: &str =
    include_str!("../queries/legacy_developer_api/multipart_state.sql");
const CREDIT_DEBIT_SQL: &str = include_str!("../queries/legacy_developer_api/credit_debit.sql");
const VIDEO_COMPLETE_SQL: &str = include_str!("../queries/legacy_developer_api/video_complete.sql");
const PART_CAPABILITY_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_api/part_capability_insert.sql");
const OUTBOX_INSERT_SQL: &str = include_str!("../queries/legacy_developer_api/outbox_insert.sql");
const OUTBOX_ATTEMPT_SQL: &str = include_str!("../queries/legacy_developer_api/outbox_attempt.sql");
const OUTBOX_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_developer_api/outbox_complete.sql");
const CRON_RUN_READ_SQL: &str = include_str!("../queries/legacy_developer_api/cron_run_read.sql");
const CRON_CANDIDATES_SQL: &str =
    include_str!("../queries/legacy_developer_api/cron_candidates.sql");
const CRON_SNAPSHOT_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_api/cron_snapshot_insert.sql");
const CRON_RUN_INSERT_SQL: &str =
    include_str!("../queries/legacy_developer_api/cron_run_insert.sql");

const OPERATION_UNIQUE: &str = "UNIQUE constraint failed: legacy_developer_api_operations_v1";
const INSUFFICIENT: &str = "frame_legacy_developer_insufficient_credits_v1";
const CAP_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

type PortResult<T> = Result<T, LegacyDeveloperApiPortErrorV1>;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyDeveloperAuthRowV1 {
    pub app_id: String,
    pub environment: String,
}

#[derive(Debug, Deserialize)]
struct AllowedRow {
    allowed: i64,
}

#[derive(Debug, Deserialize)]
struct UsageRow {
    balance_microcredits: i64,
    total_videos: i64,
    total_duration_minutes: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct VideoRow {
    #[serde(default)]
    native_video_id: Option<String>,
    legacy_video_id: String,
    legacy_app_id: String,
    external_user_id: Option<String>,
    name: String,
    duration: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    fps: Option<f64>,
    s3_key: Option<String>,
    transcription_status: Option<String>,
    metadata_json: Option<String>,
    deleted_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct AccountRow {
    id: String,
    balance_microcredits: i64,
    legacy_app_id: String,
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    status: Option<i64>,
    result_kind: Option<String>,
    result_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MultipartRow {
    provider_upload_id: String,
    object_key: String,
    state: String,
    native_video_id: String,
}

#[derive(Debug, Deserialize)]
struct CronRunRow {
    snapshot_date: String,
    apps_processed: i64,
}

#[derive(Debug, Deserialize)]
struct CronCandidateRow {
    app_id: String,
    account_id: Option<String>,
    balance_microcredits: Option<i64>,
    total_duration_minutes: f64,
    video_count: i64,
}

struct MultipartCompletionArgsV1<'a> {
    video_id: &'a str,
    upload_id: &'a str,
    parts: Vec<LegacyDeveloperPartV1>,
    duration_seconds: f64,
    width: Option<f64>,
    height: Option<f64>,
    fps: Option<f64>,
}

pub(crate) struct D1LegacyDeveloperApiPortV1<'a> {
    database: &'a D1Database,
    bucket: &'a Bucket,
    signer: Option<&'a R2DirectPutSigner>,
    web_origin: &'a str,
}

impl<'a> D1LegacyDeveloperApiPortV1<'a> {
    #[must_use]
    pub(crate) const fn new(
        database: &'a D1Database,
        bucket: &'a Bucket,
        signer: Option<&'a R2DirectPutSigner>,
        web_origin: &'a str,
    ) -> Self {
        Self {
            database,
            bucket,
            signer,
            web_origin,
        }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Unavailable)
    }

    async fn rows<T>(&self, sql: &str, bindings: Vec<JsValue>) -> PortResult<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> PortResult<Vec<D1Result>> {
        let expected = statements.len();
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyDeveloperApiPortErrorV1::Unavailable);
        }
        if let Some(result) = results.iter().find(|result| !result.success()) {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        Ok(results)
    }

    pub(crate) async fn authenticate_key(
        &self,
        key_digest: &str,
        key_kind: &str,
        now_ms: i64,
    ) -> PortResult<Option<LegacyDeveloperAuthRowV1>> {
        let row = self
            .rows::<LegacyDeveloperAuthRowV1>(
                AUTH_KEY_SQL,
                vec![JsValue::from_str(key_digest), JsValue::from_str(key_kind)],
            )
            .await?
            .into_iter()
            .next();
        if row.is_some() {
            let _ = self
                .statement(
                    AUTH_TOUCH_SQL,
                    vec![
                        JsValue::from_str(key_digest),
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?
                .run()
                .into_send()
                .await;
        }
        Ok(row)
    }

    pub(crate) async fn origin_allowed(&self, app_id: &str, origin: &str) -> PortResult<bool> {
        Ok(self
            .rows::<AllowedRow>(
                AUTH_ORIGIN_SQL,
                vec![JsValue::from_str(app_id), JsValue::from_str(origin)],
            )
            .await?
            .into_iter()
            .next()
            .is_some_and(|row| row.allowed == 1))
    }

    async fn prior_operation(
        &self,
        command: &LegacyDeveloperApiCommandV1,
    ) -> PortResult<Option<OperationRow>> {
        let app_id = command
            .app_id()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let key = command
            .idempotency_key_digest()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
        self.rows::<OperationRow>(
            OPERATION_READ_SQL,
            vec![
                JsValue::from_str(command.input().surface().operation_id()),
                JsValue::from_str(app_id),
                JsValue::from_str(&key),
            ],
        )
        .await
        .map(|rows| rows.into_iter().next())
    }

    fn claim_statement(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        target: Option<&str>,
        now_ms: i64,
    ) -> PortResult<D1PreparedStatement> {
        self.statement(
            OPERATION_CLAIM_SQL,
            vec![
                JsValue::from_str(
                    &command
                        .operation_id()
                        .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?
                        .to_string(),
                ),
                JsValue::from_str(command.input().surface().operation_id()),
                JsValue::from_str(
                    command
                        .app_id()
                        .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
                ),
                optional_text(target),
                JsValue::from_str(
                    &command
                        .idempotency_key_digest()
                        .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
                ),
                JsValue::from_str(command.request_digest()),
                JsValue::from_f64(now_ms as f64),
            ],
        )
    }

    fn durable_result_statements(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        operation_id: &str,
        result_kind: &str,
        result_json: &str,
        target: &str,
        now_ms: i64,
    ) -> PortResult<Vec<D1PreparedStatement>> {
        let result_digest = digest(result_json.as_bytes());
        Ok(vec![
            self.statement(
                RECEIPT_INSERT_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(200.0),
                    JsValue::from_str(result_kind),
                    JsValue::from_str(result_json),
                    JsValue::from_str(&result_digest),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    JsValue::from_str(&Uuid::now_v7().to_string()),
                    JsValue::from_str(operation_id),
                    JsValue::from_str(command.input().surface().operation_id()),
                    JsValue::from_str(&digest(command.app_id().unwrap_or_default().as_bytes())),
                    JsValue::from_str(&digest(target.as_bytes())),
                    JsValue::from_str(command.request_digest()),
                    JsValue::from_str(&result_digest),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![
                    JsValue::from_str(operation_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
        ])
    }

    async fn create_video(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        name: Option<String>,
        external_user_id: Option<String>,
        metadata: Option<Value>,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        if let Some(prior) = self.prior_operation(command).await? {
            return replay_or_pending(prior, command.request_digest());
        }
        let app_id = command
            .app_id()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let account = self
            .rows::<AccountRow>(ACCOUNT_READ_SQL, vec![JsValue::from_str(app_id)])
            .await?
            .into_iter()
            .next()
            .ok_or(LegacyDeveloperApiPortErrorV1::InsufficientCredits)?;
        if account.balance_microcredits < LEGACY_DEVELOPER_MIN_BALANCE_MICROCREDITS {
            return Err(LegacyDeveloperApiPortErrorV1::InsufficientCredits);
        }
        let now_ms = now_ms()?;
        let native_video_id = Uuid::now_v7().to_string();
        let legacy_video_id = random_cap_id(15)?;
        let s3_key = format!(
            "developer/{}/{legacy_video_id}/video",
            account.legacy_app_id
        );
        let share_url = format!(
            "{}/dev/{legacy_video_id}",
            self.web_origin.trim_end_matches('/')
        );
        let embed_url = format!(
            "{}/embed/{legacy_video_id}?sdk=1",
            self.web_origin.trim_end_matches('/')
        );
        let body = json!({
            "videoId": &legacy_video_id,
            "s3Key": &s3_key,
            "shareUrl": &share_url,
            "embedUrl": &embed_url,
        });
        let result_json =
            serde_json::to_string(&body).map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let operation_id = mutation_operation_id(command)?;
        let mut statements = vec![
            self.claim_statement(command, Some(&legacy_video_id), now_ms)?,
            self.statement(
                VIDEO_CREATE_SQL,
                vec![
                    JsValue::from_str(&native_video_id),
                    JsValue::from_str(&legacy_video_id),
                    JsValue::from_str(app_id),
                    optional_text(external_user_id.as_deref()),
                    JsValue::from_str(name.as_deref().unwrap_or("Untitled")),
                    JsValue::from_str(&s3_key),
                    optional_text(
                        metadata
                            .as_ref()
                            .and_then(|value| serde_json::to_string(value).ok())
                            .as_deref(),
                    ),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str(&operation_id),
                ],
            )?,
        ];
        statements.extend(self.durable_result_statements(
            command,
            &operation_id,
            "video_created",
            &result_json,
            &legacy_video_id,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyDeveloperApiOutcomeV1 {
            result: LegacyDeveloperApiResultV1::VideoCreated {
                video_id: legacy_video_id,
                s3_key,
                share_url,
                embed_url,
            },
            replayed: false,
        })
    }

    async fn initiate(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        video_id: &str,
        content_type: Option<&str>,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        if let Some(prior) = self.prior_operation(command).await? {
            return replay_or_pending(prior, command.request_digest());
        }
        let app_id = command
            .app_id()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let video = self.video_row(app_id, video_id).await?;
        let native_video_id = video
            .native_video_id
            .as_deref()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let object_key = video
            .s3_key
            .as_deref()
            .ok_or(LegacyDeveloperApiPortErrorV1::NoStorageKey)?;
        let content_type = normalized_content_type(content_type);
        let now_ms = now_ms()?;
        let operation_id = mutation_operation_id(command)?;
        self.batch(vec![
            self.claim_statement(command, Some(video_id), now_ms)?,
            self.statement(
                OPERATION_EFFECT_PENDING_SQL,
                vec![JsValue::from_str(&operation_id)],
            )?,
            self.statement(
                OUTBOX_INSERT_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_str("multipart_create"),
                    JsValue::from_str(command.request_digest()),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
        ])
        .await?;
        let upload = self
            .bucket
            .create_multipart_upload(object_key)
            .http_metadata(HttpMetadata {
                content_type: Some(content_type.into()),
                cache_control: Some("private, no-store".into()),
                ..HttpMetadata::default()
            })
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Provider)?;
        let upload_id = upload.upload_id().into_send().await;
        let result_json = serde_json::to_string(&json!({"uploadId": upload_id}))
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let mut statements = vec![
            self.statement(
                MULTIPART_SESSION_INSERT_SQL,
                vec![
                    JsValue::from_str(&upload_id),
                    JsValue::from_str(app_id),
                    JsValue::from_str(native_video_id),
                    JsValue::from_str(object_key),
                    JsValue::from_str(content_type),
                    JsValue::from_str(&operation_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
            self.statement(
                OUTBOX_COMPLETE_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
        ];
        statements.extend(self.durable_result_statements(
            command,
            &operation_id,
            "upload_initiated",
            &result_json,
            video_id,
            now_ms,
        )?);
        if let Err(error) = self.batch(statements).await {
            let _ = upload.abort().into_send().await;
            return Err(error);
        }
        Ok(LegacyDeveloperApiOutcomeV1 {
            result: LegacyDeveloperApiResultV1::UploadInitiated { upload_id },
            replayed: false,
        })
    }

    async fn presign(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        video_id: &str,
        upload_id: &str,
        part_number: u16,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        if let Some(prior) = self.prior_operation(command).await? {
            return replay_or_pending(prior, command.request_digest());
        }
        let session = self.session(command, video_id, upload_id).await?;
        if session.state != "open" {
            return Err(LegacyDeveloperApiPortErrorV1::Conflict);
        }
        let now_ms = now_ms()?;
        let capability = self
            .signer
            .ok_or(LegacyDeveloperApiPortErrorV1::Unavailable)?
            .sign_legacy_multipart_part(
                &session.object_key,
                &session.provider_upload_id,
                part_number,
                u64::try_from(now_ms).map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?,
                LEGACY_DEVELOPER_MULTIPART_URL_TTL_SECONDS,
            )
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Unavailable)?;
        let result_json = serde_json::to_string(&json!({"presignedUrl": capability.url}))
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?;
        let operation_id = mutation_operation_id(command)?;
        let mut statements = vec![
            self.claim_statement(command, Some(video_id), now_ms)?,
            self.statement(
                PART_CAPABILITY_INSERT_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(upload_id),
                    JsValue::from_f64(f64::from(part_number)),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_f64(capability.expires_at_ms as f64),
                ],
            )?,
        ];
        statements.extend(self.durable_result_statements(
            command,
            &operation_id,
            "part_presigned",
            &result_json,
            video_id,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(LegacyDeveloperApiOutcomeV1 {
            result: LegacyDeveloperApiResultV1::PartPresigned {
                presigned_url: capability.url,
            },
            replayed: false,
        })
    }

    async fn complete(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        input: MultipartCompletionArgsV1<'_>,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        let MultipartCompletionArgsV1 {
            video_id,
            upload_id,
            mut parts,
            duration_seconds,
            width,
            height,
            fps,
        } = input;
        let prior = self.prior_operation(command).await?;
        if let Some(row) = prior.as_ref() {
            if row.request_digest != command.request_digest() {
                return Err(LegacyDeveloperApiPortErrorV1::Conflict);
            }
            if row.state == "complete" {
                return replay_receipt(row);
            }
        }
        let session = self.session(command, video_id, upload_id).await?;
        if prior.is_none() && session.state != "open" {
            return Err(LegacyDeveloperApiPortErrorV1::Conflict);
        }
        let now_ms = now_ms()?;
        let operation_id = if let Some(row) = prior.as_ref() {
            row.operation_id.clone()
        } else {
            mutation_operation_id(command)?
        };
        let clamped = duration_seconds.min(LEGACY_DEVELOPER_MAX_DURATION_SECONDS);
        let billable = completion_billable_seconds(duration_seconds, &parts);
        let debit = completion_debit_microcredits(billable);
        if prior.is_none() {
            let app_id = command
                .app_id()
                .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
            let account = self
                .rows::<AccountRow>(ACCOUNT_READ_SQL, vec![JsValue::from_str(app_id)])
                .await?
                .into_iter()
                .next()
                .ok_or(LegacyDeveloperApiPortErrorV1::CreditAccountMissing)?;
            if account.balance_microcredits < debit {
                return Err(LegacyDeveloperApiPortErrorV1::InsufficientCredits);
            }
            let mut statements = vec![
                self.claim_statement(command, Some(video_id), now_ms)?,
                self.statement(
                    OPERATION_EFFECT_PENDING_SQL,
                    vec![JsValue::from_str(&operation_id)],
                )?,
                self.statement(
                    MULTIPART_STATE_SQL,
                    vec![
                        JsValue::from_str(upload_id),
                        JsValue::from_str("completing"),
                        JsValue::from_f64(now_ms as f64),
                        JsValue::from_str("open"),
                        JsValue::from_str(&operation_id),
                    ],
                )?,
                self.statement(
                    OUTBOX_INSERT_SQL,
                    vec![
                        JsValue::from_str(&operation_id),
                        JsValue::from_str("multipart_complete"),
                        JsValue::from_str(command.request_digest()),
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?,
            ];
            if debit > 0 {
                statements.push(self.statement(
                    CREDIT_DEBIT_SQL,
                    vec![
                        JsValue::from_str(&Uuid::now_v7().to_string()),
                        JsValue::from_str(&account.id),
                        JsValue::from_str("video_create"),
                        JsValue::from_f64(-(debit as f64)),
                        JsValue::from_f64((account.balance_microcredits - debit) as f64),
                        JsValue::from_str(video_id),
                        JsValue::from_str("developer_video"),
                        JsValue::from_str(&json!({"durationSeconds": billable}).to_string()),
                        JsValue::from_str(&operation_id),
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?);
            }
            self.batch(statements).await?;
        } else if session.state != "completing" {
            return Err(LegacyDeveloperApiPortErrorV1::Conflict);
        }
        let _ = self
            .statement(OUTBOX_ATTEMPT_SQL, vec![JsValue::from_str(&operation_id)])?
            .run()
            .into_send()
            .await;
        parts.sort_by_key(|part| part.part_number);
        if self
            .bucket
            .head(&session.object_key)
            .into_send()
            .await
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Provider)?
            .is_none()
        {
            let upload = self
                .bucket
                .resume_multipart_upload(&session.object_key, upload_id)
                .map_err(|_| LegacyDeveloperApiPortErrorV1::Provider)?;
            let completion = upload
                .complete(
                    parts
                        .iter()
                        .map(|part| UploadedPart::new(part.part_number, part.etag.clone()))
                        .collect::<Vec<_>>(),
                )
                .into_send()
                .await;
            if completion.is_err()
                && self
                    .bucket
                    .head(&session.object_key)
                    .into_send()
                    .await
                    .map_err(|_| LegacyDeveloperApiPortErrorV1::Provider)?
                    .is_none()
            {
                return Err(LegacyDeveloperApiPortErrorV1::Provider);
            }
        }
        let result_json = "{\"success\":true}";
        let mut statements = vec![
            self.statement(
                MULTIPART_STATE_SQL,
                vec![
                    JsValue::from_str(upload_id),
                    JsValue::from_str("complete"),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str("completing"),
                    JsValue::from_str(&operation_id),
                ],
            )?,
            self.statement(
                VIDEO_COMPLETE_SQL,
                vec![
                    JsValue::from_str(&session.native_video_id),
                    JsValue::from_f64(clamped),
                    optional_number(width),
                    optional_number(height),
                    optional_number(fps),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str(&operation_id),
                ],
            )?,
            self.statement(
                OUTBOX_COMPLETE_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
        ];
        statements.extend(self.durable_result_statements(
            command,
            &operation_id,
            "success",
            result_json,
            video_id,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(success(false))
    }

    async fn abort(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        video_id: &str,
        upload_id: &str,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        let prior = self.prior_operation(command).await?;
        if let Some(row) = prior.as_ref() {
            if row.request_digest != command.request_digest() {
                return Err(LegacyDeveloperApiPortErrorV1::Conflict);
            }
            if row.state == "complete" {
                return replay_receipt(row);
            }
        }
        let session = self.session(command, video_id, upload_id).await?;
        if prior.is_none() && session.state != "open" {
            return Err(LegacyDeveloperApiPortErrorV1::Conflict);
        }
        let now_ms = now_ms()?;
        let operation_id = if let Some(row) = prior.as_ref() {
            row.operation_id.clone()
        } else {
            mutation_operation_id(command)?
        };
        if prior.is_none() {
            self.batch(vec![
                self.claim_statement(command, Some(video_id), now_ms)?,
                self.statement(
                    OPERATION_EFFECT_PENDING_SQL,
                    vec![JsValue::from_str(&operation_id)],
                )?,
                self.statement(
                    MULTIPART_STATE_SQL,
                    vec![
                        JsValue::from_str(upload_id),
                        JsValue::from_str("aborting"),
                        JsValue::from_f64(now_ms as f64),
                        JsValue::from_str("open"),
                        JsValue::from_str(&operation_id),
                    ],
                )?,
                self.statement(
                    OUTBOX_INSERT_SQL,
                    vec![
                        JsValue::from_str(&operation_id),
                        JsValue::from_str("multipart_abort"),
                        JsValue::from_str(command.request_digest()),
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?,
            ])
            .await?;
        } else if session.state != "aborting" {
            return Err(LegacyDeveloperApiPortErrorV1::Conflict);
        }
        self.bucket
            .resume_multipart_upload(&session.object_key, upload_id)
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Provider)?
            .abort()
            .into_send()
            .await
            .map_err(|_| LegacyDeveloperApiPortErrorV1::Provider)?;
        let result_json = "{\"success\":true}";
        let mut statements = vec![
            self.statement(
                MULTIPART_STATE_SQL,
                vec![
                    JsValue::from_str(upload_id),
                    JsValue::from_str("aborted"),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str("aborting"),
                    JsValue::from_str(&operation_id),
                ],
            )?,
            self.statement(
                OUTBOX_COMPLETE_SQL,
                vec![
                    JsValue::from_str(&operation_id),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?,
        ];
        statements.extend(self.durable_result_statements(
            command,
            &operation_id,
            "success",
            result_json,
            video_id,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(success(false))
    }

    async fn delete_video(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        video_id: &str,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        if let Some(prior) = self.prior_operation(command).await? {
            return replay_or_pending(prior, command.request_digest());
        }
        let app_id = command
            .app_id()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
        self.video_row(app_id, video_id).await?;
        let now_ms = now_ms()?;
        let operation_id = mutation_operation_id(command)?;
        let mut statements = vec![
            self.claim_statement(command, Some(video_id), now_ms)?,
            self.statement(
                VIDEO_DELETE_SQL,
                vec![
                    JsValue::from_str(app_id),
                    JsValue::from_str(video_id),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str(&operation_id),
                ],
            )?,
        ];
        statements.extend(self.durable_result_statements(
            command,
            &operation_id,
            "success",
            "{\"success\":true}",
            video_id,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(success(false))
    }

    async fn video_row(&self, app_id: &str, video_id: &str) -> PortResult<VideoRow> {
        self.rows::<VideoRow>(
            VIDEO_READ_SQL,
            vec![JsValue::from_str(app_id), JsValue::from_str(video_id)],
        )
        .await?
        .into_iter()
        .next()
        .ok_or(LegacyDeveloperApiPortErrorV1::NotFound)
    }

    async fn session(
        &self,
        command: &LegacyDeveloperApiCommandV1,
        video_id: &str,
        upload_id: &str,
    ) -> PortResult<MultipartRow> {
        self.rows::<MultipartRow>(
            MULTIPART_SESSION_READ_SQL,
            vec![
                JsValue::from_str(
                    command
                        .app_id()
                        .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
                ),
                JsValue::from_str(upload_id),
                JsValue::from_str(video_id),
            ],
        )
        .await?
        .into_iter()
        .next()
        .ok_or(LegacyDeveloperApiPortErrorV1::NotFound)
    }

    async fn storage_cron(&self, day: &str) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        if let Some(run) = self
            .rows::<CronRunRow>(CRON_RUN_READ_SQL, vec![JsValue::from_str(day)])
            .await?
            .into_iter()
            .next()
        {
            return Ok(LegacyDeveloperApiOutcomeV1 {
                result: LegacyDeveloperApiResultV1::Cron {
                    date: run.snapshot_date,
                    apps_processed: u32::try_from(run.apps_processed)
                        .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?,
                },
                replayed: true,
            });
        }
        let candidates = self
            .rows::<CronCandidateRow>(CRON_CANDIDATES_SQL, vec![JsValue::from_str(day)])
            .await?;
        let now_ms = now_ms()?;
        let mut processed = 0_u32;
        for candidate in candidates {
            if candidate.total_duration_minutes <= 0.0 {
                continue;
            }
            let charge = daily_storage_charge(candidate.total_duration_minutes);
            let (Some(account_id), Some(balance)) = (
                candidate.account_id.as_deref(),
                candidate.balance_microcredits,
            ) else {
                continue;
            };
            if charge <= 0 || balance < charge {
                continue;
            }
            let statements = vec![
                self.statement(
                    CREDIT_DEBIT_SQL,
                    vec![
                        JsValue::from_str(&Uuid::now_v7().to_string()),
                        JsValue::from_str(account_id),
                        JsValue::from_str("storage_daily"),
                        JsValue::from_f64(-(charge as f64)),
                        JsValue::from_f64((balance - charge) as f64),
                        JsValue::from_str(day),
                        JsValue::from_str("manual"),
                        JsValue::from_str(
                            &json!({
                                "date": day,
                                "totalDurationMinutes": candidate.total_duration_minutes,
                                "videoCount": candidate.video_count
                            })
                            .to_string(),
                        ),
                        JsValue::NULL,
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?,
                self.statement(
                    CRON_SNAPSHOT_INSERT_SQL,
                    vec![
                        JsValue::from_str(&Uuid::now_v7().to_string()),
                        JsValue::from_str(&candidate.app_id),
                        JsValue::from_str(day),
                        JsValue::from_f64(candidate.total_duration_minutes),
                        JsValue::from_f64(candidate.video_count as f64),
                        JsValue::from_f64(charge as f64),
                        JsValue::from_f64(now_ms as f64),
                    ],
                )?,
            ];
            if self.batch(statements).await.is_ok() {
                processed = processed.saturating_add(1);
            }
        }
        let run_write = self
            .batch(vec![self.statement(
                CRON_RUN_INSERT_SQL,
                vec![
                    JsValue::from_str(day),
                    JsValue::from_f64(f64::from(processed)),
                    JsValue::from_f64(now_ms as f64),
                ],
            )?])
            .await;
        if run_write.is_err()
            && let Some(run) = self
                .rows::<CronRunRow>(CRON_RUN_READ_SQL, vec![JsValue::from_str(day)])
                .await?
                .into_iter()
                .next()
        {
            return Ok(LegacyDeveloperApiOutcomeV1 {
                result: LegacyDeveloperApiResultV1::Cron {
                    date: run.snapshot_date,
                    apps_processed: u32::try_from(run.apps_processed)
                        .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?,
                },
                replayed: true,
            });
        }
        run_write?;
        Ok(LegacyDeveloperApiOutcomeV1 {
            result: LegacyDeveloperApiResultV1::Cron {
                date: day.to_owned(),
                apps_processed: processed,
            },
            replayed: false,
        })
    }
}

#[async_trait(?Send)]
impl LegacyDeveloperApiPortV1 for D1LegacyDeveloperApiPortV1<'_> {
    async fn execute(
        &self,
        command: LegacyDeveloperApiCommandV1,
    ) -> PortResult<LegacyDeveloperApiOutcomeV1> {
        match command.input().clone() {
            LegacyDeveloperApiInputV1::StorageCron { snapshot_day } => {
                self.storage_cron(&snapshot_day).await
            }
            LegacyDeveloperApiInputV1::VideoCreate {
                name,
                external_user_id,
                metadata,
            } => {
                self.create_video(&command, name, external_user_id, metadata)
                    .await
            }
            LegacyDeveloperApiInputV1::MultipartInitiate {
                video_id,
                content_type,
            } => {
                self.initiate(&command, &video_id, content_type.as_deref())
                    .await
            }
            LegacyDeveloperApiInputV1::MultipartPresign {
                video_id,
                upload_id,
                part_number,
            } => {
                self.presign(&command, &video_id, &upload_id, part_number)
                    .await
            }
            LegacyDeveloperApiInputV1::MultipartComplete {
                video_id,
                upload_id,
                parts,
                duration_seconds,
                width,
                height,
                fps,
            } => {
                self.complete(
                    &command,
                    MultipartCompletionArgsV1 {
                        video_id: &video_id,
                        upload_id: &upload_id,
                        parts,
                        duration_seconds,
                        width,
                        height,
                        fps,
                    },
                )
                .await
            }
            LegacyDeveloperApiInputV1::MultipartAbort {
                video_id,
                upload_id,
            } => self.abort(&command, &video_id, &upload_id).await,
            LegacyDeveloperApiInputV1::VideoDelete { video_id } => {
                self.delete_video(&command, &video_id).await
            }
            LegacyDeveloperApiInputV1::Usage => {
                let app_id = command
                    .app_id()
                    .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
                let usage = self
                    .rows::<UsageRow>(USAGE_READ_SQL, vec![JsValue::from_str(app_id)])
                    .await?
                    .into_iter()
                    .next()
                    .unwrap_or(UsageRow {
                        balance_microcredits: 0,
                        total_videos: 0,
                        total_duration_minutes: 0.0,
                    });
                Ok(LegacyDeveloperApiOutcomeV1 {
                    result: LegacyDeveloperApiResultV1::Usage(LegacyDeveloperUsageV1 {
                        balance_micro_credits: usage.balance_microcredits,
                        balance_dollars: format!(
                            "{:.2}",
                            usage.balance_microcredits as f64 / 100_000.0
                        ),
                        total_videos: usage.total_videos,
                        total_duration_minutes: usage.total_duration_minutes,
                    }),
                    replayed: false,
                })
            }
            LegacyDeveloperApiInputV1::VideosList {
                external_user_id,
                limit,
                offset,
            } => {
                let rows = self
                    .rows::<VideoRow>(
                        VIDEOS_LIST_SQL,
                        vec![
                            JsValue::from_str(
                                command
                                    .app_id()
                                    .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
                            ),
                            optional_text(external_user_id.as_deref()),
                            JsValue::from_f64(f64::from(limit)),
                            JsValue::from_f64(offset as f64),
                        ],
                    )
                    .await?;
                Ok(LegacyDeveloperApiOutcomeV1 {
                    result: LegacyDeveloperApiResultV1::Videos(
                        rows.into_iter()
                            .map(project_video)
                            .collect::<PortResult<Vec<_>>>()?,
                    ),
                    replayed: false,
                })
            }
            LegacyDeveloperApiInputV1::VideoGet { video_id } => {
                let row = self
                    .video_row(
                        command
                            .app_id()
                            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
                        &video_id,
                    )
                    .await?;
                Ok(LegacyDeveloperApiOutcomeV1 {
                    result: LegacyDeveloperApiResultV1::Video(Box::new(project_video(row)?)),
                    replayed: false,
                })
            }
            LegacyDeveloperApiInputV1::VideoStatus { video_id } => {
                let row = self
                    .video_row(
                        command
                            .app_id()
                            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
                        &video_id,
                    )
                    .await?;
                Ok(LegacyDeveloperApiOutcomeV1 {
                    result: LegacyDeveloperApiResultV1::VideoStatus(LegacyDeveloperVideoStatusV1 {
                        id: row.legacy_video_id,
                        duration: row.duration,
                        width: row.width,
                        height: row.height,
                        transcription_status: row.transcription_status,
                        ready: row.duration.is_some() && row.width.is_some(),
                    }),
                    replayed: false,
                })
            }
        }
    }
}

fn completion_billable_seconds(duration_seconds: f64, parts: &[LegacyDeveloperPartV1]) -> f64 {
    let total_bytes = parts.iter().map(|part| part.size).sum::<f64>();
    duration_seconds
        .min(LEGACY_DEVELOPER_MAX_DURATION_SECONDS)
        .max(total_bytes / 2_500_000.0)
        .min(LEGACY_DEVELOPER_MAX_DURATION_SECONDS)
}

fn completion_debit_microcredits(billable_seconds: f64) -> i64 {
    (billable_seconds / 60.0 * LEGACY_DEVELOPER_MICROCREDITS_PER_MINUTE).floor() as i64
}

fn daily_storage_charge(total_duration_minutes: f64) -> i64 {
    (total_duration_minutes * LEGACY_DEVELOPER_STORAGE_RATE_NUMERATOR as f64
        / LEGACY_DEVELOPER_STORAGE_RATE_DENOMINATOR as f64)
        .floor() as i64
}

fn project_video(row: VideoRow) -> PortResult<LegacyDeveloperVideoV1> {
    let metadata = row
        .metadata_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?;
    Ok(LegacyDeveloperVideoV1 {
        id: row.legacy_video_id,
        app_id: row.legacy_app_id,
        external_user_id: row.external_user_id,
        name: row.name,
        duration: row.duration,
        width: row.width,
        height: row.height,
        fps: row.fps,
        s3_key: row.s3_key,
        transcription_status: row.transcription_status,
        metadata,
        deleted_at: row.deleted_at_ms.map(iso_from_ms),
        created_at: iso_from_ms(row.created_at_ms),
        updated_at: iso_from_ms(row.updated_at_ms),
    })
}

fn replay_or_pending(
    row: OperationRow,
    request_digest: &str,
) -> PortResult<LegacyDeveloperApiOutcomeV1> {
    if row.request_digest != request_digest {
        return Err(LegacyDeveloperApiPortErrorV1::Conflict);
    }
    if row.state != "complete" {
        return Err(LegacyDeveloperApiPortErrorV1::Unavailable);
    }
    replay_receipt(&row)
}

fn replay_receipt(row: &OperationRow) -> PortResult<LegacyDeveloperApiOutcomeV1> {
    if row.status != Some(200) {
        return Err(LegacyDeveloperApiPortErrorV1::Corrupt);
    }
    let kind = row
        .result_kind
        .as_deref()
        .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?;
    let value: Value = serde_json::from_str(
        row.result_json
            .as_deref()
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)?,
    )
    .map_err(|_| LegacyDeveloperApiPortErrorV1::Corrupt)?;
    let text = |name: &str| {
        value[name]
            .as_str()
            .map(str::to_owned)
            .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)
    };
    let result = match kind {
        "success" => LegacyDeveloperApiResultV1::Success,
        "upload_initiated" => LegacyDeveloperApiResultV1::UploadInitiated {
            upload_id: text("uploadId")?,
        },
        "part_presigned" => LegacyDeveloperApiResultV1::PartPresigned {
            presigned_url: text("presignedUrl")?,
        },
        "video_created" => LegacyDeveloperApiResultV1::VideoCreated {
            video_id: text("videoId")?,
            s3_key: text("s3Key")?,
            share_url: text("shareUrl")?,
            embed_url: text("embedUrl")?,
        },
        _ => return Err(LegacyDeveloperApiPortErrorV1::Corrupt),
    };
    Ok(LegacyDeveloperApiOutcomeV1 {
        result,
        replayed: true,
    })
}

fn success(replayed: bool) -> LegacyDeveloperApiOutcomeV1 {
    LegacyDeveloperApiOutcomeV1 {
        result: LegacyDeveloperApiResultV1::Success,
        replayed,
    }
}

fn normalized_content_type(value: Option<&str>) -> &'static str {
    match value {
        Some("video/mp4") => "video/mp4",
        Some("video/webm") => "video/webm",
        Some("video/quicktime") => "video/quicktime",
        Some("video/x-matroska") => "video/x-matroska",
        Some("video/avi") => "video/avi",
        Some("application/octet-stream") => "application/octet-stream",
        _ => "video/mp4",
    }
}

fn random_cap_id(length: usize) -> PortResult<String> {
    let mut output = String::with_capacity(length);
    while output.len() < length {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| LegacyDeveloperApiPortErrorV1::Unavailable)?;
        for byte in bytes {
            if output.len() == length {
                break;
            }
            output.push(char::from(CAP_ALPHABET[usize::from(byte & 0x1f)]));
        }
    }
    Ok(output)
}

fn optional_text(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn optional_number(value: Option<f64>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_f64)
}

fn mutation_operation_id(command: &LegacyDeveloperApiCommandV1) -> PortResult<String> {
    command
        .operation_id()
        .map(|value| value.to_string())
        .ok_or(LegacyDeveloperApiPortErrorV1::Corrupt)
}

fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn now_ms() -> PortResult<i64> {
    let value = js_sys::Date::now();
    if !value.is_finite() || !(0.0..=9_007_199_254_740_991.0).contains(&value) {
        return Err(LegacyDeveloperApiPortErrorV1::Corrupt);
    }
    Ok(value as i64)
}

fn iso_from_ms(value: i64) -> String {
    String::from(js_sys::Date::new(&JsValue::from_f64(value as f64)).to_iso_string())
}

fn map_d1(message: &str) -> LegacyDeveloperApiPortErrorV1 {
    if message.contains(INSUFFICIENT) {
        LegacyDeveloperApiPortErrorV1::InsufficientCredits
    } else if message.contains(OPERATION_UNIQUE) {
        LegacyDeveloperApiPortErrorV1::Conflict
    } else {
        LegacyDeveloperApiPortErrorV1::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(size: f64) -> LegacyDeveloperPartV1 {
        LegacyDeveloperPartV1 {
            part_number: 1,
            etag: "fixture".into(),
            size,
        }
    }

    #[test]
    fn completion_billing_uses_duration_or_size_floor_and_caps_at_four_hours() {
        assert_eq!(
            completion_billable_seconds(1.0, &[part(150_000_000.0)]),
            60.0
        );
        assert_eq!(completion_debit_microcredits(60.0), 5_000);
        assert_eq!(completion_billable_seconds(70.0, &[part(0.0)]), 70.0);
        assert_eq!(
            completion_billable_seconds(1.0, &[part(40_000_000_000.0)]),
            LEGACY_DEVELOPER_MAX_DURATION_SECONDS
        );
        assert_eq!(
            completion_debit_microcredits(LEGACY_DEVELOPER_MAX_DURATION_SECONDS),
            1_200_000
        );
    }

    #[test]
    fn daily_storage_billing_floors_the_exact_333_over_100_rate() {
        assert_eq!(daily_storage_charge(0.0), 0);
        assert_eq!(daily_storage_charge(10.0), 33);
        assert_eq!(daily_storage_charge(12.0), 39);
    }
}
