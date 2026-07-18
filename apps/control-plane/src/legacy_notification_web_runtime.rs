//! Authenticated browser ingress for Cap's two notification write actions.
//!
//! The carrier derives the actor and, for mark-as-read, the active tenant from
//! trusted session state. Exact action semantics and one-use proof consumption
//! remain inside the application contract and atomic D1 adapter.

use frame_application::{
    LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID,
    LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID, LegacyCallerV1, LegacyNotificationInputV1,
    LegacyNotificationPreferencesUpdateV1, RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1, IdempotencyKey,
};
use serde::{Deserialize, Deserializer};
use worker::{Env, Error, Request, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyWebNotificationActionInvocationV1,
    },
    legacy_notification_actions_runtime::D1LegacyNotificationAtomicPortV1,
};

pub const WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1: &str =
    "frame.web-notification-action-request.v1";

const MAX_ACTION_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationActionV1 {
    MarkAsRead,
    UpdatePreferences,
}

impl NotificationActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID => Some(Self::MarkAsRead),
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID => Some(Self::UpdatePreferences),
            _ => None,
        }
    }

    const fn operation_id(self) -> &'static str {
        match self {
            Self::MarkAsRead => LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID,
            Self::UpdatePreferences => LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID,
        }
    }

    const fn requires_active_tenant(self) -> bool {
        matches!(self, Self::MarkAsRead)
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    NotificationActionV1::parse(operation_id).is_some()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum OptionalJsonFieldV1 {
    #[default]
    Missing,
    Present(serde_json::Value),
}

impl<'de> Deserialize<'de> for OptionalJsonFieldV1 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_json::Value::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkAsReadRequestWireV1 {
    schema_version: String,
    #[serde(default)]
    notification_id: OptionalJsonFieldV1,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NotificationPreferencesWireV1 {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    #[serde(default)]
    pause_anon_views: OptionalJsonFieldV1,
}

impl std::fmt::Debug for NotificationPreferencesWireV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NotificationPreferencesWireV1([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePreferencesRequestWireV1 {
    schema_version: String,
    notifications: NotificationPreferencesWireV1,
    idempotency_key: String,
}

impl std::fmt::Debug for UpdatePreferencesRequestWireV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdatePreferencesRequestWireV1")
            .field("schema_version", &self.schema_version)
            .field("notifications", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DecodedNotificationActionV1 {
    action: NotificationActionV1,
    input: LegacyNotificationInputV1,
    idempotency_key: String,
    body_length: u64,
    content_type: String,
}

impl std::fmt::Debug for DecodedNotificationActionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedNotificationActionV1")
            .field("action", &self.action)
            .field("input", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("body_length", &self.body_length)
            .field("content_type", &self.content_type)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebNotificationActionVoidV1;

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedNotificationActionV1>> {
    let Some(action) = NotificationActionV1::parse(operation_id) else {
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
        NotificationActionV1::MarkAsRead => {
            let wire = match serde_json::from_slice::<MarkAsReadRequestWireV1>(&bytes) {
                Ok(wire) => wire,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            };
            let notification_id = match wire.notification_id {
                OptionalJsonFieldV1::Missing => None,
                OptionalJsonFieldV1::Present(serde_json::Value::String(value))
                    if valid_cap_nanoid(&value) =>
                {
                    Some(value)
                }
                OptionalJsonFieldV1::Present(_) => {
                    return Ok(Err(BrowserWebFailure::Invalid));
                }
            };
            if wire.schema_version != WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1
                || !valid_idempotency_key(&wire.idempotency_key)
            {
                return Ok(Err(BrowserWebFailure::Invalid));
            }
            (
                LegacyNotificationInputV1::MarkAsRead {
                    legacy_notification_id: notification_id,
                },
                wire.idempotency_key,
            )
        }
        NotificationActionV1::UpdatePreferences => {
            let wire = match serde_json::from_slice::<UpdatePreferencesRequestWireV1>(&bytes) {
                Ok(wire) => wire,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            };
            let pause_anon_views = match wire.notifications.pause_anon_views {
                OptionalJsonFieldV1::Missing => None,
                OptionalJsonFieldV1::Present(serde_json::Value::Bool(value)) => Some(value),
                OptionalJsonFieldV1::Present(_) => {
                    return Ok(Err(BrowserWebFailure::Invalid));
                }
            };
            if wire.schema_version != WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1
                || !valid_idempotency_key(&wire.idempotency_key)
            {
                return Ok(Err(BrowserWebFailure::Invalid));
            }
            (
                LegacyNotificationInputV1::UpdatePreferences {
                    notifications: LegacyNotificationPreferencesUpdateV1::new(
                        wire.notifications.pause_comments,
                        wire.notifications.pause_replies,
                        wire.notifications.pause_views,
                        wire.notifications.pause_reactions,
                        pause_anon_views,
                    ),
                },
                wire.idempotency_key,
            )
        }
    };
    Ok(Ok(DecodedNotificationActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len())
            .map_err(|_| Error::RustError("legacy notification body length is invalid".into()))?,
        content_type: content_type.expect("validated content type"),
    }))
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    body: &DecodedNotificationActionV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<WebNotificationActionVoidV1>> {
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
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
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
    let authenticated = if body.action.requires_active_tenant() {
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
        LegacyAuthenticatedContextV1::new(&actor_id, active_organization_id)
    } else {
        LegacyAuthenticatedContextV1::principal_only(&actor_id)
    };
    let authenticated = match authenticated {
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
    let port = D1LegacyNotificationAtomicPortV1::new(&database);
    let result = transport
        .dispatch_web_notification_action(
            &port,
            &proof,
            LegacyWebNotificationActionInvocationV1 {
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
    match result {
        Ok(_) => Ok(Ok(WebNotificationActionVoidV1)),
        Err(error) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            Ok(Err(map_api_error(error)))
        }
    }
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
    fn selector_is_closed_to_two_exact_actions() {
        assert!(is_action(LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID));
        assert!(is_action(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID
        ));
        assert!(!is_action("markAsRead"));
        assert!(!is_action("cap-v1-unknown"));
    }

    #[test]
    fn missing_and_present_optional_values_are_distinct_and_null_is_rejected() {
        let missing = serde_json::from_str::<MarkAsReadRequestWireV1>(
            r#"{"schema_version":"frame.web-notification-action-request.v1","idempotency_key":"notification-read-1"}"#,
        )
        .expect("missing selector");
        assert_eq!(missing.notification_id, OptionalJsonFieldV1::Missing);
        let null = serde_json::from_str::<MarkAsReadRequestWireV1>(
            r#"{"schema_version":"frame.web-notification-action-request.v1","notification_id":null,"idempotency_key":"notification-read-1"}"#,
        )
        .expect("present null is preserved for validation");
        assert_eq!(
            null.notification_id,
            OptionalJsonFieldV1::Present(serde_json::Value::Null)
        );
        let preferences = serde_json::from_str::<UpdatePreferencesRequestWireV1>(
            r#"{"schema_version":"frame.web-notification-action-request.v1","notifications":{"pauseComments":false,"pauseReplies":false,"pauseViews":false,"pauseReactions":false},"idempotency_key":"notification-preferences-1"}"#,
        )
        .expect("missing anonymous flag");
        assert_eq!(
            preferences.notifications.pause_anon_views,
            OptionalJsonFieldV1::Missing
        );
    }

    #[test]
    fn structural_validation_is_bounded_and_redacted() {
        assert!(valid_cap_nanoid("0123456789abcde"));
        assert!(!valid_cap_nanoid("0123456789abcdi"));
        assert!(valid_idempotency_key("notification-1"));
        assert!(!valid_idempotency_key("short"));
        let preferences = serde_json::from_str::<UpdatePreferencesRequestWireV1>(
            r#"{"schema_version":"frame.web-notification-action-request.v1","notifications":{"pauseComments":true,"pauseReplies":false,"pauseViews":true,"pauseReactions":false,"pauseAnonViews":true},"idempotency_key":"notification-preferences-1"}"#,
        )
        .expect("preferences");
        let debug = format!("{preferences:?}");
        assert!(!debug.contains("true"));
        assert!(!debug.contains("notification-preferences-1"));
    }
}
