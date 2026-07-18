//! Browser carrier for Cap's 21 provider-free organization/library actions.
//!
//! Frame's compatibility route is only a typed transport selector. The
//! application action remains source-pinned, and every authenticated mutation
//! consumes the browser mutation grant in the same D1 batch as its business
//! effects. Public collection-password verification is edge-rate-limited and
//! never receives or consumes a session proof.

use frame_application::{
    LEGACY_ORGANIZATION_LIBRARY_MAX_BODY_BYTES, LEGACY_ORGANIZATION_LIBRARY_REQUEST_SCHEMA_V1,
    LegacyOrganizationLibraryActionV1, LegacyOrganizationLibraryAdapterV1,
    LegacyOrganizationLibraryCredentialV1, LegacyOrganizationLibraryEffectsV1,
    LegacyOrganizationLibraryErrorV1, LegacyOrganizationLibraryInputV1,
    LegacyOrganizationLibraryRequestV1, LegacyOrganizationLibraryResultV1, RateLimitDecisionV1,
};
use frame_domain::{IdempotencyKey, OrganizationId};
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{Env, Error, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_organization_library_runtime::{
        D1R2LegacyOrganizationLibraryPortV1, LegacyOrganizationLibraryLocalConfigV1,
    },
};

const GOOGLE_AUTH_BASE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrganizationLibraryWireV1 {
    schema_version: String,
    #[serde(default)]
    idempotency_key: Option<String>,
    input: LegacyOrganizationLibraryInputV1,
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedOrganizationLibraryActionV1 {
    action: LegacyOrganizationLibraryActionV1,
    input: LegacyOrganizationLibraryInputV1,
    idempotency_key: Option<String>,
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    LegacyOrganizationLibraryActionV1::from_operation_id(operation_id).is_some()
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<std::result::Result<DecodedOrganizationLibraryActionV1, BrowserWebFailure>> {
    let Some(action) = LegacyOrganizationLibraryActionV1::from_operation_id(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|value| value != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let declared = match request.headers().get("content-length")? {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
        },
        None => None,
    };
    if declared
        .is_some_and(|value| value == 0 || value > LEGACY_ORGANIZATION_LIBRARY_MAX_BODY_BYTES)
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_ORGANIZATION_LIBRARY_MAX_BODY_BYTES)
            .await
        {
            Ok(bytes) => bytes,
            Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
        };
    if bytes.is_empty()
        || bytes.len() > LEGACY_ORGANIZATION_LIBRARY_MAX_BODY_BYTES
        || declared.is_some_and(|value| value != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let wire: OrganizationLibraryWireV1 = match serde_json::from_slice(&bytes) {
        Ok(wire) => wire,
        Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    if wire.schema_version != LEGACY_ORGANIZATION_LIBRARY_REQUEST_SCHEMA_V1
        || wire.input.action() != action
        || wire
            .idempotency_key
            .as_deref()
            .is_some_and(|value| IdempotencyKey::parse(value).is_err())
        || (action.requires_session() != wire.idempotency_key.is_some())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    Ok(Ok(DecodedOrganizationLibraryActionV1 {
        action,
        input: wire.input,
        idempotency_key: wire.idempotency_key,
    }))
}

pub(crate) async fn action_response(
    request: &Request,
    env: &Env,
    decoded: &DecodedOrganizationLibraryActionV1,
) -> Result<Response> {
    if request.headers().get("idempotency-key")? != decoded.idempotency_key {
        return error_response(decoded.action, LegacyOrganizationLibraryErrorV1::Invalid);
    }
    let now_ms = crate::current_time_ms()?;
    let database = env.d1("DB")?;

    let proof = if decoded.action.requires_session() {
        match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms).await?
        {
            Ok(proof) => Some(proof),
            Err(failure) => return browser_failure_response(failure),
        }
    } else {
        None
    };

    let rate_limit_result = if let Some(proof) = &proof {
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::OrganizationLibrary,
            &proof.user_id().to_string(),
            now_ms,
        )
        .await
    } else {
        compatibility_rate_limit::admit_edge_request(
            env,
            request,
            CompatibilityRateLimitBucketV1::SharePlayback,
            now_ms,
        )
        .await
    };
    let rate_limit = match rate_limit_result {
        Ok(rate_limit) => rate_limit,
        Err(error) => {
            if let Some(proof) = &proof
                && !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, proof, now_ms,
                )
                .await?
            {
                return error_response(
                    decoded.action,
                    LegacyOrganizationLibraryErrorV1::Unavailable,
                );
            }
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if let Some(proof) = &proof
            && !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, proof, now_ms,
            )
            .await?
        {
            return error_response(
                decoded.action,
                LegacyOrganizationLibraryErrorV1::Unavailable,
            );
        }
        return browser_failure_response(BrowserWebFailure::RateLimited);
    }

    let active_organization_id = if decoded.action.requires_active_tenant() {
        let actor_id = proof
            .as_ref()
            .ok_or_else(|| Error::RustError("organization action proof is unavailable".into()))?
            .user_id()
            .to_string();
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await {
            Ok(value) => value.and_then(|value| OrganizationId::parse(&value).ok()),
            Err(error) => {
                if let Some(proof) = &proof
                    && !browser_web_runtime::consume_session_grant_or_confirm_absent(
                        &database, proof, now_ms,
                    )
                    .await?
                {
                    return error_response(
                        decoded.action,
                        LegacyOrganizationLibraryErrorV1::Unavailable,
                    );
                }
                return Err(error);
            }
        }
    } else {
        None
    };
    if decoded.action.requires_active_tenant() && active_organization_id.is_none() {
        if let Some(proof) = &proof {
            let _ = browser_web_runtime::consume_session_grant(&database, proof, now_ms).await?;
        }
        return error_response(decoded.action, LegacyOrganizationLibraryErrorV1::NotFound);
    }

    let request_model = LegacyOrganizationLibraryRequestV1 {
        credential: Some(if decoded.action.requires_session() {
            LegacyOrganizationLibraryCredentialV1::Session
        } else {
            LegacyOrganizationLibraryCredentialV1::Public
        }),
        actor_id: proof.as_ref().map(|proof| proof.user_id()),
        active_organization_id,
        idempotency_key: decoded.idempotency_key.clone(),
        input: decoded.input.clone(),
    };
    let config = match local_config(env) {
        Ok(config) => config,
        Err(error) => {
            if let Some(proof) = &proof
                && !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, proof, now_ms,
                )
                .await?
            {
                return error_response(
                    decoded.action,
                    LegacyOrganizationLibraryErrorV1::Unavailable,
                );
            }
            return Err(error);
        }
    };
    let bucket = match env.bucket("RECORDINGS") {
        Ok(bucket) => bucket,
        Err(error) => {
            if let Some(proof) = &proof
                && !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, proof, now_ms,
                )
                .await?
            {
                return error_response(
                    decoded.action,
                    LegacyOrganizationLibraryErrorV1::Unavailable,
                );
            }
            return Err(error);
        }
    };
    let port = D1R2LegacyOrganizationLibraryPortV1::new(&database, &bucket, &config, now_ms);
    let adapter = LegacyOrganizationLibraryAdapterV1::new(decoded.action);
    let outcome = adapter.execute(&port, &request_model, proof.as_ref()).await;

    if let Some(proof) = &proof
        && !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, proof, now_ms)
            .await?
    {
        return error_response(
            decoded.action,
            LegacyOrganizationLibraryErrorV1::Unavailable,
        );
    }
    match outcome {
        Ok(execution) => success_response(
            request,
            env,
            decoded.action,
            execution.result(),
            execution.effects(),
        ),
        Err(error) => error_response(decoded.action, error),
    }
}

fn success_response(
    request: &Request,
    env: &Env,
    action: LegacyOrganizationLibraryActionV1,
    result: &LegacyOrganizationLibraryResultV1,
    effects: &LegacyOrganizationLibraryEffectsV1,
) -> Result<Response> {
    let mut response = match result {
        LegacyOrganizationLibraryResultV1::Success => json_response(&json!({"success": true}))?,
        LegacyOrganizationLibraryResultV1::PasswordVerified { password_hash } => {
            let cookie = crate::legacy_video_properties_web_runtime::password_cookie(
                request,
                env,
                password_hash,
            )?;
            let mut response =
                json_response(&json!({"success": true, "value": "Password verified"}))?;
            response.headers_mut().append("set-cookie", &cookie)?;
            response
        }
        LegacyOrganizationLibraryResultV1::PasswordRejected => {
            json_response(&json!({"success": false, "error": "Failed to verify password"}))?
        }
        LegacyOrganizationLibraryResultV1::OrganizationSsoData {
            organization_id,
            connection_id,
            name,
        } => json_response(&json!({
            "organizationId": organization_id,
            "connectionId": connection_id,
            "name": name,
        }))?,
        LegacyOrganizationLibraryResultV1::GoogleDriveAuthorization { url } => {
            json_response(&json!({"url": url}))?
        }
        LegacyOrganizationLibraryResultV1::OrganizationStorageSettings { settings } => {
            json_response(settings)?
        }
        LegacyOrganizationLibraryResultV1::OrganizationCreated {
            legacy_organization_id,
        } => json_response(&json!({"success": true, "organizationId": legacy_organization_id}))?,
        LegacyOrganizationLibraryResultV1::IconUploaded { object_key } => {
            json_response(&json!({"success": true, "iconUrl": object_key}))?
        }
    };
    if !effects.invalidation_paths.is_empty() {
        response.headers_mut().set(
            "x-frame-invalidate-paths",
            &effects.invalidation_paths.join(","),
        )?;
    }
    if action == LegacyOrganizationLibraryActionV1::VerifyCollectionPassword {
        response
            .headers_mut()
            .set("cache-control", "private, no-store")?;
    }
    Ok(response)
}

fn error_response(
    action: LegacyOrganizationLibraryActionV1,
    error: LegacyOrganizationLibraryErrorV1,
) -> Result<Response> {
    if action == LegacyOrganizationLibraryActionV1::VerifyCollectionPassword {
        let mut response =
            json_response(&json!({"success": false, "error": "Failed to verify password"}))?;
        response
            .headers_mut()
            .set("cache-control", "private, no-store")?;
        return Ok(response);
    }
    let status = match error {
        LegacyOrganizationLibraryErrorV1::Unauthorized => 401,
        LegacyOrganizationLibraryErrorV1::Invalid
        | LegacyOrganizationLibraryErrorV1::IdempotencyRequired => 400,
        LegacyOrganizationLibraryErrorV1::NotFound => 404,
        LegacyOrganizationLibraryErrorV1::Conflict => 409,
        LegacyOrganizationLibraryErrorV1::Unavailable => 503,
        LegacyOrganizationLibraryErrorV1::Internal => 500,
    };
    let mut response = Response::from_json(&json!({"success": false}))?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "private, no-store")?;
    Ok(response)
}

fn browser_failure_response(failure: BrowserWebFailure) -> Result<Response> {
    let status = match failure {
        BrowserWebFailure::Unauthenticated => 401,
        BrowserWebFailure::Forbidden => 403,
        BrowserWebFailure::NotFound => 404,
        BrowserWebFailure::Invalid => 400,
        BrowserWebFailure::RateLimited => 429,
        BrowserWebFailure::Conflict => 409,
        BrowserWebFailure::Unavailable => 503,
    };
    Ok(Response::from_json(&json!({"success": false}))?.with_status(status))
}

fn json_response(value: &Value) -> Result<Response> {
    let mut response = Response::from_json(value)?;
    response
        .headers_mut()
        .set("cache-control", "private, no-store")?;
    Ok(response)
}

fn local_config(env: &Env) -> Result<LegacyOrganizationLibraryLocalConfigV1> {
    let web_url = binding(env, "WEB_URL")
        .unwrap_or_else(|| "https://frame.engmanager.xyz".into())
        .trim_end_matches('/')
        .to_owned();
    let state_secret = binding(env, "FRAME_LEGACY_ORGANIZATION_LIBRARY_STATE_SECRET")
        .or_else(|| binding(env, "NEXTAUTH_SECRET"))
        .unwrap_or_default()
        .into_bytes();
    Ok(LegacyOrganizationLibraryLocalConfigV1 {
        google_client_id: binding(env, "GOOGLE_CLIENT_ID").unwrap_or_default(),
        google_redirect_uri: format!("{web_url}/api/desktop/storage/google-drive/callback"),
        google_auth_base_url: GOOGLE_AUTH_BASE_URL.into(),
        state_secret,
        google_picker_api_key: binding(env, "NEXT_PUBLIC_GOOGLE_PICKER_API_KEY"),
    })
}

fn binding(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
        .filter(|value| !value.trim().is_empty())
}
