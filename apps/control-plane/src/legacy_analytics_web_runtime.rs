//! HTTP, Effect RPC, and compatibility-action carriers for Cap analytics.
//!
//! The D1 adapter below is intentionally not a Tinybird substitute. Six
//! provider-backed operations durably stage their exact query/event intents and
//! then stop at the `provider_execution` gate. The signup marker is the sole
//! provider-free operation and executes its conditional D1 update locally.

use frame_application::{
    LEGACY_ANALYTICS_MAX_BODY_BYTES, LEGACY_ANALYTICS_SIGNUP_OPERATION_ID,
    LEGACY_ANALYTICS_VIDEO_ACTION_OPERATION_ID, LegacyAnalyticsAdapterV1, LegacyAnalyticsErrorV1,
    LegacyAnalyticsInputV1, LegacyAnalyticsNetworkV1, LegacyAnalyticsOutcomeV1,
    LegacyAnalyticsRequestV1, LegacyAnalyticsResultV1, LegacyAnalyticsTrackInputV1,
    RateLimitDecisionV1,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{Env, Error, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_analytics_runtime::D1LegacyAnalyticsPortV1,
};

const ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-analytics-action-request.v1";
const RPC_TAG: &str = "VideosGetAnalytics";
const RPC_PARSE_FAILURE: &str = "An error has occurred";
const RPC_UNKNOWN_TAG_FAILURE: &str = "Unknown request tag";
const CONTENT_TYPE: &str = "application/json; charset=utf-8";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyAnalyticsHttpRouteV1 {
    VideoCount,
    Track,
    Dashboard,
    VideoHttp,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DecodedLegacyAnalyticsActionV1 {
    Signup {
        idempotency_key: Option<String>,
    },
    Video {
        video_id: String,
        range_days: Option<f64>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackWireV1 {
    #[serde(default)]
    video_id: Option<String>,
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    owner_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    pathname: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    occurred_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignupActionWireV1 {
    schema_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoActionWireV1 {
    schema_version: String,
    video_id: String,
    #[serde(default)]
    range_days: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedRpcV1 {
    id: String,
    video_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RpcDecodeFailureV1 {
    Malformed(Option<String>),
    UnknownTag,
}

pub(crate) async fn http_response(
    route: LegacyAnalyticsHttpRouteV1,
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    match route {
        LegacyAnalyticsHttpRouteV1::VideoCount => video_count_response(request, env, now_ms).await,
        LegacyAnalyticsHttpRouteV1::Track => track_response(request, env, now_ms).await,
        LegacyAnalyticsHttpRouteV1::Dashboard => dashboard_response(request, env, now_ms).await,
        LegacyAnalyticsHttpRouteV1::VideoHttp => video_http_response(request, env, now_ms).await,
    }
}

async fn video_count_response(request: &Request, env: &Env, now_ms: i64) -> Result<Response> {
    if request.headers().get("idempotency-key")?.is_some() {
        return exact_json(400, json!({"error": "Invalid analytics request"}));
    }
    let Some(video_id) = query_parameter(request, "videoId")?.filter(|value| !value.is_empty())
    else {
        return exact_json(400, json!({"error": "Video ID is required"}));
    };
    let range = query_parameter(request, "range")?;
    let actor_id = match optional_actor(request, env, now_ms).await? {
        Ok(actor_id) => actor_id,
        Err(failure) => return browser_failure_response(failure),
    };
    if !admit_optional(
        request,
        env,
        actor_id.as_deref(),
        CompatibilityRateLimitBucketV1::SharePlayback,
        now_ms,
    )
    .await?
    {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    let password_grant_digest =
        issue_password_grant(request, env, std::slice::from_ref(&video_id), now_ms).await?;
    let outcome = execute(
        env,
        request_value(
            actor_id,
            None,
            password_grant_digest,
            None,
            now_ms,
            empty_network(),
            LegacyAnalyticsInputV1::VideoCount { video_id, range },
        ),
    )
    .await?;
    project_http_outcome(LegacyAnalyticsHttpRouteV1::VideoCount, outcome)
}

async fn video_http_response(request: &Request, env: &Env, now_ms: i64) -> Result<Response> {
    if request.headers().get("idempotency-key")?.is_some() {
        return exact_json(400, json!({"error": "Invalid analytics request"}));
    }
    let Some(video_id) = query_parameter(request, "videoId")?.filter(|value| !value.is_empty())
    else {
        return exact_json(400, json!({"error": "Video ID is required"}));
    };
    let actor_id = match optional_actor(request, env, now_ms).await? {
        Ok(actor_id) => actor_id,
        Err(failure) => return browser_failure_response(failure),
    };
    if !admit_optional(
        request,
        env,
        actor_id.as_deref(),
        CompatibilityRateLimitBucketV1::SharePlayback,
        now_ms,
    )
    .await?
    {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    let password_grant_digest =
        issue_password_grant(request, env, std::slice::from_ref(&video_id), now_ms).await?;
    let outcome = execute(
        env,
        request_value(
            actor_id,
            None,
            password_grant_digest,
            None,
            now_ms,
            empty_network(),
            LegacyAnalyticsInputV1::VideoHttp { video_id },
        ),
    )
    .await?;
    project_http_outcome(LegacyAnalyticsHttpRouteV1::VideoHttp, outcome)
}

async fn dashboard_response(request: &Request, env: &Env, now_ms: i64) -> Result<Response> {
    if request.headers().get("idempotency-key")?.is_some() {
        return exact_json(400, json!({"error": "Invalid analytics request"}));
    }
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unavailable) => {
                return exact_json(500, json!({"error": "Failed to load analytics"}));
            }
            Err(_) => return exact_json(401, json!({"error": "Unauthorized"})),
        };
    let database = env.d1("DB")?;
    let active_organization_id =
        browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await?;
    let requested_organization_id = query_parameter(request, "orgId")?;
    if requested_organization_id.as_deref() == Some("")
        || active_organization_id.as_deref().is_none_or(str::is_empty)
    {
        return exact_json(400, json!({"error": "No active organization"}));
    }
    if requested_organization_id
        .as_deref()
        .is_some_and(|requested| Some(requested) != active_organization_id.as_deref())
    {
        return exact_json(403, json!({"error": "Forbidden"}));
    }
    let admitted = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        now_ms,
    )
    .await?;
    if rejected(admitted) {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    let outcome = execute_with_database(
        &database,
        request_value(
            Some(actor_id),
            active_organization_id,
            None,
            None,
            now_ms,
            empty_network(),
            LegacyAnalyticsInputV1::Dashboard {
                requested_organization_id,
                space_id: query_parameter(request, "spaceId")?,
                video_id: query_parameter(request, "capId")?,
                range: query_parameter(request, "range")?,
            },
        ),
    )
    .await;
    project_http_outcome(LegacyAnalyticsHttpRouteV1::Dashboard, outcome)
}

async fn track_response(request: &mut Request, env: &Env, now_ms: i64) -> Result<Response> {
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_ANALYTICS_MAX_BODY_BYTES).await {
            Ok(bytes) => bytes,
            Err(()) => return exact_json(400, json!({"error": "Invalid JSON payload"})),
        };
    let wire = match decode_track_wire(&bytes) {
        Ok(wire) => wire,
        Err(()) => return exact_json(400, json!({"error": "Invalid JSON payload"})),
    };
    let Some(video_id) = wire.video_id.filter(|value| !value.is_empty()) else {
        return exact_json(400, json!({"error": "videoId is required"}));
    };
    let occurred_at_iso = match wire.occurred_at.as_deref() {
        Some(value) => match canonicalize_date(value) {
            Some(value) => Some(value),
            None => return exact_json(400, json!({"error": "Invalid JSON payload"})),
        },
        None => None,
    };
    let actor_id = match optional_actor(request, env, now_ms).await? {
        Ok(actor_id) => actor_id,
        Err(failure) => return browser_failure_response(failure),
    };
    if !admit_optional(
        request,
        env,
        actor_id.as_deref(),
        CompatibilityRateLimitBucketV1::SharePlayback,
        now_ms,
    )
    .await?
    {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    let url = request.url()?;
    let network = LegacyAnalyticsNetworkV1 {
        user_agent: request.headers().get("user-agent")?,
        request_hostname: url.host_str().map(str::to_owned),
        country: request.headers().get("x-vercel-ip-country")?,
        region: request.headers().get("x-vercel-ip-country-region")?,
        encoded_city: request.headers().get("x-vercel-ip-city")?,
    };
    let outcome = execute(
        env,
        request_value(
            actor_id,
            None,
            None,
            request.headers().get("idempotency-key")?,
            now_ms,
            network,
            LegacyAnalyticsInputV1::Track(LegacyAnalyticsTrackInputV1 {
                video_id,
                organization_id: wire.org_id,
                owner_id: wire.owner_id,
                session_id: wire.session_id,
                pathname: wire.pathname,
                hostname: wire.hostname,
                body_user_agent: wire.user_agent,
                occurred_at_iso,
            }),
        ),
    )
    .await?;
    project_http_outcome(LegacyAnalyticsHttpRouteV1::Track, outcome)
}

#[must_use]
pub(crate) fn is_analytics_rpc_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("tag").and_then(Value::as_str).map(str::to_owned))
        .as_deref()
        == Some(RPC_TAG)
}

pub(crate) async fn effect_rpc_response_from_bytes(
    bytes: &[u8],
    request: &Request,
    env: &Env,
    _request_id: &str,
) -> Result<Response> {
    let decoded = match decode_rpc_request(bytes) {
        Ok(decoded) => decoded,
        Err(RpcDecodeFailureV1::Malformed(Some(id))) => {
            return rpc_response(rpc_die(&id, RPC_PARSE_FAILURE));
        }
        Err(RpcDecodeFailureV1::Malformed(None)) => {
            return rpc_response(rpc_defect(RPC_PARSE_FAILURE));
        }
        Err(RpcDecodeFailureV1::UnknownTag) => {
            return rpc_response(rpc_defect(RPC_UNKNOWN_TAG_FAILURE));
        }
    };
    if decoded.video_ids.is_empty() {
        return rpc_response(rpc_success(&decoded.id, json!([])));
    }
    let now_ms = crate::current_time_ms()?;
    let actor_id = match optional_actor(request, env, now_ms).await? {
        Ok(actor_id) => actor_id,
        Err(BrowserWebFailure::Unavailable) => {
            return rpc_response(rpc_internal_failure(&decoded.id, "database"));
        }
        Err(_) => return rpc_response(rpc_internal_failure(&decoded.id, "unknown")),
    };
    if !admit_optional(
        request,
        env,
        actor_id.as_deref(),
        CompatibilityRateLimitBucketV1::SharePlayback,
        now_ms,
    )
    .await?
    {
        return rpc_response(rpc_internal_failure(&decoded.id, "unknown"));
    }
    let password_grant_digest =
        issue_password_grant(request, env, &decoded.video_ids, now_ms).await?;
    let outcome = execute(
        env,
        request_value(
            actor_id,
            None,
            password_grant_digest,
            None,
            now_ms,
            empty_network(),
            LegacyAnalyticsInputV1::VideoRpc {
                video_ids: decoded.video_ids,
            },
        ),
    )
    .await?;
    let value = match outcome {
        Ok(LegacyAnalyticsOutcomeV1 {
            result: LegacyAnalyticsResultV1::ProviderPending { .. },
            ..
        })
        | Err(LegacyAnalyticsErrorV1::ProviderRequired)
        | Err(LegacyAnalyticsErrorV1::Conflict)
        | Err(LegacyAnalyticsErrorV1::InvalidInput) => rpc_internal_failure(&decoded.id, "unknown"),
        Err(LegacyAnalyticsErrorV1::Unavailable | LegacyAnalyticsErrorV1::Internal) => {
            rpc_internal_failure(&decoded.id, "database")
        }
        Err(
            LegacyAnalyticsErrorV1::NotFound
            | LegacyAnalyticsErrorV1::Unauthorized
            | LegacyAnalyticsErrorV1::Forbidden,
        ) => rpc_internal_failure(&decoded.id, "unknown"),
        Ok(_) => rpc_internal_failure(&decoded.id, "unknown"),
    };
    rpc_response(value)
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    matches!(
        operation_id,
        LEGACY_ANALYTICS_SIGNUP_OPERATION_ID | LEGACY_ANALYTICS_VIDEO_ACTION_OPERATION_ID
    )
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedLegacyAnalyticsActionV1>> {
    if !is_action(operation_id)
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
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_ANALYTICS_MAX_BODY_BYTES).await {
            Ok(bytes) if !bytes.is_empty() => bytes,
            _ => return Ok(Err(BrowserWebFailure::Invalid)),
        };
    let decoded = if operation_id == LEGACY_ANALYTICS_SIGNUP_OPERATION_ID {
        let wire = match serde_json::from_slice::<SignupActionWireV1>(&bytes) {
            Ok(wire) if wire.schema_version == ACTION_REQUEST_SCHEMA_V1 => wire,
            _ => return Ok(Err(BrowserWebFailure::Invalid)),
        };
        let _ = wire;
        DecodedLegacyAnalyticsActionV1::Signup {
            idempotency_key: request.headers().get("idempotency-key")?,
        }
    } else {
        if request.headers().get("idempotency-key")?.is_some() {
            return Ok(Err(BrowserWebFailure::Invalid));
        }
        let wire = match serde_json::from_slice::<VideoActionWireV1>(&bytes) {
            Ok(wire)
                if wire.schema_version == ACTION_REQUEST_SCHEMA_V1 && !wire.video_id.is_empty() =>
            {
                wire
            }
            _ => return Ok(Err(BrowserWebFailure::Invalid)),
        };
        DecodedLegacyAnalyticsActionV1::Video {
            video_id: wire.video_id,
            range_days: wire.range_days,
        }
    };
    Ok(Ok(decoded))
}

pub(crate) async fn action_response(
    request: &Request,
    env: &Env,
    decoded: &DecodedLegacyAnalyticsActionV1,
    now_ms: i64,
) -> Result<Response> {
    match decoded {
        DecodedLegacyAnalyticsActionV1::Signup { idempotency_key } => {
            signup_action_response(request, env, idempotency_key.clone(), now_ms).await
        }
        DecodedLegacyAnalyticsActionV1::Video {
            video_id,
            range_days,
        } => video_action_response(request, env, video_id, *range_days, now_ms).await,
    }
}

async fn signup_action_response(
    request: &Request,
    env: &Env,
    idempotency_key: Option<String>,
    now_ms: i64,
) -> Result<Response> {
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unavailable) => {
                return exact_json(503, json!({"error": "Service unavailable"}));
            }
            Err(_) => return exact_json(200, json!({"shouldTrack": false})),
        };
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return browser_failure_response(failure),
    };
    if proof.user_id().to_string() != actor_id {
        return exact_json(503, json!({"error": "Service unavailable"}));
    }
    let database = env.d1("DB")?;
    let admitted = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::ServiceMisc,
        &actor_id,
        now_ms,
    )
    .await?;
    if rejected(admitted) {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    if !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, &proof, now_ms)
        .await?
    {
        return exact_json(503, json!({"error": "Service unavailable"}));
    }
    let outcome = execute_with_database(
        &database,
        request_value(
            Some(actor_id),
            None,
            None,
            idempotency_key,
            now_ms,
            empty_network(),
            LegacyAnalyticsInputV1::Signup,
        ),
    )
    .await;
    match outcome {
        Ok(LegacyAnalyticsOutcomeV1 {
            result: LegacyAnalyticsResultV1::SignupTracking { should_track },
            ..
        }) => exact_json(200, json!({"shouldTrack": should_track})),
        // Cap intentionally makes database and preference corruption failures
        // ineligible instead of surfacing an action error.
        Err(_) | Ok(_) => exact_json(200, json!({"shouldTrack": false})),
    }
}

async fn video_action_response(
    request: &Request,
    env: &Env,
    video_id: &str,
    range_days: Option<f64>,
    now_ms: i64,
) -> Result<Response> {
    let actor_id = match optional_actor(request, env, now_ms).await? {
        Ok(actor_id) => actor_id,
        Err(failure) => return browser_failure_response(failure),
    };
    if !admit_optional(
        request,
        env,
        actor_id.as_deref(),
        CompatibilityRateLimitBucketV1::SharePlayback,
        now_ms,
    )
    .await?
    {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    let video_ids = [video_id.to_owned()];
    let password_grant_digest = issue_password_grant(request, env, &video_ids, now_ms).await?;
    let outcome = execute(
        env,
        request_value(
            actor_id,
            None,
            password_grant_digest,
            None,
            now_ms,
            empty_network(),
            LegacyAnalyticsInputV1::VideoAction {
                video_id: video_id.to_owned(),
                range_days,
            },
        ),
    )
    .await?;
    match outcome {
        Ok(LegacyAnalyticsOutcomeV1 {
            result: LegacyAnalyticsResultV1::ProviderPending { .. },
            ..
        })
        | Err(LegacyAnalyticsErrorV1::ProviderRequired) => provider_gate_response(),
        Err(
            LegacyAnalyticsErrorV1::NotFound
            | LegacyAnalyticsErrorV1::Unauthorized
            | LegacyAnalyticsErrorV1::Forbidden,
        ) => exact_json(404, json!({"error": "Video not found"})),
        Err(LegacyAnalyticsErrorV1::InvalidInput) => {
            exact_json(400, json!({"error": "Invalid analytics request"}))
        }
        Err(_) | Ok(_) => exact_json(500, json!({"error": "Failed to fetch analytics"})),
    }
}

fn project_http_outcome(
    route: LegacyAnalyticsHttpRouteV1,
    outcome: std::result::Result<LegacyAnalyticsOutcomeV1, LegacyAnalyticsErrorV1>,
) -> Result<Response> {
    match outcome {
        Ok(LegacyAnalyticsOutcomeV1 {
            result: LegacyAnalyticsResultV1::ProviderPending { .. },
            ..
        })
        | Err(LegacyAnalyticsErrorV1::ProviderRequired) => provider_gate_response(),
        Ok(LegacyAnalyticsOutcomeV1 {
            result: LegacyAnalyticsResultV1::TrackSkipped,
            ..
        }) if route == LegacyAnalyticsHttpRouteV1::Track => {
            exact_json(200, json!({"success": true}))
        }
        Err(
            LegacyAnalyticsErrorV1::NotFound
            | LegacyAnalyticsErrorV1::Unauthorized
            | LegacyAnalyticsErrorV1::Forbidden,
        ) if matches!(
            route,
            LegacyAnalyticsHttpRouteV1::VideoCount | LegacyAnalyticsHttpRouteV1::VideoHttp
        ) =>
        {
            exact_json(404, json!({"error": "Video not found"}))
        }
        Err(LegacyAnalyticsErrorV1::Unauthorized)
            if route == LegacyAnalyticsHttpRouteV1::Dashboard =>
        {
            exact_json(401, json!({"error": "Unauthorized"}))
        }
        Err(LegacyAnalyticsErrorV1::Forbidden)
            if route == LegacyAnalyticsHttpRouteV1::Dashboard =>
        {
            exact_json(403, json!({"error": "Forbidden"}))
        }
        Err(LegacyAnalyticsErrorV1::NotFound) if route == LegacyAnalyticsHttpRouteV1::Dashboard => {
            exact_json(404, json!({"error": "Not found"}))
        }
        Err(LegacyAnalyticsErrorV1::InvalidInput) if route == LegacyAnalyticsHttpRouteV1::Track => {
            exact_json(400, json!({"error": "Invalid JSON payload"}))
        }
        Err(LegacyAnalyticsErrorV1::InvalidInput) => {
            exact_json(400, json!({"error": "Invalid analytics request"}))
        }
        Err(_) | Ok(_) if route == LegacyAnalyticsHttpRouteV1::Dashboard => {
            exact_json(500, json!({"error": "Failed to load analytics"}))
        }
        Err(_) | Ok(_) if route == LegacyAnalyticsHttpRouteV1::Track => {
            exact_json(500, json!({"error": "Failed to track analytics"}))
        }
        Err(_) | Ok(_) => exact_json(500, json!({"error": "Failed to fetch analytics"})),
    }
}

async fn optional_actor(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Option<String>>> {
    match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms).await? {
        Ok(actor_id) => Ok(Ok(Some(actor_id))),
        Err(BrowserWebFailure::Unauthenticated) => Ok(Ok(None)),
        Err(failure) => Ok(Err(failure)),
    }
}

async fn admit_optional(
    request: &Request,
    env: &Env,
    actor_id: Option<&str>,
    bucket: CompatibilityRateLimitBucketV1,
    now_ms: i64,
) -> Result<bool> {
    let database = env.d1("DB")?;
    let decision = match actor_id {
        Some(actor_id) => {
            compatibility_rate_limit::admit_principal(env, &database, bucket, actor_id, now_ms)
                .await?
        }
        None => compatibility_rate_limit::admit_edge_request(env, request, bucket, now_ms).await?,
    };
    Ok(!rejected(decision))
}

const fn rejected(decision: RateLimitDecisionV1) -> bool {
    matches!(decision, RateLimitDecisionV1::Rejected { .. })
}

async fn issue_password_grant(
    request: &Request,
    env: &Env,
    video_ids: &[String],
    now_ms: i64,
) -> Result<Option<String>> {
    let hashes = crate::legacy_video_properties_web_runtime::existing_password_hashes(request, env)
        .unwrap_or_default();
    if hashes.is_empty() {
        return Ok(None);
    }
    let grant_digest = format!(
        "{:x}",
        Sha256::digest(format!("{}:{now_ms}", Uuid::now_v7()).as_bytes())
    );
    let database = env.d1("DB")?;
    D1LegacyAnalyticsPortV1::new(&database)
        .issue_password_grant(video_ids, &hashes, &grant_digest, now_ms)
        .await
        .map_err(|_| Error::RustError("analytics password authority is unavailable".into()))?;
    Ok(Some(grant_digest))
}

async fn execute(
    env: &Env,
    request: LegacyAnalyticsRequestV1,
) -> Result<std::result::Result<LegacyAnalyticsOutcomeV1, LegacyAnalyticsErrorV1>> {
    let database = env.d1("DB")?;
    Ok(execute_with_database(&database, request).await)
}

async fn execute_with_database(
    database: &worker::D1Database,
    request: LegacyAnalyticsRequestV1,
) -> std::result::Result<LegacyAnalyticsOutcomeV1, LegacyAnalyticsErrorV1> {
    LegacyAnalyticsAdapterV1::new(&D1LegacyAnalyticsPortV1::new(database))
        .execute(request)
        .await
}

#[allow(clippy::too_many_arguments)]
fn request_value(
    actor_id: Option<String>,
    active_organization_id: Option<String>,
    password_grant_digest: Option<String>,
    idempotency_key: Option<String>,
    now_ms: i64,
    network: LegacyAnalyticsNetworkV1,
    input: LegacyAnalyticsInputV1,
) -> LegacyAnalyticsRequestV1 {
    LegacyAnalyticsRequestV1 {
        actor_id,
        active_organization_id,
        password_grant_digest,
        idempotency_key,
        now_ms,
        now_iso: iso_at(now_ms),
        network,
        input,
    }
}

const fn empty_network() -> LegacyAnalyticsNetworkV1 {
    LegacyAnalyticsNetworkV1 {
        user_agent: None,
        request_hostname: None,
        country: None,
        region: None,
        encoded_city: None,
    }
}

fn query_parameter(request: &Request, name: &str) -> Result<Option<String>> {
    let url = request.url()?;
    Ok(url
        .query_pairs()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.into_owned()))
}

fn decode_track_wire(bytes: &[u8]) -> std::result::Result<TrackWireV1, ()> {
    let value = serde_json::from_slice::<Value>(bytes).map_err(|_| ())?;
    if !value.is_object() {
        return Ok(TrackWireV1 {
            video_id: None,
            org_id: None,
            owner_id: None,
            session_id: None,
            pathname: None,
            hostname: None,
            user_agent: None,
            occurred_at: None,
        });
    }
    serde_json::from_value(value).map_err(|_| ())
}

fn canonicalize_date(value: &str) -> Option<String> {
    let timestamp = js_sys::Date::parse(value);
    timestamp
        .is_finite()
        .then(|| String::from(js_sys::Date::new(&JsValue::from_f64(timestamp)).to_iso_string()))
}

fn iso_at(now_ms: i64) -> String {
    String::from(js_sys::Date::new(&JsValue::from_f64(now_ms as f64)).to_iso_string())
}

fn decode_rpc_request(bytes: &[u8]) -> std::result::Result<DecodedRpcV1, RpcDecodeFailureV1> {
    if bytes.is_empty() || bytes.len() > LEGACY_ANALYTICS_MAX_BODY_BYTES {
        return Err(RpcDecodeFailureV1::Malformed(None));
    }
    let value =
        serde_json::from_slice::<Value>(bytes).map_err(|_| RpcDecodeFailureV1::Malformed(None))?;
    let object = value
        .as_object()
        .ok_or(RpcDecodeFailureV1::Malformed(None))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_rpc_id(value))
        .map(str::to_owned)
        .ok_or(RpcDecodeFailureV1::Malformed(None))?;
    let malformed = || RpcDecodeFailureV1::Malformed(Some(id.clone()));
    if object.get("_tag").and_then(Value::as_str) != Some("Request")
        || !valid_rpc_headers(object.get("headers"))
        || !valid_optional_string(object.get("traceId"))
        || !valid_optional_string(object.get("spanId"))
        || !valid_optional_bool(object.get("sampled"))
    {
        return Err(malformed());
    }
    if object.get("tag").and_then(Value::as_str) != Some(RPC_TAG) {
        return Err(RpcDecodeFailureV1::UnknownTag);
    }
    let payload = object
        .get("payload")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 50)
        .ok_or_else(malformed)?;
    let video_ids = payload
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| {
                    !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
                })
                .map(str::to_owned)
                .ok_or_else(malformed)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(DecodedRpcV1 { id, video_ids })
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

fn provider_gate_response() -> Result<Response> {
    exact_json(503, json!({"error": "provider_execution"}))
}

fn browser_failure_response(failure: BrowserWebFailure) -> Result<Response> {
    match failure {
        BrowserWebFailure::Unauthenticated => exact_json(401, json!({"error": "Unauthorized"})),
        BrowserWebFailure::Forbidden => exact_json(403, json!({"error": "Forbidden"})),
        BrowserWebFailure::Invalid => exact_json(400, json!({"error": "Invalid request"})),
        BrowserWebFailure::Conflict => exact_json(409, json!({"error": "Conflict"})),
        BrowserWebFailure::RateLimited => exact_json(429, json!({"error": "Too many requests"})),
        BrowserWebFailure::NotFound => exact_json(404, json!({"error": "Not found"})),
        BrowserWebFailure::Unavailable => exact_json(503, json!({"error": "Service unavailable"})),
    }
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

fn rpc_response(value: Value) -> Result<Response> {
    exact_json(200, value)
}

fn exact_json(status: u16, value: Value) -> Result<Response> {
    let body = serde_json::to_vec(&value)
        .map_err(|_| Error::RustError("legacy analytics response is invalid".into()))?;
    let mut response = Response::from_bytes(body)?.with_status(status);
    response.headers_mut().set("content-type", CONTENT_TYPE)?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_carrier_accepts_cap_wire_and_preserves_optional_fields() {
        let wire = decode_track_wire(
            br#"{"videoId":"video","orgId":"org","ownerId":null,"sessionId":" anon ","pathname":"/s/video","hostname":"watch.example","userAgent":"UA","occurredAt":"2026-03-04T00:00:00.000Z","ignored":true}"#,
        )
        .expect("track wire");
        assert_eq!(wire.video_id.as_deref(), Some("video"));
        assert_eq!(wire.org_id.as_deref(), Some("org"));
        assert_eq!(wire.owner_id, None);
        assert_eq!(wire.session_id.as_deref(), Some(" anon "));
        assert_eq!(wire.hostname.as_deref(), Some("watch.example"));
    }

    #[test]
    fn rpc_carrier_accepts_empty_and_fifty_but_rejects_fifty_one() {
        let empty = decode_rpc_request(
            br#"{"_tag":"Request","id":"7","tag":"VideosGetAnalytics","payload":[],"headers":[]}"#,
        )
        .expect("empty request");
        assert!(empty.video_ids.is_empty());

        let fifty = json!({
            "_tag": "Request",
            "id": "8",
            "tag": RPC_TAG,
            "payload": (0..50).map(|index| format!("video-{index}")).collect::<Vec<_>>(),
            "headers": [],
        });
        assert_eq!(
            decode_rpc_request(&serde_json::to_vec(&fifty).expect("encode"))
                .expect("fifty")
                .video_ids
                .len(),
            50
        );
        let fifty_one = json!({
            "_tag": "Request",
            "id": "9",
            "tag": RPC_TAG,
            "payload": (0..51).map(|index| format!("video-{index}")).collect::<Vec<_>>(),
            "headers": [],
        });
        assert!(matches!(
            decode_rpc_request(&serde_json::to_vec(&fifty_one).expect("encode")),
            Err(RpcDecodeFailureV1::Malformed(Some(id))) if id == "9"
        ));
    }

    #[test]
    fn rpc_carrier_is_closed_to_other_tags_and_malformed_headers() {
        assert!(!is_analytics_rpc_request(
            br#"{"_tag":"Request","id":"1","tag":"VideoDelete","payload":[],"headers":[]}"#
        ));
        assert!(matches!(
            decode_rpc_request(
                br#"{"_tag":"Request","id":"1","tag":"Unknown","payload":[],"headers":[]}"#
            ),
            Err(RpcDecodeFailureV1::UnknownTag)
        ));
        assert!(matches!(
            decode_rpc_request(
                br#"{"_tag":"Request","id":"2","tag":"VideosGetAnalytics","payload":[],"headers":{}}"#
            ),
            Err(RpcDecodeFailureV1::Malformed(Some(id))) if id == "2"
        ));
    }

    #[test]
    fn action_carrier_owns_exactly_the_two_analytics_ids() {
        assert!(is_action(LEGACY_ANALYTICS_SIGNUP_OPERATION_ID));
        assert!(is_action(LEGACY_ANALYTICS_VIDEO_ACTION_OPERATION_ID));
        assert!(!is_action("cap-v1-not-analytics"));

        let signup: SignupActionWireV1 = serde_json::from_slice(
            br#"{"schema_version":"frame.web-analytics-action-request.v1"}"#,
        )
        .expect("signup wire");
        assert_eq!(signup.schema_version, ACTION_REQUEST_SCHEMA_V1);
        let video: VideoActionWireV1 = serde_json::from_slice(
            br#"{"schema_version":"frame.web-analytics-action-request.v1","video_id":"video","range_days":7.9}"#,
        )
        .expect("video wire");
        assert_eq!(video.video_id, "video");
        assert_eq!(video.range_days, Some(7.9));
    }

    #[test]
    fn protected_rpc_projection_never_fabricates_provider_success() {
        assert_eq!(
            rpc_internal_failure("4", "unknown"),
            json!([{
                "_tag": "Exit",
                "requestId": "4",
                "exit": {
                    "_tag": "Failure",
                    "cause": {
                        "_tag": "Fail",
                        "error": {"_tag": "InternalError", "type": "unknown"}
                    }
                }
            }])
        );
    }
}
