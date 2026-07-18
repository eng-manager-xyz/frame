//! Same-origin browser carrier for Cap's two library-detail read actions.

use frame_application::{
    LEGACY_LIBRARY_DETAIL_MAX_BODY_BYTES, LegacyCallerV1, LegacyLibraryDetailActionV1,
    LegacyLibraryDetailInputV1, LegacyLibraryDetailPortErrorV1, LegacyLibraryDetailResultV1,
    RateLimitDecisionV1, RequestSecurityContextV1,
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
        LegacyCompatibilityTransportV1, LegacyWebLibraryDetailReadInvocationV1,
    },
    legacy_library_detail_read_runtime::D1LegacyLibraryDetailReadPortV1,
};

pub(crate) const WEB_LIBRARY_DETAIL_READ_SCHEMA_V1: &str =
    "frame.web-library-detail-read-request.v1";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserVideosWireV1 {
    schema_version: String,
    space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchWireV1 {
    schema_version: String,
    query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedLibraryDetailReadV1 {
    pub(crate) action: LegacyLibraryDetailActionV1,
    pub(crate) input: LegacyLibraryDetailInputV1,
    body_length: u64,
    content_type: String,
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    LegacyLibraryDetailActionV1::parse(operation_id).is_some()
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedLibraryDetailReadV1>> {
    let Some(action) = LegacyLibraryDetailActionV1::parse(operation_id) else {
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
    if declared.is_some_and(|value| value == 0 || value > LEGACY_LIBRARY_DETAIL_MAX_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let body = match crate::read_bounded_legacy_body(request, LEGACY_LIBRARY_DETAIL_MAX_BODY_BYTES)
        .await
    {
        Ok(body) => body,
        Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    if body.is_empty()
        || body.len() > LEGACY_LIBRARY_DETAIL_MAX_BODY_BYTES
        || declared.is_some_and(|value| value != body.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let mut decoded = match decode_bytes(action, &body) {
        Ok(decoded) => decoded,
        Err(failure) => return Ok(Err(failure)),
    };
    decoded.body_length = u64::try_from(body.len()).map_err(|_| {
        worker::Error::RustError("legacy library detail body length is invalid".into())
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
    decoded: &DecodedLibraryDetailReadV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<LegacyLibraryDetailResultV1>> {
    let actor_id =
        match browser_web_runtime::authenticate_compatibility_read(request, env, now_ms).await? {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unauthenticated) => {
                return Ok(Ok(empty_context_result(decoded.action)));
            }
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
    let port = D1LegacyLibraryDetailReadPortV1::new(&database);
    let principal = match port.principal_for_actor(&actor_id).await {
        Ok(principal) => Some(principal),
        Err(LegacyLibraryDetailPortErrorV1::NotVisible) => None,
        Err(LegacyLibraryDetailPortErrorV1::Unavailable) => {
            return Ok(operation_failure(decoded.action));
        }
        Err(LegacyLibraryDetailPortErrorV1::Corrupt) => {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
    };
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| {
                worker::Error::RustError("legacy compatibility registry is invalid".into())
            })?;
    let dispatched = transport
        .dispatch_web_library_detail_read(
            &port,
            LegacyWebLibraryDetailReadInvocationV1 {
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
    action: LegacyLibraryDetailActionV1,
    body: &[u8],
) -> BrowserWebOutcome<DecodedLibraryDetailReadV1> {
    let input = match action {
        LegacyLibraryDetailActionV1::GetUserVideos => {
            let wire: UserVideosWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            LegacyLibraryDetailInputV1::GetUserVideos {
                legacy_scope_id: wire.space_id,
            }
        }
        LegacyLibraryDetailActionV1::SearchDashboardVideos => {
            let wire: SearchWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            LegacyLibraryDetailInputV1::SearchDashboardVideos { query: wire.query }
        }
    };
    Ok(DecodedLibraryDetailReadV1 {
        action,
        input,
        body_length: u64::try_from(body.len()).unwrap_or(u64::MAX),
        content_type: "application/json".into(),
    })
}

fn empty_context_result(action: LegacyLibraryDetailActionV1) -> LegacyLibraryDetailResultV1 {
    match action {
        LegacyLibraryDetailActionV1::GetUserVideos => {
            LegacyLibraryDetailResultV1::GetUserVideosFailure
        }
        LegacyLibraryDetailActionV1::SearchDashboardVideos => {
            LegacyLibraryDetailResultV1::SearchDashboardVideos { data: Vec::new() }
        }
    }
}

fn operation_failure(
    action: LegacyLibraryDetailActionV1,
) -> BrowserWebOutcome<LegacyLibraryDetailResultV1> {
    match action {
        LegacyLibraryDetailActionV1::GetUserVideos => {
            Ok(LegacyLibraryDetailResultV1::GetUserVideosFailure)
        }
        LegacyLibraryDetailActionV1::SearchDashboardVideos => Err(BrowserWebFailure::Unavailable),
    }
}

fn require_schema(value: &str) -> BrowserWebOutcome<()> {
    (value == WEB_LIBRARY_DETAIL_READ_SCHEMA_V1)
        .then_some(())
        .ok_or(BrowserWebFailure::Invalid)
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

    #[test]
    fn exact_action_specific_wire_shapes_decode() {
        let user_videos = decode_bytes(
            LegacyLibraryDetailActionV1::GetUserVideos,
            br#"{"schema_version":"frame.web-library-detail-read-request.v1","space_id":"scope"}"#,
        )
        .expect("user videos");
        assert!(matches!(
            user_videos.input,
            LegacyLibraryDetailInputV1::GetUserVideos { .. }
        ));
        let search = decode_bytes(
            LegacyLibraryDetailActionV1::SearchDashboardVideos,
            br#"{"schema_version":"frame.web-library-detail-read-request.v1","query":"demo"}"#,
        )
        .expect("search");
        assert!(matches!(
            search.input,
            LegacyLibraryDetailInputV1::SearchDashboardVideos { .. }
        ));
    }

    #[test]
    fn wrong_schema_unknown_fields_and_cross_action_shapes_fail_closed() {
        for body in [
            br#"{"schema_version":"wrong","query":"demo"}"#.as_slice(),
            br#"{"schema_version":"frame.web-library-detail-read-request.v1","query":"demo","extra":true}"#,
            br#"{"schema_version":"frame.web-library-detail-read-request.v1","space_id":"scope"}"#,
        ] {
            assert_eq!(
                decode_bytes(LegacyLibraryDetailActionV1::SearchDashboardVideos, body),
                Err(BrowserWebFailure::Invalid)
            );
        }
    }

    #[test]
    fn missing_context_matches_each_source_envelope() {
        assert_eq!(
            empty_context_result(LegacyLibraryDetailActionV1::GetUserVideos),
            LegacyLibraryDetailResultV1::GetUserVideosFailure
        );
        assert_eq!(
            empty_context_result(LegacyLibraryDetailActionV1::SearchDashboardVideos),
            LegacyLibraryDetailResultV1::SearchDashboardVideos { data: vec![] }
        );
    }
}
