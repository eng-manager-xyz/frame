//! Exact HTTP carriers for Cap's extension instant-recording lifecycle.

use frame_application::{
    LEGACY_EXTENSION_INSTANT_MAX_BODY_BYTES, LegacyExtensionInstantCreateInputV1,
    LegacyExtensionInstantProgressInputV1, legacy_extension_instant_valid_wire_id,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use worker::{Env, Request, Response, Result};

use crate::{
    RuntimeConfig, direct_upload_signer,
    legacy_extension_auth_web_runtime::{LegacyExtensionHttpFailureV1, required_actor},
    legacy_extension_instant_recordings_runtime::{
        D1LegacyExtensionInstantRecordingsV1, LegacyExtensionInstantFailureV1, delete_r2_prefix,
    },
};

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

pub(crate) async fn create_response(
    request: &mut Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<Response> {
    let actor = match required_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return auth_failure(failure),
    };
    let input = match decode_json::<LegacyExtensionInstantCreateInputV1>(request).await? {
        Ok(input) if input.valid() => input,
        _ => return failure_response(LegacyExtensionInstantFailureV1::Invalid),
    };
    let Some(signer) = direct_upload_signer(env) else {
        return failure_response(LegacyExtensionInstantFailureV1::Unavailable);
    };
    let database = env.d1("DB")?;
    let web_origin = request.url()?;
    match D1LegacyExtensionInstantRecordingsV1::new(&database)
        .create(
            &actor.id,
            &input,
            &web_origin,
            config.videos_default_public,
            &signer,
            now_ms,
            None,
        )
        .await
    {
        Ok(success) => exact_json(200, serde_json::to_value(success)?),
        Err(failure) => failure_response(failure),
    }
}

pub(crate) async fn progress_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match required_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return auth_failure(failure),
    };
    let input = match decode_json::<LegacyExtensionInstantProgressInputV1>(request).await? {
        Ok(input) if input.valid() => input,
        _ => return failure_response(LegacyExtensionInstantFailureV1::Invalid),
    };
    let parsed = js_sys::Date::parse(&input.updated_at);
    if !parsed.is_finite() || !(0.0..=MAX_SAFE_INTEGER).contains(&parsed) || parsed.fract() != 0.0 {
        return failure_response(LegacyExtensionInstantFailureV1::Invalid);
    }
    #[allow(clippy::cast_possible_truncation)]
    let source_updated_at_ms = parsed as i64;
    let database = env.d1("DB")?;
    match D1LegacyExtensionInstantRecordingsV1::new(&database)
        .progress(&actor.id, &input, source_updated_at_ms, now_ms)
        .await
    {
        Ok(()) => exact_json(200, json!({"success": true})),
        Err(failure) => failure_response(failure),
    }
}

pub(crate) async fn delete_response(
    request: &mut Request,
    env: &Env,
    legacy_video_id: &str,
    now_ms: i64,
) -> Result<Response> {
    let actor = match required_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return auth_failure(failure),
    };
    if !legacy_extension_instant_valid_wire_id(legacy_video_id) || !body_is_empty(request).await? {
        return failure_response(LegacyExtensionInstantFailureV1::Invalid);
    }
    let database = env.d1("DB")?;
    let repository = D1LegacyExtensionInstantRecordingsV1::new(&database);
    let plan = match repository
        .begin_delete(&actor.id, legacy_video_id, now_ms)
        .await
    {
        Ok(plan) => plan,
        Err(failure) => return failure_response(failure),
    };
    let bucket = env.bucket("RECORDINGS")?;
    if let Err(failure) = delete_r2_prefix(&bucket, plan.storage_prefix()).await {
        return failure_response(failure);
    }
    match repository.finalize_delete(&plan, now_ms).await {
        Ok(()) => exact_json(200, json!({"success": true})),
        Err(failure) => failure_response(failure),
    }
}

async fn decode_json<T: DeserializeOwned>(
    request: &mut Request,
) -> Result<std::result::Result<T, LegacyExtensionInstantFailureV1>> {
    let Some(content_type) = request.headers().get("content-type")? else {
        return Ok(Err(LegacyExtensionInstantFailureV1::Invalid));
    };
    if content_type.split(';').next().map(str::trim) != Some("application/json")
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyExtensionInstantFailureV1::Invalid));
    }
    let declared = request
        .headers()
        .get("content-length")?
        .map(|value| value.parse::<usize>())
        .transpose()
        .ok()
        .flatten();
    if declared.is_some_and(|value| value == 0 || value > LEGACY_EXTENSION_INSTANT_MAX_BODY_BYTES) {
        return Ok(Err(LegacyExtensionInstantFailureV1::Invalid));
    }
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_EXTENSION_INSTANT_MAX_BODY_BYTES)
            .await
        {
            Ok(bytes) => bytes,
            Err(()) => return Ok(Err(LegacyExtensionInstantFailureV1::Invalid)),
        };
    if bytes.is_empty() || declared.is_some_and(|value| value != bytes.len()) {
        return Ok(Err(LegacyExtensionInstantFailureV1::Invalid));
    }
    Ok(serde_json::from_slice(&bytes).map_err(|_| LegacyExtensionInstantFailureV1::Invalid))
}

async fn body_is_empty(request: &mut Request) -> Result<bool> {
    if request
        .headers()
        .get("content-length")?
        .is_some_and(|value| value != "0")
    {
        return Ok(false);
    }
    Ok(crate::read_bounded_legacy_body(request, 0).await.is_ok())
}

fn auth_failure(failure: LegacyExtensionHttpFailureV1) -> Result<Response> {
    match failure {
        LegacyExtensionHttpFailureV1::Unauthorized | LegacyExtensionHttpFailureV1::BadRequest => {
            exact_json(401, json!({"_tag": "Unauthorized"}))
        }
        LegacyExtensionHttpFailureV1::Internal => {
            failure_response(LegacyExtensionInstantFailureV1::Unavailable)
        }
    }
}

fn failure_response(failure: LegacyExtensionInstantFailureV1) -> Result<Response> {
    let (status, tag) = failure_projection(failure);
    exact_json(status, json!({"_tag": tag}))
}

const fn failure_projection(failure: LegacyExtensionInstantFailureV1) -> (u16, &'static str) {
    match failure {
        LegacyExtensionInstantFailureV1::Invalid => (400, "BadRequest"),
        LegacyExtensionInstantFailureV1::Forbidden => (403, "PolicyDenied"),
        LegacyExtensionInstantFailureV1::NotFound => (404, "VideoNotFoundError"),
        LegacyExtensionInstantFailureV1::Corrupt | LegacyExtensionInstantFailureV1::Unavailable => {
            (500, "InternalServerError")
        }
    }
}

fn exact_json(status: u16, value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_tagged_failure_projection_is_complete() {
        for (failure, status, tag) in [
            (LegacyExtensionInstantFailureV1::Invalid, 400, "BadRequest"),
            (
                LegacyExtensionInstantFailureV1::Forbidden,
                403,
                "PolicyDenied",
            ),
            (
                LegacyExtensionInstantFailureV1::NotFound,
                404,
                "VideoNotFoundError",
            ),
            (
                LegacyExtensionInstantFailureV1::Corrupt,
                500,
                "InternalServerError",
            ),
            (
                LegacyExtensionInstantFailureV1::Unavailable,
                500,
                "InternalServerError",
            ),
        ] {
            assert_eq!(failure_projection(failure), (status, tag));
        }
    }
}
