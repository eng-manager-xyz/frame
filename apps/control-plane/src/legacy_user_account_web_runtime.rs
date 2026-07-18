//! Exact carriers for Cap's user-name route, two Effect-RPC calls, and five
//! account/devtool server actions.
//!
//! The name route parses before authentication, Effect RPC keeps its buffered
//! JSON-array `Exit` protocol, and browser actions bind a one-use mutation
//! proof that the D1 port consumes in the same batch as the account mutation.

use std::fmt;

use frame_application::{
    LEGACY_DEMOTE_FROM_PRO_OPERATION_ID, LEGACY_PATCH_ACCOUNT_OPERATION_ID,
    LEGACY_PROMOTE_TO_PRO_OPERATION_ID, LEGACY_RESTART_ONBOARDING_OPERATION_ID,
    LEGACY_SIGN_OUT_ALL_OPERATION_ID, LegacyCallerV1, LegacyImageBytesV1,
    LegacyNullableTextPatchV1, LegacyOnboardingStepInputV1, LegacyOnboardingStepResultV1,
    LegacyOptionalImageUpdateV1, LegacyUserAccountEnvironmentV1, LegacyUserAccountErrorV1,
    LegacyUserAccountInputV1, LegacyUserAccountRequestV1, LegacyUserAccountSuccessV1,
    RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1,
    IdempotencyKey,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Error, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1, LegacyUserAccountInvocationV1,
    },
    legacy_user_account_runtime::D1LegacyUserAccountAtomicPortV1,
};

pub const WEB_USER_ACCOUNT_ACTION_REQUEST_SCHEMA_V1: &str =
    "frame.web-user-account-action-request.v1";
const MAX_BODY_BYTES: usize = 256 * 1024;
const RPC_PARSE_FAILURE: &str = "Invalid Effect RPC request payload";

#[derive(Clone, PartialEq, Eq, Default)]
enum OptionalJsonFieldV1 {
    #[default]
    Missing,
    Present(Value),
}

impl fmt::Debug for OptionalJsonFieldV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Missing",
            Self::Present(_) => "Present([redacted])",
        })
    }
}

impl<'de> Deserialize<'de> for OptionalJsonFieldV1 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Debug, Deserialize)]
struct NameRouteWireV1 {
    #[serde(default, rename = "firstName")]
    first_name: OptionalJsonFieldV1,
    #[serde(default, rename = "lastName")]
    last_name: OptionalJsonFieldV1,
}

#[derive(Debug, Clone, PartialEq)]
struct DecodedRpcRequestV1 {
    id: String,
    input: LegacyUserAccountInputV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RpcDecodeFailureV1 {
    Malformed { request_id: Option<String> },
}

#[derive(Debug, Deserialize)]
struct OrganizationHintsRowV1 {
    active_organization_legacy_id: Option<String>,
    default_organization_legacy_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserAccountActionV1 {
    PatchAccountSettings,
    SignOutAllDevices,
    DemoteFromPro,
    PromoteToPro,
    RestartOnboarding,
}

impl UserAccountActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_PATCH_ACCOUNT_OPERATION_ID => Some(Self::PatchAccountSettings),
            LEGACY_SIGN_OUT_ALL_OPERATION_ID => Some(Self::SignOutAllDevices),
            LEGACY_DEMOTE_FROM_PRO_OPERATION_ID => Some(Self::DemoteFromPro),
            LEGACY_PROMOTE_TO_PRO_OPERATION_ID => Some(Self::PromoteToPro),
            LEGACY_RESTART_ONBOARDING_OPERATION_ID => Some(Self::RestartOnboarding),
            _ => None,
        }
    }

    const fn is_devtool(self) -> bool {
        matches!(
            self,
            Self::DemoteFromPro | Self::PromoteToPro | Self::RestartOnboarding
        )
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    UserAccountActionV1::parse(operation_id).is_some()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchAccountActionWireV1 {
    schema_version: String,
    #[serde(default)]
    first_name: OptionalJsonFieldV1,
    #[serde(default)]
    last_name: OptionalJsonFieldV1,
    #[serde(default)]
    default_organization_id: OptionalJsonFieldV1,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VoidAccountActionWireV1 {
    schema_version: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DecodedUserAccountActionV1 {
    action: UserAccountActionV1,
    input: LegacyUserAccountInputV1,
    idempotency_key: String,
    body_length: u64,
    content_type: String,
}

impl fmt::Debug for DecodedUserAccountActionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedUserAccountActionV1")
            .field("action", &self.action)
            .field("input", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("body_length", &self.body_length)
            .field("content_type", &self.content_type)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebUserAccountActionVoidV1;

pub async fn name_route_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
) -> Result<Response> {
    // Cap calls request.json() after getCurrentUser(), but awaits the user only
    // after body parsing. The malformed-body outcome therefore precedes the
    // explicit unauthenticated branch.
    let (bytes, content_type) = match read_json_body(request).await? {
        Ok(value) => value,
        Err(error) => return name_error_response(error),
    };
    let input = match decode_name_route(&bytes) {
        Ok(input) => input,
        Err(error) => return name_error_response(error),
    };
    if request.headers().get("idempotency-key")?.is_some() {
        return name_error_response(LegacyUserAccountErrorV1::InvalidInput);
    }
    let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
        request,
        env,
        crate::current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(BrowserWebFailure::Unavailable) => {
            return name_error_response(LegacyUserAccountErrorV1::Unavailable);
        }
        Err(_) => return name_error_response(LegacyUserAccountErrorV1::Unauthorized),
    };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        crate::current_time_ms()?,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return name_error_response(LegacyUserAccountErrorV1::Unavailable);
    }
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy user/account registry is invalid".into()))?;
    let authenticated = LegacyAuthenticatedContextV1::principal_only(&actor_id)
        .map_err(|_| Error::RustError("legacy user authority is invalid".into()))?;
    let port = D1LegacyUserAccountAtomicPortV1::new(&database);
    let result = transport
        .dispatch_user_account(
            &port,
            None,
            LegacyUserAccountInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: u64::try_from(bytes.len()).map_err(|_| {
                        Error::RustError("legacy user body length is invalid".into())
                    })?,
                    content_type: Some(content_type),
                    idempotency_key: None,
                    correlation_id: request_id.to_owned(),
                },
                security: admitted_security(rate_limit),
                authenticated,
                request: LegacyUserAccountRequestV1 {
                    environment: LegacyUserAccountEnvironmentV1::Production,
                    actor_id: frame_domain::UserId::parse(&actor_id).ok(),
                    idempotency_key: None,
                    input,
                },
            },
        )
        .await;
    match result {
        Ok(LegacyUserAccountSuccessV1::JsonTrue {
            status: 200,
            body: true,
            ..
        }) => json_response(200, &Value::Bool(true)),
        Ok(_) => Err(Error::RustError(
            "legacy user-name result projection is invalid".into(),
        )),
        Err(error) => name_error_response(error),
    }
}

/// Fast tag discriminator used by the shared `/api/erpc` carrier after it has
/// read the request body once.
#[must_use]
pub fn is_user_rpc_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("tag").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|tag| matches!(tag.as_str(), "UserCompleteOnboardingStep" | "UserUpdate"))
}

pub async fn effect_rpc_response_from_bytes(
    bytes: &[u8],
    request: &Request,
    env: &Env,
    request_id: &str,
) -> Result<Response> {
    let decoded = match decode_rpc_request(bytes) {
        Ok(decoded) => decoded,
        Err(RpcDecodeFailureV1::Malformed { request_id }) => {
            return rpc_response(match request_id {
                Some(id) => rpc_die(&id, RPC_PARSE_FAILURE),
                None => rpc_defect(RPC_PARSE_FAILURE),
            });
        }
    };
    let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
        request,
        env,
        crate::current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(BrowserWebFailure::Unavailable) => {
            return rpc_response(rpc_typed_failure(
                &decoded.id,
                json!({"_tag":"InternalError","type":"database"}),
            ));
        }
        Err(_) => {
            return rpc_response(rpc_typed_failure(
                &decoded.id,
                json!({"_tag":"UnauthenticatedError"}),
            ));
        }
    };
    let database = env.d1("DB")?;
    let hints = organization_hints(&database, &actor_id).await?;
    let input = apply_organization_hints(decoded.input, hints);
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        crate::current_time_ms()?,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return rpc_response(rpc_typed_failure(
            &decoded.id,
            json!({"_tag":"InternalError","type":"unknown"}),
        ));
    }
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy user/account registry is invalid".into()))?;
    let authenticated = LegacyAuthenticatedContextV1::principal_only(&actor_id)
        .map_err(|_| Error::RustError("legacy user authority is invalid".into()))?;
    let port = D1LegacyUserAccountAtomicPortV1::new(&database);
    let result = transport
        .dispatch_user_account(
            &port,
            None,
            LegacyUserAccountInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: u64::try_from(bytes.len()).map_err(|_| {
                        Error::RustError("legacy RPC body length is invalid".into())
                    })?,
                    content_type: Some("application/json".into()),
                    idempotency_key: None,
                    correlation_id: request_id.to_owned(),
                },
                security: admitted_security(rate_limit),
                authenticated,
                request: LegacyUserAccountRequestV1 {
                    environment: LegacyUserAccountEnvironmentV1::Production,
                    actor_id: frame_domain::UserId::parse(&actor_id).ok(),
                    idempotency_key: None,
                    input,
                },
            },
        )
        .await;
    rpc_response(match result {
        Ok(success) => rpc_success(&decoded.id, success),
        Err(error) => rpc_user_error(&decoded.id, error),
    })
}

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedUserAccountActionV1>> {
    let Some(action) = UserAccountActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    let (bytes, content_type) = match read_json_body(request).await? {
        Ok(value) => value,
        Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    let (input, idempotency_key) = match decode_action_wire(action, &bytes) {
        Ok(value) => value,
        Err(failure) => return Ok(Err(failure)),
    };
    Ok(Ok(DecodedUserAccountActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len())
            .map_err(|_| Error::RustError("legacy account body length is invalid".into()))?,
        content_type,
    }))
}

pub async fn mutate_action(
    request: &Request,
    env: &Env,
    body: &DecodedUserAccountActionV1,
    now_ms: i64,
    correlation_id: &str,
    production: bool,
) -> Result<BrowserWebOutcome<WebUserAccountActionVoidV1>> {
    // Preserve Cap's environment guard before any authentication lookup.
    if body.action.is_devtool() && production {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    if request.headers().get("idempotency-key")?.as_deref() != Some(body.idempotency_key.as_str())
        || IdempotencyKey::parse(body.idempotency_key.clone()).is_err()
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let database = env.d1("DB")?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy user/account registry is invalid".into()))?;
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let rate_limit = match compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        now_ms,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            consume_or_confirm_absent(&database, &proof, now_ms).await?;
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let authenticated = LegacyAuthenticatedContextV1::principal_only(&actor_id)
        .map_err(|_| Error::RustError("legacy account authority is invalid".into()))?;
    let port = D1LegacyUserAccountAtomicPortV1::new(&database);
    let result = transport
        .dispatch_user_account(
            &port,
            Some(&proof),
            LegacyUserAccountInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: body.body_length,
                    content_type: Some(body.content_type.clone()),
                    idempotency_key: IdempotencyKey::parse(body.idempotency_key.clone()).ok(),
                    correlation_id: correlation_id.to_owned(),
                },
                security: admitted_security(rate_limit),
                authenticated,
                request: LegacyUserAccountRequestV1 {
                    environment: if production {
                        LegacyUserAccountEnvironmentV1::Production
                    } else {
                        LegacyUserAccountEnvironmentV1::Development
                    },
                    actor_id: Some(proof.user_id()),
                    idempotency_key: Some(body.idempotency_key.clone()),
                    input: body.input.clone(),
                },
            },
        )
        .await;
    match result {
        Ok(LegacyUserAccountSuccessV1::ServerActionVoid { .. }) => {
            Ok(Ok(WebUserAccountActionVoidV1))
        }
        Ok(_) => {
            consume_or_confirm_absent(&database, &proof, now_ms).await?;
            Ok(Err(BrowserWebFailure::Unavailable))
        }
        Err(error) => {
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            Ok(Err(map_browser_error(error)))
        }
    }
}

async fn consume_or_confirm_absent(
    database: &D1Database,
    proof: &frame_application::ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<bool> {
    browser_web_runtime::consume_session_grant_or_confirm_absent(database, proof, now_ms).await
}

async fn read_json_body(
    request: &mut Request,
) -> Result<std::result::Result<(Vec<u8>, String), LegacyUserAccountErrorV1>> {
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyUserAccountErrorV1::InvalidInput));
    }
    let declared = match request.headers().get("content-length")? {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(Err(LegacyUserAccountErrorV1::InvalidInput)),
        },
        None => None,
    };
    if declared.is_some_and(|value| value == 0 || value > MAX_BODY_BYTES) {
        return Ok(Err(LegacyUserAccountErrorV1::InvalidInput));
    }
    let bytes = match crate::read_bounded_legacy_body(request, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(()) => return Ok(Err(LegacyUserAccountErrorV1::InvalidInput)),
    };
    if bytes.is_empty()
        || bytes.len() > MAX_BODY_BYTES
        || declared.is_some_and(|value| value != bytes.len())
    {
        return Ok(Err(LegacyUserAccountErrorV1::InvalidInput));
    }
    Ok(Ok((bytes, content_type.expect("validated content type"))))
}

fn decode_name_route(bytes: &[u8]) -> Result<LegacyUserAccountInputV1, LegacyUserAccountErrorV1> {
    let wire = serde_json::from_slice::<NameRouteWireV1>(bytes)
        .map_err(|_| LegacyUserAccountErrorV1::InvalidInput)?;
    Ok(LegacyUserAccountInputV1::NameRoute {
        first_name: nullable_text(wire.first_name)?,
        last_name: nullable_text(wire.last_name)?,
    })
}

fn nullable_text(
    value: OptionalJsonFieldV1,
) -> Result<LegacyNullableTextPatchV1, LegacyUserAccountErrorV1> {
    match value {
        OptionalJsonFieldV1::Missing => Ok(LegacyNullableTextPatchV1::Absent),
        OptionalJsonFieldV1::Present(Value::Null) => Ok(LegacyNullableTextPatchV1::Null),
        OptionalJsonFieldV1::Present(Value::String(value)) => {
            Ok(LegacyNullableTextPatchV1::Value(value))
        }
        OptionalJsonFieldV1::Present(_) => Err(LegacyUserAccountErrorV1::InvalidInput),
    }
}

fn decode_rpc_request(bytes: &[u8]) -> Result<DecodedRpcRequestV1, RpcDecodeFailureV1> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| RpcDecodeFailureV1::Malformed { request_id: None })?;
    let object = value
        .as_object()
        .ok_or(RpcDecodeFailureV1::Malformed { request_id: None })?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_rpc_id(value))
        .map(str::to_owned)
        .ok_or(RpcDecodeFailureV1::Malformed { request_id: None })?;
    let malformed = || RpcDecodeFailureV1::Malformed {
        request_id: Some(id.clone()),
    };
    if object.get("_tag").and_then(Value::as_str) != Some("Request")
        || !valid_rpc_headers(object.get("headers"))
        || !valid_optional_string(object.get("traceId"))
        || !valid_optional_string(object.get("spanId"))
        || !valid_optional_bool(object.get("sampled"))
    {
        return Err(malformed());
    }
    let tag = object
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(malformed)?;
    let payload = object.get("payload").ok_or_else(malformed)?;
    let input = match tag {
        "UserCompleteOnboardingStep" => decode_onboarding(payload),
        "UserUpdate" => decode_user_update(payload),
        _ => Err(LegacyUserAccountErrorV1::InvalidInput),
    }
    .map_err(|_| malformed())?;
    Ok(DecodedRpcRequestV1 { id, input })
}

fn decode_onboarding(value: &Value) -> Result<LegacyUserAccountInputV1, LegacyUserAccountErrorV1> {
    let object = value
        .as_object()
        .ok_or(LegacyUserAccountErrorV1::InvalidInput)?;
    let step = required_string(object, "step")?;
    let data = object.get("data");
    let step = match step.as_str() {
        "welcome" => {
            let data = data
                .and_then(Value::as_object)
                .ok_or(LegacyUserAccountErrorV1::InvalidInput)?;
            LegacyOnboardingStepInputV1::Welcome {
                first_name: required_string(data, "firstName")?,
                last_name: optional_string(data, "lastName")?,
            }
        }
        "organizationSetup" => {
            let data = data
                .and_then(Value::as_object)
                .ok_or(LegacyUserAccountErrorV1::InvalidInput)?;
            let organization_icon = data.get("organizationIcon").map(decode_image).transpose()?;
            LegacyOnboardingStepInputV1::OrganizationSetup {
                organization_name: required_string(data, "organizationName")?,
                organization_icon,
            }
        }
        "customDomain" => LegacyOnboardingStepInputV1::CustomDomain,
        "inviteTeam" => LegacyOnboardingStepInputV1::InviteTeam,
        "skipToDashboard" => LegacyOnboardingStepInputV1::SkipToDashboard,
        _ => return Err(LegacyUserAccountErrorV1::InvalidInput),
    };
    Ok(LegacyUserAccountInputV1::CompleteOnboardingStep {
        step,
        active_organization_legacy_id: None,
        default_organization_legacy_id: None,
    })
}

fn decode_user_update(value: &Value) -> Result<LegacyUserAccountInputV1, LegacyUserAccountErrorV1> {
    let object = value
        .as_object()
        .ok_or(LegacyUserAccountErrorV1::InvalidInput)?;
    let payload_id = required_string(object, "id")?;
    let image = match object.get("image") {
        None => LegacyOptionalImageUpdateV1::Absent,
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("None") =>
        {
            LegacyOptionalImageUpdateV1::None
        }
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("Some") =>
        {
            LegacyOptionalImageUpdateV1::Some(decode_image(
                option
                    .get("value")
                    .ok_or(LegacyUserAccountErrorV1::InvalidInput)?,
            )?)
        }
        Some(_) => return Err(LegacyUserAccountErrorV1::InvalidInput),
    };
    Ok(LegacyUserAccountInputV1::UserUpdate { payload_id, image })
}

fn decode_image(value: &Value) -> Result<LegacyImageBytesV1, LegacyUserAccountErrorV1> {
    let object = value
        .as_object()
        .ok_or(LegacyUserAccountErrorV1::InvalidInput)?;
    let data = object
        .get("data")
        .ok_or(LegacyUserAccountErrorV1::InvalidInput)
        .and_then(decode_bytes)?;
    Ok(LegacyImageBytesV1 {
        data,
        content_type: required_string(object, "contentType")?,
        file_name: required_string(object, "fileName")?,
    })
}

fn decode_bytes(value: &Value) -> Result<Vec<u8>, LegacyUserAccountErrorV1> {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or(LegacyUserAccountErrorV1::InvalidInput)
            })
            .collect(),
        Value::String(value) => decode_base64(value),
        _ => Err(LegacyUserAccountErrorV1::InvalidInput),
    }
}

fn decode_base64(value: &str) -> Result<Vec<u8>, LegacyUserAccountErrorV1> {
    if !value.len().is_multiple_of(4) || !value.is_ascii() {
        return Err(LegacyUserAccountErrorV1::InvalidInput);
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in value.as_bytes().chunks_exact(4).enumerate() {
        let last = index + 1 == value.len() / 4;
        let a = base64_digit(chunk[0])?;
        let b = base64_digit(chunk[1])?;
        let c_pad = chunk[2] == b'=';
        let d_pad = chunk[3] == b'=';
        if (c_pad && !d_pad) || (d_pad && !last) {
            return Err(LegacyUserAccountErrorV1::InvalidInput);
        }
        let c = if c_pad { 0 } else { base64_digit(chunk[2])? };
        let d = if d_pad { 0 } else { base64_digit(chunk[3])? };
        output.push((a << 2) | (b >> 4));
        if !c_pad {
            output.push((b << 4) | (c >> 2));
        }
        if !d_pad {
            output.push((c << 6) | d);
        }
    }
    Ok(output)
}

fn base64_digit(value: u8) -> Result<u8, LegacyUserAccountErrorV1> {
    match value {
        b'A'..=b'Z' => Ok(value - b'A'),
        b'a'..=b'z' => Ok(value - b'a' + 26),
        b'0'..=b'9' => Ok(value - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(LegacyUserAccountErrorV1::InvalidInput),
    }
}

async fn organization_hints(
    database: &D1Database,
    actor_id: &str,
) -> Result<OrganizationHintsRowV1> {
    database
        .prepare(
            "SELECT active_map.legacy_organization_id AS active_organization_legacy_id, \
             default_map.legacy_organization_id AS default_organization_legacy_id \
             FROM users u \
             LEFT JOIN legacy_user_account_organization_ids_v1 active_map \
               ON active_map.organization_id=u.active_organization_id \
             LEFT JOIN legacy_user_account_organization_ids_v1 default_map \
               ON default_map.organization_id=u.default_organization_id \
             WHERE u.id=?1 AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(actor_id)])?
        .first::<OrganizationHintsRowV1>(None)
        .await?
        .ok_or_else(|| Error::RustError("legacy user organization hints are unavailable".into()))
}

fn apply_organization_hints(
    input: LegacyUserAccountInputV1,
    hints: OrganizationHintsRowV1,
) -> LegacyUserAccountInputV1 {
    match input {
        LegacyUserAccountInputV1::CompleteOnboardingStep { step, .. } => {
            LegacyUserAccountInputV1::CompleteOnboardingStep {
                step,
                active_organization_legacy_id: hints.active_organization_legacy_id,
                default_organization_legacy_id: hints.default_organization_legacy_id,
            }
        }
        other => other,
    }
}

fn decode_action_wire(
    action: UserAccountActionV1,
    bytes: &[u8],
) -> BrowserWebOutcome<(LegacyUserAccountInputV1, String)> {
    if action == UserAccountActionV1::PatchAccountSettings {
        let wire = serde_json::from_slice::<PatchAccountActionWireV1>(bytes)
            .map_err(|_| BrowserWebFailure::Invalid)?;
        validate_action_common(&wire.schema_version, &wire.idempotency_key)?;
        let default_organization_legacy_id = match wire.default_organization_id {
            OptionalJsonFieldV1::Missing => None,
            OptionalJsonFieldV1::Present(Value::String(value)) => Some(value),
            OptionalJsonFieldV1::Present(_) => return Err(BrowserWebFailure::Invalid),
        };
        return Ok((
            LegacyUserAccountInputV1::PatchAccountSettings {
                first_name: nullable_text(wire.first_name)
                    .map_err(|_| BrowserWebFailure::Invalid)?,
                last_name: nullable_text(wire.last_name).map_err(|_| BrowserWebFailure::Invalid)?,
                default_organization_legacy_id,
            },
            wire.idempotency_key,
        ));
    }
    let wire = serde_json::from_slice::<VoidAccountActionWireV1>(bytes)
        .map_err(|_| BrowserWebFailure::Invalid)?;
    validate_action_common(&wire.schema_version, &wire.idempotency_key)?;
    let input = match action {
        UserAccountActionV1::SignOutAllDevices => LegacyUserAccountInputV1::SignOutAllDevices,
        UserAccountActionV1::DemoteFromPro => LegacyUserAccountInputV1::DemoteFromPro,
        UserAccountActionV1::PromoteToPro => LegacyUserAccountInputV1::PromoteToPro,
        UserAccountActionV1::RestartOnboarding => LegacyUserAccountInputV1::RestartOnboarding,
        UserAccountActionV1::PatchAccountSettings => unreachable!("handled above"),
    };
    Ok((input, wire.idempotency_key))
}

fn validate_action_common(schema: &str, idempotency_key: &str) -> BrowserWebOutcome<()> {
    if schema != WEB_USER_ACCOUNT_ACTION_REQUEST_SCHEMA_V1
        || !(8..=128).contains(&idempotency_key.len())
        || !idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(())
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<String, LegacyUserAccountErrorV1> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(LegacyUserAccountErrorV1::InvalidInput)
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, LegacyUserAccountErrorV1> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(LegacyUserAccountErrorV1::InvalidInput),
    }
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

fn rpc_success(request_id: &str, success: LegacyUserAccountSuccessV1) -> Value {
    match success {
        LegacyUserAccountSuccessV1::OnboardingExit { envelope, .. } => {
            let value = match envelope.value {
                LegacyOnboardingStepResultV1::Welcome => json!({"step":"welcome"}),
                LegacyOnboardingStepResultV1::OrganizationSetup {
                    legacy_organization_id,
                } => json!({
                    "step":"organizationSetup",
                    "data":{"organizationId":legacy_organization_id.as_str()},
                }),
                LegacyOnboardingStepResultV1::CustomDomain => json!({"step":"customDomain"}),
                LegacyOnboardingStepResultV1::InviteTeam => json!({"step":"inviteTeam"}),
                LegacyOnboardingStepResultV1::SkipToDashboard => {
                    json!({"step":"skipToDashboard"})
                }
            };
            json!([{
                "_tag": envelope.id,
                "requestId": request_id,
                "exit":{"_tag":envelope.tag,"value":value},
            }])
        }
        LegacyUserAccountSuccessV1::RpcVoidExit { envelope, .. } => json!([{
            "_tag": envelope.id,
            "requestId": request_id,
            "exit":{"_tag":envelope.tag},
        }]),
        _ => rpc_typed_failure(request_id, json!({"_tag":"InternalError","type":"unknown"})),
    }
}

fn rpc_user_error(request_id: &str, error: LegacyUserAccountErrorV1) -> Value {
    let payload = match error {
        LegacyUserAccountErrorV1::Unauthorized => json!({"_tag":"UnauthenticatedError"}),
        LegacyUserAccountErrorV1::Forbidden => json!({"_tag":"PolicyDenied"}),
        LegacyUserAccountErrorV1::Database => json!({"_tag":"InternalError","type":"database"}),
        LegacyUserAccountErrorV1::ProviderRequired => {
            json!({"_tag":"InternalError","type":"s3"})
        }
        LegacyUserAccountErrorV1::InvalidInput
        | LegacyUserAccountErrorV1::EmptyPatch
        | LegacyUserAccountErrorV1::DevelopmentOnly
        | LegacyUserAccountErrorV1::Conflict
        | LegacyUserAccountErrorV1::Unavailable
        | LegacyUserAccountErrorV1::Internal => {
            json!({"_tag":"InternalError","type":"unknown"})
        }
    };
    rpc_typed_failure(request_id, payload)
}

fn rpc_typed_failure(request_id: &str, error: Value) -> Value {
    json!([{
        "_tag":"Exit",
        "requestId":request_id,
        "exit":{"_tag":"Failure","cause":{"_tag":"Fail","error":error}},
    }])
}

fn rpc_die(request_id: &str, message: &str) -> Value {
    json!([{
        "_tag":"Exit",
        "requestId":request_id,
        "exit":{"_tag":"Failure","cause":{"_tag":"Die","defect":message}},
    }])
}

fn rpc_defect(message: &str) -> Value {
    json!([{"_tag":"Defect","defect":message}])
}

fn rpc_response(value: Value) -> Result<Response> {
    json_response(200, &value)
}

fn name_error_response(error: LegacyUserAccountErrorV1) -> Result<Response> {
    let status = match error {
        LegacyUserAccountErrorV1::Unauthorized => 401,
        LegacyUserAccountErrorV1::Forbidden => 403,
        LegacyUserAccountErrorV1::Conflict => 409,
        LegacyUserAccountErrorV1::ProviderRequired | LegacyUserAccountErrorV1::Unavailable => 503,
        LegacyUserAccountErrorV1::InvalidInput
        | LegacyUserAccountErrorV1::Database
        | LegacyUserAccountErrorV1::EmptyPatch
        | LegacyUserAccountErrorV1::DevelopmentOnly
        | LegacyUserAccountErrorV1::Internal => 500,
    };
    json_response(status, &json!({"error":true}))
}

fn json_response(status: u16, value: &Value) -> Result<Response> {
    let body = serde_json::to_vec(value)
        .map_err(|_| Error::RustError("legacy user/account response is invalid".into()))?;
    let mut response = Response::from_bytes(body)?.with_status(status);
    response
        .headers_mut()
        .set("content-type", "application/json")?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn map_browser_error(error: LegacyUserAccountErrorV1) -> BrowserWebFailure {
    match error {
        LegacyUserAccountErrorV1::Unauthorized => BrowserWebFailure::Unauthenticated,
        LegacyUserAccountErrorV1::InvalidInput | LegacyUserAccountErrorV1::EmptyPatch => {
            BrowserWebFailure::Invalid
        }
        LegacyUserAccountErrorV1::Forbidden => BrowserWebFailure::Forbidden,
        LegacyUserAccountErrorV1::Conflict => BrowserWebFailure::Conflict,
        LegacyUserAccountErrorV1::Database
        | LegacyUserAccountErrorV1::DevelopmentOnly
        | LegacyUserAccountErrorV1::ProviderRequired
        | LegacyUserAccountErrorV1::Unavailable
        | LegacyUserAccountErrorV1::Internal => BrowserWebFailure::Unavailable,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_wire_preserves_missing_null_empty_and_excess_fields() {
        assert_eq!(
            decode_name_route(br#"{"firstName":"","lastName":null,"ignored":true}"#).expect("name"),
            LegacyUserAccountInputV1::NameRoute {
                first_name: LegacyNullableTextPatchV1::Value(String::new()),
                last_name: LegacyNullableTextPatchV1::Null,
            }
        );
        assert!(matches!(
            decode_name_route(br#"{}"#).expect("missing"),
            LegacyUserAccountInputV1::NameRoute {
                first_name: LegacyNullableTextPatchV1::Absent,
                last_name: LegacyNullableTextPatchV1::Absent,
            }
        ));
    }

    #[test]
    fn rpc_wire_decodes_onboarding_and_all_image_option_layers() {
        let welcome = decode_rpc_request(
            br#"{"_tag":"Request","id":"1","tag":"UserCompleteOnboardingStep","payload":{"step":"welcome","data":{"firstName":" Ada "}},"headers":[]}"#,
        )
        .expect("welcome");
        assert!(matches!(
            welcome.input,
            LegacyUserAccountInputV1::CompleteOnboardingStep {
                step: LegacyOnboardingStepInputV1::Welcome { .. },
                ..
            }
        ));
        for (payload, expected) in [
            (r#"{"id":"ignored"}"#, LegacyOptionalImageUpdateV1::Absent),
            (
                r#"{"id":"ignored","image":{"_tag":"None"}}"#,
                LegacyOptionalImageUpdateV1::None,
            ),
        ] {
            let raw = format!(
                r#"{{"_tag":"Request","id":"2","tag":"UserUpdate","payload":{payload},"headers":[]}}"#
            );
            let decoded = decode_rpc_request(raw.as_bytes()).expect("user update");
            let LegacyUserAccountInputV1::UserUpdate { image, .. } = decoded.input else {
                panic!("user update")
            };
            assert_eq!(image, expected);
        }
        let some = decode_rpc_request(
            br#"{"_tag":"Request","id":"3","tag":"UserUpdate","payload":{"id":"ignored","image":{"_tag":"Some","value":{"data":"AQID","contentType":"image/png","fileName":"a.png"}}},"headers":[]}"#,
        )
        .expect("some image");
        assert!(matches!(
            some.input,
            LegacyUserAccountInputV1::UserUpdate {
                image: LegacyOptionalImageUpdateV1::Some(LegacyImageBytesV1 { ref data, .. }),
                ..
            } if data == &[1, 2, 3]
        ));
    }

    #[test]
    fn action_selector_and_patch_wire_are_closed_and_presence_sensitive() {
        for operation in [
            LEGACY_PATCH_ACCOUNT_OPERATION_ID,
            LEGACY_SIGN_OUT_ALL_OPERATION_ID,
            LEGACY_DEMOTE_FROM_PRO_OPERATION_ID,
            LEGACY_PROMOTE_TO_PRO_OPERATION_ID,
            LEGACY_RESTART_ONBOARDING_OPERATION_ID,
        ] {
            assert!(is_action(operation));
        }
        assert!(!is_action("cap-v1-unknown"));
        let body = br#"{"schema_version":"frame.web-user-account-action-request.v1","first_name":null,"last_name":"","idempotency_key":"account-patch-1"}"#;
        let (input, key) =
            decode_action_wire(UserAccountActionV1::PatchAccountSettings, body).expect("patch");
        assert_eq!(key, "account-patch-1");
        assert_eq!(
            input,
            LegacyUserAccountInputV1::PatchAccountSettings {
                first_name: LegacyNullableTextPatchV1::Null,
                last_name: LegacyNullableTextPatchV1::Value(String::new()),
                default_organization_legacy_id: None,
            }
        );
    }

    #[test]
    fn effect_success_projection_keeps_exit_value_shape() {
        let success = rpc_success(
            "7",
            LegacyUserAccountSuccessV1::OnboardingExit {
                envelope: frame_application::LegacyEffectRpcSuccessEnvelopeV1 {
                    id_key: "_id",
                    tag_key: "_tag",
                    id: "Exit",
                    tag: "Success",
                    value: LegacyOnboardingStepResultV1::CustomDomain,
                },
                provider_effect: frame_application::LegacyUserAccountProviderEffectV1::NotRequested,
                replayed: false,
            },
        );
        assert_eq!(
            success,
            json!([{
                "_tag":"Exit",
                "requestId":"7",
                "exit":{"_tag":"Success","value":{"step":"customDomain"}},
            }])
        );
    }
}
