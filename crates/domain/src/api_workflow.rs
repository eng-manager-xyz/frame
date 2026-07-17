//! Versioned, runtime-neutral contracts shared by every Frame API surface.
//!
//! These types deliberately exclude provider executors, credentials, object keys,
//! and database details from the public wire contract. Adapters may enrich their
//! private context after validation, but callers cannot use the API to select a
//! privileged executor or infer whether a cross-tenant resource exists.

use std::{fmt, net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{ChecksumSha256, IdempotencyKey, TimestampMillis};

pub const API_CONTRACT_VERSION_V1: &str = "frame.api.v1";
pub const MAX_API_BODY_BYTES_V1: u64 = 8 * 1024 * 1024;
pub const MAX_WEBHOOK_BODY_BYTES_V1: u64 = 1024 * 1024;
pub const MAX_PUBLIC_OUTPUTS_V1: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiAuthClassV1 {
    Public,
    OptionalSession,
    Session,
    /// A released compatibility route may authenticate with either the normal
    /// session capability or a tenant-scoped API key. The transport must still
    /// prove which credential class succeeded before admission.
    SessionOrApiKey,
    ApiKey,
    Worker,
    Webhook,
    Scheduler,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCodeV1 {
    InvalidRequest,
    Unauthenticated,
    NotFound,
    Conflict,
    RateLimited,
    Unsupported,
    UpgradeRequired,
    TemporarilyUnavailable,
    Indeterminate,
    Internal,
}

impl ApiErrorCodeV1 {
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::InvalidRequest => 400,
            Self::Unauthenticated => 401,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::RateLimited => 429,
            Self::Unsupported => 422,
            Self::UpgradeRequired => 426,
            Self::TemporarilyUnavailable | Self::Indeterminate => 503,
            Self::Internal => 500,
        }
    }

    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::TemporarilyUnavailable | Self::Indeterminate | Self::Internal
        )
    }
}

/// The public error envelope has no free-form provider or resource detail.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiErrorV1 {
    pub schema_version: String,
    pub code: ApiErrorCodeV1,
    pub correlation_id: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl ApiErrorV1 {
    pub fn new(
        code: ApiErrorCodeV1,
        correlation_id: impl Into<String>,
        retry_after_ms: Option<u64>,
    ) -> Result<Self, ApiContractErrorV1> {
        let correlation_id = correlation_id.into();
        validate_safe_token(&correlation_id, 96)?;
        if retry_after_ms.is_some_and(|value| value == 0 || value > 86_400_000)
            || (retry_after_ms.is_some() && !code.retryable())
        {
            return Err(ApiContractErrorV1::InvalidRetry);
        }
        Ok(Self {
            schema_version: API_CONTRACT_VERSION_V1.into(),
            code,
            correlation_id,
            retryable: code.retryable(),
            retry_after_ms,
        })
    }

    pub fn validate(&self) -> Result<(), ApiContractErrorV1> {
        if self.schema_version != API_CONTRACT_VERSION_V1
            || self.retryable != self.code.retryable()
            || self
                .retry_after_ms
                .is_some_and(|value| value == 0 || value > 86_400_000)
            || (self.retry_after_ms.is_some() && !self.retryable)
        {
            return Err(ApiContractErrorV1::InvalidRetry);
        }
        validate_safe_token(&self.correlation_id, 96)
    }
}

impl fmt::Debug for ApiErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiErrorV1")
            .field("code", &self.code)
            .field("retryable", &self.retryable)
            .field("correlation_id", &"<redacted>")
            .finish()
    }
}

/// Apply this to every tenant-owned lookup. Unknown and forbidden resources
/// intentionally collapse to the same public result.
pub const fn disclose_resource_v1(exists: bool, authorized: bool) -> Result<(), ApiErrorCodeV1> {
    if exists && authorized {
        Ok(())
    } else {
        Err(ApiErrorCodeV1::NotFound)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSurfaceV1 {
    Web,
    Desktop,
    Mobile,
    Extension,
    Developer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityDecisionV1 {
    Current,
    Previous,
    UpgradeRequired,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyEndpointDispositionV1 {
    Replace,
    Migrate,
    Retire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyRouteDecisionV1 {
    ServeFrameV1,
    UseLegacyFallback,
    RetirementResponse,
    RejectUpgradeRequired,
}

/// Per-route compatibility gate used by desktop, mobile, extension, and
/// developer-client strangle flags. A route cannot redirect merely because it
/// exists: endpoint evidence and the client-family flag must both be true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyRoutePolicyV1 {
    pub disposition: LegacyEndpointDispositionV1,
    pub endpoint_contract_proven: bool,
    pub client_family_enabled: bool,
    pub legacy_fallback_available: bool,
    pub retirement_approved: bool,
}

impl LegacyRoutePolicyV1 {
    pub fn decide(
        &self,
        compatibility: CompatibilityDecisionV1,
    ) -> Result<LegacyRouteDecisionV1, ApiContractErrorV1> {
        if matches!(
            compatibility,
            CompatibilityDecisionV1::UpgradeRequired | CompatibilityDecisionV1::Retired
        ) {
            return Ok(LegacyRouteDecisionV1::RejectUpgradeRequired);
        }
        match self.disposition {
            LegacyEndpointDispositionV1::Retire => {
                if self.retirement_approved {
                    Ok(LegacyRouteDecisionV1::RetirementResponse)
                } else if self.legacy_fallback_available {
                    Ok(LegacyRouteDecisionV1::UseLegacyFallback)
                } else {
                    Err(ApiContractErrorV1::UnsafeCompatibilityCutover)
                }
            }
            LegacyEndpointDispositionV1::Replace | LegacyEndpointDispositionV1::Migrate => {
                if self.endpoint_contract_proven && self.client_family_enabled {
                    Ok(LegacyRouteDecisionV1::ServeFrameV1)
                } else if self.legacy_fallback_available {
                    Ok(LegacyRouteDecisionV1::UseLegacyFallback)
                } else {
                    Err(ApiContractErrorV1::UnsafeCompatibilityCutover)
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientReleaseV1 {
    pub surface: ClientSurfaceV1,
    pub api_major: u16,
    pub release: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCompatibilityPolicyV1 {
    pub api_major: u16,
    pub current_release: u32,
    pub previous_release: u32,
    pub deprecated_after_ms: Option<i64>,
    pub retired: bool,
}

impl ClientCompatibilityPolicyV1 {
    pub fn validate(&self) -> Result<(), ApiContractErrorV1> {
        if self.api_major == 0
            || self.current_release == 0
            || self.previous_release == 0
            || self.previous_release >= self.current_release
            || self
                .deprecated_after_ms
                .is_some_and(|value| TimestampMillis::new(value).is_err())
        {
            return Err(ApiContractErrorV1::InvalidCompatibilityPolicy);
        }
        Ok(())
    }

    pub fn decide(
        &self,
        release: &ClientReleaseV1,
    ) -> Result<CompatibilityDecisionV1, ApiContractErrorV1> {
        self.validate()?;
        if self.retired {
            return Ok(CompatibilityDecisionV1::Retired);
        }
        if release.api_major != self.api_major {
            return Ok(CompatibilityDecisionV1::UpgradeRequired);
        }
        Ok(if release.release == self.current_release {
            CompatibilityDecisionV1::Current
        } else if release.release == self.previous_release {
            CompatibilityDecisionV1::Previous
        } else {
            CompatibilityDecisionV1::UpgradeRequired
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyRequirementV1 {
    Forbidden,
    Optional,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRequestPolicyV1 {
    pub auth: ApiAuthClassV1,
    pub max_body_bytes: u64,
    pub accepted_content_types: Vec<String>,
    pub idempotency: IdempotencyRequirementV1,
    pub rate_limit_bucket: String,
    pub audit_action: String,
}

impl ApiRequestPolicyV1 {
    pub fn validate(&self) -> Result<(), ApiContractErrorV1> {
        if self.max_body_bytes > MAX_API_BODY_BYTES_V1
            || self.accepted_content_types.len() > 8
            || self.accepted_content_types.iter().any(|value| {
                value.is_empty()
                    || value.len() > 96
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'/' | b'+' | b'-' | b'.')
                    })
            })
        {
            return Err(ApiContractErrorV1::InvalidPolicy);
        }
        validate_safe_token(&self.rate_limit_bucket, 64)?;
        validate_safe_token(&self.audit_action, 96)?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ApiMutationEnvelopeV1 {
    pub content_length: u64,
    pub content_type: Option<String>,
    pub idempotency_key: Option<IdempotencyKey>,
    pub correlation_id: String,
}

impl ApiMutationEnvelopeV1 {
    pub fn validate(&self, policy: &ApiRequestPolicyV1) -> Result<(), ApiContractErrorV1> {
        policy.validate()?;
        if self.content_length > policy.max_body_bytes {
            return Err(ApiContractErrorV1::BodyTooLarge);
        }
        if self.content_length > 0 && !policy.accepted_content_types.is_empty() {
            let content_type = self
                .content_type
                .as_deref()
                .and_then(|value| value.split(';').next())
                .ok_or(ApiContractErrorV1::UnsupportedContentType)?;
            if !policy
                .accepted_content_types
                .iter()
                .any(|accepted| accepted == content_type)
            {
                return Err(ApiContractErrorV1::UnsupportedContentType);
            }
        }
        match (policy.idempotency, self.idempotency_key.is_some()) {
            (IdempotencyRequirementV1::Required, false) => {
                return Err(ApiContractErrorV1::MissingIdempotencyKey);
            }
            (IdempotencyRequirementV1::Forbidden, true) => {
                return Err(ApiContractErrorV1::UnexpectedIdempotencyKey);
            }
            _ => {}
        }
        validate_safe_token(&self.correlation_id, 96)
    }
}

impl fmt::Debug for ApiMutationEnvelopeV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiMutationEnvelopeV1")
            .field("content_length", &self.content_length)
            .field(
                "content_type",
                &self.content_type.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("correlation_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivativeRequestV1 {
    pub schema_version: String,
    pub profile: String,
    pub source_version: u64,
    pub idempotency_key: String,
}

impl fmt::Debug for DerivativeRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DerivativeRequestV1")
            .field("schema_version", &self.schema_version)
            .field("profile", &self.profile)
            .field("source_version", &self.source_version)
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

impl DerivativeRequestV1 {
    pub fn validate(&self) -> Result<(), ApiContractErrorV1> {
        if self.schema_version != API_CONTRACT_VERSION_V1 || self.source_version == 0 {
            return Err(ApiContractErrorV1::InvalidDerivativeRequest);
        }
        validate_safe_token(&self.profile, 64)?;
        IdempotencyKey::parse(self.idempotency_key.clone())
            .map_err(|_| ApiContractErrorV1::InvalidDerivativeRequest)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicMediaStateV1 {
    Queued,
    Running,
    Indeterminate,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicMediaFailureV1 {
    InvalidInput,
    Unsupported,
    QuotaExceeded,
    ProviderUnavailable,
    TimedOut,
    OutputRejected,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicDerivativeV1 {
    pub role: String,
    pub path: String,
    pub content_type: String,
    pub bytes: u64,
    pub checksum_sha256: ChecksumSha256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicMediaStatusV1 {
    pub schema_version: String,
    pub state: PublicMediaStateV1,
    pub progress_basis_points: Option<u16>,
    #[serde(default)]
    pub outputs: Vec<PublicDerivativeV1>,
    pub failure: Option<PublicMediaFailureV1>,
    pub retryable: bool,
}

impl PublicMediaStatusV1 {
    pub fn validate(&self) -> Result<(), ApiContractErrorV1> {
        if self.schema_version != API_CONTRACT_VERSION_V1
            || self.outputs.len() > MAX_PUBLIC_OUTPUTS_V1
            || self
                .progress_basis_points
                .is_some_and(|value| value > 10_000)
        {
            return Err(ApiContractErrorV1::InvalidMediaStatus);
        }
        match self.state {
            PublicMediaStateV1::Queued => {
                require_media_shape(self, false, false, false)?;
            }
            PublicMediaStateV1::Running => {
                require_media_shape(self, true, false, false)?;
            }
            PublicMediaStateV1::Indeterminate => {
                require_media_shape(self, false, false, true)?;
            }
            PublicMediaStateV1::Succeeded => {
                if self.progress_basis_points != Some(10_000)
                    || self.outputs.is_empty()
                    || self.failure.is_some()
                    || self.retryable
                {
                    return Err(ApiContractErrorV1::InvalidMediaStatus);
                }
            }
            PublicMediaStateV1::Failed => {
                if self.progress_basis_points.is_some()
                    || !self.outputs.is_empty()
                    || self.failure.is_none()
                {
                    return Err(ApiContractErrorV1::InvalidMediaStatus);
                }
            }
            PublicMediaStateV1::Cancelled => {
                if self.progress_basis_points.is_some()
                    || !self.outputs.is_empty()
                    || self.failure != Some(PublicMediaFailureV1::Cancelled)
                    || self.retryable
                {
                    return Err(ApiContractErrorV1::InvalidMediaStatus);
                }
            }
        }
        for output in &self.outputs {
            validate_safe_token(&output.role, 64)?;
            validate_public_path(&output.path)?;
            validate_content_type(&output.content_type)?;
            if output.bytes == 0 {
                return Err(ApiContractErrorV1::InvalidMediaStatus);
            }
        }
        Ok(())
    }
}

fn require_media_shape(
    status: &PublicMediaStatusV1,
    progress_required: bool,
    failure_required: bool,
    retryable_required: bool,
) -> Result<(), ApiContractErrorV1> {
    if status.progress_basis_points.is_some() != progress_required
        || !status.outputs.is_empty()
        || status.failure.is_some() != failure_required
        || status.retryable != retryable_required
    {
        return Err(ApiContractErrorV1::InvalidMediaStatus);
    }
    Ok(())
}

/// Exact-origin redirect validation. Paths are accepted only when they are
/// relative; absolute redirects must be HTTPS and match the configured origin.
pub fn validate_redirect_v1(value: &str, expected_origin: &str) -> Result<(), ApiContractErrorV1> {
    if value.is_empty()
        || value.len() > 2_048
        || value.chars().any(char::is_control)
        || value.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(ApiContractErrorV1::UnsafeRedirect);
    }
    if value.starts_with('/') && !value.starts_with("//") && !value.contains('\\') {
        return Ok(());
    }
    let target = Url::parse(value).map_err(|_| ApiContractErrorV1::UnsafeRedirect)?;
    let origin = Url::parse(expected_origin).map_err(|_| ApiContractErrorV1::UnsafeRedirect)?;
    if target.scheme() != "https"
        || origin.scheme() != "https"
        || origin.username() != ""
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.path() != "/"
        || target.username() != ""
        || target.password().is_some()
        || target.fragment().is_some()
        || target.host_str() != origin.host_str()
        || target.port_or_known_default() != origin.port_or_known_default()
    {
        return Err(ApiContractErrorV1::UnsafeRedirect);
    }
    Ok(())
}

/// Pre-DNS SSRF validation. Adapters must additionally pass every resolved IP
/// through [`validate_outbound_ip_v1`] and pin that resolution for the request.
pub fn validate_outbound_url_v1(
    value: &str,
    allowed_hosts: &[&str],
) -> Result<String, ApiContractErrorV1> {
    let url = Url::parse(value).map_err(|_| ApiContractErrorV1::UnsafeOutboundUrl)?;
    let host = url
        .host_str()
        .ok_or(ApiContractErrorV1::UnsafeOutboundUrl)?
        .to_ascii_lowercase();
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
        || allowed_hosts.is_empty()
        || !allowed_hosts
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&host))
        || IpAddr::from_str(&host).is_ok()
    {
        return Err(ApiContractErrorV1::UnsafeOutboundUrl);
    }
    Ok(host)
}

pub fn validate_outbound_ip_v1(ip: IpAddr) -> Result<(), ApiContractErrorV1> {
    let blocked = match ip {
        IpAddr::V4(value) => {
            let octets = value.octets();
            value.is_private()
                || value.is_loopback()
                || value.is_link_local()
                || value.is_broadcast()
                || value.is_documentation()
                || value.is_unspecified()
                || value.is_multicast()
                || octets[0] == 0
                // RFC 6598 shared carrier-grade NAT space.
                || (octets[0] == 100 && octets[1] & 0xc0 == 0x40)
                // Protocol assignments, deprecated 6to4 relay anycast, and
                // benchmarking ranges are not public provider destinations.
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
                || (octets[0] == 198 && octets[1] & 0xfe == 18)
                // Class-E/reserved space, including the limited broadcast.
                || octets[0] >= 240
        }
        IpAddr::V6(value) => {
            if let Some(mapped) = value.to_ipv4_mapped() {
                return validate_outbound_ip_v1(IpAddr::V4(mapped));
            }
            let segments = value.segments();
            value.is_loopback()
                || value.is_unspecified()
                || value.is_multicast()
                // Deprecated IPv4-compatible form (`::w.x.y.z`).
                || segments[..6].iter().all(|segment| *segment == 0)
                || segments[0] & 0xfe00 == 0xfc00
                || segments[0] & 0xffc0 == 0xfe80
                || segments[0] & 0xffc0 == 0xfec0
                // Block IPv4 translation prefixes so an allowed hostname
                // cannot smuggle a private IPv4 destination through IPv6.
                || (segments[0] == 0x0064
                    && segments[1] == 0xff9b
                    && ((segments[2] == 0
                        && segments[3] == 0
                        && segments[4] == 0
                        && segments[5] == 0)
                        || segments[2] == 1))
                // Discard-only and IETF special-purpose assignments.
                || (segments[0] == 0x0100
                    && segments[1] == 0
                    && segments[2] == 0
                    && segments[3] == 0)
                || (segments[0] == 0x2001 && segments[1] <= 0x01ff)
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                // 6to4 can encode a private IPv4 target.
                || segments[0] == 0x2002
                // Documentation and segment-routing experiment prefixes.
                || (segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
                || segments[0] == 0x5f00
        }
    };
    if blocked {
        Err(ApiContractErrorV1::UnsafeOutboundUrl)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ApiContractErrorV1 {
    #[error("API policy is invalid")]
    InvalidPolicy,
    #[error("API compatibility policy is invalid")]
    InvalidCompatibilityPolicy,
    #[error("request body exceeds the configured limit")]
    BodyTooLarge,
    #[error("request content type is unsupported")]
    UnsupportedContentType,
    #[error("idempotency key is required")]
    MissingIdempotencyKey,
    #[error("idempotency key is not allowed")]
    UnexpectedIdempotencyKey,
    #[error("derivative request is invalid")]
    InvalidDerivativeRequest,
    #[error("public media status is invalid")]
    InvalidMediaStatus,
    #[error("redirect target is not allowed")]
    UnsafeRedirect,
    #[error("outbound URL is not allowed")]
    UnsafeOutboundUrl,
    #[error("public path is invalid")]
    InvalidPublicPath,
    #[error("safe token is invalid")]
    InvalidSafeToken,
    #[error("content type is invalid")]
    InvalidContentType,
    #[error("retry policy is invalid")]
    InvalidRetry,
    #[error("legacy compatibility cutover is not safe")]
    UnsafeCompatibilityCutover,
}

fn validate_safe_token(value: &str, max: usize) -> Result<(), ApiContractErrorV1> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(ApiContractErrorV1::InvalidSafeToken);
    }
    Ok(())
}

fn validate_public_path(value: &str) -> Result<(), ApiContractErrorV1> {
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.len() > 512
        || value.chars().any(char::is_control)
        || value.bytes().any(|byte| byte.is_ascii_whitespace())
        || value.contains(['?', '#', '\\'])
        || value.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(ApiContractErrorV1::InvalidPublicPath);
    }
    Ok(())
}

fn validate_content_type(value: &str) -> Result<(), ApiContractErrorV1> {
    let mut parts = value.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.len() <= 64
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'+' | b'-')
            })
    };
    if !parts.next().is_some_and(valid_part)
        || !parts.next().is_some_and(valid_part)
        || parts.next().is_some()
    {
        return Err(ApiContractErrorV1::InvalidContentType);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum() -> ChecksumSha256 {
        ChecksumSha256::parse("ab".repeat(32)).expect("checksum")
    }

    #[test]
    fn lookup_policy_does_not_disclose_existence() {
        assert_eq!(
            disclose_resource_v1(false, false),
            disclose_resource_v1(true, false)
        );
        assert_eq!(disclose_resource_v1(true, true), Ok(()));
    }

    #[test]
    fn mutation_policy_bounds_body_type_and_idempotency() {
        let policy = ApiRequestPolicyV1 {
            auth: ApiAuthClassV1::Session,
            max_body_bytes: 128,
            accepted_content_types: vec!["application/json".into()],
            idempotency: IdempotencyRequirementV1::Required,
            rate_limit_bucket: "video_mutation_v1".into(),
            audit_action: "video.update".into(),
        };
        let valid = ApiMutationEnvelopeV1 {
            content_length: 12,
            content_type: Some("application/json; charset=utf-8".into()),
            idempotency_key: Some(IdempotencyKey::parse("request-123").expect("key")),
            correlation_id: "trace-123".into(),
        };
        assert_eq!(valid.validate(&policy), Ok(()));
        let mut invalid = valid.clone();
        invalid.content_length = 129;
        assert_eq!(
            invalid.validate(&policy),
            Err(ApiContractErrorV1::BodyTooLarge)
        );
        invalid = valid;
        invalid.idempotency_key = None;
        assert_eq!(
            invalid.validate(&policy),
            Err(ApiContractErrorV1::MissingIdempotencyKey)
        );
    }

    #[test]
    fn media_contract_never_exposes_an_executor() {
        let request = DerivativeRequestV1 {
            schema_version: API_CONTRACT_VERSION_V1.into(),
            profile: "thumbnail_jpeg_v1".into(),
            source_version: 2,
            idempotency_key: "derivative-123".into(),
        };
        request.validate().expect("request");
        let json = serde_json::to_string(&request).expect("json");
        assert!(!json.contains("cloudflare"));
        assert!(!json.contains("gstreamer"));
        assert!(!json.contains("executor"));

        let status = PublicMediaStatusV1 {
            schema_version: API_CONTRACT_VERSION_V1.into(),
            state: PublicMediaStateV1::Succeeded,
            progress_basis_points: Some(10_000),
            outputs: vec![PublicDerivativeV1 {
                role: "thumbnail".into(),
                path: "/api/v1/media/jobs/job-1/outputs/0".into(),
                content_type: "image/jpeg".into(),
                bytes: 42,
                checksum_sha256: checksum(),
            }],
            failure: None,
            retryable: false,
        };
        status.validate().expect("status");
    }

    #[test]
    fn media_progress_and_failure_shapes_are_closed() {
        let running = PublicMediaStatusV1 {
            schema_version: API_CONTRACT_VERSION_V1.into(),
            state: PublicMediaStateV1::Running,
            progress_basis_points: None,
            outputs: vec![],
            failure: None,
            retryable: false,
        };
        assert_eq!(
            running.validate(),
            Err(ApiContractErrorV1::InvalidMediaStatus)
        );

        let indeterminate = PublicMediaStatusV1 {
            schema_version: API_CONTRACT_VERSION_V1.into(),
            state: PublicMediaStateV1::Indeterminate,
            progress_basis_points: None,
            outputs: vec![],
            failure: None,
            retryable: true,
        };
        assert_eq!(indeterminate.validate(), Ok(()));
    }

    #[test]
    fn compatibility_supports_exactly_current_and_previous() {
        let policy = ClientCompatibilityPolicyV1 {
            api_major: 1,
            current_release: 12,
            previous_release: 11,
            deprecated_after_ms: Some(1_900_000_000_000),
            retired: false,
        };
        let release = |api_major, release| ClientReleaseV1 {
            surface: ClientSurfaceV1::Desktop,
            api_major,
            release,
        };
        assert_eq!(
            policy.decide(&release(1, 12)),
            Ok(CompatibilityDecisionV1::Current)
        );
        assert_eq!(
            policy.decide(&release(1, 11)),
            Ok(CompatibilityDecisionV1::Previous)
        );
        assert_eq!(
            policy.decide(&release(1, 10)),
            Ok(CompatibilityDecisionV1::UpgradeRequired)
        );
        assert_eq!(
            policy.decide(&release(2, 12)),
            Ok(CompatibilityDecisionV1::UpgradeRequired)
        );
    }

    #[test]
    fn legacy_routes_cannot_redirect_before_endpoint_evidence() {
        let pending = LegacyRoutePolicyV1 {
            disposition: LegacyEndpointDispositionV1::Replace,
            endpoint_contract_proven: false,
            client_family_enabled: true,
            legacy_fallback_available: true,
            retirement_approved: false,
        };
        assert_eq!(
            pending.decide(CompatibilityDecisionV1::Current),
            Ok(LegacyRouteDecisionV1::UseLegacyFallback)
        );
        let ready = LegacyRoutePolicyV1 {
            endpoint_contract_proven: true,
            ..pending.clone()
        };
        assert_eq!(
            ready.decide(CompatibilityDecisionV1::Previous),
            Ok(LegacyRouteDecisionV1::ServeFrameV1)
        );
        assert_eq!(
            ready.decide(CompatibilityDecisionV1::UpgradeRequired),
            Ok(LegacyRouteDecisionV1::RejectUpgradeRequired)
        );

        let unapproved_retirement = LegacyRoutePolicyV1 {
            disposition: LegacyEndpointDispositionV1::Retire,
            endpoint_contract_proven: false,
            client_family_enabled: false,
            legacy_fallback_available: false,
            retirement_approved: false,
        };
        assert_eq!(
            unapproved_retirement.decide(CompatibilityDecisionV1::Current),
            Err(ApiContractErrorV1::UnsafeCompatibilityCutover)
        );
    }

    #[test]
    fn redirect_and_ssrf_policies_are_exact_and_two_phase() {
        assert!(validate_public_path("/api/v1/media/output\r\nheader").is_err());
        assert!(validate_redirect_v1("/dashboard", "https://frame.example").is_ok());
        assert!(
            validate_redirect_v1("https://frame.example/share/1", "https://frame.example").is_ok()
        );
        assert!(validate_redirect_v1("https://evil.example", "https://frame.example").is_err());
        assert!(validate_redirect_v1("//evil.example", "https://frame.example").is_err());
        assert!(validate_redirect_v1("/safe\nheader", "https://frame.example").is_err());

        assert_eq!(
            validate_outbound_url_v1(
                "https://api.provider.example/v1/item",
                &["api.provider.example"]
            ),
            Ok("api.provider.example".into())
        );
        assert!(validate_outbound_url_v1("https://127.0.0.1/admin", &["127.0.0.1"]).is_err());
        assert!(validate_outbound_ip_v1("10.0.0.1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("100.64.0.1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("198.18.0.1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("::ffff:127.0.0.1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("::7f00:1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("64:ff9b::7f00:1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("2002:7f00:1::".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("2001:db8::1".parse().expect("ip")).is_err());
        assert!(validate_outbound_ip_v1("8.8.8.8".parse().expect("ip")).is_ok());
        assert!(validate_outbound_ip_v1("2606:4700:4700::1111".parse().expect("ip")).is_ok());
    }

    #[test]
    fn errors_are_closed_and_redacted() {
        let error = ApiErrorV1::new(
            ApiErrorCodeV1::TemporarilyUnavailable,
            "trace-1",
            Some(1000),
        )
        .expect("error");
        let json = serde_json::to_string(&error).expect("json");
        assert!(!json.contains("provider"));
        assert!(!json.contains("secret"));
        assert!(!format!("{error:?}").contains("trace-1"));
        error.validate().expect("error contract");

        let envelope = ApiMutationEnvelopeV1 {
            content_length: 1,
            content_type: Some("secret-content-type".into()),
            idempotency_key: None,
            correlation_id: "trace-secret".into(),
        };
        assert!(!format!("{envelope:?}").contains("secret"));
        let request = DerivativeRequestV1 {
            schema_version: API_CONTRACT_VERSION_V1.into(),
            profile: "thumbnail_jpeg_v1".into(),
            source_version: 1,
            idempotency_key: "secret-idempotency-key".into(),
        };
        assert!(!format!("{request:?}").contains("secret"));
    }
}
