//! Atomic D1 authority for Cap's ten retained video-property mutations.
//!
//! The source APIs have deliberately different normalization and response
//! rules. The application crate preserves those rules; this module binds them
//! to owner/verification snapshots, PBKDF2-SHA256 password material, and
//! immutable replay evidence in one D1 batch.

use async_trait::async_trait;
use frame_application::{
    LegacyMobileCapSummaryV1, LegacyMobileUploadProgressV1, LegacyVideoPropertiesAtomicErrorV1,
    LegacyVideoPropertiesAtomicOutcomeV1, LegacyVideoPropertiesAtomicPortV1,
    LegacyVideoPropertiesAtomicResultV1, LegacyVideoPropertiesCommandV1,
    LegacyVideoPropertiesSurfaceV1, LegacyVideoPropertyMutationV1, javascript_object_spread,
};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const CLOCK_NOW_SQL: &str = include_str!("../queries/legacy_video_properties/clock_now.sql");
const VIDEO_SNAPSHOT_SQL: &str =
    include_str!("../queries/legacy_video_properties/video_snapshot.sql");
const VERIFICATION_CANDIDATES_SQL: &str =
    include_str!("../queries/legacy_video_properties/verification_candidates.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_video_properties/operation_by_key.sql");
const OPERATION_CLAIM_SQL: &str =
    include_str!("../queries/legacy_video_properties/operation_claim.sql");
const OPERATION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_video_properties/operation_assert.sql");
const OWNER_ASSERT_SQL: &str = include_str!("../queries/legacy_video_properties/owner_assert.sql");
const VERIFICATION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_video_properties/verification_assert.sql");
const MUTATION_APPLY_SQL: &str =
    include_str!("../queries/legacy_video_properties/mutation_apply.sql");
const MUTATION_ASSERT_SQL: &str =
    include_str!("../queries/legacy_video_properties/mutation_assert.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_properties/receipt_insert.sql");
const EFFECT_INSERT_SQL: &str =
    include_str!("../queries/legacy_video_properties/effect_insert.sql");
const AUDIT_INSERT_SQL: &str = include_str!("../queries/legacy_video_properties/audit_insert.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_video_properties/operation_complete.sql");
const DURABLE_ASSERT_SQL: &str =
    include_str!("../queries/legacy_video_properties/durable_assert.sql");
const ASSERTION_CLEANUP_SQL: &str =
    include_str!("../queries/legacy_video_properties/assertion_cleanup.sql");

const ASSERTION_SENTINEL: &str = "frame_legacy_video_property_assertion_v1";
const IMMUTABLE_SENTINEL: &str = "frame_legacy_video_property_evidence_immutable_v1";
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_BYTES: usize = 16;
const DERIVED_BYTES: usize = 32;

type AtomicResult<T> = Result<T, LegacyVideoPropertiesAtomicErrorV1>;

pub(crate) struct D1LegacyVideoPropertiesAtomicPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyVideoPropertiesAtomicPortV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Unavailable)
    }

    async fn rows<T>(&self, sql: &str, bindings: Vec<JsValue>) -> AtomicResult<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1_message(
                result.error().as_deref().unwrap_or_default(),
            ));
        }
        result
            .results::<T>()
            .map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Corrupt)
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AtomicResult<()> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1_message(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1_message(
                failed.error().as_deref().unwrap_or_default(),
            ));
        }
        Ok(())
    }
}

fn map_d1_message(message: &str) -> LegacyVideoPropertiesAtomicErrorV1 {
    if message.contains(ASSERTION_SENTINEL) {
        LegacyVideoPropertiesAtomicErrorV1::StaleAuthority
    } else if message.contains(IMMUTABLE_SENTINEL) {
        LegacyVideoPropertiesAtomicErrorV1::Corrupt
    } else {
        LegacyVideoPropertiesAtomicErrorV1::Unavailable
    }
}

#[derive(Debug, Deserialize)]
struct ClockRow {
    now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct VideoSnapshot {
    id: String,
    owner_id: String,
    title: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    duration_ms: Option<i64>,
    folder_id: Option<String>,
    legacy_public: i64,
    password_hash: Option<String>,
    metadata_json: Option<String>,
    settings_json: Option<String>,
    revision: i64,
    property_revision: i64,
    owner_name: String,
    comment_count: i64,
    reaction_count: i64,
    uploaded_bytes: Option<i64>,
    total_bytes: Option<i64>,
    upload_phase: Option<String>,
    joined_space_count: i64,
    joined_space_snapshot: String,
}

impl VideoSnapshot {
    fn validate(&self) -> AtomicResult<()> {
        let valid_json = |value: &Option<String>| {
            value
                .as_deref()
                .is_none_or(|value| serde_json::from_str::<Value>(value).is_ok())
        };
        if Uuid::parse_str(&self.id).is_err()
            || Uuid::parse_str(&self.owner_id).is_err()
            || !(0..=MAX_SAFE_INTEGER).contains(&self.created_at_ms)
            || !(0..=MAX_SAFE_INTEGER).contains(&self.updated_at_ms)
            || self.duration_ms.is_some_and(|value| value < 0)
            || !matches!(self.legacy_public, 0 | 1)
            || self
                .password_hash
                .as_deref()
                .is_some_and(|hash| !valid_password_hash(hash))
            || !valid_json(&self.metadata_json)
            || !valid_json(&self.settings_json)
            || self.revision < 0
            || self.property_revision < 0
            || self.comment_count < 0
            || self.reaction_count < 0
            || self.joined_space_count < 0
            || self.uploaded_bytes.is_some_and(|value| value < 0)
            || self.total_bytes.is_some_and(|value| value < 0)
        {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct CandidateRow {
    password_hash: String,
    ordinal: i64,
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    request_digest: String,
    state: String,
    result_kind: Option<String>,
    result_json: Option<String>,
    result_digest: Option<String>,
    effect_kind: Option<String>,
    effect_json: Option<String>,
    effect_count: i64,
    audit_count: i64,
}

#[derive(Clone)]
struct MutationPlan {
    title: String,
    metadata_json: Option<String>,
    public: i64,
    password_hash: Option<String>,
    settings_json: Option<String>,
    title_changed: bool,
    metadata_changed: bool,
    public_changed: bool,
    password_changed: bool,
    settings_changed: bool,
    result: LegacyVideoPropertiesAtomicResultV1,
    result_kind: &'static str,
    result_json: String,
    effect: Option<(&'static str, String)>,
}

impl D1LegacyVideoPropertiesAtomicPortV1<'_> {
    async fn clock_now(&self) -> AtomicResult<i64> {
        let mut rows = self.rows::<ClockRow>(CLOCK_NOW_SQL, Vec::new()).await?;
        if rows.len() != 1 {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        let now_ms = rows.pop().expect("one clock row").now_ms;
        if !(0..=MAX_SAFE_INTEGER).contains(&now_ms) {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        Ok(now_ms)
    }

    async fn video(&self, command: &LegacyVideoPropertiesCommandV1) -> AtomicResult<VideoSnapshot> {
        let mut rows = self
            .rows::<VideoSnapshot>(
                VIDEO_SNAPSHOT_SQL,
                vec![
                    js(command.legacy_video_id()),
                    js(&command.video_id().to_string()),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        let row = rows
            .pop()
            .ok_or(LegacyVideoPropertiesAtomicErrorV1::TargetMissing)?;
        row.validate()?;
        Ok(row)
    }

    async fn operation(
        &self,
        command: &LegacyVideoPropertiesCommandV1,
        principal_digest: &str,
        video_id: &str,
    ) -> AtomicResult<Option<OperationRow>> {
        let mut rows = self
            .rows::<OperationRow>(
                OPERATION_BY_KEY_SQL,
                vec![
                    js(command.surface().operation_id()),
                    js(principal_digest),
                    js(video_id),
                    js(&command.idempotency_key_digest_hex()),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        Ok(rows.pop())
    }

    async fn candidates(&self, video_id: &str) -> AtomicResult<Vec<CandidateRow>> {
        let rows = self
            .rows::<CandidateRow>(VERIFICATION_CANDIDATES_SQL, vec![js(video_id)])
            .await?;
        if rows.iter().enumerate().any(|(index, row)| {
            row.ordinal != i64::try_from(index).unwrap_or(i64::MAX)
                || !valid_password_hash(&row.password_hash)
        }) {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        Ok(rows)
    }
}

impl D1LegacyVideoPropertiesAtomicPortV1<'_> {
    fn owner_assertion(
        &self,
        operation_id: &str,
        video: &VideoSnapshot,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            OWNER_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&video.id),
                js(&video.owner_id),
                js(&video.title),
                js_opt(video.metadata_json.as_deref()),
                number(video.legacy_public),
                js_opt(video.password_hash.as_deref()),
                js_opt(video.settings_json.as_deref()),
                number(video.revision),
                number(video.property_revision),
                number(video.updated_at_ms),
            ],
        )
    }

    fn verification_assertion(
        &self,
        operation_id: &str,
        video: &VideoSnapshot,
    ) -> AtomicResult<D1PreparedStatement> {
        self.statement(
            VERIFICATION_ASSERT_SQL,
            vec![
                js(operation_id),
                js(&video.id),
                js_opt(video.password_hash.as_deref()),
                number(video.property_revision),
                number(video.joined_space_count),
                js(&video.joined_space_snapshot),
            ],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evidence(
        &self,
        command: &LegacyVideoPropertiesCommandV1,
        operation_id: &str,
        principal_digest: &str,
        video_id: &str,
        plan: &MutationPlan,
        result_digest: &str,
        now_ms: i64,
    ) -> AtomicResult<Vec<D1PreparedStatement>> {
        let video_digest = digest_fields(b"frame.legacy-video-property.video.v1\0", &[video_id]);
        let mut statements = vec![self.statement(
            RECEIPT_INSERT_SQL,
            vec![
                js(operation_id),
                js(plan.result_kind),
                js(&plan.result_json),
                js(result_digest),
                number(now_ms),
            ],
        )?];
        if let Some((kind, payload)) = &plan.effect {
            statements.push(self.statement(
                EFFECT_INSERT_SQL,
                vec![js(operation_id), js(kind), js(payload), number(now_ms)],
            )?);
        }
        statements.extend([
            self.statement(
                AUDIT_INSERT_SQL,
                vec![
                    js(&Uuid::now_v7().to_string()),
                    js(operation_id),
                    js(command.surface().operation_id()),
                    js(principal_digest),
                    js(&video_digest),
                    js(command.request_digest()),
                    js(result_digest),
                    number(now_ms),
                ],
            )?,
            self.statement(
                OPERATION_COMPLETE_SQL,
                vec![js(operation_id), number(now_ms)],
            )?,
            self.statement(
                DURABLE_ASSERT_SQL,
                vec![
                    js(operation_id),
                    number(now_ms),
                    js(result_digest),
                    number(i64::from(plan.effect.is_some())),
                ],
            )?,
            self.statement(ASSERTION_CLEANUP_SQL, vec![js(operation_id)])?,
        ]);
        Ok(statements)
    }
}

impl D1LegacyVideoPropertiesAtomicPortV1<'_> {
    async fn plan(
        &self,
        command: &LegacyVideoPropertiesCommandV1,
        video: &VideoSnapshot,
        now_ms: i64,
    ) -> AtomicResult<MutationPlan> {
        let mut plan = MutationPlan {
            title: video.title.clone(),
            metadata_json: video.metadata_json.clone(),
            public: video.legacy_public,
            password_hash: video.password_hash.clone(),
            settings_json: video.settings_json.clone(),
            title_changed: false,
            metadata_changed: false,
            public_changed: false,
            password_changed: false,
            settings_changed: false,
            result: LegacyVideoPropertiesAtomicResultV1::SuccessObject,
            result_kind: "success",
            result_json: "{\"success\":true}".into(),
            effect: None,
        };
        match command.mutation() {
            LegacyVideoPropertyMutationV1::MobilePassword { password } => {
                plan.password_hash = password.as_deref().map(hash_password).transpose()?;
                plan.password_changed = true;
                mobile_result(command, video, now_ms, &mut plan)?;
            }
            LegacyVideoPropertyMutationV1::MobileSharing { public } => {
                plan.public = i64::from(*public);
                plan.public_changed = true;
                mobile_result(command, video, now_ms, &mut plan)?;
            }
            LegacyVideoPropertyMutationV1::MobileTitle { title } => {
                plan.title.clone_from(title);
                plan.title_changed = true;
                plan.effect = Some(("revalidation", revalidation_paths(command, true)?));
                mobile_result(command, video, now_ms, &mut plan)?;
            }
            LegacyVideoPropertyMutationV1::MetadataReplace { metadata } => {
                plan.metadata_json = Some(json_string(metadata)?);
                plan.metadata_changed = true;
                plan.result = LegacyVideoPropertiesAtomicResultV1::JsonTrue;
                plan.result_kind = "json_true";
                plan.result_json = "true".into();
                plan.effect = None;
            }
            LegacyVideoPropertyMutationV1::MetadataCustomDate { custom_created_at } => {
                let mut metadata = spread_metadata(video.metadata_json.as_deref())?;
                metadata.insert(
                    "customCreatedAt".into(),
                    Value::String(custom_created_at.clone()),
                );
                plan.metadata_json = Some(json_string(&Value::Object(metadata))?);
                plan.metadata_changed = true;
                plan.effect = Some(("revalidation", revalidation_paths(command, false)?));
            }
            LegacyVideoPropertyMutationV1::BrowserTitle { title } => {
                let mut metadata = spread_metadata(video.metadata_json.as_deref())?;
                metadata.insert("titleManuallyEdited".into(), Value::Bool(true));
                plan.metadata_json = Some(json_string(&Value::Object(metadata))?);
                plan.metadata_changed = true;
                plan.title.clone_from(title);
                plan.title_changed = true;
                plan.effect = Some(("revalidation", revalidation_paths(command, true)?));
            }
            LegacyVideoPropertyMutationV1::RemovePassword => {
                plan.password_hash = None;
                plan.password_changed = true;
                plan.result = LegacyVideoPropertiesAtomicResultV1::PasswordRemoved;
                plan.result_kind = "password_removed";
                plan.effect = Some(("revalidation", revalidation_paths(command, true)?));
            }
            LegacyVideoPropertyMutationV1::SetPassword { password } => {
                if password.is_empty() {
                    return Err(LegacyVideoPropertiesAtomicErrorV1::Crypto);
                }
                plan.password_hash = Some(hash_password(password)?);
                plan.password_changed = true;
                plan.result = LegacyVideoPropertiesAtomicResultV1::PasswordSet;
                plan.result_kind = "password_set";
                plan.effect = Some(("revalidation", revalidation_paths(command, true)?));
            }
            LegacyVideoPropertyMutationV1::VerifyPassword { password } => {
                let matched = self
                    .candidates(&video.id)
                    .await?
                    .into_iter()
                    .find(|candidate| verify_password(password, &candidate.password_hash))
                    .map(|candidate| candidate.password_hash);
                if let Some(hash) = matched {
                    plan.result = LegacyVideoPropertiesAtomicResultV1::PasswordVerified {
                        matched_hash: hash.clone(),
                    };
                    plan.result_kind = "password_verified";
                    plan.result_json = json_string(&json!({"matchedHash": hash}))?;
                    plan.effect = Some((
                        "password_cookie",
                        json_string(&json!({"matchedHash": hash}))?,
                    ));
                } else {
                    plan.result = LegacyVideoPropertiesAtomicResultV1::PasswordRejected;
                    plan.result_kind = "password_rejected";
                    plan.result_json = "{\"success\":false}".into();
                    plan.effect = None;
                }
            }
            LegacyVideoPropertyMutationV1::SettingsReplace { settings } => {
                plan.settings_json = Some(json_string(settings)?);
                plan.settings_changed = true;
            }
        }
        Ok(plan)
    }

    async fn execute_fresh(
        &self,
        command: &LegacyVideoPropertiesCommandV1,
        video: &VideoSnapshot,
        principal_digest: &str,
    ) -> AtomicResult<LegacyVideoPropertiesAtomicResultV1> {
        let operation_id = command.operation_id().to_string();
        let now_ms = self.clock_now().await?;
        let plan = self.plan(command, video, now_ms).await?;
        let legacy_video_digest = digest_fields(
            b"frame.legacy-video-property.legacy-id.v1\0",
            &[command.legacy_video_id()],
        );
        let mut statements = vec![
            self.statement(
                OPERATION_CLAIM_SQL,
                vec![
                    js(&operation_id),
                    js(command.surface().operation_id()),
                    js(command.surface().stable_code()),
                    js(principal_digest),
                    js(&video.id),
                    js(&legacy_video_digest),
                    js(&command.idempotency_key_digest_hex()),
                    js(command.request_digest()),
                    number(now_ms),
                ],
            )?,
            self.statement(
                OPERATION_ASSERT_SQL,
                vec![
                    js(&operation_id),
                    js(command.surface().operation_id()),
                    js(command.surface().stable_code()),
                    js(principal_digest),
                    js(&video.id),
                    js(&legacy_video_digest),
                    js(&command.idempotency_key_digest_hex()),
                    js(command.request_digest()),
                ],
            )?,
        ];
        if command.surface() == LegacyVideoPropertiesSurfaceV1::VerifyPassword {
            statements.push(self.verification_assertion(&operation_id, video)?);
        } else {
            statements.push(self.owner_assertion(&operation_id, video)?);
            statements.extend([
                self.statement(
                    MUTATION_APPLY_SQL,
                    vec![
                        js(&video.id),
                        number(i64::from(plan.title_changed)),
                        js(&plan.title),
                        number(i64::from(plan.metadata_changed)),
                        js_opt(plan.metadata_json.as_deref()),
                        number(i64::from(plan.public_changed)),
                        number(plan.public),
                        number(i64::from(plan.password_changed)),
                        js_opt(plan.password_hash.as_deref()),
                        number(i64::from(plan.settings_changed)),
                        js_opt(plan.settings_json.as_deref()),
                        number(now_ms),
                        js(&operation_id),
                        number(video.revision),
                        number(video.property_revision),
                    ],
                )?,
                self.statement(
                    MUTATION_ASSERT_SQL,
                    vec![
                        js(&operation_id),
                        js(&video.id),
                        js(&plan.title),
                        js_opt(plan.metadata_json.as_deref()),
                        number(plan.public),
                        js_opt(plan.password_hash.as_deref()),
                        js_opt(plan.settings_json.as_deref()),
                        number(now_ms),
                        number(video.revision + 1),
                        number(video.property_revision + 1),
                    ],
                )?,
            ]);
        }
        let result_digest = digest_fields(
            b"frame.legacy-video-property.result.v1\0",
            &[plan.result_kind, &plan.result_json],
        );
        statements.extend(self.evidence(
            command,
            &operation_id,
            principal_digest,
            &video.id,
            &plan,
            &result_digest,
            now_ms,
        )?);
        self.batch(statements).await?;
        Ok(plan.result)
    }
}

impl D1LegacyVideoPropertiesAtomicPortV1<'_> {
    fn replay(
        &self,
        command: &LegacyVideoPropertiesCommandV1,
        operation: &OperationRow,
    ) -> AtomicResult<LegacyVideoPropertiesAtomicOutcomeV1> {
        if operation.request_digest != command.request_digest() {
            return Err(LegacyVideoPropertiesAtomicErrorV1::IdempotencyConflict);
        }
        if operation.state != "complete"
            || Uuid::parse_str(&operation.operation_id).is_err()
            || operation.audit_count != 1
        {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        let result_kind = operation
            .result_kind
            .as_deref()
            .ok_or(LegacyVideoPropertiesAtomicErrorV1::Corrupt)?;
        let result_json = operation
            .result_json
            .as_deref()
            .ok_or(LegacyVideoPropertiesAtomicErrorV1::Corrupt)?;
        let expected_digest = digest_fields(
            b"frame.legacy-video-property.result.v1\0",
            &[result_kind, result_json],
        );
        if operation.result_digest.as_deref() != Some(expected_digest.as_str())
            || operation.effect_count != i64::from(operation.effect_kind.is_some())
            || operation.effect_kind.is_some() != operation.effect_json.is_some()
        {
            return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
        }
        let result = decode_result(result_kind, result_json)?;
        Ok(LegacyVideoPropertiesAtomicOutcomeV1 {
            result,
            replayed: true,
        })
    }

    async fn reconcile(
        &self,
        command: &LegacyVideoPropertiesCommandV1,
        principal_digest: &str,
        video_id: &str,
        original: LegacyVideoPropertiesAtomicErrorV1,
    ) -> AtomicResult<LegacyVideoPropertiesAtomicOutcomeV1> {
        match self.operation(command, principal_digest, video_id).await {
            Ok(Some(operation)) => self.replay(command, &operation),
            Ok(None) => Err(original),
            Err(_) => Err(LegacyVideoPropertiesAtomicErrorV1::Unavailable),
        }
    }
}

#[async_trait(?Send)]
impl LegacyVideoPropertiesAtomicPortV1 for D1LegacyVideoPropertiesAtomicPortV1<'_> {
    async fn execute(
        &self,
        command: LegacyVideoPropertiesCommandV1,
    ) -> AtomicResult<LegacyVideoPropertiesAtomicOutcomeV1> {
        let video = self.video(&command).await?;
        if command.surface() != LegacyVideoPropertiesSurfaceV1::VerifyPassword
            && command.actor_id().map(|value| value.to_string()).as_deref()
                != Some(video.owner_id.as_str())
        {
            return Err(LegacyVideoPropertiesAtomicErrorV1::AccessDenied);
        }
        let actor = command
            .actor_id()
            .map_or_else(|| "anonymous".into(), |value| value.to_string());
        let principal_digest =
            digest_fields(b"frame.legacy-video-property.principal.v1\0", &[&actor]);
        if let Some(operation) = self
            .operation(&command, &principal_digest, &video.id)
            .await?
        {
            return self.replay(&command, &operation);
        }
        match self
            .execute_fresh(&command, &video, &principal_digest)
            .await
        {
            Ok(result) => Ok(LegacyVideoPropertiesAtomicOutcomeV1 {
                result,
                replayed: false,
            }),
            Err(error) => {
                self.reconcile(&command, &principal_digest, &video.id, error)
                    .await
            }
        }
    }
}

fn mobile_result(
    command: &LegacyVideoPropertiesCommandV1,
    video: &VideoSnapshot,
    now_ms: i64,
    plan: &mut MutationPlan,
) -> AtomicResult<()> {
    let summary = LegacyMobileCapSummaryV1 {
        id: command.legacy_video_id().into(),
        share_url: format!("https://cap.so/s/{}", command.legacy_video_id()),
        title: plan.title.clone(),
        created_at: iso_from_millis(video.created_at_ms)?,
        updated_at: iso_from_millis(now_ms)?,
        owner_name: video.owner_name.clone(),
        duration_seconds: video.duration_ms.map(|value| value as f64 / 1000.0),
        thumbnail_url: None,
        folder_id: video.folder_id.clone(),
        public: plan.public == 1,
        protected: plan.password_hash.is_some(),
        view_count: 0.0,
        comment_count: video.comment_count as f64,
        reaction_count: video.reaction_count as f64,
        upload: match (
            video.uploaded_bytes,
            video.total_bytes,
            video.upload_phase.as_deref(),
        ) {
            (Some(uploaded), Some(total), Some(phase)) => Some(LegacyMobileUploadProgressV1 {
                uploaded: uploaded as f64,
                total: total as f64,
                phase: phase.into(),
                processing_progress: 0.0,
                processing_message: None,
                processing_error: None,
            }),
            _ => None,
        },
    };
    plan.result_json = json_string(&summary)?;
    plan.result = LegacyVideoPropertiesAtomicResultV1::MobileSummary(Box::new(summary));
    plan.result_kind = "mobile_summary";
    Ok(())
}

fn spread_metadata(value: Option<&str>) -> AtomicResult<serde_json::Map<String, Value>> {
    let value = value.map_or(Ok(Value::Null), |value| {
        serde_json::from_str(value).map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Corrupt)
    })?;
    Ok(javascript_object_spread(value))
}

fn revalidation_paths(
    command: &LegacyVideoPropertiesCommandV1,
    include_share: bool,
) -> AtomicResult<String> {
    let mut paths = vec!["/dashboard/caps", "/dashboard/shared-caps"];
    let share = format!("/s/{}", command.legacy_video_id());
    if include_share {
        paths.push(&share);
    }
    json_string(&json!({"paths": paths}))
}

fn decode_result(kind: &str, payload: &str) -> AtomicResult<LegacyVideoPropertiesAtomicResultV1> {
    match kind {
        "mobile_summary" => serde_json::from_str::<LegacyMobileCapSummaryV1>(payload)
            .map(|value| LegacyVideoPropertiesAtomicResultV1::MobileSummary(Box::new(value)))
            .map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Corrupt),
        "json_true" if payload == "true" => Ok(LegacyVideoPropertiesAtomicResultV1::JsonTrue),
        "success" if payload == "{\"success\":true}" => {
            Ok(LegacyVideoPropertiesAtomicResultV1::SuccessObject)
        }
        "password_set" if payload == "{\"success\":true}" => {
            Ok(LegacyVideoPropertiesAtomicResultV1::PasswordSet)
        }
        "password_removed" if payload == "{\"success\":true}" => {
            Ok(LegacyVideoPropertiesAtomicResultV1::PasswordRemoved)
        }
        "password_verified" => {
            let value: Value = serde_json::from_str(payload)
                .map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Corrupt)?;
            let hash = value
                .get("matchedHash")
                .and_then(Value::as_str)
                .filter(|value| valid_password_hash(value))
                .ok_or(LegacyVideoPropertiesAtomicErrorV1::Corrupt)?;
            Ok(LegacyVideoPropertiesAtomicResultV1::PasswordVerified {
                matched_hash: hash.into(),
            })
        }
        "password_rejected" if payload == "{\"success\":false}" => {
            Ok(LegacyVideoPropertiesAtomicResultV1::PasswordRejected)
        }
        _ => Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt),
    }
}

fn hash_password(password: &str) -> AtomicResult<String> {
    let mut salt = [0_u8; SALT_BYTES];
    getrandom::fill(&mut salt).map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Crypto)?;
    let mut derived = [0_u8; DERIVED_BYTES];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut derived);
    let mut wire = [0_u8; SALT_BYTES + DERIVED_BYTES];
    wire[..SALT_BYTES].copy_from_slice(&salt);
    wire[SALT_BYTES..].copy_from_slice(&derived);
    Ok(base64_encode(&wire))
}

fn verify_password(password: &str, encoded: &str) -> bool {
    let Some(wire) = base64_decode(encoded) else {
        return false;
    };
    if wire.len() != SALT_BYTES + DERIVED_BYTES {
        return false;
    }
    let mut actual = [0_u8; DERIVED_BYTES];
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        &wire[..SALT_BYTES],
        PBKDF2_ITERATIONS,
        &mut actual,
    );
    constant_time_equal_bytes(&actual, &wire[SALT_BYTES..])
}

fn constant_time_equal_bytes(actual: &[u8], expected: &[u8]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let key = b"frame.video-properties.pbkdf2-compare.v1";
    let mut expected_mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    expected_mac.update(expected);
    let expected_tag = expected_mac.finalize().into_bytes();
    let mut actual_mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    actual_mac.update(actual);
    actual_mac.verify_slice(&expected_tag).is_ok()
}

fn valid_password_hash(value: &str) -> bool {
    value.len() == 64
        && base64_decode(value).is_some_and(|bytes| bytes.len() == SALT_BYTES + DERIVED_BYTES)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(TABLE[usize::from(first >> 2)]));
        output.push(char::from(
            TABLE[usize::from((first & 0x03) << 4 | second >> 4)],
        ));
        output.push(if chunk.len() > 1 {
            char::from(TABLE[usize::from((second & 0x0f) << 2 | third >> 6)])
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            char::from(TABLE[usize::from(third & 0x3f)])
        } else {
            '='
        });
    }
    output
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(4) {
        return None;
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for (chunk_index, chunk) in value.as_bytes().chunks_exact(4).enumerate() {
        let last = chunk_index + 1 == value.len() / 4;
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' {
                return None;
            }
            0
        } else {
            base64_value(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            if !last {
                return None;
            }
            0
        } else {
            base64_value(chunk[3])?
        };
        output.push(a << 2 | b >> 4);
        if chunk[2] != b'=' {
            output.push((b & 0x0f) << 4 | c >> 2);
        }
        if chunk[3] != b'=' {
            output.push((c & 0x03) << 6 | d);
        }
    }
    Some(output)
}

const fn base64_value(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn iso_from_millis(value: i64) -> AtomicResult<String> {
    const DAY_MS: i64 = 86_400_000;
    if !(0..=253_402_300_799_999).contains(&value) {
        return Err(LegacyVideoPropertiesAtomicErrorV1::Corrupt);
    }
    let days = value / DAY_MS;
    let day_ms = value % DAY_MS;
    let (year, month, day) = civil_from_days(days);
    let hour = day_ms / 3_600_000;
    let minute = day_ms % 3_600_000 / 60_000;
    let second = day_ms % 60_000 / 1_000;
    let millis = day_ms % 1_000;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn json_string<T: serde::Serialize>(value: &T) -> AtomicResult<String> {
    serde_json::to_string(value).map_err(|_| LegacyVideoPropertiesAtomicErrorV1::Corrupt)
}

fn digest_fields(domain: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    lower_hex(&digest.finalize())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
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
    fn pbkdf2_wire_is_source_exact_and_verifies_without_trimming() {
        let encoded = hash_password("  secret  ").expect("hash");
        assert_eq!(encoded.len(), 64);
        assert!(verify_password("  secret  ", &encoded));
        assert!(!verify_password("secret", &encoded));
        assert!(valid_password_hash(&encoded));
    }

    #[test]
    fn base64_round_trips_password_wire_and_rejects_bad_padding() {
        let bytes: Vec<u8> = (0..48).collect();
        let encoded = base64_encode(&bytes);
        assert_eq!(base64_decode(&encoded), Some(bytes));
        assert_eq!(encoded.len(), 64);
        assert_eq!(base64_decode("abc="), Some(vec![105, 183]));
        assert_eq!(base64_decode("ab=c"), None);
    }

    #[test]
    fn millisecond_dates_match_javascript_iso_shape() {
        assert_eq!(
            iso_from_millis(0).expect("epoch"),
            "1970-01-01T00:00:00.000Z"
        );
        assert_eq!(
            iso_from_millis(1_735_787_045_006).expect("date"),
            "2025-01-02T03:04:05.006Z"
        );
    }

    #[test]
    fn result_digest_is_domain_separated_and_stable() {
        let one = digest_fields(
            b"frame.legacy-video-property.result.v1\0",
            &["success", "{\"success\":true}"],
        );
        let two = digest_fields(
            b"frame.legacy-video-property.result.v1\0",
            &["success", "{\"success\":true}"],
        );
        assert_eq!(one, two);
        assert_eq!(one.len(), 64);
    }
}
