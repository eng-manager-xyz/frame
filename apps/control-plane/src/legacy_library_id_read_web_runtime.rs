//! Same-origin browser carrier for the three library membership ID reads.

use frame_application::{
    LEGACY_LIBRARY_ID_READ_MAX_BODY_BYTES, LegacyCallerV1, LegacyLibraryIdReadActionV1,
    LegacyLibraryIdReadInputV1, LegacyLibraryIdReadPortErrorV1, LegacyLibraryIdReadResultV1,
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
        LegacyCompatibilityTransportV1, LegacyWebLibraryIdReadInvocationV1,
    },
    legacy_library_id_read_runtime::D1LegacyLibraryIdReadPortV1,
};

pub(crate) const WEB_LIBRARY_ID_READ_SCHEMA_V1: &str = "frame.web-library-id-read-request.v1";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderWireV1 {
    schema_version: String,
    folder_id: String,
    space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrganizationWireV1 {
    schema_version: String,
    organization_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceWireV1 {
    schema_version: String,
    space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedLibraryIdReadV1 {
    pub(crate) action: LegacyLibraryIdReadActionV1,
    pub(crate) input: LegacyLibraryIdReadInputV1,
    body_length: u64,
    content_type: String,
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    LegacyLibraryIdReadActionV1::parse(operation_id).is_some()
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedLibraryIdReadV1>> {
    let Some(action) = LegacyLibraryIdReadActionV1::parse(operation_id) else {
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
    if declared.is_some_and(|value| value == 0 || value > LEGACY_LIBRARY_ID_READ_MAX_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let body = match crate::read_bounded_legacy_body(request, LEGACY_LIBRARY_ID_READ_MAX_BODY_BYTES)
        .await
    {
        Ok(body) => body,
        Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    if body.is_empty()
        || body.len() > LEGACY_LIBRARY_ID_READ_MAX_BODY_BYTES
        || declared.is_some_and(|value| value != body.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let mut decoded = match decode_bytes(action, &body) {
        Ok(decoded) => decoded,
        Err(failure) => return Ok(Err(failure)),
    };
    decoded.body_length = u64::try_from(body.len()).map_err(|_| {
        worker::Error::RustError("legacy library read body length is invalid".into())
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
    decoded: &DecodedLibraryIdReadV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<LegacyLibraryIdReadResultV1>> {
    let actor_id =
        match browser_web_runtime::authenticate_compatibility_read(request, env, now_ms).await? {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unauthenticated) => {
                return Ok(Ok(LegacyLibraryIdReadResultV1::Failure {
                    error: "Unauthorized",
                }));
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
    let port = D1LegacyLibraryIdReadPortV1::new(&database);
    let principal = match port.principal_for_actor(&actor_id).await {
        Ok(principal) => Some(principal),
        Err(LegacyLibraryIdReadPortErrorV1::NotVisible) => None,
        Err(
            LegacyLibraryIdReadPortErrorV1::Unavailable | LegacyLibraryIdReadPortErrorV1::Corrupt,
        ) => {
            return Ok(Ok(LegacyLibraryIdReadResultV1::Failure {
                error: decoded.action.stable_read_failure(),
            }));
        }
    };
    let Some(principal) = principal else {
        return Ok(Ok(LegacyLibraryIdReadResultV1::Failure {
            error: "Unauthorized",
        }));
    };
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| {
                worker::Error::RustError("legacy compatibility registry is invalid".into())
            })?;
    let dispatched = transport
        .dispatch_web_library_id_read(
            &port,
            LegacyWebLibraryIdReadInvocationV1 {
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
    action: LegacyLibraryIdReadActionV1,
    body: &[u8],
) -> BrowserWebOutcome<DecodedLibraryIdReadV1> {
    let input = match action {
        LegacyLibraryIdReadActionV1::Folder => {
            let wire: FolderWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            LegacyLibraryIdReadInputV1::Folder {
                legacy_folder_id: wire.folder_id,
                legacy_space_or_organization_id: wire.space_id,
            }
        }
        LegacyLibraryIdReadActionV1::Organization => {
            let wire: OrganizationWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            LegacyLibraryIdReadInputV1::Organization {
                legacy_organization_id: wire.organization_id,
            }
        }
        LegacyLibraryIdReadActionV1::Space => {
            let wire: SpaceWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            LegacyLibraryIdReadInputV1::Space {
                legacy_space_or_organization_id: wire.space_id,
            }
        }
    };
    Ok(DecodedLibraryIdReadV1 {
        action,
        input,
        body_length: u64::try_from(body.len()).unwrap_or(u64::MAX),
        content_type: "application/json".into(),
    })
}

fn require_schema(value: &str) -> BrowserWebOutcome<()> {
    (value == WEB_LIBRARY_ID_READ_SCHEMA_V1)
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
    use frame_application::{
        LEGACY_GET_FOLDER_VIDEO_IDS_OPERATION_ID, LEGACY_GET_ORGANIZATION_VIDEO_IDS_OPERATION_ID,
        LEGACY_GET_SPACE_VIDEO_IDS_OPERATION_ID,
    };

    #[test]
    fn exact_action_specific_wire_shapes_decode() {
        let folder = decode_bytes(
            LegacyLibraryIdReadActionV1::Folder,
            br#"{"schema_version":"frame.web-library-id-read-request.v1","folder_id":"folder","space_id":"space"}"#,
        )
        .expect("folder");
        assert!(matches!(
            folder.input,
            LegacyLibraryIdReadInputV1::Folder { .. }
        ));
        let organization = decode_bytes(
            LegacyLibraryIdReadActionV1::Organization,
            br#"{"schema_version":"frame.web-library-id-read-request.v1","organization_id":"organization"}"#,
        )
        .expect("organization");
        assert!(matches!(
            organization.input,
            LegacyLibraryIdReadInputV1::Organization { .. }
        ));
        let space = decode_bytes(
            LegacyLibraryIdReadActionV1::Space,
            br#"{"schema_version":"frame.web-library-id-read-request.v1","space_id":"space"}"#,
        )
        .expect("space");
        assert!(matches!(
            space.input,
            LegacyLibraryIdReadInputV1::Space { .. }
        ));
    }

    #[test]
    fn wrong_schema_unknown_fields_and_cross_action_shapes_fail_closed() {
        for body in [
            br#"{"schema_version":"wrong","space_id":"space"}"#.as_slice(),
            br#"{"schema_version":"frame.web-library-id-read-request.v1","space_id":"space","extra":true}"#.as_slice(),
            br#"{"schema_version":"frame.web-library-id-read-request.v1","organization_id":"organization"}"#.as_slice(),
        ] {
            assert_eq!(
                decode_bytes(LegacyLibraryIdReadActionV1::Space, body),
                Err(BrowserWebFailure::Invalid)
            );
        }
    }

    #[test]
    fn selector_matches_only_the_three_frozen_operation_ids() {
        for operation_id in [
            LEGACY_GET_FOLDER_VIDEO_IDS_OPERATION_ID,
            LEGACY_GET_ORGANIZATION_VIDEO_IDS_OPERATION_ID,
            LEGACY_GET_SPACE_VIDEO_IDS_OPERATION_ID,
        ] {
            assert!(is_action(operation_id));
        }
        assert!(!is_action("cap-v1-unknown"));
    }
}
