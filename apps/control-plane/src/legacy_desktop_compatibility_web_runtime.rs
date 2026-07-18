//! Exact HTTP and CORS carrier for retained Cap desktop compatibility routes.

use frame_application::{
    LEGACY_DESKTOP_BRANDING_MAX_BODY_BYTES, LEGACY_DESKTOP_MUTATION_MAX_BODY_BYTES, LegacyCallerV1,
    LegacyDesktopBrandingPatchWireV1, LegacyDesktopCompatibilityErrorV1,
    LegacyDesktopCompatibilityInputV1, LegacyDesktopCompatibilityRequestV1,
    LegacyDesktopCompatibilityResultV1, LegacyDesktopCompatibilitySurfaceV1,
    LegacyDesktopCredentialV1, LegacyDesktopStorageProviderV1, LegacyDesktopVideoProgressWireV1,
    RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1,
    IdempotencyKey,
};
use serde::Deserialize;
use serde_json::json;
use worker::{Env, Request, Response, Result};

use crate::{
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyCompatibilityTransportV1, LegacyDesktopCompatibilityInvocationV1,
    },
    legacy_desktop_compatibility_runtime::D1LegacyDesktopCompatibilityPortV1,
    legacy_org_custom_domain_web_runtime::{
        LegacyDesktopOrgCustomDomainAuthFailureV1, authenticate, cors_response,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyDesktopCompatibilityRouteV1<'a> {
    Organizations,
    OrganizationBranding { organization_id: &'a str },
    StorageSetActive,
    UserProfile,
    VideoDelete,
    VideoProgress,
}

impl LegacyDesktopCompatibilityRouteV1<'_> {
    const fn surface(self) -> LegacyDesktopCompatibilitySurfaceV1 {
        match self {
            Self::Organizations => LegacyDesktopCompatibilitySurfaceV1::Organizations,
            Self::OrganizationBranding { .. } => {
                LegacyDesktopCompatibilitySurfaceV1::OrganizationBranding
            }
            Self::StorageSetActive => LegacyDesktopCompatibilitySurfaceV1::StorageSetActive,
            Self::UserProfile => LegacyDesktopCompatibilitySurfaceV1::UserProfile,
            Self::VideoDelete => LegacyDesktopCompatibilitySurfaceV1::VideoDelete,
            Self::VideoProgress => LegacyDesktopCompatibilitySurfaceV1::VideoProgress,
        }
    }

    const fn rate_limit_bucket(self) -> CompatibilityRateLimitBucketV1 {
        match self {
            Self::Organizations | Self::OrganizationBranding { .. } => {
                CompatibilityRateLimitBucketV1::OrganizationLibrary
            }
            Self::StorageSetActive => CompatibilityRateLimitBucketV1::UploadStorage,
            Self::UserProfile => CompatibilityRateLimitBucketV1::ClientCompatibility,
            Self::VideoDelete | Self::VideoProgress => CompatibilityRateLimitBucketV1::VideoMedia,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageWireV1 {
    provider: LegacyDesktopStorageProviderV1,
}

pub(crate) async fn response(
    request: &mut Request,
    env: &Env,
    route: LegacyDesktopCompatibilityRouteV1<'_>,
    now_ms: i64,
    configured_origin: &str,
) -> Result<Response> {
    let origin = request.headers().get("origin")?;
    let response = match handle(request, env, route, now_ms).await {
        Ok(response) => response,
        Err(error) => error_response(route.surface(), error)?,
    };
    cors_response(response, origin.as_deref(), configured_origin)
}

async fn handle(
    request: &mut Request,
    env: &Env,
    route: LegacyDesktopCompatibilityRouteV1<'_>,
    now_ms: i64,
) -> std::result::Result<Response, LegacyDesktopCompatibilityErrorV1> {
    if !(0..=9_007_199_254_740_991).contains(&now_ms) {
        return Err(LegacyDesktopCompatibilityErrorV1::Unavailable);
    }
    let actor_id = match authenticate(request, env, now_ms)
        .await
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::Unavailable)?
    {
        Ok(actor_id) => actor_id,
        Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unauthenticated) => {
            return Err(LegacyDesktopCompatibilityErrorV1::Unauthorized);
        }
        Err(LegacyDesktopOrgCustomDomainAuthFailureV1::Unavailable) => {
            return Err(LegacyDesktopCompatibilityErrorV1::Unavailable);
        }
    };
    let database = env
        .d1("DB")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::Unavailable)?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        route.rate_limit_bucket(),
        &actor_id,
        now_ms,
    )
    .await
    .map_err(|_| LegacyDesktopCompatibilityErrorV1::Unavailable)?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        // Preserve a closed transport boundary. The exact desktop routes have
        // no source 429 body contract, so do not invent a success-shaped body.
        return Err(LegacyDesktopCompatibilityErrorV1::Unavailable);
    }
    let idempotency_key = request
        .headers()
        .get("idempotency-key")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
    let input = decode_input(request, route).await?;
    let content_length = request
        .headers()
        .get("content-length")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
        .unwrap_or_else(|| u64::from(input.surface().mutating()));
    let content_type = request
        .headers()
        .get("content-type")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
        // The carrier accepts the source-compatible JSON media type with an
        // optional charset. Admission policies classify that validated carrier
        // as application/json rather than comparing the raw parameter string.
        .map(|_| "application/json".to_owned());
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::Unavailable)?;
    let port = D1LegacyDesktopCompatibilityPortV1::new(&database, &bucket);
    let transport = LegacyCompatibilityTransportV1::new_fail_closed(
        &database,
        ClientCompatibilityPolicyV1 {
            api_major: 1,
            current_release: 2,
            previous_release: 1,
            deprecated_after_ms: None,
            retired: false,
        },
    )
    .map_err(|_| LegacyDesktopCompatibilityErrorV1::Unavailable)?;
    let outcome = transport
        .dispatch_desktop_compatibility(
            &port,
            LegacyDesktopCompatibilityInvocationV1 {
                caller: LegacyCallerV1::Released(ClientReleaseV1 {
                    surface: ClientSurfaceV1::Desktop,
                    api_major: 1,
                    release: 2,
                }),
                envelope: ApiMutationEnvelopeV1 {
                    content_length,
                    content_type,
                    idempotency_key: idempotency_key
                        .as_deref()
                        .map(IdempotencyKey::parse)
                        .transpose()
                        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?,
                    correlation_id: uuid::Uuid::now_v7().to_string(),
                },
                security: RequestSecurityContextV1 {
                    authenticated: true,
                    authorized: true,
                    browser_origin_valid: true,
                    csrf_valid: true,
                    rate_limit,
                },
                request: LegacyDesktopCompatibilityRequestV1 {
                    actor_id,
                    credential: desktop_credential(request),
                    input,
                    idempotency_key,
                },
            },
        )
        .await?;
    success_response(outcome.result).map_err(|_| LegacyDesktopCompatibilityErrorV1::Internal)
}

async fn decode_input(
    request: &mut Request,
    route: LegacyDesktopCompatibilityRouteV1<'_>,
) -> std::result::Result<LegacyDesktopCompatibilityInputV1, LegacyDesktopCompatibilityErrorV1> {
    match route {
        LegacyDesktopCompatibilityRouteV1::Organizations => {
            reject_read_carriers(request)?;
            Ok(LegacyDesktopCompatibilityInputV1::Organizations)
        }
        LegacyDesktopCompatibilityRouteV1::UserProfile => {
            reject_read_carriers(request)?;
            Ok(LegacyDesktopCompatibilityInputV1::UserProfile)
        }
        LegacyDesktopCompatibilityRouteV1::OrganizationBranding { organization_id } => {
            let body = json_body(request, LEGACY_DESKTOP_BRANDING_MAX_BODY_BYTES).await?;
            let wire: LegacyDesktopBrandingPatchWireV1 = serde_json::from_slice(&body)
                .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
            Ok(LegacyDesktopCompatibilityInputV1::OrganizationBranding {
                legacy_organization_id: organization_id.to_owned(),
                patch: wire.normalize()?,
            })
        }
        LegacyDesktopCompatibilityRouteV1::StorageSetActive => {
            let body = json_body(request, LEGACY_DESKTOP_MUTATION_MAX_BODY_BYTES).await?;
            let wire: StorageWireV1 = serde_json::from_slice(&body)
                .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
            Ok(LegacyDesktopCompatibilityInputV1::StorageSetActive {
                provider: wire.provider,
            })
        }
        LegacyDesktopCompatibilityRouteV1::VideoDelete => {
            reject_body(request)?;
            Ok(LegacyDesktopCompatibilityInputV1::VideoDelete {
                legacy_video_id: query_video_id(request)?,
            })
        }
        LegacyDesktopCompatibilityRouteV1::VideoProgress => {
            let body = json_body(request, LEGACY_DESKTOP_MUTATION_MAX_BODY_BYTES).await?;
            let wire: LegacyDesktopVideoProgressWireV1 = serde_json::from_slice(&body)
                .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
            Ok(LegacyDesktopCompatibilityInputV1::VideoProgress(
                wire.normalize()?,
            ))
        }
    }
}

fn reject_read_carriers(
    request: &Request,
) -> std::result::Result<(), LegacyDesktopCompatibilityErrorV1> {
    if request
        .headers()
        .get("idempotency-key")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
        .is_some()
    {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    reject_body(request)
}

fn reject_body(request: &Request) -> std::result::Result<(), LegacyDesktopCompatibilityErrorV1> {
    if request
        .headers()
        .get("content-length")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
        .is_some_and(|value| value.parse::<u64>().ok() != Some(0))
        || request
            .headers()
            .get("content-type")
            .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
            .is_some()
    {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    Ok(())
}

async fn json_body(
    request: &mut Request,
    max_bytes: usize,
) -> std::result::Result<Vec<u8>, LegacyDesktopCompatibilityErrorV1> {
    let content_type = request
        .headers()
        .get("content-type")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
    if !content_type.as_deref().is_some_and(valid_json_content_type)
        || request
            .headers()
            .get("content-encoding")
            .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
            .is_some_and(|value| value != "identity")
    {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    let declared = request
        .headers()
        .get("content-length")
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
    if declared.is_some_and(|value| value == 0 || value > max_bytes) {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    let body = crate::read_bounded_legacy_body(request, max_bytes)
        .await
        .map_err(|()| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
    if body.is_empty() || declared.is_some_and(|value| value != body.len()) {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    Ok(body)
}

fn valid_json_content_type(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value == "application/json" || value == "application/json; charset=utf-8"
}

fn query_video_id(
    request: &Request,
) -> std::result::Result<String, LegacyDesktopCompatibilityErrorV1> {
    let url = request
        .url()
        .map_err(|_| LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
    let mut values = url::form_urlencoded::parse(url.query().unwrap_or_default().as_bytes())
        .filter(|(name, _)| name == "videoId")
        .map(|(_, value)| value.into_owned());
    let value = values
        .next()
        .filter(|value| !value.is_empty())
        .ok_or(LegacyDesktopCompatibilityErrorV1::InvalidInput)?;
    if values.next().is_some() {
        return Err(LegacyDesktopCompatibilityErrorV1::InvalidInput);
    }
    Ok(value)
}

fn desktop_credential(request: &Request) -> LegacyDesktopCredentialV1 {
    let api_key = request
        .headers()
        .get("authorization")
        .ok()
        .flatten()
        .is_some_and(|value| {
            value
                .split(' ')
                .nth(1)
                .is_some_and(|token| token.len() == 36)
        });
    if api_key {
        LegacyDesktopCredentialV1::ApiKey
    } else {
        LegacyDesktopCredentialV1::Session
    }
}

fn success_response(result: LegacyDesktopCompatibilityResultV1) -> Result<Response> {
    match result {
        LegacyDesktopCompatibilityResultV1::Organizations(value) => Response::from_json(&value),
        LegacyDesktopCompatibilityResultV1::Organization(value) => Response::from_json(&value),
        LegacyDesktopCompatibilityResultV1::StorageSuccess => {
            Response::from_json(&json!({"success": true}))
        }
        LegacyDesktopCompatibilityResultV1::UserProfile(value) => Response::from_json(&value),
        LegacyDesktopCompatibilityResultV1::JsonTrue => Response::from_json(&true),
    }
}

fn error_response(
    surface: LegacyDesktopCompatibilitySurfaceV1,
    error: LegacyDesktopCompatibilityErrorV1,
) -> Result<Response> {
    let response = match error {
        LegacyDesktopCompatibilityErrorV1::Unauthorized => {
            Response::error("User not authenticated", 401)?
        }
        LegacyDesktopCompatibilityErrorV1::NotFound => match surface {
            LegacyDesktopCompatibilitySurfaceV1::OrganizationBranding => {
                Response::from_json(&json!({"error":"Organization not found"}))?.with_status(404)
            }
            LegacyDesktopCompatibilitySurfaceV1::VideoDelete
            | LegacyDesktopCompatibilitySurfaceV1::VideoProgress => {
                Response::from_json(&json!({"error":true,"message":"Video not found"}))?
                    .with_status(404)
            }
            _ => Response::error("Not Found", 404)?,
        },
        LegacyDesktopCompatibilityErrorV1::BrandingForbidden => Response::from_json(&json!({
            "error":"Only organization admins and owners can edit branding"
        }))?
        .with_status(403),
        LegacyDesktopCompatibilityErrorV1::StorageNotConnected => {
            Response::from_json(&json!({"error":"not_connected"}))?.with_status(404)
        }
        LegacyDesktopCompatibilityErrorV1::LogoDataInvalid
        | LegacyDesktopCompatibilityErrorV1::LogoEmpty
        | LegacyDesktopCompatibilityErrorV1::LogoTooLarge
        | LegacyDesktopCompatibilityErrorV1::LogoTypeInvalid => {
            Response::from_json(&json!({"error":error.to_string()}))?.with_status(400)
        }
        LegacyDesktopCompatibilityErrorV1::InvalidInput => {
            Response::from_json(&json!({"error":"Invalid request"}))?.with_status(400)
        }
        LegacyDesktopCompatibilityErrorV1::Conflict => {
            Response::from_json(&json!({"error":"Idempotency conflict"}))?.with_status(409)
        }
        LegacyDesktopCompatibilityErrorV1::Unavailable
        | LegacyDesktopCompatibilityErrorV1::Provider
        | LegacyDesktopCompatibilityErrorV1::Internal => {
            Response::from_json(&json!({"error":"Internal server error"}))?.with_status(500)
        }
    };
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrier_routes_have_exact_source_buckets() {
        assert_eq!(
            LegacyDesktopCompatibilityRouteV1::Organizations
                .rate_limit_bucket()
                .as_str(),
            "organization_library.v1"
        );
        assert_eq!(
            LegacyDesktopCompatibilityRouteV1::StorageSetActive
                .rate_limit_bucket()
                .as_str(),
            "upload_storage.v1"
        );
        assert_eq!(
            LegacyDesktopCompatibilityRouteV1::VideoDelete
                .rate_limit_bucket()
                .as_str(),
            "video_media.v1"
        );
    }

    #[test]
    fn json_media_type_is_strictly_bounded() {
        assert!(valid_json_content_type("application/json"));
        assert!(valid_json_content_type("Application/JSON; Charset=UTF-8"));
        assert!(!valid_json_content_type("text/json"));
    }
}
