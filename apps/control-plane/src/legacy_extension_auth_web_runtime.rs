//! Exact HTTP carriers for Cap's four provider-free extension auth routes.

use frame_application::{
    LEGACY_EXTENSION_AUTH_MAX_FORM_BYTES, LegacyExtensionAuthParamsV1,
    legacy_extension_approved_url, legacy_extension_bearer_segment, legacy_extension_consent_url,
    legacy_extension_header_selects_api_key, legacy_extension_login_url,
    render_legacy_extension_consent_page, validate_legacy_extension_redirect_uri,
};
use serde_json::json;
use url::Url;
use worker::{Env, Request, Response, Result};

use crate::{
    RuntimeConfig,
    browser_web_runtime::{self, BrowserWebFailure},
    legacy_extension_auth_runtime::{
        D1LegacyExtensionAuthV1, LegacyExtensionActorV1, LegacyExtensionRuntimeFailureV1,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyExtensionHttpFailureV1 {
    BadRequest,
    Unauthorized,
    Internal,
}

type HttpOutcome<T> = std::result::Result<T, LegacyExtensionHttpFailureV1>;

pub(crate) async fn start_response(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<Response> {
    match start(request, env, config, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(failure)) => failure_response(failure),
        Err(_) => failure_response(LegacyExtensionHttpFailureV1::Internal),
    }
}

pub(crate) async fn approve_response(
    request: &mut Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<Response> {
    match approve(request, env, config, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(failure)) => failure_response(failure),
        Err(_) => failure_response(LegacyExtensionHttpFailureV1::Internal),
    }
}

pub(crate) async fn revoke_response(request: &Request, env: &Env, now_ms: i64) -> Result<Response> {
    match revoke(request, env, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(failure)) => failure_response(failure),
        Err(_) => failure_response(LegacyExtensionHttpFailureV1::Internal),
    }
}

pub(crate) async fn bootstrap_response(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<Response> {
    match bootstrap(request, env, config, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(failure)) => failure_response(failure),
        Err(_) => failure_response(LegacyExtensionHttpFailureV1::Internal),
    }
}

async fn start(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    let current_url = request.url()?;
    let params = match decode_url_params(&current_url) {
        Ok(params) => params,
        Err(failure) => return Ok(Err(failure)),
    };
    let redirect_uri = match validate_redirect(&params.redirect_uri, config) {
        Ok(url) => url,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor = match optional_session_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return Ok(Err(failure)),
    };
    let Some(actor) = actor else {
        return Ok(login_restart(&current_url, &params, config));
    };
    let html =
        render_legacy_extension_consent_page(&actor.email, &redirect_uri, params.state.as_deref());
    let mut response = Response::from_bytes(html.into_bytes())?.with_status(200);
    let headers = response.headers_mut();
    headers.set("content-type", "text/html; charset=utf-8")?;
    headers.set("cache-control", "no-store")?;
    headers.set("pragma", "no-cache")?;
    headers.set("referrer-policy", "no-referrer")?;
    headers.set("x-content-type-options", "nosniff")?;
    headers.set(
        "content-security-policy",
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
    )?;
    Ok(Ok(response))
}

async fn approve(
    request: &mut Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    let current_url = request.url()?;
    if let Some(sec_fetch_site) = request.headers().get("sec-fetch-site")? {
        if sec_fetch_site != "same-origin" {
            return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
        }
    } else if request.headers().get("origin")?.as_deref()
        != Some(
            web_origin(config, &current_url)?
                .origin()
                .ascii_serialization()
                .as_str(),
        )
    {
        return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
    }
    let params = match decode_form(request).await? {
        Ok(params) => params,
        Err(failure) => return Ok(Err(failure)),
    };
    let redirect_uri = match validate_redirect(&params.redirect_uri, config) {
        Ok(url) => url,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor = match optional_session_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return Ok(Err(failure)),
    };
    let Some(actor) = actor else {
        return Ok(login_restart(&current_url, &params, config));
    };
    let database = env.d1("DB")?;
    let auth_api_key = match D1LegacyExtensionAuthV1::new(&database)
        .mint_auth_key(&actor.id, now_ms)
        .await?
    {
        Ok(key) => key,
        Err(LegacyExtensionRuntimeFailureV1::RateLimited) => {
            return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
        }
        Err(_) => return Ok(Err(LegacyExtensionHttpFailureV1::Internal)),
    };
    let approved = legacy_extension_approved_url(
        &redirect_uri,
        &auth_api_key,
        &actor.id,
        params.state.as_deref(),
    );
    Ok(redirect_response(&approved))
}

async fn revoke(request: &Request, env: &Env, now_ms: i64) -> Result<HttpOutcome<Response>> {
    let actor = match required_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return Ok(Err(failure)),
    };
    let authorization = request.headers().get("authorization")?;
    let Some(auth_api_key) = legacy_extension_bearer_segment(authorization.as_deref()) else {
        return json_response(&json!({ "success": false })).map(Ok);
    };
    let database = env.d1("DB")?;
    if D1LegacyExtensionAuthV1::new(&database)
        .revoke_owned_key(&actor.id, auth_api_key)
        .await?
        .is_err()
    {
        return Ok(Err(LegacyExtensionHttpFailureV1::Internal));
    }
    json_response(&json!({ "success": true })).map(Ok)
}

async fn bootstrap(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    let actor = match required_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return Ok(Err(failure)),
    };
    let database = env.d1("DB")?;
    let result = match D1LegacyExtensionAuthV1::new(&database)
        .bootstrap(&actor, now_ms, config.cap_hosted)
        .await?
    {
        Ok(result) => result,
        Err(_) => return Ok(Err(LegacyExtensionHttpFailureV1::Internal)),
    };
    json_response(&result).map(Ok)
}

pub(crate) async fn optional_session_actor(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<HttpOutcome<Option<LegacyExtensionActorV1>>> {
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unavailable) => {
                return Ok(Err(LegacyExtensionHttpFailureV1::Internal));
            }
            Err(_) => return Ok(Ok(None)),
        };
    let database = env.d1("DB")?;
    Ok(
        match D1LegacyExtensionAuthV1::new(&database)
            .session_actor(&actor_id)
            .await?
        {
            Ok(actor) => Ok(actor),
            Err(_) => Err(LegacyExtensionHttpFailureV1::Internal),
        },
    )
}

pub(crate) async fn required_actor(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<HttpOutcome<LegacyExtensionActorV1>> {
    let authorization = request.headers().get("authorization")?;
    if legacy_extension_header_selects_api_key(authorization.as_deref()) {
        let auth_api_key = legacy_extension_bearer_segment(authorization.as_deref())
            .ok_or_else(|| worker::Error::RustError("extension auth parser drifted".into()))?;
        let database = env.d1("DB")?;
        return Ok(
            match D1LegacyExtensionAuthV1::new(&database)
                .api_key_actor(auth_api_key, now_ms)
                .await?
            {
                Ok(Some(actor)) => Ok(actor),
                Ok(None) => Err(LegacyExtensionHttpFailureV1::Unauthorized),
                Err(_) => Err(LegacyExtensionHttpFailureV1::Internal),
            },
        );
    }
    Ok(match optional_session_actor(request, env, now_ms).await? {
        Ok(Some(actor)) => Ok(actor),
        Ok(None) => Err(LegacyExtensionHttpFailureV1::Unauthorized),
        Err(failure) => Err(failure),
    })
}

fn decode_url_params(url: &Url) -> HttpOutcome<LegacyExtensionAuthParamsV1> {
    decode_pairs(
        url.query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned())),
    )
}

async fn decode_form(request: &mut Request) -> Result<HttpOutcome<LegacyExtensionAuthParamsV1>> {
    let Some(content_type) = request.headers().get("content-type")? else {
        return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
    };
    if content_type.split(';').next().map(str::trim) != Some("application/x-www-form-urlencoded")
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
    }
    let declared = request
        .headers()
        .get("content-length")?
        .map(|value| value.parse::<usize>())
        .transpose()
        .ok()
        .flatten();
    if declared.is_some_and(|value| value == 0 || value > LEGACY_EXTENSION_AUTH_MAX_FORM_BYTES) {
        return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
    }
    let bytes = match crate::read_bounded_legacy_body(request, LEGACY_EXTENSION_AUTH_MAX_FORM_BYTES)
        .await
    {
        Ok(bytes) => bytes,
        Err(()) => return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest)),
    };
    if bytes.is_empty() || declared.is_some_and(|value| value != bytes.len()) {
        return Ok(Err(LegacyExtensionHttpFailureV1::BadRequest));
    }
    Ok(decode_pairs(url::form_urlencoded::parse(&bytes).map(
        |(key, value)| (key.into_owned(), value.into_owned()),
    )))
}

fn decode_pairs(
    pairs: impl Iterator<Item = (String, String)>,
) -> HttpOutcome<LegacyExtensionAuthParamsV1> {
    let mut redirect_uri = None;
    let mut state = None;
    for (key, value) in pairs {
        match key.as_str() {
            "redirectUri" if redirect_uri.is_none() => redirect_uri = Some(value),
            "state" if state.is_none() => state = Some(value),
            _ => return Err(LegacyExtensionHttpFailureV1::BadRequest),
        }
    }
    let params = LegacyExtensionAuthParamsV1 {
        redirect_uri: redirect_uri.ok_or(LegacyExtensionHttpFailureV1::BadRequest)?,
        state,
    };
    params
        .valid()
        .then_some(params)
        .ok_or(LegacyExtensionHttpFailureV1::BadRequest)
}

fn validate_redirect(redirect_uri: &str, config: &RuntimeConfig) -> HttpOutcome<Url> {
    let local_development = !config.production()
        && matches!(
            config.host_policy.public_host.as_str(),
            "localhost" | "127.0.0.1"
        );
    validate_legacy_extension_redirect_uri(
        redirect_uri,
        config.chrome_extension_id.as_deref(),
        local_development,
    )
    .map_err(|_| LegacyExtensionHttpFailureV1::BadRequest)
}

fn login_restart(
    current_url: &Url,
    params: &LegacyExtensionAuthParamsV1,
    config: &RuntimeConfig,
) -> HttpOutcome<Response> {
    let consent = legacy_extension_consent_url(current_url, params)
        .map_err(|_| LegacyExtensionHttpFailureV1::BadRequest)?;
    let login = legacy_extension_login_url(
        &web_origin(config, current_url).map_err(|_| LegacyExtensionHttpFailureV1::Internal)?,
        &consent,
    )
    .map_err(|_| LegacyExtensionHttpFailureV1::Internal)?;
    redirect_response(&login)
}

fn web_origin(config: &RuntimeConfig, current_url: &Url) -> Result<Url> {
    if !config.production()
        && matches!(
            current_url.host_str(),
            Some("localhost" | "127.0.0.1" | "::1")
        )
    {
        let mut origin = current_url.clone();
        origin.set_path("/");
        origin.set_query(None);
        origin.set_fragment(None);
        return Ok(origin);
    }
    Url::parse(&crate::storage_origin(config))
        .map_err(|_| worker::Error::RustError("extension web origin is invalid".into()))
}

fn redirect_response(location: &Url) -> HttpOutcome<Response> {
    let mut response = Response::empty()
        .map_err(|_| LegacyExtensionHttpFailureV1::Internal)?
        .with_status(302);
    let headers = response.headers_mut();
    headers
        .set("location", location.as_str())
        .map_err(|_| LegacyExtensionHttpFailureV1::Internal)?;
    headers
        .set("cache-control", "no-store")
        .map_err(|_| LegacyExtensionHttpFailureV1::Internal)?;
    headers
        .set("pragma", "no-cache")
        .map_err(|_| LegacyExtensionHttpFailureV1::Internal)?;
    headers
        .set("referrer-policy", "no-referrer")
        .map_err(|_| LegacyExtensionHttpFailureV1::Internal)?;
    Ok(response)
}

fn json_response(value: &impl serde::Serialize) -> Result<Response> {
    let mut response = Response::from_json(value)?.with_status(200);
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}

fn failure_response(failure: LegacyExtensionHttpFailureV1) -> Result<Response> {
    let (status, message) = match failure {
        LegacyExtensionHttpFailureV1::BadRequest => (400, "Bad Request"),
        LegacyExtensionHttpFailureV1::Unauthorized => (401, "Unauthorized"),
        LegacyExtensionHttpFailureV1::Internal => (500, "Internal Server Error"),
    };
    let mut response = Response::error(message, status)?;
    response.headers_mut().set("cache-control", "no-store")?;
    if failure == LegacyExtensionHttpFailureV1::Unauthorized {
        response.headers_mut().set("www-authenticate", "Bearer")?;
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_form_decoders_reject_duplicates_unknowns_and_missing_redirects() {
        let valid = decode_pairs(
            [
                (
                    "redirectUri".into(),
                    "https://extension.chromiumapp.org/cb".into(),
                ),
                ("state".into(), "state".into()),
            ]
            .into_iter(),
        )
        .expect("params");
        assert_eq!(valid.state.as_deref(), Some("state"));
        for pairs in [
            vec![("state".into(), "state".into())],
            vec![("unknown".into(), "value".into())],
            vec![
                ("redirectUri".into(), "https://one.chromiumapp.org".into()),
                ("redirectUri".into(), "https://two.chromiumapp.org".into()),
            ],
        ] {
            assert_eq!(
                decode_pairs(pairs.into_iter()),
                Err(LegacyExtensionHttpFailureV1::BadRequest)
            );
        }
    }

    #[test]
    fn exact_fetch_metadata_fallback_and_carrier_markers_remain_visible() {
        let source = include_str!("legacy_extension_auth_web_runtime.rs");
        assert!(source.contains("sec_fetch_site != \"same-origin\""));
        assert!(source.contains("application/x-www-form-urlencoded"));
        assert!(source.contains("with_status(302)"));
        assert!(source.contains("LegacyExtensionRuntimeFailureV1::RateLimited"));
        assert!(source.contains("legacy_extension_header_selects_api_key"));
        assert!(source.contains("{ \"success\": false }"));
    }
}
