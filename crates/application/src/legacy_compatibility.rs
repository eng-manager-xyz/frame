//! Evidence-gated admission for the pinned legacy operation registry.
//!
//! This module deliberately stops at the authority boundary. Registering an
//! operation proves that every transport uses the same compatibility,
//! validation, authentication, rate-limit, idempotency-header, error, trace,
//! and audit-label policy. It does not turn a family-level Rust service into a
//! successful endpoint adapter; callers may reach Frame only when the
//! operation's endpoint evidence and client-family flag are both enabled.

use std::{collections::BTreeMap, fmt};

use async_trait::async_trait;
use frame_domain::{
    ApiErrorCodeV1, ApiErrorV1, ApiMutationEnvelopeV1, ApiRequestPolicyV1, ChecksumSha256,
    ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1, CompatibilityDecisionV1,
    LegacyEndpointDispositionV1, LegacyRouteDecisionV1, LegacyRoutePolicyV1,
};
use thiserror::Error;

use crate::{ApiAdmissionV1, ApiGatewayV1, RequestSecurityContextV1};

const MAX_LEGACY_IDENTITY_BYTES: usize = 768;

pub const LEGACY_RETIREMENT_SCHEMA_V1: &str = "frame.legacy-retirement.v1";
pub const LEGACY_RETIREMENT_CODE_V1: &str = "legacy_operation_retired";
pub const LEGACY_RETIREMENT_MESSAGE_V1: &str = "This legacy operation has been retired.";
pub const LEGACY_RETIREMENT_MIGRATION_V1: &str = "privacy-safe export";
pub const LEGACY_RETIREMENT_CACHE_CONTROL_V1: &str = "no-store, max-age=0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyOperationKindV1 {
    HttpRoute,
    Rpc,
    ServerAction,
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyClientFamilyV1 {
    Web,
    Desktop,
    Mobile,
    Extension,
    Developer,
    InternalWorker,
    Provider,
    Scheduler,
}

impl From<ClientSurfaceV1> for LegacyClientFamilyV1 {
    fn from(value: ClientSurfaceV1) -> Self {
        match value {
            ClientSurfaceV1::Web => Self::Web,
            ClientSurfaceV1::Desktop => Self::Desktop,
            ClientSurfaceV1::Mobile => Self::Mobile,
            ClientSurfaceV1::Extension => Self::Extension,
            ClientSurfaceV1::Developer => Self::Developer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyCallerV1 {
    Released(ClientReleaseV1),
    InternalWorker,
    Provider,
    Scheduler,
}

impl LegacyCallerV1 {
    #[must_use]
    pub const fn family(&self) -> LegacyClientFamilyV1 {
        match self {
            Self::Released(release) => match release.surface {
                ClientSurfaceV1::Web => LegacyClientFamilyV1::Web,
                ClientSurfaceV1::Desktop => LegacyClientFamilyV1::Desktop,
                ClientSurfaceV1::Mobile => LegacyClientFamilyV1::Mobile,
                ClientSurfaceV1::Extension => LegacyClientFamilyV1::Extension,
                ClientSurfaceV1::Developer => LegacyClientFamilyV1::Developer,
            },
            Self::InternalWorker => LegacyClientFamilyV1::InternalWorker,
            Self::Provider => LegacyClientFamilyV1::Provider,
            Self::Scheduler => LegacyClientFamilyV1::Scheduler,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOperationEvidenceV1 {
    pub endpoint_contract_proven: bool,
    pub client_family_enabled: bool,
    pub legacy_fallback_available: bool,
    pub retirement_approved: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyOperationContractV1 {
    id: String,
    kind: LegacyOperationKindV1,
    method: String,
    legacy_identity: String,
    clients: Vec<LegacyClientFamilyV1>,
    request_policy: ApiRequestPolicyV1,
    disposition: LegacyEndpointDispositionV1,
    evidence: LegacyOperationEvidenceV1,
}

impl LegacyOperationContractV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        kind: LegacyOperationKindV1,
        method: impl Into<String>,
        legacy_identity: impl Into<String>,
        clients: Vec<LegacyClientFamilyV1>,
        request_policy: ApiRequestPolicyV1,
        disposition: LegacyEndpointDispositionV1,
        evidence: LegacyOperationEvidenceV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let id = id.into();
        let method = method.into();
        let legacy_identity = legacy_identity.into();
        if !valid_operation_id(&id)
            || !valid_method(kind, &method)
            || legacy_identity.is_empty()
            || legacy_identity.len() > MAX_LEGACY_IDENTITY_BYTES
            || legacy_identity.chars().any(char::is_control)
            || clients.is_empty()
            || clients
                .iter()
                .enumerate()
                .any(|(index, client)| clients[index + 1..].contains(client))
            || request_policy.validate().is_err()
            || (disposition == LegacyEndpointDispositionV1::Retire
                && evidence.endpoint_contract_proven)
            || (disposition != LegacyEndpointDispositionV1::Retire && evidence.retirement_approved)
        {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        Ok(Self {
            id,
            kind,
            method,
            legacy_identity,
            clients,
            request_policy,
            disposition,
            evidence,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> LegacyOperationKindV1 {
        self.kind
    }

    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    #[must_use]
    pub fn legacy_identity(&self) -> &str {
        &self.legacy_identity
    }

    #[must_use]
    pub fn clients(&self) -> &[LegacyClientFamilyV1] {
        &self.clients
    }

    #[must_use]
    pub const fn evidence(&self) -> LegacyOperationEvidenceV1 {
        self.evidence
    }

    fn route_policy(&self) -> LegacyRoutePolicyV1 {
        LegacyRoutePolicyV1 {
            disposition: self.disposition,
            endpoint_contract_proven: self.evidence.endpoint_contract_proven,
            client_family_enabled: self.evidence.client_family_enabled,
            legacy_fallback_available: self.evidence.legacy_fallback_available,
            retirement_approved: self.evidence.retirement_approved,
        }
    }
}

impl fmt::Debug for LegacyOperationContractV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyOperationContractV1")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("method", &self.method)
            .field("legacy_identity", &self.legacy_identity)
            .field("clients", &self.clients)
            .field("request_policy", &"<policy>")
            .field("disposition", &self.disposition)
            .field("evidence", &self.evidence)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyCompatibilityRequestV1 {
    pub operation_id: String,
    pub caller: LegacyCallerV1,
    pub envelope: ApiMutationEnvelopeV1,
    pub security: RequestSecurityContextV1,
}

impl fmt::Debug for LegacyCompatibilityRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCompatibilityRequestV1")
            .field("operation_id", &self.operation_id)
            .field("caller", &self.caller)
            .field("envelope", &self.envelope)
            .field("security", &"<security-context>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyFrameAdmissionV1 {
    pub operation_id: String,
    pub client_family: LegacyClientFamilyV1,
    pub admission: ApiAdmissionV1,
}

impl fmt::Debug for LegacyFrameAdmissionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyFrameAdmissionV1")
            .field("operation_id", &self.operation_id)
            .field("client_family", &self.client_family)
            .field("admission", &self.admission)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyFallbackV1 {
    pub operation_id: String,
    pub client_family: LegacyClientFamilyV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRetirementV1 {
    pub operation_id: String,
    pub client_family: LegacyClientFamilyV1,
    pub schema_version: &'static str,
    pub http_status: u16,
    pub code: &'static str,
    pub message: &'static str,
    pub migration_path: &'static str,
    pub cache_control: &'static str,
    pub retryable: bool,
}

impl LegacyRetirementV1 {
    #[must_use]
    pub fn deterministic(
        operation_id: impl Into<String>,
        client_family: LegacyClientFamilyV1,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            client_family,
            schema_version: LEGACY_RETIREMENT_SCHEMA_V1,
            http_status: 410,
            code: LEGACY_RETIREMENT_CODE_V1,
            message: LEGACY_RETIREMENT_MESSAGE_V1,
            migration_path: LEGACY_RETIREMENT_MIGRATION_V1,
            cache_control: LEGACY_RETIREMENT_CACHE_CONTROL_V1,
            retryable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyCompatibilityOutcomeV1 {
    ServeFrame(LegacyFrameAdmissionV1),
    UseLegacyFallback(LegacyFallbackV1),
    RetirementResponse(LegacyRetirementV1),
}

pub struct LegacyCompatibilityRegistryV1 {
    contracts: BTreeMap<String, LegacyOperationContractV1>,
    identities: BTreeMap<(LegacyOperationKindV1, String, String), String>,
    compatibility: ClientCompatibilityPolicyV1,
}

impl LegacyCompatibilityRegistryV1 {
    pub fn new(
        contracts: Vec<LegacyOperationContractV1>,
        compatibility: ClientCompatibilityPolicyV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        if contracts.is_empty() || compatibility.validate().is_err() {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        let mut identities = BTreeMap::new();
        let mut indexed = BTreeMap::new();
        for contract in contracts {
            let identity = (
                contract.kind,
                contract.method.clone(),
                contract.legacy_identity.clone(),
            );
            if identities.insert(identity, contract.id.clone()).is_some()
                || indexed.insert(contract.id.clone(), contract).is_some()
            {
                return Err(LegacyRegistryErrorV1::DuplicateContract);
            }
        }
        Ok(Self {
            contracts: indexed,
            identities,
            compatibility,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.contracts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }

    #[must_use]
    pub fn contract(&self, id: &str) -> Option<&LegacyOperationContractV1> {
        self.contracts.get(id)
    }

    #[must_use]
    pub fn resolve_exact(
        &self,
        kind: LegacyOperationKindV1,
        method: &str,
        legacy_identity: &str,
    ) -> Option<&LegacyOperationContractV1> {
        self.identities
            .get(&(kind, method.to_owned(), legacy_identity.to_owned()))
            .and_then(|id| self.contracts.get(id))
    }

    /// Resolve one raw legacy HTTP path without URL decoding. Dynamic `:name`
    /// segments match exactly one safe segment; the sole `:name*` form matches
    /// one or more safe segments. Ambiguous, encoded, dot, empty, or hostile
    /// paths never match.
    #[must_use]
    pub fn resolve_http(&self, method: &str, raw_path: &str) -> Option<&LegacyOperationContractV1> {
        if !valid_raw_legacy_path(raw_path) {
            return None;
        }
        let mut best = None;
        let mut ambiguous = false;
        for contract in self.contracts.values().filter(|contract| {
            contract.kind == LegacyOperationKindV1::HttpRoute
                && contract.method == method
                && legacy_path_matches(&contract.legacy_identity, raw_path)
        }) {
            let specificity = legacy_pattern_specificity(&contract.legacy_identity);
            match best {
                None => best = Some((specificity, contract)),
                Some((current, _)) if specificity > current => {
                    best = Some((specificity, contract));
                    ambiguous = false;
                }
                Some((current, _)) if specificity == current => ambiguous = true,
                Some(_) => {}
            }
        }
        if ambiguous {
            None
        } else {
            best.map(|(_, contract)| contract)
        }
    }

    pub fn admit(
        &self,
        request: &LegacyCompatibilityRequestV1,
    ) -> Result<LegacyCompatibilityOutcomeV1, ApiErrorV1> {
        let error = |code| public_registry_error(code, &request.envelope.correlation_id);
        let contract = self
            .contracts
            .get(&request.operation_id)
            .ok_or_else(|| error(ApiErrorCodeV1::NotFound))?;
        let family = request.caller.family();
        if !contract.clients.contains(&family) {
            return Err(error(ApiErrorCodeV1::NotFound));
        }
        let compatibility = match &request.caller {
            LegacyCallerV1::Released(release) => self
                .compatibility
                .decide(release)
                .map_err(|_| error(ApiErrorCodeV1::Internal))?,
            LegacyCallerV1::InternalWorker
            | LegacyCallerV1::Provider
            | LegacyCallerV1::Scheduler => CompatibilityDecisionV1::Current,
        };
        let decision = contract
            .route_policy()
            .decide(compatibility)
            .map_err(|_| error(ApiErrorCodeV1::TemporarilyUnavailable))?;
        match decision {
            LegacyRouteDecisionV1::RejectUpgradeRequired => {
                Err(error(ApiErrorCodeV1::UpgradeRequired))
            }
            LegacyRouteDecisionV1::UseLegacyFallback => Ok(
                LegacyCompatibilityOutcomeV1::UseLegacyFallback(LegacyFallbackV1 {
                    operation_id: contract.id.clone(),
                    client_family: family,
                }),
            ),
            LegacyRouteDecisionV1::RetirementResponse => {
                Ok(LegacyCompatibilityOutcomeV1::RetirementResponse(
                    LegacyRetirementV1::deterministic(contract.id.clone(), family),
                ))
            }
            LegacyRouteDecisionV1::ServeFrameV1 => {
                let admission = ApiGatewayV1::admit_mutation(
                    &contract.request_policy,
                    &request.envelope,
                    request.security,
                )?;
                Ok(LegacyCompatibilityOutcomeV1::ServeFrame(
                    LegacyFrameAdmissionV1 {
                        operation_id: contract.id.clone(),
                        client_family: family,
                        admission,
                    },
                ))
            }
        }
    }
}

impl fmt::Debug for LegacyCompatibilityRegistryV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyCompatibilityRegistryV1")
            .field("contract_count", &self.contracts.len())
            .field("identity_count", &self.identities.len())
            .field("compatibility", &self.compatibility)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyExecutionCommandV1 {
    operation_id: String,
    client_family: LegacyClientFamilyV1,
    idempotency_key: Option<frame_domain::IdempotencyKey>,
    scope_digest: ChecksumSha256,
    request_fingerprint: ChecksumSha256,
    audit_action: String,
    rate_limit_bucket: String,
    correlation_id: String,
}

impl LegacyExecutionCommandV1 {
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[must_use]
    pub const fn client_family(&self) -> LegacyClientFamilyV1 {
        self.client_family
    }

    #[must_use]
    pub fn idempotency_key(&self) -> Option<&frame_domain::IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    #[must_use]
    pub const fn scope_digest(&self) -> &ChecksumSha256 {
        &self.scope_digest
    }

    #[must_use]
    pub const fn request_fingerprint(&self) -> &ChecksumSha256 {
        &self.request_fingerprint
    }

    #[must_use]
    pub fn audit_action(&self) -> &str {
        &self.audit_action
    }

    #[must_use]
    pub fn rate_limit_bucket(&self) -> &str {
        &self.rate_limit_bucket
    }

    #[must_use]
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }
}

impl fmt::Debug for LegacyExecutionCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyExecutionCommandV1")
            .field("operation_id", &self.operation_id)
            .field("client_family", &self.client_family)
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("scope_digest", &"<redacted>")
            .field("request_fingerprint", &"<redacted>")
            .field("audit_action", &self.audit_action)
            .field("rate_limit_bucket", &self.rate_limit_bucket)
            .field("correlation_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyOperationReceiptV1 {
    pub status: u16,
    pub result_digest: ChecksumSha256,
}

impl LegacyOperationReceiptV1 {
    pub fn new(status: u16, result_digest: ChecksumSha256) -> Result<Self, LegacyRegistryErrorV1> {
        if !(200..=299).contains(&status) {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        Ok(Self {
            status,
            result_digest,
        })
    }
}

impl fmt::Debug for LegacyOperationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyOperationReceiptV1")
            .field("status", &self.status)
            .field("result_digest", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyExecutionOutcomeV1 {
    Completed(LegacyOperationReceiptV1),
    Replay(LegacyOperationReceiptV1),
    InFlight,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyExecutionErrorV1 {
    #[error("legacy operation request is invalid")]
    Invalid,
    #[error("legacy operation resource was not found")]
    NotFound,
    #[error("legacy operation conflicts with durable state")]
    Conflict,
    #[error("legacy operation is unsupported")]
    Unsupported,
    #[error("legacy operation authority is temporarily unavailable")]
    TemporarilyUnavailable,
    #[error("legacy operation effect has an indeterminate result")]
    Indeterminate,
    #[error("legacy operation authority failed")]
    Internal,
}

/// Durable authority boundary for one admitted operation.
///
/// Implementations must atomically bind the operation ID, request fingerprint,
/// tenant-scoped idempotency key (when present), audit attempt/result, and
/// durable business receipt. A replay returns the original receipt; a key used
/// with another fingerprint returns [`LegacyExecutionErrorV1::Conflict`]. An
/// external effect must be represented by an outbox/fenced intent before the
/// network call so an indeterminate result cannot be resubmitted under a new
/// key.
#[async_trait]
pub trait LegacyOperationExecutionPortV1: Send + Sync {
    async fn execute_once(
        &self,
        command: &LegacyExecutionCommandV1,
    ) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyEndpointOutcomeV1 {
    UseLegacyFallback(LegacyFallbackV1),
    RetirementResponse(LegacyRetirementV1),
    Completed(LegacyOperationReceiptV1),
    Replay(LegacyOperationReceiptV1),
}

pub struct LegacyEndpointCoordinatorV1<ExecutionPort> {
    registry: LegacyCompatibilityRegistryV1,
    execution_port: ExecutionPort,
}

impl<ExecutionPort> LegacyEndpointCoordinatorV1<ExecutionPort>
where
    ExecutionPort: LegacyOperationExecutionPortV1,
{
    #[must_use]
    pub const fn new(
        registry: LegacyCompatibilityRegistryV1,
        execution_port: ExecutionPort,
    ) -> Self {
        Self {
            registry,
            execution_port,
        }
    }

    #[must_use]
    pub const fn registry(&self) -> &LegacyCompatibilityRegistryV1 {
        &self.registry
    }

    pub async fn execute(
        &self,
        request: &LegacyCompatibilityRequestV1,
        scope_digest: ChecksumSha256,
        request_fingerprint: ChecksumSha256,
    ) -> Result<LegacyEndpointOutcomeV1, ApiErrorV1> {
        match self.registry.admit(request)? {
            LegacyCompatibilityOutcomeV1::UseLegacyFallback(fallback) => {
                Ok(LegacyEndpointOutcomeV1::UseLegacyFallback(fallback))
            }
            LegacyCompatibilityOutcomeV1::RetirementResponse(retirement) => {
                Ok(LegacyEndpointOutcomeV1::RetirementResponse(retirement))
            }
            LegacyCompatibilityOutcomeV1::ServeFrame(admitted) => {
                let command = LegacyExecutionCommandV1 {
                    operation_id: admitted.operation_id,
                    client_family: admitted.client_family,
                    idempotency_key: request.envelope.idempotency_key.clone(),
                    scope_digest,
                    request_fingerprint,
                    audit_action: admitted.admission.audit_action,
                    rate_limit_bucket: admitted.admission.rate_limit_bucket,
                    correlation_id: admitted.admission.correlation_id,
                };
                match self.execution_port.execute_once(&command).await {
                    Ok(LegacyExecutionOutcomeV1::Completed(receipt)) => {
                        Ok(LegacyEndpointOutcomeV1::Completed(receipt))
                    }
                    Ok(LegacyExecutionOutcomeV1::Replay(receipt)) => {
                        Ok(LegacyEndpointOutcomeV1::Replay(receipt))
                    }
                    Ok(LegacyExecutionOutcomeV1::InFlight) => Err(public_registry_error(
                        ApiErrorCodeV1::Conflict,
                        command.correlation_id(),
                    )),
                    Err(error) => Err(public_registry_error(
                        execution_error_code(error),
                        command.correlation_id(),
                    )),
                }
            }
        }
    }
}

fn execution_error_code(error: LegacyExecutionErrorV1) -> ApiErrorCodeV1 {
    match error {
        LegacyExecutionErrorV1::Invalid => ApiErrorCodeV1::InvalidRequest,
        LegacyExecutionErrorV1::NotFound => ApiErrorCodeV1::NotFound,
        LegacyExecutionErrorV1::Conflict => ApiErrorCodeV1::Conflict,
        LegacyExecutionErrorV1::Unsupported => ApiErrorCodeV1::Unsupported,
        LegacyExecutionErrorV1::TemporarilyUnavailable => ApiErrorCodeV1::TemporarilyUnavailable,
        LegacyExecutionErrorV1::Indeterminate => ApiErrorCodeV1::Indeterminate,
        LegacyExecutionErrorV1::Internal => ApiErrorCodeV1::Internal,
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyRegistryErrorV1 {
    #[error("legacy operation contract is invalid")]
    InvalidContract,
    #[error("legacy operation contract is duplicated")]
    DuplicateContract,
}

fn public_registry_error(code: ApiErrorCodeV1, correlation_id: &str) -> ApiErrorV1 {
    ApiErrorV1::new(code, correlation_id, None).unwrap_or_else(|_| {
        ApiErrorV1::new(code, "invalid-correlation", None)
            .expect("fixed registry correlation ID is valid")
    })
}

fn valid_operation_id(value: &str) -> bool {
    value.strip_prefix("cap-v1-").is_some_and(|suffix| {
        suffix.len() == 16
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn valid_method(kind: LegacyOperationKindV1, value: &str) -> bool {
    match kind {
        LegacyOperationKindV1::HttpRoute => {
            matches!(
                value,
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
            )
        }
        LegacyOperationKindV1::Rpc => value == "RPC",
        LegacyOperationKindV1::ServerAction => value == "ACTION",
        LegacyOperationKindV1::Workflow => value == "WORKFLOW",
    }
}

fn valid_raw_legacy_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 2_048
        && value.is_ascii()
        && !value.contains(['%', ';', '\\'])
        && !value.contains("//")
        && !value.bytes().any(|byte| byte.is_ascii_control())
        && value != "/"
        && !value
            .split('/')
            .skip(1)
            .any(|segment| matches!(segment, "" | "." | ".."))
}

fn legacy_path_matches(pattern: &str, raw_path: &str) -> bool {
    let pattern = pattern.split('/').skip(1).collect::<Vec<_>>();
    let path = raw_path.split('/').skip(1).collect::<Vec<_>>();
    let mut path_index = 0;
    for (pattern_index, expected) in pattern.iter().enumerate() {
        if expected.starts_with(':') && expected.ends_with('*') {
            return pattern_index + 1 == pattern.len()
                && path_index < path.len()
                && path[path_index..]
                    .iter()
                    .all(|segment| safe_path_segment(segment));
        }
        let Some(actual) = path.get(path_index) else {
            return false;
        };
        if expected.starts_with(':') {
            if !safe_path_segment(actual) {
                return false;
            }
        } else if expected != actual {
            return false;
        }
        path_index += 1;
    }
    path_index == path.len()
}

fn safe_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn legacy_pattern_specificity(pattern: &str) -> (usize, bool, usize) {
    let segments = pattern.split('/').skip(1).collect::<Vec<_>>();
    (
        segments
            .iter()
            .filter(|segment| !segment.starts_with(':'))
            .count(),
        !segments.iter().any(|segment| segment.ends_with('*')),
        segments.len(),
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use frame_domain::{
        ApiAuthClassV1, IdempotencyKey, IdempotencyRequirementV1, LegacyEndpointDispositionV1,
    };
    use serde::Deserialize;

    use super::*;
    use crate::RateLimitDecisionV1;

    type MemoryExecutionKey = (String, String, String);
    type MemoryExecutionValue = (String, LegacyOperationReceiptV1);

    #[derive(Default)]
    struct MemoryExecutionPort {
        rows: Mutex<HashMap<MemoryExecutionKey, MemoryExecutionValue>>,
    }

    #[async_trait]
    impl LegacyOperationExecutionPortV1 for MemoryExecutionPort {
        async fn execute_once(
            &self,
            command: &LegacyExecutionCommandV1,
        ) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
            let receipt = LegacyOperationReceiptV1::new(
                200,
                ChecksumSha256::parse("ab".repeat(32)).expect("result digest"),
            )
            .expect("receipt");
            let Some(key) = command.idempotency_key() else {
                return Ok(LegacyExecutionOutcomeV1::Completed(receipt));
            };
            let map_key = (
                command.scope_digest().as_str().to_owned(),
                command.operation_id().to_owned(),
                key.expose().to_owned(),
            );
            let mut rows = self
                .rows
                .lock()
                .map_err(|_| LegacyExecutionErrorV1::Internal)?;
            if let Some((fingerprint, stored)) = rows.get(&map_key) {
                if fingerprint != command.request_fingerprint().as_str() {
                    return Err(LegacyExecutionErrorV1::Conflict);
                }
                return Ok(LegacyExecutionOutcomeV1::Replay(stored.clone()));
            }
            rows.insert(
                map_key,
                (
                    command.request_fingerprint().as_str().to_owned(),
                    receipt.clone(),
                ),
            );
            Ok(LegacyExecutionOutcomeV1::Completed(receipt))
        }
    }

    struct FailingExecutionPort(LegacyExecutionErrorV1);

    #[async_trait]
    impl LegacyOperationExecutionPortV1 for FailingExecutionPort {
        async fn execute_once(
            &self,
            _command: &LegacyExecutionCommandV1,
        ) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
            Err(self.0)
        }
    }

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
        contract_evidence: ReportEvidence,
    }

    #[derive(Deserialize)]
    struct ReportSecurity {
        max_body_bytes: u64,
        accepted_content_types: Vec<String>,
        rate_limit_bucket: String,
        idempotency: String,
    }

    #[derive(Deserialize)]
    struct ReportEvidence {
        success: String,
        validation: String,
        authorization: String,
        idempotency_retry: String,
        failure: String,
    }

    fn report() -> Report {
        serde_json::from_str(include_str!(
            "../../../fixtures/api-parity/v1/route-workflow-report.json"
        ))
        .expect("pinned API parity report")
    }

    fn compatibility() -> ClientCompatibilityPolicyV1 {
        ClientCompatibilityPolicyV1 {
            api_major: 1,
            current_release: 42,
            previous_release: 41,
            deprecated_after_ms: Some(1_900_000_000_000),
            retired: false,
        }
    }

    fn contract(
        row: &ReportRow,
        endpoint_contract_proven: bool,
        retirement_approved: bool,
    ) -> LegacyOperationContractV1 {
        let kind = match row.kind.as_str() {
            "route" => LegacyOperationKindV1::HttpRoute,
            "rpc" => LegacyOperationKindV1::Rpc,
            "server_action" => LegacyOperationKindV1::ServerAction,
            "workflow" => LegacyOperationKindV1::Workflow,
            _ => panic!("unknown operation kind"),
        };
        let auth = match row.auth.as_str() {
            "anonymous" | "anonymous_or_session_or_api_key" | "public" | "public_or_flow_token" => {
                ApiAuthClassV1::Public
            }
            "optional_session_or_share_capability" | "public_or_session" => {
                ApiAuthClassV1::OptionalSession
            }
            "session" => ApiAuthClassV1::Session,
            "session_or_api_key" => ApiAuthClassV1::SessionOrApiKey,
            "developer_api_key" => ApiAuthClassV1::ApiKey,
            // Signed OAuth state resolves an authenticated, actor-bound
            // capability before this compatibility gateway is entered.
            "signed_state" => ApiAuthClassV1::Session,
            "signed_webhook" => ApiAuthClassV1::Webhook,
            // A parent receipt is an internal durable-workflow credential. It
            // must remain authenticated, but it is not a scheduler secret.
            "internal_service" | "parent_receipt" => ApiAuthClassV1::Worker,
            "scheduler_secret" => ApiAuthClassV1::Scheduler,
            "admin_session" => ApiAuthClassV1::Admin,
            unknown => panic!("unknown auth class: {unknown}"),
        };
        let idempotency = match row.security.idempotency.as_str() {
            "required" => IdempotencyRequirementV1::Required,
            "optional" => IdempotencyRequirementV1::Optional,
            "forbidden" => IdempotencyRequirementV1::Forbidden,
            _ => panic!("unknown idempotency policy"),
        };
        let disposition = match row.disposition.as_str() {
            "replace" | "protected_parity_required" => LegacyEndpointDispositionV1::Replace,
            "migrate" => LegacyEndpointDispositionV1::Migrate,
            "retire" => LegacyEndpointDispositionV1::Retire,
            _ => panic!("unknown disposition"),
        };
        let clients = row
            .clients
            .iter()
            .map(|client| match client.as_str() {
                "web" => LegacyClientFamilyV1::Web,
                "desktop" => LegacyClientFamilyV1::Desktop,
                "mobile" => LegacyClientFamilyV1::Mobile,
                "extension" => LegacyClientFamilyV1::Extension,
                "developer" => LegacyClientFamilyV1::Developer,
                "internal_worker" => LegacyClientFamilyV1::InternalWorker,
                "provider" => LegacyClientFamilyV1::Provider,
                "scheduler" => LegacyClientFamilyV1::Scheduler,
                _ => panic!("unknown client family"),
            })
            .collect();
        LegacyOperationContractV1::new(
            row.id.clone(),
            kind,
            row.method.clone(),
            row.legacy_path.clone(),
            clients,
            ApiRequestPolicyV1 {
                auth,
                max_body_bytes: row.security.max_body_bytes,
                accepted_content_types: row.security.accepted_content_types.clone(),
                idempotency,
                rate_limit_bucket: row.security.rate_limit_bucket.clone(),
                audit_action: format!("legacy.{}", row.id),
            },
            disposition,
            LegacyOperationEvidenceV1 {
                endpoint_contract_proven,
                client_family_enabled: true,
                legacy_fallback_available: true,
                retirement_approved,
            },
        )
        .expect("valid pinned operation")
    }

    fn released_caller(family: LegacyClientFamilyV1, release: u32) -> Option<LegacyCallerV1> {
        let surface = match family {
            LegacyClientFamilyV1::Web => ClientSurfaceV1::Web,
            LegacyClientFamilyV1::Desktop => ClientSurfaceV1::Desktop,
            LegacyClientFamilyV1::Mobile => ClientSurfaceV1::Mobile,
            LegacyClientFamilyV1::Extension => ClientSurfaceV1::Extension,
            LegacyClientFamilyV1::Developer => ClientSurfaceV1::Developer,
            LegacyClientFamilyV1::InternalWorker
            | LegacyClientFamilyV1::Provider
            | LegacyClientFamilyV1::Scheduler => return None,
        };
        Some(LegacyCallerV1::Released(ClientReleaseV1 {
            surface,
            api_major: 1,
            release,
        }))
    }

    fn service_caller(family: LegacyClientFamilyV1) -> LegacyCallerV1 {
        match family {
            LegacyClientFamilyV1::InternalWorker => LegacyCallerV1::InternalWorker,
            LegacyClientFamilyV1::Provider => LegacyCallerV1::Provider,
            LegacyClientFamilyV1::Scheduler => LegacyCallerV1::Scheduler,
            _ => panic!("released caller required"),
        }
    }

    fn success_envelope(contract: &LegacyOperationContractV1) -> ApiMutationEnvelopeV1 {
        let required = contract.request_policy.idempotency == IdempotencyRequirementV1::Required;
        ApiMutationEnvelopeV1 {
            content_length: u64::from(contract.request_policy.max_body_bytes > 0) * 2,
            content_type: (contract.request_policy.max_body_bytes > 0).then(|| {
                contract
                    .request_policy
                    .accepted_content_types
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "application/octet-stream".to_owned())
            }),
            idempotency_key: required
                .then(|| IdempotencyKey::parse("compatibility-case-0001").expect("key")),
            correlation_id: "compatibility-case".into(),
        }
    }

    const fn allowed_security() -> RequestSecurityContextV1 {
        RequestSecurityContextV1 {
            authenticated: true,
            authorized: true,
            browser_origin_valid: true,
            csrf_valid: true,
            rate_limit: RateLimitDecisionV1::Allowed,
        }
    }

    fn request(
        contract: &LegacyOperationContractV1,
        caller: LegacyCallerV1,
    ) -> LegacyCompatibilityRequestV1 {
        LegacyCompatibilityRequestV1 {
            operation_id: contract.id.clone(),
            caller,
            envelope: success_envelope(contract),
            security: allowed_security(),
        }
    }

    fn primary_caller(contract: &LegacyOperationContractV1, release: u32) -> LegacyCallerV1 {
        let family = contract.clients[0];
        released_caller(family, release).unwrap_or_else(|| service_caller(family))
    }

    fn concrete_http_path(pattern: &str) -> String {
        let mut output = Vec::new();
        for segment in pattern.split('/').skip(1) {
            if segment.starts_with(':') && segment.ends_with('*') {
                output.extend(["fixture", "nested"]);
            } else if segment.starts_with(':') {
                output.push("fixture");
            } else {
                output.push(segment);
            }
        }
        format!("/{}", output.join("/"))
    }

    #[test]
    fn regenerated_auth_classes_preserve_shared_admission_semantics() {
        let report = report();
        for (source, expected, requires_authentication) in [
            (
                "anonymous_or_session_or_api_key",
                ApiAuthClassV1::Public,
                false,
            ),
            ("public", ApiAuthClassV1::Public, false),
            ("public_or_session", ApiAuthClassV1::OptionalSession, false),
            ("signed_state", ApiAuthClassV1::Session, true),
            ("parent_receipt", ApiAuthClassV1::Worker, true),
        ] {
            let row = report
                .entries
                .iter()
                .find(|row| row.auth == source)
                .unwrap_or_else(|| panic!("pinned report is missing auth class: {source}"));
            let operation = contract(row, true, false);
            assert_eq!(operation.request_policy.auth, expected);

            let registry =
                LegacyCompatibilityRegistryV1::new(vec![operation.clone()], compatibility())
                    .expect("single operation registry");
            let mut unauthenticated = request(&operation, primary_caller(&operation, 42));
            unauthenticated.security.authenticated = false;
            let outcome = registry.admit(&unauthenticated);
            if requires_authentication {
                assert_eq!(
                    outcome
                        .expect_err("capability credential must be authenticated")
                        .code,
                    ApiErrorCodeV1::Unauthenticated
                );
            } else {
                assert!(matches!(
                    outcome,
                    Ok(LegacyCompatibilityOutcomeV1::ServeFrame(_))
                ));
            }
        }
    }

    #[test]
    fn pinned_registry_is_exhaustive_and_keeps_every_unproven_operation_on_fallback() {
        let report = report();
        let contracts = report
            .entries
            .iter()
            .map(|row| contract(row, false, false))
            .collect::<Vec<_>>();
        let registry = LegacyCompatibilityRegistryV1::new(contracts, compatibility())
            .expect("complete registry");
        assert_eq!(registry.len(), 288);

        let mut released_associations = 0;
        for row in &report.entries {
            let contract = registry.contract(&row.id).expect("registered row");
            for family in contract.clients() {
                if let Some(current) = released_caller(*family, 42) {
                    released_associations += 1;
                    assert!(matches!(
                        registry.admit(&request(contract, current)),
                        Ok(LegacyCompatibilityOutcomeV1::UseLegacyFallback(_))
                    ));
                    let previous = released_caller(*family, 41).expect("previous caller");
                    assert!(matches!(
                        registry.admit(&request(contract, previous)),
                        Ok(LegacyCompatibilityOutcomeV1::UseLegacyFallback(_))
                    ));
                    let old = released_caller(*family, 40).expect("old caller");
                    assert_eq!(
                        registry
                            .admit(&request(contract, old))
                            .expect_err("unsupported release must be rejected")
                            .code,
                        ApiErrorCodeV1::UpgradeRequired
                    );
                } else {
                    let caller = service_caller(*family);
                    assert!(matches!(
                        registry.admit(&request(contract, caller)),
                        Ok(LegacyCompatibilityOutcomeV1::UseLegacyFallback(_))
                    ));
                }
            }
        }
        assert_eq!(released_associations, 267);
    }

    #[test]
    fn pinned_report_identifies_only_its_exact_local_contracts() {
        const STATUS_OPERATION_ID: &str = "cap-v1-05b6ba3f76daac22";
        const MEDIA_SERVER_ROOT_OPERATION_ID: &str = "cap-v1-ff19008f47194c43";
        const CHANGELOG_STATUS_GET_OPERATION_ID: &str = "cap-v1-a1b180c5d123c870";
        const CHANGELOG_STATUS_OPTIONS_OPERATION_ID: &str = "cap-v1-16668b858461f386";
        const CHANGELOG_FEED_GET_OPERATION_ID: &str = "cap-v1-0fa8384f3666825b";
        const CHANGELOG_FEED_OPTIONS_OPERATION_ID: &str = "cap-v1-237f41f3086a2d67";
        const MOBILE_SESSION_CONFIG_OPERATION_ID: &str = "cap-v1-4f21920a947c4c84";
        const NOTIFICATION_PREFERENCES_OPERATION_ID: &str = "cap-v1-d130c840f654bd72";
        const ACTIVE_ORGANIZATION_OPERATION_ID: &str = "cap-v1-a3b4c805d409bc7c";
        const THEME_OPERATION_ID: &str = "cap-v1-7773d3e70d1d5919";
        const ADD_VIDEOS_TO_FOLDER_OPERATION_ID: &str = "cap-v1-f5daa7be337a2979";
        const REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID: &str = "cap-v1-1af3645bf2ae7168";
        const MOVE_VIDEO_TO_FOLDER_OPERATION_ID: &str = "cap-v1-eaf277e644aa4b92";
        const ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID: &str = "cap-v1-d96a1931942eb83b";
        const REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID: &str = "cap-v1-0694e68a64976c9a";
        const ADD_VIDEOS_TO_SPACE_OPERATION_ID: &str = "cap-v1-bb55b5eeeb5e31ab";
        const REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID: &str = "cap-v1-ccbe5f1381eaa1b4";
        const MARK_NOTIFICATIONS_READ_OPERATION_ID: &str = "cap-v1-74a775753d3863c7";
        const UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID: &str = "cap-v1-1f6a43a05f2f297c";
        const MOBILE_CREATE_FOLDER_OPERATION_ID: &str = "cap-v1-7160c4389375c682";
        const RPC_FOLDER_CREATE_OPERATION_ID: &str = "cap-v1-9e125712cee9ce5a";
        const RPC_FOLDER_DELETE_OPERATION_ID: &str = "cap-v1-eea1796482b3af28";
        const RPC_FOLDER_UPDATE_OPERATION_ID: &str = "cap-v1-a193e9e08b2c3f7d";

        let report = report();
        let contracts = report
            .entries
            .iter()
            .map(|row| {
                contract(
                    row,
                    row.contract_evidence.success == "local_contract",
                    false,
                )
            })
            .collect::<Vec<_>>();
        let registry = LegacyCompatibilityRegistryV1::new(contracts, compatibility())
            .expect("complete registry");

        let promoted = report
            .entries
            .iter()
            .filter(|row| row.contract_evidence.success == "local_contract")
            .collect::<Vec<_>>();
        let expected_locally_proven_ids = promoted
            .iter()
            .map(|row| row.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for (id, kind, method, path) in [
            (STATUS_OPERATION_ID, "route", "GET", "/api/status"),
            (
                MEDIA_SERVER_ROOT_OPERATION_ID,
                "route",
                "GET",
                "/media-server",
            ),
            (
                CHANGELOG_STATUS_GET_OPERATION_ID,
                "route",
                "GET",
                "/api/changelog/status",
            ),
            (
                CHANGELOG_STATUS_OPTIONS_OPERATION_ID,
                "route",
                "OPTIONS",
                "/api/changelog/status",
            ),
            (
                CHANGELOG_FEED_GET_OPERATION_ID,
                "route",
                "GET",
                "/api/changelog",
            ),
            (
                CHANGELOG_FEED_OPTIONS_OPERATION_ID,
                "route",
                "OPTIONS",
                "/api/changelog",
            ),
            (
                MOBILE_SESSION_CONFIG_OPERATION_ID,
                "route",
                "GET",
                "/api/mobile/session/config",
            ),
            (
                NOTIFICATION_PREFERENCES_OPERATION_ID,
                "route",
                "GET",
                "/api/notifications/preferences",
            ),
            (
                ACTIVE_ORGANIZATION_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/app/(org)/dashboard/_components/Navbar/server.ts#updateActiveOrganization",
            ),
            (
                THEME_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/app/(org)/dashboard/_components/actions.ts#setTheme",
            ),
            (
                ADD_VIDEOS_TO_FOLDER_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/folders/add-videos.ts#addVideosToFolder",
            ),
            (
                REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/folders/remove-videos.ts#removeVideosFromFolder",
            ),
            (
                MOVE_VIDEO_TO_FOLDER_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/folders/moveVideoToFolder.ts#moveVideoToFolder",
            ),
            (
                ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/organizations/add-videos.ts#addVideosToOrganization",
            ),
            (
                REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/organizations/remove-videos.ts#removeVideosFromOrganization",
            ),
            (
                ADD_VIDEOS_TO_SPACE_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/spaces/add-videos.ts#addVideosToSpace",
            ),
            (
                REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/spaces/remove-videos.ts#removeVideosFromSpace",
            ),
            (
                MARK_NOTIFICATIONS_READ_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/notifications/mark-as-read.ts#markAsRead",
            ),
            (
                UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID,
                "server_action",
                "ACTION",
                "action://apps/web/actions/notifications/update-preferences.ts#updatePreferences",
            ),
            (
                MOBILE_CREATE_FOLDER_OPERATION_ID,
                "route",
                "POST",
                "/api/mobile/folders",
            ),
            (
                RPC_FOLDER_CREATE_OPERATION_ID,
                "rpc",
                "RPC",
                "/api/erpc#FolderCreate",
            ),
            (
                RPC_FOLDER_DELETE_OPERATION_ID,
                "rpc",
                "RPC",
                "/api/erpc#FolderDelete",
            ),
            (
                RPC_FOLDER_UPDATE_OPERATION_ID,
                "rpc",
                "RPC",
                "/api/erpc#FolderUpdate",
            ),
        ] {
            let operation = promoted
                .iter()
                .find(|operation| operation.id == id)
                .expect("exact promoted operation");
            assert_eq!(operation.kind, kind);
            assert_eq!(operation.method, method);
            assert_eq!(operation.legacy_path, path);
        }
        for operation in &promoted {
            assert_eq!(operation.contract_evidence.validation, "local_contract");
            assert_eq!(operation.contract_evidence.authorization, "local_contract");
            assert_eq!(
                operation.contract_evidence.idempotency_retry,
                "local_contract"
            );
            assert_eq!(operation.contract_evidence.failure, "local_contract");
        }

        let mut released_associations = 0;
        let mut registered_locally_proven_ids = std::collections::BTreeSet::new();
        for row in &report.entries {
            let operation = registry.contract(&row.id).expect("registered row");
            if operation.evidence().endpoint_contract_proven {
                registered_locally_proven_ids.insert(operation.id());
            }
            let expected_frame = expected_locally_proven_ids.contains(row.id.as_str());
            assert_eq!(
                operation.evidence().endpoint_contract_proven,
                expected_frame
            );
            for family in operation.clients() {
                if let Some(current) = released_caller(*family, 42) {
                    released_associations += 1;
                    let outcome = registry
                        .admit(&request(operation, current))
                        .expect("current release decision");
                    assert_eq!(
                        matches!(outcome, LegacyCompatibilityOutcomeV1::ServeFrame(_)),
                        expected_frame
                    );

                    let previous = released_caller(*family, 41).expect("previous caller");
                    let outcome = registry
                        .admit(&request(operation, previous))
                        .expect("previous release decision");
                    assert_eq!(
                        matches!(outcome, LegacyCompatibilityOutcomeV1::ServeFrame(_)),
                        expected_frame
                    );
                } else {
                    let outcome = registry
                        .admit(&request(operation, service_caller(*family)))
                        .expect("service caller decision");
                    assert_eq!(
                        matches!(outcome, LegacyCompatibilityOutcomeV1::ServeFrame(_)),
                        expected_frame
                    );
                }
            }
        }
        assert_eq!(registry.len(), report.entries.len());
        assert_eq!(registered_locally_proven_ids, expected_locally_proven_ids);
        assert_eq!(released_associations, 267);
    }

    #[test]
    fn every_retained_row_passes_the_shared_admission_axes_when_endpoint_evidence_is_enabled() {
        let report = report();
        let retained = report
            .entries
            .iter()
            .filter(|row| row.disposition != "retire")
            .collect::<Vec<_>>();
        assert_eq!(retained.len(), 278);
        for row in retained {
            let contract = contract(row, true, false);
            let registry =
                LegacyCompatibilityRegistryV1::new(vec![contract.clone()], compatibility())
                    .expect("single operation registry");
            let caller = primary_caller(&contract, 42);
            let valid = request(&contract, caller.clone());
            let admitted = registry.admit(&valid).expect("admitted");
            let LegacyCompatibilityOutcomeV1::ServeFrame(admitted) = admitted else {
                panic!("evidence-enabled operation must reach Frame admission")
            };
            assert_eq!(admitted.operation_id, contract.id());
            assert_eq!(
                admitted.admission.audit_action,
                format!("legacy.{}", row.id)
            );
            assert_eq!(
                admitted.admission.rate_limit_bucket,
                row.security.rate_limit_bucket
            );

            let mut invalid = request(&contract, caller.clone());
            invalid.envelope.content_length = contract.request_policy.max_body_bytes + 1;
            assert_eq!(
                registry
                    .admit(&invalid)
                    .expect_err("oversized request must be rejected")
                    .code,
                ApiErrorCodeV1::InvalidRequest
            );

            let mut denied = request(&contract, caller.clone());
            if matches!(
                contract.request_policy.auth,
                ApiAuthClassV1::Public | ApiAuthClassV1::OptionalSession | ApiAuthClassV1::Webhook
            ) {
                denied.security.authorized = false;
                assert_eq!(
                    registry
                        .admit(&denied)
                        .expect_err("forbidden lookup must be hidden")
                        .code,
                    ApiErrorCodeV1::NotFound
                );
            } else {
                denied.security.authenticated = false;
                assert_eq!(
                    registry
                        .admit(&denied)
                        .expect_err("unauthenticated request must be rejected")
                        .code,
                    ApiErrorCodeV1::Unauthenticated
                );
            }

            let mut idempotency = request(&contract, caller.clone());
            match contract.request_policy.idempotency {
                IdempotencyRequirementV1::Required => idempotency.envelope.idempotency_key = None,
                IdempotencyRequirementV1::Forbidden => {
                    idempotency.envelope.idempotency_key =
                        Some(IdempotencyKey::parse("compatibility-case-0002").expect("key"));
                }
                IdempotencyRequirementV1::Optional => continue,
            }
            assert_eq!(
                registry
                    .admit(&idempotency)
                    .expect_err("idempotency policy mismatch must be rejected")
                    .code,
                ApiErrorCodeV1::InvalidRequest
            );

            let mut limited = request(&contract, caller);
            limited.security.rate_limit = RateLimitDecisionV1::Rejected {
                retry_after_ms: 1_000,
            };
            let error = registry
                .admit(&limited)
                .expect_err("rate-limited request must be rejected");
            assert_eq!(error.code, ApiErrorCodeV1::RateLimited);
            assert_eq!(error.retry_after_ms, Some(1_000));
            assert!(!format!("{error:?}").contains("compatibility-case"));
        }
    }

    #[test]
    fn retirement_requires_explicit_approval_and_never_fabricates_frame_success() {
        let report = report();
        let retirements = report
            .entries
            .iter()
            .filter(|row| row.disposition == "retire")
            .collect::<Vec<_>>();
        assert_eq!(retirements.len(), 10);
        for row in retirements {
            let pending = contract(row, false, false);
            let caller = primary_caller(&pending, 42);
            let registry =
                LegacyCompatibilityRegistryV1::new(vec![pending.clone()], compatibility())
                    .expect("pending retirement");
            assert!(matches!(
                registry.admit(&request(&pending, caller)),
                Ok(LegacyCompatibilityOutcomeV1::UseLegacyFallback(_))
            ));

            let approved = contract(row, false, true);
            let caller = primary_caller(&approved, 42);
            let registry =
                LegacyCompatibilityRegistryV1::new(vec![approved.clone()], compatibility())
                    .expect("approved retirement contract");
            let outcome = registry
                .admit(&request(&approved, caller))
                .expect("approved retirement returns the pinned response");
            let LegacyCompatibilityOutcomeV1::RetirementResponse(response) = outcome else {
                panic!("approved retirement fabricated a non-retirement outcome");
            };
            assert_eq!(response.operation_id, row.id);
            assert_eq!(response.client_family, approved.clients()[0]);
            assert_eq!(response.schema_version, LEGACY_RETIREMENT_SCHEMA_V1);
            assert_eq!(response.http_status, 410);
            assert_eq!(response.code, LEGACY_RETIREMENT_CODE_V1);
            assert_eq!(response.message, LEGACY_RETIREMENT_MESSAGE_V1);
            assert_eq!(response.migration_path, LEGACY_RETIREMENT_MIGRATION_V1);
            assert_eq!(response.cache_control, LEGACY_RETIREMENT_CACHE_CONTROL_V1);
            assert!(!response.retryable);
        }
    }

    #[test]
    fn registry_rejects_duplicates_and_cross_family_probing() {
        let row = &report().entries[0];
        let contract = contract(row, true, false);
        assert_eq!(
            LegacyCompatibilityRegistryV1::new(
                vec![contract.clone(), contract.clone()],
                compatibility()
            )
            .expect_err("duplicate registry row must be rejected"),
            LegacyRegistryErrorV1::DuplicateContract
        );
        let registry = LegacyCompatibilityRegistryV1::new(vec![contract.clone()], compatibility())
            .expect("registry");
        let disallowed = [
            LegacyCallerV1::InternalWorker,
            LegacyCallerV1::Provider,
            LegacyCallerV1::Scheduler,
        ]
        .into_iter()
        .find(|caller| !contract.clients.contains(&caller.family()))
        .expect("a disallowed family");
        assert_eq!(
            registry
                .admit(&request(&contract, disallowed))
                .expect_err("cross-family probe must be hidden")
                .code,
            ApiErrorCodeV1::NotFound
        );
    }

    #[test]
    fn raw_http_resolution_rejects_equal_specificity_ambiguity() {
        let row = report()
            .entries
            .into_iter()
            .find(|row| row.disposition != "retire")
            .expect("retained row");
        let mut by_id = contract(&row, true, false);
        by_id.id = "cap-v1-0000000000000001".into();
        by_id.kind = LegacyOperationKindV1::HttpRoute;
        by_id.method = "GET".into();
        by_id.legacy_identity = "/api/items/:id".into();
        by_id.clients = vec![LegacyClientFamilyV1::Web];
        let mut by_slug = by_id.clone();
        by_slug.id = "cap-v1-0000000000000002".into();
        by_slug.legacy_identity = "/api/items/:slug".into();
        let registry =
            LegacyCompatibilityRegistryV1::new(vec![by_id.clone(), by_slug], compatibility())
                .expect("ambiguous contracts can coexist by stable identity");
        assert!(registry.resolve_http("GET", "/api/items/one").is_none());

        let mut exact = by_id.clone();
        exact.id = "cap-v1-0000000000000003".into();
        exact.legacy_identity = "/api/items/one".into();
        let registry =
            LegacyCompatibilityRegistryV1::new(vec![by_id, exact.clone()], compatibility())
                .expect("specific contracts");
        assert_eq!(
            registry
                .resolve_http("GET", "/api/items/one")
                .map(LegacyOperationContractV1::id),
            Some(exact.id())
        );
    }

    #[test]
    fn every_inventory_identity_and_raw_http_pattern_resolves_without_decoding() {
        let report = report();
        let contracts = report
            .entries
            .iter()
            .map(|row| contract(row, false, false))
            .collect::<Vec<_>>();
        let registry = LegacyCompatibilityRegistryV1::new(contracts, compatibility())
            .expect("complete registry");
        let mut http_rows = 0;
        for row in &report.entries {
            let contract = registry.contract(&row.id).expect("registered row");
            assert_eq!(
                registry
                    .resolve_exact(
                        contract.kind(),
                        contract.method(),
                        contract.legacy_identity()
                    )
                    .map(LegacyOperationContractV1::id),
                Some(row.id.as_str())
            );
            if row.kind == "route" {
                http_rows += 1;
                let path = concrete_http_path(&row.legacy_path);
                assert_eq!(
                    registry
                        .resolve_http(&row.method, &path)
                        .map(LegacyOperationContractV1::id),
                    Some(row.id.as_str()),
                    "{} {}",
                    row.method,
                    row.legacy_path
                );
            }
        }
        assert_eq!(http_rows, 138);
        for hostile in [
            "/api//mobile",
            "/api/../private",
            "/api/%2e%2e/private",
            "/api/mobile\\caps",
            "/api/mobile;caps",
            "/api/mobile\nheader",
        ] {
            assert!(registry.resolve_http("GET", hostile).is_none(), "{hostile}");
        }
    }

    #[tokio::test]
    async fn every_retained_row_reaches_the_atomic_execution_and_audit_port_boundary() {
        let report = report();
        let retained = report
            .entries
            .iter()
            .filter(|row| row.disposition != "retire")
            .collect::<Vec<_>>();
        assert_eq!(retained.len(), 278);
        for row in retained {
            let contract = contract(row, true, false);
            let caller = primary_caller(&contract, 42);
            let request = request(&contract, caller);
            let registry =
                LegacyCompatibilityRegistryV1::new(vec![contract.clone()], compatibility())
                    .expect("registry");
            let coordinator =
                LegacyEndpointCoordinatorV1::new(registry, MemoryExecutionPort::default());
            let scope = ChecksumSha256::parse("02".repeat(32)).expect("scope digest");
            let fingerprint = ChecksumSha256::parse("cd".repeat(32)).expect("fingerprint");
            assert!(matches!(
                coordinator
                    .execute(&request, scope.clone(), fingerprint.clone())
                    .await,
                Ok(LegacyEndpointOutcomeV1::Completed(_))
            ));
            let replay = coordinator
                .execute(&request, scope.clone(), fingerprint)
                .await
                .expect("repeat outcome");
            if contract.request_policy.idempotency == IdempotencyRequirementV1::Required {
                assert!(matches!(replay, LegacyEndpointOutcomeV1::Replay(_)));
                let conflicting = ChecksumSha256::parse("ef".repeat(32)).expect("fingerprint");
                assert_eq!(
                    coordinator
                        .execute(&request, scope, conflicting)
                        .await
                        .expect_err("conflicting replay must be rejected")
                        .code,
                    ApiErrorCodeV1::Conflict
                );
            } else {
                assert!(matches!(replay, LegacyEndpointOutcomeV1::Completed(_)));
            }
        }
    }

    #[tokio::test]
    async fn execution_failures_map_to_closed_public_errors() {
        let row = report()
            .entries
            .into_iter()
            .find(|row| row.disposition == "replace")
            .expect("retained row");
        let contract = contract(&row, true, false);
        let request = request(&contract, primary_caller(&contract, 42));
        let expected = [
            (
                LegacyExecutionErrorV1::Invalid,
                ApiErrorCodeV1::InvalidRequest,
            ),
            (LegacyExecutionErrorV1::NotFound, ApiErrorCodeV1::NotFound),
            (LegacyExecutionErrorV1::Conflict, ApiErrorCodeV1::Conflict),
            (
                LegacyExecutionErrorV1::Unsupported,
                ApiErrorCodeV1::Unsupported,
            ),
            (
                LegacyExecutionErrorV1::TemporarilyUnavailable,
                ApiErrorCodeV1::TemporarilyUnavailable,
            ),
            (
                LegacyExecutionErrorV1::Indeterminate,
                ApiErrorCodeV1::Indeterminate,
            ),
            (LegacyExecutionErrorV1::Internal, ApiErrorCodeV1::Internal),
        ];
        for (failure, code) in expected {
            let registry =
                LegacyCompatibilityRegistryV1::new(vec![contract.clone()], compatibility())
                    .expect("registry");
            let coordinator =
                LegacyEndpointCoordinatorV1::new(registry, FailingExecutionPort(failure));
            let error = coordinator
                .execute(
                    &request,
                    ChecksumSha256::parse("02".repeat(32)).expect("scope digest"),
                    ChecksumSha256::parse("01".repeat(32)).expect("fingerprint"),
                )
                .await
                .expect_err("execution failure must be public error");
            assert_eq!(error.code, code);
            assert!(!format!("{error:?}").contains("compatibility-case"));
        }
    }
}
