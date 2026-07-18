//! Authenticated browser ingress for Cap's four library-placement actions.
//!
//! The HTTP path is only a Frame selector. The compatibility registry retains
//! each frozen `server_action`/`ACTION` identity while trusted session state
//! supplies the actor and active tenant to the atomic D1 adapter.

use frame_application::{
    LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID, LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID,
    LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID,
    LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID, LegacyCallerV1, LegacyLibraryPlacementInputV1,
    LegacyLibraryPlacementSuccessV1, RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1, IdempotencyKey,
};
use serde::Deserialize;
use worker::{Env, Error, Request, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyWebLibraryPlacementActionInvocationV1,
    },
    legacy_library_placement_runtime::D1LegacyLibraryPlacementAtomicPortV1,
};

pub const WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1: &str = "frame.web-library-placement-request.v1";

const MAX_ACTION_BODY_BYTES: usize = 256 * 1024;
const MAX_VIDEO_IDS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibraryPlacementActionV1 {
    AddToOrganization,
    RemoveFromOrganization,
    AddToSpace,
    RemoveFromSpace,
}

impl LibraryPlacementActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID => Some(Self::AddToOrganization),
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID => {
                Some(Self::RemoveFromOrganization)
            }
            LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID => Some(Self::AddToSpace),
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID => Some(Self::RemoveFromSpace),
            _ => None,
        }
    }

    const fn operation_id(self) -> &'static str {
        match self {
            Self::AddToOrganization => LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID,
            Self::RemoveFromOrganization => LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID,
            Self::AddToSpace => LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID,
            Self::RemoveFromSpace => LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID,
        }
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    LibraryPlacementActionV1::parse(operation_id).is_some()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrganizationRequestWireV1 {
    schema_version: String,
    organization_id: String,
    video_ids: Vec<String>,
    idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeRequestWireV1 {
    schema_version: String,
    scope_id: String,
    video_ids: Vec<String>,
    idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedLibraryPlacementActionV1 {
    action: LibraryPlacementActionV1,
    input: LegacyLibraryPlacementInputV1,
    idempotency_key: String,
    body_length: u64,
    content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebLibraryPlacementActionEffectV1 {
    OrganizationAdded { message: String },
    OrganizationRemoved { message: String },
    ScopeAdded { message: String },
    ScopeRemoved { message: String, deleted_count: u16 },
}

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedLibraryPlacementActionV1>> {
    let Some(action) = LibraryPlacementActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
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
    let (input, idempotency_key) = match action {
        LibraryPlacementActionV1::AddToOrganization
        | LibraryPlacementActionV1::RemoveFromOrganization => {
            let wire = match serde_json::from_slice::<OrganizationRequestWireV1>(&bytes) {
                Ok(wire) => wire,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            };
            if wire.schema_version != WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1
                || !valid_cap_nanoid(&wire.organization_id)
                || !valid_video_ids(&wire.video_ids)
                || !valid_idempotency_key(&wire.idempotency_key)
            {
                return Ok(Err(BrowserWebFailure::Invalid));
            }
            let input = match action {
                LibraryPlacementActionV1::AddToOrganization => {
                    LegacyLibraryPlacementInputV1::AddToOrganization {
                        legacy_organization_id: wire.organization_id,
                        legacy_video_ids: wire.video_ids,
                    }
                }
                LibraryPlacementActionV1::RemoveFromOrganization => {
                    LegacyLibraryPlacementInputV1::RemoveFromOrganization {
                        legacy_organization_id: wire.organization_id,
                        legacy_video_ids: wire.video_ids,
                    }
                }
                LibraryPlacementActionV1::AddToSpace
                | LibraryPlacementActionV1::RemoveFromSpace => {
                    unreachable!("matched organization branch")
                }
            };
            (input, wire.idempotency_key)
        }
        LibraryPlacementActionV1::AddToSpace | LibraryPlacementActionV1::RemoveFromSpace => {
            let wire = match serde_json::from_slice::<ScopeRequestWireV1>(&bytes) {
                Ok(wire) => wire,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            };
            if wire.schema_version != WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1
                || !valid_cap_nanoid(&wire.scope_id)
                || !valid_video_ids(&wire.video_ids)
                || !valid_idempotency_key(&wire.idempotency_key)
            {
                return Ok(Err(BrowserWebFailure::Invalid));
            }
            let input = match action {
                LibraryPlacementActionV1::AddToSpace => LegacyLibraryPlacementInputV1::AddToSpace {
                    legacy_scope_id: wire.scope_id,
                    legacy_video_ids: wire.video_ids,
                },
                LibraryPlacementActionV1::RemoveFromSpace => {
                    LegacyLibraryPlacementInputV1::RemoveFromSpace {
                        legacy_scope_id: wire.scope_id,
                        legacy_video_ids: wire.video_ids,
                    }
                }
                LibraryPlacementActionV1::AddToOrganization
                | LibraryPlacementActionV1::RemoveFromOrganization => {
                    unreachable!("matched scope branch")
                }
            };
            (input, wire.idempotency_key)
        }
    };
    Ok(Ok(DecodedLibraryPlacementActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len()).map_err(|_| {
            Error::RustError("legacy library-placement body length is invalid".into())
        })?,
        content_type: content_type.expect("validated content type"),
    }))
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    body: &DecodedLibraryPlacementActionV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<WebLibraryPlacementActionEffectV1>> {
    let header_key = request.headers().get("idempotency-key")?;
    if header_key.as_deref() != Some(body.idempotency_key.as_str()) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let idempotency_key = match IdempotencyKey::parse(body.idempotency_key.clone()) {
        Ok(key) => key,
        Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    let database = env.d1("DB")?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
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
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        now_ms,
    )
    .await
    {
        Ok(rate_limit) => rate_limit,
        Err(error) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, &proof, now_ms)
            .await?
        {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let active_organization_id =
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await {
            Ok(Some(organization_id)) => organization_id,
            Ok(None) => {
                if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, &proof, now_ms,
                )
                .await?
                {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Ok(Err(BrowserWebFailure::NotFound));
            }
            Err(error) => {
                if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, &proof, now_ms,
                )
                .await?
                {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Err(error);
            }
        };
    let authenticated = match LegacyAuthenticatedContextV1::new(&actor_id, active_organization_id) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
    };
    let security = RequestSecurityContextV1 {
        authenticated: true,
        authorized: true,
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
    };
    let port = D1LegacyLibraryPlacementAtomicPortV1::new(&database);
    let result = transport
        .dispatch_web_library_placement_action(
            &port,
            &proof,
            LegacyWebLibraryPlacementActionInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: body.body_length,
                    content_type: Some(body.content_type.clone()),
                    idempotency_key: Some(idempotency_key),
                    correlation_id: correlation_id.to_owned(),
                },
                security,
                authenticated,
                operation_id: body.action.operation_id(),
                input: body.input.clone(),
                idempotency_key: body.idempotency_key.clone(),
            },
        )
        .await;
    let execution = match result {
        Ok(execution) => execution,
        Err(error) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(map_api_error(error)));
        }
    };
    let message = execution.success().message();
    let effect = match execution.success() {
        LegacyLibraryPlacementSuccessV1::OrganizationAdded { .. } => {
            WebLibraryPlacementActionEffectV1::OrganizationAdded { message }
        }
        LegacyLibraryPlacementSuccessV1::OrganizationRemoved { .. }
        | LegacyLibraryPlacementSuccessV1::OrganizationNoMatching => {
            WebLibraryPlacementActionEffectV1::OrganizationRemoved { message }
        }
        LegacyLibraryPlacementSuccessV1::ScopeAdded { .. } => {
            WebLibraryPlacementActionEffectV1::ScopeAdded { message }
        }
        LegacyLibraryPlacementSuccessV1::ScopeRemoved { .. } => {
            WebLibraryPlacementActionEffectV1::ScopeRemoved {
                message,
                deleted_count: execution
                    .success()
                    .deleted_count()
                    .expect("scope removal has a deleted count"),
            }
        }
    };
    Ok(Ok(effect))
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

fn valid_video_ids(values: &[String]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_VIDEO_IDS
        && values.iter().all(|value| valid_cap_nanoid(value))
}

fn valid_cap_nanoid(value: &str) -> bool {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    value.len() == 15 && value.bytes().all(|byte| ALPHABET.contains(&byte))
}

fn valid_idempotency_key(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
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
    fn selector_is_closed_to_the_four_pinned_operation_ids() {
        for operation_id in [
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_OPERATION_ID,
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_OPERATION_ID,
            LEGACY_ADD_VIDEOS_TO_SPACE_OPERATION_ID,
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_OPERATION_ID,
        ] {
            assert!(is_action(operation_id));
        }
        assert!(!is_action("addVideosToOrganization"));
        assert!(!is_action("cap-v1-unknown"));
    }

    #[test]
    fn payloads_are_action_specific_and_deny_unknown_fields() {
        let organization = serde_json::from_str::<OrganizationRequestWireV1>(
            r#"{"schema_version":"frame.web-library-placement-request.v1","organization_id":"0123456789abcde","video_ids":["1123456789abcde"],"idempotency_key":"library-org-1"}"#,
        )
        .expect("organization wire");
        assert_eq!(organization.video_ids.len(), 1);
        assert!(
            serde_json::from_str::<ScopeRequestWireV1>(
                r#"{"schema_version":"frame.web-library-placement-request.v1","scope_id":"0123456789abcde","video_ids":["1123456789abcde"],"idempotency_key":"library-space-1","tenant_id":"forbidden"}"#,
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<OrganizationRequestWireV1>(
                r#"{"schema_version":"frame.web-library-placement-request.v1","scope_id":"0123456789abcde","video_ids":["1123456789abcde"],"idempotency_key":"library-space-1"}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn structural_bounds_are_closed_before_authentication() {
        assert!(valid_cap_nanoid("0123456789abcde"));
        assert!(!valid_cap_nanoid("0123456789abcdi"));
        assert!(valid_video_ids(&["1123456789abcde".into()]));
        assert!(!valid_video_ids(&[]));
        assert!(!valid_video_ids(&vec![
            "1123456789abcde".into();
            MAX_VIDEO_IDS + 1
        ]));
        assert!(valid_idempotency_key("library-1"));
        assert!(!valid_idempotency_key("short"));
        assert_eq!(declared_body_length(Some("42")), Ok(Some(42)));
        assert_eq!(
            declared_body_length(Some("invalid")),
            Err(BrowserWebFailure::Invalid)
        );
    }
}
