//! Exact ingress for Cap's retained collaboration mutations.
//!
//! Three mobile routes preserve Cap's session-or-36-character-API-key
//! boundary, the legacy web DELETE route preserves its unusual missing-data
//! responses, and two authenticated action carriers preserve the source action
//! inputs and projections. All six operations converge only after their wire
//! contracts have been decoded and admitted independently.

use frame_application::{
    LEGACY_COLLABORATION_CONTENT_TYPE, LEGACY_COLLABORATION_MAX_BODY_BYTES,
    LEGACY_WEB_DELETE_COMMENT_ACTION_OPERATION_ID, LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID,
    LegacyCallerV1, LegacyCollaborationCredentialV1, LegacyCollaborationErrorV1,
    LegacyCollaborationInputV1, LegacyCollaborationSuccessV1, RateLimitDecisionV1,
    RequestSecurityContextV1,
};
use frame_domain::{
    ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1,
    IdempotencyKey,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{Env, Error, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_collaboration_runtime::D1LegacyCollaborationAtomicPortV1,
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCollaborationInvocationV1,
        LegacyCompatibilityTransportV1,
    },
};

pub const WEB_COLLABORATION_ACTION_REQUEST_SCHEMA_V1: &str =
    "frame.web-collaboration-action-request.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollaborationActionV1 {
    Delete,
    New,
}

impl CollaborationActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_WEB_DELETE_COMMENT_ACTION_OPERATION_ID => Some(Self::Delete),
            LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID => Some(Self::New),
            _ => None,
        }
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    CollaborationActionV1::parse(operation_id).is_some()
}

#[derive(Debug, Clone, PartialEq)]
struct RequiredNullableString(Option<String>);

impl<'de> Deserialize<'de> for RequiredNullableString {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::Null => Ok(Self(None)),
            Value::String(value) => Ok(Self(Some(value))),
            _ => Err(serde::de::Error::custom("expected string or null")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RequiredNullableNumber(Option<f64>);

impl<'de> Deserialize<'de> for RequiredNullableNumber {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::Null => Ok(Self(None)),
            Value::Number(value) => value
                .as_f64()
                .map(|value| Self(Some(value)))
                .ok_or_else(|| serde::de::Error::custom("expected finite number or null")),
            _ => Err(serde::de::Error::custom("expected number or null")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteActionWireV1 {
    schema_version: String,
    comment_id: String,
    parent_id: RequiredNullableString,
    video_id: String,
    idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewActionWireV1 {
    schema_version: String,
    content: String,
    video_id: String,
    #[serde(rename = "type")]
    kind: String,
    author_image: RequiredNullableString,
    parent_comment_id: String,
    timestamp: RequiredNullableNumber,
    idempotency_key: String,
}

#[derive(Clone, PartialEq)]
pub struct DecodedCollaborationActionV1 {
    action: CollaborationActionV1,
    input: LegacyCollaborationInputV1,
    idempotency_key: String,
    body_length: u64,
}

impl std::fmt::Debug for DecodedCollaborationActionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedCollaborationActionV1")
            .field("action", &self.action)
            .field("input", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("body_length", &self.body_length)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebCollaborationActionEffectV1 {
    SuccessObject,
    Comment(Box<WebCollaborationCommentEffectV1>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebCollaborationCommentEffectV1 {
    pub id: String,
    pub author_id: String,
    pub kind: &'static str,
    pub content: String,
    pub video_id: String,
    pub timestamp: Option<f64>,
    pub parent_comment_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub author_name: Option<String>,
    pub author_image: Option<String>,
    pub sending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CollaborationAuthorityV1 {
    actor_id: String,
    organization_id: String,
    credential: LegacyCollaborationCredentialV1,
}

struct CollaborationDispatchV1 {
    caller: LegacyCallerV1,
    authority: CollaborationAuthorityV1,
    input: LegacyCollaborationInputV1,
    idempotency_key: String,
    content_length: u64,
    content_type: Option<String>,
    security: RequestSecurityContextV1,
}

#[derive(Debug, Deserialize)]
struct ApiKeyActorRowV1 {
    user_id: String,
}

pub async fn mobile_create_comment_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    video_id: String,
) -> Result<Response> {
    let (input, body_length) = match decode_mobile_create(request, video_id, false).await? {
        Ok(value) => value,
        Err(error) => return mobile_error_response(error),
    };
    mobile_mutation_response(request, env, request_id, input, body_length).await
}

pub async fn mobile_create_reaction_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    video_id: String,
) -> Result<Response> {
    let (input, body_length) = match decode_mobile_create(request, video_id, true).await? {
        Ok(value) => value,
        Err(error) => return mobile_error_response(error),
    };
    mobile_mutation_response(request, env, request_id, input, body_length).await
}

pub async fn mobile_delete_comment_response(
    request: &Request,
    env: &Env,
    request_id: &str,
    comment_id: String,
) -> Result<Response> {
    if !empty_body_headers(request)? {
        return mobile_error_response(LegacyCollaborationErrorV1::InvalidInput);
    }
    mobile_mutation_response(
        request,
        env,
        request_id,
        LegacyCollaborationInputV1::MobileDeleteComment {
            legacy_comment_id: comment_id,
        },
        0,
    )
    .await
}

async fn mobile_mutation_response(
    request: &Request,
    env: &Env,
    request_id: &str,
    input: LegacyCollaborationInputV1,
    body_length: u64,
) -> Result<Response> {
    let idempotency_key = match required_idempotency_key(request)? {
        Ok(value) => value,
        Err(error) => return mobile_error_response(error),
    };
    let now_ms = crate::current_time_ms()?;
    let authority = match authenticate_mobile(request, env, now_ms).await? {
        Ok(value) => value,
        Err(error) => return mobile_error_response(error),
    };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        &authority.actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return tagged_http_error(429, "TooManyRequests");
    }
    let execution = dispatch(
        &database,
        request_id,
        CollaborationDispatchV1 {
            caller: mobile_caller(),
            authority,
            input,
            idempotency_key,
            content_length: body_length,
            content_type: if body_length == 0 {
                None
            } else {
                Some(LEGACY_COLLABORATION_CONTENT_TYPE.into())
            },
            security: admitted_security(rate_limit),
        },
    )
    .await;
    match execution {
        Ok(LegacyCollaborationSuccessV1::MobileComment(comment)) => json_http_response(
            200,
            &json!({
                "id": comment.id,
                "videoId": comment.video_id,
                "type": comment.kind.as_str(),
                "content": comment.content,
                "timestamp": comment.timestamp,
                "parentCommentId": comment.parent_comment_id,
                "createdAt": iso_utc(comment.created_at_ms).ok_or_else(|| Error::RustError("legacy collaboration timestamp is invalid".into()))?,
                "updatedAt": iso_utc(comment.updated_at_ms).ok_or_else(|| Error::RustError("legacy collaboration timestamp is invalid".into()))?,
                "author": {
                    "id": comment.author.id,
                    "name": comment.author.name,
                    "imageUrl": comment.author.image_url,
                },
            }),
        ),
        Ok(LegacyCollaborationSuccessV1::Success { success }) => {
            json_http_response(200, &json!({"success": success}))
        }
        Ok(LegacyCollaborationSuccessV1::WebComment(_)) => Err(Error::RustError(
            "legacy mobile collaboration result is invalid".into(),
        )),
        Err(error) => mobile_error_response(error),
    }
}

pub async fn web_delete_comment_response(
    request: &Request,
    env: &Env,
    request_id: &str,
) -> Result<Response> {
    let comment_id = web_comment_query(request)?;
    let idempotency_key = required_idempotency_key(request)?;
    let now_ms = crate::current_time_ms()?;
    let actor_id =
        browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
            .ok();
    if comment_id.is_none() || actor_id.is_none() || idempotency_key.is_err() {
        return web_route_error(400, "Missing required data");
    }
    if !empty_body_headers(request)? {
        return web_route_error(400, "Missing required data");
    }
    let actor_id = actor_id.expect("checked actor");
    let database = env.d1("DB")?;
    let Some(organization_id) =
        browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await?
    else {
        return web_route_error(404, "Comment not found or unauthorized");
    };
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return web_route_error(429, "Too many requests");
    }
    let execution = dispatch(
        &database,
        request_id,
        CollaborationDispatchV1 {
            caller: web_caller(),
            authority: CollaborationAuthorityV1 {
                actor_id,
                organization_id,
                credential: LegacyCollaborationCredentialV1::Session,
            },
            input: LegacyCollaborationInputV1::WebDeleteCommentRoute {
                legacy_comment_id: comment_id,
            },
            idempotency_key: idempotency_key.expect("checked idempotency key"),
            content_length: 0,
            content_type: None,
            security: admitted_security(rate_limit),
        },
    )
    .await;
    match execution {
        Ok(LegacyCollaborationSuccessV1::Success { success }) => {
            json_http_response(200, &json!({"success": success}))
        }
        Ok(_) => Err(Error::RustError(
            "legacy web delete result is invalid".into(),
        )),
        Err(LegacyCollaborationErrorV1::NotFound) => {
            web_route_error(404, "Comment not found or unauthorized")
        }
        Err(_) => web_route_error(500, "Failed to delete comment"),
    }
}

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedCollaborationActionV1>> {
    let Some(action) = CollaborationActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    let bytes = match decode_json_body(request).await? {
        Ok(value) => value,
        Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    let (input, idempotency_key) = match decode_action_wire(action, &bytes) {
        Ok(value) => value,
        Err(failure) => return Ok(Err(failure)),
    };
    Ok(Ok(DecodedCollaborationActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len())
            .map_err(|_| Error::RustError("legacy collaboration body length is invalid".into()))?,
    }))
}

pub async fn mutate_action(
    request: &Request,
    env: &Env,
    body: &DecodedCollaborationActionV1,
    now_ms: i64,
    request_id: &str,
) -> Result<BrowserWebOutcome<WebCollaborationActionEffectV1>> {
    if request.headers().get("idempotency-key")?.as_deref() != Some(body.idempotency_key.as_str())
        || IdempotencyKey::parse(body.idempotency_key.clone()).is_err()
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(value) => value,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        consume_proof(&database, &proof, now_ms).await?;
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let organization_id =
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await? {
            Some(value) => value,
            None => {
                consume_proof(&database, &proof, now_ms).await?;
                return Ok(Err(BrowserWebFailure::NotFound));
            }
        };
    if !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, &proof, now_ms)
        .await?
    {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    let result = dispatch(
        &database,
        request_id,
        CollaborationDispatchV1 {
            caller: web_caller(),
            authority: CollaborationAuthorityV1 {
                actor_id,
                organization_id,
                credential: LegacyCollaborationCredentialV1::Session,
            },
            input: body.input.clone(),
            idempotency_key: body.idempotency_key.clone(),
            content_length: body.body_length,
            content_type: Some(LEGACY_COLLABORATION_CONTENT_TYPE.into()),
            security: admitted_security(rate_limit),
        },
    )
    .await;
    match result {
        Ok(success) => project_action_success(body.action, success).map(Ok),
        Err(error) => Ok(Err(map_browser_error(error))),
    }
}

async fn consume_proof(
    database: &worker::D1Database,
    proof: &frame_application::ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<()> {
    if browser_web_runtime::consume_session_grant_or_confirm_absent(database, proof, now_ms).await?
    {
        Ok(())
    } else {
        Err(Error::RustError(
            "legacy collaboration browser proof is unavailable".into(),
        ))
    }
}

fn project_action_success(
    action: CollaborationActionV1,
    success: LegacyCollaborationSuccessV1,
) -> Result<WebCollaborationActionEffectV1> {
    match (action, success) {
        (CollaborationActionV1::Delete, LegacyCollaborationSuccessV1::Success { .. }) => {
            Ok(WebCollaborationActionEffectV1::SuccessObject)
        }
        (CollaborationActionV1::New, LegacyCollaborationSuccessV1::WebComment(comment)) => Ok(
            WebCollaborationActionEffectV1::Comment(Box::new(WebCollaborationCommentEffectV1 {
                id: comment.id,
                author_id: comment.author_id,
                kind: comment.kind.as_str(),
                content: comment.content,
                video_id: comment.video_id,
                timestamp: comment.timestamp,
                parent_comment_id: comment.parent_comment_id,
                created_at: iso_utc(comment.created_at_ms).ok_or_else(|| {
                    Error::RustError("legacy collaboration timestamp is invalid".into())
                })?,
                updated_at: iso_utc(comment.updated_at_ms).ok_or_else(|| {
                    Error::RustError("legacy collaboration timestamp is invalid".into())
                })?,
                author_name: comment.author_name,
                author_image: comment.author_image,
                sending: comment.sending,
            })),
        ),
        _ => Err(Error::RustError(
            "legacy collaboration action result is invalid".into(),
        )),
    }
}

async fn dispatch(
    database: &worker::D1Database,
    request_id: &str,
    invocation: CollaborationDispatchV1,
) -> std::result::Result<LegacyCollaborationSuccessV1, LegacyCollaborationErrorV1> {
    let authenticated = LegacyAuthenticatedContextV1::new(
        invocation.authority.actor_id,
        invocation.authority.organization_id,
    )
    .map_err(|_| LegacyCollaborationErrorV1::Unauthorized)?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(database, compatibility_policy())
            .map_err(|_| LegacyCollaborationErrorV1::Unavailable)?;
    let port = D1LegacyCollaborationAtomicPortV1::new(database);
    transport
        .dispatch_collaboration(
            &port,
            LegacyCollaborationInvocationV1 {
                caller: invocation.caller,
                envelope: ApiMutationEnvelopeV1 {
                    content_length: invocation.content_length,
                    content_type: invocation.content_type,
                    idempotency_key: IdempotencyKey::parse(invocation.idempotency_key.clone()).ok(),
                    correlation_id: request_id.to_owned(),
                },
                security: invocation.security,
                authenticated,
                credential: invocation.authority.credential,
                input: invocation.input,
                idempotency_key: invocation.idempotency_key,
            },
        )
        .await
        .map(|execution| execution.success)
}

async fn decode_mobile_create(
    request: &mut Request,
    video_id: String,
    reaction: bool,
) -> Result<std::result::Result<(LegacyCollaborationInputV1, u64), LegacyCollaborationErrorV1>> {
    let bytes = match decode_json_body(request).await? {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    let input = decode_mobile_wire(&bytes, video_id, reaction);
    Ok(input.map(|value| {
        (
            value,
            u64::try_from(bytes.len()).expect("bounded body length fits u64"),
        )
    }))
}

async fn decode_json_body(
    request: &mut Request,
) -> Result<std::result::Result<Vec<u8>, LegacyCollaborationErrorV1>> {
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyCollaborationErrorV1::InvalidInput));
    }
    let declared = declared_body_length(request.headers().get("content-length")?.as_deref());
    let Some(declared) = declared
        .transpose()
        .map_err(|_| Error::RustError("legacy collaboration content length is invalid".into()))?
    else {
        return read_body_without_declared_length(request).await;
    };
    if declared == 0 || declared > LEGACY_COLLABORATION_MAX_BODY_BYTES {
        return Ok(Err(LegacyCollaborationErrorV1::InvalidInput));
    }
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_COLLABORATION_MAX_BODY_BYTES).await {
            Ok(value) => value,
            Err(()) => return Ok(Err(LegacyCollaborationErrorV1::InvalidInput)),
        };
    if bytes.len() != declared {
        return Ok(Err(LegacyCollaborationErrorV1::InvalidInput));
    }
    Ok(Ok(bytes))
}

async fn read_body_without_declared_length(
    request: &mut Request,
) -> Result<std::result::Result<Vec<u8>, LegacyCollaborationErrorV1>> {
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_COLLABORATION_MAX_BODY_BYTES).await {
            Ok(value) => value,
            Err(()) => return Ok(Err(LegacyCollaborationErrorV1::InvalidInput)),
        };
    if bytes.is_empty() {
        Ok(Err(LegacyCollaborationErrorV1::InvalidInput))
    } else {
        Ok(Ok(bytes))
    }
}

fn decode_mobile_wire(
    bytes: &[u8],
    video_id: String,
    reaction: bool,
) -> std::result::Result<LegacyCollaborationInputV1, LegacyCollaborationErrorV1> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| LegacyCollaborationErrorV1::InvalidInput)?;
    let object = value
        .as_object()
        .ok_or(LegacyCollaborationErrorV1::InvalidInput)?;
    let content = required_string(object, "content")?;
    let timestamp = required_nullable_number(object, "timestamp")?;
    if reaction {
        Ok(LegacyCollaborationInputV1::MobileCreateReaction {
            legacy_video_id: video_id,
            content,
            timestamp,
        })
    } else {
        Ok(LegacyCollaborationInputV1::MobileCreateComment {
            legacy_video_id: video_id,
            content,
            timestamp,
            legacy_parent_comment_id: optional_nullable_string(object, "parentCommentId")?,
        })
    }
}

fn decode_action_wire(
    action: CollaborationActionV1,
    bytes: &[u8],
) -> BrowserWebOutcome<(LegacyCollaborationInputV1, String)> {
    match action {
        CollaborationActionV1::Delete => {
            let wire = serde_json::from_slice::<DeleteActionWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_action_common(&wire.schema_version, &wire.idempotency_key)?;
            Ok((
                LegacyCollaborationInputV1::WebDeleteCommentAction {
                    legacy_comment_id: wire.comment_id,
                    caller_parent_id: wire.parent_id.0,
                    legacy_video_id: wire.video_id,
                },
                wire.idempotency_key,
            ))
        }
        CollaborationActionV1::New => {
            let wire = serde_json::from_slice::<NewActionWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_action_common(&wire.schema_version, &wire.idempotency_key)?;
            if !matches!(wire.kind.as_str(), "text" | "emoji") {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok((
                LegacyCollaborationInputV1::WebNewCommentAction {
                    content: wire.content,
                    legacy_video_id: wire.video_id,
                    kind: wire.kind,
                    author_image: wire.author_image.0,
                    legacy_parent_comment_id: wire.parent_comment_id,
                    timestamp: wire.timestamp.0,
                },
                wire.idempotency_key,
            ))
        }
    }
}

fn validate_action_common(schema: &str, key: &str) -> BrowserWebOutcome<()> {
    if schema != WEB_COLLABORATION_ACTION_REQUEST_SCHEMA_V1
        || IdempotencyKey::parse(key.to_owned()).is_err()
    {
        Err(BrowserWebFailure::Invalid)
    } else {
        Ok(())
    }
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
) -> std::result::Result<String, LegacyCollaborationErrorV1> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(LegacyCollaborationErrorV1::InvalidInput)
}

fn required_nullable_number(
    object: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<f64>, LegacyCollaborationErrorV1> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_f64()
            .map(Some)
            .ok_or(LegacyCollaborationErrorV1::InvalidInput),
        _ => Err(LegacyCollaborationErrorV1::InvalidInput),
    }
}

fn optional_nullable_string(
    object: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<String>, LegacyCollaborationErrorV1> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(LegacyCollaborationErrorV1::InvalidInput),
    }
}

async fn authenticate_mobile(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<CollaborationAuthorityV1, LegacyCollaborationErrorV1>> {
    let api_key = request
        .headers()
        .get("authorization")?
        .as_deref()
        .and_then(|value| value.split(' ').nth(1))
        .filter(|value| value.len() == 36)
        .map(str::to_owned);
    if let Some(api_key) = api_key {
        return authenticate_api_key(&api_key, env, now_ms).await;
    }
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(value) => value,
            Err(BrowserWebFailure::Unavailable) => {
                return Ok(Err(LegacyCollaborationErrorV1::Unavailable));
            }
            Err(_) => return Ok(Err(LegacyCollaborationErrorV1::Unauthorized)),
        };
    authority_for_actor(
        &env.d1("DB")?,
        actor_id,
        LegacyCollaborationCredentialV1::Session,
    )
    .await
}

async fn authenticate_api_key(
    key: &str,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<CollaborationAuthorityV1, LegacyCollaborationErrorV1>> {
    let digest = lower_hex(&Sha256::digest(key.as_bytes()));
    let row = env
        .d1("DB")?
        .prepare(
            "SELECT k.user_id FROM auth_api_keys k JOIN users u ON u.id=k.user_id \
             WHERE k.key_digest=?1 AND k.revoked_at_ms IS NULL \
               AND (k.expires_at_ms IS NULL OR k.expires_at_ms>?2) \
               AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(&digest), JsValue::from_f64(now_ms as f64)])?
        .first::<ApiKeyActorRowV1>(None)
        .await?;
    let Some(row) = row else {
        return Ok(Err(LegacyCollaborationErrorV1::Unauthorized));
    };
    authority_for_actor(
        &env.d1("DB")?,
        row.user_id,
        LegacyCollaborationCredentialV1::ApiKey,
    )
    .await
}

async fn authority_for_actor(
    database: &worker::D1Database,
    actor_id: String,
    credential: LegacyCollaborationCredentialV1,
) -> Result<std::result::Result<CollaborationAuthorityV1, LegacyCollaborationErrorV1>> {
    let Some(organization_id) =
        browser_web_runtime::trusted_active_organization_id(database, &actor_id).await?
    else {
        return Ok(Err(LegacyCollaborationErrorV1::NotFound));
    };
    Ok(Ok(CollaborationAuthorityV1 {
        actor_id,
        organization_id,
        credential,
    }))
}

fn required_idempotency_key(
    request: &Request,
) -> Result<std::result::Result<String, LegacyCollaborationErrorV1>> {
    Ok(match request.headers().get("idempotency-key")? {
        Some(value) if IdempotencyKey::parse(value.clone()).is_ok() => Ok(value),
        _ => Err(LegacyCollaborationErrorV1::InvalidInput),
    })
}

fn web_comment_query(request: &Request) -> Result<Option<String>> {
    let url = url::Url::parse(&request.inner().url())
        .map_err(|_| Error::RustError("legacy web comment target is invalid".into()))?;
    Ok(url
        .query_pairs()
        .find(|(key, _)| key == "commentId")
        .map(|(_, value)| value.into_owned()))
}

fn empty_body_headers(request: &Request) -> Result<bool> {
    Ok(match request.headers().get("content-length")? {
        None => true,
        Some(value) if value == "0" => true,
        Some(_) => false,
    })
}

fn declared_body_length(
    value: Option<&str>,
) -> Option<std::result::Result<usize, LegacyCollaborationErrorV1>> {
    value.map(|value| {
        value
            .parse::<usize>()
            .map_err(|_| LegacyCollaborationErrorV1::InvalidInput)
    })
}

fn mobile_error_response(error: LegacyCollaborationErrorV1) -> Result<Response> {
    match error {
        LegacyCollaborationErrorV1::Unauthorized => tagged_http_error(401, "Unauthorized"),
        LegacyCollaborationErrorV1::InvalidInput => tagged_http_error(400, "BadRequest"),
        LegacyCollaborationErrorV1::NotFound => tagged_http_error(404, "NotFound"),
        LegacyCollaborationErrorV1::Conflict => tagged_http_error(409, "Conflict"),
        LegacyCollaborationErrorV1::Unavailable | LegacyCollaborationErrorV1::Internal => {
            tagged_http_error(500, "InternalServerError")
        }
    }
}

fn map_browser_error(error: LegacyCollaborationErrorV1) -> BrowserWebFailure {
    match error {
        LegacyCollaborationErrorV1::Unauthorized => BrowserWebFailure::Unauthenticated,
        LegacyCollaborationErrorV1::InvalidInput => BrowserWebFailure::Invalid,
        LegacyCollaborationErrorV1::NotFound => BrowserWebFailure::NotFound,
        LegacyCollaborationErrorV1::Conflict => BrowserWebFailure::Conflict,
        LegacyCollaborationErrorV1::Unavailable | LegacyCollaborationErrorV1::Internal => {
            BrowserWebFailure::Unavailable
        }
    }
}

fn web_route_error(status: u16, message: &str) -> Result<Response> {
    json_http_response(status, &json!({"error": message}))
}

fn tagged_http_error(status: u16, tag: &str) -> Result<Response> {
    json_http_response(status, &json!({"_tag": tag}))
}

fn json_http_response(status: u16, value: &Value) -> Result<Response> {
    let body = serde_json::to_vec(value)
        .map_err(|_| Error::RustError("legacy collaboration response is invalid".into()))?;
    let mut response = Response::from_bytes(body)?.with_status(status);
    response
        .headers_mut()
        .set("content-type", LEGACY_COLLABORATION_CONTENT_TYPE)?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

const fn compatibility_policy() -> ClientCompatibilityPolicyV1 {
    ClientCompatibilityPolicyV1 {
        api_major: 1,
        current_release: 2,
        previous_release: 1,
        deprecated_after_ms: None,
        retired: false,
    }
}

const fn mobile_caller() -> LegacyCallerV1 {
    LegacyCallerV1::Released(ClientReleaseV1 {
        surface: ClientSurfaceV1::Mobile,
        api_major: 1,
        release: 2,
    })
}

const fn web_caller() -> LegacyCallerV1 {
    LegacyCallerV1::Released(ClientReleaseV1 {
        surface: ClientSurfaceV1::Web,
        api_major: 1,
        release: 2,
    })
}

const fn admitted_security(rate_limit: RateLimitDecisionV1) -> RequestSecurityContextV1 {
    RequestSecurityContextV1 {
        authenticated: true,
        authorized: true,
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
    }
}

fn lower_hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn iso_utc(milliseconds: i64) -> Option<String> {
    let seconds = milliseconds.div_euclid(1_000);
    let millis = milliseconds.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

fn civil_from_days(days: i64) -> Option<(i64, i64, i64)> {
    let shifted = days.checked_add(719_468)?;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted.checked_sub(146_096)?
    } / 146_097;
    let day_of_era = shifted.checked_sub(era.checked_mul(146_097)?)?;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era.checked_add(era.checked_mul(400)?)?;
    let day_of_year =
        day_of_era.checked_sub(365 * year_of_era + year_of_era / 4 - year_of_era / 100)?;
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Some((year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_selector_and_wires_are_exact() {
        assert!(is_action(LEGACY_WEB_DELETE_COMMENT_ACTION_OPERATION_ID));
        assert!(is_action(LEGACY_WEB_NEW_COMMENT_ACTION_OPERATION_ID));
        assert!(!is_action("cap-v1-unknown"));

        let delete = br#"{"schema_version":"frame.web-collaboration-action-request.v1","comment_id":"comment","parent_id":null,"video_id":"video","idempotency_key":"comment-delete-1"}"#;
        let (input, _) =
            decode_action_wire(CollaborationActionV1::Delete, delete).expect("delete action wire");
        assert!(matches!(
            input,
            LegacyCollaborationInputV1::WebDeleteCommentAction {
                caller_parent_id: None,
                ..
            }
        ));

        let create = br#"{"schema_version":"frame.web-collaboration-action-request.v1","content":"   ","video_id":"video","type":"text","author_image":null,"parent_comment_id":"","timestamp":null,"idempotency_key":"comment-create-1"}"#;
        let (input, _) =
            decode_action_wire(CollaborationActionV1::New, create).expect("new action wire");
        assert!(matches!(
            input,
            LegacyCollaborationInputV1::WebNewCommentAction {
                content,
                legacy_parent_comment_id,
                ..
            } if content == "   " && legacy_parent_comment_id.is_empty()
        ));

        let missing_parent = br#"{"schema_version":"frame.web-collaboration-action-request.v1","comment_id":"comment","video_id":"video","idempotency_key":"comment-delete-1"}"#;
        assert_eq!(
            decode_action_wire(CollaborationActionV1::Delete, missing_parent),
            Err(BrowserWebFailure::Invalid)
        );
    }

    #[test]
    fn mobile_wire_requires_nullable_timestamp_and_strips_excess_fields() {
        let input = decode_mobile_wire(
            br#"{"content":" hi ","timestamp":null,"parentCommentId":null,"extra":true}"#,
            "video".into(),
            false,
        )
        .expect("mobile comment");
        assert!(matches!(
            input,
            LegacyCollaborationInputV1::MobileCreateComment {
                content,
                timestamp: None,
                legacy_parent_comment_id: None,
                ..
            } if content == " hi "
        ));
        assert!(decode_mobile_wire(br#"{"content":"hi"}"#, "video".into(), false).is_err());
        assert!(
            decode_mobile_wire(
                br#"{"content":"hi","timestamp":null,"parentCommentId":7}"#,
                "video".into(),
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn timestamp_projection_matches_ecmascript_json_dates() {
        assert_eq!(iso_utc(0).as_deref(), Some("1970-01-01T00:00:00.000Z"));
        assert_eq!(
            iso_utc(946_684_800_123).as_deref(),
            Some("2000-01-01T00:00:00.123Z")
        );
        assert_eq!(iso_utc(-1).as_deref(), Some("1969-12-31T23:59:59.999Z"));
    }

    #[test]
    fn api_key_selector_uses_caps_literal_split_and_length_rule() {
        let selector = |value: &str| {
            value
                .split(' ')
                .nth(1)
                .filter(|candidate| candidate.len() == 36)
                .is_some()
        };
        assert!(selector(&format!("Bearer {}", "a".repeat(36))));
        assert!(!selector(&format!("Bearer  {}", "a".repeat(36))));
        assert!(!selector(&format!("Bearer {}", "a".repeat(35))));
    }
}
