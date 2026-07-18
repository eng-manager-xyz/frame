//! D1 authority for Cap transcript visibility, retry state, and durable effects.

use frame_application::{LegacyTranscriptResultV1, LegacyTranscriptSurfaceV1};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const VIDEO_AUTHORITY_SQL: &str = include_str!("../queries/legacy_transcripts/video_authority.sql");
const ACTOR_EMAIL_SQL: &str = include_str!("../queries/legacy_transcripts/actor_email.sql");
const EXPLICIT_ACCESS_SQL: &str = include_str!("../queries/legacy_transcripts/explicit_access.sql");
const PASSWORD_CANDIDATES_SQL: &str =
    include_str!("../queries/legacy_transcripts/password_candidates.sql");
const RETRY_STATUS_RESET_SQL: &str =
    include_str!("../queries/legacy_transcripts/retry_status_reset.sql");
const OPERATION_BY_KEY_SQL: &str =
    include_str!("../queries/legacy_transcripts/operation_by_key.sql");
const OPERATION_INSERT_SQL: &str =
    include_str!("../queries/legacy_transcripts/operation_insert.sql");
const OPERATION_STORAGE_APPLIED_SQL: &str =
    include_str!("../queries/legacy_transcripts/operation_storage_applied.sql");
const OPERATION_COMPLETE_SQL: &str =
    include_str!("../queries/legacy_transcripts/operation_complete.sql");
const STORAGE_RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_transcripts/storage_receipt_insert.sql");
const TRANSLATION_OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_transcripts/translation_outbox_insert.sql");
const OPERATION_PROVIDER_PENDING_SQL: &str =
    include_str!("../queries/legacy_transcripts/operation_provider_pending.sql");

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyTranscriptRuntimeErrorV1 {
    Invalid,
    NotFound,
    Unauthorized,
    Conflict,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyTranscriptVideoV1 {
    pub(crate) mapped_video_id: String,
    pub(crate) legacy_video_id: String,
    pub(crate) owner_id: String,
    pub(crate) organization_id: Option<String>,
    pub(crate) object_prefix: String,
    pub(crate) transcription_status: Option<String>,
    legacy_public: i64,
    video_password_hash: Option<String>,
    allowed_email_restriction: Option<String>,
}

impl LegacyTranscriptVideoV1 {
    fn validate(&self) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        if self.mapped_video_id.len() != 36
            || self.legacy_video_id.is_empty()
            || self.legacy_video_id.len() > 1020
            || self.owner_id.len() != 36
            || self
                .organization_id
                .as_ref()
                .is_some_and(|value| value.len() != 36)
            || !matches!(self.legacy_public, 0 | 1)
            || self.video_password_hash.as_deref().is_some_and(|value| {
                value.len() != 64
                    || value.bytes().any(|byte| {
                        !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'/' | b'=')
                    })
            })
            || self
                .allowed_email_restriction
                .as_ref()
                .is_some_and(|value| value.len() > 4096 || value.chars().any(char::is_control))
            || self.transcription_status.as_deref().is_some_and(|value| {
                !matches!(
                    value,
                    "PROCESSING" | "COMPLETE" | "ERROR" | "SKIPPED" | "NO_AUDIO"
                )
            })
        {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        frame_application::legacy_transcript_object_key(&self.object_prefix, None)
            .ok_or(LegacyTranscriptRuntimeErrorV1::Corrupt)?;
        Ok(())
    }

    #[must_use]
    pub(crate) fn is_owner(&self, actor_id: Option<&str>) -> bool {
        actor_id == Some(self.owner_id.as_str())
    }

    #[must_use]
    pub(crate) fn original_object_key(&self) -> Option<String> {
        frame_application::legacy_transcript_object_key(&self.object_prefix, None)
    }

    #[must_use]
    pub(crate) fn translated_object_key(&self, language: &str) -> Option<String> {
        frame_application::legacy_transcript_object_key(&self.object_prefix, Some(language))
    }
}

#[derive(Debug, Deserialize)]
struct ActorEmailRowV1 {
    email: String,
}

#[derive(Debug, Deserialize)]
struct AllowedRowV1 {
    allowed: i64,
}

#[derive(Debug, Deserialize)]
struct PasswordCandidateRowV1 {
    password_hash: String,
    ordinal: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyTranscriptOperationRowV1 {
    pub(crate) operation_id: String,
    pub(crate) request_digest: String,
    pub(crate) state: String,
    pub(crate) result_json: Option<String>,
    pub(crate) failure_code: Option<String>,
    pub(crate) object_key: String,
    pub(crate) target_language: Option<String>,
    pub(crate) entry_id: Option<i64>,
    pub(crate) replacement_text: Option<String>,
    pub(crate) attempt_count: i64,
}

impl LegacyTranscriptOperationRowV1 {
    fn validate(&self) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        if self.operation_id.len() != 36
            || !valid_hex_digest(&self.request_digest)
            || !matches!(
                self.state.as_str(),
                "claimed" | "storage_applied" | "provider_pending" | "complete" | "failed"
            )
            || self.attempt_count < 0
            || self.result_json.as_deref().is_some_and(|value| {
                value.len() > 262_144 || serde_json::from_str::<serde_json::Value>(value).is_err()
            })
            || self
                .failure_code
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 64)
            || self.object_key.is_empty()
            || self.target_language.as_deref().is_some_and(|value| {
                frame_application::legacy_transcript_language_name(value).is_none()
            })
            || self
                .entry_id
                .is_some_and(|value| !(0..=MAX_SAFE_INTEGER).contains(&value))
            || self
                .replacement_text
                .as_ref()
                .is_some_and(|value| value.len() > 262_144)
        {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        Ok(())
    }

    pub(crate) fn completed_result(
        &self,
    ) -> Result<Option<LegacyTranscriptResultV1>, LegacyTranscriptRuntimeErrorV1> {
        if self.state != "complete" {
            return Ok(None);
        }
        let value = self
            .result_json
            .as_deref()
            .ok_or(LegacyTranscriptRuntimeErrorV1::Corrupt)?;
        serde_json::from_str(value)
            .map(Some)
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)
    }
}

pub(crate) struct NewLegacyTranscriptOperationV1<'a> {
    pub(crate) operation_id: &'a str,
    pub(crate) surface: LegacyTranscriptSurfaceV1,
    pub(crate) actor_scope_digest: &'a str,
    pub(crate) actor_id: Option<&'a str>,
    pub(crate) video: &'a LegacyTranscriptVideoV1,
    pub(crate) idempotency_key_digest: &'a str,
    pub(crate) request_digest: &'a str,
    pub(crate) object_key: &'a str,
    pub(crate) target_language: Option<&'a str>,
    pub(crate) entry_id: Option<u64>,
    pub(crate) replacement_text: Option<&'a str>,
    pub(crate) now_ms: i64,
}

pub(crate) struct D1LegacyTranscriptAuthorityV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyTranscriptAuthorityV1<'database> {
    #[must_use]
    pub(crate) const fn new(database: &'database D1Database) -> Self {
        Self { database }
    }

    pub(crate) async fn video(
        &self,
        legacy_video_id: &str,
    ) -> Result<LegacyTranscriptVideoV1, LegacyTranscriptRuntimeErrorV1> {
        if legacy_video_id.is_empty()
            || legacy_video_id.len() > 1020
            || legacy_video_id.chars().any(char::is_control)
        {
            return Err(LegacyTranscriptRuntimeErrorV1::Invalid);
        }
        let mut rows = self
            .rows::<LegacyTranscriptVideoV1>(VIDEO_AUTHORITY_SQL, vec![js(legacy_video_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        let row = rows.pop().ok_or(LegacyTranscriptRuntimeErrorV1::NotFound)?;
        row.validate()?;
        Ok(row)
    }

    pub(crate) async fn can_view(
        &self,
        video: &LegacyTranscriptVideoV1,
        actor_id: Option<&str>,
        verified_password_hashes: &[String],
    ) -> Result<bool, LegacyTranscriptRuntimeErrorV1> {
        if video.is_owner(actor_id) {
            return Ok(true);
        }
        let actor_email = match actor_id {
            Some(actor_id) => self.actor_email(actor_id).await?,
            None => None,
        };
        let explicit = match actor_id {
            Some(actor_id) => {
                self.explicit_access(&video.mapped_video_id, actor_id)
                    .await?
            }
            None => false,
        };
        let candidates = self.password_candidates(&video.mapped_video_id).await?;
        let password_verified = candidates.is_empty()
            || candidates.iter().any(|candidate| {
                verified_password_hashes
                    .iter()
                    .any(|value| value == candidate)
            });
        if explicit {
            return Ok(password_verified);
        }
        if video.legacy_public != 1 {
            return Ok(false);
        }
        if let Some(restriction) = video
            .allowed_email_restriction
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some(email) = actor_email.as_deref() else {
                return Ok(false);
            };
            if !email_allowed(email, restriction) {
                return Ok(false);
            }
        }
        Ok(password_verified)
    }

    pub(crate) async fn reset_retry_status(
        &self,
        video: &LegacyTranscriptVideoV1,
        now_ms: i64,
    ) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        let result = self
            .run(
                RETRY_STATUS_RESET_SQL,
                vec![
                    js(&video.mapped_video_id),
                    number(now_ms),
                    js(&video.owner_id),
                ],
            )
            .await?;
        if changes(&result) != Some(1) {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        Ok(())
    }

    pub(crate) async fn claim_operation(
        &self,
        operation: &NewLegacyTranscriptOperationV1<'_>,
    ) -> Result<LegacyTranscriptOperationRowV1, LegacyTranscriptRuntimeErrorV1> {
        if !valid_hex_digest(operation.actor_scope_digest)
            || !valid_hex_digest(operation.idempotency_key_digest)
            || !valid_hex_digest(operation.request_digest)
            || !(0..=MAX_SAFE_INTEGER).contains(&operation.now_ms)
        {
            return Err(LegacyTranscriptRuntimeErrorV1::Invalid);
        }
        let kind = match operation.surface {
            LegacyTranscriptSurfaceV1::Retry => "retry",
            LegacyTranscriptSurfaceV1::Edit => "edit",
            LegacyTranscriptSurfaceV1::Translate => "translate",
            LegacyTranscriptSurfaceV1::Get | LegacyTranscriptSurfaceV1::AvailableTranslations => {
                return Err(LegacyTranscriptRuntimeErrorV1::Invalid);
            }
        };
        self.run(
            OPERATION_INSERT_SQL,
            vec![
                js(operation.operation_id),
                js(operation.surface.operation_id()),
                js(kind),
                js(operation.actor_scope_digest),
                js_opt(operation.actor_id),
                js(&operation.video.mapped_video_id),
                js(&operation.video.legacy_video_id),
                js(operation.idempotency_key_digest),
                js(operation.request_digest),
                js(operation.object_key),
                js_opt(operation.target_language),
                operation
                    .entry_id
                    .map(|value| number(i64::try_from(value).unwrap_or(i64::MAX)))
                    .unwrap_or(JsValue::NULL),
                js_opt(operation.replacement_text),
                js(match operation.surface {
                    LegacyTranscriptSurfaceV1::Translate => "claimed",
                    _ => "claimed",
                }),
                number(operation.now_ms),
            ],
        )
        .await?;
        let row = self
            .operation(
                operation.surface,
                operation.actor_scope_digest,
                &operation.video.mapped_video_id,
                operation.idempotency_key_digest,
            )
            .await?
            .ok_or(LegacyTranscriptRuntimeErrorV1::Unavailable)?;
        if row.request_digest != operation.request_digest
            || row.object_key != operation.object_key
            || row.target_language.as_deref() != operation.target_language
            || row.entry_id
                != operation
                    .entry_id
                    .and_then(|value| i64::try_from(value).ok())
            || row.replacement_text.as_deref() != operation.replacement_text
        {
            return Err(LegacyTranscriptRuntimeErrorV1::Conflict);
        }
        Ok(row)
    }

    pub(crate) async fn mark_storage_applied(
        &self,
        operation: &LegacyTranscriptOperationRowV1,
        prior_etag: Option<&str>,
        applied_etag: &str,
        content_sha256: &str,
        content_bytes: u64,
        now_ms: i64,
    ) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        let statements = vec![
            self.statement(
                STORAGE_RECEIPT_INSERT_SQL,
                vec![
                    js(&operation.operation_id),
                    js(&operation.object_key),
                    js_opt(prior_etag),
                    js(applied_etag),
                    js(content_sha256),
                    number(
                        i64::try_from(content_bytes)
                            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Invalid)?,
                    ),
                    number(now_ms),
                ],
            )?,
            self.statement(
                OPERATION_STORAGE_APPLIED_SQL,
                vec![
                    js(&operation.operation_id),
                    number(now_ms),
                    js(&operation.request_digest),
                ],
            )?,
        ];
        self.batch(statements).await
    }

    pub(crate) async fn queue_translation(
        &self,
        operation: &LegacyTranscriptOperationRowV1,
        source_object_key: &str,
        target_language: &str,
        now_ms: i64,
    ) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        let statements = vec![
            self.statement(
                TRANSLATION_OUTBOX_INSERT_SQL,
                vec![
                    js(&operation.operation_id),
                    js(source_object_key),
                    js(&operation.object_key),
                    js(target_language),
                    number(now_ms),
                ],
            )?,
            self.statement(
                OPERATION_PROVIDER_PENDING_SQL,
                vec![
                    js(&operation.operation_id),
                    number(now_ms),
                    js(&operation.request_digest),
                ],
            )?,
        ];
        self.batch(statements).await
    }

    pub(crate) async fn complete(
        &self,
        operation: &LegacyTranscriptOperationRowV1,
        result: &LegacyTranscriptResultV1,
        now_ms: i64,
    ) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        let result_json =
            serde_json::to_string(result).map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)?;
        let updated = self
            .run(
                OPERATION_COMPLETE_SQL,
                vec![
                    js(&operation.operation_id),
                    js(&result_json),
                    number(now_ms),
                    js(&operation.request_digest),
                ],
            )
            .await?;
        if changes(&updated) != Some(1) {
            let current = self
                .operation_by_id(&operation.operation_id)
                .await?
                .ok_or(LegacyTranscriptRuntimeErrorV1::Corrupt)?;
            if current.request_digest != operation.request_digest || current.state != "complete" {
                return Err(LegacyTranscriptRuntimeErrorV1::Conflict);
            }
        }
        Ok(())
    }

    async fn actor_email(
        &self,
        actor_id: &str,
    ) -> Result<Option<String>, LegacyTranscriptRuntimeErrorV1> {
        let mut rows = self
            .rows::<ActorEmailRowV1>(ACTOR_EMAIL_SQL, vec![js(actor_id)])
            .await?;
        if rows.len() > 1 {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        Ok(rows.pop().map(|row| row.email))
    }

    async fn explicit_access(
        &self,
        mapped_video_id: &str,
        actor_id: &str,
    ) -> Result<bool, LegacyTranscriptRuntimeErrorV1> {
        let mut rows = self
            .rows::<AllowedRowV1>(EXPLICIT_ACCESS_SQL, vec![js(mapped_video_id), js(actor_id)])
            .await?;
        if rows.len() != 1 || !matches!(rows[0].allowed, 0 | 1) {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        Ok(rows.pop().is_some_and(|row| row.allowed == 1))
    }

    async fn password_candidates(
        &self,
        mapped_video_id: &str,
    ) -> Result<Vec<String>, LegacyTranscriptRuntimeErrorV1> {
        let rows = self
            .rows::<PasswordCandidateRowV1>(PASSWORD_CANDIDATES_SQL, vec![js(mapped_video_id)])
            .await?;
        if rows.iter().enumerate().any(|(index, row)| {
            row.ordinal != i64::try_from(index).unwrap_or(i64::MAX) || row.password_hash.len() != 64
        }) {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        Ok(rows.into_iter().map(|row| row.password_hash).collect())
    }

    async fn operation(
        &self,
        surface: LegacyTranscriptSurfaceV1,
        actor_scope_digest: &str,
        mapped_video_id: &str,
        idempotency_key_digest: &str,
    ) -> Result<Option<LegacyTranscriptOperationRowV1>, LegacyTranscriptRuntimeErrorV1> {
        let mut rows = self
            .rows::<LegacyTranscriptOperationRowV1>(
                OPERATION_BY_KEY_SQL,
                vec![
                    js(surface.operation_id()),
                    js(actor_scope_digest),
                    js(mapped_video_id),
                    js(idempotency_key_digest),
                ],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        let row = rows.pop();
        if let Some(row) = &row {
            row.validate()?;
        }
        Ok(row)
    }

    async fn operation_by_id(
        &self,
        operation_id: &str,
    ) -> Result<Option<LegacyTranscriptOperationRowV1>, LegacyTranscriptRuntimeErrorV1> {
        let mut rows = self
            .rows::<LegacyTranscriptOperationRowV1>(
                "SELECT operation_id,request_digest,state,result_json,failure_code,object_key,target_language,entry_id,replacement_text,attempt_count FROM legacy_transcript_operations_v1 WHERE operation_id=?1 LIMIT 2",
                vec![js(operation_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
        }
        let row = rows.pop();
        if let Some(row) = &row {
            row.validate()?;
        }
        Ok(row)
    }

    fn statement(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> Result<D1PreparedStatement, LegacyTranscriptRuntimeErrorV1> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)
    }

    async fn rows<T>(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> Result<Vec<T>, LegacyTranscriptRuntimeErrorV1>
    where
        T: for<'de> Deserialize<'de>,
    {
        let result = self
            .statement(sql, bindings)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
        }
        result
            .results::<T>()
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)
    }

    async fn run(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> Result<D1Result, LegacyTranscriptRuntimeErrorV1> {
        let result = self
            .statement(sql, bindings)?
            .run()
            .into_send()
            .await
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
        }
        Ok(result)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyTranscriptRuntimeErrorV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
        if results.len() != expected || results.iter().any(|result| !result.success()) {
            return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
        }
        Ok(())
    }
}

fn email_allowed(email: &str, restriction: &str) -> bool {
    let email = email.to_lowercase();
    restriction
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let entry = entry.to_lowercase();
            if entry.contains('@') {
                email == entry
            } else {
                email.ends_with(&format!("@{entry}"))
            }
        })
}

fn changes(result: &D1Result) -> Option<usize> {
    result.meta().ok().flatten().and_then(|meta| meta.changes)
}

fn valid_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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
    fn email_restrictions_match_exact_addresses_or_domains() {
        assert!(email_allowed("Person@Example.com", "example.com"));
        assert!(email_allowed(
            "person@example.com",
            "other.test, PERSON@example.com"
        ));
        assert!(!email_allowed("person@badexample.com", "example.com"));
        assert!(!email_allowed("person@example.com", "other.test"));
    }

    #[test]
    fn queries_pin_public_policy_and_restart_safe_effects() {
        assert!(VIDEO_AUTHORITY_SQL.contains("legacy_allowed_email_restriction"));
        assert!(EXPLICIT_ACCESS_SQL.contains("organization_members"));
        assert!(EXPLICIT_ACCESS_SQL.contains("space_members"));
        assert!(PASSWORD_CANDIDATES_SQL.contains("legacy_password_hash"));
        assert!(OPERATION_INSERT_SQL.contains("INSERT OR IGNORE"));
        assert!(TRANSLATION_OUTBOX_INSERT_SQL.contains("openai/gpt-oss-120b"));
    }
}
