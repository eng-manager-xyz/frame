//! Exact HTTP carrier for Cap's released mobile upload lifecycle.

use frame_application::{
    LEGACY_MOBILE_UPLOADS_MAX_BODY_BYTES, LegacyMobileUploadCompleteInputV1,
    LegacyMobileUploadCreateInputV1, LegacyMobileUploadProgressInputV1,
    LegacyMobileUploadSuccessV1, RateLimitDecisionV1,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use worker::{Env, Request, Response, Result, send::IntoSendFuture};

use crate::{
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    direct_upload_signer,
    legacy_extension_auth_web_runtime::{LegacyExtensionHttpFailureV1, required_actor},
    legacy_mobile_uploads_runtime::{D1LegacyMobileUploadsV1, LegacyMobileUploadsFailureV1},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMobileUploadsRouteV1<'a> {
    Create,
    Complete { video_id: &'a str },
    Progress { video_id: &'a str },
}

pub(crate) async fn response(
    request: &mut Request,
    env: &Env,
    route: LegacyMobileUploadsRouteV1<'_>,
    now_ms: i64,
) -> Result<Response> {
    match handle(request, env, route, now_ms).await {
        Ok(response) => Ok(response),
        Err(failure) => failure_response(failure),
    }
}

async fn handle(
    request: &mut Request,
    env: &Env,
    route: LegacyMobileUploadsRouteV1<'_>,
    now_ms: i64,
) -> std::result::Result<Response, LegacyMobileUploadsFailureV1> {
    if request.method().to_string() != "POST" {
        return Err(LegacyMobileUploadsFailureV1::NotFound);
    }
    let database = env
        .d1("DB")
        .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
    let edge = compatibility_rate_limit::admit_edge_request(
        env,
        request,
        CompatibilityRateLimitBucketV1::UploadStorage,
        now_ms,
    )
    .await
    .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
    if matches!(edge, RateLimitDecisionV1::Rejected { .. }) {
        return exact_json(429, json!({"_tag": "TooManyRequests"}));
    }
    let actor = match required_actor(request, env, now_ms)
        .await
        .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?
    {
        Ok(actor) => actor,
        Err(failure) => return auth_failure(failure),
    };
    let principal = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::UploadStorage,
        &actor.id,
        now_ms,
    )
    .await
    .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
    if matches!(principal, RateLimitDecisionV1::Rejected { .. }) {
        return exact_json(429, json!({"_tag": "TooManyRequests"}));
    }
    let repository = D1LegacyMobileUploadsV1::new(&database);
    match route {
        LegacyMobileUploadsRouteV1::Create => {
            let input = decode_json::<LegacyMobileUploadCreateInputV1>(request).await?;
            if !input.valid() {
                return Err(LegacyMobileUploadsFailureV1::Invalid);
            }
            let signer =
                direct_upload_signer(env).ok_or(LegacyMobileUploadsFailureV1::Unavailable)?;
            let web_url = web_url(env)?;
            let default_public = videos_default_public(env)?;
            let result = repository
                .create(&actor.id, &input, &web_url, default_public, &signer, now_ms)
                .await?;
            exact_json(
                200,
                serde_json::to_value(result).map_err(|_| LegacyMobileUploadsFailureV1::Corrupt)?,
            )
        }
        LegacyMobileUploadsRouteV1::Progress { video_id } => {
            let input = decode_json::<LegacyMobileUploadProgressInputV1>(request).await?;
            repository
                .progress(&actor.id, video_id, &input, now_ms)
                .await?;
            exact_json(
                200,
                serde_json::to_value(LegacyMobileUploadSuccessV1 { success: true })
                    .map_err(|_| LegacyMobileUploadsFailureV1::Corrupt)?,
            )
        }
        LegacyMobileUploadsRouteV1::Complete { video_id } => {
            let input = decode_json::<LegacyMobileUploadCompleteInputV1>(request).await?;
            let snapshot = repository
                .completion_snapshot(&actor.id, video_id, &input)
                .await?;
            let bucket = env
                .bucket("RECORDINGS")
                .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
            let object = bucket
                .head(&snapshot.raw_file_key)
                .into_send()
                .await
                .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?
                .ok_or(LegacyMobileUploadsFailureV1::NotFound)?;
            if object.key() != snapshot.raw_file_key {
                return Err(LegacyMobileUploadsFailureV1::Corrupt);
            }
            repository
                .begin_completion(&snapshot, &input, object.size(), now_ms)
                .await?;
            exact_json(
                200,
                serde_json::to_value(LegacyMobileUploadSuccessV1 { success: true })
                    .map_err(|_| LegacyMobileUploadsFailureV1::Corrupt)?,
            )
        }
    }
}

async fn decode_json<T: DeserializeOwned>(
    request: &mut Request,
) -> std::result::Result<T, LegacyMobileUploadsFailureV1> {
    let headers = request.headers();
    if headers
        .get("idempotency-key")
        .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?
        .is_some()
        || headers
            .get("transfer-encoding")
            .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?
            .is_some()
        || headers
            .get("content-encoding")
            .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?
            .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(LegacyMobileUploadsFailureV1::Invalid);
    }
    let content_type = headers
        .get("content-type")
        .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?
        .ok_or(LegacyMobileUploadsFailureV1::Invalid)?;
    if content_type.split(';').next().map(str::trim) != Some("application/json") {
        return Err(LegacyMobileUploadsFailureV1::Invalid);
    }
    let declared = headers
        .get("content-length")
        .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| LegacyMobileUploadsFailureV1::Invalid)?;
    if declared.is_some_and(|value| value == 0 || value > LEGACY_MOBILE_UPLOADS_MAX_BODY_BYTES) {
        return Err(LegacyMobileUploadsFailureV1::Invalid);
    }
    let bytes = crate::read_bounded_legacy_body(request, LEGACY_MOBILE_UPLOADS_MAX_BODY_BYTES)
        .await
        .map_err(|()| LegacyMobileUploadsFailureV1::Invalid)?;
    if bytes.is_empty() || declared.is_some_and(|value| value != bytes.len()) {
        return Err(LegacyMobileUploadsFailureV1::Invalid);
    }
    serde_json::from_slice(&bytes).map_err(|_| LegacyMobileUploadsFailureV1::Invalid)
}

fn auth_failure(
    failure: LegacyExtensionHttpFailureV1,
) -> std::result::Result<Response, LegacyMobileUploadsFailureV1> {
    match failure {
        LegacyExtensionHttpFailureV1::BadRequest | LegacyExtensionHttpFailureV1::Unauthorized => {
            exact_json(401, json!({"_tag": "Unauthorized"}))
        }
        LegacyExtensionHttpFailureV1::Internal => Err(LegacyMobileUploadsFailureV1::Unavailable),
    }
}

fn failure_response(failure: LegacyMobileUploadsFailureV1) -> Result<Response> {
    let (status, tag) = failure_projection(failure);
    let mut response = Response::from_json(&json!({"_tag": tag}))?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

const fn failure_projection(failure: LegacyMobileUploadsFailureV1) -> (u16, &'static str) {
    match failure {
        LegacyMobileUploadsFailureV1::Invalid => (400, "BadRequest"),
        LegacyMobileUploadsFailureV1::Forbidden => (403, "Forbidden"),
        LegacyMobileUploadsFailureV1::NotFound => (404, "NotFound"),
        LegacyMobileUploadsFailureV1::Conflict => (409, "Conflict"),
        LegacyMobileUploadsFailureV1::ProviderGated => (503, "provider_execution"),
        LegacyMobileUploadsFailureV1::Corrupt | LegacyMobileUploadsFailureV1::Unavailable => {
            (500, "InternalServerError")
        }
    }
}

fn exact_json(
    status: u16,
    value: Value,
) -> std::result::Result<Response, LegacyMobileUploadsFailureV1> {
    let mut response = Response::from_json(&value)
        .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?
        .with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")
        .map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
    Ok(response)
}

fn web_url(env: &Env) -> std::result::Result<String, LegacyMobileUploadsFailureV1> {
    let value = env_value(env, "WEB_URL").ok_or(LegacyMobileUploadsFailureV1::Unavailable)?;
    let url = url::Url::parse(&value).map_err(|_| LegacyMobileUploadsFailureV1::Unavailable)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(LegacyMobileUploadsFailureV1::Unavailable);
    }
    Ok(url.origin().ascii_serialization())
}

fn videos_default_public(env: &Env) -> std::result::Result<bool, LegacyMobileUploadsFailureV1> {
    match env_value(env, "CAP_VIDEOS_DEFAULT_PUBLIC")
        .unwrap_or_else(|| "true".into())
        .as_str()
    {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(LegacyMobileUploadsFailureV1::Unavailable),
    }
}

fn env_value(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_projection_keeps_provider_intent_distinct_from_success() {
        assert_eq!(
            failure_projection(LegacyMobileUploadsFailureV1::ProviderGated),
            (503, "provider_execution")
        );
        assert_eq!(
            failure_projection(LegacyMobileUploadsFailureV1::NotFound),
            (404, "NotFound")
        );
    }
}
