//! Browser-direct ingress for source-pinned legacy server actions.
//!
//! The HTTP route is a Frame transport selector only. After its small typed
//! payload crosses the authenticated same-origin boundary, each dispatcher
//! constructs the frozen zero-body `server_action`/`ACTION` identity. No
//! `action://` identity is ever resolved as an HTTP path.

use frame_application::{
    LegacyCallerV1, LegacyOrganizationSelectionErrorV1, RateLimitDecisionV1,
    RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1, TimestampMillis, TombstonePolicy,
};
use serde::{Deserialize, Serialize};
use worker::{Env, Error, Request, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyWebActiveOrganizationActionEffectV1, LegacyWebActiveOrganizationActionInvocationV1,
        LegacyWebThemeActionEffectV1, LegacyWebThemeActionInvocationV1,
    },
    organization_repository::BrowserFencedLegacySelectionRepository,
};

pub const WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1: &str =
    "frame.web-compatibility-action-request.v1";
pub const ACTIVE_ORGANIZATION_ACTION_ID: &str = "cap-v1-a3b4c805d409bc7c";
pub const THEME_ACTION_ID: &str = "cap-v1-7773d3e70d1d5919";

const MAX_ACTION_BODY_BYTES: usize = 256;
const MAX_ACTION_VALUE_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebCompatibilityActionV1 {
    ActiveOrganization,
    Theme,
}

impl WebCompatibilityActionV1 {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            ACTIVE_ORGANIZATION_ACTION_ID => Some(Self::ActiveOrganization),
            THEME_ACTION_ID => Some(Self::Theme),
            _ => None,
        }
    }

    #[must_use]
    const fn rate_limit_bucket(self) -> CompatibilityRateLimitBucketV1 {
        match self {
            Self::ActiveOrganization => CompatibilityRateLimitBucketV1::OrganizationLibrary,
            Self::Theme => CompatibilityRateLimitBucketV1::ServiceMisc,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebCompatibilityActionRequestV1 {
    pub schema_version: String,
    pub value: String,
}

impl WebCompatibilityActionRequestV1 {
    pub fn validate(&self, action: WebCompatibilityActionV1) -> BrowserWebOutcome<()> {
        if self.schema_version != WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1 {
            return Err(BrowserWebFailure::Invalid);
        }
        let valid = match action {
            WebCompatibilityActionV1::Theme => {
                self.value.len() <= MAX_ACTION_VALUE_BYTES
                    && matches!(self.value.as_str(), "light" | "dark")
            }
            // Target syntax is intentionally evaluated by the authenticated
            // source-pinned adapter, which maps every malformed/unknown Cap ID
            // to the same non-disclosing not-found result.
            WebCompatibilityActionV1::ActiveOrganization => true,
        };
        valid.then_some(()).ok_or(BrowserWebFailure::Invalid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebCompatibilityActionEffectV1 {
    ActiveOrganizationChanged,
    ThemeCookie {
        name: &'static str,
        value: &'static str,
        path: &'static str,
    },
}

pub async fn decode_action_request(
    request: &mut Request,
) -> Result<BrowserWebOutcome<WebCompatibilityActionRequestV1>> {
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|encoding| encoding != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let declared_length =
        match declared_body_length(request.headers().get("content-length")?.as_deref()) {
            Ok(length) => length,
            Err(failure) => return Ok(Err(failure)),
        };
    if declared_length.is_some_and(|length| length == 0 || length > MAX_ACTION_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = match crate::read_bounded_legacy_body(request, MAX_ACTION_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    if bytes.is_empty()
        || bytes.len() > MAX_ACTION_BODY_BYTES
        || declared_length.is_some_and(|length| length != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    Ok(serde_json::from_slice(&bytes).map_err(|_| BrowserWebFailure::Invalid))
}

fn declared_body_length(value: Option<&str>) -> BrowserWebOutcome<Option<usize>> {
    match value {
        Some(value) => value
            .parse::<usize>()
            .map(Some)
            .map_err(|_| BrowserWebFailure::Invalid),
        None => Ok(None),
    }
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    action_text: &str,
    body: &WebCompatibilityActionRequestV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<WebCompatibilityActionEffectV1>> {
    let Some(action) = WebCompatibilityActionV1::parse(action_text) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    if body.validate(action).is_err() || request.headers().get("idempotency-key")?.is_some() {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let database = env.d1("DB")?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
    let occurred_at = TimestampMillis::new(now_ms)
        .map_err(|_| Error::RustError("legacy action clock is invalid".into()))?;
    let tombstone_policy = TombstonePolicy::new(1, 1)
        .map_err(|_| Error::RustError("organization policy is invalid".into()))?;
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let rate_limit = match compatibility_rate_limit::admit_principal(
        env,
        &database,
        action.rate_limit_bucket(),
        &actor_id,
        now_ms,
    )
    .await
    {
        Ok(rate_limit) => rate_limit,
        Err(error) => {
            let _ = browser_web_runtime::consume_session_grant(&database, &proof, now_ms).await;
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if !browser_web_runtime::consume_session_grant(&database, &proof, now_ms).await? {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let security = RequestSecurityContextV1 {
        authenticated: true,
        authorized: true,
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
    };
    let authenticated = match LegacyAuthenticatedContextV1::principal_only(actor_id) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            let _ = browser_web_runtime::consume_session_grant(&database, &proof, now_ms).await?;
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
    };
    let envelope = exact_action_envelope(correlation_id);
    match action {
        WebCompatibilityActionV1::Theme => {
            let effect =
                match transport.dispatch_web_theme_action(LegacyWebThemeActionInvocationV1 {
                    caller: web_caller(),
                    envelope,
                    security,
                    authenticated,
                    value: body.value.clone(),
                }) {
                    Ok(effect) => effect,
                    Err(error) => {
                        if !browser_web_runtime::consume_session_grant(&database, &proof, now_ms)
                            .await?
                        {
                            return Ok(Err(BrowserWebFailure::Unavailable));
                        }
                        return Ok(Err(map_api_error(error)));
                    }
                };
            // The cookie is a response-only effect, so first consume and
            // generation-fence the one-use proof in D1. A failed response is
            // safely retried with a fresh proof under last-write-wins semantics.
            if !browser_web_runtime::consume_session_grant(&database, &proof, now_ms).await? {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            Ok(Ok(match effect {
                LegacyWebThemeActionEffectV1::SetCookieThenResolveVoid { name, value, path } => {
                    WebCompatibilityActionEffectV1::ThemeCookie {
                        name,
                        value: value.as_str(),
                        path,
                    }
                }
            }))
        }
        WebCompatibilityActionV1::ActiveOrganization => {
            // The narrowed repository asserts and consumes this one-use proof
            // atomically with the active selection, operation row, and audit.
            let repository =
                BrowserFencedLegacySelectionRepository::new(&database, tombstone_policy, &proof);
            let result = transport
                .dispatch_web_active_organization_action(
                    &repository,
                    LegacyWebActiveOrganizationActionInvocationV1 {
                        caller: web_caller(),
                        envelope,
                        security,
                        authenticated,
                        legacy_organization_id: body.value.clone(),
                        occurred_at,
                    },
                )
                .await;
            match result {
                Ok(LegacyWebActiveOrganizationActionEffectV1::InvalidateThenResolveVoid {
                    path: "/dashboard",
                }) => Ok(Ok(
                    WebCompatibilityActionEffectV1::ActiveOrganizationChanged,
                )),
                Ok(LegacyWebActiveOrganizationActionEffectV1::InvalidateThenResolveVoid {
                    ..
                }) => unreachable!("dispatcher validates the exact invalidation path"),
                Err(error) => {
                    // A denied/corrupt organization batch rolls back the proof
                    // deletion; consume it separately before returning the
                    // redacted failure, matching native mutation behavior.
                    if !browser_web_runtime::consume_session_grant(&database, &proof, now_ms)
                        .await?
                    {
                        return Ok(Err(BrowserWebFailure::Unavailable));
                    }
                    Ok(Err(map_selection_error(error)))
                }
            }
        }
    }
}

fn exact_action_envelope(correlation_id: &str) -> ApiMutationEnvelopeV1 {
    ApiMutationEnvelopeV1 {
        content_length: 0,
        content_type: None,
        idempotency_key: None,
        correlation_id: correlation_id.to_owned(),
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

const fn web_caller() -> LegacyCallerV1 {
    LegacyCallerV1::Released(ClientReleaseV1 {
        surface: ClientSurfaceV1::Web,
        api_major: 1,
        release: 2,
    })
}

fn map_api_error(error: frame_domain::ApiErrorV1) -> BrowserWebFailure {
    match error.code {
        ApiErrorCodeV1::InvalidRequest => BrowserWebFailure::Invalid,
        ApiErrorCodeV1::Unauthenticated => BrowserWebFailure::Unauthenticated,
        ApiErrorCodeV1::NotFound => BrowserWebFailure::NotFound,
        ApiErrorCodeV1::Conflict => BrowserWebFailure::Conflict,
        ApiErrorCodeV1::RateLimited => BrowserWebFailure::RateLimited,
        ApiErrorCodeV1::Unsupported
        | ApiErrorCodeV1::UpgradeRequired
        | ApiErrorCodeV1::TemporarilyUnavailable
        | ApiErrorCodeV1::Indeterminate
        | ApiErrorCodeV1::Internal => BrowserWebFailure::Unavailable,
    }
}

fn map_selection_error(error: LegacyOrganizationSelectionErrorV1) -> BrowserWebFailure {
    match error {
        LegacyOrganizationSelectionErrorV1::Unauthorized => BrowserWebFailure::Unauthenticated,
        LegacyOrganizationSelectionErrorV1::OrganizationNotFound
        | LegacyOrganizationSelectionErrorV1::Forbidden => BrowserWebFailure::NotFound,
        LegacyOrganizationSelectionErrorV1::ProjectionUnavailable(_)
        | LegacyOrganizationSelectionErrorV1::AuthorityUnavailable
        | LegacyOrganizationSelectionErrorV1::Internal => BrowserWebFailure::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: &str) -> WebCompatibilityActionRequestV1 {
        WebCompatibilityActionRequestV1 {
            schema_version: WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1.into(),
            value: value.into(),
        }
    }

    #[test]
    fn action_transport_selector_is_closed_to_two_operation_ids() {
        assert_eq!(
            WebCompatibilityActionV1::parse(ACTIVE_ORGANIZATION_ACTION_ID),
            Some(WebCompatibilityActionV1::ActiveOrganization)
        );
        assert_eq!(
            WebCompatibilityActionV1::parse(THEME_ACTION_ID),
            Some(WebCompatibilityActionV1::Theme)
        );
        assert_eq!(WebCompatibilityActionV1::parse("setTheme"), None);
        assert_eq!(WebCompatibilityActionV1::parse("cap-v1-unknown"), None);
    }

    #[test]
    fn ingress_payloads_are_small_typed_arguments_not_action_envelopes() {
        assert_eq!(
            request("light").validate(WebCompatibilityActionV1::Theme),
            Ok(())
        );
        assert_eq!(
            request("dark").validate(WebCompatibilityActionV1::Theme),
            Ok(())
        );
        assert_eq!(
            request("0123456789abcde").validate(WebCompatibilityActionV1::ActiveOrganization),
            Ok(())
        );
        for value in ["", "system", "Light", " dark", "dark\n"] {
            assert_eq!(
                request(value).validate(WebCompatibilityActionV1::Theme),
                Err(BrowserWebFailure::Invalid)
            );
        }
        assert_eq!(
            request("not-a-cap-id").validate(WebCompatibilityActionV1::ActiveOrganization),
            Ok(())
        );
        for malformed_target in ["", " ", "<invalid>", "iiiiiiiiiiiiiii"] {
            assert_eq!(
                request(malformed_target).validate(WebCompatibilityActionV1::ActiveOrganization),
                Ok(())
            );
        }
        assert_eq!(
            request(&"x".repeat(MAX_ACTION_VALUE_BYTES + 1))
                .validate(WebCompatibilityActionV1::ActiveOrganization),
            Ok(())
        );
    }

    #[test]
    fn abstract_action_envelope_forbids_http_body_and_client_idempotency() {
        let envelope = exact_action_envelope("correlation");
        assert_eq!(envelope.content_length, 0);
        assert_eq!(envelope.content_type, None);
        assert_eq!(envelope.idempotency_key, None);
        assert_eq!(envelope.correlation_id, "correlation");
    }

    #[test]
    fn ingress_json_denies_unknown_fields() {
        assert!(
            serde_json::from_str::<WebCompatibilityActionRequestV1>(
                r#"{"schema_version":"frame.web-compatibility-action-request.v1","value":"dark","tenant_id":"forbidden"}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn declared_body_length_fails_closed_on_syntax_overflow_and_mismatch() {
        assert_eq!(declared_body_length(None), Ok(None));
        assert_eq!(declared_body_length(Some("42")), Ok(Some(42)));
        assert_eq!(
            declared_body_length(Some("not-a-number")),
            Err(BrowserWebFailure::Invalid)
        );
        assert_eq!(
            declared_body_length(Some("184467440737095516160")),
            Err(BrowserWebFailure::Invalid)
        );
        let actual = 41;
        assert!(declared_body_length(Some("42")).is_ok_and(|value| value != Some(actual)));
    }

    #[test]
    fn absent_length_uses_streaming_bound_and_target_denials_are_not_found() {
        let source = include_str!("legacy_web_action_runtime.rs");
        let decoder = source
            .split("pub async fn decode_action_request")
            .nth(1)
            .and_then(|tail| tail.split("fn declared_body_length").next())
            .expect("decoder source");
        assert!(decoder.contains("read_bounded_legacy_body(request, MAX_ACTION_BODY_BYTES)"));
        assert!(!decoder.contains("request.bytes()"));
        assert_eq!(
            map_selection_error(LegacyOrganizationSelectionErrorV1::OrganizationNotFound),
            BrowserWebFailure::NotFound
        );
    }
}
