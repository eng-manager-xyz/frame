//! Durable D1 staging for hardware-gated legacy media contracts.
//!
//! The adapter writes the receipt and execution outbox in one D1 batch and
//! then reports an evidence gate. It never invokes a codec, accelerator, AI,
//! transcription, object-signing, or other provider from the request path.

use frame_application::{
    LegacyProtectedMediaAuthV1, LegacyProtectedMediaEnvelopeV1, LegacyProtectedMediaKindV1,
    LegacyProtectedMediaProfileV1, LegacyProtectedMediaReplayOriginV1,
    LegacyProtectedMediaTerminalKindV1, LegacyProtectedMediaValidationErrorV1,
    validate_legacy_protected_media_envelope,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const RECEIPT_REPLAY_SQL: &str =
    include_str!("../queries/legacy_protected_media/receipt_replay.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_media/receipt_insert.sql");
const OUTBOX_INSERT_SQL: &str = include_str!("../queries/legacy_protected_media/outbox_insert.sql");
const GENERATED_REPLAY_SQL: &str =
    include_str!("../queries/legacy_protected_media/generated_replay.sql");
const GENERATED_CLAIM_UPSERT_SQL: &str =
    include_str!("../queries/legacy_protected_media/generated_claim_upsert.sql");
const RECEIPT_UNIQUE: &str = "UNIQUE constraint failed: legacy_protected_media_receipts_v1";
pub(crate) const LEGACY_PROTECTED_MEDIA_REPLAY_RETENTION_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProtectedMediaFailureV1 {
    Invalid,
    Conflict,
    ExecutionEvidenceRequired,
    Corrupt,
    Unavailable,
}

impl From<LegacyProtectedMediaValidationErrorV1> for LegacyProtectedMediaFailureV1 {
    fn from(_: LegacyProtectedMediaValidationErrorV1) -> Self {
        Self::Invalid
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyProtectedMediaStageOutcomeV1 {
    ExecutionEvidenceRequired {
        receipt_id: String,
        replayed: bool,
        provider_execution_required: bool,
    },
    VerifiedSealedTerminal {
        receipt_id: String,
        terminal_kind: LegacyProtectedMediaTerminalKindV1,
        sealed_terminal_ref: String,
        sealed_terminal_digest: String,
    },
}

#[derive(Debug, Deserialize)]
struct ReceiptRow {
    receipt_id: String,
    request_digest: String,
    state: String,
    provider_required: i64,
    terminal_kind: String,
    sealed_terminal_ref: Option<String>,
    sealed_terminal_digest: Option<String>,
    terminal_expires_at_ms: Option<i64>,
}

pub struct D1LegacyProtectedMediaRuntimeV1<'a> {
    database: &'a D1Database,
}

impl<'a> D1LegacyProtectedMediaRuntimeV1<'a> {
    #[must_use]
    pub const fn new(database: &'a D1Database) -> Self {
        Self { database }
    }

    pub async fn stage(
        &self,
        profile: &LegacyProtectedMediaProfileV1,
        envelope: &LegacyProtectedMediaEnvelopeV1,
        now_ms: i64,
    ) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
        if envelope.source_operation_id != profile.operation_id
            || !(0..=9_007_199_254_740_991).contains(&now_ms)
        {
            return Err(LegacyProtectedMediaFailureV1::Invalid);
        }
        let validated = validate_legacy_protected_media_envelope(envelope)?;
        let prior = if validated.replay_origin == LegacyProtectedMediaReplayOriginV1::Natural {
            self.prior_generated(
                profile.operation_id,
                &validated.principal_digest,
                &validated.request_digest,
                &validated.authority_binding_digest,
                now_ms,
            )
            .await?
        } else {
            self.prior(
                profile.operation_id,
                &validated.principal_digest,
                &validated.execution_key_digest,
                &validated.authority_binding_digest,
                now_ms,
            )
            .await?
        };
        if let Some(row) = prior {
            return project_prior(
                row,
                &validated.request_digest,
                profile.terminal_kind(),
                now_ms,
            );
        }

        let receipt_id = Uuid::new_v4().to_string();
        let descriptor = json!({
            "schema_version": "frame.legacy-protected-media-execution.v2",
            "receipt_id": receipt_id,
            "source_operation_id": profile.operation_id,
            "kind": profile.kind.as_str(),
            "method": profile.method,
            "path": profile.path,
            "principal_digest": validated.principal_digest,
            "target_id": validated.target_id,
            "request_digest": validated.request_digest,
            "request_descriptor": serde_json::from_str::<serde_json::Value>(
                &validated.request_descriptor_json,
            )
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?,
            "terminal_kind": profile.terminal_kind().as_str(),
            "executor_kind": profile.executor_kind().as_str(),
            "required_evidence": {
                "executor": profile.executor_kind().as_str(),
                "provider_execution": profile.provider_execution_required,
            },
        });
        let descriptor_json = serde_json::to_string(&descriptor)
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
        let descriptor_digest = digest(descriptor_json.as_bytes());
        let credential_subject_id = envelope
            .principal
            .credential_subject_id
            .as_deref()
            .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        let credential_key_version = envelope
            .principal
            .credential_key_version
            .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        let credential_digest = envelope
            .principal
            .credential_digest
            .as_deref()
            .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        let policy_proofs_json = serde_json::to_string(&envelope.principal.policy_proofs)
            .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
        let entitlement = envelope.principal.entitlement_binding.as_ref();
        let authority_class = authority_class(profile, envelope)?;

        let receipt = self.statement(
            RECEIPT_INSERT_SQL,
            vec![
                text(&receipt_id),
                text(profile.operation_id),
                text(profile.kind.as_str()),
                text(profile.method),
                text(profile.path),
                text(profile.auth.as_str()),
                text(authority_class),
                text(&validated.principal_digest),
                nullable(envelope.principal.actor_id.as_deref()),
                nullable(envelope.principal.tenant_id.as_deref()),
                text(&envelope.principal.credential_kind),
                text(credential_subject_id),
                number(credential_key_version),
                text(credential_digest),
                text(&policy_proofs_json),
                nullable(entitlement.map(|value| value.kind.as_str())),
                nullable(entitlement.map(|value| value.subject_id.as_str())),
                optional_number(entitlement.map(|value| value.revision)),
                optional_number(entitlement.and_then(|value| value.expires_at_ms)),
                nullable(validated.target_id.as_deref()),
                text(&validated.authority_binding_digest),
                nullable(envelope.parent_family.as_deref()),
                nullable(envelope.parent_receipt_id.as_deref()),
                nullable(envelope.parent_request_digest.as_deref()),
                nullable(envelope.parent_authority_binding_digest.as_deref()),
                text(&validated.execution_key_digest),
                text(validated.replay_origin.as_str()),
                text(profile.idempotency.as_str()),
                text(&validated.request_digest),
                text(&validated.payload_digest),
                text(&validated.request_descriptor_json),
                nullable(envelope.sealed_request_ref.as_deref()),
                nullable(envelope.sealed_request_digest.as_deref()),
                text(profile.terminal_kind().as_str()),
                text(profile.executor_kind().as_str()),
                number(i64::from(profile.provider_execution_required)),
                number(now_ms),
            ],
        )?;
        let outbox = self.statement(
            OUTBOX_INSERT_SQL,
            vec![
                text(&receipt_id),
                text(profile.executor_kind().as_str()),
                text(&descriptor_json),
                text(&descriptor_digest),
                number(now_ms),
            ],
        )?;

        let mut statements = vec![receipt, outbox];
        if validated.replay_origin == LegacyProtectedMediaReplayOriginV1::Natural {
            statements.push(self.statement(
                GENERATED_CLAIM_UPSERT_SQL,
                vec![
                    text(profile.operation_id),
                    text(&validated.principal_digest),
                    text(&validated.request_digest),
                    text(&receipt_id),
                    number(now_ms),
                ],
            )?);
        }
        match self.batch(statements).await {
            Ok(()) => Ok(
                LegacyProtectedMediaStageOutcomeV1::ExecutionEvidenceRequired {
                    receipt_id,
                    replayed: false,
                    provider_execution_required: profile.provider_execution_required,
                },
            ),
            Err(LegacyProtectedMediaFailureV1::Conflict) => {
                let row = if validated.replay_origin == LegacyProtectedMediaReplayOriginV1::Natural
                {
                    self.prior_generated(
                        profile.operation_id,
                        &validated.principal_digest,
                        &validated.request_digest,
                        &validated.authority_binding_digest,
                        now_ms,
                    )
                    .await?
                } else {
                    self.prior(
                        profile.operation_id,
                        &validated.principal_digest,
                        &validated.execution_key_digest,
                        &validated.authority_binding_digest,
                        now_ms,
                    )
                    .await?
                }
                .ok_or(LegacyProtectedMediaFailureV1::Conflict)?;
                project_prior(
                    row,
                    &validated.request_digest,
                    profile.terminal_kind(),
                    now_ms,
                )
            }
            Err(failure) => Err(failure),
        }
    }

    async fn prior(
        &self,
        operation_id: &str,
        principal_digest: &str,
        execution_key_digest: &str,
        authority_binding_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedMediaFailureV1> {
        let result = self
            .statement(
                RECEIPT_REPLAY_SQL,
                vec![
                    text(operation_id),
                    text(principal_digest),
                    text(execution_key_digest),
                    text(authority_binding_digest),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        let rows = result
            .results::<ReceiptRow>()
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
        Ok(rows.into_iter().next())
    }

    async fn prior_generated(
        &self,
        operation_id: &str,
        principal_digest: &str,
        request_digest: &str,
        authority_binding_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedMediaFailureV1> {
        let result = self
            .statement(
                GENERATED_REPLAY_SQL,
                vec![
                    text(operation_id),
                    text(principal_digest),
                    text(request_digest),
                    text(authority_binding_digest),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        let rows = result
            .results::<ReceiptRow>()
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
        Ok(rows.into_iter().next())
    }

    fn statement(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> Result<D1PreparedStatement, LegacyProtectedMediaFailureV1> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyProtectedMediaFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyProtectedMediaFailureV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1(failed.error().as_deref().unwrap_or_default()));
        }
        Ok(())
    }
}

fn authority_class(
    profile: &LegacyProtectedMediaProfileV1,
    envelope: &LegacyProtectedMediaEnvelopeV1,
) -> Result<&'static str, LegacyProtectedMediaFailureV1> {
    let class = match profile.auth {
        LegacyProtectedMediaAuthV1::SchedulerSecret => "scheduler_service",
        LegacyProtectedMediaAuthV1::InternalService => "internal_service",
        LegacyProtectedMediaAuthV1::PublicEdgeOrJobCapability => {
            if envelope.principal.credential_kind == "job_capability" {
                "job_status"
            } else {
                "public_edge"
            }
        }
        LegacyProtectedMediaAuthV1::OptionalSessionOrShareCapability => {
            if envelope.principal.credential_kind == "session_token" {
                "video_view"
            } else {
                "video_share"
            }
        }
        LegacyProtectedMediaAuthV1::PublicOrFlowToken => {
            if envelope.principal.credential_kind == "flow_token" {
                "flow_service"
            } else {
                "active_session"
            }
        }
        LegacyProtectedMediaAuthV1::Session => match profile.operation_id {
            "cap-v1-24ef9eb18c4b0555" => "organization_member",
            "cap-v1-c1ae43fcf8ad7018" => "video_view_ai_owner_entitled",
            "cap-v1-39909646286251af" => "video_owner_ai_entitled",
            _ if profile.target_field == Some("videoId")
                && (profile.kind == LegacyProtectedMediaKindV1::ServerAction
                    || profile.method == "POST") =>
            {
                "video_owner"
            }
            _ if profile.target_field == Some("videoId") => "video_view",
            _ => "active_session",
        },
        LegacyProtectedMediaAuthV1::ParentDerived => {
            if envelope.principal.policy_proofs.is_empty() {
                match envelope.principal.credential_kind.as_str() {
                    "scheduler_secret" => "scheduler_service",
                    "service_secret" => "internal_service",
                    "flow_token" => "flow_service",
                    "edge_read" => "public_edge",
                    "job_capability" => "job_status",
                    "session_token" => "active_session",
                    _ => return Err(LegacyProtectedMediaFailureV1::Invalid),
                }
            } else if envelope.principal.entitlement_binding.is_some()
                && envelope
                    .principal
                    .policy_proofs
                    .iter()
                    .all(|proof| proof.kind == "owner_bypass")
            {
                "video_owner_ai_entitled"
            } else if envelope
                .principal
                .policy_proofs
                .iter()
                .all(|proof| proof.kind == "owner_bypass")
            {
                "video_owner"
            } else if envelope.principal.actor_id.is_some() {
                "video_view"
            } else {
                "video_share"
            }
        }
    };
    Ok(class)
}

fn project_prior(
    row: ReceiptRow,
    expected_request_digest: &str,
    expected_terminal_kind: LegacyProtectedMediaTerminalKindV1,
    now_ms: i64,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    if row.request_digest != expected_request_digest
        || !matches!(row.provider_required, 0 | 1)
        || row.terminal_kind != expected_terminal_kind.as_str()
    {
        return Err(LegacyProtectedMediaFailureV1::Conflict);
    }
    match row.state.as_str() {
        "pending_execution_evidence" => Ok(
            LegacyProtectedMediaStageOutcomeV1::ExecutionEvidenceRequired {
                receipt_id: row.receipt_id,
                replayed: true,
                provider_execution_required: row.provider_required == 1,
            },
        ),
        "verified" => {
            let sealed_terminal_ref = row
                .sealed_terminal_ref
                .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
            let sealed_terminal_digest = row
                .sealed_terminal_digest
                .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
            let terminal_expires_at_ms = row
                .terminal_expires_at_ms
                .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
            if !valid_sealed_terminal_ref(&sealed_terminal_ref)
                || !valid_lower_digest(&sealed_terminal_digest)
            {
                return Err(LegacyProtectedMediaFailureV1::Corrupt);
            }
            if terminal_expires_at_ms <= now_ms {
                return Err(LegacyProtectedMediaFailureV1::ExecutionEvidenceRequired);
            }
            Ok(LegacyProtectedMediaStageOutcomeV1::VerifiedSealedTerminal {
                receipt_id: row.receipt_id,
                terminal_kind: expected_terminal_kind,
                sealed_terminal_ref,
                sealed_terminal_digest,
            })
        }
        "dead_letter" => Err(LegacyProtectedMediaFailureV1::ExecutionEvidenceRequired),
        _ => Err(LegacyProtectedMediaFailureV1::Corrupt),
    }
}

fn map_d1(message: &str) -> LegacyProtectedMediaFailureV1 {
    if message.contains(RECEIPT_UNIQUE)
        || message.contains("frame_protected_media_generated_replay_claimed_v1")
    {
        LegacyProtectedMediaFailureV1::Conflict
    } else if message.contains("authority_stale") || message.contains("workflow_parent_invalid") {
        LegacyProtectedMediaFailureV1::Unavailable
    } else if message.contains("constraint") || message.contains("immutable") {
        LegacyProtectedMediaFailureV1::Corrupt
    } else {
        LegacyProtectedMediaFailureV1::Unavailable
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_lower_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_sealed_terminal_ref(value: &str) -> bool {
    value
        .strip_prefix("frame-pm-terminal-v1:")
        .is_some_and(valid_lower_digest)
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn nullable(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn optional_number(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_receipt_never_projects_success() {
        let row = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "pending_execution_evidence".into(),
            provider_required: 1,
            terminal_kind: "json".into(),
            sealed_terminal_ref: None,
            sealed_terminal_digest: None,
            terminal_expires_at_ms: None,
        };
        assert!(matches!(
            project_prior(
                row,
                &"a".repeat(64),
                LegacyProtectedMediaTerminalKindV1::Json,
                1,
            ),
            Ok(
                LegacyProtectedMediaStageOutcomeV1::ExecutionEvidenceRequired {
                    replayed: true,
                    provider_execution_required: true,
                    ..
                }
            )
        ));
    }

    #[test]
    fn verified_terminal_must_be_opaque_well_formed_and_live() {
        let row = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "verified".into(),
            provider_required: 0,
            terminal_kind: "json".into(),
            sealed_terminal_ref: Some("plaintext-response".into()),
            sealed_terminal_digest: Some("0".repeat(64)),
            terminal_expires_at_ms: Some(10),
        };
        assert_eq!(
            project_prior(
                row,
                &"a".repeat(64),
                LegacyProtectedMediaTerminalKindV1::Json,
                1,
            ),
            Err(LegacyProtectedMediaFailureV1::Corrupt)
        );
    }

    #[test]
    fn verified_terminal_projects_only_a_typed_sealed_reference() {
        let row = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "verified".into(),
            provider_required: 0,
            terminal_kind: "redirect".into(),
            sealed_terminal_ref: Some(format!("frame-pm-terminal-v1:{}", "b".repeat(64))),
            sealed_terminal_digest: Some("c".repeat(64)),
            terminal_expires_at_ms: Some(10),
        };
        assert!(matches!(
            project_prior(
                row,
                &"a".repeat(64),
                LegacyProtectedMediaTerminalKindV1::Redirect,
                1,
            ),
            Ok(LegacyProtectedMediaStageOutcomeV1::VerifiedSealedTerminal {
                terminal_kind: LegacyProtectedMediaTerminalKindV1::Redirect,
                ..
            })
        ));
    }
}
