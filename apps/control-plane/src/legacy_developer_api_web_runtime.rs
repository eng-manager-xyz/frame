//! Exact HTTP carrier for Cap's developer REST/SDK APIs and storage cron.

use frame_application::{
    LEGACY_DEVELOPER_API_JSON_CONTENT_TYPE, LEGACY_DEVELOPER_API_MAX_BODY_BYTES, LegacyCallerV1,
    LegacyDeveloperApiErrorV1, LegacyDeveloperApiInputV1, LegacyDeveloperApiRequestV1,
    LegacyDeveloperApiResultV1, LegacyDeveloperApiSurfaceV1, LegacyDeveloperPartV1,
    RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1,
    IdempotencyKey,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use worker::{Env, Method, Request, Response, Result};

use crate::{
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    direct_upload_signer,
    legacy_compatibility_runtime::{
        LegacyCompatibilityTransportV1, LegacyDeveloperApiInvocationV1,
    },
    legacy_developer_actions_runtime::LocalLegacyDeveloperSecretAuthorityV1,
    legacy_developer_api_runtime::D1LegacyDeveloperApiPortV1,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyDeveloperApiRouteV1 {
    pub surface: LegacyDeveloperApiSurfaceV1,
    pub video_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultipartTargetWireV1 {
    video_id: String,
    upload_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultipartInitiateWireV1 {
    video_id: String,
    content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultipartPresignWireV1 {
    video_id: String,
    upload_id: String,
    part_number: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultipartPartWireV1 {
    part_number: u16,
    etag: String,
    size: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultipartCompleteWireV1 {
    video_id: String,
    upload_id: String,
    parts: Vec<MultipartPartWireV1>,
    duration_in_secs: f64,
    width: Option<f64>,
    height: Option<f64>,
    fps: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoCreateWireV1 {
    name: Option<String>,
    user_id: Option<String>,
    metadata: Option<Value>,
}

pub(crate) async fn response(
    route: LegacyDeveloperApiRouteV1,
    request: &mut Request,
    env: &Env,
) -> Result<Response> {
    if request.method() == Method::Options && sdk_surface(route.surface) {
        return cors(Response::empty()?.with_status(204));
    }
    if request.method().to_string() != route.surface.method() {
        return source_error(405, "Method not allowed", Some(route.surface));
    }
    let now_ms = current_time_ms();
    if route.surface != LegacyDeveloperApiSurfaceV1::StorageCron
        && !matches!(
            compatibility_rate_limit::admit_edge_request(
                env,
                request,
                if matches!(
                    route.surface,
                    LegacyDeveloperApiSurfaceV1::MultipartAbort
                        | LegacyDeveloperApiSurfaceV1::MultipartComplete
                        | LegacyDeveloperApiSurfaceV1::MultipartInitiate
                        | LegacyDeveloperApiSurfaceV1::MultipartPresign
                ) {
                    CompatibilityRateLimitBucketV1::UploadStorage
                } else {
                    CompatibilityRateLimitBucketV1::DeveloperApi
                },
                now_ms,
            )
            .await?,
            RateLimitDecisionV1::Allowed
        )
    {
        let mut response = source_error(429, "Rate limit exceeded", Some(route.surface))?;
        response.headers_mut().set(
            "retry-after",
            &compatibility_rate_limit::RETRY_AFTER_SECONDS.to_string(),
        )?;
        return Ok(response);
    }

    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    let signer = direct_upload_signer(env);
    let web_origin = env_value(env, "WEB_URL")
        .or_else(|| env_value(env, "NEXT_PUBLIC_WEB_URL"))
        .unwrap_or_else(|| "https://frame.engmanager.xyz".into());
    let port = D1LegacyDeveloperApiPortV1::new(&database, &bucket, signer.as_ref(), &web_origin);
    let app_id = match route.surface {
        LegacyDeveloperApiSurfaceV1::StorageCron => {
            if !valid_cron_authorization(request, env)? {
                return if env_value(env, "CRON_SECRET").is_none() {
                    source_error(500, "Server misconfiguration", Some(route.surface))
                } else {
                    source_error(401, "Unauthorized", Some(route.surface))
                };
            }
            None
        }
        surface if sdk_surface(surface) => {
            let Some(token) = owned_bearer_token(request)? else {
                return source_error(401, "Invalid public key", Some(route.surface));
            };
            if !token.starts_with("cpk_") {
                return source_error(401, "Invalid public key", Some(route.surface));
            }
            let digest = key_digest(env, &token).ok_or_else(|| {
                worker::Error::RustError("developer key authority is unavailable".into())
            })?;
            let Some(auth) = port
                .authenticate_key(&digest, "public", now_ms)
                .await
                .map_err(|_| worker::Error::RustError("developer key lookup failed".into()))?
            else {
                return source_error(401, "Invalid or revoked public key", Some(route.surface));
            };
            if auth.environment == "production" {
                let Some(origin) = request.headers().get("origin")? else {
                    return source_error(
                        403,
                        "Origin header required for production apps",
                        Some(route.surface),
                    );
                };
                if !port
                    .origin_allowed(&auth.app_id, &origin)
                    .await
                    .map_err(|_| {
                        worker::Error::RustError("developer origin lookup failed".into())
                    })?
                {
                    return source_error(403, "Origin not allowed", Some(route.surface));
                }
            }
            Some(auth.app_id)
        }
        _ => {
            let Some(token) = owned_bearer_token(request)? else {
                return source_error(401, "Invalid secret key", Some(route.surface));
            };
            if !token.starts_with("csk_") {
                return source_error(401, "Invalid secret key", Some(route.surface));
            }
            let digest = key_digest(env, &token).ok_or_else(|| {
                worker::Error::RustError("developer key authority is unavailable".into())
            })?;
            let Some(auth) = port
                .authenticate_key(&digest, "secret", now_ms)
                .await
                .map_err(|_| worker::Error::RustError("developer key lookup failed".into()))?
            else {
                return source_error(401, "Invalid or revoked secret key", Some(route.surface));
            };
            Some(auth.app_id)
        }
    };

    let input = match decode_input(&route, request).await? {
        Ok(input) => input,
        Err(()) => return source_error(400, "Invalid request", Some(route.surface)),
    };
    let idempotency_key = if route.surface.mutation() {
        request.headers().get("idempotency-key")?
    } else {
        None
    };
    let api_request = LegacyDeveloperApiRequestV1 {
        app_id,
        input,
        idempotency_key: idempotency_key.clone(),
    };
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| worker::Error::RustError("legacy developer registry is invalid".into()))?;
    let caller = if route.surface == LegacyDeveloperApiSurfaceV1::StorageCron {
        LegacyCallerV1::Scheduler
    } else {
        LegacyCallerV1::Released(ClientReleaseV1 {
            surface: ClientSurfaceV1::Developer,
            api_major: 1,
            release: 2,
        })
    };
    let envelope_key = match idempotency_key
        .as_deref()
        .map(IdempotencyKey::parse)
        .transpose()
    {
        Ok(value) => value,
        Err(_) => return source_error(400, "Invalid request", Some(route.surface)),
    };
    match transport
        .dispatch_developer_api(
            &port,
            LegacyDeveloperApiInvocationV1 {
                caller,
                envelope: ApiMutationEnvelopeV1 {
                    content_length: u64::from(matches!(route.surface.method(), "POST")),
                    content_type: matches!(route.surface.method(), "POST")
                        .then(|| LEGACY_DEVELOPER_API_JSON_CONTENT_TYPE.to_owned()),
                    idempotency_key: envelope_key,
                    correlation_id: "legacy-developer-api".into(),
                },
                security: RequestSecurityContextV1 {
                    authenticated: true,
                    authorized: true,
                    browser_origin_valid: true,
                    csrf_valid: true,
                    rate_limit: RateLimitDecisionV1::Allowed,
                },
                request: api_request,
            },
        )
        .await
    {
        Ok(outcome) => success_response(outcome.result, route.surface),
        Err(error) => adapter_error(error, route.surface),
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

async fn decode_input(
    route: &LegacyDeveloperApiRouteV1,
    request: &mut Request,
) -> Result<std::result::Result<LegacyDeveloperApiInputV1, ()>> {
    let invalid = || Ok(Err(()));
    let input = match route.surface {
        LegacyDeveloperApiSurfaceV1::StorageCron => {
            if reject_read_carriers(request).is_err() {
                return invalid();
            }
            let iso = String::from(js_sys::Date::new_0().to_iso_string());
            LegacyDeveloperApiInputV1::StorageCron {
                snapshot_day: iso.get(..10).unwrap_or_default().to_owned(),
            }
        }
        LegacyDeveloperApiSurfaceV1::MultipartAbort => {
            let wire: MultipartTargetWireV1 = match json_body(request).await {
                Ok(value) => value,
                Err(()) => return invalid(),
            };
            LegacyDeveloperApiInputV1::MultipartAbort {
                video_id: wire.video_id,
                upload_id: wire.upload_id,
            }
        }
        LegacyDeveloperApiSurfaceV1::MultipartComplete => {
            let wire: MultipartCompleteWireV1 = match json_body(request).await {
                Ok(value) => value,
                Err(()) => return invalid(),
            };
            LegacyDeveloperApiInputV1::MultipartComplete {
                video_id: wire.video_id,
                upload_id: wire.upload_id,
                parts: wire
                    .parts
                    .into_iter()
                    .map(|part| LegacyDeveloperPartV1 {
                        part_number: part.part_number,
                        etag: part.etag,
                        size: part.size,
                    })
                    .collect(),
                duration_seconds: wire.duration_in_secs,
                width: wire.width,
                height: wire.height,
                fps: wire.fps,
            }
        }
        LegacyDeveloperApiSurfaceV1::MultipartInitiate => {
            let wire: MultipartInitiateWireV1 = match json_body(request).await {
                Ok(value) => value,
                Err(()) => return invalid(),
            };
            LegacyDeveloperApiInputV1::MultipartInitiate {
                video_id: wire.video_id,
                content_type: wire.content_type,
            }
        }
        LegacyDeveloperApiSurfaceV1::MultipartPresign => {
            let wire: MultipartPresignWireV1 = match json_body(request).await {
                Ok(value) => value,
                Err(()) => return invalid(),
            };
            LegacyDeveloperApiInputV1::MultipartPresign {
                video_id: wire.video_id,
                upload_id: wire.upload_id,
                part_number: wire.part_number,
            }
        }
        LegacyDeveloperApiSurfaceV1::VideoCreate => {
            let wire: VideoCreateWireV1 = match json_body(request).await {
                Ok(value) => value,
                Err(()) => return invalid(),
            };
            LegacyDeveloperApiInputV1::VideoCreate {
                name: wire.name,
                external_user_id: wire.user_id,
                metadata: wire.metadata,
            }
        }
        LegacyDeveloperApiSurfaceV1::Usage => {
            if reject_read_carriers(request).is_err() {
                return invalid();
            }
            LegacyDeveloperApiInputV1::Usage
        }
        LegacyDeveloperApiSurfaceV1::VideosList => {
            if reject_read_carriers(request).is_err() {
                return invalid();
            }
            let url = request.url()?;
            let mut user_id = None;
            let mut limit = None;
            let mut offset = None;
            for (name, value) in url.query_pairs() {
                match name.as_ref() {
                    "userId" if user_id.is_none() => user_id = Some(value.into_owned()),
                    "limit" if limit.is_none() => limit = Some(value.into_owned()),
                    "offset" if offset.is_none() => offset = Some(value.into_owned()),
                    _ => {}
                }
            }
            let limit = limit
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(50)
                .min(100);
            let offset = offset
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            LegacyDeveloperApiInputV1::VideosList {
                external_user_id: user_id,
                limit,
                offset,
            }
        }
        LegacyDeveloperApiSurfaceV1::VideoDelete => {
            if reject_delete_carriers(request).is_err() {
                return invalid();
            }
            LegacyDeveloperApiInputV1::VideoDelete {
                video_id: route.video_id.clone().unwrap_or_default(),
            }
        }
        LegacyDeveloperApiSurfaceV1::VideoGet => {
            if reject_read_carriers(request).is_err() {
                return invalid();
            }
            LegacyDeveloperApiInputV1::VideoGet {
                video_id: route.video_id.clone().unwrap_or_default(),
            }
        }
        LegacyDeveloperApiSurfaceV1::VideoStatus => {
            if reject_read_carriers(request).is_err() {
                return invalid();
            }
            LegacyDeveloperApiInputV1::VideoStatus {
                video_id: route.video_id.clone().unwrap_or_default(),
            }
        }
    };
    Ok(Ok(input))
}

async fn json_body<T: for<'de> Deserialize<'de>>(
    request: &mut Request,
) -> std::result::Result<T, ()> {
    let content_type = request.headers().get("content-type").map_err(|_| ())?;
    if !content_type.as_deref().is_some_and(|value| {
        value.eq_ignore_ascii_case(LEGACY_DEVELOPER_API_JSON_CONTENT_TYPE)
            || value.eq_ignore_ascii_case("application/json; charset=utf-8")
    }) || request
        .headers()
        .get("content-encoding")
        .map_err(|_| ())?
        .is_some_and(|value| value != "identity")
    {
        return Err(());
    }
    let bytes =
        crate::read_bounded_legacy_body(request, LEGACY_DEVELOPER_API_MAX_BODY_BYTES).await?;
    if bytes.is_empty() {
        return Err(());
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn reject_read_carriers(request: &Request) -> std::result::Result<(), ()> {
    if request
        .headers()
        .get("idempotency-key")
        .map_err(|_| ())?
        .is_some()
    {
        return Err(());
    }
    reject_delete_carriers(request)
}

fn reject_delete_carriers(request: &Request) -> std::result::Result<(), ()> {
    if request
        .headers()
        .get("content-length")
        .map_err(|_| ())?
        .is_some_and(|value| value.parse::<u64>().ok() != Some(0))
        || request
            .headers()
            .get("content-type")
            .map_err(|_| ())?
            .is_some()
    {
        return Err(());
    }
    Ok(())
}

fn success_response(
    result: LegacyDeveloperApiResultV1,
    surface: LegacyDeveloperApiSurfaceV1,
) -> Result<Response> {
    let value = match result {
        LegacyDeveloperApiResultV1::Cron {
            date,
            apps_processed,
        } => json!({"success": true, "date": date, "appsProcessed": apps_processed}),
        LegacyDeveloperApiResultV1::Success => json!({"success": true}),
        LegacyDeveloperApiResultV1::UploadInitiated { upload_id } => {
            json!({"uploadId": upload_id})
        }
        LegacyDeveloperApiResultV1::PartPresigned { presigned_url } => {
            json!({"presignedUrl": presigned_url})
        }
        LegacyDeveloperApiResultV1::VideoCreated {
            video_id,
            s3_key,
            share_url,
            embed_url,
        } => json!({
            "videoId": video_id,
            "s3Key": s3_key,
            "shareUrl": share_url,
            "embedUrl": embed_url,
        }),
        LegacyDeveloperApiResultV1::Usage(value) => json!({"data": value}),
        LegacyDeveloperApiResultV1::Videos(value) => json!({"data": value}),
        LegacyDeveloperApiResultV1::Video(value) => json!({"data": value}),
        LegacyDeveloperApiResultV1::VideoStatus(value) => json!({"data": value}),
    };
    let mut response = Response::from_json(&value)?;
    response.headers_mut().set("cache-control", "no-store")?;
    if sdk_surface(surface) {
        cors(response)
    } else {
        Ok(response)
    }
}

fn adapter_error(
    error: LegacyDeveloperApiErrorV1,
    surface: LegacyDeveloperApiSurfaceV1,
) -> Result<Response> {
    let (status, message) = match error {
        LegacyDeveloperApiErrorV1::InvalidInput => (400, "Invalid request"),
        LegacyDeveloperApiErrorV1::InvalidPublicKey => (401, "Invalid public key"),
        LegacyDeveloperApiErrorV1::RevokedPublicKey => (401, "Invalid or revoked public key"),
        LegacyDeveloperApiErrorV1::InvalidSecretKey => (401, "Invalid secret key"),
        LegacyDeveloperApiErrorV1::RevokedSecretKey => (401, "Invalid or revoked secret key"),
        LegacyDeveloperApiErrorV1::OriginRequired => {
            (403, "Origin header required for production apps")
        }
        LegacyDeveloperApiErrorV1::OriginNotAllowed => (403, "Origin not allowed"),
        LegacyDeveloperApiErrorV1::CronUnauthorized => (401, "Unauthorized"),
        LegacyDeveloperApiErrorV1::Misconfigured => (500, "Server misconfiguration"),
        LegacyDeveloperApiErrorV1::VideoNotFound => (404, "Video not found"),
        LegacyDeveloperApiErrorV1::NoStorageKey => (400, "Video has no S3 key"),
        LegacyDeveloperApiErrorV1::InsufficientCredits => (402, "Insufficient credits"),
        LegacyDeveloperApiErrorV1::CreditAccountMissing => (402, "Credit account not found"),
        LegacyDeveloperApiErrorV1::Conflict => (409, "Idempotency conflict"),
        LegacyDeveloperApiErrorV1::Provider => (500, provider_failure(surface)),
        LegacyDeveloperApiErrorV1::Unavailable => (503, "Service unavailable"),
        LegacyDeveloperApiErrorV1::Internal => (500, "Internal server error"),
    };
    source_error(status, message, Some(surface))
}

fn provider_failure(surface: LegacyDeveloperApiSurfaceV1) -> &'static str {
    match surface {
        LegacyDeveloperApiSurfaceV1::MultipartInitiate => "Failed to initiate upload",
        LegacyDeveloperApiSurfaceV1::MultipartPresign => "Failed to create presigned URL",
        LegacyDeveloperApiSurfaceV1::MultipartComplete => "Failed to complete upload",
        LegacyDeveloperApiSurfaceV1::MultipartAbort => "Failed to abort upload",
        _ => "Internal server error",
    }
}

fn source_error(
    status: u16,
    message: &str,
    surface: Option<LegacyDeveloperApiSurfaceV1>,
) -> Result<Response> {
    let mut response = Response::from_json(&json!({"error": message}))?.with_status(status);
    response.headers_mut().set("cache-control", "no-store")?;
    if surface.is_some_and(sdk_surface) {
        cors(response)
    } else {
        Ok(response)
    }
}

fn cors(mut response: Response) -> Result<Response> {
    response
        .headers_mut()
        .set("access-control-allow-origin", "*")?;
    response
        .headers_mut()
        .set("access-control-allow-methods", "GET, POST, OPTIONS")?;
    response.headers_mut().set(
        "access-control-allow-headers",
        "Content-Type, Authorization",
    )?;
    Ok(response)
}

fn sdk_surface(surface: LegacyDeveloperApiSurfaceV1) -> bool {
    matches!(
        surface,
        LegacyDeveloperApiSurfaceV1::MultipartAbort
            | LegacyDeveloperApiSurfaceV1::MultipartComplete
            | LegacyDeveloperApiSurfaceV1::MultipartInitiate
            | LegacyDeveloperApiSurfaceV1::MultipartPresign
            | LegacyDeveloperApiSurfaceV1::VideoCreate
    )
}

fn owned_bearer_token(request: &Request) -> Result<Option<String>> {
    Ok(request
        .headers()
        .get("authorization")?
        .and_then(|value| value.split(' ').nth(1).map(str::to_owned)))
}

fn key_digest(env: &Env, token: &str) -> Option<String> {
    let mut secret = env_value(env, "FRAME_LEGACY_DEVELOPER_SECRET_HEX_V1")?;
    let authority = LocalLegacyDeveloperSecretAuthorityV1::from_hex(&secret).ok()?;
    use zeroize::Zeroize;
    secret.zeroize();
    Some(authority.key_digest_for_auth(token))
}

fn valid_cron_authorization(request: &Request, env: &Env) -> Result<bool> {
    let Some(expected) = env_value(env, "CRON_SECRET") else {
        return Ok(false);
    };
    let Some(actual) = request.headers().get("authorization")? else {
        return Ok(false);
    };
    let expected = format!("Bearer {expected}");
    let key = b"frame.developer-cron.compare.v1";
    let mut expected_mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    expected_mac.update(expected.as_bytes());
    let expected_tag = expected_mac.finalize().into_bytes();
    let mut actual_mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    actual_mac.update(actual.as_bytes());
    Ok(actual.len() == expected.len() && actual_mac.verify_slice(&expected_tag).is_ok())
}

fn env_value(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
        .filter(|value| !value.is_empty())
}

fn current_time_ms() -> i64 {
    js_sys::Date::now().clamp(0.0, 9_007_199_254_740_991.0) as i64
}
