//! Durable D1 staging for externally protected authentication, billing, and
//! administrator contracts.
//!
//! The request path atomically writes an immutable receipt, provider outbox,
//! and (for protected billing/admin surfaces) human-approval request. It has no
//! API for adding evidence or marking an operation successful.

use frame_application::{
    LegacyProtectedBillingAuthAuthV1, LegacyProtectedBillingAuthCredentialKindV1,
    LegacyProtectedBillingAuthEnvelopeV1, LegacyProtectedBillingAuthKindV1,
    LegacyProtectedBillingAuthPrincipalV1, LegacyProtectedBillingAuthProfileV1,
    LegacyProtectedBillingAuthReplayOriginV1, LegacyProtectedBillingAuthValidatedV1,
    LegacyProtectedBillingAuthValidationErrorV1, ValidatedBrowserMutationProof,
    validate_legacy_protected_billing_auth_envelope,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const AUTHORITY_READ_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/authority_read.sql");
const RECEIPT_REPLAY_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/receipt_replay.sql");
const RECEIPT_REPLAY_ASSERT_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/receipt_replay_assert.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/receipt_insert.sql");
const GENERATED_RECEIPT_REPLAY_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/generated_receipt_replay.sql");
const GENERATED_CLAIM_UPSERT_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/generated_claim_upsert.sql");
const OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/outbox_insert.sql");
const APPROVAL_REQUEST_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/approval_request_insert.sql");
const DELIVERY_AUDIT_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/delivery_audit_insert.sql");
const WORKFLOW_PARENT_READ_SQL: &str =
    include_str!("../queries/legacy_protected_billing_auth/workflow_parent_read.sql");
const RECEIPT_UNIQUE: &str = "UNIQUE constraint failed: legacy_protected_billing_auth_receipts_v1";
const AUTHORITY_STALE: &str = "frame_protected_billing_auth_authority_stale_v1";
const WORKFLOW_PARENT_INVALID: &str = "frame_protected_billing_auth_workflow_parent_invalid_v1";
const GENERATED_REPLAY_CLAIMED: &str = "frame_protected_billing_auth_generated_replay_claimed_v1";
pub(crate) const GENERATED_REPLAY_TERMINAL_RETENTION_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProtectedBillingAuthFailureV1 {
    Invalid,
    Unauthorized,
    Conflict,
    HumanApprovalRejected,
    EvidenceRequired,
    Corrupt,
    Unavailable,
}

impl From<LegacyProtectedBillingAuthValidationErrorV1> for LegacyProtectedBillingAuthFailureV1 {
    fn from(_: LegacyProtectedBillingAuthValidationErrorV1) -> Self {
        Self::Invalid
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyProtectedBillingAuthStageOutcomeV1 {
    EvidenceRequired {
        receipt_id: String,
        provider: String,
        replayed: bool,
        human_approval_required: bool,
        provider_execution_required: bool,
    },
    /// A trusted provider stored the secret-bearing HTTP response outside D1.
    /// Only this opaque binding may leave the durable runtime; the web adapter
    /// must use its narrow resolver before any status/header/body is emitted.
    VerifiedSealedHttp {
        receipt_id: String,
        sealed_response_ref: String,
        sealed_response_digest: String,
    },
}

#[derive(Debug, Deserialize)]
struct AuthorityRow {
    authorized: i64,
}

#[derive(Debug, Deserialize)]
struct ReceiptRow {
    receipt_id: String,
    request_digest: String,
    state: String,
    provider_kind: String,
    human_approval_required: i64,
    human_decision: Option<String>,
    sealed_response_ref: Option<String>,
    sealed_response_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkflowParentRow {
    actor_id: String,
    target_id: String,
    credential_kind: String,
    credential_subject_id: String,
    credential_key_version: i64,
    credential_digest: String,
}

pub struct D1LegacyProtectedBillingAuthRuntimeV1<'a> {
    database: &'a D1Database,
}

impl<'a> D1LegacyProtectedBillingAuthRuntimeV1<'a> {
    #[must_use]
    pub const fn new(database: &'a D1Database) -> Self {
        Self { database }
    }

    pub async fn stage(
        &self,
        profile: &LegacyProtectedBillingAuthProfileV1,
        envelope: &LegacyProtectedBillingAuthEnvelopeV1,
        now_ms: i64,
    ) -> Result<LegacyProtectedBillingAuthStageOutcomeV1, LegacyProtectedBillingAuthFailureV1> {
        self.stage_with_browser_proof(profile, envelope, None, now_ms)
            .await
    }

    /// Stage an authenticated browser action while consuming its one-use
    /// mutation grant in the same D1 transaction as the receipt and outbox.
    /// A replay consumes the new grant only after an in-transaction assertion
    /// proves that the exact immutable receipt already exists.
    pub async fn stage_with_browser_proof(
        &self,
        profile: &LegacyProtectedBillingAuthProfileV1,
        envelope: &LegacyProtectedBillingAuthEnvelopeV1,
        browser_proof: Option<&ValidatedBrowserMutationProof>,
        now_ms: i64,
    ) -> Result<LegacyProtectedBillingAuthStageOutcomeV1, LegacyProtectedBillingAuthFailureV1> {
        if profile.operation_id == "cap-v1-572763e7b4977abd"
            || envelope.source_operation_id != profile.operation_id
            || !(0..=9_007_199_254_740_991).contains(&now_ms)
        {
            return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
        }
        let validated = validate_legacy_protected_billing_auth_envelope(envelope)?;
        let authority = self
            .authority(profile, envelope, validated.target_id.as_deref(), now_ms)
            .await?;
        if authority.authorized != 1 {
            return Err(LegacyProtectedBillingAuthFailureV1::Unauthorized);
        }

        if let Some(prior) = self
            .prior_for_validated(profile, &validated, now_ms)
            .await?
        {
            let prior_receipt_id = prior.receipt_id.clone();
            let outcome = project_prior(prior, &validated.request_digest)?;
            self.record_transport_delivery(
                &prior_receipt_id,
                envelope,
                &validated.request_digest,
                now_ms,
            )
            .await?;
            if let Some(proof) = browser_proof {
                self.consume_browser_grant_for_replay(
                    profile.operation_id,
                    &validated.principal_digest,
                    &validated.replay_key_digest,
                    &validated.request_digest,
                    proof,
                    now_ms,
                )
                .await?;
            }
            return Ok(outcome);
        }

        let receipt_id = Uuid::new_v4().to_string();
        let redacted_request: Value = serde_json::from_str(&validated.request_json)
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Corrupt)?;
        let payload = json!({
            "schema_version": "frame.legacy-protected-billing-auth-outbox.v1",
            "receipt_id": receipt_id,
            "source_operation_id": profile.operation_id,
            "kind": profile.kind.as_str(),
            "method": profile.method,
            "path": profile.path,
            "provider": profile.provider,
            "principal_digest": validated.principal_digest,
            "target_id": validated.target_id,
            "request_digest": validated.request_digest,
            "redacted_request": redacted_request,
            "required_evidence": {
                "human_approval": profile.human_approval_required,
                "provider_execution": true,
            },
            "release_gate": "independent_human_and_provider_evidence",
        });
        let payload_json = serde_json::to_string(&payload)
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Corrupt)?;
        let payload_digest = digest(payload_json.as_bytes());

        let receipt = self.statement(
            RECEIPT_INSERT_SQL,
            vec![
                text(&receipt_id),
                text(profile.operation_id),
                text(profile.kind.as_str()),
                text(profile.method),
                text(profile.path),
                text(profile.auth.as_str()),
                text(profile.authority.as_str()),
                text(profile.provider),
                number(i64::from(profile.human_approval_required)),
                text(&validated.principal_digest),
                nullable(envelope.principal.actor_id.as_deref()),
                text(envelope.principal.credential_kind.as_str()),
                nullable(envelope.principal.credential_subject_id.as_deref()),
                optional_number(envelope.principal.credential_key_version),
                nullable(envelope.principal.credential_digest.as_deref()),
                nullable(envelope.sealed_request_ref.as_deref()),
                nullable(envelope.sealed_request_digest.as_deref()),
                nullable(validated.target_id.as_deref()),
                text(&validated.replay_key_digest),
                text(validated.replay_origin.as_str()),
                text(profile.idempotency.as_str()),
                text(&validated.request_digest),
                text(&validated.request_json),
                number(now_ms),
            ],
        )?;
        let outbox = self.statement(
            OUTBOX_INSERT_SQL,
            vec![
                text(&receipt_id),
                text(profile.provider),
                text(&payload_json),
                text(&payload_digest),
                number(i64::from(profile.human_approval_required)),
                number(now_ms),
            ],
        )?;
        let mut statements = Vec::with_capacity(if profile.human_approval_required {
            if browser_proof.is_some() { 7 } else { 3 }
        } else if browser_proof.is_some() {
            6
        } else {
            2
        });
        if let Some(proof) = browser_proof {
            statements.push(
                crate::browser_web_runtime::grant_assertion_statement(
                    self.database,
                    &receipt_id,
                    proof,
                    now_ms,
                )
                .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?,
            );
        }
        statements.push(receipt);
        if validated.replay_origin == LegacyProtectedBillingAuthReplayOriginV1::Generated {
            statements.push(self.statement(
                GENERATED_CLAIM_UPSERT_SQL,
                vec![
                    text(profile.operation_id),
                    text(&validated.principal_digest),
                    text(&validated.request_digest),
                    text(&receipt_id),
                    number(now_ms),
                    number(now_ms.saturating_sub(GENERATED_REPLAY_TERMINAL_RETENTION_MS)),
                ],
            )?);
        }
        if let Some(delivery) = self.transport_delivery_statement(
            &receipt_id,
            envelope,
            &validated.request_digest,
            now_ms,
        )? {
            statements.push(delivery);
        }
        statements.push(outbox);
        if profile.human_approval_required {
            statements.push(self.statement(
                APPROVAL_REQUEST_INSERT_SQL,
                vec![
                    text(&receipt_id),
                    text(&validated.request_digest),
                    number(now_ms),
                ],
            )?);
        }
        if let Some(proof) = browser_proof {
            statements.push(
                crate::browser_web_runtime::grant_delete_statement(self.database, proof)
                    .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?,
            );
            statements.push(
                crate::browser_web_runtime::change_assertion_statement(
                    self.database,
                    &receipt_id,
                    "grant_consumed",
                )
                .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?,
            );
            statements.push(self.statement(
                "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?1",
                vec![text(&receipt_id)],
            )?);
        }

        match self.batch(statements).await {
            Ok(()) => Ok(LegacyProtectedBillingAuthStageOutcomeV1::EvidenceRequired {
                receipt_id,
                provider: profile.provider.to_owned(),
                replayed: false,
                human_approval_required: profile.human_approval_required,
                provider_execution_required: true,
            }),
            Err(LegacyProtectedBillingAuthFailureV1::Conflict) => {
                let prior = self
                    .prior_for_validated(profile, &validated, now_ms)
                    .await?
                    .ok_or(LegacyProtectedBillingAuthFailureV1::Conflict)?;
                let prior_receipt_id = prior.receipt_id.clone();
                let outcome = project_prior(prior, &validated.request_digest)?;
                self.record_transport_delivery(
                    &prior_receipt_id,
                    envelope,
                    &validated.request_digest,
                    now_ms,
                )
                .await?;
                if let Some(proof) = browser_proof {
                    self.consume_browser_grant_for_replay(
                        profile.operation_id,
                        &validated.principal_digest,
                        &validated.replay_key_digest,
                        &validated.request_digest,
                        proof,
                        now_ms,
                    )
                    .await?;
                }
                Ok(outcome)
            }
            Err(failure) => Err(failure),
        }
    }

    fn transport_delivery_statement(
        &self,
        receipt_id: &str,
        envelope: &LegacyProtectedBillingAuthEnvelopeV1,
        request_digest: &str,
        now_ms: i64,
    ) -> Result<Option<D1PreparedStatement>, LegacyProtectedBillingAuthFailureV1> {
        match (
            envelope.transport_credential_digest.as_deref(),
            envelope.transport_body_digest.as_deref(),
        ) {
            (Some(credential_digest), Some(body_digest)) => self
                .statement(
                    DELIVERY_AUDIT_INSERT_SQL,
                    vec![
                        text(receipt_id),
                        text(credential_digest),
                        text(body_digest),
                        text(request_digest),
                        number(now_ms),
                    ],
                )
                .map(Some),
            (None, None) => Ok(None),
            _ => Err(LegacyProtectedBillingAuthFailureV1::Invalid),
        }
    }

    async fn record_transport_delivery(
        &self,
        receipt_id: &str,
        envelope: &LegacyProtectedBillingAuthEnvelopeV1,
        request_digest: &str,
        now_ms: i64,
    ) -> Result<(), LegacyProtectedBillingAuthFailureV1> {
        if let Some(statement) =
            self.transport_delivery_statement(receipt_id, envelope, request_digest, now_ms)?
        {
            self.batch(vec![statement]).await?;
        }
        Ok(())
    }

    /// Construct the administrator workflow intent only from the immutable
    /// receipt of the exact source action that admitted it. The caller cannot
    /// supply or upgrade an actor, video target, or replay namespace.
    pub async fn stage_workflow_from_parent(
        &self,
        workflow_profile: &LegacyProtectedBillingAuthProfileV1,
        parent_receipt_id: &str,
        parent_request_digest: &str,
        now_ms: i64,
    ) -> Result<LegacyProtectedBillingAuthStageOutcomeV1, LegacyProtectedBillingAuthFailureV1> {
        if workflow_profile.operation_id != "cap-v1-5a990f470c701cec"
            || workflow_profile.kind != LegacyProtectedBillingAuthKindV1::Workflow
            || workflow_profile.auth != LegacyProtectedBillingAuthAuthV1::AdminSession
            || parent_receipt_id.len() != 36
            || parent_request_digest.len() != 64
            || !parent_request_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
        }
        let result = self
            .statement(
                WORKFLOW_PARENT_READ_SQL,
                vec![
                    text(parent_receipt_id),
                    text(parent_request_digest),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        let parent = result
            .results::<WorkflowParentRow>()
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Corrupt)?
            .into_iter()
            .next()
            .ok_or(LegacyProtectedBillingAuthFailureV1::Unauthorized)?;
        let envelope = LegacyProtectedBillingAuthEnvelopeV1 {
            source_operation_id: workflow_profile.operation_id.into(),
            principal: LegacyProtectedBillingAuthPrincipalV1 {
                class: LegacyProtectedBillingAuthAuthV1::AdminSession,
                actor_id: Some(parent.actor_id),
                credential_kind: match parent.credential_kind.as_str() {
                    "session_token" => LegacyProtectedBillingAuthCredentialKindV1::SessionToken,
                    _ => return Err(LegacyProtectedBillingAuthFailureV1::Corrupt),
                },
                credential_subject_id: Some(parent.credential_subject_id),
                credential_key_version: Some(parent.credential_key_version),
                credential_digest: Some(parent.credential_digest),
            },
            caller_idempotency_key: Some(format!("parent-receipt:{parent_receipt_id}")),
            replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Natural,
            request_nonce: parent_receipt_id.into(),
            payload: json!({
                "videoId": parent.target_id,
                "_frameParentReceiptId": parent_receipt_id,
                "_frameParentRequestDigest": parent_request_digest,
            }),
            sealed_request_ref: None,
            sealed_request_digest: None,
            transport_body_digest: None,
            transport_credential_digest: None,
        };
        self.stage(workflow_profile, &envelope, now_ms).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn consume_browser_grant_for_replay(
        &self,
        operation_id: &str,
        principal_digest: &str,
        replay_key_digest: &str,
        request_digest: &str,
        proof: &ValidatedBrowserMutationProof,
        now_ms: i64,
    ) -> Result<(), LegacyProtectedBillingAuthFailureV1> {
        let assertion_id = Uuid::new_v4().to_string();
        let statements = vec![
            self.statement(
                RECEIPT_REPLAY_ASSERT_SQL,
                vec![
                    text(&assertion_id),
                    text(operation_id),
                    text(principal_digest),
                    text(replay_key_digest),
                    text(request_digest),
                    number(now_ms),
                ],
            )?,
            crate::browser_web_runtime::grant_assertion_statement(
                self.database,
                &assertion_id,
                proof,
                now_ms,
            )
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?,
            crate::browser_web_runtime::grant_delete_statement(self.database, proof)
                .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?,
            crate::browser_web_runtime::change_assertion_statement(
                self.database,
                &assertion_id,
                "grant_consumed",
            )
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?,
            self.statement(
                "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?1",
                vec![text(&assertion_id)],
            )?,
        ];
        self.batch(statements).await
    }

    async fn authority(
        &self,
        profile: &LegacyProtectedBillingAuthProfileV1,
        envelope: &LegacyProtectedBillingAuthEnvelopeV1,
        target_id: Option<&str>,
        now_ms: i64,
    ) -> Result<AuthorityRow, LegacyProtectedBillingAuthFailureV1> {
        let result = self
            .statement(
                AUTHORITY_READ_SQL,
                vec![
                    nullable(envelope.principal.actor_id.as_deref()),
                    text(profile.authority.as_str()),
                    nullable(target_id),
                    text(envelope.principal.credential_kind.as_str()),
                    nullable(envelope.principal.credential_subject_id.as_deref()),
                    optional_number(envelope.principal.credential_key_version),
                    nullable(envelope.principal.credential_digest.as_deref()),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        result
            .results::<AuthorityRow>()
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Corrupt)?
            .into_iter()
            .next()
            .ok_or(LegacyProtectedBillingAuthFailureV1::Corrupt)
    }

    async fn prior(
        &self,
        operation_id: &str,
        principal_digest: &str,
        replay_key_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedBillingAuthFailureV1> {
        let result = self
            .statement(
                RECEIPT_REPLAY_SQL,
                vec![
                    text(operation_id),
                    text(principal_digest),
                    text(replay_key_digest),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        Ok(result
            .results::<ReceiptRow>()
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Corrupt)?
            .into_iter()
            .next())
    }

    async fn prior_for_validated(
        &self,
        profile: &LegacyProtectedBillingAuthProfileV1,
        validated: &LegacyProtectedBillingAuthValidatedV1,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedBillingAuthFailureV1> {
        if validated.replay_origin == LegacyProtectedBillingAuthReplayOriginV1::Generated {
            self.generated_prior(
                profile.operation_id,
                &validated.principal_digest,
                &validated.request_digest,
                now_ms,
            )
            .await
        } else {
            self.prior(
                profile.operation_id,
                &validated.principal_digest,
                &validated.replay_key_digest,
                now_ms,
            )
            .await
        }
    }

    async fn generated_prior(
        &self,
        operation_id: &str,
        principal_digest: &str,
        request_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedBillingAuthFailureV1> {
        let result = self
            .statement(
                GENERATED_RECEIPT_REPLAY_SQL,
                vec![
                    text(operation_id),
                    text(principal_digest),
                    text(request_digest),
                    number(now_ms.saturating_sub(GENERATED_REPLAY_TERMINAL_RETENTION_MS)),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        Ok(result
            .results::<ReceiptRow>()
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Corrupt)?
            .into_iter()
            .next())
    }

    fn statement(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> Result<D1PreparedStatement, LegacyProtectedBillingAuthFailureV1> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Unavailable)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyProtectedBillingAuthFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyProtectedBillingAuthFailureV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1(failed.error().as_deref().unwrap_or_default()));
        }
        Ok(())
    }
}

fn project_prior(
    row: ReceiptRow,
    expected_request_digest: &str,
) -> Result<LegacyProtectedBillingAuthStageOutcomeV1, LegacyProtectedBillingAuthFailureV1> {
    if row.request_digest != expected_request_digest {
        return Err(LegacyProtectedBillingAuthFailureV1::Conflict);
    }
    if row.human_decision.as_deref() == Some("rejected") || row.state == "rejected" {
        return Err(LegacyProtectedBillingAuthFailureV1::HumanApprovalRejected);
    }
    match row.state.as_str() {
        "awaiting_human_approval" | "awaiting_provider_evidence" => {
            Ok(LegacyProtectedBillingAuthStageOutcomeV1::EvidenceRequired {
                receipt_id: row.receipt_id,
                provider: row.provider_kind,
                replayed: true,
                human_approval_required: row.human_approval_required == 1
                    && row.human_decision.as_deref() != Some("approved"),
                provider_execution_required: true,
            })
        }
        "verified" => {
            let sealed_response_ref = row
                .sealed_response_ref
                .filter(|value| valid_sealed_response_ref(value))
                .ok_or(LegacyProtectedBillingAuthFailureV1::Corrupt)?;
            let sealed_response_digest = row
                .sealed_response_digest
                .filter(|value| valid_lower_digest(value))
                .ok_or(LegacyProtectedBillingAuthFailureV1::Corrupt)?;
            Ok(
                LegacyProtectedBillingAuthStageOutcomeV1::VerifiedSealedHttp {
                    receipt_id: row.receipt_id,
                    sealed_response_ref,
                    sealed_response_digest,
                },
            )
        }
        "dead_letter" => Err(LegacyProtectedBillingAuthFailureV1::EvidenceRequired),
        _ => Err(LegacyProtectedBillingAuthFailureV1::Corrupt),
    }
}

fn map_d1(message: &str) -> LegacyProtectedBillingAuthFailureV1 {
    if message.contains(AUTHORITY_STALE) || message.contains(WORKFLOW_PARENT_INVALID) {
        LegacyProtectedBillingAuthFailureV1::Unauthorized
    } else if message.contains(RECEIPT_UNIQUE) || message.contains(GENERATED_REPLAY_CLAIMED) {
        LegacyProtectedBillingAuthFailureV1::Conflict
    } else if message.contains("constraint") || message.contains("immutable") {
        LegacyProtectedBillingAuthFailureV1::Corrupt
    } else {
        LegacyProtectedBillingAuthFailureV1::Unavailable
    }
}

fn valid_lower_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_sealed_response_ref(value: &str) -> bool {
    value
        .strip_prefix("frame-pba-http-v1:")
        .is_some_and(valid_lower_digest)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn nullable(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn optional_number(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64))
}

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_human_gate_never_projects_provider_success() {
        let row = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "awaiting_human_approval".into(),
            provider_kind: "stripe_checkout".into(),
            human_approval_required: 1,
            human_decision: None,
            sealed_response_ref: None,
            sealed_response_digest: None,
        };
        assert!(matches!(
            project_prior(row, &"a".repeat(64)),
            Ok(LegacyProtectedBillingAuthStageOutcomeV1::EvidenceRequired {
                human_approval_required: true,
                provider_execution_required: true,
                replayed: true,
                ..
            })
        ));
    }

    #[test]
    fn verified_response_is_cryptographically_bound() {
        let row = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "verified".into(),
            provider_kind: "stripe_checkout".into(),
            human_approval_required: 1,
            human_decision: Some("approved".into()),
            sealed_response_ref: Some("plaintext-capability-url".into()),
            sealed_response_digest: Some("0".repeat(64)),
        };
        assert_eq!(
            project_prior(row, &"a".repeat(64)),
            Err(LegacyProtectedBillingAuthFailureV1::Corrupt)
        );
    }
}
