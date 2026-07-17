//! Production-shaped legacy compatibility transport and D1 execution journal.
//!
//! The pinned registry promotes only a frozen semantic adapter whose exact
//! request and response are proven against the pinned Cap source. Every other
//! endpoint evidence bit, client flag, fallback, and retirement approval stays
//! disabled. Adding an operation ID alone can never make it executable.

use async_trait::async_trait;
use frame_application::{
    LegacyCallerV1, LegacyClientFamilyV1, LegacyCompatibilityRegistryV1,
    LegacyCompatibilityRequestV1, LegacyEndpointCoordinatorV1, LegacyEndpointOutcomeV1,
    LegacyExecutionCommandV1, LegacyExecutionErrorV1, LegacyExecutionOutcomeV1,
    LegacyOperationContractV1, LegacyOperationEvidenceV1, LegacyOperationExecutionPortV1,
    LegacyOperationKindV1, LegacyOperationReceiptV1, LegacyRegistryErrorV1,
    RequestSecurityContextV1,
};
use frame_domain::{
    ApiAuthClassV1, ApiErrorCodeV1, ApiErrorV1, ApiMutationEnvelopeV1, ApiRequestPolicyV1,
    ChecksumSha256, ClientCompatibilityPolicyV1, IdempotencyRequirementV1,
    LegacyEndpointDispositionV1,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, send::IntoSendFuture};

const REPORT: &str = include_str!("../../../fixtures/api-parity/v1/route-workflow-report.json");
const CLAIM_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_claim.sql");
const INTENT_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_intent.sql");
const COMPLETE_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_complete.sql");
const AUDIT_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_audit.sql");
const LOAD_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_load.sql");
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
pub const LEGACY_STATUS_OPERATION_ID: &str = "cap-v1-05b6ba3f76daac22";
pub const LEGACY_STATUS_PATH: &str = "/api/status";
const LEGACY_STATUS_SOURCE_PATH: &str = "apps/web/app/api/status/route.ts";
const LEGACY_STATUS_SOURCE_SHA256: &str =
    "ba3eb1177da489a10f74c9dbc68e0db8324b695c82499e35d6f8d9da8aaf5797";
const LEGACY_STATUS_BODY: &str = "OK";
const LEGACY_STATUS_CONTENT_TYPE: &str = "text/plain;charset=UTF-8";

// Every enabled ID must also resolve to a typed registration below, match the
// pinned report identity, and carry report success evidence. Durable adapters
// are a separate allowlist so a static read can never accidentally enter the
// D1 mutation journal.
const ENABLED_SEMANTIC_ADAPTERS: &[&str] = &[LEGACY_STATUS_OPERATION_ID];
const ENABLED_DURABLE_ADAPTERS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacySemanticAdapterV1 {
    PublicStatusOk,
}

#[derive(Debug, Clone, Copy)]
struct LegacySemanticRegistrationV1 {
    operation_id: &'static str,
    kind: &'static str,
    method: &'static str,
    legacy_path: &'static str,
    adapter: LegacySemanticAdapterV1,
}

const SEMANTIC_ADAPTERS: &[LegacySemanticRegistrationV1] = &[LegacySemanticRegistrationV1 {
    operation_id: LEGACY_STATUS_OPERATION_ID,
    kind: "route",
    method: "GET",
    legacy_path: LEGACY_STATUS_PATH,
    adapter: LegacySemanticAdapterV1::PublicStatusOk,
}];

#[derive(Deserialize)]
struct Report {
    entries: Vec<ReportRow>,
}

#[derive(Deserialize)]
struct ReportRow {
    id: String,
    kind: String,
    legacy_path: String,
    method: String,
    clients: Vec<String>,
    auth: String,
    disposition: String,
    security: ReportSecurity,
    sources: Vec<ReportSource>,
    contract_evidence: ReportEvidence,
}

#[derive(Deserialize)]
struct ReportSecurity {
    max_body_bytes: u64,
    rate_limit_bucket: String,
    idempotency: String,
}

#[derive(Deserialize)]
struct ReportSource {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
struct ReportEvidence {
    success: String,
}

#[derive(Deserialize)]
struct ExecutionRow {
    request_fingerprint: String,
    reservation_digest: String,
    state: String,
    response_status: Option<u16>,
    result_digest: Option<String>,
    intent_reservation_digest: Option<String>,
    audit_reservation_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyHttpTransportResponseV1 {
    status: u16,
    content_type: &'static str,
    body: &'static str,
}

impl LegacyHttpTransportResponseV1 {
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub const fn content_type(&self) -> &'static str {
        self.content_type
    }

    #[must_use]
    pub const fn body(&self) -> &'static str {
        self.body
    }
}

fn semantic_registration(operation_id: &str) -> Option<LegacySemanticRegistrationV1> {
    SEMANTIC_ADAPTERS
        .iter()
        .copied()
        .find(|registration| registration.operation_id == operation_id)
}

fn semantic_response(
    adapter: LegacySemanticAdapterV1,
) -> Result<LegacyHttpTransportResponseV1, LegacyExecutionErrorV1> {
    let response = match adapter {
        LegacySemanticAdapterV1::PublicStatusOk => LegacyHttpTransportResponseV1 {
            status: 200,
            content_type: LEGACY_STATUS_CONTENT_TYPE,
            body: LEGACY_STATUS_BODY,
        },
    };
    if !(200..=299).contains(&response.status)
        || response.body.len() > 64 * 1_024
        || !response.content_type.is_ascii()
    {
        return Err(LegacyExecutionErrorV1::Internal);
    }
    Ok(response)
}

fn semantic_receipt(
    adapter: LegacySemanticAdapterV1,
) -> Result<LegacyOperationReceiptV1, LegacyExecutionErrorV1> {
    let response = semantic_response(adapter)?;
    LegacyOperationReceiptV1::new(
        response.status,
        ChecksumSha256::digest_bytes(response.body.as_bytes()),
    )
    .map_err(|_| LegacyExecutionErrorV1::Internal)
}

fn static_execution_outcome(
    command: &LegacyExecutionCommandV1,
) -> Option<Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1>> {
    let registration = semantic_registration(command.operation_id())?;
    Some(match registration.adapter {
        LegacySemanticAdapterV1::PublicStatusOk => {
            semantic_receipt(registration.adapter).map(LegacyExecutionOutcomeV1::Completed)
        }
    })
}

pub struct D1LegacyOperationExecutionPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyOperationExecutionPortV1<'database> {
    #[must_use]
    pub const fn new_fail_closed(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyExecutionErrorV1::Internal)
    }

    async fn load(
        &self,
        scope_digest: &str,
        operation_id: &str,
        idempotency_key_digest: &str,
    ) -> Result<Option<ExecutionRow>, LegacyExecutionErrorV1> {
        self.statement(
            LOAD_SQL,
            &[
                JsValue::from_str(scope_digest),
                JsValue::from_str(operation_id),
                JsValue::from_str(idempotency_key_digest),
            ],
        )?
        .first::<ExecutionRow>(None)
        .into_send()
        .await
        .map_err(|_| LegacyExecutionErrorV1::Internal)
    }
}

#[async_trait]
impl LegacyOperationExecutionPortV1 for D1LegacyOperationExecutionPortV1<'_> {
    async fn execute_once(
        &self,
        command: &LegacyExecutionCommandV1,
    ) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
        if !ENABLED_DURABLE_ADAPTERS.contains(&command.operation_id()) {
            return Err(LegacyExecutionErrorV1::Unsupported);
        }

        let now_ms = current_time_ms()?;
        let idempotency_key_digest = digest_parts(
            "legacy-idempotency-v1",
            &[
                command.scope_digest().as_str(),
                command.operation_id(),
                command.idempotency_key().map_or(
                    command.correlation_id(),
                    frame_domain::IdempotencyKey::expose,
                ),
            ],
        );
        let reservation_nonce = Uuid::now_v7().to_string();
        let reservation_digest = digest_parts(
            "legacy-reservation-v1",
            &[reservation_nonce.as_str(), command.operation_id()],
        );
        let result_digest = digest_parts(
            "legacy-accepted-result-v1",
            &[
                command.scope_digest().as_str(),
                command.operation_id(),
                idempotency_key_digest.as_str(),
                command.request_fingerprint().as_str(),
            ],
        );
        let audit_id = digest_parts(
            "legacy-audit-v1",
            &[reservation_digest.as_str(), result_digest.as_str()],
        );
        let correlation_digest = digest_parts("legacy-correlation-v1", &[command.correlation_id()]);
        let values = BoundExecutionValues {
            scope_digest: command.scope_digest().as_str(),
            operation_id: command.operation_id(),
            idempotency_key_digest: &idempotency_key_digest,
            request_fingerprint: command.request_fingerprint().as_str(),
            reservation_digest: &reservation_digest,
            response_status: 202,
            result_digest: &result_digest,
            now_ms,
            audit_id: &audit_id,
            audit_action: command.audit_action(),
            correlation_digest: &correlation_digest,
        };
        let statements = vec![
            self.claim_statement(&values)?,
            self.intent_statement(&values)?,
            self.complete_statement(&values)?,
            self.audit_statement(&values)?,
        ];
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyExecutionErrorV1::Internal)?;
        if results.len() != 4 || results.iter().any(|result| !result.success()) {
            return Err(LegacyExecutionErrorV1::Internal);
        }

        let row = self
            .load(
                command.scope_digest().as_str(),
                command.operation_id(),
                &idempotency_key_digest,
            )
            .await?
            .ok_or(LegacyExecutionErrorV1::Internal)?;
        execution_outcome(
            row,
            command.request_fingerprint().as_str(),
            &reservation_digest,
        )
    }
}

struct BoundExecutionValues<'value> {
    scope_digest: &'value str,
    operation_id: &'value str,
    idempotency_key_digest: &'value str,
    request_fingerprint: &'value str,
    reservation_digest: &'value str,
    response_status: u16,
    result_digest: &'value str,
    now_ms: i64,
    audit_id: &'value str,
    audit_action: &'value str,
    correlation_digest: &'value str,
}

impl D1LegacyOperationExecutionPortV1<'_> {
    fn claim_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            CLAIM_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_f64(values.now_ms as f64),
            ],
        )
    }

    fn intent_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            INTENT_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_f64(values.now_ms as f64),
            ],
        )
    }

    fn complete_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            COMPLETE_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_f64(f64::from(values.response_status)),
                JsValue::from_str(values.result_digest),
                JsValue::from_f64(values.now_ms as f64),
            ],
        )
    }

    fn audit_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            AUDIT_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_f64(f64::from(values.response_status)),
                JsValue::from_str(values.result_digest),
                JsValue::from_f64(values.now_ms as f64),
                JsValue::from_str(values.audit_id),
                JsValue::from_str(values.audit_action),
                JsValue::from_str(values.correlation_digest),
            ],
        )
    }
}

struct LegacyRuntimeExecutionPortV1<'database> {
    durable: Option<D1LegacyOperationExecutionPortV1<'database>>,
}

#[async_trait]
impl LegacyOperationExecutionPortV1 for LegacyRuntimeExecutionPortV1<'_> {
    async fn execute_once(
        &self,
        command: &LegacyExecutionCommandV1,
    ) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
        if let Some(outcome) = static_execution_outcome(command) {
            return outcome;
        }
        let durable = self
            .durable
            .as_ref()
            .ok_or(LegacyExecutionErrorV1::Unsupported)?;
        durable.execute_once(command).await
    }
}

pub struct LegacyCompatibilityTransportV1<'database> {
    coordinator: LegacyEndpointCoordinatorV1<LegacyRuntimeExecutionPortV1<'database>>,
}

impl LegacyCompatibilityTransportV1<'static> {
    /// Construct the exact static compatibility surface without introducing a
    /// database dependency into a legacy operation that never had one.
    pub fn new_static_only(
        compatibility: ClientCompatibilityPolicyV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let registry = fail_closed_registry(compatibility)?;
        Ok(Self {
            coordinator: LegacyEndpointCoordinatorV1::new(
                registry,
                LegacyRuntimeExecutionPortV1 { durable: None },
            ),
        })
    }
}

impl<'database> LegacyCompatibilityTransportV1<'database> {
    pub fn new_fail_closed(
        database: &'database D1Database,
        compatibility: ClientCompatibilityPolicyV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let registry = fail_closed_registry(compatibility)?;
        Ok(Self {
            coordinator: LegacyEndpointCoordinatorV1::new(
                registry,
                LegacyRuntimeExecutionPortV1 {
                    durable: Some(D1LegacyOperationExecutionPortV1::new_fail_closed(database)),
                },
            ),
        })
    }

    pub async fn dispatch_http(
        &self,
        input: LegacyHttpTransportRequestV1,
    ) -> Result<LegacyEndpointOutcomeV1, ApiErrorV1> {
        let operation_id = self
            .coordinator
            .registry()
            .resolve_http(&input.method, &input.raw_path)
            .map(|contract| contract.id().to_owned())
            .ok_or_else(|| {
                public_error(ApiErrorCodeV1::NotFound, &input.envelope.correlation_id)
            })?;
        self.coordinator
            .execute(
                &LegacyCompatibilityRequestV1 {
                    operation_id,
                    caller: input.caller,
                    envelope: input.envelope,
                    security: input.security,
                },
                input.scope_digest,
                input.request_fingerprint,
            )
            .await
    }

    pub async fn dispatch_http_response(
        &self,
        input: LegacyHttpTransportRequestV1,
    ) -> Result<LegacyHttpTransportResponseV1, ApiErrorV1> {
        let correlation_id = input.envelope.correlation_id.clone();
        let operation_id = self
            .coordinator
            .registry()
            .resolve_http(&input.method, &input.raw_path)
            .map(|contract| contract.id().to_owned())
            .ok_or_else(|| public_error(ApiErrorCodeV1::NotFound, &correlation_id))?;
        let registration = semantic_registration(&operation_id)
            .ok_or_else(|| public_error(ApiErrorCodeV1::Unsupported, &correlation_id))?;
        let outcome = self
            .coordinator
            .execute(
                &LegacyCompatibilityRequestV1 {
                    operation_id,
                    caller: input.caller,
                    envelope: input.envelope,
                    security: input.security,
                },
                input.scope_digest,
                input.request_fingerprint,
            )
            .await?;
        let receipt = match outcome {
            LegacyEndpointOutcomeV1::Completed(receipt)
            | LegacyEndpointOutcomeV1::Replay(receipt) => receipt,
            LegacyEndpointOutcomeV1::UseLegacyFallback(_)
            | LegacyEndpointOutcomeV1::RetirementResponse(_) => {
                return Err(public_error(
                    ApiErrorCodeV1::TemporarilyUnavailable,
                    &correlation_id,
                ));
            }
        };
        let response = semantic_response(registration.adapter)
            .map_err(|_| public_error(ApiErrorCodeV1::Internal, &correlation_id))?;
        if receipt.status != response.status
            || receipt.result_digest != ChecksumSha256::digest_bytes(response.body.as_bytes())
        {
            return Err(public_error(ApiErrorCodeV1::Internal, &correlation_id));
        }
        Ok(response)
    }
}

pub struct LegacyHttpTransportRequestV1 {
    pub method: String,
    pub raw_path: String,
    pub caller: LegacyCallerV1,
    pub envelope: ApiMutationEnvelopeV1,
    pub security: RequestSecurityContextV1,
    pub scope_digest: ChecksumSha256,
    pub request_fingerprint: ChecksumSha256,
}

fn fail_closed_registry(
    compatibility: ClientCompatibilityPolicyV1,
) -> Result<LegacyCompatibilityRegistryV1, LegacyRegistryErrorV1> {
    let report: Report =
        serde_json::from_str(REPORT).map_err(|_| LegacyRegistryErrorV1::InvalidContract)?;
    let contracts = report
        .entries
        .into_iter()
        .map(evidence_gated_contract)
        .collect::<Result<Vec<_>, _>>()?;
    LegacyCompatibilityRegistryV1::new(contracts, compatibility)
}

fn evidence_gated_contract(
    row: ReportRow,
) -> Result<LegacyOperationContractV1, LegacyRegistryErrorV1> {
    let registration = semantic_registration(&row.id);
    let enabled = ENABLED_SEMANTIC_ADAPTERS.contains(&row.id.as_str());
    if enabled != registration.is_some() {
        return Err(LegacyRegistryErrorV1::InvalidContract);
    }
    let endpoint_promoted = if let Some(registration) = registration {
        let source_matches = row.sources.iter().any(|source| {
            source.path == LEGACY_STATUS_SOURCE_PATH && source.sha256 == LEGACY_STATUS_SOURCE_SHA256
        });
        if registration.kind != row.kind
            || registration.method != row.method
            || registration.legacy_path != row.legacy_path
            || row.clients != ["web"]
            || row.auth != "public_or_flow_token"
            || row.disposition != "replace"
            || row.security.max_body_bytes != 0
            || row.security.rate_limit_bucket != "service_misc.v1"
            || row.security.idempotency != "forbidden"
            || row.contract_evidence.success != "local_contract"
            || !source_matches
        {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        true
    } else {
        if row.contract_evidence.success == "local_contract" {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        false
    };
    let kind = match row.kind.as_str() {
        "route" => LegacyOperationKindV1::HttpRoute,
        "rpc" => LegacyOperationKindV1::Rpc,
        "server_action" => LegacyOperationKindV1::ServerAction,
        "workflow" => LegacyOperationKindV1::Workflow,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let auth = match row.auth.as_str() {
        "public_or_flow_token" => ApiAuthClassV1::Public,
        "optional_session_or_share_capability" => ApiAuthClassV1::OptionalSession,
        "session" => ApiAuthClassV1::Session,
        "developer_api_key" => ApiAuthClassV1::ApiKey,
        "internal_service" => ApiAuthClassV1::Worker,
        "signed_webhook" => ApiAuthClassV1::Webhook,
        "scheduler_secret" => ApiAuthClassV1::Scheduler,
        "admin_session" => ApiAuthClassV1::Admin,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let idempotency = match row.security.idempotency.as_str() {
        "required" => IdempotencyRequirementV1::Required,
        "optional" => IdempotencyRequirementV1::Optional,
        "forbidden" => IdempotencyRequirementV1::Forbidden,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let disposition = match row.disposition.as_str() {
        "replace" | "protected_parity_required" => LegacyEndpointDispositionV1::Replace,
        "migrate" => LegacyEndpointDispositionV1::Migrate,
        "retire" => LegacyEndpointDispositionV1::Retire,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let clients = row
        .clients
        .into_iter()
        .map(|client| match client.as_str() {
            "web" => Ok(LegacyClientFamilyV1::Web),
            "desktop" => Ok(LegacyClientFamilyV1::Desktop),
            "mobile" => Ok(LegacyClientFamilyV1::Mobile),
            "extension" => Ok(LegacyClientFamilyV1::Extension),
            "developer" => Ok(LegacyClientFamilyV1::Developer),
            "internal_worker" => Ok(LegacyClientFamilyV1::InternalWorker),
            "provider" => Ok(LegacyClientFamilyV1::Provider),
            "scheduler" => Ok(LegacyClientFamilyV1::Scheduler),
            _ => Err(LegacyRegistryErrorV1::InvalidContract),
        })
        .collect::<Result<Vec<_>, _>>()?;
    LegacyOperationContractV1::new(
        row.id.clone(),
        kind,
        row.method,
        row.legacy_path,
        clients,
        ApiRequestPolicyV1 {
            auth,
            max_body_bytes: row.security.max_body_bytes,
            accepted_content_types: if row.security.max_body_bytes > 0 {
                vec!["application/json".into()]
            } else {
                Vec::new()
            },
            idempotency,
            rate_limit_bucket: row.security.rate_limit_bucket,
            audit_action: format!("legacy.{}", row.id),
        },
        disposition,
        LegacyOperationEvidenceV1 {
            endpoint_contract_proven: endpoint_promoted,
            client_family_enabled: endpoint_promoted,
            legacy_fallback_available: false,
            retirement_approved: false,
        },
    )
}

fn execution_outcome(
    row: ExecutionRow,
    expected_request_fingerprint: &str,
    reservation_digest: &str,
) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
    if row.request_fingerprint != expected_request_fingerprint {
        return Err(LegacyExecutionErrorV1::Conflict);
    }
    if row.state == "pending" {
        return Ok(LegacyExecutionOutcomeV1::InFlight);
    }
    let (Some(status), Some(result_digest)) = (row.response_status, row.result_digest) else {
        return Err(LegacyExecutionErrorV1::Internal);
    };
    if row.state != "complete"
        || row.intent_reservation_digest.as_deref() != Some(row.reservation_digest.as_str())
        || row.audit_reservation_digest.as_deref() != Some(row.reservation_digest.as_str())
    {
        return Err(LegacyExecutionErrorV1::Internal);
    }
    let receipt = LegacyOperationReceiptV1::new(
        status,
        ChecksumSha256::parse(result_digest).map_err(|_| LegacyExecutionErrorV1::Internal)?,
    )
    .map_err(|_| LegacyExecutionErrorV1::Internal)?;
    if row.reservation_digest == reservation_digest {
        Ok(LegacyExecutionOutcomeV1::Completed(receipt))
    } else {
        Ok(LegacyExecutionOutcomeV1::Replay(receipt))
    }
}

fn current_time_ms() -> Result<i64, LegacyExecutionErrorV1> {
    let value = js_sys::Date::now();
    if !value.is_finite() || value < 0.0 || value > MAX_SAFE_INTEGER as f64 {
        return Err(LegacyExecutionErrorV1::Internal);
    }
    Ok(value as i64)
}

fn digest_parts(domain: &str, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    for part in parts {
        digest.update([0]);
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn public_error(code: ApiErrorCodeV1, correlation_id: &str) -> ApiErrorV1 {
    ApiErrorV1::new(code, correlation_id, None).unwrap_or_else(|_| {
        ApiErrorV1::new(code, "invalid-correlation", None).expect("fixed correlation ID is valid")
    })
}

#[cfg(test)]
mod tests {
    use frame_application::RateLimitDecisionV1;
    use frame_domain::{
        ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1, IdempotencyKey,
    };
    use futures::executor::block_on;

    use super::*;

    fn compatibility() -> ClientCompatibilityPolicyV1 {
        ClientCompatibilityPolicyV1 {
            api_major: 1,
            current_release: 42,
            previous_release: 41,
            deprecated_after_ms: Some(1_900_000_000_000),
            retired: false,
        }
    }

    #[test]
    fn production_registry_promotes_only_the_pinned_status_contract() {
        let registry = fail_closed_registry(compatibility()).expect("registry");
        assert_eq!(registry.len(), 288);
        let report = serde_json::from_str::<Report>(REPORT).expect("report");
        let mut promoted = 0;
        for contract in &report.entries {
            let stored = registry.contract(&contract.id).expect("contract");
            let expected = contract.id == LEGACY_STATUS_OPERATION_ID;
            assert_eq!(stored.evidence().endpoint_contract_proven, expected);
            assert_eq!(stored.evidence().client_family_enabled, expected);
            assert!(!stored.evidence().legacy_fallback_available);
            assert!(!stored.evidence().retirement_approved);
            promoted += usize::from(expected);
        }
        assert_eq!(promoted, 1);
        assert_eq!(ENABLED_SEMANTIC_ADAPTERS, [LEGACY_STATUS_OPERATION_ID]);
        assert!(ENABLED_DURABLE_ADAPTERS.is_empty());

        let contract = registry
            .contract(
                &report
                    .entries
                    .iter()
                    .find(|row| row.id != LEGACY_STATUS_OPERATION_ID)
                    .expect("unpromoted contract")
                    .id,
            )
            .expect("unpromoted contract");
        let caller = match contract.clients()[0] {
            LegacyClientFamilyV1::Web => released_caller(ClientSurfaceV1::Web),
            LegacyClientFamilyV1::Desktop => released_caller(ClientSurfaceV1::Desktop),
            LegacyClientFamilyV1::Mobile => released_caller(ClientSurfaceV1::Mobile),
            LegacyClientFamilyV1::Extension => released_caller(ClientSurfaceV1::Extension),
            LegacyClientFamilyV1::Developer => released_caller(ClientSurfaceV1::Developer),
            LegacyClientFamilyV1::InternalWorker => LegacyCallerV1::InternalWorker,
            LegacyClientFamilyV1::Provider => LegacyCallerV1::Provider,
            LegacyClientFamilyV1::Scheduler => LegacyCallerV1::Scheduler,
        };
        let error = registry
            .admit(&request_for(contract, caller))
            .expect_err("unverified fallback must fail closed");
        assert_eq!(error.code, ApiErrorCodeV1::TemporarilyUnavailable);

        let status = registry
            .contract(LEGACY_STATUS_OPERATION_ID)
            .expect("status contract");
        assert_eq!(status.method(), "GET");
        assert_eq!(status.legacy_identity(), LEGACY_STATUS_PATH);
        assert!(matches!(
            registry.admit(&request_for(status, released_caller(ClientSurfaceV1::Web))),
            Ok(frame_application::LegacyCompatibilityOutcomeV1::ServeFrame(
                _
            ))
        ));
        let registration = semantic_registration(LEGACY_STATUS_OPERATION_ID).expect("adapter");
        let response = semantic_response(registration.adapter).expect("response");
        assert_eq!(response.status(), 200);
        assert_eq!(response.content_type(), "text/plain;charset=UTF-8");
        assert_eq!(response.body(), "OK");
        assert_eq!(
            semantic_receipt(registration.adapter)
                .expect("receipt")
                .result_digest,
            ChecksumSha256::digest_bytes(b"OK")
        );
    }

    fn released_caller(surface: ClientSurfaceV1) -> LegacyCallerV1 {
        LegacyCallerV1::Released(ClientReleaseV1 {
            surface,
            api_major: 1,
            release: 42,
        })
    }

    fn request_for(
        contract: &LegacyOperationContractV1,
        caller: LegacyCallerV1,
    ) -> LegacyCompatibilityRequestV1 {
        LegacyCompatibilityRequestV1 {
            operation_id: contract.id().to_owned(),
            caller,
            envelope: ApiMutationEnvelopeV1 {
                content_length: 0,
                content_type: None,
                idempotency_key: None,
                correlation_id: "legacy-runtime-test".into(),
            },
            security: RequestSecurityContextV1 {
                authenticated: true,
                authorized: true,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit: RateLimitDecisionV1::Allowed,
            },
        }
    }

    fn status_transport_request(
        method: &str,
        raw_path: &str,
        content_length: u64,
        idempotency_key: Option<IdempotencyKey>,
    ) -> LegacyHttpTransportRequestV1 {
        LegacyHttpTransportRequestV1 {
            method: method.into(),
            raw_path: raw_path.into(),
            caller: released_caller(ClientSurfaceV1::Web),
            envelope: ApiMutationEnvelopeV1 {
                content_length,
                content_type: None,
                idempotency_key,
                correlation_id: "legacy-status-transport".into(),
            },
            security: RequestSecurityContextV1 {
                authenticated: false,
                authorized: true,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit: RateLimitDecisionV1::Allowed,
            },
            scope_digest: ChecksumSha256::digest_bytes(b"legacy-status-scope"),
            request_fingerprint: ChecksumSha256::digest_bytes(b"GET\0/api/status"),
        }
    }

    #[test]
    fn static_transport_serves_only_the_exact_status_request_and_response() {
        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("static transport");
        let response = block_on(transport.dispatch_http_response(status_transport_request(
            "GET",
            LEGACY_STATUS_PATH,
            0,
            None,
        )))
        .expect("exact status response");
        assert_eq!(response.status(), 200);
        assert_eq!(response.content_type(), "text/plain;charset=UTF-8");
        assert_eq!(response.body(), "OK");

        let oversized = block_on(transport.dispatch_http_response(status_transport_request(
            "GET",
            LEGACY_STATUS_PATH,
            1,
            None,
        )))
        .expect_err("status body must remain empty");
        assert_eq!(oversized.code, ApiErrorCodeV1::InvalidRequest);

        let keyed = block_on(transport.dispatch_http_response(status_transport_request(
            "GET",
            LEGACY_STATUS_PATH,
            0,
            Some(IdempotencyKey::parse("status-must-not-have-a-key").expect("key")),
        )))
        .expect_err("status idempotency key must remain forbidden");
        assert_eq!(keyed.code, ApiErrorCodeV1::InvalidRequest);

        for (method, raw_path) in [("POST", LEGACY_STATUS_PATH), ("GET", "/api/status/")] {
            let unknown = block_on(
                transport
                    .dispatch_http_response(status_transport_request(method, raw_path, 0, None)),
            )
            .expect_err("non-exact identity must fail closed");
            assert_eq!(unknown.code, ApiErrorCodeV1::NotFound);
        }
    }

    #[test]
    fn d1_queries_bind_every_transition_to_the_winning_reservation() {
        assert!(CLAIM_SQL.starts_with("INSERT OR IGNORE"));
        assert!(CLAIM_SQL.contains("RETURNING reservation_digest"));
        for query in [INTENT_SQL, COMPLETE_SQL, AUDIT_SQL] {
            assert!(query.contains("reservation_digest = ?4"));
            assert!(query.contains("request_fingerprint = ?5"));
        }
        assert!(COMPLETE_SQL.contains("state = 'pending'"));
        assert!(AUDIT_SQL.contains("state = 'complete'"));
        assert!(LOAD_SQL.contains("LEFT JOIN legacy_api_execution_intents_v1"));
        assert!(LOAD_SQL.contains("LEFT JOIN legacy_api_execution_audit_v1"));
    }

    #[test]
    fn operation_outcome_rejects_conflict_partial_commit_and_corruption() {
        let complete = ExecutionRow {
            request_fingerprint: "02".repeat(32),
            reservation_digest: "03".repeat(32),
            state: "complete".into(),
            response_status: Some(202),
            result_digest: Some("04".repeat(32)),
            intent_reservation_digest: Some("03".repeat(32)),
            audit_reservation_digest: Some("03".repeat(32)),
        };
        assert!(matches!(
            execution_outcome(complete, &"02".repeat(32), &"03".repeat(32)),
            Ok(LegacyExecutionOutcomeV1::Completed(_))
        ));

        let partial = ExecutionRow {
            request_fingerprint: "02".repeat(32),
            reservation_digest: "03".repeat(32),
            state: "complete".into(),
            response_status: Some(202),
            result_digest: Some("04".repeat(32)),
            intent_reservation_digest: None,
            audit_reservation_digest: Some("03".repeat(32)),
        };
        assert_eq!(
            execution_outcome(partial, &"02".repeat(32), &"03".repeat(32)),
            Err(LegacyExecutionErrorV1::Internal)
        );

        let conflict = ExecutionRow {
            request_fingerprint: "ff".repeat(32),
            reservation_digest: "03".repeat(32),
            state: "pending".into(),
            response_status: None,
            result_digest: None,
            intent_reservation_digest: None,
            audit_reservation_digest: None,
        };
        assert_eq!(
            execution_outcome(conflict, &"02".repeat(32), &"03".repeat(32)),
            Err(LegacyExecutionErrorV1::Conflict)
        );
    }
}
