//! Exact HTTP carrier for Cap's invite accept/decline routes.

use frame_application::{
    LEGACY_INVITE_ACCEPT_OPERATION_ID, LEGACY_INVITE_DECLINE_OPERATION_ID, LegacyCallerV1,
    LegacyInviteActionV1, LegacyInviteErrorV1, LegacyInviteReceiptV1, RateLimitDecisionV1,
    RequestSecurityContextV1,
};
use frame_domain::{
    ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1,
};
use serde_json::{Value, json};
use worker::{Env, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyInviteLifecycleInvocationV1,
    },
    legacy_invite_lifecycle_runtime::D1LegacyInviteAtomicPortV1,
};

const MAX_BODY_BYTES: usize = 256 * 1024;

pub(crate) async fn response(
    request: &mut Request,
    env: &Env,
    request_id: &str,
    action: LegacyInviteActionV1,
) -> Result<Response> {
    // Cap resolves getCurrentUser() before request.json(), so authentication
    // wins over malformed-body errors.
    let actor_id = match browser_web_runtime::authenticate_host_only_browser_session(
        request,
        env,
        crate::current_time_ms()?,
    )
    .await?
    {
        Ok(actor_id) => actor_id,
        Err(BrowserWebFailure::Unavailable) => {
            return exact_error(LegacyInviteErrorV1::Internal);
        }
        Err(_) => return exact_error(LegacyInviteErrorV1::Unauthorized),
    };
    let (legacy_invite_id, body_length, content_type) = match decode_body(request).await? {
        Ok(decoded) => decoded,
        Err(error) => return exact_error(error),
    };
    let database = env.d1("DB")?;
    let now_ms = crate::current_time_ms()?;
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::AuthSession,
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        return exact_error(LegacyInviteErrorV1::Internal);
    }
    let authenticated = LegacyAuthenticatedContextV1::principal_only(&actor_id)
        .map_err(|_| worker::Error::RustError("legacy invite principal is invalid".into()))?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| worker::Error::RustError("legacy invite registry is invalid".into()))?;
    let cap_hosted = env
        .var("FRAME_LEGACY_CAP_HOSTED")
        .map_or(true, |value| value.to_string() != "false");
    let port = D1LegacyInviteAtomicPortV1::new(&database, now_ms, cap_hosted);
    let operation_id = match action {
        LegacyInviteActionV1::Accept => LEGACY_INVITE_ACCEPT_OPERATION_ID,
        LegacyInviteActionV1::Decline => LEGACY_INVITE_DECLINE_OPERATION_ID,
    };
    let result = transport
        .dispatch_invite_lifecycle(
            &port,
            LegacyInviteLifecycleInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: body_length,
                    content_type: Some(content_type),
                    idempotency_key: None,
                    correlation_id: request_id.to_owned(),
                },
                security: admitted_security(rate_limit),
                authenticated,
                operation_id,
                action,
                legacy_invite_id,
            },
        )
        .await;
    match result {
        Ok(LegacyInviteReceiptV1 {
            action: observed, ..
        }) if observed == action => exact_json(200, json!({"success": true})),
        Ok(_) => exact_error(LegacyInviteErrorV1::Internal),
        Err(error) => exact_error(error),
    }
}

async fn decode_body(
    request: &mut Request,
) -> Result<std::result::Result<(String, u64, String), LegacyInviteErrorV1>> {
    let content_type = request
        .headers()
        .get("content-type")?
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if content_type != "application/json" {
        return Ok(Err(LegacyInviteErrorV1::InvalidRequestBody));
    }
    let bytes = request.bytes().await?;
    if bytes.len() > MAX_BODY_BYTES {
        return Ok(Err(LegacyInviteErrorV1::InvalidRequestBody));
    }
    let body_length = match u64::try_from(bytes.len()) {
        Ok(value) => value,
        Err(_) => return Ok(Err(LegacyInviteErrorV1::InvalidRequestBody)),
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(Err(LegacyInviteErrorV1::InvalidRequestBody)),
    };
    if value.is_null() {
        return Ok(Err(LegacyInviteErrorV1::InvalidRequestBody));
    }
    let Some(invite_id) = value
        .as_object()
        .and_then(|object| object.get("inviteId"))
        .and_then(Value::as_str)
    else {
        return Ok(Err(LegacyInviteErrorV1::InvalidInviteId));
    };
    if invite_id.is_empty() {
        return Ok(Err(LegacyInviteErrorV1::InvalidInviteId));
    }
    Ok(Ok((invite_id.to_owned(), body_length, content_type)))
}

fn exact_error(error: LegacyInviteErrorV1) -> Result<Response> {
    let (status, message) = error_projection(error);
    exact_json(status, json!({"error": message}))
}

const fn error_projection(error: LegacyInviteErrorV1) -> (u16, &'static str) {
    match error {
        LegacyInviteErrorV1::Unauthorized => (401, "Unauthorized"),
        LegacyInviteErrorV1::InvalidRequestBody => (400, "Invalid request body"),
        LegacyInviteErrorV1::InvalidInviteId => (400, "Invalid invite ID"),
        LegacyInviteErrorV1::InviteNotFound => (404, "Invite not found"),
        LegacyInviteErrorV1::EmailMismatch => (403, "Email mismatch"),
        LegacyInviteErrorV1::Internal => (500, "Internal server error"),
    }
}

fn exact_json(status: u16, value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
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

const fn admitted_security(rate_limit: RateLimitDecisionV1) -> RequestSecurityContextV1 {
    RequestSecurityContextV1 {
        authenticated: true,
        authorized: true,
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_failure_projection_is_complete() {
        for (error, status, message) in [
            (LegacyInviteErrorV1::Unauthorized, 401, "Unauthorized"),
            (
                LegacyInviteErrorV1::InvalidRequestBody,
                400,
                "Invalid request body",
            ),
            (
                LegacyInviteErrorV1::InvalidInviteId,
                400,
                "Invalid invite ID",
            ),
            (LegacyInviteErrorV1::InviteNotFound, 404, "Invite not found"),
            (LegacyInviteErrorV1::EmailMismatch, 403, "Email mismatch"),
            (LegacyInviteErrorV1::Internal, 500, "Internal server error"),
        ] {
            assert_eq!(error_projection(error), (status, message));
        }
    }
}
