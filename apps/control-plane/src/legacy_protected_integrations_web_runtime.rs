//! Callable route, Effect-RPC, server-action, and workflow carriers for the
//! provider-gated integration family.
//!
//! Shared authentication/CSRF/RPC decoders may call the typed carriers below.
//! Direct HTTP routes authenticate locally, decode the pinned carrier, and
//! stop at a 503 evidence gate after durable D1 staging.

use frame_application::{
    LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES, LegacyProtectedIntegrationAuthV1,
    LegacyProtectedIntegrationCredentialKindV1, LegacyProtectedIntegrationEntitlementBindingV1,
    LegacyProtectedIntegrationEnvelopeV1, LegacyProtectedIntegrationKindV1,
    LegacyProtectedIntegrationPolicyProofV1, LegacyProtectedIntegrationPrincipalV1,
    LegacyProtectedIntegrationProfileV1, RateLimitDecisionV1, ValidatedBrowserMutationProof,
    legacy_protected_integration_plaintext_request_digest, legacy_protected_integration_profile,
    legacy_protected_integration_replay_origin,
};
use frame_ui::class_contract::{ALERT_BASE, ALERT_DESTRUCTIVE, CARD};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{
    D1Database, Env, FormEntry, Request, Response, ResponseBody, Result, send::IntoSendFuture,
};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_protected_integrations_runtime::{
        D1LegacyProtectedIntegrationRuntimeV1, LegacyProtectedIntegrationFailureV1,
        LegacyProtectedIntegrationStageOutcomeV1, WORKFLOW_PARENT_READ_SQL,
    },
};

const PROTECTED_INTEGRATION_RPC_OPERATION_ID: &str = "cap-v1-5cd4cac9da73f975";
const PROTECTED_INTEGRATION_RPC_TAG: &str = "OrganisationSoftDelete";
const RPC_PARSE_FAILURE: &str = "Invalid Effect RPC request payload";
const RPC_UNKNOWN_TAG_FAILURE: &str = "Unknown Effect RPC request tag";
const API_KEY_ACTOR_SQL: &str =
    include_str!("../queries/legacy_org_custom_domain/api_key_actor.sql");
const VIDEO_POLICY_READ_SQL: &str = r#"
SELECT video.id AS canonical_video_id,video.owner_id,
       video.legacy_property_revision AS video_revision,
       video.legacy_password_hash AS video_password_hash,
       protected_space.id AS protected_space_id,
       protected_space.legacy_password_revision AS protected_space_revision,
       protected_space.legacy_password_hash AS protected_space_password_hash
FROM legacy_collaboration_video_aliases_v1 alias
JOIN videos video ON video.id=alias.mapped_video_id
LEFT JOIN spaces protected_space ON protected_space.id=(
  SELECT MIN(candidate.id)
  FROM space_videos placement
  JOIN spaces candidate ON candidate.id=placement.space_id
  WHERE placement.video_id=video.id AND candidate.deleted_at_ms IS NULL
    AND candidate.legacy_password_hash IS NOT NULL
)
WHERE alias.legacy_video_id=?1 AND video.deleted_at_ms IS NULL
LIMIT 2
"#;

#[derive(Debug, Deserialize)]
struct ApiKeyActorRowV1 {
    credential_subject_id: String,
    user_id: String,
}

#[derive(Debug, Deserialize)]
struct VideoPolicyRowV1 {
    canonical_video_id: String,
    owner_id: String,
    video_revision: i64,
    video_password_hash: Option<String>,
    protected_space_id: Option<String>,
    protected_space_revision: Option<i64>,
    protected_space_password_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkflowParentRowV1 {
    actor_id: Option<String>,
    tenant_id: Option<String>,
    credential_kind: String,
    credential_subject_id: Option<String>,
    credential_key_version: Option<i64>,
    credential_digest: Option<String>,
    credential_expires_at_ms: Option<i64>,
    policy_proofs_json: String,
    entitlement_kind: Option<String>,
    entitlement_subject_id: Option<String>,
    entitlement_revision: Option<i64>,
    entitlement_expires_at_ms: Option<i64>,
    authority_binding_digest: String,
}

#[derive(Debug, Clone, PartialEq)]
struct DecodedProtectedIntegrationRpcV1 {
    id: String,
    payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProtectedIntegrationRpcDecodeFailureV1 {
    Malformed(Option<String>),
    UnknownTag,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecodedProtectedIntegrationActionV1 {
    operation_id: String,
    payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
struct DecodedProtectedIntegrationRouteV1 {
    payload: Value,
    transport_body_digest: Option<String>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct ProtectedIntegrationProviderRequestV1 {
    operation_id: String,
    payload: Value,
    transport_body_digest: Option<String>,
}

impl std::fmt::Debug for ProtectedIntegrationProviderRequestV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProtectedIntegrationProviderRequestV1")
            .field("operation_id", &self.operation_id)
            .field("payload", &"[REDACTED]")
            .field("transport_body_digest", &self.transport_body_digest)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SealedProtectedIntegrationRequestV1 {
    pub opaque_ref: String,
    pub plaintext_digest: String,
}

/// The only interface that may receive exact provider inputs. Implementations
/// seal outside D1 and return a randomized opaque reference plus the
/// deterministic plaintext digest that the application contract recomputes.
pub(crate) trait ProtectedIntegrationRequestVaultV1 {
    fn seal(
        &self,
        request: &ProtectedIntegrationProviderRequestV1,
    ) -> std::result::Result<SealedProtectedIntegrationRequestV1, LegacyProtectedIntegrationFailureV1>;
}

struct UnavailableProtectedIntegrationRequestVaultV1;

impl ProtectedIntegrationRequestVaultV1 for UnavailableProtectedIntegrationRequestVaultV1 {
    fn seal(
        &self,
        _request: &ProtectedIntegrationProviderRequestV1,
    ) -> std::result::Result<SealedProtectedIntegrationRequestV1, LegacyProtectedIntegrationFailureV1>
    {
        Err(LegacyProtectedIntegrationFailureV1::Unavailable)
    }
}

pub async fn route_response(
    operation_id: &str,
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let Some(profile) = legacy_protected_integration_profile(operation_id) else {
        return failure_response(LegacyProtectedIntegrationFailureV1::Invalid);
    };
    if profile.kind != LegacyProtectedIntegrationKindV1::Route
        || request.method().to_string() != profile.method
    {
        return failure_response(LegacyProtectedIntegrationFailureV1::Invalid);
    }
    if profile.operation_id == "cap-v1-49531a09fd9433e7"
        && let Some(response) = google_callback_public_error_response(request)?
    {
        return Ok(response);
    }
    let principal = match authenticate_route(profile, request, env, now_ms).await? {
        Ok(principal) => principal,
        Err(response) => return Ok(response),
    };
    let database = env.d1("DB")?;
    let rate_subject = route_rate_subject(&principal, request)?;
    if matches!(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::OrganizationLibrary,
            &rate_subject,
            now_ms,
        )
        .await?,
        RateLimitDecisionV1::Rejected { .. }
    ) {
        return failure_response(LegacyProtectedIntegrationFailureV1::Unavailable);
    }
    let mut decoded = match decode_route_payload(profile, request).await {
        Ok(payload) => payload,
        Err(failure) => return failure_response(failure),
    };
    if let Err(failure) = canonicalize_source_route_payload(profile, request, &mut decoded.payload)
    {
        return failure_response(failure);
    }
    if profile.auth == LegacyProtectedIntegrationAuthV1::SignedState
        && let Err(failure) =
            bind_signed_state_organization(&mut decoded.payload, principal.tenant_id.as_deref())
    {
        return failure_response(failure);
    }
    let sealed = match seal_provider_request(
        profile,
        &decoded.payload,
        decoded.transport_body_digest.as_deref(),
        &UnavailableProtectedIntegrationRequestVaultV1,
    ) {
        Ok(sealed) => sealed,
        Err(failure) => return failure_response(failure),
    };
    let envelope = LegacyProtectedIntegrationEnvelopeV1 {
        source_operation_id: operation_id.into(),
        principal,
        replay_origin: legacy_protected_integration_replay_origin(profile),
        request_nonce: Uuid::new_v4().to_string(),
        payload: decoded.payload,
        sealed_request_ref: sealed.opaque_ref,
        sealed_request_digest: sealed.plaintext_digest,
        transport_body_digest: decoded.transport_body_digest,
        parent_family: None,
        parent_receipt_id: None,
        parent_request_digest: None,
        parent_authority_binding_digest: None,
    };
    stage_response(
        D1LegacyProtectedIntegrationRuntimeV1::new(&database)
            .stage(profile, &envelope, now_ms)
            .await,
        &UnavailableProtectedIntegrationTerminalResolverV1,
    )
    .await
}

fn google_callback_public_error_response(request: &Request) -> Result<Option<Response>> {
    let url = request.url()?;
    let query = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let message = if query.get("error").is_some_and(|value| !value.is_empty()) {
        Some("You can close this window and try again from Cap settings.")
    } else if query.get("code").is_none_or(|value| value.is_empty())
        || query.get("state").is_none_or(|value| value.is_empty())
    {
        Some("The authorization response was missing required data.")
    } else {
        None
    };
    let Some(message) = message else {
        return Ok(None);
    };
    let message = crate::control_plane_ui::escape_html(message);
    let content = format!(
        r#"<main class="w-full max-w-lg" tabindex="-1"><section class="{CARD}" aria-labelledby="integration-error-title"><p class="mb-2 text-sm font-semibold text-destructive">Integration error</p><h1 id="integration-error-title" class="mt-0 text-2xl font-bold">Google Drive was not connected</h1><div class="{ALERT_BASE} {ALERT_DESTRUCTIVE}" role="alert">{message}</div></section></main>"#
    );
    let body =
        crate::control_plane_ui::utility_document("Google Drive was not connected", &content);
    let mut response = Response::from_html(body)?.with_status(400);
    harden_terminal_response(&mut response)?;
    Ok(Some(response))
}

/// Return whether an already bounded Effect-RPC request is the one protected
/// integration RPC. The shared ERPC multiplexer performs this exact tag check
/// before invoking the strict decoder below.
#[must_use]
pub(crate) fn is_protected_integration_rpc_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("tag").and_then(Value::as_str).map(str::to_owned))
        .as_deref()
        == Some(PROTECTED_INTEGRATION_RPC_TAG)
}

/// Decode and stage Cap's `OrganisationSoftDelete` Effect-RPC request.
/// Provider execution remains fail-closed: a durable outbox receipt is
/// returned only as an opaque response header while the RPC exits with an
/// internal failure until independent provider evidence is present.
pub(crate) async fn effect_rpc_response_from_bytes(
    bytes: &[u8],
    request: &Request,
    env: &Env,
    _request_id: &str,
) -> Result<Response> {
    let decoded = match decode_protected_integration_rpc(bytes) {
        Ok(decoded) => decoded,
        Err(ProtectedIntegrationRpcDecodeFailureV1::Malformed(Some(id))) => {
            return protected_rpc_response(rpc_die(&id, RPC_PARSE_FAILURE), None);
        }
        Err(ProtectedIntegrationRpcDecodeFailureV1::Malformed(None)) => {
            return protected_rpc_response(rpc_defect(RPC_PARSE_FAILURE), None);
        }
        Err(ProtectedIntegrationRpcDecodeFailureV1::UnknownTag) => {
            return protected_rpc_response(rpc_defect(RPC_UNKNOWN_TAG_FAILURE), None);
        }
    };
    let now_ms = crate::current_time_ms()?;
    let binding = match browser_web_runtime::authenticate_host_only_browser_session_binding(
        request, env, now_ms,
    )
    .await?
    {
        Ok(binding) => binding,
        Err(BrowserWebFailure::Unavailable) => {
            return protected_rpc_response(rpc_internal_failure(&decoded.id, "database"), None);
        }
        Err(_) => {
            return protected_rpc_response(
                rpc_typed_failure(&decoded.id, json!({"_tag":"UnauthenticatedError"})),
                None,
            );
        }
    };
    let actor_id = binding.user_id.clone();
    let database = env.d1("DB")?;
    if matches!(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::OrganizationLibrary,
            &actor_id,
            now_ms,
        )
        .await?,
        RateLimitDecisionV1::Rejected { .. }
    ) {
        return protected_rpc_response(rpc_internal_failure(&decoded.id, "unknown"), None);
    }
    let tenant_id =
        browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await?;
    let profile = legacy_protected_integration_profile(PROTECTED_INTEGRATION_RPC_OPERATION_ID)
        .expect("checked-in protected integration RPC profile");
    let outcome = callable_with_vault(
        profile,
        &database,
        session_principal(
            LegacyProtectedIntegrationAuthV1::Session,
            &binding,
            tenant_id.as_deref(),
        ),
        decoded.payload,
        None,
        None,
        None,
        None,
        None,
        &UnavailableProtectedIntegrationRequestVaultV1,
        now_ms,
    )
    .await;
    let (value, receipt_id) = match outcome {
        Ok(LegacyProtectedIntegrationStageOutcomeV1::ProviderEvidenceRequired {
            receipt_id,
            ..
        }) => (
            rpc_internal_failure(&decoded.id, "unknown"),
            Some(receipt_id),
        ),
        Ok(LegacyProtectedIntegrationStageOutcomeV1::VerifiedSealedTerminal {
            sealed_terminal_ref,
            sealed_terminal_digest,
            ..
        }) => match resolve_terminal_json(
            &sealed_terminal_ref,
            &sealed_terminal_digest,
            &UnavailableProtectedIntegrationTerminalResolverV1,
        ) {
            Ok(response) => (rpc_success(&decoded.id, response), None),
            Err(_) => (rpc_internal_failure(&decoded.id, "unknown"), None),
        },
        Err(LegacyProtectedIntegrationFailureV1::Invalid) => {
            (rpc_die(&decoded.id, RPC_PARSE_FAILURE), None)
        }
        Err(LegacyProtectedIntegrationFailureV1::Unauthorized) => (
            rpc_typed_failure(&decoded.id, json!({"_tag":"PolicyDenied"})),
            None,
        ),
        Err(LegacyProtectedIntegrationFailureV1::Corrupt)
        | Err(LegacyProtectedIntegrationFailureV1::Unavailable) => {
            (rpc_internal_failure(&decoded.id, "database"), None)
        }
        Err(
            LegacyProtectedIntegrationFailureV1::Conflict
            | LegacyProtectedIntegrationFailureV1::ProviderEvidenceRequired,
        ) => (rpc_internal_failure(&decoded.id, "unknown"), None),
    };
    protected_rpc_response(value, receipt_id.as_deref())
}

#[must_use]
pub(crate) fn is_server_action(operation_id: &str) -> bool {
    legacy_protected_integration_profile(operation_id)
        .is_some_and(|profile| profile.kind == LegacyProtectedIntegrationKindV1::ServerAction)
}

/// Decode the authenticated compatibility-action bridge for one of the 22
/// source-pinned provider-backed actions. Authentication and one-use CSRF
/// grant consumption happen in `server_action_http_response`.
pub(crate) async fn decode_server_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedProtectedIntegrationActionV1>> {
    let Some(profile) = legacy_protected_integration_profile(operation_id)
        .filter(|profile| profile.kind == LegacyProtectedIntegrationKindV1::ServerAction)
    else {
        return Ok(Err(BrowserWebFailure::Invalid));
    };
    if !matches!(
        request.headers().get("content-type")?.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let declared = match request.headers().get("content-length")? {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
        },
        None => None,
    };
    let max_body_bytes = profile
        .max_body_bytes
        .clamp(4_096, LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES);
    if declared.is_some_and(|value| value == 0 || value > max_body_bytes) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = match crate::read_bounded_legacy_body(request, max_body_bytes).await {
        Ok(bytes) => bytes,
        Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    if bytes.is_empty()
        || bytes.len() > max_body_bytes
        || declared.is_some_and(|value| value != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let payload = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    Ok(Ok(DecodedProtectedIntegrationActionV1 {
        operation_id: operation_id.into(),
        payload,
    }))
}

/// Authenticate, rate-limit, consume the browser's one-use mutation grant,
/// and durably stage the protected provider intent. Provider evidence remains
/// a 503 gate rather than being projected as action success.
pub(crate) async fn server_action_http_response(
    request: &Request,
    env: &Env,
    decoded: &DecodedProtectedIntegrationActionV1,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Response>> {
    let Some(profile) = legacy_protected_integration_profile(&decoded.operation_id)
        .filter(|profile| profile.kind == LegacyProtectedIntegrationKindV1::ServerAction)
    else {
        return Ok(Err(BrowserWebFailure::Invalid));
    };
    let database = env.d1("DB")?;

    // These pinned actions are public carriers. They use edge-scoped
    // admission and generated replay, never a fabricated browser session or
    // mutation grant.
    if profile.auth == LegacyProtectedIntegrationAuthV1::Public {
        let rate_subject = edge_rate_subject(request)?;
        if matches!(
            compatibility_rate_limit::admit_principal(
                env,
                &database,
                CompatibilityRateLimitBucketV1::OrganizationLibrary,
                &rate_subject,
                now_ms,
            )
            .await?,
            RateLimitDecisionV1::Rejected { .. }
        ) {
            return Ok(Err(BrowserWebFailure::RateLimited));
        }
        let outcome = callable_with_vault(
            profile,
            &database,
            public_principal(profile.auth),
            decoded.payload.clone(),
            None,
            None,
            None,
            None,
            None,
            &UnavailableProtectedIntegrationRequestVaultV1,
            now_ms,
        )
        .await;
        return Ok(Ok(stage_response(
            outcome,
            &UnavailableProtectedIntegrationTerminalResolverV1,
        )
        .await?));
    }

    // getVideoStatus is read-like optional auth plus Cap's VideosPolicy. The
    // policy proof is video-first, then the lowest stable protected-space id,
    // and contains only a hash digest/revision rather than the raw password.
    if profile.auth == LegacyProtectedIntegrationAuthV1::PublicOrSession {
        let principal = match video_policy_principal(
            request,
            env,
            &database,
            &decoded.payload,
            now_ms,
        )
        .await?
        {
            Ok(principal) => principal,
            Err(failure) => return Ok(Err(failure)),
        };
        let rate_subject = route_rate_subject(&principal, request)?;
        if matches!(
            compatibility_rate_limit::admit_principal(
                env,
                &database,
                CompatibilityRateLimitBucketV1::OrganizationLibrary,
                &rate_subject,
                now_ms,
            )
            .await?,
            RateLimitDecisionV1::Rejected { .. }
        ) {
            return Ok(Err(BrowserWebFailure::RateLimited));
        }
        let outcome = callable_with_vault(
            profile,
            &database,
            principal,
            decoded.payload.clone(),
            None,
            None,
            None,
            None,
            None,
            &UnavailableProtectedIntegrationRequestVaultV1,
            now_ms,
        )
        .await;
        return Ok(Ok(stage_response(
            outcome,
            &UnavailableProtectedIntegrationTerminalResolverV1,
        )
        .await?));
    }

    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let binding = match browser_web_runtime::validated_browser_mutation_session_binding(
        &database, &proof, now_ms,
    )
    .await
    {
        Ok(Ok(binding)) => binding,
        Ok(Err(failure)) => {
            if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
                &database, &proof,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(failure));
        }
        Err(error) => {
            if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
                &database, &proof,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Err(error);
        }
    };
    let admission = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        now_ms,
    )
    .await;
    match admission {
        Ok(RateLimitDecisionV1::Allowed) => {}
        Ok(RateLimitDecisionV1::Rejected { .. }) => {
            if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
                &database, &proof,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(BrowserWebFailure::RateLimited));
        }
        Err(error) => {
            if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
                &database, &proof,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Err(error);
        }
    }
    let tenant_id =
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await {
            Ok(tenant_id) => tenant_id,
            Err(error) => {
                if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
                    &database, &proof,
                )
                .await?
                {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Err(error);
            }
        };
    let sealed = match seal_provider_request(
        profile,
        &decoded.payload,
        None,
        &UnavailableProtectedIntegrationRequestVaultV1,
    ) {
        Ok(sealed) => sealed,
        Err(failure) => {
            if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
                &database, &proof,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Ok(failure_response(failure)?));
        }
    };
    let outcome = callable_sealed(
        profile,
        &database,
        session_principal(profile.auth, &binding, tenant_id.as_deref()),
        decoded.payload.clone(),
        None,
        None,
        sealed,
        Some(&proof),
        now_ms,
    )
    .await;
    if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(&database, &proof)
        .await?
    {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    Ok(Ok(stage_response(
        outcome,
        &UnavailableProtectedIntegrationTerminalResolverV1,
    )
    .await?))
}

#[allow(clippy::too_many_arguments)]
pub async fn workflow_response(
    operation_id: &str,
    database: &D1Database,
    parent_family: &str,
    parent_receipt_id: &str,
    parent_request_digest: &str,
    payload: Value,
    now_ms: i64,
) -> std::result::Result<
    LegacyProtectedIntegrationStageOutcomeV1,
    LegacyProtectedIntegrationFailureV1,
> {
    let profile = legacy_protected_integration_profile(operation_id)
        .filter(|profile| profile.kind == LegacyProtectedIntegrationKindV1::Workflow)
        .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
    if !matches!(parent_family, "protected_integrations" | "protected_media")
        || !valid_uuid(parent_receipt_id)
        || !valid_digest(parent_request_digest)
    {
        return Err(LegacyProtectedIntegrationFailureV1::Invalid);
    }
    let result = database
        .prepare(WORKFLOW_PARENT_READ_SQL)
        .bind(&[
            JsValue::from_str(parent_family),
            JsValue::from_str(parent_receipt_id),
            JsValue::from_str(parent_request_digest),
            JsValue::from_str(operation_id),
        ])
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Unavailable)?;
    if !result.success() {
        return Err(LegacyProtectedIntegrationFailureV1::Unavailable);
    }
    let rows = result
        .results::<WorkflowParentRowV1>()
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?;
    let [parent] = rows.as_slice() else {
        return Err(if rows.is_empty() {
            LegacyProtectedIntegrationFailureV1::Unauthorized
        } else {
            LegacyProtectedIntegrationFailureV1::Corrupt
        });
    };
    let credential_kind = parse_credential_kind(&parent.credential_kind)
        .ok_or(LegacyProtectedIntegrationFailureV1::Corrupt)?;
    let policy_proofs = serde_json::from_str::<Vec<LegacyProtectedIntegrationPolicyProofV1>>(
        &parent.policy_proofs_json,
    )
    .map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)?;
    let inherited_entitlement_binding = match (
        parent.entitlement_kind.as_deref(),
        parent.entitlement_subject_id.as_deref(),
        parent.entitlement_revision,
        parent.entitlement_expires_at_ms,
    ) {
        (None, None, None, None) => None,
        (Some(kind), Some(subject_id), Some(revision), expires_at_ms) => {
            Some(LegacyProtectedIntegrationEntitlementBindingV1 {
                kind: kind.into(),
                subject_id: subject_id.into(),
                revision,
                expires_at_ms,
            })
        }
        _ => return Err(LegacyProtectedIntegrationFailureV1::Corrupt),
    };
    let principal = LegacyProtectedIntegrationPrincipalV1 {
        class: LegacyProtectedIntegrationAuthV1::ParentReceipt,
        actor_id: parent.actor_id.clone(),
        tenant_id: parent.tenant_id.clone(),
        credential_kind,
        credential_subject_id: parent.credential_subject_id.clone(),
        credential_key_version: parent.credential_key_version,
        credential_digest: parent.credential_digest.clone(),
        credential_expires_at_ms: parent.credential_expires_at_ms,
        policy_proofs,
        inherited_entitlement_binding,
    };
    callable_with_vault(
        profile,
        database,
        principal,
        payload,
        None,
        Some(parent_family.into()),
        Some(parent_receipt_id.into()),
        Some(parent_request_digest.into()),
        Some(parent.authority_binding_digest.clone()),
        &UnavailableProtectedIntegrationRequestVaultV1,
        now_ms,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn callable_with_vault(
    profile: &LegacyProtectedIntegrationProfileV1,
    database: &D1Database,
    principal: LegacyProtectedIntegrationPrincipalV1,
    payload: Value,
    transport_body_digest: Option<String>,
    parent_family: Option<String>,
    parent_receipt_id: Option<String>,
    parent_request_digest: Option<String>,
    parent_authority_binding_digest: Option<String>,
    vault: &dyn ProtectedIntegrationRequestVaultV1,
    now_ms: i64,
) -> std::result::Result<
    LegacyProtectedIntegrationStageOutcomeV1,
    LegacyProtectedIntegrationFailureV1,
> {
    let sealed = seal_provider_request(profile, &payload, transport_body_digest.as_deref(), vault)?;
    callable_sealed(
        profile,
        database,
        principal,
        payload,
        transport_body_digest,
        parent_family
            .zip(parent_receipt_id)
            .zip(parent_request_digest)
            .zip(parent_authority_binding_digest)
            .map(
                |(((family, receipt_id), request_digest), authority_digest)| {
                    (family, receipt_id, request_digest, authority_digest)
                },
            ),
        sealed,
        None,
        now_ms,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn callable_sealed(
    profile: &LegacyProtectedIntegrationProfileV1,
    database: &D1Database,
    principal: LegacyProtectedIntegrationPrincipalV1,
    payload: Value,
    transport_body_digest: Option<String>,
    parent: Option<(String, String, String, String)>,
    sealed: SealedProtectedIntegrationRequestV1,
    browser_proof: Option<&ValidatedBrowserMutationProof>,
    now_ms: i64,
) -> std::result::Result<
    LegacyProtectedIntegrationStageOutcomeV1,
    LegacyProtectedIntegrationFailureV1,
> {
    let envelope = LegacyProtectedIntegrationEnvelopeV1 {
        source_operation_id: profile.operation_id.into(),
        principal,
        replay_origin: legacy_protected_integration_replay_origin(profile),
        request_nonce: Uuid::new_v4().to_string(),
        payload,
        sealed_request_ref: sealed.opaque_ref,
        sealed_request_digest: sealed.plaintext_digest,
        transport_body_digest,
        parent_family: parent.as_ref().map(|(family, _, _, _)| family.clone()),
        parent_receipt_id: parent
            .as_ref()
            .map(|(_, receipt_id, _, _)| receipt_id.clone()),
        parent_request_digest: parent
            .as_ref()
            .map(|(_, _, request_digest, _)| request_digest.clone()),
        parent_authority_binding_digest: parent.map(|(_, _, _, authority_digest)| authority_digest),
    };
    D1LegacyProtectedIntegrationRuntimeV1::new(database)
        .stage_with_browser_proof(profile, &envelope, browser_proof, now_ms)
        .await
}

fn decode_protected_integration_rpc(
    bytes: &[u8],
) -> std::result::Result<DecodedProtectedIntegrationRpcV1, ProtectedIntegrationRpcDecodeFailureV1> {
    if bytes.is_empty() || bytes.len() > LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES {
        return Err(ProtectedIntegrationRpcDecodeFailureV1::Malformed(None));
    }
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| ProtectedIntegrationRpcDecodeFailureV1::Malformed(None))?;
    let object = value
        .as_object()
        .ok_or(ProtectedIntegrationRpcDecodeFailureV1::Malformed(None))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_rpc_id(value))
        .map(str::to_owned)
        .ok_or(ProtectedIntegrationRpcDecodeFailureV1::Malformed(None))?;
    let malformed = || ProtectedIntegrationRpcDecodeFailureV1::Malformed(Some(id.clone()));
    if object.get("_tag").and_then(Value::as_str) != Some("Request")
        || !valid_rpc_headers(object.get("headers"))
        || !valid_optional_string(object.get("traceId"))
        || !valid_optional_string(object.get("spanId"))
        || !valid_optional_bool(object.get("sampled"))
    {
        return Err(malformed());
    }
    if object.get("tag").and_then(Value::as_str) != Some(PROTECTED_INTEGRATION_RPC_TAG) {
        return Err(ProtectedIntegrationRpcDecodeFailureV1::UnknownTag);
    }
    let payload = object
        .get("payload")
        .and_then(Value::as_object)
        .filter(|payload| payload.len() == 1)
        .ok_or_else(malformed)?;
    if payload
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.is_empty() || id.len() > 255 || id.chars().any(char::is_control))
    {
        return Err(malformed());
    }
    Ok(DecodedProtectedIntegrationRpcV1 {
        id,
        payload: Value::Object(payload.clone()),
    })
}

fn valid_rpc_id(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.len() <= 256 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_rpc_headers(value: Option<&Value>) -> bool {
    value.and_then(Value::as_array).is_some_and(|headers| {
        headers.iter().all(|entry| {
            entry.as_array().is_some_and(|pair| {
                pair.len() == 2 && pair.iter().all(|value| value.as_str().is_some())
            })
        })
    })
}

fn valid_optional_string(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_str().is_some())
}

fn valid_optional_bool(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_bool().is_some())
}

fn rpc_success(id: &str, value: Value) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Success","value":value}}])
}

fn rpc_internal_failure(id: &str, error_type: &str) -> Value {
    rpc_typed_failure(id, json!({"_tag":"InternalError","type":error_type}))
}

fn rpc_typed_failure(id: &str, error: Value) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Failure","cause":{"_tag":"Fail","error":error}}}])
}

fn rpc_die(id: &str, message: &str) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Failure","cause":{"_tag":"Die","defect":message}}}])
}

fn rpc_defect(message: &str) -> Value {
    json!([{"_tag":"Defect","defect":message}])
}

fn protected_rpc_response(value: Value, receipt_id: Option<&str>) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(200);
    response
        .headers_mut()
        .set("content-type", "application/json")?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    if let Some(receipt_id) = receipt_id {
        response.headers_mut().set("retry-after", "15")?;
        response
            .headers_mut()
            .set("x-frame-provider-receipt", receipt_id)?;
    }
    Ok(response)
}

async fn authenticate_route(
    profile: &LegacyProtectedIntegrationProfileV1,
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedIntegrationPrincipalV1, Response>> {
    match profile.auth {
        LegacyProtectedIntegrationAuthV1::Public => Ok(Ok(public_principal(profile.auth))),
        LegacyProtectedIntegrationAuthV1::SignedState => {
            signed_state_principal(request, env, now_ms)
        }
        LegacyProtectedIntegrationAuthV1::SignedWebhook => {
            let Some(expected) = env_value(env, "MEDIA_SERVER_WEBHOOK_SECRET") else {
                return Ok(Err(json_status(
                    503,
                    json!({"error":"Provider webhook verification is unavailable"}),
                )?));
            };
            let actual = request.headers().get("x-media-server-secret")?;
            if actual
                .as_deref()
                .is_none_or(|actual| !constant_time_equal(actual, &expected))
            {
                return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
            }
            Ok(Ok(LegacyProtectedIntegrationPrincipalV1 {
                class: LegacyProtectedIntegrationAuthV1::SignedWebhook,
                actor_id: None,
                tenant_id: None,
                credential_kind: LegacyProtectedIntegrationCredentialKindV1::SignedEndpoint,
                credential_subject_id: Some("media-server-webhook.endpoint.v1".into()),
                credential_key_version: Some(1),
                credential_digest: Some(digest(expected.as_bytes())),
                credential_expires_at_ms: None,
                policy_proofs: Vec::new(),
                inherited_entitlement_binding: None,
            }))
        }
        LegacyProtectedIntegrationAuthV1::AnonymousOrSessionOrApiKey => {
            if request
                .headers()
                .get("authorization")?
                .as_deref()
                .and_then(desktop_api_key_selector)
                .is_some()
            {
                required_session_or_api_key_principal(profile.auth, request, env, now_ms).await
            } else {
                match browser_web_runtime::authenticate_host_only_browser_session_binding(
                    request, env, now_ms,
                )
                .await?
                {
                    Ok(binding) => Ok(Ok(session_principal(profile.auth, &binding, None))),
                    Err(BrowserWebFailure::Unavailable) => {
                        Ok(Err(json_status(503, json!({"error":"Unavailable"}))?))
                    }
                    Err(_) => Ok(Ok(public_principal(profile.auth))),
                }
            }
        }
        LegacyProtectedIntegrationAuthV1::Session => {
            required_session_principal(profile.auth, request, env, now_ms).await
        }
        LegacyProtectedIntegrationAuthV1::SessionOrApiKey => {
            required_session_or_api_key_principal(profile.auth, request, env, now_ms).await
        }
        LegacyProtectedIntegrationAuthV1::PublicOrSession => {
            match browser_web_runtime::authenticate_host_only_browser_session_binding(
                request, env, now_ms,
            )
            .await?
            {
                Ok(binding) => Ok(Ok(session_principal(profile.auth, &binding, None))),
                Err(BrowserWebFailure::Unavailable) => {
                    Ok(Err(json_status(503, json!({"error":"Unavailable"}))?))
                }
                Err(_) => Ok(Ok(public_principal(profile.auth))),
            }
        }
        LegacyProtectedIntegrationAuthV1::ParentReceipt => Ok(Err(json_status(
            503,
            json!({"error":"Parent receipt authority is required"}),
        )?)),
    }
}

async fn required_session_principal(
    class: LegacyProtectedIntegrationAuthV1,
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedIntegrationPrincipalV1, Response>> {
    match browser_web_runtime::authenticate_host_only_browser_session_binding(request, env, now_ms)
        .await?
    {
        Ok(binding) => Ok(Ok(session_principal(class, &binding, None))),
        Err(BrowserWebFailure::Unavailable) => {
            Ok(Err(json_status(503, json!({"error":"Unavailable"}))?))
        }
        Err(_) => Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?)),
    }
}

async fn required_session_or_api_key_principal(
    class: LegacyProtectedIntegrationAuthV1,
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedIntegrationPrincipalV1, Response>> {
    let api_key = request
        .headers()
        .get("authorization")?
        .as_deref()
        .and_then(desktop_api_key_selector)
        .map(str::to_owned);
    let Some(api_key) = api_key else {
        return required_session_principal(class, request, env, now_ms).await;
    };
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
    }
    let credential_digest = digest(api_key.as_bytes());
    let result = env
        .d1("DB")?
        .prepare(API_KEY_ACTOR_SQL)
        .bind(&[
            JsValue::from_str(&credential_digest),
            JsValue::from_f64(now_ms as f64),
        ])?
        .all()
        .into_send()
        .await;
    let result = match result {
        Ok(result) if result.success() => result,
        Ok(_) | Err(_) => {
            return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
        }
    };
    let rows = match result.results::<ApiKeyActorRowV1>() {
        Ok(rows) => rows,
        Err(_) => return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?)),
    };
    match rows.as_slice() {
        [row]
            if valid_actor_id(&row.user_id)
                && valid_credential_subject_id(&row.credential_subject_id) =>
        {
            Ok(Ok(LegacyProtectedIntegrationPrincipalV1 {
                class,
                actor_id: Some(row.user_id.clone()),
                tenant_id: None,
                credential_kind: LegacyProtectedIntegrationCredentialKindV1::ApiKey,
                credential_subject_id: Some(row.credential_subject_id.clone()),
                credential_key_version: None,
                credential_digest: Some(credential_digest),
                credential_expires_at_ms: None,
                policy_proofs: Vec::new(),
                inherited_entitlement_binding: None,
            }))
        }
        [] => Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?)),
        _ => Ok(Err(json_status(503, json!({"error":"Unavailable"}))?)),
    }
}

fn desktop_api_key_selector(authorization: &str) -> Option<&str> {
    authorization
        .split(' ')
        .nth(1)
        .filter(|value| value.len() == 36)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoogleDriveOAuthStateV1 {
    user_id: String,
    expires_at: i64,
    scope: String,
    organization_id: Option<String>,
}

fn signed_state_principal(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedIntegrationPrincipalV1, Response>> {
    let state = request
        .url()?
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()));
    let Some(state) = state.filter(|state| state.len() <= 8_192) else {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    };
    let Some((payload, signature)) = state.split_once('.') else {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    };
    if payload.is_empty() || signature.is_empty() || signature.contains('.') {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    }
    let Some(secret) = env_value(env, "NEXTAUTH_SECRET") else {
        return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
    };
    let Some(signature) = base64_url_decode(signature) else {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    };
    let mut verifier =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    verifier.update(payload.as_bytes());
    if verifier.verify_slice(&signature).is_err() {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    }
    let parsed = base64_url_decode(payload)
        .and_then(|bytes| serde_json::from_slice::<GoogleDriveOAuthStateV1>(&bytes).ok());
    let Some(parsed) = parsed.filter(|state| {
        valid_actor_id(&state.user_id)
            && state.expires_at > now_ms
            && state.expires_at <= now_ms.saturating_add(10 * 60 * 1_000)
            && match state.scope.as_str() {
                "user" => state.organization_id.is_none(),
                "organization" => state
                    .organization_id
                    .as_deref()
                    .is_some_and(valid_credential_subject_id),
                _ => false,
            }
    }) else {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    };
    Ok(Ok(LegacyProtectedIntegrationPrincipalV1 {
        class: LegacyProtectedIntegrationAuthV1::SignedState,
        actor_id: Some(parsed.user_id),
        tenant_id: parsed.organization_id,
        credential_kind: LegacyProtectedIntegrationCredentialKindV1::SignedState,
        credential_subject_id: Some("google-drive-oauth-state.v1".into()),
        credential_key_version: Some(1),
        credential_digest: Some(digest(state.as_bytes())),
        credential_expires_at_ms: Some(parsed.expires_at),
        policy_proofs: Vec::new(),
        inherited_entitlement_binding: None,
    }))
}

fn valid_actor_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_credential_subject_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

async fn video_policy_principal(
    request: &Request,
    env: &Env,
    database: &D1Database,
    payload: &Value,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyProtectedIntegrationPrincipalV1>> {
    let mut principal = match browser_web_runtime::authenticate_host_only_browser_session_binding(
        request, env, now_ms,
    )
    .await?
    {
        Ok(binding) => session_principal(
            LegacyProtectedIntegrationAuthV1::PublicOrSession,
            &binding,
            None,
        ),
        Err(BrowserWebFailure::Unavailable) => return Ok(Err(BrowserWebFailure::Unavailable)),
        Err(_) => public_principal(LegacyProtectedIntegrationAuthV1::PublicOrSession),
    };
    let Some(legacy_video_id) = legacy_video_id_from_payload(payload) else {
        return Ok(Err(BrowserWebFailure::Invalid));
    };
    let result = database
        .prepare(VIDEO_POLICY_READ_SQL)
        .bind(&[JsValue::from_str(legacy_video_id)])?
        .all()
        .into_send()
        .await?;
    if !result.success() {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    let rows = match result.results::<VideoPolicyRowV1>() {
        Ok(rows) => rows,
        Err(_) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    let [row] = rows.as_slice() else {
        return Ok(Err(if rows.is_empty() {
            BrowserWebFailure::Forbidden
        } else {
            BrowserWebFailure::Unavailable
        }));
    };
    if !valid_uuid(&row.canonical_video_id)
        || !valid_actor_id(&row.owner_id)
        || !(0..=9_007_199_254_740_991).contains(&row.video_revision)
    {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    let actor_is_owner = principal.actor_id.as_deref() == Some(row.owner_id.as_str());
    let verified_hashes =
        crate::legacy_video_properties_web_runtime::existing_password_hashes(request, env)
            .unwrap_or_default();
    let (kind, subject_id, revision, audit_digest) = if actor_is_owner {
        (
            "owner_bypass",
            row.canonical_video_id.clone(),
            row.video_revision,
            policy_snapshot_digest(
                "owner_bypass",
                &row.canonical_video_id,
                &row.canonical_video_id,
                row.video_revision,
            ),
        )
    } else if let Some(password_hash) = row.video_password_hash.as_deref() {
        if !verified_hashes.iter().any(|value| value == password_hash) {
            return Ok(Err(BrowserWebFailure::Forbidden));
        }
        (
            "video_password",
            row.canonical_video_id.clone(),
            row.video_revision,
            digest(password_hash.as_bytes()),
        )
    } else if let (Some(space_id), Some(revision), Some(password_hash)) = (
        row.protected_space_id.as_deref(),
        row.protected_space_revision,
        row.protected_space_password_hash.as_deref(),
    ) {
        if !valid_uuid(space_id)
            || !(0..=9_007_199_254_740_991).contains(&revision)
            || !verified_hashes.iter().any(|value| value == password_hash)
        {
            return Ok(Err(BrowserWebFailure::Forbidden));
        }
        (
            "space_password",
            space_id.into(),
            revision,
            digest(password_hash.as_bytes()),
        )
    } else if row.protected_space_id.is_some()
        || row.protected_space_revision.is_some()
        || row.protected_space_password_hash.is_some()
    {
        return Ok(Err(BrowserWebFailure::Unavailable));
    } else {
        (
            "unprotected_video_policy",
            row.canonical_video_id.clone(),
            row.video_revision,
            policy_snapshot_digest(
                "unprotected_video_policy",
                &row.canonical_video_id,
                &row.canonical_video_id,
                row.video_revision,
            ),
        )
    };
    principal.policy_proofs = vec![LegacyProtectedIntegrationPolicyProofV1 {
        target_id: row.canonical_video_id.clone(),
        kind: kind.into(),
        subject_id,
        revision,
        audit_digest,
    }];
    Ok(Ok(principal))
}

fn policy_snapshot_digest(kind: &str, target_id: &str, subject_id: &str, revision: i64) -> String {
    let mut material = Vec::new();
    for part in [
        b"frame.protected-integration-video-policy.v1".as_slice(),
        kind.as_bytes(),
        target_id.as_bytes(),
        subject_id.as_bytes(),
        revision.to_string().as_bytes(),
    ] {
        push_digest_part(&mut material, part);
    }
    digest(&material)
}

fn bind_signed_state_organization(
    payload: &mut Value,
    signed_organization_id: Option<&str>,
) -> std::result::Result<(), LegacyProtectedIntegrationFailureV1> {
    let object = payload
        .as_object_mut()
        .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
    if object.contains_key("orgId") {
        return Err(LegacyProtectedIntegrationFailureV1::Invalid);
    }
    if let Some(organization_id) = signed_organization_id {
        object.insert("orgId".into(), Value::String(organization_id.into()));
    }
    Ok(())
}

fn legacy_video_id_from_payload(payload: &Value) -> Option<&str> {
    payload
        .pointer("/videoId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 512)
}

async fn decode_route_payload(
    profile: &LegacyProtectedIntegrationProfileV1,
    request: &mut Request,
) -> std::result::Result<DecodedProtectedIntegrationRouteV1, LegacyProtectedIntegrationFailureV1> {
    let mut object = Map::new();
    let mut transport_body_digest = None;
    for (key, value) in request
        .url()
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?
        .query_pairs()
    {
        object.insert(key.into_owned(), Value::String(value.into_owned()));
    }
    inject_path_parameters(profile, request, &mut object)?;

    if matches!(profile.method, "POST" | "PATCH") {
        let content_type = request
            .headers()
            .get("content-type")
            .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
        if content_type.as_deref().is_some_and(|value| {
            value.starts_with("multipart/form-data")
                || value.starts_with("application/x-www-form-urlencoded")
        }) {
            let content_length = request
                .headers()
                .get("content-length")
                .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|length| *length <= profile.max_body_bytes)
                .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
            if content_length == 0 {
                return Err(LegacyProtectedIntegrationFailureV1::Invalid);
            }
            let form = request
                .form_data()
                .await
                .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
            let fields: &[&str] = if profile.operation_id == "cap-v1-30b7af7323aa2c37" {
                &["feedback", "os", "version"]
            } else if profile.operation_id == "cap-v1-dfbbc4c0b56179d1" {
                &["log", "os", "version", "diagnostics"]
            } else {
                &[]
            };
            if fields.is_empty() {
                return Err(LegacyProtectedIntegrationFailureV1::Invalid);
            }
            for field in fields {
                if let Some(entry) = form.get(field) {
                    match entry {
                        FormEntry::Field(value) => {
                            object.insert((*field).into(), Value::String(value));
                        }
                        FormEntry::File(_) => {
                            return Err(LegacyProtectedIntegrationFailureV1::Invalid);
                        }
                    }
                }
            }
        } else {
            if content_type
                .as_deref()
                .is_some_and(|value| !value.to_ascii_lowercase().starts_with("application/json"))
            {
                return Err(LegacyProtectedIntegrationFailureV1::Invalid);
            }
            let bytes = crate::read_bounded_legacy_body(request, profile.max_body_bytes.max(4_096))
                .await
                .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
            if !bytes.is_empty() {
                transport_body_digest = Some(digest(&bytes));
                let body: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
                let body = body
                    .as_object()
                    .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
                for (key, value) in body {
                    object.insert(key.clone(), value.clone());
                }
            }
        }
    }
    Ok(DecodedProtectedIntegrationRouteV1 {
        payload: Value::Object(object),
        transport_body_digest,
    })
}

fn canonicalize_source_route_payload(
    profile: &LegacyProtectedIntegrationProfileV1,
    request: &Request,
    payload: &mut Value,
) -> std::result::Result<(), LegacyProtectedIntegrationFailureV1> {
    if profile.operation_id != "cap-v1-60f863b2cb19353f" {
        return Ok(());
    }
    let features = request
        .headers()
        .get("x-cap-desktop-features")
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
    let version = request
        .headers()
        .get("x-cap-desktop-version")
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
    canonicalize_f60_payload(payload, features.as_deref(), version.as_deref())
}

fn canonicalize_f60_payload(
    payload: &mut Value,
    desktop_features: Option<&str>,
    desktop_version: Option<&str>,
) -> std::result::Result<(), LegacyProtectedIntegrationFailureV1> {
    let object = payload
        .as_object_mut()
        .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
    const ALLOWED: &[&str] = &[
        "recordingMode",
        "isScreenshot",
        "videoId",
        "name",
        "durationInSecs",
        "width",
        "height",
        "fps",
        "orgId",
        "_frame",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str()))
        || object
            .iter()
            .any(|(key, value)| key != "_frame" && value.as_str().is_none())
    {
        return Err(LegacyProtectedIntegrationFailureV1::Invalid);
    }
    for key in ["durationInSecs", "width", "height", "fps"] {
        if let Some(raw) = object.get(key).and_then(Value::as_str) {
            let number = parse_finite_query_number(raw)
                .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
            object.insert(
                key.into(),
                Value::Number(
                    serde_json::Number::from_f64(number)
                        .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?,
                ),
            );
        }
    }
    let is_screenshot = object
        .get("isScreenshot")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    object.insert("isScreenshot".into(), Value::Bool(is_screenshot));

    let client_supports_google_drive_upload = desktop_features.is_some_and(|features| {
        features
            .split(',')
            .map(str::trim)
            .any(|feature| feature == "googleDriveUpload")
    });
    let client_supports_upload_progress =
        desktop_version.is_some_and(cap_desktop_supports_upload_progress);
    object.insert(
        "_frame".into(),
        json!({
            "clientSupportsGoogleDriveUpload": client_supports_google_drive_upload,
            "clientSupportsUploadProgress": client_supports_upload_progress,
        }),
    );
    Ok(())
}

fn parse_finite_query_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    let number = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok().map(|value| value as f64)
    } else if let Some(binary) = trimmed
        .strip_prefix("0b")
        .or_else(|| trimmed.strip_prefix("0B"))
    {
        u64::from_str_radix(binary, 2)
            .ok()
            .map(|value| value as f64)
    } else if let Some(octal) = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
    {
        u64::from_str_radix(octal, 8).ok().map(|value| value as f64)
    } else {
        trimmed.parse::<f64>().ok()
    }?;
    number.is_finite().then_some(number)
}

fn cap_desktop_supports_upload_progress(raw: &str) -> bool {
    let version = raw.strip_prefix('v').unwrap_or(raw);
    let mut parts = version.splitn(3, '.');
    let major_raw = parts.next().unwrap_or_default();
    let Some(major) = leading_decimal(major_raw).filter(|(_, length)| *length == major_raw.len())
    else {
        return false;
    };
    let Some(minor_raw) = parts.next() else {
        return false;
    };
    let Some(minor) = leading_decimal(minor_raw).filter(|(_, length)| *length == minor_raw.len())
    else {
        return false;
    };
    let Some(patch_tail) = parts.next() else {
        return false;
    };
    let Some(patch) = leading_decimal(patch_tail) else {
        return false;
    };
    let prerelease = patch_tail
        .get(patch.1..)
        .and_then(|tail| tail.strip_prefix('-'))
        .is_some_and(|value| !value.is_empty());
    (major.0, minor.0, patch.0) > (0, 3, 68)
        || ((major.0, minor.0, patch.0) == (0, 3, 68) && !prerelease)
}

fn leading_decimal(value: &str) -> Option<(u64, usize)> {
    let length = value.bytes().take_while(u8::is_ascii_digit).count();
    if length == 0 {
        return None;
    }
    Some((value[..length].parse::<u64>().unwrap_or(u64::MAX), length))
}

fn inject_path_parameters(
    profile: &LegacyProtectedIntegrationProfileV1,
    request: &Request,
    object: &mut Map<String, Value>,
) -> std::result::Result<(), LegacyProtectedIntegrationFailureV1> {
    let url = request
        .url()
        .map_err(|_| LegacyProtectedIntegrationFailureV1::Invalid)?;
    let segments = url
        .path_segments()
        .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?
        .collect::<Vec<_>>();
    if profile.operation_id == "cap-v1-8a1e6c87b4426f93" {
        let tauri = segments
            .iter()
            .position(|segment| *segment == "tauri")
            .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
        for (offset, key) in [(1, "version"), (2, "target"), (3, "arch")] {
            let value = segments
                .get(tauri + offset)
                .filter(|value| !value.is_empty())
                .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
            object.insert(key.into(), Value::String((*value).into()));
        }
    }
    if profile.operation_id == "cap-v1-5af545d5d20508bd" {
        let action = segments
            .last()
            .filter(|value| !value.is_empty())
            .ok_or(LegacyProtectedIntegrationFailureV1::Invalid)?;
        object.insert("action".into(), Value::String((*action).into()));
    }
    Ok(())
}

fn session_principal(
    class: LegacyProtectedIntegrationAuthV1,
    binding: &browser_web_runtime::HostOnlyBrowserSessionBindingV1,
    tenant_id: Option<&str>,
) -> LegacyProtectedIntegrationPrincipalV1 {
    LegacyProtectedIntegrationPrincipalV1 {
        class,
        actor_id: Some(binding.user_id.clone()),
        tenant_id: tenant_id.map(str::to_owned),
        credential_kind: LegacyProtectedIntegrationCredentialKindV1::SessionToken,
        credential_subject_id: Some(binding.session_id.clone()),
        credential_key_version: Some(binding.token_key_version),
        credential_digest: Some(binding.credential_digest.clone()),
        credential_expires_at_ms: None,
        policy_proofs: Vec::new(),
        inherited_entitlement_binding: None,
    }
}

fn parse_credential_kind(value: &str) -> Option<LegacyProtectedIntegrationCredentialKindV1> {
    match value {
        "none" => Some(LegacyProtectedIntegrationCredentialKindV1::None),
        "session_token" => Some(LegacyProtectedIntegrationCredentialKindV1::SessionToken),
        "api_key" => Some(LegacyProtectedIntegrationCredentialKindV1::ApiKey),
        "signed_state" => Some(LegacyProtectedIntegrationCredentialKindV1::SignedState),
        "signed_endpoint" => Some(LegacyProtectedIntegrationCredentialKindV1::SignedEndpoint),
        _ => None,
    }
}

fn public_principal(
    class: LegacyProtectedIntegrationAuthV1,
) -> LegacyProtectedIntegrationPrincipalV1 {
    LegacyProtectedIntegrationPrincipalV1 {
        class,
        actor_id: None,
        tenant_id: None,
        credential_kind: frame_application::LegacyProtectedIntegrationCredentialKindV1::None,
        credential_subject_id: None,
        credential_key_version: None,
        credential_digest: None,
        credential_expires_at_ms: None,
        policy_proofs: Vec::new(),
        inherited_entitlement_binding: None,
    }
}

fn route_rate_subject(
    principal: &LegacyProtectedIntegrationPrincipalV1,
    request: &Request,
) -> Result<String> {
    if let Some(actor_id) = principal.actor_id.as_deref() {
        Ok(format!("actor:{actor_id}"))
    } else if let Some(credential_digest) = principal.credential_digest.as_deref() {
        Ok(format!("credential:{credential_digest}"))
    } else {
        edge_rate_subject(request)
    }
}

fn edge_rate_subject(request: &Request) -> Result<String> {
    let mut material = Vec::new();
    material.extend_from_slice(b"frame.protected-integration-edge-rate.v1\0");
    for name in ["cf-connecting-ip", "user-agent", "accept-language"] {
        material.extend_from_slice(name.as_bytes());
        material.push(0);
        if let Some(value) = request.headers().get(name)? {
            material.extend_from_slice(digest(value.as_bytes()).as_bytes());
        }
        material.push(0xff);
    }
    Ok(format!("edge:{}", digest(&material)))
}

fn seal_provider_request(
    profile: &LegacyProtectedIntegrationProfileV1,
    payload: &Value,
    transport_body_digest: Option<&str>,
    vault: &dyn ProtectedIntegrationRequestVaultV1,
) -> std::result::Result<SealedProtectedIntegrationRequestV1, LegacyProtectedIntegrationFailureV1> {
    let request = ProtectedIntegrationProviderRequestV1 {
        operation_id: profile.operation_id.into(),
        payload: payload.clone(),
        transport_body_digest: transport_body_digest.map(str::to_owned),
    };
    let expected = legacy_protected_integration_plaintext_request_digest(
        profile.operation_id,
        payload,
        transport_body_digest,
    )
    .map_err(LegacyProtectedIntegrationFailureV1::from)?;
    let sealed = vault.seal(&request)?;
    if !valid_opaque_ref("frame-pi-request-v1:", &sealed.opaque_ref)
        || sealed.plaintext_digest != expected
    {
        return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
    }
    Ok(sealed)
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ProtectedIntegrationTerminalV1 {
    Http {
        status: u16,
        location: Option<String>,
        set_cookies: Vec<String>,
        content_type: Option<String>,
        body: Vec<u8>,
    },
    JsonArtifact {
        body: Vec<u8>,
    },
    WorkflowArtifact {
        body: Vec<u8>,
    },
}

impl std::fmt::Debug for ProtectedIntegrationTerminalV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http {
                status,
                location,
                set_cookies,
                content_type,
                body,
            } => formatter
                .debug_struct("ProtectedIntegrationTerminalV1::Http")
                .field("status", status)
                .field("location", &location.as_ref().map(|_| "[REDACTED]"))
                .field("set_cookie_count", &set_cookies.len())
                .field("content_type", content_type)
                .field("body_bytes", &body.len())
                .finish(),
            Self::JsonArtifact { body } => formatter
                .debug_struct("ProtectedIntegrationTerminalV1::JsonArtifact")
                .field("body_bytes", &body.len())
                .finish(),
            Self::WorkflowArtifact { body } => formatter
                .debug_struct("ProtectedIntegrationTerminalV1::WorkflowArtifact")
                .field("body_bytes", &body.len())
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResolvedProtectedIntegrationTerminalV1 {
    pub terminal: ProtectedIntegrationTerminalV1,
    pub plaintext_digest: String,
}

impl std::fmt::Debug for ResolvedProtectedIntegrationTerminalV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedProtectedIntegrationTerminalV1")
            .field("terminal", &self.terminal)
            .field("plaintext_digest", &self.plaintext_digest)
            .finish()
    }
}

/// Secret-bearing provider terminals can enter a carrier only through this
/// resolver. D1 supplies an opaque reference and expected plaintext digest;
/// the resolver decrypts outside D1 and returns a typed HTTP/JSON/workflow
/// artifact.
pub(crate) trait ProtectedIntegrationTerminalResolverV1 {
    fn resolve(
        &self,
        opaque_ref: &str,
        expected_plaintext_digest: &str,
    ) -> std::result::Result<
        Option<ResolvedProtectedIntegrationTerminalV1>,
        LegacyProtectedIntegrationFailureV1,
    >;
}

struct UnavailableProtectedIntegrationTerminalResolverV1;

impl ProtectedIntegrationTerminalResolverV1 for UnavailableProtectedIntegrationTerminalResolverV1 {
    fn resolve(
        &self,
        _opaque_ref: &str,
        _expected_plaintext_digest: &str,
    ) -> std::result::Result<
        Option<ResolvedProtectedIntegrationTerminalV1>,
        LegacyProtectedIntegrationFailureV1,
    > {
        Ok(None)
    }
}

impl ProtectedIntegrationTerminalV1 {
    /// Rebuild the resolver-owned value into the carrier-owned enum. Besides
    /// preventing a resolver implementation from retaining aliases to secret
    /// buffers, this is the single production construction path for every
    /// supported terminal shape.
    fn into_carrier_owned(self) -> Self {
        match self {
            Self::Http {
                status,
                location,
                set_cookies,
                content_type,
                body,
            } => Self::Http {
                status,
                location,
                set_cookies,
                content_type,
                body,
            },
            Self::JsonArtifact { body } => Self::JsonArtifact { body },
            Self::WorkflowArtifact { body } => Self::WorkflowArtifact { body },
        }
    }

    #[must_use]
    pub(crate) fn plaintext_digest(&self) -> String {
        let mut material = Vec::new();
        material.extend_from_slice(b"frame.protected-integration-terminal.v1\0");
        match self {
            Self::Http {
                status,
                location,
                set_cookies,
                content_type,
                body,
            } => {
                push_digest_part(&mut material, b"http");
                push_digest_part(&mut material, status.to_string().as_bytes());
                push_digest_part(
                    &mut material,
                    location.as_deref().unwrap_or_default().as_bytes(),
                );
                push_digest_part(
                    &mut material,
                    content_type.as_deref().unwrap_or_default().as_bytes(),
                );
                for cookie in set_cookies {
                    push_digest_part(&mut material, cookie.as_bytes());
                }
                push_digest_part(&mut material, body);
            }
            Self::JsonArtifact { body } => {
                push_digest_part(&mut material, b"json");
                push_digest_part(&mut material, body);
            }
            Self::WorkflowArtifact { body } => {
                push_digest_part(&mut material, b"workflow");
                push_digest_part(&mut material, body);
            }
        }
        digest(&material)
    }

    fn validate(&self) -> std::result::Result<(), LegacyProtectedIntegrationFailureV1> {
        match self {
            Self::Http {
                status,
                location,
                set_cookies,
                content_type,
                body,
            } => {
                if !(200..=599).contains(status)
                    || body.len() > LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES
                    || set_cookies.len() > 32
                    || content_type.as_deref().is_some_and(|value| {
                        value.is_empty()
                            || value.len() > 255
                            || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
                    })
                    || set_cookies.iter().any(|cookie| {
                        cookie.is_empty()
                            || cookie.len() > 8_192
                            || !cookie.contains('=')
                            || cookie.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
                    })
                {
                    return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
                }
                match (*status, location.as_deref()) {
                    (300..=399, Some(value))
                        if *status != 304
                            && value.len() <= 8_192
                            && !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0)) => {}
                    (304, None) | (200..=299 | 400..=599, None) => {}
                    _ => return Err(LegacyProtectedIntegrationFailureV1::Corrupt),
                }
                if matches!(status, 204 | 205 | 304) && !body.is_empty() {
                    return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
                }
            }
            Self::JsonArtifact { body } | Self::WorkflowArtifact { body } => {
                if body.is_empty()
                    || body.len() > LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES
                    || serde_json::from_slice::<Value>(body).is_err()
                {
                    return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
                }
            }
        }
        Ok(())
    }

    fn into_worker_response(self) -> Result<Response> {
        match self {
            Self::Http {
                status,
                location,
                set_cookies,
                content_type,
                body,
            } => {
                let mut response = if body.is_empty() {
                    Response::empty()?.with_status(status)
                } else {
                    Response::from_body(ResponseBody::Body(body))?.with_status(status)
                };
                if let Some(location) = location {
                    response.headers_mut().set("location", &location)?;
                }
                if let Some(content_type) = content_type {
                    response.headers_mut().set("content-type", &content_type)?;
                }
                for cookie in set_cookies {
                    response.headers_mut().append("set-cookie", &cookie)?;
                }
                harden_terminal_response(&mut response)?;
                Ok(response)
            }
            Self::JsonArtifact { body } => {
                let mut response = Response::from_body(ResponseBody::Body(body))?.with_status(200);
                response
                    .headers_mut()
                    .set("content-type", "application/json")?;
                harden_terminal_response(&mut response)?;
                Ok(response)
            }
            Self::WorkflowArtifact { .. } => {
                failure_response(LegacyProtectedIntegrationFailureV1::Corrupt)
            }
        }
    }
}

fn resolve_terminal(
    sealed_terminal_ref: &str,
    sealed_terminal_digest: &str,
    resolver: &dyn ProtectedIntegrationTerminalResolverV1,
) -> std::result::Result<ProtectedIntegrationTerminalV1, LegacyProtectedIntegrationFailureV1> {
    if !valid_opaque_ref("frame-pi-terminal-v1:", sealed_terminal_ref)
        || !valid_digest(sealed_terminal_digest)
    {
        return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
    }
    let resolved = resolver
        .resolve(sealed_terminal_ref, sealed_terminal_digest)?
        .ok_or(LegacyProtectedIntegrationFailureV1::ProviderEvidenceRequired)?;
    let terminal = resolved.terminal.into_carrier_owned();
    if resolved.plaintext_digest != sealed_terminal_digest
        || terminal.plaintext_digest() != sealed_terminal_digest
    {
        return Err(LegacyProtectedIntegrationFailureV1::Corrupt);
    }
    terminal.validate()?;
    Ok(terminal)
}

fn resolve_terminal_json(
    sealed_terminal_ref: &str,
    sealed_terminal_digest: &str,
    resolver: &dyn ProtectedIntegrationTerminalResolverV1,
) -> std::result::Result<Value, LegacyProtectedIntegrationFailureV1> {
    match resolve_terminal(sealed_terminal_ref, sealed_terminal_digest, resolver)? {
        ProtectedIntegrationTerminalV1::JsonArtifact { body } => {
            serde_json::from_slice(&body).map_err(|_| LegacyProtectedIntegrationFailureV1::Corrupt)
        }
        _ => Err(LegacyProtectedIntegrationFailureV1::Corrupt),
    }
}

async fn stage_response(
    result: std::result::Result<
        LegacyProtectedIntegrationStageOutcomeV1,
        LegacyProtectedIntegrationFailureV1,
    >,
    resolver: &dyn ProtectedIntegrationTerminalResolverV1,
) -> Result<Response> {
    match result {
        Ok(LegacyProtectedIntegrationStageOutcomeV1::ProviderEvidenceRequired {
            receipt_id,
            provider,
            replayed,
        }) => {
            let mut response = json_status(
                503,
                json!({
                    "error":"Provider execution evidence required",
                    "code":"PROVIDER_EXECUTION_REQUIRED",
                    "receiptId":receipt_id,
                    "provider":provider,
                    "replayed":replayed,
                }),
            )?;
            response.headers_mut().set("retry-after", "15")?;
            response
                .headers_mut()
                .set("x-frame-provider-receipt", &receipt_id)?;
            Ok(response)
        }
        Ok(LegacyProtectedIntegrationStageOutcomeV1::VerifiedSealedTerminal {
            sealed_terminal_ref,
            sealed_terminal_digest,
            ..
        }) => match resolve_terminal(&sealed_terminal_ref, &sealed_terminal_digest, resolver) {
            Ok(terminal) => terminal.into_worker_response(),
            Err(failure) => failure_response(failure),
        },
        Err(failure) => failure_response(failure),
    }
}

fn harden_terminal_response(response: &mut Response) -> Result<()> {
    response.headers_mut().set("cache-control", "no-store")?;
    response.headers_mut().set("pragma", "no-cache")?;
    response
        .headers_mut()
        .set("referrer-policy", "no-referrer")?;
    response
        .headers_mut()
        .set("x-content-type-options", "nosniff")?;
    Ok(())
}

fn failure_response(failure: LegacyProtectedIntegrationFailureV1) -> Result<Response> {
    let (status, code) = match failure {
        LegacyProtectedIntegrationFailureV1::Invalid => (400, "INVALID_REQUEST"),
        LegacyProtectedIntegrationFailureV1::Unauthorized => (403, "FORBIDDEN"),
        LegacyProtectedIntegrationFailureV1::Conflict => (409, "IDEMPOTENCY_CONFLICT"),
        LegacyProtectedIntegrationFailureV1::ProviderEvidenceRequired => {
            (503, "PROVIDER_EXECUTION_REQUIRED")
        }
        LegacyProtectedIntegrationFailureV1::Corrupt
        | LegacyProtectedIntegrationFailureV1::Unavailable => (503, "UNAVAILABLE"),
    };
    json_status(status, json!({"error":code,"code":code}))
}

fn json_status(status: u16, value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(status);
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}

fn constant_time_equal(actual: &str, expected: &str) -> bool {
    type HmacSha256 = Hmac<Sha256>;
    let Ok(mut expected_mac) =
        HmacSha256::new_from_slice(b"frame.protected-integrations.compare.v1")
    else {
        return false;
    };
    expected_mac.update(expected.as_bytes());
    let expected_tag = expected_mac.finalize().into_bytes();
    let Ok(mut actual_mac) = HmacSha256::new_from_slice(b"frame.protected-integrations.compare.v1")
    else {
        return false;
    };
    actual_mac.update(actual.as_bytes());
    actual.len() == expected.len() && actual_mac.verify_slice(&expected_tag).is_ok()
}

fn base64_url_encode(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(value.len().div_ceil(3) * 4);
    let mut chunks = value.chunks_exact(3);
    for chunk in &mut chunks {
        output.push(char::from(ALPHABET[usize::from(chunk[0] >> 2)]));
        output.push(char::from(
            ALPHABET[usize::from((chunk[0] & 3) << 4 | chunk[1] >> 4)],
        ));
        output.push(char::from(
            ALPHABET[usize::from((chunk[1] & 15) << 2 | chunk[2] >> 6)],
        ));
        output.push(char::from(ALPHABET[usize::from(chunk[2] & 63)]));
    }
    match chunks.remainder() {
        [one] => {
            output.push(char::from(ALPHABET[usize::from(one >> 2)]));
            output.push(char::from(ALPHABET[usize::from((one & 3) << 4)]));
        }
        [one, two] => {
            output.push(char::from(ALPHABET[usize::from(one >> 2)]));
            output.push(char::from(ALPHABET[usize::from((one & 3) << 4 | two >> 4)]));
            output.push(char::from(ALPHABET[usize::from((two & 15) << 2)]));
        }
        _ => {}
    }
    output
}

fn base64_url_decode(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || value.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3 + 2);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in value.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        accumulator = accumulator << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1_u32 << bits).saturating_sub(1);
        }
    }
    if bits > 0 && accumulator != 0 {
        return None;
    }
    (base64_url_encode(&output) == value).then_some(output)
}

fn env_value(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
        .filter(|value| !value.is_empty())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn push_digest_part(material: &mut Vec<u8>, part: &[u8]) {
    material.extend_from_slice(&(part.len() as u64).to_be_bytes());
    material.extend_from_slice(part);
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

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct RandomizingVault(Cell<u64>);

    impl ProtectedIntegrationRequestVaultV1 for RandomizingVault {
        fn seal(
            &self,
            request: &ProtectedIntegrationProviderRequestV1,
        ) -> std::result::Result<
            SealedProtectedIntegrationRequestV1,
            LegacyProtectedIntegrationFailureV1,
        > {
            let generation = self.0.get() + 1;
            self.0.set(generation);
            Ok(SealedProtectedIntegrationRequestV1 {
                opaque_ref: format!("frame-pi-request-v1:{generation:064x}"),
                plaintext_digest: legacy_protected_integration_plaintext_request_digest(
                    &request.operation_id,
                    &request.payload,
                    request.transport_body_digest.as_deref(),
                )
                .map_err(LegacyProtectedIntegrationFailureV1::from)?,
            })
        }
    }

    struct FixedTerminalResolver(ProtectedIntegrationTerminalV1);

    impl ProtectedIntegrationTerminalResolverV1 for FixedTerminalResolver {
        fn resolve(
            &self,
            _opaque_ref: &str,
            _expected_plaintext_digest: &str,
        ) -> std::result::Result<
            Option<ResolvedProtectedIntegrationTerminalV1>,
            LegacyProtectedIntegrationFailureV1,
        > {
            Ok(Some(ResolvedProtectedIntegrationTerminalV1 {
                terminal: self.0.clone(),
                plaintext_digest: self.0.plaintext_digest(),
            }))
        }
    }

    #[test]
    fn callable_kinds_are_not_interchangeable() {
        let action = legacy_protected_integration_profile("cap-v1-cb3fade2af06d6bd")
            .expect("checked-in action");
        let workflow = legacy_protected_integration_profile("cap-v1-b9fcb0fbd25b2234")
            .expect("checked-in workflow");
        assert_eq!(action.kind, LegacyProtectedIntegrationKindV1::ServerAction);
        assert_eq!(workflow.kind, LegacyProtectedIntegrationKindV1::Workflow);
    }

    #[test]
    fn callable_inventory_preserves_all_four_carriers() {
        let counts = frame_application::LEGACY_PROTECTED_INTEGRATION_PROFILES
            .iter()
            .fold([0_usize; 4], |mut counts, profile| {
                let index = match profile.kind {
                    LegacyProtectedIntegrationKindV1::Route => 0,
                    LegacyProtectedIntegrationKindV1::Rpc => 1,
                    LegacyProtectedIntegrationKindV1::ServerAction => 2,
                    LegacyProtectedIntegrationKindV1::Workflow => 3,
                };
                counts[index] += 1;
                counts
            });
        assert_eq!(counts, [20, 1, 22, 2]);
    }

    #[test]
    fn organisation_soft_delete_rpc_decoder_is_exact() {
        let request = json!({
            "_tag":"Request",
            "id":"42",
            "tag":PROTECTED_INTEGRATION_RPC_TAG,
            "headers":[],
            "payload":{"id":"org-1"},
        });
        let bytes = serde_json::to_vec(&request).expect("encode request");
        let decoded = decode_protected_integration_rpc(&bytes).expect("decode request");
        assert_eq!(decoded.id, "42");
        assert_eq!(decoded.payload, json!({"id":"org-1"}));
        assert!(is_protected_integration_rpc_request(&bytes));

        let mut extra_payload = request;
        extra_payload["payload"]["unexpected"] = json!(true);
        assert!(matches!(
            decode_protected_integration_rpc(
                &serde_json::to_vec(&extra_payload).expect("encode malformed request")
            ),
            Err(ProtectedIntegrationRpcDecodeFailureV1::Malformed(Some(id))) if id == "42"
        ));
        assert!(!is_protected_integration_rpc_request(
            br#"{"_tag":"Request","id":"42","tag":"OrganisationUpdate","headers":[],"payload":{}}"#
        ));
    }

    #[test]
    fn webhook_secret_comparison_is_exact() {
        assert!(constant_time_equal("secret", "secret"));
        assert!(!constant_time_equal("secret", "Secret"));
        assert!(!constant_time_equal("secret-prefix", "secret"));
    }

    #[test]
    fn signed_state_is_the_only_organization_authority() {
        let mut user_scope = json!({"code":"oauth-code","state":"signed-state"});
        bind_signed_state_organization(&mut user_scope, None).expect("user-scoped state");
        assert!(user_scope.get("orgId").is_none());

        let mut organization_scope = json!({"code":"oauth-code","state":"signed-state"});
        bind_signed_state_organization(&mut organization_scope, Some("01h000000000001"))
            .expect("organization-scoped state");
        assert_eq!(organization_scope["orgId"], "01h000000000001");

        let mut attacker_scope =
            json!({"code":"oauth-code","state":"signed-state","orgId":"attacker"});
        assert_eq!(
            bind_signed_state_organization(&mut attacker_scope, None),
            Err(LegacyProtectedIntegrationFailureV1::Invalid)
        );
    }

    #[test]
    fn video_policy_target_is_bounded_before_d1() {
        assert_eq!(
            legacy_video_id_from_payload(&json!({"videoId":"legacy-video"})),
            Some("legacy-video")
        );
        assert!(legacy_video_id_from_payload(&json!({})).is_none());
        assert!(legacy_video_id_from_payload(&json!({"videoId":""})).is_none());
        assert!(legacy_video_id_from_payload(&json!({"videoId":"x".repeat(513)})).is_none());
    }

    #[test]
    fn f60_query_and_header_inputs_are_canonical_before_sealing() {
        let mut payload = json!({
            "isScreenshot":"false",
            "durationInSecs":"300.5",
            "width":"",
            "videoId":"01h000000000004",
            "_frame":"attacker-controlled",
        });
        canonicalize_f60_payload(
            &mut payload,
            Some("other, googleDriveUpload"),
            Some("v0.3.68"),
        )
        .expect("source-valid query");
        assert_eq!(payload["isScreenshot"], true);
        assert_eq!(payload["durationInSecs"], 300.5);
        assert_eq!(payload["width"], 0.0);
        assert_eq!(payload["_frame"]["clientSupportsGoogleDriveUpload"], true);
        assert_eq!(payload["_frame"]["clientSupportsUploadProgress"], true);

        let mut omitted = json!({});
        canonicalize_f60_payload(&mut omitted, None, Some("0.3.68-beta"))
            .expect("all source query values are optional");
        assert_eq!(omitted["isScreenshot"], false);
        assert_eq!(omitted["_frame"]["clientSupportsGoogleDriveUpload"], false);
        assert_eq!(omitted["_frame"]["clientSupportsUploadProgress"], false);

        for mut invalid in [
            json!({"durationInSecs":"not-a-number"}),
            json!({"durationInSecs":"Infinity"}),
            json!({"unexpected":"value"}),
        ] {
            assert_eq!(
                canonicalize_f60_payload(&mut invalid, None, None),
                Err(LegacyProtectedIntegrationFailureV1::Invalid)
            );
        }

        let profile =
            legacy_protected_integration_profile("cap-v1-60f863b2cb19353f").expect("f60 profile");
        let mut headers_off = json!({});
        canonicalize_f60_payload(&mut headers_off, None, Some("0.3.67"))
            .expect("headers-off request");
        let mut headers_on = json!({});
        canonicalize_f60_payload(&mut headers_on, Some("googleDriveUpload"), Some("0.3.68"))
            .expect("headers-on request");
        let first_digest = legacy_protected_integration_plaintext_request_digest(
            profile.operation_id,
            &headers_off,
            None,
        )
        .expect("first canonical request");
        let changed_digest = legacy_protected_integration_plaintext_request_digest(
            profile.operation_id,
            &headers_on,
            None,
        )
        .expect("second canonical request");
        assert_ne!(first_digest, changed_digest);
    }

    #[test]
    fn released_carriers_do_not_depend_on_invented_headers() {
        let source = include_str!("legacy_protected_integrations_web_runtime.rs");
        let idempotency_header = ["idempotency", "key"].join("-");
        let client_sealed_header = ["x", "frame", "sealed", "payload", "ref"].join("-");
        let unverified_flow_header = ["x", "frame", "flow", "token"].join("-");
        assert!(!source.contains(&idempotency_header));
        assert!(!source.contains(&client_sealed_header));
        assert!(!source.contains(&unverified_flow_header));
    }

    #[test]
    fn server_vault_randomizes_refs_but_binds_exact_plaintext() {
        let profile =
            legacy_protected_integration_profile("cap-v1-9d91d42d52472a83").expect("S3 profile");
        let payload = json!({
            "provider":"aws",
            "accessKeyId":"secret-access",
            "secretAccessKey":"secret-key",
            "bucketName":"frame",
            "region":"us-east-1",
        });
        let vault = RandomizingVault(Cell::new(0));
        let first =
            seal_provider_request(profile, &payload, None, &vault).expect("first vault seal");
        let second =
            seal_provider_request(profile, &payload, None, &vault).expect("randomized retry seal");
        assert_ne!(first.opaque_ref, second.opaque_ref);
        assert_eq!(first.plaintext_digest, second.plaintext_digest);
        assert!(valid_opaque_ref("frame-pi-request-v1:", &first.opaque_ref));
        assert!(!format!("{payload:?}").is_empty());
        let request = ProtectedIntegrationProviderRequestV1 {
            operation_id: profile.operation_id.into(),
            payload,
            transport_body_digest: None,
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("secret-access"));
        assert!(!debug.contains("secret-key"));
    }

    #[test]
    fn typed_terminal_is_resolved_only_by_opaque_digest_binding() {
        let http = ProtectedIntegrationTerminalV1::Http {
            status: 302,
            location: Some("https://provider.example/opaque-destination".into()),
            set_cookies: Vec::new(),
            content_type: None,
            body: Vec::new(),
        };
        http.validate().expect("typed redirect is valid");
        let workflow = ProtectedIntegrationTerminalV1::WorkflowArtifact {
            body: br#"{"completed":true}"#.to_vec(),
        };
        workflow
            .validate()
            .expect("typed workflow artifact is valid");
        let terminal = ProtectedIntegrationTerminalV1::JsonArtifact {
            body: br#"{"downloadUrl":"https://provider.example/signed-secret"}"#.to_vec(),
        };
        let digest = terminal.plaintext_digest();
        let reference = format!("frame-pi-terminal-v1:{}", "a".repeat(64));
        let resolver = FixedTerminalResolver(terminal.clone());
        let value = resolve_terminal_json(&reference, &digest, &resolver)
            .expect("trusted resolver projects typed JSON");
        assert_eq!(
            value.pointer("/downloadUrl").and_then(Value::as_str),
            Some("https://provider.example/signed-secret")
        );
        assert!(!format!("{terminal:?}").contains("signed-secret"));
        assert_eq!(
            resolve_terminal_json(&reference, &"0".repeat(64), &resolver),
            Err(LegacyProtectedIntegrationFailureV1::Corrupt)
        );
        assert_eq!(
            resolve_terminal_json("https://provider.example/token", &digest, &resolver),
            Err(LegacyProtectedIntegrationFailureV1::Corrupt)
        );
    }
}
