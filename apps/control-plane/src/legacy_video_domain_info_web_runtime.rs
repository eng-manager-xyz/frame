//! Exact anonymous HTTP carrier for Cap's video custom-domain lookup.

use serde_json::{Value, json};
use worker::{Env, Request, Response, Result};

use crate::legacy_video_domain_info_runtime::{
    D1LegacyVideoDomainInfoAuthorityV1, LegacyVideoDomainInfoReadErrorV1,
    LegacyVideoDomainInfoReadV1,
};

pub(crate) async fn response(request: &Request, env: &Env) -> Result<Response> {
    let Some(video_id) = first_video_id(request)? else {
        return exact_json(400, json!({"error": "Video ID is required"}));
    };
    let database = env.d1("DB")?;
    match D1LegacyVideoDomainInfoAuthorityV1::new(&database)
        .read(&video_id)
        .await
    {
        Ok(LegacyVideoDomainInfoReadV1::Found(projection)) => exact_json(
            200,
            json!({
                "customDomain": projection.custom_domain,
                "domainVerified": projection.domain_verified_iso
                    .map_or(Value::Bool(false), Value::String),
            }),
        ),
        Ok(LegacyVideoDomainInfoReadV1::VideoNotFound)
        | Err(LegacyVideoDomainInfoReadErrorV1::InvalidVideoId) => {
            exact_json(404, json!({"error": "Video not found"}))
        }
        Ok(LegacyVideoDomainInfoReadV1::InvalidVideoData) => {
            exact_json(500, json!({"error": "Invalid video data"}))
        }
        Err(
            LegacyVideoDomainInfoReadErrorV1::Unavailable
            | LegacyVideoDomainInfoReadErrorV1::Corrupt,
        ) => exact_json(500, json!({"error": "Internal server error"})),
    }
}

fn first_video_id(request: &Request) -> Result<Option<String>> {
    Ok(request
        .url()?
        .query_pairs()
        .find_map(|(key, value)| (key == "videoId").then(|| value.into_owned()))
        .filter(|value| !value.is_empty()))
}

fn exact_json(status: u16, body: Value) -> Result<Response> {
    let mut response = Response::from_json(&body)?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "private, no-store")?;
    response
        .headers_mut()
        .set("x-content-type-options", "nosniff")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_shapes_keep_timestamp_or_false_union() {
        assert_eq!(
            json!({
                "customDomain": "example.com",
                "domainVerified": Some("2026-07-17T19:00:00.000Z")
                    .map_or(Value::Bool(false), |value| Value::String(value.into())),
            }),
            json!({
                "customDomain": "example.com",
                "domainVerified": "2026-07-17T19:00:00.000Z"
            })
        );
        assert_eq!(
            None::<String>.map_or(Value::Bool(false), Value::String),
            Value::Bool(false)
        );
    }
}
