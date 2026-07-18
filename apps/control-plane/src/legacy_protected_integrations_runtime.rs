//! Durable D1 intent staging for provider-gated legacy integration contracts.
//!
//! The request path writes one immutable receipt and one immutable outbox row
//! in a D1 batch. It cannot mark either row verified and it never contacts an
//! external provider.

use frame_application::{
    LegacyProtectedIntegrationConditionalBindingV1, LegacyProtectedIntegrationEntitlementBindingV1,
    LegacyProtectedIntegrationEntitlementV1, LegacyProtectedIntegrationEnvelopeV1,
    LegacyProtectedIntegrationProfileV1, LegacyProtectedIntegrationReplayOriginV1,
    LegacyProtectedIntegrationValidatedV1, LegacyProtectedIntegrationValidationErrorV1,
    ValidatedBrowserMutationProof, legacy_protected_integration_authority_binding_digest,
    legacy_protected_integration_entitlement, legacy_protected_integration_target_domain,
    legacy_protected_integration_tenant_domain, validate_legacy_protected_integration_envelope,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, send::IntoSendFuture};

const AUTHORITY_READ_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/authority_read.sql");
const RECEIPT_REPLAY_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/receipt_replay.sql");
const RECEIPT_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/receipt_insert.sql");
const GENERATED_RECEIPT_REPLAY_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/generated_receipt_replay.sql");
const GENERATED_CLAIM_UPSERT_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/generated_claim_upsert.sql");
const OUTBOX_INSERT_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/outbox_insert.sql");
pub(crate) const WORKFLOW_PARENT_READ_SQL: &str =
    include_str!("../queries/legacy_protected_integrations/workflow_parent_read.sql");
const RECEIPT_REPLAY_ASSERT_SQL: &str = "INSERT INTO authenticated_web_action_assertions_v1(\
operation_id,assertion_kind,expected_count,actual_count) VALUES(?1,'operation_complete',1,(\
SELECT COUNT(*) FROM legacy_protected_integration_receipts_v1 receipt \
JOIN legacy_protected_integration_live_authority_v1 live ON live.receipt_id=receipt.receipt_id \
WHERE receipt.source_operation_id=?2 AND receipt.principal_digest=?3 \
AND receipt.replay_key_digest=?4 AND receipt.request_digest=?5 \
AND receipt.authority_binding_digest=?6 AND live.authority_expires_at_ms>?7))";
const RECEIPT_UNIQUE: &str = "UNIQUE constraint failed: legacy_protected_integration_receipts_v1";
const AUTHORITY_STALE: &str = "frame_protected_integration_authority_stale_v1";
const WORKFLOW_PARENT_INVALID: &str = "frame_protected_integration_workflow_parent_invalid_v1";
const GENERATED_REPLAY_CLAIMED: &str = "frame_protected_integration_generated_replay_claimed_v1";
pub(crate) const GENERATED_REPLAY_TERMINAL_RETENTION_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProtectedIntegrationFailureV1 {
    Invalid,
    Unauthorized,
    Conflict,
    ProviderEvidenceRequired,
    Corrupt,
    Unavailable,
}

impl From<LegacyProtectedIntegrationValidationErrorV1> for LegacyProtectedIntegrationFailureV1 {
    fn from(_: LegacyProtectedIntegrationValidationErrorV1) -> Self {
        Self::Invalid
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyProtectedIntegrationStageOutcomeV1 {
    ProviderEvidenceRequired {
        receipt_id: String,
        provider: String,
        replayed: bool,
    },
    VerifiedSealedTerminal {
        receipt_id: String,
        sealed_terminal_ref: String,
        sealed_terminal_digest: String,
    },
}

#[derive(Debug, Deserialize)]
struct AuthorityRow {
    authorized: i64,
    resolved_tenant_id: Option<String>,
    resolved_target_id: Option<String>,
    authority_expires_at_ms: Option<i64>,
    actor_revision: Option<i64>,
    scope_revision: Option<i64>,
    resource_revision: Option<i64>,
    current_public: Option<i64>,
    owner_id: Option<String>,
    owner_revision: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ReceiptRow {
    receipt_id: String,
    request_digest: String,
    state: String,
    provider_kind: String,
    sealed_terminal_ref: Option<String>,
    sealed_terminal_digest: Option<String>,
}

struct AuthorityInputV1<'a> {
    actor_id: Option<&'a str>,
    authority: &'a str,
    authenticated_tenant_id: Option<&'a str>,
    legacy_tenant_id: Option<&'a str>,
    legacy_target_id: Option<&'a str>,
    legacy_workflow_actor_id: Option<&'a str>,
    legacy_workflow_cap_tenant_id: Option<&'a str>,
    workflow_raw_file_key: Option<&'a str>,
    tenant_domain: &'a str,
    target_domain: &'a str,
    credential_kind: &'a str,
    credential_subject_id: Option<&'a str>,
    credential_key_version: Option<i64>,
    credential_digest: Option<&'a str>,
    credential_expires_at_ms: Option<i64>,
    policy_proofs_json: &'a str,
    entitlement: &'a str,
    operation_id: &'a str,
    now_ms: i64,
    parent_family: Option<&'a str>,
    parent_receipt_id: Option<&'a str>,
    parent_request_digest: Option<&'a str>,
    parent_authority_binding_digest: Option<&'a str>,
}

pub struct D1LegacyProtectedIntegrationRuntimeV1<'a> {
    database: &'a D1Database,
}

impl<'a> D1LegacyProtectedIntegrationRuntimeV1<'a> {
    #[must_use]
    pub const fn new(database: &'a D1Database) -> Self {
        Self { database }
    }

    pub async fn stage(
        &self,
        profile: &LegacyProtectedIntegrationProfileV1,
        envelope: &LegacyProtectedIntegrationEnvelopeV1,
        now_ms: i64,
    ) -> Result<LegacyProtectedIntegrationStageOutcomeV1, LegacyProtectedIntegrationFailureV1> {
        self.stage_with_browser_proof(profile, envelope, None, now_ms)
            .await
    }

    /// Consume an authenticated browser mutation grant in the same D1 batch as
    /// a new receipt/outbox, or in an exact live-replay assertion batch.
    pub async fn stage_with_browser_proof(
        &self,
        profile: &LegacyProtectedIntegrationProfileV1,
        envelope: &LegacyProtectedIntegrationEnvelopeV1,
        browser_proof: Option<&ValidatedBrowserMutationProof>,
        now_ms: i64,
    ) -> Result<LegacyProtectedIntegrationStageOutcomeV1, LegacyProtectedIntegrationFailureV1> {
        if envelope.source_operation_id != profile.operation_id
            || !(0..=9_007_199_254_740_991).contains(&now_ms)
        {
            return Err(LegacyProtectedIntegrationFailureV1::Invalid);
        }
        if browser_proof.is_some_and(|proof| {
            envelope.principal.actor_id.as_deref() != Some(&proof.user_id().to_string())
                || envelope.principal.credential_subject_id.as_deref()
                    != Some(&proof.session_id().to_string())
        }) {
            return Err(LegacyProtectedIntegrationFailureV1::Unauthorized);
        }
        let validated = validate_legacy_protected_integration_envelope(envelope)?;
        let policy_proofs_json = serde_json::to_string(&envelope.principal.policy_proofs)
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?;
        let entitlement = legacy_protected_integration_entitlement(profile.operation_id);
        let entitlement_kind_for_authority =
            if profile.auth == frame_application::LegacyProtectedIntegrationAuthV1::ParentReceipt {
                envelope
                    .principal
                    .inherited_entitlement_binding
                    .as_ref()
                    .map_or("none", |binding| binding.kind.as_str())
            } else {
                entitlement.as_str()
            };
        let tenant_domain = legacy_protected_integration_tenant_domain(profile.operation_id);
        let target_domain = legacy_protected_integration_target_domain(profile.operation_id);
        let legacy_workflow_actor_id = match profile.operation_id {
            "cap-v1-b9fcb0fbd25b2234" => payload_string(&envelope.payload, "/userId"),
            "cap-v1-bd1b9d67380624f7" => payload_string(&envelope.payload, "/cap/userId"),
            "cap-v1-f0a00e93ab606a52" => payload_string(&envelope.payload, "/loom/userId"),
            _ => None,
        };
        let legacy_workflow_cap_tenant_id = match profile.operation_id {
            "cap-v1-bd1b9d67380624f7" => payload_string(&envelope.payload, "/cap/orgId"),
            "cap-v1-f0a00e93ab606a52" => payload_string(&envelope.payload, "/loom/orgId"),
            _ => None,
        };
        let workflow_raw_file_key = (profile.operation_id == "cap-v1-b9fcb0fbd25b2234")
            .then(|| payload_string(&envelope.payload, "/rawFileKey"))
            .flatten();
        let authority = self
            .authority(&AuthorityInputV1 {
                actor_id: envelope.principal.actor_id.as_deref(),
                authority: profile.authority.as_str(),
                authenticated_tenant_id: validated.authenticated_tenant_id.as_deref(),
                legacy_tenant_id: validated.legacy_tenant_id.as_deref(),
                legacy_target_id: validated.legacy_target_id.as_deref(),
                legacy_workflow_actor_id,
                legacy_workflow_cap_tenant_id,
                workflow_raw_file_key,
                tenant_domain,
                target_domain,
                credential_kind: envelope.principal.credential_kind.as_str(),
                credential_subject_id: envelope.principal.credential_subject_id.as_deref(),
                credential_key_version: envelope.principal.credential_key_version,
                credential_digest: envelope.principal.credential_digest.as_deref(),
                credential_expires_at_ms: envelope.principal.credential_expires_at_ms,
                policy_proofs_json: &policy_proofs_json,
                entitlement: entitlement_kind_for_authority,
                operation_id: profile.operation_id,
                now_ms,
                parent_family: validated.parent_family.as_deref(),
                parent_receipt_id: validated.parent_receipt_id.as_deref(),
                parent_request_digest: validated.parent_request_digest.as_deref(),
                parent_authority_binding_digest: validated
                    .parent_authority_binding_digest
                    .as_deref(),
            })
            .await?;
        if authority.authorized != 1
            || authority
                .authority_expires_at_ms
                .is_none_or(|expires_at_ms| expires_at_ms <= now_ms)
        {
            return Err(LegacyProtectedIntegrationFailureV1::Unauthorized);
        }
        let tenant_id = authority.resolved_tenant_id.as_deref();
        let target_id = authority.resolved_target_id.as_deref();
        // The desktop create source treats its selectors as branch inputs:
        // an existing owned video ignores `orgId`, while an unknown `videoId`
        // falls through to create-new and is not an existing target binding.
        let receipt_legacy_tenant_id =
            if profile.operation_id == "cap-v1-60f863b2cb19353f" && target_id.is_some() {
                None
            } else {
                validated.legacy_tenant_id.as_deref()
            };
        let receipt_legacy_target_id =
            if profile.operation_id == "cap-v1-60f863b2cb19353f" && target_id.is_none() {
                None
            } else {
                validated.legacy_target_id.as_deref()
            };
        let conditional_bindings =
            derive_conditional_bindings(profile, envelope, &authority, tenant_id, target_id)?;
        let conditional_bindings_json = serde_json::to_string(&conditional_bindings)
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?;
        let entitlement_binding =
            if profile.auth == frame_application::LegacyProtectedIntegrationAuthV1::ParentReceipt {
                envelope.principal.inherited_entitlement_binding.clone()
            } else if entitlement == LegacyProtectedIntegrationEntitlementV1::None {
                None
            } else {
                Some(LegacyProtectedIntegrationEntitlementBindingV1 {
                    kind: entitlement.as_str().into(),
                    subject_id: envelope
                        .principal
                        .actor_id
                        .clone()
                        .ok_or(LegacyProtectedIntegrationFailureV1::Unauthorized)?,
                    revision: authority
                        .actor_revision
                        .ok_or(LegacyProtectedIntegrationFailureV1::Unauthorized)?,
                    expires_at_ms: None,
                })
            };
        let authority_binding_digest = legacy_protected_integration_authority_binding_digest(
            profile,
            &envelope.principal,
            tenant_id,
            target_id,
            entitlement_binding.as_ref(),
            &conditional_bindings,
        )?;

        if let Some(prior) = self
            .prior_for_validated(profile, &validated, &authority_binding_digest, now_ms)
            .await?
        {
            let outcome = project_prior(prior, &validated.request_digest)?;
            if let Some(proof) = browser_proof {
                self.consume_browser_grant_for_replay(
                    profile.operation_id,
                    &validated.principal_digest,
                    &validated.replay_key_digest,
                    &validated.request_digest,
                    &authority_binding_digest,
                    proof,
                    now_ms,
                )
                .await?;
            }
            return Ok(outcome);
        }

        let receipt_id = Uuid::new_v4().to_string();
        let request: Value = serde_json::from_str(&validated.request_json)
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?;
        let payload = json!({
            "schema_version": "frame.legacy-protected-integration-outbox.v1",
            "receipt_id": receipt_id,
            "source_operation_id": profile.operation_id,
            "kind": profile.kind.as_str(),
            "method": profile.method,
            "path": profile.path,
            "provider": profile.provider,
            "principal_digest": validated.principal_digest,
            "tenant_id": tenant_id,
            "target_id": target_id,
            "legacy_tenant_id": receipt_legacy_tenant_id,
            "legacy_target_id": receipt_legacy_target_id,
            "request_digest": validated.request_digest,
            "authority_binding_digest": authority_binding_digest,
            "conditional_bindings": conditional_bindings,
            "redacted_request": request,
            "sealed_request_ref": envelope.sealed_request_ref,
            "sealed_request_digest": envelope.sealed_request_digest,
            "release_gate": "independent_provider_executor_evidence",
        });
        let payload_json = serde_json::to_string(&payload)
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?;
        let payload_digest = digest(payload_json.as_bytes());
        let entitlement_kind = entitlement_binding
            .as_ref()
            .map(|binding| binding.kind.as_str());
        let entitlement_subject_id = entitlement_binding
            .as_ref()
            .map(|binding| binding.subject_id.as_str());
        let entitlement_revision = entitlement_binding.as_ref().map(|binding| binding.revision);
        let entitlement_expires_at_ms = entitlement_binding
            .as_ref()
            .and_then(|binding| binding.expires_at_ms);

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
                text(&validated.principal_digest),
                nullable(envelope.principal.actor_id.as_deref()),
                nullable(tenant_id),
                nullable(target_id),
                text(tenant_domain),
                text(target_domain),
                nullable(receipt_legacy_tenant_id),
                nullable(receipt_legacy_target_id),
                nullable(legacy_workflow_actor_id),
                nullable(legacy_workflow_cap_tenant_id),
                nullable(workflow_raw_file_key),
                text(envelope.principal.credential_kind.as_str()),
                nullable(envelope.principal.credential_subject_id.as_deref()),
                optional_number(envelope.principal.credential_key_version),
                nullable(envelope.principal.credential_digest.as_deref()),
                optional_number(envelope.principal.credential_expires_at_ms),
                text(&policy_proofs_json),
                nullable(entitlement_kind),
                nullable(entitlement_subject_id),
                optional_number(entitlement_revision),
                optional_number(entitlement_expires_at_ms),
                text(&conditional_bindings_json),
                text(&authority_binding_digest),
                nullable(validated.parent_family.as_deref()),
                nullable(validated.parent_receipt_id.as_deref()),
                nullable(validated.parent_request_digest.as_deref()),
                nullable(validated.parent_authority_binding_digest.as_deref()),
                text(&validated.replay_key_digest),
                text(validated.replay_origin.as_str()),
                text(profile.idempotency.as_str()),
                text(&validated.request_digest),
                text(&validated.request_json),
                text(&envelope.sealed_request_ref),
                text(&envelope.sealed_request_digest),
                nullable(envelope.transport_body_digest.as_deref()),
                text(terminal_kind(profile)),
                optional_number(conditional_duration_marker(&envelope.payload)),
                number(i64::from(password_requested(&envelope.payload))),
                number(i64::from(pro_space_settings_requested(&envelope.payload))),
                number(i64::from(public_requested(&envelope.payload))),
                optional_number(payload_integer(&envelope.payload, "/newQuantity")),
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
                number(now_ms),
            ],
        )?;

        let mut statements = Vec::with_capacity(if browser_proof.is_some() { 7 } else { 3 });
        if let Some(proof) = browser_proof {
            statements.push(
                crate::browser_web_runtime::grant_assertion_statement(
                    self.database,
                    &receipt_id,
                    proof,
                    now_ms,
                )
                .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?,
            );
        }
        statements.push(receipt);
        if validated.replay_origin == LegacyProtectedIntegrationReplayOriginV1::Generated {
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
        statements.push(outbox);
        if let Some(proof) = browser_proof {
            statements.push(
                crate::browser_web_runtime::grant_delete_statement(self.database, proof)
                    .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?,
            );
            statements.push(
                crate::browser_web_runtime::change_assertion_statement(
                    self.database,
                    &receipt_id,
                    "grant_consumed",
                )
                .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?,
            );
            statements.push(self.statement(
                "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?1",
                vec![text(&receipt_id)],
            )?);
        }

        match self.batch(statements).await {
            Ok(()) => Ok(
                LegacyProtectedIntegrationStageOutcomeV1::ProviderEvidenceRequired {
                    receipt_id,
                    provider: profile.provider.to_owned(),
                    replayed: false,
                },
            ),
            Err(LegacyProtectedIntegrationFailureV1::Conflict) => {
                let prior = self
                    .prior_for_validated(profile, &validated, &authority_binding_digest, now_ms)
                    .await?
                    .ok_or(LegacyProtectedIntegrationFailureV1::Conflict)?;
                let outcome = project_prior(prior, &validated.request_digest)?;
                if let Some(proof) = browser_proof {
                    self.consume_browser_grant_for_replay(
                        profile.operation_id,
                        &validated.principal_digest,
                        &validated.replay_key_digest,
                        &validated.request_digest,
                        &authority_binding_digest,
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

    async fn authority(
        &self,
        input: &AuthorityInputV1<'_>,
    ) -> Result<AuthorityRow, LegacyProtectedIntegrationFailureV1> {
        let result = self
            .statement(
                AUTHORITY_READ_SQL,
                vec![
                    nullable(input.actor_id),
                    text(input.authority),
                    nullable(input.authenticated_tenant_id),
                    nullable(input.legacy_tenant_id),
                    nullable(input.legacy_target_id),
                    text(input.tenant_domain),
                    text(input.target_domain),
                    text(input.credential_kind),
                    nullable(input.credential_subject_id),
                    optional_number(input.credential_key_version),
                    nullable(input.credential_digest),
                    optional_number(input.credential_expires_at_ms),
                    text(input.policy_proofs_json),
                    text(input.entitlement),
                    text(input.operation_id),
                    number(input.now_ms),
                    nullable(input.legacy_workflow_actor_id),
                    nullable(input.legacy_workflow_cap_tenant_id),
                    nullable(input.parent_family),
                    nullable(input.parent_receipt_id),
                    nullable(input.parent_request_digest),
                    nullable(input.parent_authority_binding_digest),
                    nullable(input.workflow_raw_file_key),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        result
            .results::<AuthorityRow>()
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?
            .into_iter()
            .next()
            .ok_or(LegacyProtectedIntegrationFailureV1::Corrupt)
    }

    async fn prior(
        &self,
        operation_id: &str,
        principal_digest: &str,
        replay_key_digest: &str,
        authority_binding_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedIntegrationFailureV1> {
        let result = self
            .statement(
                RECEIPT_REPLAY_SQL,
                vec![
                    text(operation_id),
                    text(principal_digest),
                    text(replay_key_digest),
                    text(authority_binding_digest),
                    number(now_ms),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        Ok(result
            .results::<ReceiptRow>()
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?
            .into_iter()
            .next())
    }

    async fn generated_prior(
        &self,
        operation_id: &str,
        principal_digest: &str,
        request_digest: &str,
        authority_binding_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedIntegrationFailureV1> {
        let result = self
            .statement(
                GENERATED_RECEIPT_REPLAY_SQL,
                vec![
                    text(operation_id),
                    text(principal_digest),
                    text(request_digest),
                    text(authority_binding_digest),
                    number(now_ms),
                    number(now_ms.saturating_sub(GENERATED_REPLAY_TERMINAL_RETENTION_MS)),
                ],
            )?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?;
        if !result.success() {
            return Err(map_d1(result.error().as_deref().unwrap_or_default()));
        }
        Ok(result
            .results::<ReceiptRow>()
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?
            .into_iter()
            .next())
    }

    async fn prior_for_validated(
        &self,
        profile: &LegacyProtectedIntegrationProfileV1,
        validated: &LegacyProtectedIntegrationValidatedV1,
        authority_binding_digest: &str,
        now_ms: i64,
    ) -> Result<Option<ReceiptRow>, LegacyProtectedIntegrationFailureV1> {
        match validated.replay_origin {
            LegacyProtectedIntegrationReplayOriginV1::Generated => {
                self.generated_prior(
                    profile.operation_id,
                    &validated.principal_digest,
                    &validated.request_digest,
                    authority_binding_digest,
                    now_ms,
                )
                .await
            }
            LegacyProtectedIntegrationReplayOriginV1::Natural => {
                self.prior(
                    profile.operation_id,
                    &validated.principal_digest,
                    &validated.replay_key_digest,
                    authority_binding_digest,
                    now_ms,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn consume_browser_grant_for_replay(
        &self,
        operation_id: &str,
        principal_digest: &str,
        replay_key_digest: &str,
        request_digest: &str,
        authority_binding_digest: &str,
        proof: &ValidatedBrowserMutationProof,
        now_ms: i64,
    ) -> Result<(), LegacyProtectedIntegrationFailureV1> {
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
                    text(authority_binding_digest),
                    number(now_ms),
                ],
            )?,
            crate::browser_web_runtime::grant_assertion_statement(
                self.database,
                &assertion_id,
                proof,
                now_ms,
            )
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?,
            crate::browser_web_runtime::grant_delete_statement(self.database, proof)
                .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?,
            crate::browser_web_runtime::change_assertion_statement(
                self.database,
                &assertion_id,
                "grant_consumed",
            )
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?,
            self.statement(
                "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?1",
                vec![text(&assertion_id)],
            )?,
        ];
        self.batch(statements).await
    }

    fn statement(
        &self,
        sql: &str,
        bindings: Vec<JsValue>,
    ) -> Result<D1PreparedStatement, LegacyProtectedIntegrationFailureV1> {
        self.database
            .prepare(sql)
            .bind(&bindings)
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)
    }

    async fn batch(
        &self,
        statements: Vec<D1PreparedStatement>,
    ) -> Result<(), LegacyProtectedIntegrationFailureV1> {
        let expected = statements.len();
        let results: Vec<D1Result> = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|error| map_d1(&error.to_string()))?;
        if results.len() != expected {
            return Err(LegacyProtectedIntegrationFailureV1::Unavailable);
        }
        if let Some(failed) = results.iter().find(|result| !result.success()) {
            return Err(map_d1(failed.error().as_deref().unwrap_or_default()));
        }
        Ok(())
    }
}

fn derive_conditional_bindings(
    profile: &LegacyProtectedIntegrationProfileV1,
    envelope: &LegacyProtectedIntegrationEnvelopeV1,
    authority: &AuthorityRow,
    tenant_id: Option<&str>,
    target_id: Option<&str>,
) -> Result<Vec<LegacyProtectedIntegrationConditionalBindingV1>, LegacyProtectedIntegrationFailureV1>
{
    let mut bindings = Vec::new();
    let actor_id = envelope.principal.actor_id.as_deref();
    match profile.operation_id {
        "cap-v1-60f863b2cb19353f" => {
            if target_id.is_some() {
                bindings.push(conditional_binding(
                    "video_existing_owner",
                    target_id,
                    authority.resource_revision,
                    None,
                )?);
            } else {
                bindings.push(conditional_binding(
                    "video_new_organization_member",
                    tenant_id,
                    authority.scope_revision,
                    None,
                )?);
            }
            if payload_finite_number(&envelope.payload, "/durationInSecs")
                .is_some_and(|value| value > 300.0)
            {
                bindings.push(conditional_binding(
                    "video_duration_pro",
                    actor_id,
                    authority.actor_revision,
                    None,
                )?);
            }
        }
        "cap-v1-0c233c1115838206" | "cap-v1-5e7e4265d65c8365" => {
            if password_requested(&envelope.payload) {
                bindings.push(conditional_binding(
                    "space_password_pro",
                    actor_id,
                    authority.actor_revision,
                    None,
                )?);
            }
            if pro_space_settings_requested(&envelope.payload) {
                bindings.push(conditional_binding(
                    "space_settings_pro",
                    actor_id,
                    authority.actor_revision,
                    None,
                )?);
            }
            if public_requested(&envelope.payload) {
                bindings.push(conditional_binding(
                    "space_publish_owner_pro",
                    authority.owner_id.as_deref(),
                    authority.owner_revision,
                    None,
                )?);
            }
        }
        "cap-v1-3a394a2798233b0b" | "cap-v1-d05af581fbeb145e" => {
            if password_requested(&envelope.payload) {
                bindings.push(conditional_binding(
                    "space_password_pro",
                    actor_id,
                    authority.actor_revision,
                    None,
                )?);
            }
            if public_requested(&envelope.payload) && authority.current_public == Some(0) {
                bindings.push(conditional_binding(
                    "space_publish_owner_pro",
                    authority.owner_id.as_deref(),
                    authority.owner_revision,
                    None,
                )?);
            }
        }
        "cap-v1-aa00fc906599e89c" | "cap-v1-17470f7df902263e" => {
            bindings.push(conditional_binding(
                "seat_capacity",
                tenant_id,
                authority.scope_revision,
                payload_integer(&envelope.payload, "/newQuantity"),
            )?);
        }
        _ => {}
    }
    Ok(bindings)
}

fn conditional_binding(
    kind: &str,
    subject_id: Option<&str>,
    revision: Option<i64>,
    value: Option<i64>,
) -> Result<LegacyProtectedIntegrationConditionalBindingV1, LegacyProtectedIntegrationFailureV1> {
    Ok(LegacyProtectedIntegrationConditionalBindingV1 {
        kind: kind.into(),
        subject_id: subject_id
            .filter(|value| !value.is_empty())
            .ok_or(LegacyProtectedIntegrationFailureV1::Unauthorized)?
            .into(),
        revision: revision.ok_or(LegacyProtectedIntegrationFailureV1::Unauthorized)?,
        value,
    })
}

fn payload_integer(payload: &Value, pointer: &str) -> Option<i64> {
    payload.pointer(pointer).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn payload_finite_number(payload: &Value, pointer: &str) -> Option<f64> {
    payload
        .pointer(pointer)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn conditional_duration_marker(payload: &Value) -> Option<i64> {
    payload_finite_number(payload, "/durationInSecs")
        .map(|seconds| if seconds > 300.0 { 301 } else { 0 })
}

fn payload_string<'a>(payload: &'a Value, pointer: &str) -> Option<&'a str> {
    payload.pointer(pointer).and_then(Value::as_str)
}

fn payload_bool(payload: &Value, pointer: &str) -> bool {
    payload.pointer(pointer).is_some_and(|value| {
        value.as_bool() == Some(true)
            || value
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    })
}

fn password_requested(payload: &Value) -> bool {
    payload_bool(payload, "/passwordEnabled")
        || payload.pointer("/passwordAction").and_then(Value::as_str) == Some("set")
}

fn pro_space_settings_requested(payload: &Value) -> bool {
    ["/disableSummary", "/disableChapters", "/disableTranscript"]
        .iter()
        .any(|pointer| payload_bool(payload, pointer))
}

fn public_requested(payload: &Value) -> bool {
    payload_bool(payload, "/public")
}

fn terminal_kind(profile: &LegacyProtectedIntegrationProfileV1) -> &'static str {
    match profile.kind {
        frame_application::LegacyProtectedIntegrationKindV1::Route => "http",
        frame_application::LegacyProtectedIntegrationKindV1::Rpc
        | frame_application::LegacyProtectedIntegrationKindV1::ServerAction => "json",
        frame_application::LegacyProtectedIntegrationKindV1::Workflow => "workflow",
    }
}

fn project_prior(
    row: ReceiptRow,
    expected_request_digest: &str,
) -> Result<LegacyProtectedIntegrationStageOutcomeV1, LegacyProtectedIntegrationFailureV1> {
    if row.request_digest != expected_request_digest {
        return Err(LegacyProtectedIntegrationFailureV1::Conflict);
    }
    match row.state.as_str() {
        "pending_provider_evidence" => Ok(
            LegacyProtectedIntegrationStageOutcomeV1::ProviderEvidenceRequired {
                receipt_id: row.receipt_id,
                provider: row.provider_kind,
                replayed: true,
            },
        ),
        "verified" => {
            let sealed_terminal_ref = row
                .sealed_terminal_ref
                .ok_or(LegacyProtectedIntegrationFailureV1::Corrupt)?;
            let sealed_terminal_digest = row
                .sealed_terminal_digest
                .ok_or(LegacyProtectedIntegrationFailureV1::Corrupt)?;
            if !valid_opaque_ref("frame-pi-terminal-v1:", &sealed_terminal_ref)
                || !valid_digest(&sealed_terminal_digest)
            {
                return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
            }
            Ok(
                LegacyProtectedIntegrationStageOutcomeV1::VerifiedSealedTerminal {
                    receipt_id: row.receipt_id,
                    sealed_terminal_ref,
                    sealed_terminal_digest,
                },
            )
        }
        "dead_letter" => Err(LegacyProtectedIntegrationFailureV1::ProviderEvidenceRequired),
        _ => Err(LegacyProtectedIntegrationFailureV1::Corrupt),
    }
}

fn map_d1(message: &str) -> LegacyProtectedIntegrationFailureV1 {
    if message.contains(AUTHORITY_STALE) || message.contains(WORKFLOW_PARENT_INVALID) {
        LegacyProtectedIntegrationFailureV1::Unauthorized
    } else if message.contains(RECEIPT_UNIQUE) || message.contains(GENERATED_REPLAY_CLAIMED) {
        LegacyProtectedIntegrationFailureV1::Conflict
    } else if message.contains("constraint") || message.contains("immutable") {
        LegacyProtectedIntegrationFailureV1::Corrupt
    } else {
        LegacyProtectedIntegrationFailureV1::Unavailable
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_opaque_ref(prefix: &str, value: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(valid_digest)
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

fn number(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn optional_number(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conditional_test_envelope(
        operation_id: &str,
        payload: Value,
    ) -> LegacyProtectedIntegrationEnvelopeV1 {
        LegacyProtectedIntegrationEnvelopeV1 {
            source_operation_id: operation_id.into(),
            principal: frame_application::LegacyProtectedIntegrationPrincipalV1 {
                class: frame_application::LegacyProtectedIntegrationAuthV1::Session,
                actor_id: Some("actor.user.v1".into()),
                tenant_id: Some("tenant.organization.v1".into()),
                credential_kind:
                    frame_application::LegacyProtectedIntegrationCredentialKindV1::SessionToken,
                credential_subject_id: Some("session.v1".into()),
                credential_key_version: Some(1),
                credential_digest: Some("a".repeat(64)),
                credential_expires_at_ms: None,
                policy_proofs: Vec::new(),
                inherited_entitlement_binding: None,
            },
            replay_origin: LegacyProtectedIntegrationReplayOriginV1::Generated,
            request_nonce: "request.v1".into(),
            payload,
            sealed_request_ref: format!("frame-pi-request-v1:{}", "b".repeat(64)),
            sealed_request_digest: "c".repeat(64),
            transport_body_digest: None,
            parent_family: None,
            parent_receipt_id: None,
            parent_request_digest: None,
            parent_authority_binding_digest: None,
        }
    }

    #[test]
    fn pending_receipt_never_projects_provider_success() {
        let row = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "pending_provider_evidence".into(),
            provider_kind: "github_releases".into(),
            sealed_terminal_ref: None,
            sealed_terminal_digest: None,
        };
        assert!(matches!(
            project_prior(row, &"a".repeat(64)),
            Ok(
                LegacyProtectedIntegrationStageOutcomeV1::ProviderEvidenceRequired {
                    replayed: true,
                    ..
                }
            )
        ));
    }

    #[test]
    fn f60_fractional_duration_uses_the_exact_pro_threshold() {
        assert_eq!(conditional_duration_marker(&json!({})), None);
        assert_eq!(
            conditional_duration_marker(&json!({"durationInSecs":300})),
            Some(0)
        );
        assert_eq!(
            conditional_duration_marker(&json!({"durationInSecs":300.5})),
            Some(301)
        );
        assert_eq!(
            conditional_duration_marker(&json!({"durationInSecs":301})),
            Some(301)
        );
    }

    #[test]
    fn create_space_binds_actor_features_and_owner_plan_separately() {
        let profile =
            frame_application::legacy_protected_integration_profile("cap-v1-0c233c1115838206")
                .expect("create-space profile");
        let envelope = conditional_test_envelope(
            profile.operation_id,
            json!({
                "passwordEnabled": true,
                "disableSummary": true,
                "disableChapters": false,
                "disableTranscript": false,
                "public": true,
            }),
        );
        let authority = AuthorityRow {
            authorized: 1,
            resolved_tenant_id: Some("tenant.organization.v1".into()),
            resolved_target_id: None,
            authority_expires_at_ms: Some(100),
            actor_revision: Some(7),
            scope_revision: Some(8),
            resource_revision: None,
            current_public: None,
            owner_id: Some("owner.user.v1".into()),
            owner_revision: Some(11),
        };
        let bindings = derive_conditional_bindings(
            profile,
            &envelope,
            &authority,
            Some("tenant.organization.v1"),
            None,
        )
        .expect("source-derived bindings");
        assert_eq!(
            bindings,
            vec![
                conditional_binding("space_password_pro", Some("actor.user.v1"), Some(7), None)
                    .expect("actor password binding"),
                conditional_binding("space_settings_pro", Some("actor.user.v1"), Some(7), None)
                    .expect("actor settings binding"),
                conditional_binding(
                    "space_publish_owner_pro",
                    Some("owner.user.v1"),
                    Some(11),
                    None,
                )
                .expect("owner publish binding"),
            ]
        );
    }

    #[test]
    fn verified_terminal_is_only_an_opaque_digest_binding() {
        let valid = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000001".into(),
            request_digest: "a".repeat(64),
            state: "verified".into(),
            provider_kind: "github_releases".into(),
            sealed_terminal_ref: Some(format!("frame-pi-terminal-v1:{}", "a".repeat(64))),
            sealed_terminal_digest: Some("0".repeat(64)),
        };
        assert!(matches!(
            project_prior(valid, &"a".repeat(64)),
            Ok(LegacyProtectedIntegrationStageOutcomeV1::VerifiedSealedTerminal { .. })
        ));
        let invalid = ReceiptRow {
            receipt_id: "00000000-0000-4000-8000-000000000002".into(),
            request_digest: "a".repeat(64),
            state: "verified".into(),
            provider_kind: "github_releases".into(),
            sealed_terminal_ref: Some("https://provider.example/token".into()),
            sealed_terminal_digest: Some("0".repeat(64)),
        };
        assert_eq!(
            project_prior(invalid, &"a".repeat(64)),
            Err(LegacyProtectedIntegrationFailureV1::Corrupt)
        );
    }
}
