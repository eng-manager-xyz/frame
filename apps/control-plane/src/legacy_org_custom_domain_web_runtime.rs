//! Exact transport boundary for Cap's desktop organization custom-domain GET.
//!
//! Cap's `withAuth` gives a literal 36-character bearer token precedence over
//! its web session. Frame preserves that selector while authenticating the
//! token through its digest-only D1 representation. The route-level CORS
//! middleware is reproduced for both normal and preflight responses.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Method, Request, Response, Result, send::IntoSendFuture};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    legacy_org_custom_domain_runtime::legacy_desktop_cors_headers,
};

const API_KEY_ACTOR_SQL: &str =
    include_str!("../queries/legacy_org_custom_domain/api_key_actor.sql");
const LEGACY_DESKTOP_ALLOW_METHODS: &str = "GET, POST, PATCH, DELETE, OPTIONS";
const LEGACY_DESKTOP_ALLOW_HEADERS: &str = "Content-Type, Authorization, sentry-trace, baggage";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyDesktopOrgCustomDomainAuthFailureV1 {
    Unauthenticated,
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct ApiKeyActorRowV1 {
    user_id: String,
}

pub(crate) async fn authenticate(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<String, LegacyDesktopOrgCustomDomainAuthFailureV1>> {
    let authorization = request.headers().get("authorization")?;
    let bearer = desktop_api_key_selector(authorization.as_deref());
    if let Some(api_key) = bearer {
        return authenticate_api_key(&env.d1("DB")?, api_key, now_ms).await;
    }
    Ok(
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => Ok(actor_id),
            Err(BrowserWebFailure::Unavailable) => {
                Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable)
            }
            Err(_) => Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unauthenticated),
        },
    )
}

async fn authenticate_api_key(
    database: &D1Database,
    api_key: &str,
    now_ms: i64,
) -> Result<std::result::Result<String, LegacyDesktopOrgCustomDomainAuthFailureV1>> {
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Ok(Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable));
    }
    let digest = lower_hex(&Sha256::digest(api_key.as_bytes()));
    let result = database
        .prepare(API_KEY_ACTOR_SQL)
        .bind(&[JsValue::from_str(&digest), JsValue::from_f64(now_ms as f64)])?
        .all()
        .into_send()
        .await;
    let result = match result {
        Ok(result) if result.success() => result,
        Ok(_) | Err(_) => {
            return Ok(Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable));
        }
    };
    let rows = match result.results::<ApiKeyActorRowV1>() {
        Ok(rows) => rows,
        Err(_) => {
            return Ok(Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable));
        }
    };
    match rows.as_slice() {
        [row] if valid_actor_id(&row.user_id) => Ok(Ok(row.user_id.clone())),
        [] => Ok(Err(
            LegacyDesktopOrgCustomDomainAuthFailureV1::Unauthenticated,
        )),
        _ => Ok(Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable)),
    }
}

fn desktop_api_key_selector(authorization: Option<&str>) -> Option<&str> {
    authorization
        .and_then(|value| value.split(' ').nth(1))
        .filter(|value| value.len() == 36)
}

fn valid_actor_id(actor_id: &str) -> bool {
    !actor_id.is_empty()
        && actor_id.len() <= 256
        && actor_id.is_ascii()
        && !actor_id.bytes().any(|byte| byte.is_ascii_control())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub(crate) fn cors_response(
    mut response: Response,
    request_origin: Option<&str>,
    configured_origin: &str,
) -> Result<Response> {
    for (name, value) in legacy_desktop_cors_headers(request_origin, configured_origin) {
        response.headers_mut().set(&name, &value)?;
    }
    Ok(response)
}

pub(crate) fn preflight_response(request: &Request, configured_origin: &str) -> Result<Response> {
    debug_assert_eq!(request.method(), Method::Options);
    let origin = request.headers().get("origin")?;
    let mut response = cors_response(
        Response::empty()?.with_status(204),
        origin.as_deref(),
        configured_origin,
    )?;
    response
        .headers_mut()
        .set("access-control-allow-methods", LEGACY_DESKTOP_ALLOW_METHODS)?;
    response
        .headers_mut()
        .set("access-control-allow-headers", LEGACY_DESKTOP_ALLOW_HEADERS)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_selector_matches_caps_literal_split_and_length_rule() {
        let key = "12345678-1234-1234-1234-123456789012";
        assert_eq!(
            desktop_api_key_selector(Some(&format!("Bearer {key}"))),
            Some(key)
        );
        assert_eq!(
            desktop_api_key_selector(Some(&format!("Anything {key}"))),
            Some(key)
        );
        assert_eq!(
            desktop_api_key_selector(Some(&format!("Bearer  {key}"))),
            None
        );
        assert_eq!(desktop_api_key_selector(Some("Bearer too-short")), None);
        assert_eq!(desktop_api_key_selector(None), None);
    }

    #[test]
    fn digest_query_is_bounded_and_rejects_inactive_credentials() {
        assert_eq!(API_KEY_ACTOR_SQL.matches('?').count(), 2);
        for token in [
            "k.key_digest = ?1",
            "k.revoked_at_ms IS NULL",
            "k.expires_at_ms > ?2",
            "u.status = 'active'",
            "u.deleted_at_ms IS NULL",
            "LIMIT 1",
        ] {
            assert!(API_KEY_ACTOR_SQL.contains(token), "missing guard: {token}");
        }
        let digest = lower_hex(&Sha256::digest(b"secret"));
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn cors_contract_keeps_hono_allow_lists_exact() {
        assert_eq!(
            LEGACY_DESKTOP_ALLOW_METHODS,
            "GET, POST, PATCH, DELETE, OPTIONS"
        );
        assert_eq!(
            LEGACY_DESKTOP_ALLOW_HEADERS,
            "Content-Type, Authorization, sentry-trace, baggage"
        );
    }
}
