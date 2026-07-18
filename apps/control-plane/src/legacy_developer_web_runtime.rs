//! Authenticated browser ingress for Cap's eight developer-dashboard actions.
//!
//! These actions are owned by the authenticated user rather than an active
//! organization. The carrier therefore derives principal-only authority from
//! trusted session state, preserves every source-observed optional-field
//! distinction, and delegates mutation plus one-use proof consumption to the
//! atomic D1 adapter. Plaintext API credentials exist only long enough to form
//! the exact source-compatible response and are redacted from every `Debug`
//! implementation.

use std::fmt;

use frame_application::{
    LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID, LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID,
    LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID, LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID,
    LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID, LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID,
    LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID, LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID,
    LegacyCallerV1, LegacyDeveloperEnvironmentV1, LegacyDeveloperInputV1,
    LegacyDeveloperNullableLogoPatchV1, LegacyDeveloperSuccessV1, RateLimitDecisionV1,
    RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1, IdempotencyKey,
};
use serde::{Deserialize, Deserializer};
use worker::{Env, Error, Request, Result};
use zeroize::Zeroize;

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyWebDeveloperActionInvocationV1,
    },
    legacy_developer_actions_runtime::{
        D1LegacyDeveloperAtomicPortV1, LocalLegacyDeveloperSecretAuthorityV1,
    },
};

pub const WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-developer-action-request.v1";

const MAX_ACTION_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeveloperActionV1 {
    CreateApp,
    UpdateApp,
    DeleteApp,
    AddDomain,
    RemoveDomain,
    RegenerateKeys,
    DeleteVideo,
    UpdateAutoTopUp,
}

impl DeveloperActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID => Some(Self::CreateApp),
            LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID => Some(Self::UpdateApp),
            LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID => Some(Self::DeleteApp),
            LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID => Some(Self::AddDomain),
            LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID => Some(Self::RemoveDomain),
            LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID => Some(Self::RegenerateKeys),
            LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID => Some(Self::DeleteVideo),
            LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID => Some(Self::UpdateAutoTopUp),
            _ => None,
        }
    }

    const fn operation_id(self) -> &'static str {
        match self {
            Self::CreateApp => LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID,
            Self::UpdateApp => LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID,
            Self::DeleteApp => LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID,
            Self::AddDomain => LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID,
            Self::RemoveDomain => LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID,
            Self::RegenerateKeys => LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID,
            Self::DeleteVideo => LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID,
            Self::UpdateAutoTopUp => LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID,
        }
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    DeveloperActionV1::parse(operation_id).is_some()
}

#[derive(Clone, PartialEq, Eq, Default)]
enum OptionalJsonFieldV1 {
    #[default]
    Missing,
    Present(serde_json::Value),
}

impl fmt::Debug for OptionalJsonFieldV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Missing",
            Self::Present(_) => "Present([redacted])",
        })
    }
}

impl<'de> Deserialize<'de> for OptionalJsonFieldV1 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_json::Value::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAppRequestWireV1 {
    schema_version: String,
    name: String,
    environment: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAppRequestWireV1 {
    schema_version: String,
    app_id: String,
    #[serde(default)]
    name: OptionalJsonFieldV1,
    #[serde(default)]
    environment: OptionalJsonFieldV1,
    #[serde(default)]
    logo_url: OptionalJsonFieldV1,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppOnlyRequestWireV1 {
    schema_version: String,
    app_id: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddDomainRequestWireV1 {
    schema_version: String,
    app_id: String,
    domain: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveDomainRequestWireV1 {
    schema_version: String,
    app_id: String,
    domain_id: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteVideoRequestWireV1 {
    schema_version: String,
    app_id: String,
    video_id: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAutoTopUpRequestWireV1 {
    schema_version: String,
    app_id: String,
    enabled: bool,
    #[serde(default)]
    threshold_micro_credits: OptionalJsonFieldV1,
    #[serde(default)]
    amount_cents: OptionalJsonFieldV1,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DecodedDeveloperActionV1 {
    action: DeveloperActionV1,
    input: LegacyDeveloperInputV1,
    idempotency_key: String,
    body_length: u64,
    content_type: String,
}

impl fmt::Debug for DecodedDeveloperActionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedDeveloperActionV1")
            .field("action", &self.action)
            .field("input", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("body_length", &self.body_length)
            .field("content_type", &self.content_type)
            .finish()
    }
}

/// Exact source response projection. Credential-bearing variants redact their
/// debug output and zeroize the extra response copies as soon as serialization
/// completes.
#[derive(PartialEq, Eq)]
pub enum WebDeveloperActionEffectV1 {
    AppCreated {
        app_id: String,
        public_key: String,
        secret_key: String,
    },
    KeysRegenerated {
        public_key: String,
        secret_key: String,
    },
    SuccessObject,
}

impl fmt::Debug for WebDeveloperActionEffectV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AppCreated { .. } => "AppCreated([redacted])",
            Self::KeysRegenerated { .. } => "KeysRegenerated([redacted])",
            Self::SuccessObject => "SuccessObject",
        })
    }
}

impl Drop for WebDeveloperActionEffectV1 {
    fn drop(&mut self) {
        match self {
            Self::AppCreated {
                app_id,
                public_key,
                secret_key,
            } => {
                app_id.zeroize();
                public_key.zeroize();
                secret_key.zeroize();
            }
            Self::KeysRegenerated {
                public_key,
                secret_key,
            } => {
                public_key.zeroize();
                secret_key.zeroize();
            }
            Self::SuccessObject => {}
        }
    }
}

impl WebDeveloperActionEffectV1 {
    #[must_use]
    pub const fn app_created(&self) -> Option<(&str, &str, &str)> {
        match self {
            Self::AppCreated {
                app_id,
                public_key,
                secret_key,
            } => Some((app_id.as_str(), public_key.as_str(), secret_key.as_str())),
            Self::KeysRegenerated { .. } | Self::SuccessObject => None,
        }
    }

    #[must_use]
    pub const fn regenerated_keys(&self) -> Option<(&str, &str)> {
        match self {
            Self::KeysRegenerated {
                public_key,
                secret_key,
            } => Some((public_key.as_str(), secret_key.as_str())),
            Self::AppCreated { .. } | Self::SuccessObject => None,
        }
    }

    #[must_use]
    pub const fn is_success_object(&self) -> bool {
        matches!(self, Self::SuccessObject)
    }
}

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedDeveloperActionV1>> {
    let Some(action) = DeveloperActionV1::parse(operation_id) else {
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
    let (input, idempotency_key) = match decode_wire(action, &bytes) {
        Ok(decoded) => decoded,
        Err(failure) => return Ok(Err(failure)),
    };
    Ok(Ok(DecodedDeveloperActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len())
            .map_err(|_| Error::RustError("legacy developer body length is invalid".into()))?,
        content_type: content_type.expect("validated content type"),
    }))
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    body: &DecodedDeveloperActionV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<WebDeveloperActionEffectV1>> {
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
    // Validate key authority before issuing a one-use proof. Missing,
    // uppercase, malformed, or all-zero material is a closed configuration
    // failure and its plaintext representation is zeroized immediately.
    let secrets = match developer_secret_authority(env) {
        Ok(secrets) => secrets,
        Err(failure) => return Ok(Err(failure)),
    };
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
        CompatibilityRateLimitBucketV1::DeveloperApi,
        &actor_id,
        now_ms,
    )
    .await
    {
        Ok(rate_limit) => rate_limit,
        Err(error) => {
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let authenticated = match LegacyAuthenticatedContextV1::principal_only(&actor_id) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
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
    let port = D1LegacyDeveloperAtomicPortV1::new(&database);
    let result = transport
        .dispatch_web_developer_action(
            &port,
            &secrets,
            &proof,
            LegacyWebDeveloperActionInvocationV1 {
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
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(map_api_error(error)));
        }
    };
    let effect = match execution.success() {
        LegacyDeveloperSuccessV1::AppCreated {
            legacy_app_id,
            keys,
        } => WebDeveloperActionEffectV1::AppCreated {
            app_id: legacy_app_id.clone(),
            public_key: keys.expose_public_key().to_owned(),
            secret_key: keys.expose_secret_key().to_owned(),
        },
        LegacyDeveloperSuccessV1::KeysRegenerated { keys } => {
            WebDeveloperActionEffectV1::KeysRegenerated {
                public_key: keys.expose_public_key().to_owned(),
                secret_key: keys.expose_secret_key().to_owned(),
            }
        }
        LegacyDeveloperSuccessV1::SuccessObject => WebDeveloperActionEffectV1::SuccessObject,
    };
    Ok(Ok(effect))
}

async fn consume_or_confirm_absent(
    database: &worker::D1Database,
    proof: &frame_application::ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<bool> {
    browser_web_runtime::consume_session_grant_or_confirm_absent(database, proof, now_ms).await
}

fn decode_wire(
    action: DeveloperActionV1,
    bytes: &[u8],
) -> BrowserWebOutcome<(LegacyDeveloperInputV1, String)> {
    match action {
        DeveloperActionV1::CreateApp => {
            let wire = serde_json::from_slice::<CreateAppRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            let environment = parse_environment(&wire.environment)?;
            Ok((
                LegacyDeveloperInputV1::CreateApp {
                    name: wire.name,
                    environment,
                },
                wire.idempotency_key,
            ))
        }
        DeveloperActionV1::UpdateApp => {
            let wire = serde_json::from_slice::<UpdateAppRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.app_id)?;
            let name = optional_string(wire.name, false)?;
            let environment = optional_environment(wire.environment)?;
            let logo_url = nullable_logo(wire.logo_url)?;
            Ok((
                LegacyDeveloperInputV1::UpdateApp {
                    legacy_app_id: wire.app_id,
                    name,
                    environment,
                    logo_url,
                },
                wire.idempotency_key,
            ))
        }
        DeveloperActionV1::DeleteApp | DeveloperActionV1::RegenerateKeys => {
            let wire = serde_json::from_slice::<AppOnlyRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.app_id)?;
            let input = if action == DeveloperActionV1::DeleteApp {
                LegacyDeveloperInputV1::DeleteApp {
                    legacy_app_id: wire.app_id,
                }
            } else {
                LegacyDeveloperInputV1::RegenerateKeys {
                    legacy_app_id: wire.app_id,
                }
            };
            Ok((input, wire.idempotency_key))
        }
        DeveloperActionV1::AddDomain => {
            let wire = serde_json::from_slice::<AddDomainRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.app_id)?;
            Ok((
                LegacyDeveloperInputV1::AddDomain {
                    legacy_app_id: wire.app_id,
                    domain: wire.domain,
                },
                wire.idempotency_key,
            ))
        }
        DeveloperActionV1::RemoveDomain => {
            let wire = serde_json::from_slice::<RemoveDomainRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.app_id)?;
            validate_cap_nanoid(&wire.domain_id)?;
            Ok((
                LegacyDeveloperInputV1::RemoveDomain {
                    legacy_app_id: wire.app_id,
                    legacy_domain_id: wire.domain_id,
                },
                wire.idempotency_key,
            ))
        }
        DeveloperActionV1::DeleteVideo => {
            let wire = serde_json::from_slice::<DeleteVideoRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.app_id)?;
            validate_cap_nanoid(&wire.video_id)?;
            Ok((
                LegacyDeveloperInputV1::DeleteVideo {
                    legacy_app_id: wire.app_id,
                    legacy_video_id: wire.video_id,
                },
                wire.idempotency_key,
            ))
        }
        DeveloperActionV1::UpdateAutoTopUp => {
            let wire = serde_json::from_slice::<UpdateAutoTopUpRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.app_id)?;
            let threshold_micro_credits = optional_i64(wire.threshold_micro_credits)?;
            let amount_cents = optional_i64(wire.amount_cents)?;
            Ok((
                LegacyDeveloperInputV1::UpdateAutoTopUp {
                    legacy_app_id: wire.app_id,
                    enabled: wire.enabled,
                    threshold_micro_credits,
                    amount_cents,
                },
                wire.idempotency_key,
            ))
        }
    }
}

fn validate_common(schema_version: &str, idempotency_key: &str) -> BrowserWebOutcome<()> {
    if schema_version != WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1
        || !valid_idempotency_key(idempotency_key)
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(())
}

fn validate_cap_nanoid(value: &str) -> BrowserWebOutcome<()> {
    valid_cap_nanoid(value)
        .then_some(())
        .ok_or(BrowserWebFailure::Invalid)
}

fn optional_string(
    field: OptionalJsonFieldV1,
    nullable: bool,
) -> BrowserWebOutcome<Option<String>> {
    match field {
        OptionalJsonFieldV1::Missing => Ok(None),
        OptionalJsonFieldV1::Present(serde_json::Value::String(value)) => Ok(Some(value)),
        OptionalJsonFieldV1::Present(serde_json::Value::Null) if nullable => Ok(None),
        OptionalJsonFieldV1::Present(_) => Err(BrowserWebFailure::Invalid),
    }
}

fn nullable_logo(
    field: OptionalJsonFieldV1,
) -> BrowserWebOutcome<LegacyDeveloperNullableLogoPatchV1> {
    match field {
        OptionalJsonFieldV1::Missing => Ok(LegacyDeveloperNullableLogoPatchV1::Missing),
        OptionalJsonFieldV1::Present(serde_json::Value::Null) => {
            Ok(LegacyDeveloperNullableLogoPatchV1::Null)
        }
        OptionalJsonFieldV1::Present(serde_json::Value::String(value)) => {
            Ok(LegacyDeveloperNullableLogoPatchV1::Value(value))
        }
        OptionalJsonFieldV1::Present(_) => Err(BrowserWebFailure::Invalid),
    }
}

fn optional_environment(
    field: OptionalJsonFieldV1,
) -> BrowserWebOutcome<Option<LegacyDeveloperEnvironmentV1>> {
    match field {
        OptionalJsonFieldV1::Missing => Ok(None),
        OptionalJsonFieldV1::Present(serde_json::Value::String(value)) => {
            parse_environment(&value).map(Some)
        }
        OptionalJsonFieldV1::Present(_) => Err(BrowserWebFailure::Invalid),
    }
}

fn parse_environment(value: &str) -> BrowserWebOutcome<LegacyDeveloperEnvironmentV1> {
    match value {
        "development" => Ok(LegacyDeveloperEnvironmentV1::Development),
        "production" => Ok(LegacyDeveloperEnvironmentV1::Production),
        _ => Err(BrowserWebFailure::Invalid),
    }
}

fn optional_i64(field: OptionalJsonFieldV1) -> BrowserWebOutcome<Option<i64>> {
    match field {
        OptionalJsonFieldV1::Missing => Ok(None),
        OptionalJsonFieldV1::Present(serde_json::Value::Number(value)) => {
            value.as_i64().map(Some).ok_or(BrowserWebFailure::Invalid)
        }
        OptionalJsonFieldV1::Present(_) => Err(BrowserWebFailure::Invalid),
    }
}

fn developer_secret_authority(
    env: &Env,
) -> BrowserWebOutcome<LocalLegacyDeveloperSecretAuthorityV1> {
    let secret = env.secret("FRAME_LEGACY_DEVELOPER_SECRET_HEX_V1");
    let secret = secret.map_err(|_| BrowserWebFailure::Unavailable)?;
    let mut value = secret.to_string();
    let authority = LocalLegacyDeveloperSecretAuthorityV1::from_hex(&value)
        .map_err(|_| BrowserWebFailure::Unavailable);
    value.zeroize();
    authority
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

    const APP_ID: &str = "0123456789abcde";

    #[test]
    fn selector_is_closed_to_eight_exact_actions() {
        for operation_id in [
            LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID,
            LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID,
            LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID,
            LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID,
            LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID,
            LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID,
            LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID,
            LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID,
        ] {
            assert!(is_action(operation_id), "missing {operation_id}");
        }
        assert!(!is_action("createDeveloperApp"));
        assert!(!is_action("cap-v1-unknown"));
    }

    #[test]
    fn update_preserves_missing_null_and_value_logo_states() {
        let missing = decode_update(
            r#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","idempotency_key":"developer-update-1"}"#,
        );
        assert!(matches!(
            missing,
            LegacyDeveloperNullableLogoPatchV1::Missing
        ));

        let null = decode_update(
            r#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","logo_url":null,"idempotency_key":"developer-update-1"}"#,
        );
        assert!(matches!(null, LegacyDeveloperNullableLogoPatchV1::Null));

        let value = decode_update(
            r#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","logo_url":"https://cdn.example/logo.png","idempotency_key":"developer-update-1"}"#,
        );
        assert!(matches!(
            value,
            LegacyDeveloperNullableLogoPatchV1::Value(value)
                if value == "https://cdn.example/logo.png"
        ));
    }

    #[test]
    fn optional_non_nullable_fields_reject_null_and_wrong_action_shapes() {
        let null_name = br#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","name":null,"idempotency_key":"developer-update-1"}"#;
        assert_eq!(
            decode_wire(DeveloperActionV1::UpdateApp, null_name),
            Err(BrowserWebFailure::Invalid)
        );
        let null_threshold = br#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","enabled":true,"threshold_micro_credits":null,"idempotency_key":"developer-top-up-1"}"#;
        assert_eq!(
            decode_wire(DeveloperActionV1::UpdateAutoTopUp, null_threshold),
            Err(BrowserWebFailure::Invalid)
        );
        let delete_with_domain = br#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","domain":"https://example.com","idempotency_key":"developer-delete-1"}"#;
        assert_eq!(
            decode_wire(DeveloperActionV1::DeleteApp, delete_with_domain),
            Err(BrowserWebFailure::Invalid)
        );
    }

    #[test]
    fn auto_top_up_preserves_each_optional_property_presence() {
        let bytes = br#"{"schema_version":"frame.web-developer-action-request.v1","app_id":"0123456789abcde","enabled":true,"threshold_micro_credits":500,"idempotency_key":"developer-top-up-1"}"#;
        let (input, _) = decode_wire(DeveloperActionV1::UpdateAutoTopUp, bytes).expect("top up");
        assert!(matches!(
            input,
            LegacyDeveloperInputV1::UpdateAutoTopUp {
                legacy_app_id,
                enabled: true,
                threshold_micro_credits: Some(500),
                amount_cents: None,
            } if legacy_app_id == APP_ID
        ));
    }

    #[test]
    fn secret_bearing_debug_output_is_always_redacted() {
        let public_key = format!("cpk_{}", "0".repeat(30));
        let secret_key = format!("csk_{}", "1".repeat(30));
        let effect = WebDeveloperActionEffectV1::AppCreated {
            app_id: APP_ID.into(),
            public_key: public_key.clone(),
            secret_key: secret_key.clone(),
        };
        let debug = format!("{effect:?}");
        assert!(!debug.contains(APP_ID));
        assert!(!debug.contains(&public_key));
        assert!(!debug.contains(&secret_key));
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn structural_bounds_and_secret_material_validation_are_closed() {
        assert!(valid_cap_nanoid(APP_ID));
        assert!(!valid_cap_nanoid("0123456789abcdi"));
        assert!(valid_idempotency_key("developer-1"));
        assert!(!valid_idempotency_key("short"));
        assert!(LocalLegacyDeveloperSecretAuthorityV1::from_hex(&"01".repeat(32)).is_ok());
        assert!(LocalLegacyDeveloperSecretAuthorityV1::from_hex(&"00".repeat(32)).is_err());
        assert!(LocalLegacyDeveloperSecretAuthorityV1::from_hex(&"AB".repeat(32)).is_err());
    }

    fn decode_update(body: &str) -> LegacyDeveloperNullableLogoPatchV1 {
        let (input, _) =
            decode_wire(DeveloperActionV1::UpdateApp, body.as_bytes()).expect("valid update wire");
        let LegacyDeveloperInputV1::UpdateApp { logo_url, .. } = input else {
            panic!("wrong developer input")
        };
        logo_url
    }
}
