//! Exact HTTP ingress for Cap's mobile folder-create route and Effect-RPC
//! folder CRUD protocol.
//!
//! The mobile carrier and the RPC carrier intentionally do not share a wire
//! schema. Mobile accepts Cap's legacy 36-character API key or the host-only
//! session cookie and returns a folder projection. Effect RPC accepts one
//! tagged request per POST, authenticates only with the host-only session, and
//! returns the pinned JSON-array `Exit` protocol. The three RPC operations are
//! registered as locally complete but remain production-gated; the ingress
//! therefore returns a stable typed failure when the production compatibility
//! registry refuses execution.

use frame_application::{
    LEGACY_FOLDER_CRUD_CONTENT_TYPE, LEGACY_FOLDER_CRUD_MAX_BODY_BYTES,
    LEGACY_PUBLIC_PAGE_CTA_LABEL_MAX_UTF16_CODE_UNITS,
    LEGACY_PUBLIC_PAGE_CTA_URL_MAX_UTF16_CODE_UNITS,
    LEGACY_PUBLIC_PAGE_SUBTITLE_MAX_UTF16_CODE_UNITS,
    LEGACY_PUBLIC_PAGE_TITLE_MAX_UTF16_CODE_UNITS, LegacyCallerV1, LegacyFolderCrudCredentialV1,
    LegacyFolderCrudErrorV1, LegacyFolderCrudInputV1, LegacyFolderCrudSuccessV1,
    LegacyFolderLayoutV1, LegacyFolderLogoModeV1, LegacyFolderParentPatchV1,
    LegacyFolderPublicPagePatchV1, RateLimitDecisionV1, RequestSecurityContextV1,
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

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1, LegacyFolderCrudInvocationV1,
    },
    legacy_folder_crud_runtime::D1LegacyFolderCrudAtomicPortV1,
};

const RPC_PARSE_FAILURE: &str = "Invalid Effect RPC request payload";
const RPC_UNKNOWN_TAG_FAILURE: &str = "Unknown Effect RPC request tag";

#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderAuthorityV1 {
    actor_id: String,
    organization_id: String,
    credential: LegacyFolderCrudCredentialV1,
}

#[derive(Debug, Deserialize)]
struct ApiKeyActorRowV1 {
    user_id: String,
}

#[derive(Debug, Clone, PartialEq)]
struct DecodedRpcRequestV1 {
    id: String,
    input: LegacyFolderCrudInputV1,
}

pub async fn mobile_create_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
) -> Result<Response> {
    let bytes = match decode_body(request).await? {
        Ok(bytes) => bytes,
        Err(failure) => return mobile_failure_response(failure),
    };
    let input = match decode_mobile_create(&bytes) {
        Ok(input) => input,
        Err(failure) => return mobile_failure_response(failure),
    };
    let idempotency_key = match request.headers().get("idempotency-key")? {
        Some(value) => match IdempotencyKey::parse(value.clone()) {
            Ok(_) => Some(value),
            Err(_) => return mobile_failure_response(LegacyFolderCrudErrorV1::InvalidInput),
        },
        None => None,
    };
    let now_ms = crate::current_time_ms()?;
    let authority = match authenticate_mobile(request, env, now_ms).await? {
        Ok(authority) => authority,
        Err(failure) => return mobile_failure_response(failure),
    };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &authority.actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return tagged_http_error(429, "TooManyRequests");
    }
    let authenticated = LegacyAuthenticatedContextV1::new(
        authority.actor_id.clone(),
        authority.organization_id.clone(),
    )
    .map_err(|_| Error::RustError("legacy folder authority is invalid".into()))?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
    let port = D1LegacyFolderCrudAtomicPortV1::new(&database);
    let result = transport
        .dispatch_folder_crud(
            &port,
            LegacyFolderCrudInvocationV1 {
                caller: mobile_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: u64::try_from(bytes.len()).map_err(|_| {
                        Error::RustError("legacy folder body length is invalid".into())
                    })?,
                    content_type: Some(LEGACY_FOLDER_CRUD_CONTENT_TYPE.into()),
                    idempotency_key: idempotency_key
                        .clone()
                        .map(IdempotencyKey::parse)
                        .transpose()
                        .map_err(|_| {
                            Error::RustError("legacy mobile idempotency key is invalid".into())
                        })?,
                    correlation_id: request_id.to_owned(),
                },
                security: admitted_security(rate_limit),
                authenticated,
                credential: authority.credential,
                input,
                idempotency_key,
            },
        )
        .await;
    match result {
        Ok(LegacyFolderCrudSuccessV1::MobileFolder {
            id,
            name,
            color,
            parent_id,
            video_count,
            ..
        }) => json_http_response(
            200,
            &json!({
                "id": id,
                "name": name,
                "color": color,
                "parentId": parent_id,
                "videoCount": video_count,
            }),
        ),
        Ok(LegacyFolderCrudSuccessV1::RpcVoid { .. }) => Err(Error::RustError(
            "legacy mobile folder result is invalid".into(),
        )),
        Err(error) => mobile_folder_error_response(error),
    }
}

pub async fn effect_rpc_response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
) -> Result<Response> {
    let bytes = match decode_body(request).await? {
        Ok(bytes) => bytes,
        Err(_) => return rpc_response(rpc_defect(RPC_PARSE_FAILURE)),
    };
    if crate::legacy_video_lifecycle_web_runtime::is_video_lifecycle_rpc_request(&bytes) {
        return crate::legacy_video_lifecycle_web_runtime::effect_rpc_response_from_bytes(
            &bytes, request, env, request_id,
        )
        .await;
    }
    if crate::legacy_analytics_web_runtime::is_analytics_rpc_request(&bytes) {
        return crate::legacy_analytics_web_runtime::effect_rpc_response_from_bytes(
            &bytes, request, env, request_id,
        )
        .await;
    }
    if crate::legacy_user_account_web_runtime::is_user_rpc_request(&bytes) {
        return crate::legacy_user_account_web_runtime::effect_rpc_response_from_bytes(
            &bytes, request, env, request_id,
        )
        .await;
    }
    if crate::legacy_upload_storage_web_runtime::is_upload_storage_rpc_request(&bytes) {
        return crate::legacy_upload_storage_web_runtime::effect_rpc_response_from_bytes(
            &bytes, request, env, request_id,
        )
        .await;
    }
    if crate::legacy_protected_integrations_web_runtime::is_protected_integration_rpc_request(
        &bytes,
    ) {
        return crate::legacy_protected_integrations_web_runtime::effect_rpc_response_from_bytes(
            &bytes, request, env, request_id,
        )
        .await;
    }
    if crate::legacy_protected_media_web_runtime::is_protected_media_rpc_request(&bytes) {
        return crate::legacy_protected_media_web_runtime::effect_rpc_response_from_bytes(
            &bytes, request, env, request_id,
        )
        .await;
    }
    let decoded = match decode_rpc_request(&bytes) {
        Ok(decoded) => decoded,
        Err(RpcDecodeFailureV1::Malformed { request_id }) => {
            return rpc_response(match request_id {
                Some(request_id) => rpc_die(&request_id, RPC_PARSE_FAILURE),
                None => rpc_defect(RPC_PARSE_FAILURE),
            });
        }
        Err(RpcDecodeFailureV1::UnknownTag) => {
            return rpc_response(rpc_defect(RPC_UNKNOWN_TAG_FAILURE));
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
                json!({"_tag": "InternalError", "type": "database"}),
            ));
        }
        Err(_) => {
            return rpc_response(rpc_typed_failure(
                &decoded.id,
                json!({"_tag": "UnauthenticatedError"}),
            ));
        }
    };
    let database = env.d1("DB")?;
    let organization_id =
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await? {
            Some(organization_id) => organization_id,
            None => {
                return rpc_response(rpc_typed_failure(
                    &decoded.id,
                    json!({"_tag": "PolicyDenied"}),
                ));
            }
        };
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
            json!({"_tag": "InternalError", "type": "unknown"}),
        ));
    }
    let authenticated = LegacyAuthenticatedContextV1::new(actor_id, organization_id)
        .map_err(|_| Error::RustError("legacy RPC folder authority is invalid".into()))?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
    let port = D1LegacyFolderCrudAtomicPortV1::new(&database);
    let result = transport
        .dispatch_folder_crud(
            &port,
            LegacyFolderCrudInvocationV1 {
                caller: rpc_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: u64::try_from(bytes.len()).map_err(|_| {
                        Error::RustError("legacy RPC body length is invalid".into())
                    })?,
                    content_type: Some(LEGACY_FOLDER_CRUD_CONTENT_TYPE.into()),
                    idempotency_key: None,
                    correlation_id: request_id.to_owned(),
                },
                security: admitted_security(rate_limit),
                authenticated,
                credential: LegacyFolderCrudCredentialV1::Session,
                input: decoded.input,
                idempotency_key: None,
            },
        )
        .await;
    let value = match result {
        Ok(LegacyFolderCrudSuccessV1::RpcVoid { .. }) => rpc_success(&decoded.id),
        Ok(LegacyFolderCrudSuccessV1::MobileFolder { .. }) => rpc_typed_failure(
            &decoded.id,
            json!({"_tag": "InternalError", "type": "unknown"}),
        ),
        Err(error) => rpc_folder_error(&decoded.id, error),
    };
    rpc_response(value)
}

async fn decode_body(request: &mut Request) -> Result<Result<Vec<u8>, LegacyFolderCrudErrorV1>> {
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyFolderCrudErrorV1::InvalidInput));
    }
    let declared_length = match request.headers().get("content-length")? {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(Err(LegacyFolderCrudErrorV1::InvalidInput)),
        },
        None => None,
    };
    if declared_length
        .is_some_and(|length| length == 0 || length > LEGACY_FOLDER_CRUD_MAX_BODY_BYTES)
    {
        return Ok(Err(LegacyFolderCrudErrorV1::InvalidInput));
    }
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_FOLDER_CRUD_MAX_BODY_BYTES).await {
            Ok(bytes) => bytes,
            Err(()) => return Ok(Err(LegacyFolderCrudErrorV1::InvalidInput)),
        };
    if bytes.is_empty()
        || bytes.len() > LEGACY_FOLDER_CRUD_MAX_BODY_BYTES
        || declared_length.is_some_and(|length| length != bytes.len())
    {
        return Ok(Err(LegacyFolderCrudErrorV1::InvalidInput));
    }
    Ok(Ok(bytes))
}

fn decode_mobile_create(bytes: &[u8]) -> Result<LegacyFolderCrudInputV1, LegacyFolderCrudErrorV1> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| LegacyFolderCrudErrorV1::InvalidInput)?;
    let object = value
        .as_object()
        .ok_or(LegacyFolderCrudErrorV1::InvalidInput)?;
    let name = required_string(object, "name")?;
    let color = optional_string(object, "color")?;
    if color
        .as_deref()
        .is_some_and(|value| !matches!(value, "normal" | "blue" | "red" | "yellow"))
    {
        return Err(LegacyFolderCrudErrorV1::InvalidInput);
    }
    Ok(LegacyFolderCrudInputV1::MobileCreate { name, color })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RpcDecodeFailureV1 {
    Malformed { request_id: Option<String> },
    UnknownTag,
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
        "FolderCreate" => decode_rpc_create(payload),
        "FolderDelete" => payload
            .as_str()
            .map(|folder_id| LegacyFolderCrudInputV1::RpcDelete {
                folder_id: folder_id.to_owned(),
            })
            .ok_or(LegacyFolderCrudErrorV1::InvalidInput),
        "FolderUpdate" => decode_rpc_update(payload),
        _ => return Err(RpcDecodeFailureV1::UnknownTag),
    }
    .map_err(|_| malformed())?;
    Ok(DecodedRpcRequestV1 { id, input })
}

fn decode_rpc_create(value: &Value) -> Result<LegacyFolderCrudInputV1, LegacyFolderCrudErrorV1> {
    let object = value
        .as_object()
        .ok_or(LegacyFolderCrudErrorV1::InvalidInput)?;
    let name = required_string(object, "name")?;
    let color = required_string(object, "color")?;
    if !matches!(color.as_str(), "normal" | "blue" | "red" | "yellow") {
        return Err(LegacyFolderCrudErrorV1::InvalidInput);
    }
    Ok(LegacyFolderCrudInputV1::RpcCreate {
        name,
        color,
        public: optional_bool(object, "public")?,
        space_id: optional_string(object, "spaceId")?,
        parent_id: optional_string(object, "parentId")?,
    })
}

fn decode_rpc_update(value: &Value) -> Result<LegacyFolderCrudInputV1, LegacyFolderCrudErrorV1> {
    let object = value
        .as_object()
        .ok_or(LegacyFolderCrudErrorV1::InvalidInput)?;
    let folder_id = required_string(object, "id")?;
    let color = optional_string(object, "color")?;
    if color
        .as_deref()
        .is_some_and(|value| !matches!(value, "normal" | "blue" | "red" | "yellow"))
    {
        return Err(LegacyFolderCrudErrorV1::InvalidInput);
    }
    let parent_id = match object.get("parentId") {
        None => LegacyFolderParentPatchV1::Absent,
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("None") =>
        {
            LegacyFolderParentPatchV1::Root
        }
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("Some") =>
        {
            LegacyFolderParentPatchV1::Parent(required_string(option, "value")?)
        }
        Some(_) => return Err(LegacyFolderCrudErrorV1::InvalidInput),
    };
    let public_page = match object.get("publicPage") {
        None => None,
        Some(Value::Object(value)) => Some(decode_public_page(value)?),
        Some(_) => return Err(LegacyFolderCrudErrorV1::InvalidInput),
    };
    Ok(LegacyFolderCrudInputV1::RpcUpdate {
        folder_id,
        name: optional_string(object, "name")?,
        color,
        public: optional_bool(object, "public")?,
        public_page,
        parent_id,
    })
}

fn decode_public_page(
    object: &Map<String, Value>,
) -> Result<LegacyFolderPublicPagePatchV1, LegacyFolderCrudErrorV1> {
    let logo_mode = optional_string(object, "logoMode")?
        .map(|value| match value.as_str() {
            "cap" => Ok(LegacyFolderLogoModeV1::Cap),
            "organization" => Ok(LegacyFolderLogoModeV1::Organization),
            "custom" => Ok(LegacyFolderLogoModeV1::Custom),
            "none" => Ok(LegacyFolderLogoModeV1::None),
            _ => Err(LegacyFolderCrudErrorV1::InvalidInput),
        })
        .transpose()?;
    let layout = optional_string(object, "layout")?
        .map(|value| match value.as_str() {
            "grid" => Ok(LegacyFolderLayoutV1::Grid),
            "list" => Ok(LegacyFolderLayoutV1::List),
            _ => Err(LegacyFolderCrudErrorV1::InvalidInput),
        })
        .transpose()?;
    let grid_columns = match object.get("gridColumns") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .filter(|value| matches!(value, 2..=5))
                .ok_or(LegacyFolderCrudErrorV1::InvalidInput)?,
        ),
    };
    Ok(LegacyFolderPublicPagePatchV1 {
        hide_title: optional_bool(object, "hideTitle")?,
        hide_copy_link: optional_bool(object, "hideCopyLink")?,
        logo_mode,
        title: optional_bounded_string(
            object,
            "title",
            LEGACY_PUBLIC_PAGE_TITLE_MAX_UTF16_CODE_UNITS,
        )?,
        subtitle: optional_bounded_string(
            object,
            "subtitle",
            LEGACY_PUBLIC_PAGE_SUBTITLE_MAX_UTF16_CODE_UNITS,
        )?,
        cta_label: optional_bounded_string(
            object,
            "ctaLabel",
            LEGACY_PUBLIC_PAGE_CTA_LABEL_MAX_UTF16_CODE_UNITS,
        )?,
        cta_url: optional_bounded_string(
            object,
            "ctaUrl",
            LEGACY_PUBLIC_PAGE_CTA_URL_MAX_UTF16_CODE_UNITS,
        )?,
        layout,
        grid_columns,
    })
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<String, LegacyFolderCrudErrorV1> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(LegacyFolderCrudErrorV1::InvalidInput)
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, LegacyFolderCrudErrorV1> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(LegacyFolderCrudErrorV1::InvalidInput),
    }
}

fn optional_bool(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, LegacyFolderCrudErrorV1> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(LegacyFolderCrudErrorV1::InvalidInput),
    }
}

fn optional_bounded_string(
    object: &Map<String, Value>,
    key: &str,
    maximum_utf16_code_units: usize,
) -> Result<Option<String>, LegacyFolderCrudErrorV1> {
    let value = optional_string(object, key)?;
    if value
        .as_deref()
        .is_some_and(|value| value.encode_utf16().count() > maximum_utf16_code_units)
    {
        return Err(LegacyFolderCrudErrorV1::InvalidInput);
    }
    Ok(value)
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

async fn authenticate_mobile(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<Result<FolderAuthorityV1, LegacyFolderCrudErrorV1>> {
    let authorization = request.headers().get("authorization")?;
    let legacy_api_key = authorization
        .as_deref()
        .and_then(|value| value.split(' ').nth(1))
        .filter(|value| value.len() == 36);
    if let Some(api_key) = legacy_api_key {
        return authenticate_mobile_api_key(api_key, env, now_ms).await;
    }
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unavailable) => {
                return Ok(Err(LegacyFolderCrudErrorV1::Unavailable));
            }
            Err(_) => return Ok(Err(LegacyFolderCrudErrorV1::Unauthorized)),
        };
    authority_for_actor(
        &env.d1("DB")?,
        actor_id,
        LegacyFolderCrudCredentialV1::Session,
    )
    .await
}

async fn authenticate_mobile_api_key(
    api_key: &str,
    env: &Env,
    now_ms: i64,
) -> Result<Result<FolderAuthorityV1, LegacyFolderCrudErrorV1>> {
    let digest = sha256_hex(api_key.as_bytes());
    let row = env
        .d1("DB")?
        .prepare(
            "SELECT k.user_id FROM auth_api_keys k \
             JOIN users u ON u.id=k.user_id \
             WHERE k.key_digest=?1 AND k.revoked_at_ms IS NULL \
               AND (k.expires_at_ms IS NULL OR k.expires_at_ms>?2) \
               AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(&digest), JsValue::from_f64(now_ms as f64)])?
        .first::<ApiKeyActorRowV1>(None)
        .await?;
    let Some(row) = row else {
        return Ok(Err(LegacyFolderCrudErrorV1::Unauthorized));
    };
    authority_for_actor(
        &env.d1("DB")?,
        row.user_id,
        LegacyFolderCrudCredentialV1::ApiKey,
    )
    .await
}

async fn authority_for_actor(
    database: &worker::D1Database,
    actor_id: String,
    credential: LegacyFolderCrudCredentialV1,
) -> Result<Result<FolderAuthorityV1, LegacyFolderCrudErrorV1>> {
    let Some(organization_id) =
        browser_web_runtime::trusted_active_organization_id(database, &actor_id).await?
    else {
        return Ok(Err(LegacyFolderCrudErrorV1::AccessDenied));
    };
    Ok(Ok(FolderAuthorityV1 {
        actor_id,
        organization_id,
        credential,
    }))
}

fn mobile_folder_error_response(error: LegacyFolderCrudErrorV1) -> Result<Response> {
    match error {
        LegacyFolderCrudErrorV1::Unauthorized => tagged_http_error(401, "Unauthorized"),
        LegacyFolderCrudErrorV1::InvalidInput => tagged_http_error(400, "BadRequest"),
        LegacyFolderCrudErrorV1::AccessDenied => tagged_http_error(403, "Forbidden"),
        LegacyFolderCrudErrorV1::NotFound | LegacyFolderCrudErrorV1::ParentNotFound => {
            tagged_http_error(404, "NotFound")
        }
        LegacyFolderCrudErrorV1::RecursiveDefinition
        | LegacyFolderCrudErrorV1::ScopeConflict
        | LegacyFolderCrudErrorV1::Conflict => tagged_http_error(409, "Conflict"),
        LegacyFolderCrudErrorV1::Unavailable
        | LegacyFolderCrudErrorV1::Internal
        | LegacyFolderCrudErrorV1::Database => tagged_http_error(500, "InternalServerError"),
    }
}

fn mobile_failure_response(error: LegacyFolderCrudErrorV1) -> Result<Response> {
    mobile_folder_error_response(error)
}

fn rpc_folder_error(request_id: &str, error: LegacyFolderCrudErrorV1) -> Value {
    let payload = match error {
        LegacyFolderCrudErrorV1::Unauthorized => json!({"_tag": "UnauthenticatedError"}),
        LegacyFolderCrudErrorV1::AccessDenied => json!({"_tag": "PolicyDenied"}),
        LegacyFolderCrudErrorV1::NotFound | LegacyFolderCrudErrorV1::InvalidInput => {
            json!({"_tag": "FolderNotFoundError"})
        }
        LegacyFolderCrudErrorV1::ParentNotFound => json!({"_tag": "ParentNotFoundError"}),
        LegacyFolderCrudErrorV1::RecursiveDefinition => {
            json!({"_tag": "RecursiveDefinitionError"})
        }
        LegacyFolderCrudErrorV1::ScopeConflict
        | LegacyFolderCrudErrorV1::Conflict
        | LegacyFolderCrudErrorV1::Unavailable
        | LegacyFolderCrudErrorV1::Internal => {
            json!({"_tag": "InternalError", "type": "unknown"})
        }
        LegacyFolderCrudErrorV1::Database => {
            json!({"_tag": "InternalError", "type": "database"})
        }
    };
    rpc_typed_failure(request_id, payload)
}

pub(crate) fn rpc_success(request_id: &str) -> Value {
    json!([{
        "_tag": "Exit",
        "requestId": request_id,
        "exit": {"_tag": "Success"},
    }])
}

fn rpc_typed_failure(request_id: &str, error: Value) -> Value {
    json!([{
        "_tag": "Exit",
        "requestId": request_id,
        "exit": {
            "_tag": "Failure",
            "cause": {"_tag": "Fail", "error": error},
        },
    }])
}

fn rpc_die(request_id: &str, message: &str) -> Value {
    json!([{
        "_tag": "Exit",
        "requestId": request_id,
        "exit": {
            "_tag": "Failure",
            "cause": {"_tag": "Die", "defect": message},
        },
    }])
}

fn rpc_defect(message: &str) -> Value {
    json!([{"_tag": "Defect", "defect": message}])
}

fn rpc_response(value: Value) -> Result<Response> {
    json_http_response(200, &value)
}

fn tagged_http_error(status: u16, tag: &str) -> Result<Response> {
    json_http_response(status, &json!({"_tag": tag}))
}

fn json_http_response(status: u16, value: &Value) -> Result<Response> {
    let body = serde_json::to_vec(value)
        .map_err(|_| Error::RustError("legacy folder response is invalid".into()))?;
    let mut response = Response::from_bytes(body)?.with_status(status);
    response
        .headers_mut()
        .set("content-type", LEGACY_FOLDER_CRUD_CONTENT_TYPE)?;
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

const fn rpc_caller() -> LegacyCallerV1 {
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
        // These carriers are not Frame browser mutations. Their exact auth
        // boundary has already completed and Cap does not send Frame CSRF
        // material, so the generic gateway receives the non-browser exemption.
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
    }
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    let mut output = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_schema_defaults_color_rejects_null_and_strips_excess_fields() {
        let input = decode_mobile_create(br#"{"name":"  Launches  ","extra":"stripped"}"#)
            .expect("mobile input");
        assert_eq!(
            input,
            LegacyFolderCrudInputV1::MobileCreate {
                name: "  Launches  ".into(),
                color: None,
            }
        );
        assert!(decode_mobile_create(br#"{"name":"Folder","color":null}"#).is_err());
        assert!(decode_mobile_create(br#"[{"name":"Folder"}]"#).is_err());
    }

    #[test]
    fn rpc_transport_accepts_one_request_and_preserves_option_three_state() {
        let absent = decode_rpc_request(
            br#"{"_tag":"Request","id":"0","tag":"FolderUpdate","payload":{"id":"0123456789abcde"},"headers":[]}"#,
        )
        .expect("absent parent");
        assert!(matches!(
            absent.input,
            LegacyFolderCrudInputV1::RpcUpdate {
                parent_id: LegacyFolderParentPatchV1::Absent,
                ..
            }
        ));
        let root = decode_rpc_request(
            br#"{"_tag":"Request","id":"1","tag":"FolderUpdate","payload":{"id":"0123456789abcde","parentId":{"_tag":"None"}},"headers":[]}"#,
        )
        .expect("root parent");
        assert!(matches!(
            root.input,
            LegacyFolderCrudInputV1::RpcUpdate {
                parent_id: LegacyFolderParentPatchV1::Root,
                ..
            }
        ));
        let parent = decode_rpc_request(
            br#"{"_tag":"Request","id":"2","tag":"FolderUpdate","payload":{"id":"0123456789abcde","parentId":{"_tag":"Some","value":"1123456789abcde"}},"headers":[]}"#,
        )
        .expect("some parent");
        assert!(matches!(
            parent.input,
            LegacyFolderCrudInputV1::RpcUpdate {
                parent_id: LegacyFolderParentPatchV1::Parent(ref value),
                ..
            } if value == "1123456789abcde"
        ));
        assert!(matches!(
            decode_rpc_request(
                br#"{"_tag":"Request","id":"3","tag":"FolderUpdate","payload":{"id":"0123456789abcde","parentId":null},"headers":[]}"#,
            ),
            Err(RpcDecodeFailureV1::Malformed { .. })
        ));
    }

    #[test]
    fn rpc_transport_rejects_batching_and_unknown_tags_without_json_rpc() {
        assert!(matches!(
            decode_rpc_request(
                br#"[{"_tag":"Request","id":"0","tag":"FolderDelete","payload":"0123456789abcde","headers":[]}]"#,
            ),
            Err(RpcDecodeFailureV1::Malformed { request_id: None })
        ));
        assert_eq!(
            decode_rpc_request(
                br#"{"jsonrpc":"2.0","id":"0","method":"FolderDelete","params":"0123456789abcde"}"#,
            ),
            Err(RpcDecodeFailureV1::Malformed {
                request_id: Some("0".into()),
            })
        );
        assert_eq!(
            decode_rpc_request(
                br#"{"_tag":"Request","id":"0","tag":"Unknown","payload":{},"headers":[]}"#,
            ),
            Err(RpcDecodeFailureV1::UnknownTag)
        );
    }

    #[test]
    fn rpc_exit_projection_is_the_pinned_buffered_json_array() {
        assert_eq!(
            rpc_success("7"),
            json!([{
                "_tag": "Exit",
                "requestId": "7",
                "exit": {"_tag": "Success"},
            }])
        );
        assert_eq!(
            rpc_typed_failure("8", json!({"_tag": "UnauthenticatedError"})),
            json!([{
                "_tag": "Exit",
                "requestId": "8",
                "exit": {
                    "_tag": "Failure",
                    "cause": {
                        "_tag": "Fail",
                        "error": {"_tag": "UnauthenticatedError"},
                    },
                },
            }])
        );
    }

    #[test]
    fn mobile_api_key_selector_matches_caps_literal_split_and_length_rule() {
        fn candidate(value: &str) -> Option<&str> {
            value.split(' ').nth(1).filter(|value| value.len() == 36)
        }
        assert_eq!(
            candidate("Bearer 01234567-89ab-cdef-0123-456789abcdef"),
            Some("01234567-89ab-cdef-0123-456789abcdef")
        );
        assert_eq!(candidate("Bearer short"), None);
        assert_eq!(
            candidate("Bearer  01234567-89ab-cdef-0123-456789abcdef"),
            None
        );
    }

    #[test]
    fn public_page_strips_logo_url_but_validates_effect_schema_fields() {
        let decoded = decode_rpc_request(
            br#"{"_tag":"Request","id":"9","tag":"FolderUpdate","payload":{"id":"0123456789abcde","publicPage":{"logoUrl":"not-client-writable","logoMode":"custom","layout":"grid","gridColumns":5}},"headers":[]}"#,
        )
        .expect("public page");
        let LegacyFolderCrudInputV1::RpcUpdate {
            public_page: Some(public_page),
            ..
        } = decoded.input
        else {
            panic!("update public page")
        };
        assert_eq!(public_page.logo_mode, Some(LegacyFolderLogoModeV1::Custom));
        assert_eq!(public_page.layout, Some(LegacyFolderLayoutV1::Grid));
        assert_eq!(public_page.grid_columns, Some(5));
        assert!(decode_rpc_request(
            br#"{"_tag":"Request","id":"10","tag":"FolderUpdate","payload":{"id":"0123456789abcde","publicPage":{"gridColumns":6}},"headers":[]}"#,
        )
        .is_err());

        let overlong_title = "x".repeat(LEGACY_PUBLIC_PAGE_TITLE_MAX_UTF16_CODE_UNITS + 1);
        let encoded = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": "10",
            "tag": "FolderUpdate",
            "payload": {
                "id": "0123456789abcde",
                "publicPage": {"title": overlong_title},
            },
            "headers": [],
        }))
        .expect("encode overlong public-page request");
        assert_eq!(
            decode_rpc_request(&encoded),
            Err(RpcDecodeFailureV1::Malformed {
                request_id: Some("10".into()),
            })
        );
        assert_eq!(
            rpc_die("10", RPC_PARSE_FAILURE)[0]["exit"]["cause"]["_tag"],
            "Die"
        );
    }

    #[test]
    fn folder_delete_has_no_wire_idempotency_field_and_projects_an_exit() {
        let decoded = decode_rpc_request(
            br#"{"_tag":"Request","id":"11","tag":"FolderDelete","payload":"0123456789abcde","headers":[]}"#,
        )
        .expect("folder delete");
        assert!(matches!(
            decoded.input,
            LegacyFolderCrudInputV1::RpcDelete { ref folder_id }
                if folder_id == "0123456789abcde"
        ));
        assert_eq!(rpc_success(&decoded.id)[0]["requestId"], "11");

        let arbitrary = decode_rpc_request(
            br#"{"_tag":"Request","id":"12","tag":"FolderDelete","payload":"not-a-cap-nanoid","headers":[]}"#,
        )
        .expect("folder ids are raw branded strings on the Effect wire");
        assert!(matches!(
            arbitrary.input,
            LegacyFolderCrudInputV1::RpcDelete { ref folder_id }
                if folder_id == "not-a-cap-nanoid"
        ));
    }

    #[test]
    fn persistence_and_typed_failure_projection_keep_the_pinned_distinctions() {
        let overlong_name = "x".repeat(256);
        let encoded = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": "13",
            "tag": "FolderCreate",
            "payload": {"name": overlong_name, "color": "normal"},
            "headers": [],
        }))
        .expect("encode overlong name request");
        assert!(decode_rpc_request(&encoded).is_ok());
        assert_eq!(
            rpc_folder_error("13", LegacyFolderCrudErrorV1::Database)[0]["exit"]["cause"]["error"],
            json!({"_tag": "InternalError", "type": "database"})
        );
        assert_eq!(
            rpc_folder_error("14", LegacyFolderCrudErrorV1::ParentNotFound)[0]["exit"]["cause"]["error"]
                ["_tag"],
            "ParentNotFoundError"
        );
        assert_eq!(
            rpc_folder_error("15", LegacyFolderCrudErrorV1::RecursiveDefinition)[0]["exit"]["cause"]
                ["error"]["_tag"],
            "RecursiveDefinitionError"
        );
    }
}
