//! Callable HTTP/RPC/action/workflow carriers for protected media contracts.
//!
//! Every admitted call reaches D1 before returning the fail-closed evidence
//! gate. These functions are intentionally isolated so the central router can
//! wire them without weakening an existing carrier while cutover is reviewed.

use frame_application::{
    LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES, LegacyProtectedMediaAuthV1,
    LegacyProtectedMediaEntitlementBindingV1, LegacyProtectedMediaEnvelopeV1,
    LegacyProtectedMediaIdempotencyV1, LegacyProtectedMediaKindV1,
    LegacyProtectedMediaPolicyProofV1, LegacyProtectedMediaPrincipalV1,
    LegacyProtectedMediaProfileV1, LegacyProtectedMediaReplayOriginV1,
    LegacyProtectedMediaTerminalKindV1, RateLimitDecisionV1,
    legacy_protected_media_authority_binding_digest, legacy_protected_media_credential_digest,
    legacy_protected_media_profile,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Request, Response, ResponseBody, Result, send::IntoSendFuture};

use crate::{
    browser_web_runtime::{
        self, BrowserWebFailure, BrowserWebOutcome, HostOnlyBrowserSessionBindingV1,
    },
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_protected_media_runtime::{
        D1LegacyProtectedMediaRuntimeV1, LEGACY_PROTECTED_MEDIA_REPLAY_RETENTION_MS,
        LegacyProtectedMediaFailureV1, LegacyProtectedMediaStageOutcomeV1,
    },
    legacy_transcripts_runtime::{D1LegacyTranscriptAuthorityV1, LegacyTranscriptRuntimeErrorV1},
};

const PROTECTED_MEDIA_RPC_OPERATION_ID: &str = "cap-v1-aa2bd4c3be69ed42";
const PROTECTED_MEDIA_RPC_TAG: &str = "VideosGetThumbnails";
const RPC_PARSE_FAILURE: &str = "Invalid Effect RPC request payload";
const RPC_UNKNOWN_TAG_FAILURE: &str = "Unknown Effect RPC request tag";
const MAX_TERMINAL_BODY_BYTES: usize = 32 * 1_024 * 1_024;
const SHARE_CAPABILITY_BY_HASH_SQL: &str =
    include_str!("../queries/legacy_protected_media/share_capability_by_hash.sql");
const SHARE_PUBLIC_CAPABILITY_SQL: &str =
    include_str!("../queries/legacy_protected_media/share_public_capability.sql");
const JOB_CAPABILITY_SQL: &str =
    include_str!("../queries/legacy_protected_media/job_capability.sql");
const VIDEO_POLICY_BASE_SQL: &str =
    include_str!("../queries/legacy_protected_media/video_policy_base.sql");
const ORGANIZATION_ACCESS_SQL: &str =
    include_str!("../queries/legacy_protected_media/organization_access.sql");
const AI_ENTITLEMENT_SQL: &str =
    include_str!("../queries/legacy_protected_media/ai_entitlement.sql");
const WORKFLOW_PARENT_READ_SQL: &str =
    include_str!("../queries/legacy_protected_media/workflow_parent_read.sql");

#[derive(Debug, Clone, PartialEq)]
struct DecodedProtectedMediaRpcV1 {
    id: String,
    payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProtectedMediaRpcDecodeFailureV1 {
    Malformed(Option<String>),
    UnknownTag,
}

#[derive(Debug, Clone, Deserialize)]
struct ShareCapabilityRowV1 {
    capability_kind: String,
    capability_ordinal: i64,
    capability_subject_id: String,
    capability_revision: i64,
    password_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct JobCapabilityRowV1 {
    id: String,
    video_id: String,
    state: String,
    attempt: i64,
    updated_at_ms: i64,
    lease_expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct VideoPolicyBaseRowV1 {
    legacy_video_id: String,
    mapped_video_id: String,
    owner_id: String,
    legacy_property_revision: i64,
    legacy_public: i64,
    legacy_allowed_email_restriction: Option<String>,
    explicit_access: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct OrganizationAccessRowV1 {
    organization_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AiEntitlementRowV1 {
    user_id: String,
    entitlement_revision: i64,
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkflowParentRowV1 {
    source_operation_id: String,
    actor_id: Option<String>,
    tenant_id: Option<String>,
    target_id: Option<String>,
    credential_kind: String,
    credential_subject_id: Option<String>,
    credential_key_version: Option<i64>,
    credential_digest: Option<String>,
    policy_proofs_json: String,
    entitlement_kind: Option<String>,
    entitlement_subject_id: Option<String>,
    entitlement_revision: Option<i64>,
    entitlement_expires_at_ms: Option<i64>,
    authority_binding_digest: String,
    created_at_ms: i64,
    target_binding_rule: String,
    translated_legacy_target_id: Option<String>,
}

#[derive(Clone, Copy)]
struct ParentReceiptClaimV1<'parent> {
    family: &'parent str,
    receipt_id: &'parent str,
    request_digest: &'parent str,
    authority_binding_digest: &'parent str,
}

#[derive(Clone, PartialEq)]
pub(crate) struct DecodedProtectedMediaActionV1 {
    operation_id: String,
    payload: Value,
}

impl std::fmt::Debug for DecodedProtectedMediaActionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedProtectedMediaActionV1")
            .field("operation_id", &self.operation_id)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProtectedMediaRequestV1 {
    operation_id: String,
    payload: Vec<u8>,
}

impl std::fmt::Debug for ProtectedMediaRequestV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProtectedMediaRequestV1")
            .field("operation_id", &self.operation_id)
            .field("payload", &"[REDACTED]")
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SealedProtectedMediaRequestV1 {
    pub opaque_ref: String,
    pub plaintext_digest: String,
}

/// Exact signed URLs, provider callbacks, object keys, and edit documents may
/// cross only this narrow boundary. Implementations seal outside D1 and return
/// an opaque reference plus the independently reproducible plaintext digest.
pub(crate) trait ProtectedMediaRequestVaultV1 {
    fn seal(
        &self,
        request: &ProtectedMediaRequestV1,
    ) -> std::result::Result<SealedProtectedMediaRequestV1, LegacyProtectedMediaFailureV1>;
}

struct UnavailableProtectedMediaRequestVaultV1;

impl ProtectedMediaRequestVaultV1 for UnavailableProtectedMediaRequestVaultV1 {
    fn seal(
        &self,
        _request: &ProtectedMediaRequestV1,
    ) -> std::result::Result<SealedProtectedMediaRequestV1, LegacyProtectedMediaFailureV1> {
        Err(LegacyProtectedMediaFailureV1::Unavailable)
    }
}

fn seal_protected_media_request(
    profile: &LegacyProtectedMediaProfileV1,
    payload: &Value,
    vault: &dyn ProtectedMediaRequestVaultV1,
) -> std::result::Result<Option<SealedProtectedMediaRequestV1>, LegacyProtectedMediaFailureV1> {
    let protected = payload.as_object().is_some_and(|object| {
        object.keys().any(|field| {
            matches!(
                field.as_str(),
                "videoUrl"
                    | "audioUrl"
                    | "sourceUrl"
                    | "outputPresignedUrl"
                    | "outputVerificationUrl"
                    | "thumbnailPresignedUrl"
                    | "previewGifPresignedUrl"
                    | "webhookUrl"
                    | "webhookSecret"
                    | "videoInitUrl"
                    | "videoSegmentUrls"
                    | "audioInitUrl"
                    | "audioSegmentUrls"
                    | "signPartUrl"
                    | "completeUrl"
                    | "abortUrl"
                    | "outputUpload"
                    | "rawFileKey"
                    | "sourceKey"
                    | "bucketId"
                    | "uploadId"
                    | "editSpec"
                    | "previousSpec"
            )
        })
    });
    if !protected {
        return Ok(None);
    }
    let request = ProtectedMediaRequestV1 {
        operation_id: profile.operation_id.into(),
        payload: serde_json::to_vec(payload).map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?,
    };
    let expected_digest = digest(&request.payload);
    let sealed = vault.seal(&request)?;
    if !valid_opaque_ref("frame-pm-request-v1:", &sealed.opaque_ref)
        || sealed.plaintext_digest != expected_digest
    {
        return Err(LegacyProtectedMediaFailureV1::Corrupt);
    }
    Ok(Some(sealed))
}

pub async fn route_response(
    operation_id: &str,
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let Some(profile) = legacy_protected_media_profile(operation_id) else {
        return failure_response(LegacyProtectedMediaFailureV1::Invalid, false);
    };
    if profile.kind != LegacyProtectedMediaKindV1::Route
        || request.method().to_string() != profile.method
    {
        return failure_response(LegacyProtectedMediaFailureV1::Invalid, false);
    }
    if profile.auth == LegacyProtectedMediaAuthV1::PublicEdgeOrJobCapability
        && matches!(
            compatibility_rate_limit::admit_edge_request(
                env,
                request,
                CompatibilityRateLimitBucketV1::ServiceMisc,
                now_ms,
            )
            .await?,
            RateLimitDecisionV1::Rejected { .. }
        )
    {
        let mut response = json_status(429, json!({"error":"RATE_LIMITED"}))?;
        response.headers_mut().set("retry-after", "60")?;
        return Ok(response);
    }
    let payload = match decode_route_payload(profile, request).await {
        Ok(payload) => payload,
        Err(failure) => return failure_response(failure, profile.method == "HEAD"),
    };
    let database = env.d1("DB")?;
    let principal =
        match authenticate_route(profile, request, env, &database, &payload, now_ms).await? {
            Ok(principal) => principal,
            Err(response) => return Ok(response),
        };
    let (execution_key, replay_origin) =
        match bounded_route_replay_key(profile, &principal, &payload, now_ms) {
            Ok(value) => value,
            Err(failure) => return failure_response(failure, profile.method == "HEAD"),
        };
    let result = stage(
        &database,
        profile,
        principal,
        &execution_key,
        replay_origin,
        payload,
        now_ms,
    )
    .await;
    stage_response(
        result,
        profile.method == "HEAD",
        &UnavailableProtectedMediaTerminalResolverV1,
    )
}

/// Return whether an already-bounded Effect-RPC body belongs to the protected
/// media family. The shared ERPC multiplexer uses this exact tag check before
/// handing the body to the strict decoder below.
#[must_use]
pub(crate) fn is_protected_media_rpc_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("tag").and_then(Value::as_str).map(str::to_owned))
        .as_deref()
        == Some(PROTECTED_MEDIA_RPC_TAG)
}

/// Decode and dispatch Cap's `VideosGetThumbnails` Effect-RPC carrier. The
/// RPC wire remains a successful HTTP exchange while the typed exit and an
/// opaque response header communicate the fail-closed execution-evidence
/// gate.
pub(crate) async fn effect_rpc_response_from_bytes(
    bytes: &[u8],
    request: &Request,
    env: &Env,
    _request_id: &str,
) -> Result<Response> {
    let decoded = match decode_protected_media_rpc(bytes) {
        Ok(decoded) => decoded,
        Err(ProtectedMediaRpcDecodeFailureV1::Malformed(Some(id))) => {
            return protected_rpc_response(rpc_die(&id, RPC_PARSE_FAILURE), None);
        }
        Err(ProtectedMediaRpcDecodeFailureV1::Malformed(None)) => {
            return protected_rpc_response(rpc_defect(RPC_PARSE_FAILURE), None);
        }
        Err(ProtectedMediaRpcDecodeFailureV1::UnknownTag) => {
            return protected_rpc_response(rpc_defect(RPC_UNKNOWN_TAG_FAILURE), None);
        }
    };
    let now_ms = crate::current_time_ms()?;
    let database = env.d1("DB")?;
    let principal = match authenticate_rpc_principal(
        request,
        env,
        &database,
        &decoded.payload,
        now_ms,
    )
    .await?
    {
        Ok(principal) => principal,
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
    let rate_subject = principal
        .actor_id
        .clone()
        .unwrap_or_else(|| principal.digest());
    if matches!(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::VideoMedia,
            &rate_subject,
            now_ms,
        )
        .await?,
        RateLimitDecisionV1::Rejected { .. }
    ) {
        return protected_rpc_response(rpc_internal_failure(&decoded.id, "unknown"), None);
    }
    let profile = legacy_protected_media_profile(PROTECTED_MEDIA_RPC_OPERATION_ID)
        .ok_or_else(|| worker::Error::RustError("protected media profile is unavailable".into()))?;
    let (execution_key, replay_origin) =
        match bounded_replay_key(profile, &principal, &decoded.payload, now_ms) {
            Ok(value) => value,
            Err(_) => return protected_rpc_response(rpc_die(&decoded.id, RPC_PARSE_FAILURE), None),
        };
    let outcome = callable(
        PROTECTED_MEDIA_RPC_OPERATION_ID,
        LegacyProtectedMediaKindV1::Rpc,
        &database,
        principal,
        ReplayClaimV1 {
            key: &execution_key,
            origin: replay_origin,
        },
        decoded.payload,
        now_ms,
    )
    .await;
    let (value, receipt_id) = match outcome {
        Ok(LegacyProtectedMediaStageOutcomeV1::ExecutionEvidenceRequired {
            receipt_id, ..
        }) => (
            rpc_internal_failure(&decoded.id, "unknown"),
            Some(receipt_id),
        ),
        Ok(LegacyProtectedMediaStageOutcomeV1::VerifiedSealedTerminal {
            terminal_kind,
            sealed_terminal_ref,
            sealed_terminal_digest,
            ..
        }) => match resolve_sealed_terminal(
            terminal_kind,
            &sealed_terminal_ref,
            &sealed_terminal_digest,
            &UnavailableProtectedMediaTerminalResolverV1,
        ) {
            Ok(terminal) => match terminal.json_value() {
                Ok(value) => (rpc_success(&decoded.id, value), None),
                Err(_) => (rpc_internal_failure(&decoded.id, "unknown"), None),
            },
            Err(_) => (rpc_internal_failure(&decoded.id, "unknown"), None),
        },
        Err(LegacyProtectedMediaFailureV1::Invalid) => {
            (rpc_die(&decoded.id, RPC_PARSE_FAILURE), None)
        }
        Err(LegacyProtectedMediaFailureV1::Corrupt)
        | Err(LegacyProtectedMediaFailureV1::Unavailable) => {
            (rpc_internal_failure(&decoded.id, "database"), None)
        }
        Err(
            LegacyProtectedMediaFailureV1::Conflict
            | LegacyProtectedMediaFailureV1::ExecutionEvidenceRequired,
        ) => (rpc_internal_failure(&decoded.id, "unknown"), None),
    };
    protected_rpc_response(value, receipt_id.as_deref())
}

#[must_use]
pub(crate) fn is_server_action(operation_id: &str) -> bool {
    legacy_protected_media_profile(operation_id)
        .is_some_and(|profile| profile.kind == LegacyProtectedMediaKindV1::ServerAction)
}

/// Decode Frame's authenticated compatibility bridge for one of the seven
/// source-pinned Cap server actions. The body is the action's exact JSON
/// payload. Cap did not define an idempotency header; replay is bound to the
/// independently validated one-use mutation grant.
pub(crate) async fn decode_server_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedProtectedMediaActionV1>> {
    if !is_server_action(operation_id)
        || !matches!(
            request.headers().get("content-type")?.as_deref(),
            Some("application/json" | "application/json; charset=utf-8")
        )
        || request
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
    if declared.is_some_and(|value| value == 0 || value > LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES).await
        {
            Ok(bytes) => bytes,
            Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
        };
    if bytes.is_empty()
        || bytes.len() > LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES
        || declared.is_some_and(|value| value != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let payload = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    Ok(Ok(DecodedProtectedMediaActionV1 {
        operation_id: operation_id.into(),
        payload,
    }))
}

/// Authenticate, rate-limit, consume the one-use CSRF grant, and stage a
/// protected server action. Grant consumption happens before staging so no
/// durable media intent can be created with a reusable browser mutation proof.
pub(crate) async fn server_action_http_response(
    request: &Request,
    env: &Env,
    decoded: &DecodedProtectedMediaActionV1,
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
    .await?
    {
        Ok(binding) => binding,
        Err(failure) => return Ok(Err(failure)),
    };
    if matches!(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::VideoMedia,
            &actor_id,
            now_ms,
        )
        .await?,
        RateLimitDecisionV1::Rejected { .. }
    ) {
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let tenant_id =
        browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await?;
    let mut principal = session_binding_principal(&binding, tenant_id.as_deref());
    let profile = legacy_protected_media_profile(&decoded.operation_id)
        .filter(|profile| profile.kind == LegacyProtectedMediaKindV1::ServerAction)
        .ok_or_else(|| worker::Error::RustError("protected media action profile missing".into()))?;
    match authorize_video_payload(
        profile,
        request,
        env,
        &database,
        &decoded.payload,
        Some(&actor_id),
        true,
        true,
    )
    .await
    {
        Ok(proofs) => principal.policy_proofs = proofs,
        Err(failure) => return Ok(Err(map_authority_browser_failure(failure))),
    }
    if let Err(failure) =
        authorize_action_scope(profile, &database, &decoded.payload, &actor_id, now_ms).await
    {
        return Ok(Err(map_authority_browser_failure(failure)));
    }
    if !browser_web_runtime::consume_session_grant(&database, &proof, now_ms).await? {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    let execution_key = bounded_named_replay_key(
        "grant",
        profile,
        &proof.mutation_grant_id().to_string(),
        &principal,
        &decoded.payload,
        now_ms,
    )
    .map_err(|_| worker::Error::RustError("invalid protected media grant claim".into()))?;
    let outcome = callable(
        &decoded.operation_id,
        LegacyProtectedMediaKindV1::ServerAction,
        &database,
        principal,
        ReplayClaimV1 {
            key: &execution_key,
            origin: LegacyProtectedMediaReplayOriginV1::Grant,
        },
        decoded.payload.clone(),
        now_ms,
    )
    .await;
    Ok(Ok(stage_response(
        outcome,
        false,
        &UnavailableProtectedMediaTerminalResolverV1,
    )?))
}

/// Effect-RPC carrier for `VideosGetThumbnails`. The shared ERPC decoder
/// supplies the authenticated actor and exact payload array.
pub async fn rpc_response(
    _operation_id: &str,
    _database: &D1Database,
    _actor_id: &str,
    _tenant_id: Option<&str>,
    _idempotency_key: &str,
    _payload: Value,
    _now_ms: i64,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    // The legacy dispatcher did not carry the exact D1 session id/version/
    // digest. Keep its source-compatible signature but fail closed; released
    // HTTP RPC ingress above uses the exact binding path.
    Err(LegacyProtectedMediaFailureV1::Unavailable)
}

/// Server-action carrier. Authentication/CSRF happens in the shared action
/// dispatcher; this function still binds the resulting actor into the receipt.
pub async fn server_action_response(
    _operation_id: &str,
    _database: &D1Database,
    _actor_id: &str,
    _tenant_id: Option<&str>,
    _idempotency_key: &str,
    _payload: Value,
    _now_ms: i64,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    Err(LegacyProtectedMediaFailureV1::Unavailable)
}

/// Workflow carrier. The scheduler supplies only the immutable parent receipt
/// identity and request digest. Frame reloads the exact parent authority and
/// derives the child replay key; caller-provided actor or credential material
/// is never accepted at this boundary.
#[allow(clippy::too_many_arguments)]
pub async fn workflow_response(
    operation_id: &str,
    database: &D1Database,
    parent_family: &str,
    parent_receipt_id: &str,
    parent_request_digest: &str,
    payload: Value,
    now_ms: i64,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    let profile = legacy_protected_media_profile(operation_id)
        .filter(|profile| profile.kind == LegacyProtectedMediaKindV1::Workflow)
        .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
    if !matches!(parent_family, "protected_media" | "protected_integrations")
        || !valid_uuid(parent_receipt_id)
        || !valid_lower_digest(parent_request_digest)
        || !(0..=9_007_199_254_740_991).contains(&now_ms)
    {
        return Err(LegacyProtectedMediaFailureV1::Invalid);
    }
    let result = database
        .prepare(WORKFLOW_PARENT_READ_SQL)
        .bind(&[
            JsValue::from_str(parent_family),
            JsValue::from_str(parent_receipt_id),
            JsValue::from_str(parent_request_digest),
            JsValue::from_str(operation_id),
            JsValue::from_f64(now_ms as f64),
        ])
        .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?;
    if !result.success() {
        return Err(LegacyProtectedMediaFailureV1::Unavailable);
    }
    let rows = result
        .results::<WorkflowParentRowV1>()
        .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
    let [parent] = rows.as_slice() else {
        return Err(if rows.is_empty() {
            LegacyProtectedMediaFailureV1::Unavailable
        } else {
            LegacyProtectedMediaFailureV1::Corrupt
        });
    };
    let principal =
        workflow_principal(profile, parent_family, parent_receipt_id, parent, &payload)?;
    let execution_key = format!("{parent_family}:{parent_receipt_id}");
    stage_with_vault_and_parent(
        database,
        profile,
        principal,
        &execution_key,
        LegacyProtectedMediaReplayOriginV1::Workflow,
        payload,
        now_ms,
        Some(ParentReceiptClaimV1 {
            family: parent_family,
            receipt_id: parent_receipt_id,
            request_digest: parent_request_digest,
            authority_binding_digest: &parent.authority_binding_digest,
        }),
        &UnavailableProtectedMediaRequestVaultV1,
    )
    .await
}

fn workflow_principal(
    profile: &LegacyProtectedMediaProfileV1,
    parent_family: &str,
    parent_receipt_id: &str,
    parent: &WorkflowParentRowV1,
    payload: &Value,
) -> Result<LegacyProtectedMediaPrincipalV1, LegacyProtectedMediaFailureV1> {
    if !valid_lower_digest(&parent.authority_binding_digest)
        || !(0..=9_007_199_254_740_991).contains(&parent.created_at_ms)
        || !matches!(
            parent.target_binding_rule.as_str(),
            "same" | "child_derived"
        )
    {
        return Err(LegacyProtectedMediaFailureV1::Corrupt);
    }

    let mut policy_proofs =
        serde_json::from_str::<Vec<LegacyProtectedMediaPolicyProofV1>>(&parent.policy_proofs_json)
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
    if parent_family == "protected_integrations"
        && parent.source_operation_id == "cap-v1-d9b654b30f6c362a"
    {
        let parent_target = parent
            .target_id
            .as_deref()
            .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
        let child_target = parent
            .translated_legacy_target_id
            .as_deref()
            .ok_or(LegacyProtectedMediaFailureV1::Unavailable)?;
        if parent.target_binding_rule != "child_derived"
            || payload.get("videoId").and_then(Value::as_str) != Some(child_target)
            || policy_proofs.is_empty()
        {
            return Err(LegacyProtectedMediaFailureV1::Unavailable);
        }
        for proof in &mut policy_proofs {
            if proof.target_id != parent_target {
                return Err(LegacyProtectedMediaFailureV1::Corrupt);
            }
            proof.target_id = child_target.into();
        }
    } else if parent.target_binding_rule == "same" {
        let target_field = profile
            .target_field
            .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
        if payload.get(target_field).and_then(Value::as_str) != parent.target_id.as_deref() {
            return Err(LegacyProtectedMediaFailureV1::Unavailable);
        }
    }

    let entitlement_binding = match (
        parent.entitlement_kind.as_deref(),
        parent.entitlement_subject_id.as_deref(),
        parent.entitlement_revision,
        parent.entitlement_expires_at_ms,
    ) {
        (None, None, None, None) => None,
        (Some(kind), Some(subject_id), Some(revision), expires_at_ms) => {
            Some(LegacyProtectedMediaEntitlementBindingV1 {
                kind: kind.into(),
                subject_id: subject_id.into(),
                revision,
                expires_at_ms,
            })
        }
        _ => return Err(LegacyProtectedMediaFailureV1::Corrupt),
    };
    let (credential_kind, credential_subject_id, credential_key_version, credential_digest) =
        if parent.credential_kind == "none" {
            if parent.actor_id.is_some()
                || parent.credential_subject_id.is_some()
                || parent.credential_key_version.is_some()
                || parent.credential_digest.is_some()
            {
                return Err(LegacyProtectedMediaFailureV1::Corrupt);
            }
            (
                "parent_capability".into(),
                Some(format!("{parent_family}:{parent_receipt_id}")),
                Some(parent.created_at_ms),
                Some(parent.authority_binding_digest.clone()),
            )
        } else {
            let subject_id = parent
                .credential_subject_id
                .clone()
                .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
            let key_version = parent
                .credential_key_version
                .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
            let credential_digest = parent
                .credential_digest
                .clone()
                .filter(|value| valid_lower_digest(value))
                .ok_or(LegacyProtectedMediaFailureV1::Corrupt)?;
            (
                parent.credential_kind.clone(),
                Some(subject_id),
                Some(key_version),
                Some(credential_digest),
            )
        };
    Ok(LegacyProtectedMediaPrincipalV1 {
        class: "parent_derived".into(),
        actor_id: parent.actor_id.clone(),
        tenant_id: parent.tenant_id.clone(),
        credential_kind,
        credential_subject_id,
        credential_key_version,
        credential_digest,
        policy_proofs,
        entitlement_binding,
    })
}

#[derive(Clone, Copy)]
struct ReplayClaimV1<'claim> {
    key: &'claim str,
    origin: LegacyProtectedMediaReplayOriginV1,
}

async fn callable(
    operation_id: &str,
    expected_kind: LegacyProtectedMediaKindV1,
    database: &D1Database,
    principal: LegacyProtectedMediaPrincipalV1,
    replay: ReplayClaimV1<'_>,
    payload: Value,
    now_ms: i64,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    let profile = legacy_protected_media_profile(operation_id)
        .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
    if profile.kind != expected_kind
        || profile.idempotency != LegacyProtectedMediaIdempotencyV1::Required
    {
        return Err(LegacyProtectedMediaFailureV1::Invalid);
    }
    stage(
        database,
        profile,
        principal,
        replay.key,
        replay.origin,
        payload,
        now_ms,
    )
    .await
}

async fn stage(
    database: &D1Database,
    profile: &LegacyProtectedMediaProfileV1,
    principal: LegacyProtectedMediaPrincipalV1,
    execution_key: &str,
    replay_origin: LegacyProtectedMediaReplayOriginV1,
    payload: Value,
    now_ms: i64,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    stage_with_vault(
        database,
        profile,
        principal,
        execution_key,
        replay_origin,
        payload,
        now_ms,
        &UnavailableProtectedMediaRequestVaultV1,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn stage_with_vault(
    database: &D1Database,
    profile: &LegacyProtectedMediaProfileV1,
    principal: LegacyProtectedMediaPrincipalV1,
    execution_key: &str,
    replay_origin: LegacyProtectedMediaReplayOriginV1,
    payload: Value,
    now_ms: i64,
    vault: &dyn ProtectedMediaRequestVaultV1,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    stage_with_vault_and_parent(
        database,
        profile,
        principal,
        execution_key,
        replay_origin,
        payload,
        now_ms,
        None,
        vault,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn stage_with_vault_and_parent(
    database: &D1Database,
    profile: &LegacyProtectedMediaProfileV1,
    principal: LegacyProtectedMediaPrincipalV1,
    execution_key: &str,
    replay_origin: LegacyProtectedMediaReplayOriginV1,
    payload: Value,
    now_ms: i64,
    parent: Option<ParentReceiptClaimV1<'_>>,
    vault: &dyn ProtectedMediaRequestVaultV1,
) -> Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1> {
    let sealed = seal_protected_media_request(profile, &payload, vault)?;
    let authority_binding_digest =
        legacy_protected_media_authority_binding_digest(profile, &principal, &payload)
            .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
    let envelope = LegacyProtectedMediaEnvelopeV1 {
        source_operation_id: profile.operation_id.into(),
        principal,
        execution_key: execution_key.into(),
        replay_origin,
        parent_family: parent.map(|value| value.family.into()),
        parent_receipt_id: parent.map(|value| value.receipt_id.into()),
        parent_request_digest: parent.map(|value| value.request_digest.into()),
        parent_authority_binding_digest: parent.map(|value| value.authority_binding_digest.into()),
        authority_binding_digest,
        payload,
        sealed_request_ref: sealed.as_ref().map(|value| value.opaque_ref.clone()),
        sealed_request_digest: sealed.map(|value| value.plaintext_digest),
    };
    D1LegacyProtectedMediaRuntimeV1::new(database)
        .stage(profile, &envelope, now_ms)
        .await
}

fn decode_protected_media_rpc(
    bytes: &[u8],
) -> std::result::Result<DecodedProtectedMediaRpcV1, ProtectedMediaRpcDecodeFailureV1> {
    if bytes.is_empty() || bytes.len() > LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES {
        return Err(ProtectedMediaRpcDecodeFailureV1::Malformed(None));
    }
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| ProtectedMediaRpcDecodeFailureV1::Malformed(None))?;
    let object = value
        .as_object()
        .ok_or(ProtectedMediaRpcDecodeFailureV1::Malformed(None))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_rpc_id(value))
        .map(str::to_owned)
        .ok_or(ProtectedMediaRpcDecodeFailureV1::Malformed(None))?;
    let malformed = || ProtectedMediaRpcDecodeFailureV1::Malformed(Some(id.clone()));
    if object.get("_tag").and_then(Value::as_str) != Some("Request")
        || !valid_rpc_headers(object.get("headers"))
        || !valid_optional_string(object.get("traceId"))
        || !valid_optional_string(object.get("spanId"))
        || !valid_optional_bool(object.get("sampled"))
    {
        return Err(malformed());
    }
    if object.get("tag").and_then(Value::as_str) != Some(PROTECTED_MEDIA_RPC_TAG) {
        return Err(ProtectedMediaRpcDecodeFailureV1::UnknownTag);
    }
    let payload = object
        .get("payload")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 50)
        .ok_or_else(malformed)?;
    if payload.iter().any(|value| {
        value.as_str().is_none_or(|value| {
            value.is_empty() || value.len() > 255 || value.chars().any(char::is_control)
        })
    }) {
        return Err(malformed());
    }
    Ok(DecodedProtectedMediaRpcV1 {
        id,
        payload: Value::Array(payload.clone()),
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
            .set("x-frame-execution-receipt", receipt_id)?;
    }
    Ok(response)
}

async fn authenticate_route(
    profile: &LegacyProtectedMediaProfileV1,
    request: &Request,
    env: &Env,
    database: &D1Database,
    payload: &Value,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedMediaPrincipalV1, Response>> {
    let mut principal = match profile.auth {
        LegacyProtectedMediaAuthV1::SchedulerSecret => {
            let Some(expected) = env_value(env, "CRON_SECRET") else {
                return Ok(Err(json_status(
                    500,
                    json!({"error":"Server misconfiguration"}),
                )?));
            };
            let actual = request.headers().get("authorization")?;
            let bearer = format!("Bearer {expected}");
            if actual
                .as_deref()
                .is_none_or(|actual| !constant_time_equal(actual, &bearer))
            {
                return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
            }
            credential_principal(
                "scheduler_secret",
                "scheduler_secret",
                "CRON_SECRET.v1",
                &expected,
            )
        }
        LegacyProtectedMediaAuthV1::InternalService => {
            let Some(expected) = env_value(env, "MEDIA_SERVER_WEBHOOK_SECRET") else {
                return Ok(Err(json_status(
                    503,
                    json!({"error":"Media execution is unavailable"}),
                )?));
            };
            let actual = request.headers().get("x-media-server-secret")?;
            if actual
                .as_deref()
                .is_none_or(|actual| !constant_time_equal(actual, &expected))
            {
                return Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?));
            }
            credential_principal(
                "internal_service",
                "service_secret",
                "MEDIA_SERVER_WEBHOOK_SECRET.v1",
                &expected,
            )
        }
        LegacyProtectedMediaAuthV1::PublicEdgeOrJobCapability => {
            match public_edge_or_job_principal(database, request, profile, payload).await {
                Ok(principal) => principal,
                Err(LegacyProtectedMediaFailureV1::Invalid) => {
                    return Ok(Err(json_status(404, json!({"error":"NOT_FOUND"}))?));
                }
                Err(_) => {
                    return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
                }
            }
        }
        LegacyProtectedMediaAuthV1::Session => {
            match required_session_principal(request, env, now_ms).await? {
                Ok(principal) => principal,
                Err(response) => return Ok(Err(response)),
            }
        }
        LegacyProtectedMediaAuthV1::OptionalSessionOrShareCapability => {
            match browser_web_runtime::authenticate_host_only_browser_session_binding(
                request, env, now_ms,
            )
            .await?
            {
                Ok(binding) => {
                    let tenant = browser_web_runtime::trusted_active_organization_id(
                        database,
                        &binding.user_id,
                    )
                    .await?;
                    session_binding_principal(&binding, tenant.as_deref())
                }
                Err(BrowserWebFailure::Unauthenticated) => {
                    match share_capability_principal(database, request, env, payload).await {
                        Ok(principal) => principal,
                        Err(LegacyProtectedMediaFailureV1::Invalid) => {
                            return Ok(Err(json_status(404, json!({"error":"NOT_FOUND"}))?));
                        }
                        Err(_) => {
                            return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
                        }
                    }
                }
                Err(_) => return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?)),
            }
        }
        LegacyProtectedMediaAuthV1::PublicOrFlowToken => {
            match browser_web_runtime::authenticate_host_only_browser_session_binding(
                request, env, now_ms,
            )
            .await?
            {
                Ok(binding) => {
                    let tenant = browser_web_runtime::trusted_active_organization_id(
                        database,
                        &binding.user_id,
                    )
                    .await?;
                    session_binding_principal(&binding, tenant.as_deref())
                }
                Err(BrowserWebFailure::Unauthenticated) => {
                    let expected = env_value(env, "FRAME_MEDIA_FLOW_TOKEN");
                    let actual = request.headers().get("x-frame-flow-token")?;
                    match (expected, actual) {
                        (Some(expected), Some(actual))
                            if constant_time_equal(&actual, &expected) =>
                        {
                            credential_principal(
                                "public_or_flow_token",
                                "flow_token",
                                "FRAME_MEDIA_FLOW_TOKEN.v1",
                                &expected,
                            )
                        }
                        _ => return Ok(Err(json_status(401, json!({"auth":false}))?)),
                    }
                }
                Err(_) => return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?)),
            }
        }
        LegacyProtectedMediaAuthV1::ParentDerived => {
            return Ok(Err(json_status(503, json!({"error":"Unavailable"}))?));
        }
    };
    let require_owner = profile.method == "POST"
        && principal.class == "session"
        && profile.auth == LegacyProtectedMediaAuthV1::Session;
    match authorize_video_payload(
        profile,
        request,
        env,
        database,
        payload,
        principal.actor_id.as_deref(),
        require_owner,
        matches!(
            profile.auth,
            LegacyProtectedMediaAuthV1::Session
                | LegacyProtectedMediaAuthV1::OptionalSessionOrShareCapability
                | LegacyProtectedMediaAuthV1::PublicOrFlowToken
        ),
    )
    .await
    {
        Ok(proofs) => principal.policy_proofs = proofs,
        Err(failure) => return Ok(Err(authority_failure_response(failure)?)),
    }
    match authorize_ai_entitlement(profile, database, payload, now_ms).await {
        Ok(binding) => principal.entitlement_binding = binding,
        Err(failure) => return Ok(Err(authority_failure_response(failure)?)),
    }
    Ok(Ok(principal))
}

async fn required_session_principal(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<LegacyProtectedMediaPrincipalV1, Response>> {
    match browser_web_runtime::authenticate_host_only_browser_session_binding(request, env, now_ms)
        .await?
    {
        Ok(binding) => {
            let database = env.d1("DB")?;
            let tenant =
                browser_web_runtime::trusted_active_organization_id(&database, &binding.user_id)
                    .await?;
            Ok(Ok(session_binding_principal(&binding, tenant.as_deref())))
        }
        Err(BrowserWebFailure::Unavailable) => {
            Ok(Err(json_status(503, json!({"error":"Unavailable"}))?))
        }
        Err(_) => Ok(Err(json_status(401, json!({"error":"Unauthorized"}))?)),
    }
}

async fn decode_route_payload(
    profile: &LegacyProtectedMediaProfileV1,
    request: &mut Request,
) -> std::result::Result<Value, LegacyProtectedMediaFailureV1> {
    let mut object = Map::new();
    for (key, value) in request
        .url()
        .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?
        .query_pairs()
    {
        object.insert(key.into_owned(), Value::String(value.into_owned()));
    }
    inject_path_parameter(profile, request, &mut object)?;
    if profile.method == "POST" {
        if request
            .headers()
            .get("content-type")
            .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?
            .is_some_and(|value| !value.to_ascii_lowercase().starts_with("application/json"))
        {
            return Err(LegacyProtectedMediaFailureV1::Invalid);
        }
        let bytes = crate::read_bounded_legacy_body(request, LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES)
            .await
            .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
        if !bytes.is_empty() {
            let body: Value = serde_json::from_slice(&bytes)
                .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
            let body = body
                .as_object()
                .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
            for (key, value) in body {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(Value::Object(object))
}

fn inject_path_parameter(
    profile: &LegacyProtectedMediaProfileV1,
    request: &Request,
    object: &mut Map<String, Value>,
) -> std::result::Result<(), LegacyProtectedMediaFailureV1> {
    let url = request
        .url()
        .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
    let segments: Vec<_> = url
        .path_segments()
        .ok_or(LegacyProtectedMediaFailureV1::Invalid)?
        .collect();
    if profile.path == "/api/videos/:videoId/retry-ai" {
        let video_id = segments
            .iter()
            .rev()
            .nth(1)
            .filter(|segment| !segment.is_empty())
            .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        object.insert("videoId".into(), Value::String((*video_id).into()));
    }
    if profile.path.contains(":jobId") {
        let process = segments
            .iter()
            .position(|segment| *segment == "process")
            .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        let job_id = segments
            .get(process + 1)
            .filter(|segment| !segment.is_empty())
            .ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        object.insert("jobId".into(), Value::String((*job_id).into()));
    }
    Ok(())
}

fn session_binding_principal(
    binding: &HostOnlyBrowserSessionBindingV1,
    tenant_id: Option<&str>,
) -> LegacyProtectedMediaPrincipalV1 {
    LegacyProtectedMediaPrincipalV1 {
        class: "session".into(),
        actor_id: Some(binding.user_id.clone()),
        tenant_id: tenant_id.map(str::to_owned),
        credential_kind: "session_token".into(),
        credential_subject_id: Some(binding.session_id.clone()),
        credential_key_version: Some(binding.token_key_version),
        credential_digest: Some(binding.credential_digest.clone()),
        policy_proofs: Vec::new(),
        entitlement_binding: None,
    }
}

fn credential_principal(
    class: &str,
    credential_kind: &str,
    credential_subject_id: &str,
    credential: &str,
) -> LegacyProtectedMediaPrincipalV1 {
    LegacyProtectedMediaPrincipalV1 {
        class: class.into(),
        actor_id: None,
        tenant_id: None,
        credential_kind: credential_kind.into(),
        credential_subject_id: Some(credential_subject_id.into()),
        credential_key_version: Some(1),
        credential_digest: Some(legacy_protected_media_credential_digest(credential)),
        policy_proofs: Vec::new(),
        entitlement_binding: None,
    }
}

async fn public_edge_or_job_principal(
    database: &D1Database,
    request: &Request,
    profile: &LegacyProtectedMediaProfileV1,
    payload: &Value,
) -> std::result::Result<LegacyProtectedMediaPrincipalV1, LegacyProtectedMediaFailureV1> {
    if let Some(job_id) = payload
        .get("jobId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 255)
    {
        let result = database
            .prepare(JOB_CAPABILITY_SQL)
            .bind(&[JsValue::from_str(job_id)])
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyProtectedMediaFailureV1::Unavailable);
        }
        let mut rows = result
            .results::<JobCapabilityRowV1>()
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
        if rows.len() != 1 {
            return Err(LegacyProtectedMediaFailureV1::Invalid);
        }
        let row = rows.pop().ok_or(LegacyProtectedMediaFailureV1::Invalid)?;
        if row.id != job_id
            || row.video_id.len() != 36
            || row.attempt < 0
            || !(0..=9_007_199_254_740_991).contains(&row.updated_at_ms)
            || !matches!(
                row.state.as_str(),
                "queued" | "leased" | "running" | "succeeded" | "failed" | "cancelled"
            )
        {
            return Err(LegacyProtectedMediaFailureV1::Corrupt);
        }
        let binding = serde_json::to_vec(&json!({
            "domain": "frame.protected-media-job-capability.v1",
            "operation_id": profile.operation_id,
            "job_id": row.id,
            "video_id": row.video_id,
            "state": row.state,
            "attempt": row.attempt,
            "updated_at_ms": row.updated_at_ms,
            "lease_expires_at_ms": row.lease_expires_at_ms,
        }))
        .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
        return Ok(LegacyProtectedMediaPrincipalV1 {
            class: "public_edge_or_job_capability".into(),
            actor_id: None,
            tenant_id: None,
            credential_kind: "job_capability".into(),
            credential_subject_id: Some(job_id.into()),
            credential_key_version: Some(row.updated_at_ms),
            credential_digest: Some(digest(&binding)),
            policy_proofs: Vec::new(),
            entitlement_binding: None,
        });
    }
    let mut binding = Vec::new();
    binding.extend_from_slice(b"frame.protected-media-edge-read.v1\0");
    push_digest_part(&mut binding, profile.operation_id.as_bytes());
    for header in ["cf-connecting-ip", "user-agent", "accept"] {
        let value = request
            .headers()
            .get(header)
            .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
        push_digest_part(
            &mut binding,
            digest(value.as_deref().unwrap_or_default().as_bytes()).as_bytes(),
        );
    }
    Ok(LegacyProtectedMediaPrincipalV1 {
        class: "public_edge_or_job_capability".into(),
        actor_id: None,
        tenant_id: None,
        credential_kind: "edge_read".into(),
        credential_subject_id: Some(profile.operation_id.into()),
        credential_key_version: Some(1),
        credential_digest: Some(digest(&binding)),
        policy_proofs: Vec::new(),
        entitlement_binding: None,
    })
}

async fn share_capability_principal(
    database: &D1Database,
    request: &Request,
    env: &Env,
    payload: &Value,
) -> std::result::Result<LegacyProtectedMediaPrincipalV1, LegacyProtectedMediaFailureV1> {
    let mut video_ids = payload_video_ids(payload)?;
    video_ids.sort();
    video_ids.dedup();
    if video_ids.len() != 1 {
        return Err(LegacyProtectedMediaFailureV1::Invalid);
    }
    let mut verified =
        crate::legacy_video_properties_web_runtime::existing_password_hashes(request, env)
            .unwrap_or_default();
    verified.sort();
    verified.dedup();
    let video_id = &video_ids[0];
    let mut matched = Vec::new();
    for password_hash in &verified {
        let rows = database
            .prepare(SHARE_CAPABILITY_BY_HASH_SQL)
            .bind(&[
                JsValue::from_str(video_id),
                JsValue::from_str(password_hash),
            ])
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?;
        if !rows.success() {
            return Err(LegacyProtectedMediaFailureV1::Unavailable);
        }
        matched.extend(
            rows.results::<ShareCapabilityRowV1>()
                .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?,
        );
    }
    matched.sort_by(|left, right| {
        (left.capability_ordinal, &left.capability_subject_id)
            .cmp(&(right.capability_ordinal, &right.capability_subject_id))
    });
    let capability = if let Some(capability) = matched.into_iter().next() {
        capability
    } else {
        let result = database
            .prepare(SHARE_PUBLIC_CAPABILITY_SQL)
            .bind(&[JsValue::from_str(video_id)])
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyProtectedMediaFailureV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyProtectedMediaFailureV1::Unavailable);
        }
        let mut rows = result
            .results::<ShareCapabilityRowV1>()
            .map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)?;
        if rows.len() != 1 {
            return Err(LegacyProtectedMediaFailureV1::Invalid);
        }
        rows.pop().ok_or(LegacyProtectedMediaFailureV1::Invalid)?
    };
    if capability.capability_subject_id.len() != 36
        || !matches!(capability.capability_ordinal, 0 | 1)
        || !(0..=9_007_199_254_740_991).contains(&capability.capability_revision)
        || !matches!(
            capability.capability_kind.as_str(),
            "video_password_capability" | "space_password_capability" | "public_video_capability"
        )
    {
        return Err(LegacyProtectedMediaFailureV1::Corrupt);
    }
    let credential_digest = match capability.password_hash.as_deref() {
        Some(password_hash) if verified.iter().any(|value| value == password_hash) => {
            digest(password_hash.as_bytes())
        }
        None if capability.capability_kind == "public_video_capability" => digest(
            format!(
                "public:{video_id}:{}:{}",
                capability.capability_subject_id, capability.capability_revision
            )
            .as_bytes(),
        ),
        _ => return Err(LegacyProtectedMediaFailureV1::Corrupt),
    };
    Ok(LegacyProtectedMediaPrincipalV1 {
        class: "optional_session_or_share_capability".into(),
        actor_id: None,
        tenant_id: None,
        credential_kind: capability.capability_kind,
        credential_subject_id: Some(capability.capability_subject_id),
        credential_key_version: Some(capability.capability_revision),
        credential_digest: Some(credential_digest),
        policy_proofs: Vec::new(),
        entitlement_binding: None,
    })
}

async fn authenticate_rpc_principal(
    request: &Request,
    env: &Env,
    database: &D1Database,
    payload: &Value,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyProtectedMediaPrincipalV1>> {
    let mut principal = match browser_web_runtime::authenticate_host_only_browser_session_binding(
        request, env, now_ms,
    )
    .await?
    {
        Ok(binding) => {
            let tenant =
                browser_web_runtime::trusted_active_organization_id(database, &binding.user_id)
                    .await?;
            session_binding_principal(&binding, tenant.as_deref())
        }
        Err(BrowserWebFailure::Unauthenticated) => {
            match share_capability_principal(database, request, env, payload).await {
                Ok(principal) => principal,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            }
        }
        Err(failure) => return Ok(Err(failure)),
    };
    let Some(profile) = legacy_protected_media_profile(PROTECTED_MEDIA_RPC_OPERATION_ID) else {
        return Ok(Err(BrowserWebFailure::Unavailable));
    };
    match authorize_video_payload(
        profile,
        request,
        env,
        database,
        payload,
        principal.actor_id.as_deref(),
        false,
        true,
    )
    .await
    {
        Ok(proofs) => principal.policy_proofs = proofs,
        Err(failure) => return Ok(Err(map_authority_browser_failure(failure))),
    }
    Ok(Ok(principal))
}

#[allow(clippy::too_many_arguments)]
async fn authorize_video_payload(
    _profile: &LegacyProtectedMediaProfileV1,
    request: &Request,
    env: &Env,
    database: &D1Database,
    payload: &Value,
    actor_id: Option<&str>,
    require_owner: bool,
    enforce_visibility: bool,
) -> std::result::Result<Vec<LegacyProtectedMediaPolicyProofV1>, LegacyTranscriptRuntimeErrorV1> {
    let video_ids = payload_video_ids(payload).map_err(|failure| match failure {
        LegacyProtectedMediaFailureV1::Invalid => LegacyTranscriptRuntimeErrorV1::Invalid,
        _ => LegacyTranscriptRuntimeErrorV1::Corrupt,
    })?;
    if video_ids.is_empty() {
        return Ok(Vec::new());
    }
    let authority = D1LegacyTranscriptAuthorityV1::new(database);
    let verified =
        crate::legacy_video_properties_web_runtime::existing_password_hashes(request, env)
            .unwrap_or_default();
    let mut proofs = Vec::with_capacity(video_ids.len());
    for video_id in video_ids {
        let video = authority.video(&video_id).await?;
        if require_owner {
            if !video.is_owner(actor_id) {
                return Err(LegacyTranscriptRuntimeErrorV1::Unauthorized);
            }
            proofs
                .push(resolve_video_policy_proof(database, &video_id, actor_id, &verified).await?);
        } else if enforce_visibility && !authority.can_view(&video, actor_id, &verified).await? {
            return Err(LegacyTranscriptRuntimeErrorV1::Unauthorized);
        } else if enforce_visibility {
            proofs
                .push(resolve_video_policy_proof(database, &video_id, actor_id, &verified).await?);
        }
    }
    Ok(proofs)
}

async fn authorize_action_scope(
    profile: &LegacyProtectedMediaProfileV1,
    database: &D1Database,
    payload: &Value,
    actor_id: &str,
    _now_ms: i64,
) -> std::result::Result<(), LegacyTranscriptRuntimeErrorV1> {
    if profile.operation_id != "cap-v1-24ef9eb18c4b0555" {
        return Ok(());
    }
    let org_id = payload
        .get("orgId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or(LegacyTranscriptRuntimeErrorV1::Invalid)?;
    let result = database
        .prepare(ORGANIZATION_ACCESS_SQL)
        .bind(&[JsValue::from_str(org_id), JsValue::from_str(actor_id)])
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
    if !result.success() {
        return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
    }
    let rows = result
        .results::<OrganizationAccessRowV1>()
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)?;
    if rows.len() != 1 || rows[0].organization_id.len() != 36 {
        return Err(LegacyTranscriptRuntimeErrorV1::Unauthorized);
    }
    Ok(())
}

async fn authorize_ai_entitlement(
    profile: &LegacyProtectedMediaProfileV1,
    database: &D1Database,
    payload: &Value,
    now_ms: i64,
) -> std::result::Result<
    Option<LegacyProtectedMediaEntitlementBindingV1>,
    LegacyTranscriptRuntimeErrorV1,
> {
    if !matches!(
        profile.operation_id,
        "cap-v1-c1ae43fcf8ad7018" | "cap-v1-39909646286251af"
    ) {
        return Ok(None);
    }
    let video_id = payload
        .get("videoId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or(LegacyTranscriptRuntimeErrorV1::Invalid)?;
    let result = database
        .prepare(AI_ENTITLEMENT_SQL)
        .bind(&[
            JsValue::from_str(video_id),
            JsValue::from_f64(now_ms as f64),
        ])
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
    if !result.success() {
        return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
    }
    let mut rows = result
        .results::<AiEntitlementRowV1>()
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)?;
    if rows.len() != 1 {
        return Err(LegacyTranscriptRuntimeErrorV1::Unauthorized);
    }
    let row = rows
        .pop()
        .ok_or(LegacyTranscriptRuntimeErrorV1::Unauthorized)?;
    if row.user_id.len() != 36
        || !(0..=9_007_199_254_740_991).contains(&row.entitlement_revision)
        || row.expires_at_ms.is_some_and(|value| value <= now_ms)
    {
        return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
    }
    Ok(Some(LegacyProtectedMediaEntitlementBindingV1 {
        kind: "ai_owner".into(),
        subject_id: row.user_id,
        revision: row.entitlement_revision,
        expires_at_ms: row.expires_at_ms,
    }))
}

async fn resolve_video_policy_proof(
    database: &D1Database,
    video_id: &str,
    actor_id: Option<&str>,
    verified_password_hashes: &[String],
) -> std::result::Result<LegacyProtectedMediaPolicyProofV1, LegacyTranscriptRuntimeErrorV1> {
    let result = database
        .prepare(VIDEO_POLICY_BASE_SQL)
        .bind(&[
            JsValue::from_str(video_id),
            actor_id.map_or(JsValue::NULL, JsValue::from_str),
        ])
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?
        .all()
        .into_send()
        .await
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
    if !result.success() {
        return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
    }
    let mut rows = result
        .results::<VideoPolicyBaseRowV1>()
        .map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)?;
    if rows.len() != 1 {
        return Err(LegacyTranscriptRuntimeErrorV1::NotFound);
    }
    let base = rows.pop().ok_or(LegacyTranscriptRuntimeErrorV1::NotFound)?;
    if base.legacy_video_id != video_id
        || base.mapped_video_id.len() != 36
        || base.owner_id.len() != 36
        || !(0..=9_007_199_254_740_991).contains(&base.legacy_property_revision)
        || !matches!(base.legacy_public, 0 | 1)
        || !matches!(base.explicit_access, 0 | 1)
        || base
            .legacy_allowed_email_restriction
            .as_deref()
            .is_some_and(|value| value.len() > 4096 || value.chars().any(char::is_control))
    {
        return Err(LegacyTranscriptRuntimeErrorV1::Corrupt);
    }
    if actor_id == Some(base.owner_id.as_str()) {
        return Ok(policy_proof(
            video_id,
            "owner_bypass",
            &base.mapped_video_id,
            base.legacy_property_revision,
            None,
        ));
    }

    let mut verified = verified_password_hashes.to_vec();
    verified.sort();
    verified.dedup();
    let mut matches = Vec::new();
    for password_hash in &verified {
        let result = database
            .prepare(SHARE_CAPABILITY_BY_HASH_SQL)
            .bind(&[
                JsValue::from_str(video_id),
                JsValue::from_str(password_hash),
            ])
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?
            .all()
            .into_send()
            .await
            .map_err(|_| LegacyTranscriptRuntimeErrorV1::Unavailable)?;
        if !result.success() {
            return Err(LegacyTranscriptRuntimeErrorV1::Unavailable);
        }
        matches.extend(
            result
                .results::<ShareCapabilityRowV1>()
                .map_err(|_| LegacyTranscriptRuntimeErrorV1::Corrupt)?,
        );
    }
    matches.sort_by(|left, right| {
        (left.capability_ordinal, &left.capability_subject_id)
            .cmp(&(right.capability_ordinal, &right.capability_subject_id))
    });
    if let Some(matched) = matches.into_iter().next() {
        let password_hash = matched
            .password_hash
            .as_deref()
            .filter(|value| verified.iter().any(|candidate| candidate == *value))
            .ok_or(LegacyTranscriptRuntimeErrorV1::Corrupt)?;
        let kind = match matched.capability_kind.as_str() {
            "video_password_capability" => "video_password",
            "space_password_capability" => "space_password",
            _ => return Err(LegacyTranscriptRuntimeErrorV1::Corrupt),
        };
        return Ok(policy_proof(
            video_id,
            kind,
            &matched.capability_subject_id,
            matched.capability_revision,
            Some(password_hash),
        ));
    }
    Ok(policy_proof(
        video_id,
        "unprotected_video_policy",
        &base.mapped_video_id,
        base.legacy_property_revision,
        None,
    ))
}

fn policy_proof(
    target_id: &str,
    kind: &str,
    subject_id: &str,
    revision: i64,
    password_hash: Option<&str>,
) -> LegacyProtectedMediaPolicyProofV1 {
    let audit_digest = password_hash.map_or_else(
        || {
            digest(
                format!(
                    "frame.protected-media-policy.v1:{target_id}:{kind}:{subject_id}:{revision}"
                )
                .as_bytes(),
            )
        },
        |value| digest(value.as_bytes()),
    );
    LegacyProtectedMediaPolicyProofV1 {
        target_id: target_id.into(),
        kind: kind.into(),
        subject_id: subject_id.into(),
        revision,
        audit_digest,
    }
}

fn payload_video_ids(
    payload: &Value,
) -> std::result::Result<Vec<String>, LegacyProtectedMediaFailureV1> {
    if let Some(values) = payload.as_array() {
        return values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| !value.is_empty() && value.len() <= 255)
                    .map(str::to_owned)
                    .ok_or(LegacyProtectedMediaFailureV1::Invalid)
            })
            .collect();
    }
    let Some(object) = payload.as_object() else {
        return Err(LegacyProtectedMediaFailureV1::Invalid);
    };
    Ok(object
        .get("videoId")
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 255)
                .map(str::to_owned)
                .ok_or(LegacyProtectedMediaFailureV1::Invalid)
        })
        .transpose()?
        .into_iter()
        .collect())
}

fn map_authority_browser_failure(failure: LegacyTranscriptRuntimeErrorV1) -> BrowserWebFailure {
    match failure {
        LegacyTranscriptRuntimeErrorV1::Invalid => BrowserWebFailure::Invalid,
        LegacyTranscriptRuntimeErrorV1::NotFound | LegacyTranscriptRuntimeErrorV1::Unauthorized => {
            BrowserWebFailure::NotFound
        }
        LegacyTranscriptRuntimeErrorV1::Conflict
        | LegacyTranscriptRuntimeErrorV1::Unavailable
        | LegacyTranscriptRuntimeErrorV1::Corrupt => BrowserWebFailure::Unavailable,
    }
}

fn authority_failure_response(failure: LegacyTranscriptRuntimeErrorV1) -> Result<Response> {
    match map_authority_browser_failure(failure) {
        BrowserWebFailure::Invalid => json_status(400, json!({"error":"INVALID_REQUEST"})),
        BrowserWebFailure::NotFound
        | BrowserWebFailure::Forbidden
        | BrowserWebFailure::Unauthenticated => json_status(404, json!({"error":"NOT_FOUND"})),
        _ => json_status(503, json!({"error":"UNAVAILABLE"})),
    }
}

fn bounded_route_replay_key(
    profile: &LegacyProtectedMediaProfileV1,
    principal: &LegacyProtectedMediaPrincipalV1,
    payload: &Value,
    now_ms: i64,
) -> std::result::Result<(String, LegacyProtectedMediaReplayOriginV1), LegacyProtectedMediaFailureV1>
{
    bounded_replay_key(profile, principal, payload, now_ms)
}

fn bounded_replay_key(
    profile: &LegacyProtectedMediaProfileV1,
    principal: &LegacyProtectedMediaPrincipalV1,
    payload: &Value,
    now_ms: i64,
) -> std::result::Result<(String, LegacyProtectedMediaReplayOriginV1), LegacyProtectedMediaFailureV1>
{
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Err(LegacyProtectedMediaFailureV1::Invalid);
    }
    let label = if profile.idempotency == LegacyProtectedMediaIdempotencyV1::Forbidden {
        "read"
    } else {
        "natural"
    };
    Ok((
        bounded_named_replay_key(label, profile, "", principal, payload, now_ms)?,
        LegacyProtectedMediaReplayOriginV1::Natural,
    ))
}

fn bounded_named_replay_key(
    label: &str,
    profile: &LegacyProtectedMediaProfileV1,
    claim: &str,
    principal: &LegacyProtectedMediaPrincipalV1,
    payload: &Value,
    now_ms: i64,
) -> std::result::Result<String, LegacyProtectedMediaFailureV1> {
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Err(LegacyProtectedMediaFailureV1::Invalid);
    }
    let payload =
        serde_json::to_vec(payload).map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
    let material = serde_json::to_vec(&json!({
        "operation_id": profile.operation_id,
        "principal_digest": principal.digest(),
        "payload_digest": digest(&payload),
        "claim_digest": digest(claim.as_bytes()),
        "generation": now_ms / LEGACY_PROTECTED_MEDIA_REPLAY_RETENTION_MS,
    }))
    .map_err(|_| LegacyProtectedMediaFailureV1::Invalid)?;
    Ok(format!(
        "server-{label}:{}:{}",
        profile.operation_id,
        digest(&material)
    ))
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProtectedMediaTerminalV1 {
    pub kind: LegacyProtectedMediaTerminalKindV1,
    pub status: u16,
    pub content_type: Option<String>,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for ProtectedMediaTerminalV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProtectedMediaTerminalV1")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("location", &self.location.as_ref().map(|_| "[REDACTED]"))
            .field("body", &"[REDACTED]")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResolvedProtectedMediaTerminalV1 {
    pub terminal: ProtectedMediaTerminalV1,
    pub plaintext_digest: String,
}

impl std::fmt::Debug for ResolvedProtectedMediaTerminalV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedProtectedMediaTerminalV1")
            .field("terminal", &self.terminal)
            .field("plaintext_digest", &self.plaintext_digest)
            .finish()
    }
}

pub(crate) trait ProtectedMediaTerminalResolverV1 {
    fn resolve(
        &self,
        opaque_ref: &str,
        expected_plaintext_digest: &str,
        expected_kind: LegacyProtectedMediaTerminalKindV1,
    ) -> std::result::Result<Option<ResolvedProtectedMediaTerminalV1>, LegacyProtectedMediaFailureV1>;
}

struct UnavailableProtectedMediaTerminalResolverV1;

impl ProtectedMediaTerminalResolverV1 for UnavailableProtectedMediaTerminalResolverV1 {
    fn resolve(
        &self,
        _opaque_ref: &str,
        _expected_plaintext_digest: &str,
        _expected_kind: LegacyProtectedMediaTerminalKindV1,
    ) -> std::result::Result<Option<ResolvedProtectedMediaTerminalV1>, LegacyProtectedMediaFailureV1>
    {
        Ok(None)
    }
}

impl ProtectedMediaTerminalV1 {
    fn plaintext_digest(&self) -> String {
        let mut material = Vec::new();
        material.extend_from_slice(b"frame.protected-media-terminal.v1\0");
        push_digest_part(&mut material, self.kind.as_str().as_bytes());
        push_digest_part(&mut material, self.status.to_string().as_bytes());
        push_digest_part(
            &mut material,
            self.content_type.as_deref().unwrap_or_default().as_bytes(),
        );
        push_digest_part(
            &mut material,
            self.location.as_deref().unwrap_or_default().as_bytes(),
        );
        push_digest_part(&mut material, &self.body);
        digest(&material)
    }

    fn validate(&self) -> std::result::Result<(), LegacyProtectedMediaFailureV1> {
        if self.body.len() > MAX_TERMINAL_BODY_BYTES
            || self
                .content_type
                .as_deref()
                .is_some_and(invalid_header_value)
            || self.location.as_deref().is_some_and(invalid_header_value)
        {
            return Err(LegacyProtectedMediaFailureV1::Corrupt);
        }
        let media_type = self
            .content_type
            .as_deref()
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match self.kind {
            LegacyProtectedMediaTerminalKindV1::Json => {
                if !(200..=599).contains(&self.status)
                    || self.location.is_some()
                    || media_type != "application/json"
                    || serde_json::from_slice::<Value>(&self.body).is_err()
                {
                    return Err(LegacyProtectedMediaFailureV1::Corrupt);
                }
            }
            LegacyProtectedMediaTerminalKindV1::Redirect => {
                if !(300..=399).contains(&self.status)
                    || self.status == 304
                    || !self.body.is_empty()
                    || !self.location.as_deref().is_some_and(|value| {
                        value.starts_with("https://")
                            || (value.starts_with('/') && !value.starts_with("//"))
                    })
                {
                    return Err(LegacyProtectedMediaFailureV1::Corrupt);
                }
            }
            LegacyProtectedMediaTerminalKindV1::Binary => {
                if !(200..=299).contains(&self.status)
                    || self.location.is_some()
                    || !(media_type.starts_with("image/")
                        || media_type.starts_with("audio/")
                        || media_type.starts_with("video/")
                        || media_type == "application/octet-stream")
                {
                    return Err(LegacyProtectedMediaFailureV1::Corrupt);
                }
            }
            LegacyProtectedMediaTerminalKindV1::EventStream => {
                if self.status != 200
                    || self.location.is_some()
                    || media_type != "text/event-stream"
                    || std::str::from_utf8(&self.body).is_err()
                {
                    return Err(LegacyProtectedMediaFailureV1::Corrupt);
                }
            }
        }
        Ok(())
    }

    fn json_value(&self) -> std::result::Result<Value, LegacyProtectedMediaFailureV1> {
        self.validate()?;
        if self.kind != LegacyProtectedMediaTerminalKindV1::Json {
            return Err(LegacyProtectedMediaFailureV1::Corrupt);
        }
        serde_json::from_slice(&self.body).map_err(|_| LegacyProtectedMediaFailureV1::Corrupt)
    }

    fn into_worker_response(self, head_only: bool) -> Result<Response> {
        let mut response = if head_only || self.body.is_empty() {
            Response::empty()?.with_status(self.status)
        } else {
            Response::from_body(ResponseBody::Body(self.body))?.with_status(self.status)
        };
        if let Some(content_type) = self.content_type {
            response.headers_mut().set("content-type", &content_type)?;
        }
        if let Some(location) = self.location {
            response.headers_mut().set("location", &location)?;
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

fn resolve_sealed_terminal(
    expected_kind: LegacyProtectedMediaTerminalKindV1,
    sealed_terminal_ref: &str,
    sealed_terminal_digest: &str,
    resolver: &dyn ProtectedMediaTerminalResolverV1,
) -> std::result::Result<ProtectedMediaTerminalV1, LegacyProtectedMediaFailureV1> {
    if !valid_opaque_ref("frame-pm-terminal-v1:", sealed_terminal_ref)
        || !valid_lower_digest(sealed_terminal_digest)
    {
        return Err(LegacyProtectedMediaFailureV1::Corrupt);
    }
    let resolved = resolver
        .resolve(sealed_terminal_ref, sealed_terminal_digest, expected_kind)?
        .ok_or(LegacyProtectedMediaFailureV1::ExecutionEvidenceRequired)?;
    if resolved.plaintext_digest != sealed_terminal_digest
        || resolved.terminal.kind != expected_kind
        || resolved.terminal.plaintext_digest() != sealed_terminal_digest
    {
        return Err(LegacyProtectedMediaFailureV1::Corrupt);
    }
    resolved.terminal.validate()?;
    Ok(resolved.terminal)
}

fn stage_response(
    outcome: std::result::Result<LegacyProtectedMediaStageOutcomeV1, LegacyProtectedMediaFailureV1>,
    head_only: bool,
    resolver: &dyn ProtectedMediaTerminalResolverV1,
) -> Result<Response> {
    match outcome {
        Ok(LegacyProtectedMediaStageOutcomeV1::ExecutionEvidenceRequired {
            receipt_id,
            replayed,
            provider_execution_required,
        }) => {
            let mut response = if head_only {
                Response::empty()?.with_status(503)
            } else {
                json_status(
                    503,
                    json!({
                        "error": "Execution evidence required",
                        "code": "EXECUTION_EVIDENCE_REQUIRED",
                        "receiptId": receipt_id,
                        "replayed": replayed,
                        "requiredEvidence": {
                            "hardwareExecution": true,
                            "providerExecution": provider_execution_required,
                        },
                    }),
                )?
            };
            response.headers_mut().set("cache-control", "no-store")?;
            response.headers_mut().set("retry-after", "15")?;
            response
                .headers_mut()
                .set("x-frame-execution-receipt", &receipt_id)?;
            Ok(response)
        }
        Ok(LegacyProtectedMediaStageOutcomeV1::VerifiedSealedTerminal {
            terminal_kind,
            sealed_terminal_ref,
            sealed_terminal_digest,
            ..
        }) => match resolve_sealed_terminal(
            terminal_kind,
            &sealed_terminal_ref,
            &sealed_terminal_digest,
            resolver,
        ) {
            Ok(terminal) => terminal.into_worker_response(head_only),
            Err(failure) => failure_response(failure, head_only),
        },
        Err(failure) => failure_response(failure, head_only),
    }
}

fn failure_response(failure: LegacyProtectedMediaFailureV1, head_only: bool) -> Result<Response> {
    let (status, code) = match failure {
        LegacyProtectedMediaFailureV1::Invalid => (400, "INVALID_REQUEST"),
        LegacyProtectedMediaFailureV1::Conflict => (409, "IDEMPOTENCY_CONFLICT"),
        LegacyProtectedMediaFailureV1::ExecutionEvidenceRequired => {
            (503, "EXECUTION_EVIDENCE_REQUIRED")
        }
        LegacyProtectedMediaFailureV1::Corrupt | LegacyProtectedMediaFailureV1::Unavailable => {
            (503, "UNAVAILABLE")
        }
    };
    if head_only {
        Ok(Response::empty()?.with_status(status))
    } else {
        json_status(status, json!({"error":code, "code":code}))
    }
}

fn json_status(status: u16, value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(status);
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}

fn constant_time_equal(actual: &str, expected: &str) -> bool {
    type HmacSha256 = Hmac<Sha256>;
    const COMPARISON_KEY: &[u8] = b"frame.protected-media.compare.v1";

    let Ok(mut expected_mac) = HmacSha256::new_from_slice(COMPARISON_KEY) else {
        return false;
    };
    expected_mac.update(expected.as_bytes());
    let expected_tag = expected_mac.finalize().into_bytes();

    let Ok(mut actual_mac) = HmacSha256::new_from_slice(COMPARISON_KEY) else {
        return false;
    };
    actual_mac.update(actual.as_bytes());
    actual.len() == expected.len() && actual_mac.verify_slice(&expected_tag).is_ok()
}

fn env_value(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
        .filter(|value| !value.is_empty())
}

fn invalid_header_value(value: &str) -> bool {
    value.is_empty()
        || value.len() > 8_192
        || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn push_digest_part(material: &mut Vec<u8>, value: &[u8]) {
    material.extend_from_slice(&(value.len() as u64).to_be_bytes());
    material.extend_from_slice(value);
}

fn valid_lower_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_opaque_ref(prefix: &str, value: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(valid_lower_digest)
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

    #[test]
    fn secret_comparison_is_exact_and_constant_time_primitive_backed() {
        assert!(constant_time_equal("secret", "secret"));
        assert!(!constant_time_equal("secret", "Secret"));
        assert!(!constant_time_equal("secret-prefix", "secret"));
    }

    #[test]
    fn callable_kinds_are_not_interchangeable() {
        let action = legacy_protected_media_profile("cap-v1-24ef9eb18c4b0555")
            .expect("checked-in action profile");
        assert_eq!(action.kind, LegacyProtectedMediaKindV1::ServerAction);
        assert_ne!(action.kind, LegacyProtectedMediaKindV1::Workflow);
    }

    #[test]
    fn workflow_principal_inherits_exact_session_policy_and_entitlement() {
        let profile = legacy_protected_media_profile("cap-v1-3e0dec6125f270bf")
            .expect("checked-in AI workflow profile");
        let parent = WorkflowParentRowV1 {
            source_operation_id: "cap-v1-c1ae43fcf8ad7018".into(),
            actor_id: Some("11111111-1111-4111-8111-111111111111".into()),
            tenant_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
            target_id: Some("123456789abcdeg".into()),
            credential_kind: "session_token".into(),
            credential_subject_id: Some("10000000-0000-4000-8000-000000000001".into()),
            credential_key_version: Some(7),
            credential_digest: Some("a".repeat(64)),
            policy_proofs_json: serde_json::to_string(&vec![LegacyProtectedMediaPolicyProofV1 {
                target_id: "123456789abcdeg".into(),
                kind: "owner_bypass".into(),
                subject_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
                revision: 9,
                audit_digest: "b".repeat(64),
            }])
            .expect("policy proof JSON"),
            entitlement_kind: Some("ai_owner".into()),
            entitlement_subject_id: Some("11111111-1111-4111-8111-111111111111".into()),
            entitlement_revision: Some(3),
            entitlement_expires_at_ms: Some(9_000_000),
            authority_binding_digest: "c".repeat(64),
            created_at_ms: 4_000,
            target_binding_rule: "same".into(),
            translated_legacy_target_id: None,
        };
        let principal = workflow_principal(
            profile,
            "protected_media",
            "50000000-0000-4000-8000-000000000021",
            &parent,
            &json!({
                "videoId": "123456789abcdeg",
                "userId": "11111111-1111-4111-8111-111111111111"
            }),
        )
        .expect("exact inherited workflow principal");
        assert_eq!(principal.class, "parent_derived");
        assert_eq!(principal.actor_id, parent.actor_id);
        assert_eq!(principal.tenant_id, parent.tenant_id);
        assert_eq!(principal.credential_kind, "session_token");
        assert_eq!(
            principal.credential_subject_id,
            parent.credential_subject_id
        );
        assert_eq!(principal.credential_key_version, Some(7));
        assert_eq!(principal.credential_digest, parent.credential_digest);
        assert_eq!(principal.policy_proofs[0].target_id, "123456789abcdeg");
        assert_eq!(
            principal
                .entitlement_binding
                .as_ref()
                .map(|binding| binding.revision),
            Some(3)
        );
    }

    #[test]
    fn anonymous_integration_parent_becomes_alias_bound_parent_capability() {
        let profile = legacy_protected_media_profile("cap-v1-39c33826cf514552")
            .expect("checked-in finalization workflow profile");
        let parent_receipt_id = "50000000-0000-4000-8000-000000000030";
        let parent = WorkflowParentRowV1 {
            source_operation_id: "cap-v1-d9b654b30f6c362a".into(),
            actor_id: None,
            tenant_id: None,
            target_id: Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into()),
            credential_kind: "none".into(),
            credential_subject_id: None,
            credential_key_version: None,
            credential_digest: None,
            policy_proofs_json: serde_json::to_string(&vec![LegacyProtectedMediaPolicyProofV1 {
                target_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
                kind: "unprotected_video_policy".into(),
                subject_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
                revision: 3,
                audit_digest: "d".repeat(64),
            }])
            .expect("policy proof JSON"),
            entitlement_kind: None,
            entitlement_subject_id: None,
            entitlement_revision: None,
            entitlement_expires_at_ms: None,
            authority_binding_digest: "e".repeat(64),
            created_at_ms: 4_000,
            target_binding_rule: "child_derived".into(),
            translated_legacy_target_id: Some("123456789abcdeg".into()),
        };
        let principal = workflow_principal(
            profile,
            "protected_integrations",
            parent_receipt_id,
            &parent,
            &json!({
                "videoId": "123456789abcdeg",
                "userId": "11111111-1111-4111-8111-111111111111"
            }),
        )
        .expect("alias-bound anonymous workflow principal");
        assert_eq!(principal.credential_kind, "parent_capability");
        assert_eq!(
            principal.credential_subject_id.as_deref(),
            Some("protected_integrations:50000000-0000-4000-8000-000000000030")
        );
        assert_eq!(principal.credential_key_version, Some(4_000));
        assert_eq!(principal.credential_digest, Some("e".repeat(64)));
        assert_eq!(principal.policy_proofs[0].target_id, "123456789abcdeg");

        assert!(matches!(
            workflow_principal(
                profile,
                "protected_integrations",
                parent_receipt_id,
                &parent,
                &json!({
                    "videoId": "223456789abcdeg",
                    "userId": "11111111-1111-4111-8111-111111111111"
                }),
            ),
            Err(LegacyProtectedMediaFailureV1::Unavailable)
        ));
    }

    #[test]
    fn effect_rpc_decoder_is_exact_bounded_and_tag_scoped() {
        let decoded = decode_protected_media_rpc(
            br#"{"_tag":"Request","id":"7","tag":"VideosGetThumbnails","payload":["video-1"],"headers":[]}"#,
        )
        .expect("protected media RPC");
        assert_eq!(decoded.id, "7");
        assert_eq!(decoded.payload, json!(["video-1"]));
        assert!(is_protected_media_rpc_request(
            br#"{"_tag":"Request","id":"7","tag":"VideosGetThumbnails","payload":[],"headers":[]}"#
        ));
        assert!(!is_protected_media_rpc_request(
            br#"{"_tag":"Request","id":"7","tag":"VideosGetAnalytics","payload":[],"headers":[]}"#
        ));
        let too_many = json!({
            "_tag": "Request",
            "id": "8",
            "tag": PROTECTED_MEDIA_RPC_TAG,
            "payload": (0..51).map(|index| format!("video-{index}")).collect::<Vec<_>>(),
            "headers": [],
        });
        assert!(matches!(
            decode_protected_media_rpc(&serde_json::to_vec(&too_many).expect("encode")),
            Err(ProtectedMediaRpcDecodeFailureV1::Malformed(Some(id))) if id == "8"
        ));
    }

    #[test]
    fn server_action_selector_owns_exactly_the_profiled_action_family() {
        assert_eq!(
            frame_application::LEGACY_PROTECTED_MEDIA_PROFILES
                .iter()
                .filter(|profile| is_server_action(profile.operation_id))
                .count(),
            7
        );
        assert!(is_server_action("cap-v1-24ef9eb18c4b0555"));
        assert!(!is_server_action(PROTECTED_MEDIA_RPC_OPERATION_ID));
        assert!(!is_server_action("cap-v1-not-protected-media"));
    }
}
