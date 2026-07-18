//! Exact HTTP carriers for Cap's four mobile session routes.

use frame_application::RateLimitDecisionV1;
use frame_application::{
    LEGACY_MOBILE_EMAIL_CODE_TTL_MS, LEGACY_MOBILE_SESSION_MAX_BODY_BYTES, LegacyMobileApiKeyV1,
    LegacyMobileEmailRequestV1, LegacyMobileEmailVerifyV1, LegacyMobileProviderV1,
    LegacyMobileSessionErrorV1, legacy_mobile_authenticated_redirect, legacy_mobile_bearer_token,
    legacy_mobile_email_code_digest, legacy_mobile_email_code_is_valid,
    legacy_mobile_email_identifier_digest, legacy_mobile_email_is_valid,
    legacy_mobile_login_redirect, legacy_mobile_middleware_api_key,
    legacy_mobile_signup_domain_allowed, legacy_mobile_trim, normalize_legacy_mobile_email,
};
use frame_domain::TimestampMillis;
use serde_json::{Value, json};
use url::Url;
use worker::{Env, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_mobile_session_runtime::{
        D1LegacyMobileSessionV1, LegacyMobileEmailVerifyOutcomeV1,
        LegacyMobileSessionRuntimeFailureV1,
    },
    worker_auth_runtime::WorkerDeliverySealer,
};

type HttpOutcome<T> = std::result::Result<T, LegacyMobileSessionErrorV1>;

pub(crate) async fn email_request_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    match email_request(request, env, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(LegacyMobileSessionErrorV1::Internal),
    }
}

pub(crate) async fn email_verify_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    match email_verify(request, env, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(LegacyMobileSessionErrorV1::Internal),
    }
}

pub(crate) async fn session_request_response(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    match session_request(request, env, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(LegacyMobileSessionErrorV1::Internal),
    }
}

pub(crate) async fn session_revoke_response(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    match session_revoke(request, env, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => error_response(error),
        Err(_) => error_response(LegacyMobileSessionErrorV1::Internal),
    }
}

async fn email_request(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    if !edge_admitted(request, env, now_ms).await? {
        return Ok(Err(LegacyMobileSessionErrorV1::Internal));
    }
    let payload = match decode_json::<LegacyMobileEmailRequestV1>(request).await? {
        Ok(payload) => payload,
        Err(error) => return Ok(Err(error)),
    };
    let email = normalize_legacy_mobile_email(&payload.email);
    if !legacy_mobile_email_is_valid(&email) {
        return Ok(Err(LegacyMobileSessionErrorV1::BadRequest));
    }
    let database = env.d1("DB")?;
    let authority = D1LegacyMobileSessionV1::new(&database);
    if let Err(error) = ensure_signup_allowed(&authority, env, &email).await {
        return Ok(Err(error));
    }
    let code = match random_mobile_code() {
        Ok(code) => code,
        Err(error) => return Ok(Err(error)),
    };
    let secret = match nextauth_secret(env) {
        Some(secret) => secret,
        None => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
    };
    let identifier_digest = legacy_mobile_email_identifier_digest(&email);
    let token_digest = legacy_mobile_email_code_digest(&code, &secret);
    let now = match TimestampMillis::new(now_ms) {
        Ok(now) => now,
        Err(_) => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
    };
    let expires_at = match now_ms
        .checked_add(LEGACY_MOBILE_EMAIL_CODE_TTL_MS)
        .and_then(|value| TimestampMillis::new(value).ok())
    {
        Some(expires_at) => expires_at,
        None => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
    };
    let envelope = match WorkerDeliverySealer::from_env(env).and_then(|sealer| {
        sealer
            .seal_mobile_email(&email, &code, expires_at, now)
            .map_err(|_| BrowserWebFailure::Unavailable)
    }) {
        Ok(envelope) => envelope,
        Err(_) => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
    };
    if let Err(error) = authority
        .request_email(&identifier_digest, &token_digest, &envelope, now_ms)
        .await
    {
        return Ok(Err(map_runtime_failure(error)));
    }
    // Cap stores/replaces the token before awaiting Resend. Frame therefore
    // also commits the encrypted handoff on provider failure, but never claims
    // success until provider execution is genuinely available. The local-only
    // acknowledgement is deterministic test plumbing, not a production flag.
    if !local_provider_test_ack(env) {
        return Ok(Err(LegacyMobileSessionErrorV1::Internal));
    }
    json_response(200, &json!({"success": true})).map(Ok)
}

async fn email_verify(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    if !edge_admitted(request, env, now_ms).await? {
        return Ok(Err(LegacyMobileSessionErrorV1::Internal));
    }
    let payload = match decode_json::<LegacyMobileEmailVerifyV1>(request).await? {
        Ok(payload) => payload,
        Err(error) => return Ok(Err(error)),
    };
    let email = normalize_legacy_mobile_email(&payload.email);
    let code = legacy_mobile_trim(&payload.code).to_owned();
    if !legacy_mobile_email_is_valid(&email) || !legacy_mobile_email_code_is_valid(&code) {
        return Ok(Err(LegacyMobileSessionErrorV1::BadRequest));
    }
    let database = env.d1("DB")?;
    let authority = D1LegacyMobileSessionV1::new(&database);
    if let Err(error) = ensure_signup_allowed(&authority, env, &email).await {
        return Ok(Err(error));
    }
    let secret = match nextauth_secret(env) {
        Some(secret) => secret,
        None => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
    };
    let identifier_digest = legacy_mobile_email_identifier_digest(&email);
    let token_digest = legacy_mobile_email_code_digest(&code, &secret);
    if let Err(error) = authority
        .consume_email_challenge(&identifier_digest, &token_digest, now_ms)
        .await
    {
        return Ok(Err(map_runtime_failure(error)));
    }
    let outcome = match authority
        .verify_email_user(&email, &identifier_digest, stripe_available(env), now_ms)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => return Ok(Err(map_runtime_failure(error))),
    };
    match outcome {
        LegacyMobileEmailVerifyOutcomeV1::ApiKey { api_key, actor } => json_response(
            200,
            &serde_json::to_value(LegacyMobileApiKeyV1::new(api_key, actor.legacy_user_id))?,
        )
        .map(Ok),
        // createUser has already committed local user/org state, exactly as
        // Cap does before Stripe calls. No key is minted and HTTP fails closed
        // until the queued Stripe effect has real execution/receipt plumbing.
        LegacyMobileEmailVerifyOutcomeV1::StripeEffectPending { .. } => {
            Ok(Err(LegacyMobileSessionErrorV1::Internal))
        }
    }
}

async fn session_request(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    let request_url = request.url()?;
    let query = match decode_session_query(&request_url) {
        Ok(query) => query,
        Err(error) => return Ok(Err(error)),
    };
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => Some(actor_id),
            Err(BrowserWebFailure::Unauthenticated | BrowserWebFailure::Invalid) => None,
            Err(_) => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
        };
    let database = env.d1("DB")?;
    let authority = D1LegacyMobileSessionV1::new(&database);
    let Some(actor_id) = actor_id else {
        if !edge_admitted(request, env, now_ms).await? {
            return Ok(Err(LegacyMobileSessionErrorV1::Internal));
        }
        let origin = match deployment_origin(env) {
            Some(origin) => origin,
            None => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
        };
        let redirect = match legacy_mobile_login_redirect(
            &origin,
            &request_url,
            query.provider,
            query.organization_id.as_deref(),
        ) {
            Some(redirect) => redirect,
            None => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
        };
        return Response::redirect_with_status(redirect, 302).map(Ok);
    };
    if !principal_admitted(env, &database, &actor_id, now_ms).await? {
        return Ok(Err(LegacyMobileSessionErrorV1::Internal));
    }
    let actor = match authority.session_actor(&actor_id).await {
        Ok(Some(actor)) => actor,
        Ok(None) => return Ok(Err(LegacyMobileSessionErrorV1::Internal)),
        Err(error) => return Ok(Err(map_runtime_failure(error))),
    };
    let api_key = match authority.request_session_key(&actor, now_ms).await {
        Ok(key) => key,
        Err(error) => return Ok(Err(map_runtime_failure(error))),
    };
    if let Some(redirect_uri) = query.redirect_uri {
        let redirect = match legacy_mobile_authenticated_redirect(
            &redirect_uri,
            &api_key,
            &actor.legacy_user_id,
        ) {
            Some(redirect) => redirect,
            None => return Ok(Err(LegacyMobileSessionErrorV1::BadRequest)),
        };
        return Response::redirect_with_status(redirect, 302).map(Ok);
    }
    json_response(
        200,
        &serde_json::to_value(LegacyMobileApiKeyV1::new(api_key, actor.legacy_user_id))?,
    )
    .map(Ok)
}

async fn session_revoke(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    let authorization = request.headers().get("authorization")?;
    let database = env.d1("DB")?;
    let authority = D1LegacyMobileSessionV1::new(&database);
    let actor = if let Some(api_key) = legacy_mobile_middleware_api_key(authorization.as_deref()) {
        match authority.api_key_actor(api_key, now_ms).await {
            Ok(Some(actor)) => actor,
            Ok(None) => return Ok(Err(LegacyMobileSessionErrorV1::Unauthorized)),
            Err(error) => return Ok(Err(map_runtime_failure(error))),
        }
    } else {
        let actor_id =
            match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
                .await?
            {
                Ok(actor_id) => actor_id,
                Err(BrowserWebFailure::Unavailable) => {
                    return Ok(Err(LegacyMobileSessionErrorV1::Internal));
                }
                Err(_) => return Ok(Err(LegacyMobileSessionErrorV1::Unauthorized)),
            };
        match authority.session_actor(&actor_id).await {
            Ok(Some(actor)) => actor,
            Ok(None) => return Ok(Err(LegacyMobileSessionErrorV1::Unauthorized)),
            Err(error) => return Ok(Err(map_runtime_failure(error))),
        }
    };
    if !principal_admitted(env, &database, &actor.mapped_user_id, now_ms).await? {
        return Ok(Err(LegacyMobileSessionErrorV1::Internal));
    }
    let Some(bearer) = legacy_mobile_bearer_token(authorization.as_deref()) else {
        return Ok(Err(LegacyMobileSessionErrorV1::Unauthorized));
    };
    if let Err(error) = authority.revoke_session_key(&actor, bearer, now_ms).await {
        return Ok(Err(map_runtime_failure(error)));
    }
    json_response(200, &json!({"success": true})).map(Ok)
}

async fn ensure_signup_allowed(
    authority: &D1LegacyMobileSessionV1<'_>,
    env: &Env,
    email: &str,
) -> HttpOutcome<()> {
    let configured = env_value(env, "CAP_ALLOWED_SIGNUP_DOMAINS");
    if configured.as_deref().is_none_or(|value| value.is_empty()) {
        return Ok(());
    }
    match authority.email_user_exists(email).await {
        Ok(true) => Ok(()),
        Ok(false) if legacy_mobile_signup_domain_allowed(email, configured.as_deref()) => Ok(()),
        Ok(false) => Err(LegacyMobileSessionErrorV1::Forbidden),
        Err(error) => Err(map_runtime_failure(error)),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SessionQueryV1 {
    redirect_uri: Option<String>,
    provider: Option<LegacyMobileProviderV1>,
    organization_id: Option<String>,
}

fn decode_session_query(url: &Url) -> HttpOutcome<SessionQueryV1> {
    let mut result = SessionQueryV1::default();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "redirectUri" => {
                if !frame_application::is_legacy_mobile_auth_redirect_uri(&value) {
                    return Err(LegacyMobileSessionErrorV1::BadRequest);
                }
                result.redirect_uri = Some(value.into_owned());
            }
            "provider" => {
                result.provider = Some(
                    LegacyMobileProviderV1::parse(&value)
                        .ok_or(LegacyMobileSessionErrorV1::BadRequest)?,
                );
            }
            "organizationId" => result.organization_id = Some(value.into_owned()),
            _ => {}
        }
    }
    Ok(result)
}

async fn decode_json<T: for<'de> serde::Deserialize<'de>>(
    request: &mut Request,
) -> Result<HttpOutcome<T>> {
    let content_type = request
        .headers()
        .get("content-type")?
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if content_type != "application/json"
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyMobileSessionErrorV1::BadRequest));
    }
    let bytes = request.bytes().await?;
    if bytes.len() > LEGACY_MOBILE_SESSION_MAX_BODY_BYTES {
        return Ok(Err(LegacyMobileSessionErrorV1::BadRequest));
    }
    Ok(serde_json::from_slice(&bytes).map_err(|_| LegacyMobileSessionErrorV1::BadRequest))
}

async fn edge_admitted(request: &Request, env: &Env, now_ms: i64) -> Result<bool> {
    Ok(matches!(
        compatibility_rate_limit::admit_edge_request(
            env,
            request,
            CompatibilityRateLimitBucketV1::AuthSession,
            now_ms,
        )
        .await?,
        RateLimitDecisionV1::Allowed
    ))
}

async fn principal_admitted(
    env: &Env,
    database: &worker::D1Database,
    actor_id: &str,
    now_ms: i64,
) -> Result<bool> {
    Ok(matches!(
        compatibility_rate_limit::admit_principal(
            env,
            database,
            CompatibilityRateLimitBucketV1::AuthSession,
            actor_id,
            now_ms,
        )
        .await?,
        RateLimitDecisionV1::Allowed
    ))
}

fn random_mobile_code() -> HttpOutcome<String> {
    const SPACE: u32 = 900_000;
    const LIMIT: u32 = u32::MAX - (u32::MAX % SPACE);
    loop {
        let mut bytes = [0_u8; 4];
        getrandom::fill(&mut bytes).map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
        let candidate = u32::from_be_bytes(bytes);
        if candidate < LIMIT {
            return Ok((100_000 + candidate % SPACE).to_string());
        }
    }
}

fn nextauth_secret(env: &Env) -> Option<String> {
    env_value(env, "FRAME_LEGACY_MOBILE_NEXTAUTH_SECRET")
}

fn stripe_available(env: &Env) -> bool {
    env_value(env, "STRIPE_SECRET_KEY").is_some_and(|value| !value.is_empty())
}

fn local_provider_test_ack(env: &Env) -> bool {
    let local = env
        .var("FRAME_DEPLOYMENT")
        .map(|value| value.to_string())
        .is_ok_and(|value| matches!(value.as_str(), "local" | "development" | "test"));
    local
        && env
            .var("FRAME_LEGACY_MOBILE_PROVIDER_TEST_ACK")
            .map(|value| value.to_string())
            .is_ok_and(|value| value == "true")
}

fn deployment_origin(env: &Env) -> Option<Url> {
    let web_url = env_value(env, "WEB_URL")?;
    let vercel_env = env_value(env, "VERCEL_ENV");
    let selected = if vercel_env.as_deref() == Some("preview") {
        env_value(env, "VERCEL_BRANCH_URL_HOST")
            .filter(|host| host.ends_with(".vercel.app"))
            .map_or(web_url, |host| format!("https://{host}"))
    } else {
        web_url
    };
    Url::parse(&selected).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn env_value(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
}

const fn map_runtime_failure(
    failure: LegacyMobileSessionRuntimeFailureV1,
) -> LegacyMobileSessionErrorV1 {
    match failure {
        LegacyMobileSessionRuntimeFailureV1::Forbidden => LegacyMobileSessionErrorV1::Forbidden,
        LegacyMobileSessionRuntimeFailureV1::Corrupt
        | LegacyMobileSessionRuntimeFailureV1::Unavailable => LegacyMobileSessionErrorV1::Internal,
    }
}

fn error_response(error: LegacyMobileSessionErrorV1) -> Result<Response> {
    json_response(error.status(), &json!({"_tag": error.tag()}))
}

fn json_response(status: u16, value: &Value) -> Result<Response> {
    let mut response = Response::from_json(value)?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_error_shapes_match_effect_http_api() {
        for (error, status, tag) in [
            (LegacyMobileSessionErrorV1::BadRequest, 400, "BadRequest"),
            (
                LegacyMobileSessionErrorV1::Unauthorized,
                401,
                "Unauthorized",
            ),
            (LegacyMobileSessionErrorV1::Forbidden, 403, "Forbidden"),
            (LegacyMobileSessionErrorV1::NotFound, 404, "NotFound"),
            (
                LegacyMobileSessionErrorV1::Internal,
                500,
                "InternalServerError",
            ),
        ] {
            assert_eq!(error.status(), status);
            assert_eq!(error.tag(), tag);
        }
    }

    #[test]
    fn session_query_ignores_unknowns_but_rejects_provider_and_redirect_drift() {
        let valid = Url::parse(
            "https://example.com/api/mobile/session/request?provider=workos&organizationId=org&redirectUri=cap%3A%2F%2Fauth&ignored=x",
        )
        .expect("checked-in valid session request URL must parse");
        let query =
            decode_session_query(&valid).expect("valid session query must decode successfully");
        assert_eq!(query.provider, Some(LegacyMobileProviderV1::Workos));
        assert_eq!(query.redirect_uri.as_deref(), Some("cap://auth"));
        let bad_provider =
            Url::parse("https://example.com/api/mobile/session/request?provider=github")
                .expect("checked-in invalid-provider URL must still parse");
        assert_eq!(
            decode_session_query(&bad_provider),
            Err(LegacyMobileSessionErrorV1::BadRequest)
        );
        let bad_redirect = Url::parse(
            "https://example.com/api/mobile/session/request?redirectUri=https%3A%2F%2Fevil.test",
        )
        .expect("checked-in invalid-redirect URL must still parse");
        assert_eq!(
            decode_session_query(&bad_redirect),
            Err(LegacyMobileSessionErrorV1::BadRequest)
        );
    }

    #[test]
    fn source_code_range_never_has_a_leading_zero() {
        for _ in 0..256 {
            let code = random_mobile_code().expect("mobile code generation must succeed");
            assert!(legacy_mobile_email_code_is_valid(&code));
            let numeric = code
                .parse::<u32>()
                .expect("generated mobile code must contain only decimal digits");
            assert!((100_000..1_000_000).contains(&numeric));
        }
    }
}
