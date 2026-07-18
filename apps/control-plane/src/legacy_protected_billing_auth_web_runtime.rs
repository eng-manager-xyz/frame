//! Callable HTTP, server-action, and workflow carriers for the final protected
//! authentication/billing compatibility contracts.
//!
//! Route authentication and transport checks happen before D1 staging. Stripe
//! webhooks are verified over their exact raw body with timestamp tolerance.
//! Every unverified operation returns a fail-closed evidence gate.

use frame_application::{
    LEGACY_PROTECTED_BILLING_AUTH_MAX_BODY_BYTES, LegacyProtectedBillingAuthAuthV1,
    LegacyProtectedBillingAuthCredentialKindV1, LegacyProtectedBillingAuthEnvelopeV1,
    LegacyProtectedBillingAuthIdempotencyV1, LegacyProtectedBillingAuthKindV1,
    LegacyProtectedBillingAuthPrincipalV1, LegacyProtectedBillingAuthProfileV1,
    LegacyProtectedBillingAuthReplayOriginV1, RateLimitDecisionV1, ValidatedBrowserMutationProof,
    legacy_protected_billing_auth_profile,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Request, Response, ResponseBody, Result, send::IntoSendFuture};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1, RETRY_AFTER_SECONDS},
    legacy_protected_billing_auth_runtime::{
        D1LegacyProtectedBillingAuthRuntimeV1, LegacyProtectedBillingAuthFailureV1,
        LegacyProtectedBillingAuthStageOutcomeV1,
    },
};

const STRIPE_SIGNATURE_TOLERANCE_SECONDS: i64 = 300;
const LEGACY_ROUTE_REPLAY_WINDOW_MS: i64 = 5 * 60 * 1_000;
const API_KEY_ACTOR_SQL: &str =
    include_str!("../queries/legacy_org_custom_domain/api_key_actor.sql");
const VERIFIED_STRIPE_WEBHOOK_PRINCIPAL: &[u8] = b"frame.verified-stripe-webhook.endpoint.v1";
const VERIFIED_STRIPE_WEBHOOK_RATE_SUBJECT: &str = "verified-stripe-webhook.endpoint.v1";
const MAX_TERMINAL_HTTP_BODY_BYTES: usize = 1_048_576;
const MAX_TERMINAL_HTTP_SET_COOKIES: usize = 32;

#[derive(Debug, Deserialize)]
struct ApiKeyActorRowV1 {
    credential_subject_id: String,
    user_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecodedProtectedBillingAuthActionV1 {
    operation_id: String,
    idempotency_key: String,
    payload: Value,
}

pub async fn route_response(
    operation_id: &str,
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let Some(profile) = legacy_protected_billing_auth_profile(operation_id) else {
        return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
    };
    let mut response = route_response_for_profile(profile, request, env, now_ms).await?;
    if profile.path == "/api/developer/credits/checkout" {
        add_developer_checkout_cors(&mut response, request, env)?;
    }
    Ok(response)
}

async fn route_response_for_profile(
    profile: &LegacyProtectedBillingAuthProfileV1,
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actual_path = match request.url() {
        Ok(url) => url.path().to_owned(),
        Err(_) => return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid),
    };
    if profile.kind != LegacyProtectedBillingAuthKindV1::Route
        || request.method().to_string() != profile.method
        || !route_path_matches(profile.path, &actual_path)
    {
        return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
    }
    if profile.operation_id == "cap-v1-572763e7b4977abd" {
        // Cap registers credentialed CORS before `withAuth`; Hono terminates a
        // valid OPTIONS preflight before the authentication/provider handler.
        // Keep that local terminal behavior out of the protected D1 outbox.
        let mut response = Response::empty()?.with_status(204);
        response.headers_mut().set("cache-control", "no-store")?;
        response.headers_mut().set("content-length", "0")?;
        return Ok(response);
    }
    if profile.auth == LegacyProtectedBillingAuthAuthV1::SignedWebhook {
        match compatibility_rate_limit::admit_edge_request(
            env,
            request,
            CompatibilityRateLimitBucketV1::StripeWebhookIngress,
            now_ms,
        )
        .await
        {
            Ok(RateLimitDecisionV1::Allowed) => {}
            Ok(RateLimitDecisionV1::Rejected { .. }) => return rate_limited_response(),
            Err(_) => {
                return failure_response(LegacyProtectedBillingAuthFailureV1::Unavailable);
            }
        }
    }
    let supplied_caller_key = request.headers().get("idempotency-key")?;
    if supplied_caller_key
        .as_deref()
        .is_some_and(|key| key.trim().is_empty() || key.len() > 512)
        || (profile.idempotency == LegacyProtectedBillingAuthIdempotencyV1::Forbidden
            && supplied_caller_key.is_some())
        || (profile.idempotency == LegacyProtectedBillingAuthIdempotencyV1::Required
            && profile.auth != LegacyProtectedBillingAuthAuthV1::SignedWebhook
            && supplied_caller_key.is_none())
    {
        return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
    };

    let decoded = match decode_route_payload(profile, request).await {
        Ok(decoded) => decoded,
        Err(failure) => return failure_response(failure),
    };
    let (principal, natural_replay_key, transport_credential_digest) = match profile.auth {
        LegacyProtectedBillingAuthAuthV1::Anonymous => {
            let credential_digest = match edge_cookie_flow_principal_digest(request) {
                Ok(digest) => digest,
                Err(failure) => return failure_response(failure),
            };
            (
                LegacyProtectedBillingAuthPrincipalV1 {
                    class: LegacyProtectedBillingAuthAuthV1::Anonymous,
                    actor_id: None,
                    credential_kind: LegacyProtectedBillingAuthCredentialKindV1::PublicFlow,
                    credential_subject_id: None,
                    credential_key_version: None,
                    credential_digest: Some(credential_digest),
                },
                None,
                None,
            )
        }
        LegacyProtectedBillingAuthAuthV1::SignedWebhook => {
            let Some(raw_body) = decoded.raw_body.as_deref() else {
                return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
            };
            let (principal, delivery_signature_digest) =
                match authenticate_stripe_webhook(request, env, now_ms, raw_body) {
                    Ok(verified) => verified,
                    Err(failure) => return failure_response(failure),
                };
            let Some(event_id) = decoded
                .payload
                .pointer("/id")
                .and_then(Value::as_str)
                .filter(|event_id| !event_id.is_empty() && event_id.len() <= 255)
            else {
                return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
            };
            if request
                .headers()
                .get("idempotency-key")?
                .is_some_and(|key| key != event_id)
            {
                return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
            }
            (
                principal,
                Some(event_id.to_owned()),
                Some(delivery_signature_digest),
            )
        }
        LegacyProtectedBillingAuthAuthV1::PublicOrFlowToken => {
            let principal = match public_flow_principal(request, env, &decoded.payload)? {
                Ok(principal) => principal,
                Err(response) => return Ok(response),
            };
            (principal, None, None)
        }
        LegacyProtectedBillingAuthAuthV1::Session
        | LegacyProtectedBillingAuthAuthV1::AdminSession => {
            let principal =
                match required_session_principal(profile.auth, request, env, now_ms).await? {
                    Ok(principal) => principal,
                    Err(response) => return Ok(response),
                };
            (principal, None, None)
        }
        LegacyProtectedBillingAuthAuthV1::SessionOrApiKey => {
            let principal =
                match required_session_or_api_key_principal(request, env, now_ms).await? {
                    Ok(principal) => principal,
                    Err(response) => return Ok(response),
                };
            (principal, None, None)
        }
    };

    let database = env.d1("DB")?;
    let admission = match profile.auth {
        LegacyProtectedBillingAuthAuthV1::SignedWebhook => {
            let Some(event_id) = natural_replay_key.as_deref() else {
                return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
            };
            compatibility_rate_limit::admit_principal(
                env,
                &database,
                CompatibilityRateLimitBucketV1::StripeWebhookIngress,
                verified_stripe_webhook_rate_subject(event_id),
                now_ms,
            )
            .await
        }
        LegacyProtectedBillingAuthAuthV1::Anonymous
        | LegacyProtectedBillingAuthAuthV1::PublicOrFlowToken => {
            let bucket = if profile.rate_limit_bucket == "auth_session.v1" {
                CompatibilityRateLimitBucketV1::AuthSession
            } else {
                CompatibilityRateLimitBucketV1::BillingAdmin
            };
            compatibility_rate_limit::admit_edge_request(env, request, bucket, now_ms).await
        }
        LegacyProtectedBillingAuthAuthV1::Session
        | LegacyProtectedBillingAuthAuthV1::SessionOrApiKey
        | LegacyProtectedBillingAuthAuthV1::AdminSession => {
            let Some(subject) = principal.actor_id.as_deref() else {
                return failure_response(LegacyProtectedBillingAuthFailureV1::Unauthorized);
            };
            compatibility_rate_limit::admit_principal(
                env,
                &database,
                CompatibilityRateLimitBucketV1::BillingAdmin,
                subject,
                now_ms,
            )
            .await
        }
    };
    match admission {
        Ok(RateLimitDecisionV1::Allowed) => {}
        Ok(RateLimitDecisionV1::Rejected { .. }) => return rate_limited_response(),
        Err(_) => return failure_response(LegacyProtectedBillingAuthFailureV1::Unavailable),
    }

    let (caller_idempotency_key, replay_origin, request_nonce) = if let Some(key) =
        natural_replay_key
    {
        (
            Some(key),
            LegacyProtectedBillingAuthReplayOriginV1::Natural,
            Uuid::new_v4().to_string(),
        )
    } else if let Some(key) = supplied_caller_key {
        (
            Some(key),
            LegacyProtectedBillingAuthReplayOriginV1::Caller,
            Uuid::new_v4().to_string(),
        )
    } else if profile.idempotency != LegacyProtectedBillingAuthIdempotencyV1::Required {
        let key = match bounded_route_replay_key(profile, request, &principal, &decoded, now_ms) {
            Ok(key) => key,
            Err(failure) => return failure_response(failure),
        };
        (
            None,
            LegacyProtectedBillingAuthReplayOriginV1::Generated,
            key,
        )
    } else {
        return failure_response(LegacyProtectedBillingAuthFailureV1::Invalid);
    };
    let sealed_request = match seal_nextauth_request(
        profile,
        request,
        &decoded,
        &UnavailableProtectedRequestVaultV1,
    ) {
        Ok(sealed) => sealed,
        Err(failure) => return failure_response(failure),
    };
    let envelope = LegacyProtectedBillingAuthEnvelopeV1 {
        source_operation_id: profile.operation_id.into(),
        principal,
        caller_idempotency_key,
        replay_origin,
        request_nonce,
        payload: decoded.payload,
        sealed_request_ref: sealed_request
            .as_ref()
            .map(|sealed| sealed.opaque_ref.clone()),
        sealed_request_digest: sealed_request.map(|sealed| sealed.plaintext_digest),
        transport_body_digest: decoded.raw_body.as_deref().map(digest),
        transport_credential_digest,
    };
    let result = D1LegacyProtectedBillingAuthRuntimeV1::new(&database)
        .stage(profile, &envelope, now_ms)
        .await;
    stage_response(result, &UnavailableTerminalHttpResponseResolverV1).await
}

fn verified_stripe_webhook_rate_subject(_event_id: &str) -> &'static str {
    VERIFIED_STRIPE_WEBHOOK_RATE_SUBJECT
}

/// Return whether an operation ID belongs to one of the five source-pinned
/// authenticated billing/administrator actions. The shared compatibility
/// dispatcher uses this exact profile check before decoding any request body.
#[must_use]
pub(crate) fn is_server_action(operation_id: &str) -> bool {
    legacy_protected_billing_auth_profile(operation_id)
        .is_some_and(|profile| profile.kind == LegacyProtectedBillingAuthKindV1::ServerAction)
}

/// Decode the authenticated compatibility-action bridge while retaining the
/// caller-supplied replay key. Authentication and one-use CSRF grant
/// consumption happen in `server_action_http_response` after this bounded
/// transport decode succeeds.
pub(crate) async fn decode_server_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedProtectedBillingAuthActionV1>> {
    let Some(profile) = legacy_protected_billing_auth_profile(operation_id)
        .filter(|profile| profile.kind == LegacyProtectedBillingAuthKindV1::ServerAction)
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
    let idempotency_key = match request.headers().get("idempotency-key")? {
        Some(value) if !value.trim().is_empty() && value.len() <= 512 => value,
        _ => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    let declared = match request.headers().get("content-length")? {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
        },
        None => None,
    };
    let max_body_bytes = profile
        .max_body_bytes
        .clamp(4_096, LEGACY_PROTECTED_BILLING_AUTH_MAX_BODY_BYTES);
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
    Ok(Ok(DecodedProtectedBillingAuthActionV1 {
        operation_id: operation_id.into(),
        idempotency_key,
        payload,
    }))
}

/// Authenticate the browser mutation proof, rate-limit the authenticated
/// principal, and stage the fail-closed billing/administrator intent while
/// consuming the one-use CSRF grant in that same D1 transaction.
pub(crate) async fn server_action_http_response(
    request: &Request,
    env: &Env,
    decoded: &DecodedProtectedBillingAuthActionV1,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Response>> {
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let database = env.d1("DB")?;
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
    let admission = match compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::BillingAdmin,
        &actor_id,
        now_ms,
    )
    .await
    {
        Ok(admission) => admission,
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
    if matches!(admission, RateLimitDecisionV1::Rejected { .. }) {
        if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(
            &database, &proof,
        )
        .await?
        {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let outcome = server_action_response(
        &decoded.operation_id,
        &database,
        &proof,
        &binding,
        &decoded.idempotency_key,
        decoded.payload.clone(),
        now_ms,
    )
    .await;
    // The protected runtime consumes the proof in the receipt transaction for
    // a new intent and in its exact-replay transaction for a prior intent. Any
    // validation, authority, conflict, or storage failure before those batches
    // is still an attempted browser mutation, so consume the grant here (or
    // prove that the inner transaction already did) before returning.
    if !browser_web_runtime::consume_attempted_session_grant_or_confirm_absent(&database, &proof)
        .await?
    {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    Ok(Ok(stage_response(
        outcome,
        &UnavailableTerminalHttpResponseResolverV1,
    )
    .await?))
}

/// Exact server-action carrier. The actor can only come from the opaque proof
/// produced by the shared browser boundary; the runtime consumes that proof's
/// one-use grant atomically with protected intent staging.
async fn server_action_response(
    operation_id: &str,
    database: &D1Database,
    proof: &ValidatedBrowserMutationProof,
    binding: &browser_web_runtime::HostOnlyBrowserSessionBindingV1,
    idempotency_key: &str,
    payload: Value,
    now_ms: i64,
) -> std::result::Result<
    LegacyProtectedBillingAuthStageOutcomeV1,
    LegacyProtectedBillingAuthFailureV1,
> {
    let profile = legacy_protected_billing_auth_profile(operation_id)
        .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?;
    if profile.kind != LegacyProtectedBillingAuthKindV1::ServerAction
        || profile.idempotency != LegacyProtectedBillingAuthIdempotencyV1::Required
        || !matches!(
            profile.auth,
            LegacyProtectedBillingAuthAuthV1::Session
                | LegacyProtectedBillingAuthAuthV1::AdminSession
        )
    {
        return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
    }
    let envelope = LegacyProtectedBillingAuthEnvelopeV1 {
        source_operation_id: operation_id.into(),
        principal: LegacyProtectedBillingAuthPrincipalV1 {
            class: profile.auth,
            actor_id: Some(binding.user_id.clone()),
            credential_kind: LegacyProtectedBillingAuthCredentialKindV1::SessionToken,
            credential_subject_id: Some(binding.session_id.clone()),
            credential_key_version: Some(binding.token_key_version),
            credential_digest: Some(binding.credential_digest.clone()),
        },
        caller_idempotency_key: Some(idempotency_key.into()),
        replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Caller,
        request_nonce: Uuid::new_v4().to_string(),
        payload: normalize_action_payload(operation_id, payload)?,
        sealed_request_ref: None,
        sealed_request_digest: None,
        transport_body_digest: None,
        transport_credential_digest: None,
    };
    D1LegacyProtectedBillingAuthRuntimeV1::new(database)
        .stage_with_browser_proof(profile, &envelope, Some(proof), now_ms)
        .await
}

/// Exact workflow carrier. The caller supplies no actor, replay key, or video
/// payload; those values are reloaded and transactionally reasserted from the
/// exact administrator action receipt.
pub(crate) async fn workflow_response(
    operation_id: &str,
    database: &D1Database,
    parent_receipt_id: &str,
    parent_request_digest: &str,
    now_ms: i64,
) -> std::result::Result<
    LegacyProtectedBillingAuthStageOutcomeV1,
    LegacyProtectedBillingAuthFailureV1,
> {
    let profile = legacy_protected_billing_auth_profile(operation_id)
        .filter(|profile| profile.kind == LegacyProtectedBillingAuthKindV1::Workflow)
        .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?;
    D1LegacyProtectedBillingAuthRuntimeV1::new(database)
        .stage_workflow_from_parent(profile, parent_receipt_id, parent_request_digest, now_ms)
        .await
}

struct DecodedRoutePayload {
    payload: Value,
    raw_body: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProtectedNextAuthRequestV1 {
    method: String,
    url: String,
    content_type: Option<String>,
    cookie_header: Option<String>,
    body: Vec<u8>,
}

impl std::fmt::Debug for ProtectedNextAuthRequestV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProtectedNextAuthRequestV1")
            .field("method", &self.method)
            .field("url", &"[REDACTED]")
            .field("content_type", &self.content_type)
            .field(
                "cookie_header",
                &self.cookie_header.as_ref().map(|_| "[REDACTED]"),
            )
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SealedProtectedRequestV1 {
    pub opaque_ref: String,
    pub plaintext_digest: String,
}

/// The only interface allowed to receive the exact secret-bearing NextAuth
/// transport. Implementations must seal it outside D1 and return an opaque
/// reference; callers verify the deterministic plaintext digest themselves.
pub(crate) trait ProtectedRequestVaultV1 {
    fn seal(
        &self,
        request: &ProtectedNextAuthRequestV1,
    ) -> std::result::Result<SealedProtectedRequestV1, LegacyProtectedBillingAuthFailureV1>;
}

struct UnavailableProtectedRequestVaultV1;

impl ProtectedRequestVaultV1 for UnavailableProtectedRequestVaultV1 {
    fn seal(
        &self,
        _request: &ProtectedNextAuthRequestV1,
    ) -> std::result::Result<SealedProtectedRequestV1, LegacyProtectedBillingAuthFailureV1> {
        Err(LegacyProtectedBillingAuthFailureV1::Unavailable)
    }
}

impl ProtectedNextAuthRequestV1 {
    fn plaintext_digest(&self) -> String {
        let mut material = Vec::new();
        material.extend_from_slice(b"frame.protected-nextauth-request.v1\0");
        push_digest_part(&mut material, self.method.as_bytes());
        push_digest_part(&mut material, self.url.as_bytes());
        push_digest_part(
            &mut material,
            self.content_type.as_deref().unwrap_or_default().as_bytes(),
        );
        push_digest_part(
            &mut material,
            self.cookie_header.as_deref().unwrap_or_default().as_bytes(),
        );
        push_digest_part(&mut material, &self.body);
        digest(&material)
    }
}

fn seal_nextauth_request(
    profile: &LegacyProtectedBillingAuthProfileV1,
    request: &Request,
    decoded: &DecodedRoutePayload,
    vault: &dyn ProtectedRequestVaultV1,
) -> std::result::Result<Option<SealedProtectedRequestV1>, LegacyProtectedBillingAuthFailureV1> {
    if profile.path != "/api/auth/:nextauth*" {
        return Ok(None);
    }
    let url = request
        .url()
        .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?;
    let protected = ProtectedNextAuthRequestV1 {
        method: request.method().to_string(),
        url: url.as_str().to_owned(),
        content_type: request
            .headers()
            .get("content-type")
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?,
        cookie_header: request
            .headers()
            .get("cookie")
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?,
        body: decoded.raw_body.clone().unwrap_or_default(),
    };
    let expected_digest = protected.plaintext_digest();
    let sealed = vault.seal(&protected)?;
    if !valid_opaque_ref("frame-pba-request-v1:", &sealed.opaque_ref)
        || sealed.plaintext_digest != expected_digest
    {
        return Err(LegacyProtectedBillingAuthFailureV1::Corrupt);
    }
    Ok(Some(sealed))
}

fn edge_cookie_flow_principal_digest(
    request: &Request,
) -> std::result::Result<String, LegacyProtectedBillingAuthFailureV1> {
    let mut material = Vec::new();
    material.extend_from_slice(b"frame.edge-cookie-flow-principal.v1\0");
    for name in [
        "cf-connecting-ip",
        "cookie",
        "user-agent",
        "accept-language",
        "sec-ch-ua",
        "sec-ch-ua-mobile",
        "sec-ch-ua-platform",
        "origin",
    ] {
        let value = request
            .headers()
            .get(name)
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?;
        material.extend_from_slice(name.as_bytes());
        material.push(0);
        if let Some(value) = value {
            material.extend_from_slice(digest(value.as_bytes()).as_bytes());
        }
        material.push(0xff);
    }
    Ok(digest(&material))
}

fn bounded_route_replay_key(
    profile: &LegacyProtectedBillingAuthProfileV1,
    request: &Request,
    principal: &LegacyProtectedBillingAuthPrincipalV1,
    decoded: &DecodedRoutePayload,
    now_ms: i64,
) -> std::result::Result<String, LegacyProtectedBillingAuthFailureV1> {
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
    }
    let payload_digest = match decoded.raw_body.as_deref() {
        Some(raw_body) => digest(raw_body),
        None => digest(
            &serde_json::to_vec(&decoded.payload)
                .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?,
        ),
    };
    let principal_digest = digest(
        &serde_json::to_vec(principal).map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?,
    );
    let flow_principal_digest = edge_cookie_flow_principal_digest(request)?;
    let window = now_ms / LEGACY_ROUTE_REPLAY_WINDOW_MS;
    let material = format!(
        "frame.legacy-cap-route-replay.v1\0{}\0{principal_digest}\0{payload_digest}\0{}\0{window}",
        profile.operation_id, flow_principal_digest,
    );
    Ok(format!(
        "legacy-window-v1:{window}:{}",
        digest(material.as_bytes())
    ))
}

fn route_path_matches(profile_path: &str, actual_path: &str) -> bool {
    if profile_path != "/api/auth/:nextauth*" {
        return profile_path == actual_path;
    }
    let Some(suffix) = actual_path.strip_prefix("/api/auth/") else {
        return false;
    };
    !suffix.is_empty()
        && suffix.len() <= 256
        && !suffix.contains("..")
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_'))
        && matches!(
            suffix.split('/').next().unwrap_or_default(),
            "callback"
                | "csrf"
                | "error"
                | "providers"
                | "session"
                | "signin"
                | "signout"
                | "verify-request"
        )
}

async fn decode_route_payload(
    profile: &LegacyProtectedBillingAuthProfileV1,
    request: &mut Request,
) -> std::result::Result<DecodedRoutePayload, LegacyProtectedBillingAuthFailureV1> {
    let url = request
        .url()
        .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?;
    let mut object = Map::new();
    for (key, value) in url.query_pairs() {
        object.insert(key.into_owned(), Value::String(value.into_owned()));
    }

    let should_read_body = profile.max_body_bytes > 0 && profile.method == "POST";
    let raw_body = if should_read_body {
        if request
            .headers()
            .get("content-encoding")
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?
            .is_some_and(|encoding| encoding != "identity")
        {
            return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
        }
        let declared = match request
            .headers()
            .get("content-length")
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?
        {
            Some(value) => Some(
                value
                    .parse::<usize>()
                    .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?,
            ),
            None => None,
        };
        let max_body_bytes = profile
            .max_body_bytes
            .min(LEGACY_PROTECTED_BILLING_AUTH_MAX_BODY_BYTES);
        if declared.is_some_and(|value| value > max_body_bytes) {
            return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
        }
        let bytes = crate::read_bounded_legacy_body(request, max_body_bytes)
            .await
            .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?;
        if declared.is_some_and(|value| value != bytes.len()) {
            return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
        }
        if !bytes.is_empty() {
            let content_type = request
                .headers()
                .get("content-type")
                .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?
                .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?;
            let media_type = content_type
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if !profile
                .accepted_content_types
                .iter()
                .any(|accepted| media_type == accepted.to_ascii_lowercase())
            {
                return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
            }
            if media_type == "application/json" {
                let body: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?;
                let body = body
                    .as_object()
                    .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?;
                for (key, value) in body {
                    object.insert(key.clone(), value.clone());
                }
            } else if media_type == "application/x-www-form-urlencoded"
                && profile.path == "/api/auth/:nextauth*"
            {
                for (key, value) in url::form_urlencoded::parse(&bytes) {
                    if key.is_empty()
                        || key.len() > 128
                        || value.len() > 8_192
                        || object.contains_key(key.as_ref())
                    {
                        return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
                    }
                    object.insert(key.into_owned(), Value::String(value.into_owned()));
                }
            } else {
                return Err(LegacyProtectedBillingAuthFailureV1::Invalid);
            }
        }
        Some(bytes)
    } else {
        None
    };
    if profile.path == "/api/auth/:nextauth*" {
        object.insert("nextauthPath".into(), Value::String(url.path().to_owned()));
    }
    Ok(DecodedRoutePayload {
        payload: Value::Object(object),
        raw_body,
    })
}

async fn required_session_principal(
    class: LegacyProtectedBillingAuthAuthV1,
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedBillingAuthPrincipalV1, Response>> {
    match browser_web_runtime::authenticate_host_only_browser_session_binding(request, env, now_ms)
        .await?
    {
        Ok(binding) => Ok(Ok(LegacyProtectedBillingAuthPrincipalV1 {
            class,
            actor_id: Some(binding.user_id),
            credential_kind: LegacyProtectedBillingAuthCredentialKindV1::SessionToken,
            credential_subject_id: Some(binding.session_id),
            credential_key_version: Some(binding.token_key_version),
            credential_digest: Some(binding.credential_digest),
        })),
        Err(BrowserWebFailure::Unauthenticated) => {
            Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?))
        }
        Err(_) => Ok(Err(json_status(503, json!({"error":"Unavailable"}))?)),
    }
}

async fn required_session_or_api_key_principal(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedBillingAuthPrincipalV1, Response>> {
    let api_key = request
        .headers()
        .get("authorization")?
        .as_deref()
        .and_then(desktop_api_key_selector)
        .map(str::to_owned);
    if let Some(api_key) = api_key {
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
            Err(_) => {
                return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
            }
        };
        return match rows.as_slice() {
            [row]
                if valid_actor_id(&row.user_id)
                    && valid_credential_subject_id(&row.credential_subject_id) =>
            {
                Ok(Ok(LegacyProtectedBillingAuthPrincipalV1 {
                    class: LegacyProtectedBillingAuthAuthV1::SessionOrApiKey,
                    actor_id: Some(row.user_id.clone()),
                    credential_kind: LegacyProtectedBillingAuthCredentialKindV1::ApiKey,
                    credential_subject_id: Some(row.credential_subject_id.clone()),
                    credential_key_version: None,
                    credential_digest: Some(credential_digest),
                }))
            }
            [] => Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?)),
            _ => Ok(Err(json_status(503, json!({"error":"Unavailable"}))?)),
        };
    }

    required_session_principal(
        LegacyProtectedBillingAuthAuthV1::SessionOrApiKey,
        request,
        env,
        now_ms,
    )
    .await
}

fn desktop_api_key_selector(authorization: &str) -> Option<&str> {
    authorization
        .split(' ')
        .nth(1)
        .filter(|value| value.len() == 36)
}

fn public_flow_principal(
    request: &Request,
    env: &Env,
    payload: &Value,
) -> Result<std::result::Result<LegacyProtectedBillingAuthPrincipalV1, Response>> {
    let actual = request.headers().get("x-frame-auth-flow-token")?;
    let token_digest = if let Some(actual) = actual {
        let Some(expected) = env_value(env, "FRAME_AUTH_FLOW_TOKEN") else {
            return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
        };
        if !constant_time_equal(&actual, &expected) {
            return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
        }
        Some(digest(actual.as_bytes()))
    } else {
        None
    };
    let flow_digest = match nextauth_flow_digest(request, payload)? {
        Ok(digest) => digest,
        Err(response) => return Ok(Err(response)),
    };
    let credential_digest = match token_digest {
        Some(token) => digest(format!("{token}\0{flow_digest}").as_bytes()),
        None => flow_digest,
    };
    Ok(Ok(LegacyProtectedBillingAuthPrincipalV1 {
        class: LegacyProtectedBillingAuthAuthV1::PublicOrFlowToken,
        actor_id: None,
        credential_kind: LegacyProtectedBillingAuthCredentialKindV1::PublicFlow,
        credential_subject_id: None,
        credential_key_version: None,
        credential_digest: Some(credential_digest),
    }))
}

fn nextauth_flow_digest(
    request: &Request,
    payload: &Value,
) -> Result<std::result::Result<String, Response>> {
    let method = request.method().to_string();
    let path = payload
        .pointer("/nextauthPath")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let cookies = nextauth_flow_cookies(request.headers().get("cookie")?.as_deref());
    let Some(bound_flow_digest) = bound_nextauth_flow_digest(&method, path, payload, &cookies)
    else {
        return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
    };
    let edge_digest = match edge_cookie_flow_principal_digest(request) {
        Ok(digest) => digest,
        Err(_) => return Ok(Err(json_status(400, json!({"error":"Invalid request"}))?)),
    };
    Ok(Ok(digest(
        format!("{bound_flow_digest}\0{edge_digest}").as_bytes(),
    )))
}

fn bound_nextauth_flow_digest(
    method: &str,
    path: &str,
    payload: &Value,
    cookies: &[(String, String)],
) -> Option<String> {
    let head = path
        .strip_prefix("/api/auth/")
        .and_then(|suffix| suffix.split('/').next())
        .unwrap_or_default();
    if !matches!(method, "GET" | "POST") {
        return None;
    }
    let csrf_token = payload
        .pointer("/csrfToken")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 512);
    if method == "POST" {
        if let Some(csrf_token) = csrf_token {
            let csrf_cookie = cookies.iter().find_map(|(name, value)| {
                (name.ends_with("next-auth.csrf-token") || name.ends_with("authjs.csrf-token"))
                    .then_some(value)
            });
            if csrf_cookie
                .and_then(|value| value.split('|').next())
                .is_none_or(|cookie_token| !constant_time_equal(csrf_token, cookie_token))
            {
                return None;
            }
        } else if head != "callback" {
            return None;
        }
    }
    let callback_has_state = ["/state", "/code", "/id_token", "/token", "/error"]
        .iter()
        .any(|pointer| payload.pointer(pointer).is_some());
    let invalid_callback = head == "callback" && (!callback_has_state || cookies.is_empty());
    if invalid_callback {
        return None;
    }

    let mut material = Vec::new();
    material.extend_from_slice(b"frame.nextauth-flow.v1\0");
    push_digest_part(&mut material, method.as_bytes());
    push_digest_part(&mut material, path.as_bytes());
    push_digest_part(&mut material, &serde_json::to_vec(payload).ok()?);
    for (name, value) in cookies {
        push_digest_part(&mut material, name.as_bytes());
        push_digest_part(&mut material, value.as_bytes());
    }
    Some(digest(&material))
}

fn nextauth_flow_cookies(header: Option<&str>) -> Vec<(String, String)> {
    let mut cookies = header
        .into_iter()
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            let normalized = name
                .trim_start_matches("__Host-")
                .trim_start_matches("__Secure-");
            if !(normalized.starts_with("next-auth.") || normalized.starts_with("authjs.")) {
                return None;
            }
            let encoded = format!("value={value}");
            let decoded = url::form_urlencoded::parse(encoded.as_bytes())
                .next()
                .map(|(_, value)| value.into_owned())?;
            Some((name.to_owned(), decoded))
        })
        .collect::<Vec<_>>();
    cookies.sort_unstable();
    cookies
}

fn valid_actor_id(actor_id: &str) -> bool {
    !actor_id.is_empty()
        && actor_id.len() <= 255
        && actor_id.is_ascii()
        && !actor_id.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_credential_subject_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn authenticate_stripe_webhook(
    request: &Request,
    env: &Env,
    now_ms: i64,
    raw_body: &[u8],
) -> std::result::Result<
    (LegacyProtectedBillingAuthPrincipalV1, String),
    LegacyProtectedBillingAuthFailureV1,
> {
    let secret = env_value(env, "STRIPE_WEBHOOK_SECRET")
        .ok_or(LegacyProtectedBillingAuthFailureV1::Unavailable)?;
    let signature_header = request
        .headers()
        .get("stripe-signature")
        .map_err(|_| LegacyProtectedBillingAuthFailureV1::Invalid)?
        .ok_or(LegacyProtectedBillingAuthFailureV1::Unauthorized)?;
    if !stripe_signature_valid(&secret, &signature_header, now_ms, raw_body) {
        return Err(LegacyProtectedBillingAuthFailureV1::Unauthorized);
    }
    Ok((
        LegacyProtectedBillingAuthPrincipalV1 {
            class: LegacyProtectedBillingAuthAuthV1::SignedWebhook,
            actor_id: None,
            credential_kind: LegacyProtectedBillingAuthCredentialKindV1::SignedEndpoint,
            credential_subject_id: Some("stripe-webhook.endpoint.v1".into()),
            credential_key_version: None,
            // Stripe re-signs each delivery. Keep the successfully verified
            // endpoint identity stable so retries converge on the event replay key.
            credential_digest: Some(digest(VERIFIED_STRIPE_WEBHOOK_PRINCIPAL)),
        },
        digest(signature_header.as_bytes()),
    ))
}

fn stripe_signature_valid(
    secret: &str,
    signature_header: &str,
    now_ms: i64,
    raw_body: &[u8],
) -> bool {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for component in signature_header.split(',') {
        let Some((key, value)) = component.trim().split_once('=') else {
            continue;
        };
        match key {
            "t" => timestamp = value.parse::<i64>().ok(),
            "v1" if value.len() == 64 => signatures.push(value),
            _ => {}
        }
    }
    let Some(timestamp) = timestamp else {
        return false;
    };
    let now_seconds = now_ms / 1_000;
    if timestamp > now_seconds + STRIPE_SIGNATURE_TOLERANCE_SECONDS
        || now_seconds.saturating_sub(timestamp) > STRIPE_SIGNATURE_TOLERANCE_SECONDS
    {
        return false;
    }
    let mut signed = timestamp.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(raw_body);
    signatures.iter().any(|signature| {
        decode_lower_hex(signature).is_some_and(|tag| {
            let mut verification = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                .expect("HMAC accepts any key length");
            verification.update(&signed);
            verification.verify_slice(&tag).is_ok()
        })
    })
}

fn normalize_action_payload(
    operation_id: &str,
    mut payload: Value,
) -> std::result::Result<Value, LegacyProtectedBillingAuthFailureV1> {
    if operation_id != "cap-v1-14ea978608dcf07e" || payload.pointer("/videoId").is_some() {
        return Ok(payload);
    }
    let input = payload
        .pointer("/input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|input| !input.is_empty())
        .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?;
    let video_id = if let Ok(url) = Url::parse(input) {
        let segments = url
            .path_segments()
            .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if let Some(share) = segments.windows(2).find(|window| window[0] == "s") {
            share[1].to_owned()
        } else {
            segments
                .last()
                .copied()
                .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?
                .to_owned()
        }
    } else {
        input.to_owned()
    };
    let object = payload
        .as_object_mut()
        .ok_or(LegacyProtectedBillingAuthFailureV1::Invalid)?;
    object.remove("input");
    object.insert("videoId".into(), Value::String(video_id));
    Ok(payload)
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProtectedTerminalHttpResponseV1 {
    pub status: u16,
    pub location: Option<String>,
    pub set_cookies: Vec<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for ProtectedTerminalHttpResponseV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProtectedTerminalHttpResponseV1")
            .field("status", &self.status)
            .field("location", &self.location.as_ref().map(|_| "[REDACTED]"))
            .field("set_cookie_count", &self.set_cookies.len())
            .field("content_type", &self.content_type)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResolvedProtectedTerminalHttpResponseV1 {
    pub response: ProtectedTerminalHttpResponseV1,
    pub plaintext_digest: String,
}

impl std::fmt::Debug for ResolvedProtectedTerminalHttpResponseV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedProtectedTerminalHttpResponseV1")
            .field("response", &self.response)
            .field("plaintext_digest", &self.plaintext_digest)
            .finish()
    }
}

/// Secret-bearing terminal transport can enter the HTTP adapter only through
/// this resolver. D1 supplies an opaque reference and expected digest; a
/// trusted implementation decrypts outside D1 and returns a typed response.
pub(crate) trait ProtectedTerminalHttpResponseResolverV1 {
    fn resolve(
        &self,
        opaque_ref: &str,
        expected_plaintext_digest: &str,
    ) -> std::result::Result<
        Option<ResolvedProtectedTerminalHttpResponseV1>,
        LegacyProtectedBillingAuthFailureV1,
    >;
}

struct UnavailableTerminalHttpResponseResolverV1;

impl ProtectedTerminalHttpResponseResolverV1 for UnavailableTerminalHttpResponseResolverV1 {
    fn resolve(
        &self,
        _opaque_ref: &str,
        _expected_plaintext_digest: &str,
    ) -> std::result::Result<
        Option<ResolvedProtectedTerminalHttpResponseV1>,
        LegacyProtectedBillingAuthFailureV1,
    > {
        Ok(None)
    }
}

impl ProtectedTerminalHttpResponseV1 {
    #[must_use]
    pub(crate) fn plaintext_digest(&self) -> String {
        let mut material = Vec::new();
        material.extend_from_slice(b"frame.protected-terminal-http-response.v1\0");
        push_digest_part(&mut material, self.status.to_string().as_bytes());
        push_digest_part(
            &mut material,
            self.location.as_deref().unwrap_or_default().as_bytes(),
        );
        push_digest_part(
            &mut material,
            self.content_type.as_deref().unwrap_or_default().as_bytes(),
        );
        for cookie in &self.set_cookies {
            push_digest_part(&mut material, cookie.as_bytes());
        }
        push_digest_part(&mut material, &self.body);
        digest(&material)
    }

    fn validate(&self) -> std::result::Result<(), LegacyProtectedBillingAuthFailureV1> {
        if !(200..=599).contains(&self.status)
            || self.body.len() > MAX_TERMINAL_HTTP_BODY_BYTES
            || self.set_cookies.len() > MAX_TERMINAL_HTTP_SET_COOKIES
            || self.content_type.as_deref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > 255
                    || value
                        .bytes()
                        .any(|byte| byte.is_ascii_control() && byte != b'\t')
            })
            || self.set_cookies.iter().any(|cookie| {
                cookie.is_empty()
                    || cookie.len() > 8_192
                    || !cookie.contains('=')
                    || cookie.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
            })
        {
            return Err(LegacyProtectedBillingAuthFailureV1::Corrupt);
        }
        match (self.status, self.location.as_deref()) {
            (300..=399, Some(location))
                if self.status != 304
                    && location.len() <= 8_192
                    && !location
                        .bytes()
                        .any(|byte| matches!(byte, b'\r' | b'\n' | 0)) => {}
            (300..=399, None) if self.status == 304 => {}
            (200..=299 | 400..=599, None) => {}
            _ => return Err(LegacyProtectedBillingAuthFailureV1::Corrupt),
        }
        if matches!(self.status, 204 | 205 | 304) && !self.body.is_empty() {
            return Err(LegacyProtectedBillingAuthFailureV1::Corrupt);
        }
        Ok(())
    }

    fn into_worker_response(self) -> Result<Response> {
        let mut response = if self.body.is_empty() {
            Response::empty()?.with_status(self.status)
        } else {
            Response::from_body(ResponseBody::Body(self.body))?.with_status(self.status)
        };
        if let Some(location) = self.location {
            response.headers_mut().set("location", &location)?;
        }
        if let Some(content_type) = self.content_type {
            response.headers_mut().set("content-type", &content_type)?;
        }
        for cookie in self.set_cookies {
            response.headers_mut().append("set-cookie", &cookie)?;
        }
        response.headers_mut().set("cache-control", "no-store")?;
        response.headers_mut().set("pragma", "no-cache")?;
        response
            .headers_mut()
            .set("referrer-policy", "no-referrer")?;
        response
            .headers_mut()
            .set("x-content-type-options", "nosniff")?;
        Ok(response)
    }
}

fn resolve_sealed_terminal_http_response(
    sealed_response_ref: &str,
    sealed_response_digest: &str,
    resolver: &dyn ProtectedTerminalHttpResponseResolverV1,
) -> std::result::Result<ProtectedTerminalHttpResponseV1, LegacyProtectedBillingAuthFailureV1> {
    if !valid_opaque_ref("frame-pba-http-v1:", sealed_response_ref) {
        return Err(LegacyProtectedBillingAuthFailureV1::Corrupt);
    }
    let resolved = resolver
        .resolve(sealed_response_ref, sealed_response_digest)?
        .ok_or(LegacyProtectedBillingAuthFailureV1::EvidenceRequired)?;
    if resolved.plaintext_digest != sealed_response_digest
        || resolved.response.plaintext_digest() != sealed_response_digest
    {
        return Err(LegacyProtectedBillingAuthFailureV1::Corrupt);
    }
    resolved.response.validate()?;
    Ok(resolved.response)
}

async fn stage_response(
    outcome: std::result::Result<
        LegacyProtectedBillingAuthStageOutcomeV1,
        LegacyProtectedBillingAuthFailureV1,
    >,
    resolver: &dyn ProtectedTerminalHttpResponseResolverV1,
) -> Result<Response> {
    match outcome {
        Ok(LegacyProtectedBillingAuthStageOutcomeV1::EvidenceRequired {
            receipt_id,
            provider,
            replayed,
            human_approval_required,
            provider_execution_required,
        }) => {
            let mut response = json_status(
                503,
                json!({
                    "error":"Protected execution evidence required",
                    "code":"PROTECTED_EXECUTION_EVIDENCE_REQUIRED",
                    "receiptId":receipt_id,
                    "provider":provider,
                    "replayed":replayed,
                    "requiredEvidence":{
                        "humanApproval":human_approval_required,
                        "providerExecution":provider_execution_required,
                    },
                }),
            )?;
            response.headers_mut().set("retry-after", "15")?;
            response
                .headers_mut()
                .set("x-frame-protected-receipt", &receipt_id)?;
            Ok(response)
        }
        Ok(LegacyProtectedBillingAuthStageOutcomeV1::VerifiedSealedHttp {
            sealed_response_ref,
            sealed_response_digest,
            ..
        }) => match resolve_sealed_terminal_http_response(
            &sealed_response_ref,
            &sealed_response_digest,
            resolver,
        ) {
            Ok(response) => response.into_worker_response(),
            Err(failure) => failure_response(failure),
        },
        Err(failure) => failure_response(failure),
    }
}

fn failure_response(failure: LegacyProtectedBillingAuthFailureV1) -> Result<Response> {
    let (status, code) = match failure {
        LegacyProtectedBillingAuthFailureV1::Invalid => (400, "INVALID_REQUEST"),
        LegacyProtectedBillingAuthFailureV1::Unauthorized => (401, "UNAUTHORIZED"),
        LegacyProtectedBillingAuthFailureV1::Conflict => (409, "IDEMPOTENCY_CONFLICT"),
        LegacyProtectedBillingAuthFailureV1::HumanApprovalRejected => {
            (409, "HUMAN_APPROVAL_REJECTED")
        }
        LegacyProtectedBillingAuthFailureV1::EvidenceRequired => {
            (503, "PROTECTED_EXECUTION_EVIDENCE_REQUIRED")
        }
        LegacyProtectedBillingAuthFailureV1::Corrupt
        | LegacyProtectedBillingAuthFailureV1::Unavailable => (503, "UNAVAILABLE"),
    };
    json_status(status, json!({"error":code,"code":code}))
}

pub(crate) fn add_developer_checkout_cors(
    response: &mut Response,
    request: &Request,
    env: &Env,
) -> Result<()> {
    let origin = request.headers().get("origin")?;
    let configured_origin = env_value(env, "WEB_URL")
        .or_else(|| env_value(env, "NEXT_PUBLIC_WEB_URL"))
        .and_then(|value| normalized_origin(&value));
    if origin.as_deref().is_some_and(|origin| {
        developer_checkout_origin_allowed(origin, configured_origin.as_deref())
    }) {
        response.headers_mut().set(
            "access-control-allow-origin",
            origin.as_deref().unwrap_or_default(),
        )?;
        response
            .headers_mut()
            .set("access-control-allow-credentials", "true")?;
    }
    response.headers_mut().set("vary", "Origin")?;
    if request.method().to_string() == "OPTIONS" {
        response.headers_mut().set(
            "access-control-allow-methods",
            "GET,POST,PATCH,DELETE,OPTIONS",
        )?;
        response.headers_mut().set(
            "access-control-allow-headers",
            "Content-Type,Authorization,sentry-trace,baggage",
        )?;
    }
    Ok(())
}

fn normalized_origin(value: &str) -> Option<String> {
    if value == "tauri://localhost" {
        return Some(value.into());
    }
    let url = Url::parse(value).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "http" | "https" | "tauri")
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn developer_checkout_origin_allowed(origin: &str, configured_origin: Option<&str>) -> bool {
    normalized_origin(origin).as_deref() == Some(origin)
        && (configured_origin == Some(origin)
            || matches!(
                origin,
                "http://localhost:3001"
                    | "http://localhost:3000"
                    | "tauri://localhost"
                    | "http://tauri.localhost"
                    | "https://tauri.localhost"
            ))
}

fn rate_limited_response() -> Result<Response> {
    let mut response = json_status(429, json!({"error":"RATE_LIMITED","code":"RATE_LIMITED"}))?;
    response
        .headers_mut()
        .set("retry-after", &RETRY_AFTER_SECONDS.to_string())?;
    Ok(response)
}

fn json_status(status: u16, value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(status);
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}

fn constant_time_equal(actual: &str, expected: &str) -> bool {
    let key = b"frame.protected-billing-auth.compare.v1";
    let mut expected_mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    expected_mac.update(expected.as_bytes());
    let expected_tag = expected_mac.finalize().into_bytes();
    let mut actual_mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    actual_mac.update(actual.as_bytes());
    actual.len() == expected.len() && actual_mac.verify_slice(&expected_tag).is_ok()
}

fn decode_lower_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = (chunk[0] as char).to_digit(16)?;
            let low = (chunk[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
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

fn push_digest_part(material: &mut Vec<u8>, value: &[u8]) {
    material.extend_from_slice(&value.len().to_be_bytes());
    material.extend_from_slice(value);
}

fn valid_opaque_ref(prefix: &str, value: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

#[cfg(test)]
fn encode_lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct RandomizingRequestVault(Cell<u8>);

    impl ProtectedRequestVaultV1 for RandomizingRequestVault {
        fn seal(
            &self,
            request: &ProtectedNextAuthRequestV1,
        ) -> std::result::Result<SealedProtectedRequestV1, LegacyProtectedBillingAuthFailureV1>
        {
            let generation = self.0.get().saturating_add(1);
            self.0.set(generation);
            Ok(SealedProtectedRequestV1 {
                opaque_ref: format!("frame-pba-request-v1:{generation:064x}"),
                plaintext_digest: request.plaintext_digest(),
            })
        }
    }

    struct FixedTerminalResolver(ProtectedTerminalHttpResponseV1);

    impl ProtectedTerminalHttpResponseResolverV1 for FixedTerminalResolver {
        fn resolve(
            &self,
            _opaque_ref: &str,
            _expected_plaintext_digest: &str,
        ) -> std::result::Result<
            Option<ResolvedProtectedTerminalHttpResponseV1>,
            LegacyProtectedBillingAuthFailureV1,
        > {
            Ok(Some(ResolvedProtectedTerminalHttpResponseV1 {
                plaintext_digest: self.0.plaintext_digest(),
                response: self.0.clone(),
            }))
        }
    }

    #[test]
    fn stripe_signature_hex_is_strict() {
        assert_eq!(decode_lower_hex("00ff"), Some(vec![0, 255]));
        assert_eq!(decode_lower_hex("00FF"), None);
        assert_eq!(decode_lower_hex("xyz"), None);
    }

    #[test]
    fn stripe_signature_binds_timestamp_and_exact_raw_body() {
        let secret = "whsec_test";
        let timestamp = 1_700_000_000_i64;
        let body = br#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let mut signed = timestamp.to_string().into_bytes();
        signed.push(b'.');
        signed.extend_from_slice(body);
        let mut signer =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
        signer.update(&signed);
        let signature = signer.finalize().into_bytes();
        let header = format!("t={timestamp},v1={}", encode_lower_hex(&signature));
        assert!(stripe_signature_valid(
            secret,
            &header,
            timestamp * 1_000,
            body
        ));
        assert!(!stripe_signature_valid(
            secret,
            &header,
            timestamp * 1_000,
            b"{}"
        ));
        assert!(!stripe_signature_valid(
            secret,
            &header,
            (timestamp + STRIPE_SIGNATURE_TOLERANCE_SECONDS + 1) * 1_000,
            body
        ));
    }

    #[test]
    fn stripe_principal_is_stable_while_delivery_signatures_remain_distinct() {
        let stable = digest(VERIFIED_STRIPE_WEBHOOK_PRINCIPAL);
        assert_eq!(stable, digest(VERIFIED_STRIPE_WEBHOOK_PRINCIPAL));
        assert_ne!(digest(b"t=1,v1=aaaa"), digest(b"t=2,v1=bbbb"));
    }

    #[test]
    fn distinct_stripe_event_ids_share_one_endpoint_capacity_subject() {
        let subjects = (0..250)
            .map(|index| verified_stripe_webhook_rate_subject(&format!("evt_{index}")))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(subjects, [VERIFIED_STRIPE_WEBHOOK_RATE_SUBJECT].into());
    }

    #[test]
    fn sealed_nextauth_request_is_deterministic_without_exposing_secrets() {
        let request = ProtectedNextAuthRequestV1 {
            method: "POST".into(),
            url: "https://frame.engmanager.xyz/api/auth/callback/google?code=oauth-code-secret&state=state-secret".into(),
            content_type: Some("application/x-www-form-urlencoded".into()),
            cookie_header: Some("__Secure-next-auth.pkce.code_verifier=pkce-cookie-secret".into()),
            body: b"csrfToken=csrf-secret&code=oauth-code-secret".to_vec(),
        };
        let vault = RandomizingRequestVault(Cell::new(0));
        let first = vault.seal(&request).expect("first seal");
        let second = vault.seal(&request).expect("randomized retry seal");
        assert_ne!(first.opaque_ref, second.opaque_ref);
        assert_eq!(first.plaintext_digest, second.plaintext_digest);
        assert!(valid_opaque_ref("frame-pba-request-v1:", &first.opaque_ref));
        let debug = format!("{request:?} {first:?} {second:?}");
        for secret in [
            "oauth-code-secret",
            "state-secret",
            "pkce-cookie-secret",
            "csrf-secret",
        ] {
            assert!(!debug.contains(secret), "request secret leaked via Debug");
        }
    }

    #[test]
    fn sealed_terminal_replays_redirect_and_ordered_cookies_without_debug_leaks() {
        let response = ProtectedTerminalHttpResponseV1 {
            status: 302,
            location: Some(
                "https://checkout.stripe.example/session?client_secret=checkout-url-secret".into(),
            ),
            set_cookies: vec![
                "__Secure-next-auth.session-token=session-cookie-secret; Path=/; HttpOnly".into(),
                "__Host-next-auth.csrf-token=csrf-cookie-secret; Path=/; Secure".into(),
            ],
            content_type: Some("text/html; charset=utf-8".into()),
            body: b"provider-token-secret presigned-url-secret X-Amz-Signature=signature-secret"
                .to_vec(),
        };
        let expected_digest = response.plaintext_digest();
        let resolver = FixedTerminalResolver(response.clone());
        let reference = format!("frame-pba-http-v1:{}", "a".repeat(64));
        let projected =
            resolve_sealed_terminal_http_response(&reference, &expected_digest, &resolver)
                .expect("trusted resolver must project exact typed HTTP response");
        assert_eq!(projected.status, 302);
        assert_eq!(projected.location, response.location);
        assert_eq!(projected.set_cookies, response.set_cookies);
        let debug = format!(
            "{response:?} {:?}",
            ResolvedProtectedTerminalHttpResponseV1 {
                response: response.clone(),
                plaintext_digest: expected_digest.clone(),
            }
        );
        for secret in [
            "checkout-url-secret",
            "session-cookie-secret",
            "csrf-cookie-secret",
            "provider-token-secret",
            "presigned-url-secret",
            "signature-secret",
        ] {
            assert!(!debug.contains(secret), "terminal secret leaked via Debug");
        }
        assert_eq!(
            resolve_sealed_terminal_http_response(&reference, &"0".repeat(64), &resolver),
            Err(LegacyProtectedBillingAuthFailureV1::Corrupt)
        );
    }

    #[test]
    fn released_cap_api_key_selector_precedes_session_only_for_36_char_tokens() {
        let key = "12345678-1234-1234-1234-123456789012";
        assert_eq!(
            desktop_api_key_selector(&format!("Bearer {key}")),
            Some(key)
        );
        assert_eq!(
            desktop_api_key_selector(&format!("Anything {key}")),
            Some(key)
        );
        assert_eq!(desktop_api_key_selector(&format!("Bearer  {key}")), None);
        assert_eq!(desktop_api_key_selector("Bearer short"), None);
    }

    #[test]
    fn nextauth_form_flow_binds_decoded_csrf_and_callback_cookies() {
        let cookies = nextauth_flow_cookies(Some(
            "ordinary=ignored; __Host-next-auth.csrf-token=csrf-value%7Ccookie-hash; __Secure-next-auth.callback-url=https%3A%2F%2Fframe.engmanager.xyz%2Fdashboard",
        ));
        assert_eq!(cookies.len(), 2);
        assert!(
            cookies
                .iter()
                .any(|(_, value)| value == "csrf-value|cookie-hash")
        );
        let signin = json!({
            "nextauthPath":"/api/auth/signin/email",
            "csrfToken":"csrf-value",
            "callbackUrl":"https://frame.engmanager.xyz/dashboard",
            "email":"person@example.test"
        });
        let first = bound_nextauth_flow_digest("POST", "/api/auth/signin/email", &signin, &cookies)
            .expect("matching CSRF cookie must bind the form flow");
        assert_eq!(first.len(), 64);
        assert_eq!(
            Some(first),
            bound_nextauth_flow_digest("POST", "/api/auth/signin/email", &signin, &cookies)
        );
        let mismatch = json!({
            "nextauthPath":"/api/auth/signin/email",
            "csrfToken":"different",
            "callbackUrl":"https://frame.engmanager.xyz/dashboard"
        });
        assert!(
            bound_nextauth_flow_digest("POST", "/api/auth/signin/email", &mismatch, &cookies)
                .is_none()
        );

        let callback = json!({
            "nextauthPath":"/api/auth/callback/google",
            "state":"state-1",
            "code":"code-1"
        });
        let callback_a = vec![("__Secure-next-auth.state".into(), "cookie-a".into())];
        let callback_b = vec![("__Secure-next-auth.state".into(), "cookie-b".into())];
        let digest_a =
            bound_nextauth_flow_digest("GET", "/api/auth/callback/google", &callback, &callback_a)
                .expect("callback state and cookie must bind");
        let digest_b =
            bound_nextauth_flow_digest("GET", "/api/auth/callback/google", &callback, &callback_b)
                .expect("second callback cookie must bind");
        assert_ne!(digest_a, digest_b);
        assert!(
            bound_nextauth_flow_digest("GET", "/api/auth/callback/google", &callback, &[])
                .is_none()
        );
    }

    #[test]
    fn reprocess_share_url_is_canonicalized_to_video_id() {
        let payload = normalize_action_payload(
            "cap-v1-14ea978608dcf07e",
            json!({"input":"https://cap.so/s/video-123?x=1"}),
        )
        .expect("valid Cap share URL must normalize to a video identifier");
        assert_eq!(payload, json!({"videoId":"video-123"}));
    }

    #[test]
    fn nextauth_path_binding_accepts_only_the_pinned_wildcard_heads() {
        for path in [
            "/api/auth/session",
            "/api/auth/callback/google",
            "/api/auth/verify-request",
        ] {
            assert!(route_path_matches("/api/auth/:nextauth*", path), "{path}");
        }
        for path in [
            "/api/auth",
            "/api/auth/",
            "/api/auth/unknown",
            "/api/auth/callback/../session",
            "/api/auth/callback/foo..bar",
            "/api/auth/callback/foo.bar",
            "/api/auth/callback/$provider",
            "/api/settings/billing/manage",
        ] {
            assert!(!route_path_matches("/api/auth/:nextauth*", path), "{path}");
        }
        assert!(route_path_matches(
            "/api/settings/billing/manage",
            "/api/settings/billing/manage"
        ));
        assert!(!route_path_matches(
            "/api/settings/billing/manage",
            "/api/settings/billing/manage/extra"
        ));
    }

    #[test]
    fn protected_action_inventory_is_exact() {
        let action_ids = [
            "cap-v1-90a6eb69c3fd7b4b",
            "cap-v1-e488991f97723847",
            "cap-v1-14ea978608dcf07e",
            "cap-v1-dfd7a4c3d234ccd7",
            "cap-v1-0553f2fcdacfe2a9",
        ];
        assert_eq!(
            frame_application::LEGACY_PROTECTED_BILLING_AUTH_PROFILES
                .iter()
                .filter(|profile| profile.kind == LegacyProtectedBillingAuthKindV1::ServerAction)
                .count(),
            action_ids.len()
        );
        for operation_id in action_ids {
            assert!(is_server_action(operation_id), "{operation_id}");
        }
        for operation_id in [
            "cap-v1-46bda1c18ffba076",
            "cap-v1-5a990f470c701cec",
            "cap-v1-b9fcb0fbd25b2234",
        ] {
            assert!(!is_server_action(operation_id), "{operation_id}");
        }
    }

    #[test]
    fn developer_checkout_cors_is_credentialed_and_allowlisted() {
        for origin in [
            "https://frame.engmanager.xyz",
            "http://localhost:3000",
            "http://localhost:3001",
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
        ] {
            assert!(
                developer_checkout_origin_allowed(origin, Some("https://frame.engmanager.xyz")),
                "{origin}"
            );
        }
        for origin in [
            "https://evil.example",
            "https://frame.engmanager.xyz.evil.example",
            "https://frame.engmanager.xyz/",
            "null",
        ] {
            assert!(
                !developer_checkout_origin_allowed(origin, Some("https://frame.engmanager.xyz")),
                "{origin}"
            );
        }
        assert_eq!(
            normalized_origin("https://frame.engmanager.xyz/path"),
            Some("https://frame.engmanager.xyz".into())
        );
    }

    #[test]
    fn action_and_workflow_kinds_are_not_interchangeable() {
        let action = legacy_protected_billing_auth_profile("cap-v1-14ea978608dcf07e")
            .expect("checked-in action");
        let workflow = legacy_protected_billing_auth_profile("cap-v1-5a990f470c701cec")
            .expect("checked-in workflow");
        assert_eq!(action.kind, LegacyProtectedBillingAuthKindV1::ServerAction);
        assert_eq!(workflow.kind, LegacyProtectedBillingAuthKindV1::Workflow);
    }
}
