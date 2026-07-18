//! Exact HTTP and browser-action carriers for retained video properties.

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, Tag, aead::AeadInPlace};
use frame_application::{
    LEGACY_EDIT_VIDEO_DATE_OPERATION_ID, LEGACY_EDIT_VIDEO_TITLE_OPERATION_ID,
    LEGACY_REMOVE_VIDEO_PASSWORD_OPERATION_ID, LEGACY_SET_VIDEO_PASSWORD_OPERATION_ID,
    LEGACY_UPDATE_VIDEO_SETTINGS_OPERATION_ID, LEGACY_VERIFY_VIDEO_PASSWORD_OPERATION_ID,
    LegacyCallerV1, LegacyVideoPropertiesAtomicResultV1, LegacyVideoPropertiesCredentialV1,
    LegacyVideoPropertiesErrorV1, LegacyVideoPropertiesInputV1, RateLimitDecisionV1,
    RequestSecurityContextV1,
};
use frame_domain::{
    ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1,
    IdempotencyKey,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{Env, Error, Request, Response, Result};
use zeroize::Zeroize;

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyCompatibilityTransportV1, LegacyVideoPropertiesInvocationV1,
    },
    legacy_video_properties_runtime::D1LegacyVideoPropertiesAtomicPortV1,
};

pub const ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-video-property-action-request.v1";
const MAX_BODY_BYTES: usize = 256 * 1024;
const COOKIE_NAME: &str = "x-cap-password";
const COOKIE_KEY_SECRET: &str = "FRAME_LEGACY_PASSWORD_COOKIE_KEY_V1";
const COOKIE_AAD: &[u8] = b"frame/x-cap-password/aes-256-gcm/v1\0";
const COOKIE_VERSION: u8 = 1;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const MAX_VERIFIED_HASHES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MobileVideoPropertyActionV1 {
    Password,
    Sharing,
    Title,
}

impl MobileVideoPropertyActionV1 {
    const fn bucket(self) -> CompatibilityRateLimitBucketV1 {
        match self {
            Self::Password => CompatibilityRateLimitBucketV1::SharePlayback,
            Self::Sharing | Self::Title => CompatibilityRateLimitBucketV1::ClientCompatibility,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiKeyActorRowV1 {
    user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebActionV1 {
    EditDate,
    EditTitle,
    RemovePassword,
    SetPassword,
    VerifyPassword,
    UpdateSettings,
}

impl WebActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_EDIT_VIDEO_DATE_OPERATION_ID => Some(Self::EditDate),
            LEGACY_EDIT_VIDEO_TITLE_OPERATION_ID => Some(Self::EditTitle),
            LEGACY_REMOVE_VIDEO_PASSWORD_OPERATION_ID => Some(Self::RemovePassword),
            LEGACY_SET_VIDEO_PASSWORD_OPERATION_ID => Some(Self::SetPassword),
            LEGACY_VERIFY_VIDEO_PASSWORD_OPERATION_ID => Some(Self::VerifyPassword),
            LEGACY_UPDATE_VIDEO_SETTINGS_OPERATION_ID => Some(Self::UpdateSettings),
            _ => None,
        }
    }

    const fn anonymous(self) -> bool {
        matches!(self, Self::VerifyPassword)
    }

    const fn password_result(self) -> bool {
        matches!(
            self,
            Self::RemovePassword | Self::SetPassword | Self::VerifyPassword
        )
    }

    const fn bucket(self) -> CompatibilityRateLimitBucketV1 {
        match self {
            Self::RemovePassword | Self::SetPassword | Self::VerifyPassword => {
                CompatibilityRateLimitBucketV1::SharePlayback
            }
            Self::EditDate | Self::EditTitle | Self::UpdateSettings => {
                CompatibilityRateLimitBucketV1::VideoMedia
            }
        }
    }
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    WebActionV1::parse(operation_id).is_some()
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedVideoPropertyActionV1 {
    action: WebActionV1,
    input: LegacyVideoPropertiesInputV1,
    idempotency_key: Option<String>,
    body_length: u64,
    content_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditDateWireV1 {
    schema_version: String,
    video_id: String,
    date: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditTitleWireV1 {
    schema_version: String,
    video_id: String,
    title: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoOnlyWireV1 {
    schema_version: String,
    video_id: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordWireV1 {
    schema_version: String,
    video_id: String,
    password: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsWireV1 {
    schema_version: String,
    video_id: String,
    video_settings: Value,
    #[serde(default)]
    idempotency_key: Option<String>,
}

pub(crate) async fn mobile_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    video_id: String,
    action: MobileVideoPropertyActionV1,
) -> Result<Response> {
    let bytes = match decode_body(request).await? {
        Ok(bytes) => bytes,
        Err(error) => return mobile_error(error),
    };
    let input = match decode_mobile_input(action, video_id, &bytes) {
        Ok(input) => input,
        Err(error) => return mobile_error(error),
    };
    let idempotency_key = match optional_header_idempotency(request)? {
        Ok(value) => value,
        Err(error) => return mobile_error(error),
    };
    let now_ms = crate::current_time_ms()?;
    let (actor_id, credential) = match authenticate_mobile(request, env, now_ms).await? {
        Ok(value) => value,
        Err(error) => return mobile_error(error),
    };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        action.bucket(),
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return json_response(429, &json!({"_tag": "TooManyRequests"}));
    }
    let outcome = execute(
        env,
        VideoPropertyExecutionV1 {
            caller: mobile_caller(),
            security: admitted_security(true, rate_limit),
            credential,
            actor_id: Some(actor_id),
            input,
            idempotency_key,
            body_length: bytes.len(),
            content_type: request
                .headers()
                .get("content-type")?
                .expect("validated content type"),
            correlation_id: request_id.into(),
        },
    )
    .await;
    match outcome {
        Ok(result) => match result.result {
            LegacyVideoPropertiesAtomicResultV1::MobileSummary(summary) => {
                json_response(200, &serde_json::to_value(summary).map_err(json_error)?)
            }
            _ => Err(Error::RustError(
                "legacy mobile video-property result is invalid".into(),
            )),
        },
        Err(error) => mobile_error(error),
    }
}

pub(crate) async fn metadata_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
) -> Result<Response> {
    let bytes = match decode_body(request).await? {
        Ok(bytes) => bytes,
        Err(_) => return metadata_error(),
    };
    let value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(value)) => value,
        _ => return metadata_error(),
    };
    let Some(video_id) = value.get("videoId").and_then(Value::as_str) else {
        return metadata_error();
    };
    let Some(metadata) = value.get("metadata") else {
        return metadata_error();
    };
    let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
        request,
        env,
        crate::current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(_) => return metadata_error(),
    };
    let idempotency_key = match optional_header_idempotency(request)? {
        Ok(value) => value,
        Err(_) => return metadata_error(),
    };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::VideoMedia,
        &actor_id,
        crate::current_time_ms()?,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return metadata_error();
    }
    let outcome = execute(
        env,
        VideoPropertyExecutionV1 {
            caller: web_caller(),
            security: admitted_security(true, rate_limit),
            credential: LegacyVideoPropertiesCredentialV1::Session,
            actor_id: Some(actor_id),
            input: LegacyVideoPropertiesInputV1::MetadataPut {
                legacy_video_id: video_id.into(),
                metadata: metadata.clone(),
            },
            idempotency_key,
            body_length: bytes.len(),
            content_type: request
                .headers()
                .get("content-type")?
                .expect("validated content type"),
            correlation_id: request_id.into(),
        },
    )
    .await;
    match outcome {
        Ok(result) if matches!(result.result, LegacyVideoPropertiesAtomicResultV1::JsonTrue) => {
            json_response(200, &Value::Bool(true))
        }
        Ok(_) => Err(Error::RustError("legacy metadata result is invalid".into())),
        Err(LegacyVideoPropertiesErrorV1::Unauthorized)
        | Err(LegacyVideoPropertiesErrorV1::NotFound)
        | Err(LegacyVideoPropertiesErrorV1::AccessDenied)
        | Err(LegacyVideoPropertiesErrorV1::InvalidInput) => metadata_error(),
        Err(_) => json_response(500, &json!({"error": true})),
    }
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
    now_ms: i64,
) -> Result<std::result::Result<DecodedVideoPropertyActionV1, BrowserWebFailure>> {
    let Some(action) = WebActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = match crate::read_bounded_legacy_body(request, MAX_BODY_BYTES).await {
        Ok(bytes) if !bytes.is_empty() && bytes.len() <= MAX_BODY_BYTES => bytes,
        _ => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    let (input, idempotency_key) = match decode_action(action, &bytes, now_ms) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    Ok(Ok(DecodedVideoPropertyActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len())
            .map_err(|_| Error::RustError("legacy action body length is invalid".into()))?,
        content_type: content_type.expect("validated content type"),
    }))
}

pub(crate) async fn action_response(
    request: &Request,
    env: &Env,
    decoded: &DecodedVideoPropertyActionV1,
    request_id: &str,
) -> Result<Response> {
    let header_key = request.headers().get("idempotency-key")?;
    if header_key != decoded.idempotency_key {
        return action_error(decoded.action, LegacyVideoPropertiesErrorV1::InvalidInput);
    }
    let database = env.d1("DB")?;
    let now_ms = crate::current_time_ms()?;
    let (actor_id, proof) = if decoded.action.anonymous() {
        (None, None)
    } else {
        match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms).await?
        {
            Ok(proof) => (Some(proof.user_id().to_string()), Some(proof)),
            Err(failure) => return browser_failure_response(failure),
        }
    };
    let rate_limit = match &actor_id {
        Some(actor_id) => {
            compatibility_rate_limit::admit_principal(
                env,
                &database,
                decoded.action.bucket(),
                actor_id,
                now_ms,
            )
            .await?
        }
        None => {
            compatibility_rate_limit::admit_edge_request(
                env,
                request,
                decoded.action.bucket(),
                now_ms,
            )
            .await?
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return action_error(decoded.action, LegacyVideoPropertiesErrorV1::Unavailable);
    }
    let outcome = execute(
        env,
        VideoPropertyExecutionV1 {
            caller: web_caller(),
            security: admitted_security(actor_id.is_some(), rate_limit),
            credential: if actor_id.is_some() {
                LegacyVideoPropertiesCredentialV1::Session
            } else {
                LegacyVideoPropertiesCredentialV1::Anonymous
            },
            actor_id,
            input: decoded.input.clone(),
            idempotency_key: decoded.idempotency_key.clone(),
            body_length: usize::try_from(decoded.body_length)
                .map_err(|_| Error::RustError("legacy action body length is invalid".into()))?,
            content_type: decoded.content_type.clone(),
            correlation_id: request_id.into(),
        },
    )
    .await;
    if let Some(proof) = &proof
        && !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, proof, now_ms)
            .await?
    {
        return action_error(decoded.action, LegacyVideoPropertiesErrorV1::Unavailable);
    }
    match outcome {
        Ok(outcome) => project_action_success(request, env, decoded.action, outcome.result),
        Err(error) => action_error(decoded.action, error),
    }
}

struct VideoPropertyExecutionV1 {
    caller: LegacyCallerV1,
    security: RequestSecurityContextV1,
    credential: LegacyVideoPropertiesCredentialV1,
    actor_id: Option<String>,
    input: LegacyVideoPropertiesInputV1,
    idempotency_key: Option<String>,
    body_length: usize,
    content_type: String,
    correlation_id: String,
}

async fn execute(
    env: &Env,
    execution: VideoPropertyExecutionV1,
) -> std::result::Result<
    frame_application::LegacyVideoPropertiesAtomicOutcomeV1,
    LegacyVideoPropertiesErrorV1,
> {
    let database = env
        .d1("DB")
        .map_err(|_| LegacyVideoPropertiesErrorV1::Unavailable)?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| LegacyVideoPropertiesErrorV1::Unavailable)?;
    let port = D1LegacyVideoPropertiesAtomicPortV1::new(&database);
    let envelope_key = execution
        .idempotency_key
        .clone()
        .map(IdempotencyKey::parse)
        .transpose()
        .map_err(|_| LegacyVideoPropertiesErrorV1::InvalidInput)?;
    transport
        .dispatch_video_properties(
            &port,
            LegacyVideoPropertiesInvocationV1 {
                caller: execution.caller,
                envelope: ApiMutationEnvelopeV1 {
                    content_length: u64::try_from(execution.body_length)
                        .map_err(|_| LegacyVideoPropertiesErrorV1::InvalidInput)?,
                    content_type: Some(execution.content_type),
                    idempotency_key: envelope_key,
                    correlation_id: execution.correlation_id,
                },
                security: execution.security,
                credential: execution.credential,
                actor_id: execution.actor_id,
                input: execution.input,
                idempotency_key: execution.idempotency_key,
            },
        )
        .await
}

fn decode_mobile_input(
    action: MobileVideoPropertyActionV1,
    video_id: String,
    bytes: &[u8],
) -> std::result::Result<LegacyVideoPropertiesInputV1, LegacyVideoPropertiesErrorV1> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| LegacyVideoPropertiesErrorV1::InvalidInput)?;
    let object = value
        .as_object()
        .ok_or(LegacyVideoPropertiesErrorV1::InvalidInput)?;
    match action {
        MobileVideoPropertyActionV1::Password => {
            let password = match object.get("password") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => return Err(LegacyVideoPropertiesErrorV1::InvalidInput),
            };
            Ok(LegacyVideoPropertiesInputV1::MobilePassword {
                legacy_video_id: video_id,
                password,
            })
        }
        MobileVideoPropertyActionV1::Sharing => Ok(LegacyVideoPropertiesInputV1::MobileSharing {
            legacy_video_id: video_id,
            public: object
                .get("public")
                .and_then(Value::as_bool)
                .ok_or(LegacyVideoPropertiesErrorV1::InvalidInput)?,
        }),
        MobileVideoPropertyActionV1::Title => Ok(LegacyVideoPropertiesInputV1::MobileTitle {
            legacy_video_id: video_id,
            title: required_string(object, "title")?,
        }),
    }
}

fn decode_action(
    action: WebActionV1,
    bytes: &[u8],
    now_ms: i64,
) -> std::result::Result<(LegacyVideoPropertiesInputV1, Option<String>), BrowserWebFailure> {
    match action {
        WebActionV1::EditDate => {
            let wire = serde_json::from_slice::<EditDateWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_wire(&wire.schema_version, &wire.video_id, &wire.idempotency_key)?;
            Ok((
                LegacyVideoPropertiesInputV1::EditDate {
                    legacy_video_id: wire.video_id,
                    date: wire.date,
                    now_ms,
                },
                wire.idempotency_key,
            ))
        }
        WebActionV1::EditTitle => {
            let wire = serde_json::from_slice::<EditTitleWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_wire(&wire.schema_version, &wire.video_id, &wire.idempotency_key)?;
            Ok((
                LegacyVideoPropertiesInputV1::EditTitle {
                    legacy_video_id: wire.video_id,
                    title: wire.title,
                },
                wire.idempotency_key,
            ))
        }
        WebActionV1::RemovePassword => {
            let wire = serde_json::from_slice::<VideoOnlyWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_wire(&wire.schema_version, &wire.video_id, &wire.idempotency_key)?;
            Ok((
                LegacyVideoPropertiesInputV1::RemovePassword {
                    legacy_video_id: wire.video_id,
                },
                wire.idempotency_key,
            ))
        }
        WebActionV1::SetPassword | WebActionV1::VerifyPassword => {
            let wire = serde_json::from_slice::<PasswordWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_wire(&wire.schema_version, &wire.video_id, &wire.idempotency_key)?;
            let input = if action == WebActionV1::SetPassword {
                LegacyVideoPropertiesInputV1::SetPassword {
                    legacy_video_id: wire.video_id,
                    password: wire.password,
                }
            } else {
                LegacyVideoPropertiesInputV1::VerifyPassword {
                    legacy_video_id: wire.video_id,
                    password: wire.password,
                }
            };
            Ok((input, wire.idempotency_key))
        }
        WebActionV1::UpdateSettings => {
            let wire = serde_json::from_slice::<SettingsWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_wire(&wire.schema_version, &wire.video_id, &wire.idempotency_key)?;
            Ok((
                LegacyVideoPropertiesInputV1::UpdateSettings {
                    legacy_video_id: wire.video_id,
                    settings: wire.video_settings,
                },
                wire.idempotency_key,
            ))
        }
    }
}

fn validate_wire(
    schema_version: &str,
    video_id: &str,
    idempotency_key: &Option<String>,
) -> std::result::Result<(), BrowserWebFailure> {
    if schema_version != ACTION_REQUEST_SCHEMA_V1
        || video_id.is_empty()
        || idempotency_key
            .as_deref()
            .is_some_and(|value| IdempotencyKey::parse(value).is_err())
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(())
}

fn project_action_success(
    request: &Request,
    env: &Env,
    action: WebActionV1,
    result: LegacyVideoPropertiesAtomicResultV1,
) -> Result<Response> {
    match (action, result) {
        (
            WebActionV1::EditDate | WebActionV1::EditTitle | WebActionV1::UpdateSettings,
            LegacyVideoPropertiesAtomicResultV1::SuccessObject,
        ) => json_response(200, &json!({"success": true})),
        (WebActionV1::SetPassword, LegacyVideoPropertiesAtomicResultV1::PasswordSet) => {
            json_response(
                200,
                &json!({"success": true, "value": "Password updated successfully"}),
            )
        }
        (WebActionV1::RemovePassword, LegacyVideoPropertiesAtomicResultV1::PasswordRemoved) => {
            json_response(
                200,
                &json!({"success": true, "value": "Password removed successfully"}),
            )
        }
        (
            WebActionV1::VerifyPassword,
            LegacyVideoPropertiesAtomicResultV1::PasswordVerified { matched_hash },
        ) => {
            let cookie = password_cookie(request, env, &matched_hash)?;
            let mut response =
                json_response(200, &json!({"success": true, "value": "Password verified"}))?;
            response.headers_mut().append("set-cookie", &cookie)?;
            Ok(response)
        }
        (WebActionV1::VerifyPassword, LegacyVideoPropertiesAtomicResultV1::PasswordRejected) => {
            json_response(
                200,
                &json!({"success": false, "error": "Failed to verify password"}),
            )
        }
        _ => Err(Error::RustError(
            "legacy video-property action result is invalid".into(),
        )),
    }
}

fn action_error(action: WebActionV1, error: LegacyVideoPropertiesErrorV1) -> Result<Response> {
    if action.password_result() {
        let message = match action {
            WebActionV1::SetPassword => "Failed to update password",
            WebActionV1::RemovePassword => "Failed to remove password",
            WebActionV1::VerifyPassword => "Failed to verify password",
            _ => unreachable!(),
        };
        return json_response(200, &json!({"success": false, "error": message}));
    }
    let status = match error {
        LegacyVideoPropertiesErrorV1::Unauthorized => 401,
        LegacyVideoPropertiesErrorV1::NotFound => 404,
        LegacyVideoPropertiesErrorV1::AccessDenied => 403,
        LegacyVideoPropertiesErrorV1::InvalidInput
        | LegacyVideoPropertiesErrorV1::UnsupportedDate => 400,
        LegacyVideoPropertiesErrorV1::Conflict => 409,
        LegacyVideoPropertiesErrorV1::Database
        | LegacyVideoPropertiesErrorV1::PasswordFailure
        | LegacyVideoPropertiesErrorV1::Unavailable
        | LegacyVideoPropertiesErrorV1::Internal => 500,
    };
    json_response(status, &json!({"success": false}))
}

fn browser_failure_response(error: BrowserWebFailure) -> Result<Response> {
    let status = match error {
        BrowserWebFailure::Unauthenticated => 401,
        BrowserWebFailure::Forbidden => 403,
        BrowserWebFailure::NotFound => 404,
        BrowserWebFailure::Invalid => 400,
        BrowserWebFailure::RateLimited => 429,
        BrowserWebFailure::Conflict => 409,
        BrowserWebFailure::Unavailable => 503,
    };
    json_response(status, &json!({"success": false}))
}

fn mobile_error(error: LegacyVideoPropertiesErrorV1) -> Result<Response> {
    let (status, tag) = match error {
        LegacyVideoPropertiesErrorV1::Unauthorized => (401, "Unauthorized"),
        LegacyVideoPropertiesErrorV1::InvalidInput
        | LegacyVideoPropertiesErrorV1::UnsupportedDate => (400, "BadRequest"),
        LegacyVideoPropertiesErrorV1::NotFound => (404, "NotFound"),
        LegacyVideoPropertiesErrorV1::AccessDenied => (403, "Forbidden"),
        LegacyVideoPropertiesErrorV1::Conflict => (409, "Conflict"),
        LegacyVideoPropertiesErrorV1::Database
        | LegacyVideoPropertiesErrorV1::PasswordFailure
        | LegacyVideoPropertiesErrorV1::Unavailable
        | LegacyVideoPropertiesErrorV1::Internal => (500, "InternalServerError"),
    };
    json_response(status, &json!({"_tag": tag}))
}

fn metadata_error() -> Result<Response> {
    json_response(401, &json!({"error": true}))
}

async fn decode_body(
    request: &mut Request,
) -> Result<std::result::Result<Vec<u8>, LegacyVideoPropertiesErrorV1>> {
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyVideoPropertiesErrorV1::InvalidInput));
    }
    let declared = match request.headers().get("content-length")? {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(Err(LegacyVideoPropertiesErrorV1::InvalidInput)),
        },
        None => None,
    };
    if declared.is_some_and(|value| value == 0 || value > MAX_BODY_BYTES) {
        return Ok(Err(LegacyVideoPropertiesErrorV1::InvalidInput));
    }
    let bytes = match crate::read_bounded_legacy_body(request, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(()) => return Ok(Err(LegacyVideoPropertiesErrorV1::InvalidInput)),
    };
    if bytes.is_empty()
        || bytes.len() > MAX_BODY_BYTES
        || declared.is_some_and(|value| value != bytes.len())
    {
        return Ok(Err(LegacyVideoPropertiesErrorV1::InvalidInput));
    }
    Ok(Ok(bytes))
}

async fn authenticate_mobile(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<
    std::result::Result<(String, LegacyVideoPropertiesCredentialV1), LegacyVideoPropertiesErrorV1>,
> {
    let authorization = request.headers().get("authorization")?;
    let api_key = authorization
        .as_deref()
        .and_then(|value| value.split(' ').nth(1))
        .filter(|value| value.len() == 36);
    if let Some(api_key) = api_key {
        let digest = sha256_hex(api_key.as_bytes());
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
        return Ok(
            row.map_or(Err(LegacyVideoPropertiesErrorV1::Unauthorized), |row| {
                Ok((row.user_id, LegacyVideoPropertiesCredentialV1::ApiKey))
            }),
        );
    }
    Ok(
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => Ok((actor_id, LegacyVideoPropertiesCredentialV1::Session)),
            Err(BrowserWebFailure::Unavailable) => Err(LegacyVideoPropertiesErrorV1::Unavailable),
            Err(_) => Err(LegacyVideoPropertiesErrorV1::Unauthorized),
        },
    )
}

fn optional_header_idempotency(
    request: &Request,
) -> Result<std::result::Result<Option<String>, LegacyVideoPropertiesErrorV1>> {
    let value = request.headers().get("idempotency-key")?;
    if value
        .as_deref()
        .is_some_and(|value| IdempotencyKey::parse(value).is_err())
    {
        return Ok(Err(LegacyVideoPropertiesErrorV1::InvalidInput));
    }
    Ok(Ok(value))
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
) -> std::result::Result<String, LegacyVideoPropertiesErrorV1> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(LegacyVideoPropertiesErrorV1::InvalidInput)
}

pub(crate) fn password_cookie(request: &Request, env: &Env, matched_hash: &str) -> Result<String> {
    let mut hashes = existing_password_hashes(request, env).unwrap_or_default();
    hashes.retain(|value| value != matched_hash);
    hashes.push(matched_hash.into());
    if hashes.len() > MAX_VERIFIED_HASHES {
        hashes.drain(..hashes.len() - MAX_VERIFIED_HASHES);
    }
    let plaintext = serde_json::to_vec(&hashes).map_err(json_error)?;
    let value = seal_cookie(env, &plaintext)?;
    Ok(format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; Secure; SameSite=Lax"
    ))
}

pub(crate) fn existing_password_hashes(request: &Request, env: &Env) -> Option<Vec<String>> {
    let cookies = request.headers().get("cookie").ok()??;
    let encoded = cookies.split(';').find_map(|part| {
        part.trim()
            .strip_prefix(&format!("{COOKIE_NAME}="))
            .map(str::to_owned)
    })?;
    let plaintext = open_cookie(env, &encoded).ok()?;
    let value = String::from_utf8(plaintext).ok()?;
    match serde_json::from_str::<Vec<String>>(&value) {
        Ok(values) => Some(values),
        Err(_) => Some(vec![value]),
    }
}

fn cookie_key(env: &Env) -> Result<[u8; 32]> {
    let secret = env
        .secret(COOKIE_KEY_SECRET)
        .map_err(|_| Error::RustError("password-cookie authority is unavailable".into()))?;
    let mut encoded = secret.to_string();
    let result = decode_hex_key(&encoded)
        .ok_or_else(|| Error::RustError("password-cookie authority is invalid".into()));
    encoded.zeroize();
    result
}

fn seal_cookie(env: &Env, plaintext: &[u8]) -> Result<String> {
    let mut key = cookie_key(env)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| Error::RustError("password-cookie authority is invalid".into()))?;
    key.zeroize();
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce)
        .map_err(|_| Error::RustError("password-cookie randomness is unavailable".into()))?;
    let mut ciphertext = plaintext.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), COOKIE_AAD, &mut ciphertext)
        .map_err(|_| Error::RustError("password-cookie encryption failed".into()))?;
    let mut envelope = Vec::with_capacity(1 + NONCE_BYTES + ciphertext.len() + TAG_BYTES);
    envelope.push(COOKIE_VERSION);
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    envelope.extend_from_slice(&tag);
    Ok(base64_url_encode(&envelope))
}

fn open_cookie(env: &Env, encoded: &str) -> Result<Vec<u8>> {
    let envelope = base64_url_decode(encoded)
        .ok_or_else(|| Error::RustError("password cookie is malformed".into()))?;
    if envelope.len() < 1 + NONCE_BYTES + TAG_BYTES || envelope[0] != COOKIE_VERSION {
        return Err(Error::RustError("password cookie is malformed".into()));
    }
    let nonce = Nonce::from_slice(&envelope[1..1 + NONCE_BYTES]);
    let mut ciphertext = envelope[1 + NONCE_BYTES..].to_vec();
    let plaintext_len = ciphertext.len() - TAG_BYTES;
    let tag = Tag::clone_from_slice(&ciphertext[plaintext_len..]);
    let mut key = cookie_key(env)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| Error::RustError("password-cookie authority is invalid".into()))?;
    key.zeroize();
    cipher
        .decrypt_in_place_detached(nonce, COOKIE_AAD, &mut ciphertext[..plaintext_len], &tag)
        .map_err(|_| Error::RustError("password cookie is invalid".into()))?;
    ciphertext.truncate(plaintext_len);
    Ok(ciphertext)
}

fn decode_hex_key(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = hex_nibble(pair[0])? << 4 | hex_nibble(pair[1])?;
    }
    (!output.iter().all(|byte| *byte == 0)).then_some(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
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

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    let mut output = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 15)]));
    }
    output
}

fn json_response(status: u16, value: &Value) -> Result<Response> {
    let mut response =
        Response::from_bytes(serde_json::to_vec(value).map_err(json_error)?)?.with_status(status);
    response
        .headers_mut()
        .set("content-type", "application/json")?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn json_error(_: serde_json::Error) -> Error {
    Error::RustError("legacy video-property JSON projection failed".into())
}

const fn admitted_security(
    authenticated: bool,
    rate_limit: RateLimitDecisionV1,
) -> RequestSecurityContextV1 {
    RequestSecurityContextV1 {
        authenticated,
        authorized: true,
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_wires_keep_three_distinct_source_contracts() {
        assert!(matches!(
            decode_mobile_input(
                MobileVideoPropertyActionV1::Password,
                "video".into(),
                br#"{"password":null}"#,
            ),
            Ok(LegacyVideoPropertiesInputV1::MobilePassword { password: None, .. })
        ));
        assert!(matches!(
            decode_mobile_input(
                MobileVideoPropertyActionV1::Sharing,
                "video".into(),
                br#"{"public":true}"#,
            ),
            Ok(LegacyVideoPropertiesInputV1::MobileSharing { public: true, .. })
        ));
        assert!(
            decode_mobile_input(
                MobileVideoPropertyActionV1::Title,
                "video".into(),
                br#"{"title":null}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn action_wire_allows_internal_idempotency_and_preserves_password_whitespace() {
        let (input, key) = decode_action(
            WebActionV1::SetPassword,
            br#"{"schema_version":"frame.web-video-property-action-request.v1","video_id":"video","password":"  secret  "}"#,
            1,
        )
        .expect("wire");
        assert!(key.is_none());
        assert!(matches!(
            input,
            LegacyVideoPropertiesInputV1::SetPassword { password, .. } if password == "  secret  "
        ));
    }

    #[test]
    fn cookie_envelope_primitives_are_canonical_and_bounded() {
        let key = decode_hex_key(&"01".repeat(32)).expect("key");
        assert_ne!(key, [0; 32]);
        assert!(decode_hex_key(&"00".repeat(32)).is_none());
        for bytes in [vec![0], vec![1, 2], vec![3, 4, 5], (0..48).collect()] {
            let encoded = base64_url_encode(&bytes);
            assert_eq!(base64_url_decode(&encoded), Some(bytes));
        }
        assert_eq!(MAX_VERIFIED_HASHES, 10);
    }
}
