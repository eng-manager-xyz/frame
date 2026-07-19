//! Exact HTTP carrier for Cap's desktop session request route.

use frame_application::{
    LegacyDesktopSessionCredentialTypeV1, LegacyDesktopSessionDestinationV1,
    LegacyDesktopSessionPayloadV1, legacy_desktop_deployment_origin, legacy_desktop_login_url,
    legacy_desktop_session_destination, parse_legacy_desktop_session_query,
};
use frame_ui::class_contract::{
    ALERT_BASE, ALERT_DEFAULT, BUTTON_BASE, BUTTON_DEFAULT_SIZE, BUTTON_GROUP, BUTTON_LINK,
    BUTTON_OUTLINE, BUTTON_PRIMARY, CARD,
};
use url::Url;
use worker::{Env, Request, Response, Result};

use crate::{
    RuntimeConfig,
    browser_web_runtime::{self, BrowserWebFailure},
    legacy_desktop_session_runtime::D1LegacyDesktopSessionV1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyDesktopSessionHttpFailureV1 {
    BadRequest,
    Unavailable,
}

type HttpOutcome<T> = std::result::Result<T, LegacyDesktopSessionHttpFailureV1>;

pub(crate) async fn response(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<Response> {
    match handle(request, env, config, now_ms).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(failure)) => failure_response(failure),
        Err(_) => failure_response(LegacyDesktopSessionHttpFailureV1::Unavailable),
    }
}

async fn handle(
    request: &Request,
    env: &Env,
    config: &RuntimeConfig,
    now_ms: i64,
) -> Result<HttpOutcome<Response>> {
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable));
    }
    if request.headers().get("idempotency-key")?.is_some() {
        return Ok(Err(LegacyDesktopSessionHttpFailureV1::BadRequest));
    }
    let current_url = request.url()?;
    let query = match parse_legacy_desktop_session_query(current_url.query()) {
        Ok(query) => query,
        Err(_) => return Ok(Err(LegacyDesktopSessionHttpFailureV1::BadRequest)),
    };
    let deployment_origin = match deployment_origin(env, config) {
        Some(origin) => origin,
        None => return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable)),
    };
    let request_path_and_query = match current_url.query() {
        Some(query) => format!("{}?{query}", current_url.path()),
        None => current_url.path().to_owned(),
    };
    let login = match legacy_desktop_login_url(&deployment_origin, &request_path_and_query) {
        Some(login) => login,
        None => return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable)),
    };

    let (actor_id, payload) = match query.credential_type {
        LegacyDesktopSessionCredentialTypeV1::Session => {
            let export = match browser_web_runtime::authenticate_host_only_browser_session_export(
                request, env, now_ms,
            )
            .await?
            {
                Ok(export) => export,
                Err(BrowserWebFailure::Unauthenticated | BrowserWebFailure::Invalid) => {
                    return Ok(redirect_response(&login));
                }
                Err(_) => return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable)),
            };
            let expires_seconds = export.expires_at_ms.div_euclid(1_000);
            if expires_seconds <= 0 {
                return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable));
            }
            (
                export.user_id,
                LegacyDesktopSessionPayloadV1::Token {
                    token: export.token,
                    expires: expires_seconds.to_string(),
                },
            )
        }
        LegacyDesktopSessionCredentialTypeV1::ApiKey => {
            let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
                request, env, now_ms,
            )
            .await?
            {
                Ok(actor_id) => actor_id,
                Err(BrowserWebFailure::Unauthenticated | BrowserWebFailure::Invalid) => {
                    return Ok(redirect_response(&login));
                }
                Err(_) => return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable)),
            };
            let database = env.d1("DB")?;
            let api_key = match D1LegacyDesktopSessionV1::new(&database)
                .mint_desktop_key(&actor_id, now_ms)
                .await?
            {
                Ok(api_key) => api_key,
                Err(_) => return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable)),
            };
            (actor_id, LegacyDesktopSessionPayloadV1::ApiKey { api_key })
        }
    };
    let destination = match legacy_desktop_session_destination(query, &payload, &actor_id) {
        Some(destination) => destination,
        None => return Ok(Err(LegacyDesktopSessionHttpFailureV1::Unavailable)),
    };
    Ok(match destination {
        LegacyDesktopSessionDestinationV1::Redirect(url) => redirect_response(&url),
        LegacyDesktopSessionDestinationV1::HybridPage {
            primary_url,
            fallback_url,
        } => hybrid_response(&primary_url, &fallback_url),
    })
}

fn deployment_origin(env: &Env, config: &RuntimeConfig) -> Option<Url> {
    let canonical = env
        .var("WEB_URL")
        .map(|value| value.to_string())
        .unwrap_or_else(|_| crate::storage_origin(config));
    let deployment_environment = env.var("VERCEL_ENV").ok().map(|value| value.to_string());
    let preview_host = env
        .var("VERCEL_BRANCH_URL_HOST")
        .ok()
        .map(|value| value.to_string());
    legacy_desktop_deployment_origin(
        &canonical,
        deployment_environment.as_deref(),
        preview_host.as_deref(),
    )
}

fn redirect_response(url: &Url) -> HttpOutcome<Response> {
    let mut response = Response::empty()
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?
        .with_status(302);
    let headers = response.headers_mut();
    headers
        .set("location", url.as_str())
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    set_no_store(headers).map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    Ok(response)
}

fn hybrid_response(primary_url: &Url, fallback_url: &Url) -> HttpOutcome<Response> {
    let html = render_legacy_desktop_redirect_page(primary_url, fallback_url);
    let mut response = Response::from_bytes(html.into_bytes())
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?
        .with_status(200);
    let headers = response.headers_mut();
    headers
        .set("content-type", "text/html; charset=utf-8")
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    set_no_store(headers).map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    headers
        .set("referrer-policy", "no-referrer")
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    headers
        .set("x-content-type-options", "nosniff")
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    headers
        .set("content-security-policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'")
        .map_err(|_| LegacyDesktopSessionHttpFailureV1::Unavailable)?;
    Ok(response)
}

fn render_legacy_desktop_redirect_page(primary_url: &Url, fallback_url: &Url) -> String {
    let fallback_href = crate::control_plane_ui::escape_html(fallback_url.as_str());
    let serialized_state = serde_json::json!({
        "primaryUrl": primary_url.as_str(),
        "fallbackUrl": fallback_url.as_str(),
    })
    .to_string()
    .replace('&', "\\u0026")
    .replace('<', "\\u003c")
    .replace('>', "\\u003e");
    let content = format!(
        r#"<main class="w-full max-w-xl" tabindex="-1"><section class="{CARD}" aria-labelledby="open-cap-title"><p class="mb-2 text-sm font-semibold text-primary">Desktop handoff</p><h1 id="open-cap-title" class="mt-0 text-2xl font-bold">Opening Cap</h1><p class="text-muted-foreground">If Cap does not open automatically, try the button below. Browser fallback will start in a moment.</p><div class="{BUTTON_GROUP} my-5"><button class="{BUTTON_BASE} {BUTTON_PRIMARY} {BUTTON_DEFAULT_SIZE}" id="open-cap" type="button">Open Cap</button><a class="{BUTTON_BASE} {BUTTON_OUTLINE} {BUTTON_DEFAULT_SIZE} {BUTTON_LINK}" id="browser-fallback" href="{fallback_href}">Use browser fallback</a></div><div class="{ALERT_BASE} {ALERT_DEFAULT} mb-0" id="status" role="status">Trying the desktop app first...</div></section></main>"#
    );
    let script = format!(
        r#"<script>const state={serialized_state};const status=document.getElementById("status");let fallbackStarted=false;const startFallback=()=>{{if(fallbackStarted)return;fallbackStarted=true;status.textContent="Switching to the browser fallback...";window.location.replace(state.fallbackUrl);}};const openCap=()=>{{status.textContent="Trying to open the Cap desktop app...";window.location.href=state.primaryUrl;}};document.getElementById("open-cap").addEventListener("click",openCap);document.getElementById("browser-fallback").addEventListener("click",()=>{{fallbackStarted=true;}});openCap();window.setTimeout(startFallback,1800);</script>"#
    );
    crate::control_plane_ui::utility_document("Open Cap", &format!("{content}{script}"))
}

fn set_no_store(headers: &mut worker::Headers) -> Result<()> {
    headers.set("cache-control", "no-store, no-cache, must-revalidate")?;
    headers.set("pragma", "no-cache")?;
    Ok(())
}

fn failure_response(failure: LegacyDesktopSessionHttpFailureV1) -> Result<Response> {
    let (status, message) = match failure {
        LegacyDesktopSessionHttpFailureV1::BadRequest => (400, "Bad Request"),
        LegacyDesktopSessionHttpFailureV1::Unavailable => (500, "Internal Server Error"),
    };
    let mut response = Response::error(message, status)?;
    set_no_store(response.headers_mut())?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_store_and_failure_shapes_are_explicit() {
        assert_eq!(
            LegacyDesktopSessionHttpFailureV1::BadRequest,
            LegacyDesktopSessionHttpFailureV1::BadRequest
        );
        assert_eq!(
            LegacyDesktopSessionHttpFailureV1::Unavailable,
            LegacyDesktopSessionHttpFailureV1::Unavailable
        );
    }

    #[test]
    fn hybrid_page_uses_shared_ui_and_safe_json_state() {
        let primary =
            Url::parse("cap-desktop://signin?type=api_key&api_key=x").expect("primary deep link");
        let fallback = Url::parse("http://127.0.0.1:43117?x=1&y=2").expect("loopback fallback");
        let html = render_legacy_desktop_redirect_page(&primary, &fallback);
        assert!(html.contains("data-frame-ui=\"shadcn-tailwind\""));
        assert!(html.contains("window.setTimeout(startFallback,1800)"));
        assert!(html.contains("http://127.0.0.1:43117/?x=1&amp;y=2"));
        assert!(html.contains("\\u0026api_key=x"));
        assert!(!html.contains("href=\"http://127.0.0.1:43117/?x=1&y=2\""));
    }
}
