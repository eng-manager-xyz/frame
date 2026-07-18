//! Same-origin browser carrier for Cap's two space authorization actions.

use frame_application::{
    LEGACY_SPACE_AUTHORIZATION_MAX_BODY_BYTES, LegacyCallerV1, LegacySpaceAuthorizationActionV1,
    LegacySpaceAuthorizationInputV1, LegacySpaceAuthorizationPortErrorV1,
    LegacySpaceAuthorizationResultV1, RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1,
};
use serde::Deserialize;
use worker::{Env, Request, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyCompatibilityTransportV1, LegacyWebSpaceAuthorizationInvocationV1,
    },
    legacy_space_authorization_runtime::D1LegacySpaceAuthorizationPortV1,
};

pub(crate) const WEB_SPACE_AUTHORIZATION_SCHEMA_V1: &str =
    "frame.web-space-authorization-request.v1";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceAuthorizationWireV1 {
    schema_version: String,
    space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedSpaceAuthorizationV1 {
    pub(crate) action: LegacySpaceAuthorizationActionV1,
    pub(crate) input: LegacySpaceAuthorizationInputV1,
    body_length: u64,
    content_type: String,
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    LegacySpaceAuthorizationActionV1::parse(operation_id).is_some()
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedSpaceAuthorizationV1>> {
    let Some(action) = LegacySpaceAuthorizationActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    if request.headers().get("idempotency-key")?.is_some()
        || !matches!(
            request.headers().get("content-type")?.as_deref(),
            Some("application/json" | "application/json; charset=utf-8")
        )
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let declared = match declared_body_length(request.headers().get("content-length")?.as_deref()) {
        Ok(value) => value,
        Err(failure) => return Ok(Err(failure)),
    };
    if declared.is_some_and(|value| value == 0 || value > LEGACY_SPACE_AUTHORIZATION_MAX_BODY_BYTES)
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let body =
        match crate::read_bounded_legacy_body(request, LEGACY_SPACE_AUTHORIZATION_MAX_BODY_BYTES)
            .await
        {
            Ok(body) => body,
            Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
        };
    if body.is_empty()
        || body.len() > LEGACY_SPACE_AUTHORIZATION_MAX_BODY_BYTES
        || declared.is_some_and(|value| value != body.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let mut decoded = match decode_bytes(action, &body) {
        Ok(decoded) => decoded,
        Err(failure) => return Ok(Err(failure)),
    };
    decoded.body_length = u64::try_from(body.len()).map_err(|_| {
        worker::Error::RustError("legacy space authorization body length is invalid".into())
    })?;
    decoded.content_type = request
        .headers()
        .get("content-type")?
        .expect("validated content type");
    Ok(Ok(decoded))
}

pub(crate) async fn read(
    request: &Request,
    env: &Env,
    decoded: &DecodedSpaceAuthorizationV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<LegacySpaceAuthorizationResultV1>> {
    let actor_id =
        match browser_web_runtime::authenticate_compatibility_read(request, env, now_ms).await? {
            Ok(actor_id) => actor_id,
            Err(failure) => return Ok(Err(failure)),
        };
    let database = env.d1("DB")?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let port = D1LegacySpaceAuthorizationPortV1::new(&database);
    let principal = match port.principal_for_actor(&actor_id).await {
        Ok(principal) => Some(principal),
        Err(LegacySpaceAuthorizationPortErrorV1::NotVisible) => None,
        Err(
            LegacySpaceAuthorizationPortErrorV1::Unavailable
            | LegacySpaceAuthorizationPortErrorV1::Corrupt,
        ) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| {
                worker::Error::RustError("legacy compatibility registry is invalid".into())
            })?;
    let dispatched = transport
        .dispatch_web_space_authorization(
            &port,
            LegacyWebSpaceAuthorizationInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: decoded.body_length,
                    content_type: Some(decoded.content_type.clone()),
                    idempotency_key: None,
                    correlation_id: correlation_id.to_owned(),
                },
                security: RequestSecurityContextV1 {
                    authenticated: true,
                    authorized: true,
                    browser_origin_valid: true,
                    csrf_valid: true,
                    rate_limit,
                },
                principal,
                input: decoded.input.clone(),
            },
        )
        .await;
    match dispatched {
        Ok(result) => Ok(Ok(result)),
        Err(error) => Ok(Err(map_api_error(error))),
    }
}

fn decode_bytes(
    action: LegacySpaceAuthorizationActionV1,
    body: &[u8],
) -> BrowserWebOutcome<DecodedSpaceAuthorizationV1> {
    let wire: SpaceAuthorizationWireV1 =
        serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
    (wire.schema_version == WEB_SPACE_AUTHORIZATION_SCHEMA_V1)
        .then_some(())
        .ok_or(BrowserWebFailure::Invalid)?;
    Ok(DecodedSpaceAuthorizationV1 {
        action,
        input: LegacySpaceAuthorizationInputV1 {
            action,
            legacy_space_id: wire.space_id,
        },
        body_length: u64::try_from(body.len()).unwrap_or(u64::MAX),
        content_type: "application/json".into(),
    })
}

fn declared_body_length(value: Option<&str>) -> BrowserWebOutcome<Option<usize>> {
    value
        .map(str::parse::<usize>)
        .transpose()
        .map_err(|_| BrowserWebFailure::Invalid)
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

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE_ID: &str = "0123456789abcdf";

    #[test]
    fn both_actions_use_the_same_strict_source_shaped_argument() {
        for action in [
            LegacySpaceAuthorizationActionV1::GetSpaceAccess,
            LegacySpaceAuthorizationActionV1::RequireSpaceManager,
        ] {
            let body = format!(
                r#"{{"schema_version":"{WEB_SPACE_AUTHORIZATION_SCHEMA_V1}","space_id":"{SPACE_ID}"}}"#
            );
            let decoded = decode_bytes(action, body.as_bytes()).expect("valid body");
            assert_eq!(decoded.action, action);
            assert_eq!(decoded.input.legacy_space_id, SPACE_ID);
        }
    }

    #[test]
    fn wrong_schema_unknown_fields_and_missing_space_fail_closed() {
        for body in [
            br#"{"schema_version":"wrong","space_id":"0123456789abcdf"}"#.as_slice(),
            br#"{"schema_version":"frame.web-space-authorization-request.v1","space_id":"0123456789abcdf","user_id":"caller"}"#,
            br#"{"schema_version":"frame.web-space-authorization-request.v1"}"#,
        ] {
            assert_eq!(
                decode_bytes(LegacySpaceAuthorizationActionV1::GetSpaceAccess, body),
                Err(BrowserWebFailure::Invalid)
            );
        }
    }
}
