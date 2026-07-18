use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ApiKeyId, ApiKeyScope, ContractError, CorrelationId, DurationMillis, OrganizationRole,
    SecretDigest, SessionId, TenantId, TimestampMillis, UserId,
};

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum IdentityContractError {
    #[error("authentication secret is invalid")]
    InvalidSecret,
    #[error("browser origin is invalid")]
    InvalidOrigin,
    #[error("authentication policy is invalid")]
    InvalidPolicy,
    #[error("authentication state transition is invalid")]
    InvalidTransition,
    #[error("authentication digest key version is invalid")]
    InvalidKeyVersion,
}

macro_rules! identity_uuid {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(value: &str) -> Result<Self, ContractError> {
                let value =
                    Uuid::parse_str(value).map_err(|_| ContractError::InvalidIdentifier($kind))?;
                if value.is_nil() {
                    return Err(ContractError::InvalidIdentifier($kind));
                }
                Ok(Self(value))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = ContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

identity_uuid!(SessionFamilyId, "session family");
identity_uuid!(VerificationId, "verification");
identity_uuid!(AuthAuditEventId, "authentication audit event");
identity_uuid!(AuthDeliveryId, "authentication delivery");
identity_uuid!(OAuthFlowId, "OAuth flow");
identity_uuid!(PrincipalIssuanceGrantId, "principal issuance grant");
identity_uuid!(SessionMutationGrantId, "session mutation grant");
identity_uuid!(OAuthExchangeReservationId, "OAuth exchange reservation");
identity_uuid!(AuthDeliveryLeaseId, "authentication delivery lease");
identity_uuid!(IdentityProvisioningGrantId, "identity provisioning grant");

fn valid_url_safe_secret(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

macro_rules! redacted_secret {
    ($name:ident, $minimum:expr, $maximum:expr) => {
        #[derive(Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
                let value = value.into();
                if !valid_url_safe_secret(&value, $minimum, $maximum) {
                    return Err(IdentityContractError::InvalidSecret);
                }
                Ok(Self(value))
            }

            /// Exposes material only to a cryptographic or sealed-delivery boundary.
            #[must_use]
            pub fn expose_for_hashing(&self) -> &[u8] {
                self.0.as_bytes()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("[redacted]")
            }
        }
    };
}

redacted_secret!(OpaqueAuthToken, 32, 512);
redacted_secret!(CsrfToken, 32, 512);
redacted_secret!(ApiKeySecret, 32, 512);
redacted_secret!(OAuthState, 32, 512);
redacted_secret!(PkceVerifier, 43, 128);
redacted_secret!(AbuseSignal, 3, 512);
redacted_secret!(DeliveryDestinationRef, 8, 512);

/// Provider-controlled OAuth authorization code after exactly one query percent-decode.
///
/// RFC 6749 deliberately leaves code size undefined and permits visible ASCII. This parser does
/// not impose the entropy or URL-safe alphabet rules used by Frame-minted bearer values. The
/// callback adapter must preserve `+` as data rather than applying form-urlencoded space folding.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthAuthorizationCode(String);

impl OAuthAuthorizationCode {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 2_048
            || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        {
            return Err(IdentityContractError::InvalidSecret);
        }
        Ok(Self(value))
    }

    /// Exposes the opaque code only to the provider token-exchange boundary.
    #[must_use]
    pub fn expose_for_exchange(&self) -> &str {
        &self.0
    }

    /// Backward-compatible byte boundary for provider adapters.
    #[must_use]
    pub fn expose_for_hashing(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for OAuthAuthorizationCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuthAuthorizationCode([redacted])")
    }
}

impl fmt::Display for OAuthAuthorizationCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// RFC 7636 S256 challenge. It is kept redacted in generic formatting so authorization URLs do
/// not accidentally enter logs through enclosing request values.
#[derive(Clone, PartialEq, Eq)]
pub struct PkceChallenge(String);

impl PkceChallenge {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        if value.len() != 43
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(IdentityContractError::InvalidSecret);
        }
        Ok(Self(value))
    }

    /// Exposes the challenge only to the provider authorization-request boundary.
    #[must_use]
    pub fn expose_for_authorization(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PkceChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PkceChallenge([redacted])")
    }
}

impl fmt::Display for PkceChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// The only PKCE transformation accepted by the OAuth authorization boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PkceChallengeMethod {
    S256,
}

impl PkceChallengeMethod {
    #[must_use]
    pub const fn as_provider_value(self) -> &'static str {
        match self {
            Self::S256 => "S256",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VerificationDestination(String);

impl VerificationDestination {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        if !(3..=320).contains(&value.len())
            || !value.is_ascii()
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(IdentityContractError::InvalidSecret);
        }
        Ok(Self(value))
    }

    /// Exposes the destination only to an authenticated-encryption boundary.
    #[must_use]
    pub fn expose_for_sealing(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for VerificationDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerificationDestination([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OneTimeCode(String);

impl OneTimeCode {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        if !(6..=12).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(IdentityContractError::InvalidSecret);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_hashing(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for OneTimeCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OneTimeCode([redacted])")
    }
}

impl fmt::Display for OneTimeCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum VerificationSecret {
    MagicLink(OpaqueAuthToken),
    OneTimeCode(OneTimeCode),
}

impl VerificationSecret {
    #[must_use]
    pub fn expose_for_hashing(&self) -> &[u8] {
        match self {
            Self::MagicLink(value) => value.expose_for_hashing(),
            Self::OneTimeCode(value) => value.expose_for_hashing(),
        }
    }

    #[must_use]
    pub const fn channel(&self) -> VerificationChannel {
        match self {
            Self::MagicLink(_) => VerificationChannel::MagicLink,
            Self::OneTimeCode(_) => VerificationChannel::OneTimeCode,
        }
    }
}

impl fmt::Debug for VerificationSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerificationSecret([redacted])")
    }
}

impl fmt::Display for VerificationSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HashKeyVersion(u16);

impl HashKeyVersion {
    pub fn new(value: u16) -> Result<Self, IdentityContractError> {
        if value == 0 {
            return Err(IdentityContractError::InvalidKeyVersion);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VersionedSecretDigest {
    pub key_version: HashKeyVersion,
    pub digest: SecretDigest,
}

impl VersionedSecretDigest {
    #[must_use]
    pub const fn new(key_version: HashKeyVersion, digest: SecretDigest) -> Self {
        Self {
            key_version,
            digest,
        }
    }
}

impl fmt::Debug for VersionedSecretDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionedSecretDigest")
            .field("key_version", &self.key_version)
            .field("digest", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretDigestCandidates {
    active: VersionedSecretDigest,
    fallback: Vec<VersionedSecretDigest>,
}

impl SecretDigestCandidates {
    pub fn new(
        active: VersionedSecretDigest,
        fallback: Vec<VersionedSecretDigest>,
    ) -> Result<Self, IdentityContractError> {
        if fallback.len() > 4
            || fallback
                .iter()
                .any(|item| item.key_version == active.key_version)
            || fallback.iter().enumerate().any(|(index, item)| {
                fallback[..index]
                    .iter()
                    .any(|other| other.key_version == item.key_version)
            })
        {
            return Err(IdentityContractError::InvalidKeyVersion);
        }
        Ok(Self { active, fallback })
    }

    #[must_use]
    pub const fn active(&self) -> &VersionedSecretDigest {
        &self.active
    }

    pub fn iter(&self) -> impl Iterator<Item = &VersionedSecretDigest> {
        std::iter::once(&self.active).chain(self.fallback.iter())
    }
}

impl fmt::Debug for SecretDigestCandidates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretDigestCandidates")
            .field("active_version", &self.active.key_version)
            .field("fallback_count", &self.fallback.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthClientKind {
    Browser,
    Desktop,
    Mobile,
    Extension,
    Api,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRevocationReason {
    UserLogout,
    LogoutAll,
    ReplayDetected,
    Expired,
    SessionVersionChanged,
    AccountRecovery,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSessionState {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSessionDecision {
    Authenticated,
    Expired,
    Revoked,
    SessionVersionMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSessionRecord {
    pub id: SessionId,
    pub family_id: SessionFamilyId,
    pub user_id: UserId,
    pub client_kind: AuthClientKind,
    pub token_digest: VersionedSecretDigest,
    pub csrf_digest: Option<VersionedSecretDigest>,
    pub browser_origin: Option<ExactBrowserOrigin>,
    pub issued_at: TimestampMillis,
    pub rotated_at: TimestampMillis,
    pub idle_expires_at: TimestampMillis,
    pub absolute_expires_at: TimestampMillis,
    pub session_version: u64,
    pub generation: u64,
    pub state: AuthSessionState,
    pub revoked_at: Option<TimestampMillis>,
    pub revocation_reason: Option<SessionRevocationReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuthSession {
    pub id: SessionId,
    pub family_id: SessionFamilyId,
    pub user_id: UserId,
    pub client_kind: AuthClientKind,
    pub token_digest: VersionedSecretDigest,
    pub csrf_digest: Option<VersionedSecretDigest>,
    pub browser_origin: Option<ExactBrowserOrigin>,
    pub issued_at: TimestampMillis,
    pub idle_expires_at: TimestampMillis,
    pub absolute_expires_at: TimestampMillis,
    pub session_version: u64,
}

impl AuthSessionRecord {
    pub fn new(input: NewAuthSession) -> Result<Self, IdentityContractError> {
        if input.idle_expires_at <= input.issued_at
            || input.absolute_expires_at <= input.issued_at
            || input.idle_expires_at > input.absolute_expires_at
            || (input.client_kind == AuthClientKind::Browser
                && (input.csrf_digest.is_none() || input.browser_origin.is_none()))
            || (input.client_kind != AuthClientKind::Browser
                && (input.csrf_digest.is_some() || input.browser_origin.is_some()))
        {
            return Err(IdentityContractError::InvalidTransition);
        }
        Ok(Self {
            id: input.id,
            family_id: input.family_id,
            user_id: input.user_id,
            client_kind: input.client_kind,
            token_digest: input.token_digest,
            csrf_digest: input.csrf_digest,
            browser_origin: input.browser_origin,
            issued_at: input.issued_at,
            rotated_at: input.issued_at,
            idle_expires_at: input.idle_expires_at,
            absolute_expires_at: input.absolute_expires_at,
            session_version: input.session_version,
            generation: 0,
            state: AuthSessionState::Active,
            revoked_at: None,
            revocation_reason: None,
        })
    }

    #[must_use]
    pub fn evaluate(
        &self,
        now: TimestampMillis,
        current_session_version: u64,
    ) -> AuthSessionDecision {
        if self.state == AuthSessionState::Revoked {
            AuthSessionDecision::Revoked
        } else if now < self.issued_at
            || self.idle_expires_at <= now
            || self.absolute_expires_at <= now
        {
            AuthSessionDecision::Expired
        } else if self.session_version != current_session_version {
            AuthSessionDecision::SessionVersionMismatch
        } else {
            AuthSessionDecision::Authenticated
        }
    }

    pub fn rotate(
        &mut self,
        expected_generation: u64,
        token_digest: VersionedSecretDigest,
        csrf_digest: Option<VersionedSecretDigest>,
        now: TimestampMillis,
        idle_expires_at: TimestampMillis,
    ) -> Result<(), IdentityContractError> {
        if self.state != AuthSessionState::Active
            || self.generation != expected_generation
            || now < self.rotated_at
            || now >= self.absolute_expires_at
            || idle_expires_at <= now
            || idle_expires_at > self.absolute_expires_at
            || (self.client_kind == AuthClientKind::Browser && csrf_digest.is_none())
            || (self.client_kind != AuthClientKind::Browser && csrf_digest.is_some())
        {
            return Err(IdentityContractError::InvalidTransition);
        }
        self.token_digest = token_digest;
        self.csrf_digest = csrf_digest;
        self.rotated_at = now;
        self.idle_expires_at = idle_expires_at;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(IdentityContractError::InvalidTransition)?;
        Ok(())
    }

    pub fn revoke(&mut self, now: TimestampMillis, reason: SessionRevocationReason) {
        if self.state == AuthSessionState::Active {
            self.state = AuthSessionState::Revoked;
            self.revoked_at = Some(now);
            self.revocation_reason = Some(reason);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationPurpose {
    IdentityProvisioning,
    EmailVerify,
    SignIn,
    AccountRecovery,
    AccountLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationChannel {
    MagicLink,
    OneTimeCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    Pending,
    Consumed,
    Locked,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationDecision {
    Verified(UserId),
    Invalid,
    Expired,
    AttemptsExhausted,
    Replayed,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationChallenge {
    pub id: VerificationId,
    pub user_id: Option<UserId>,
    pub initiator: Option<SessionContinuationBinding>,
    pub provisioning_revision: Option<u64>,
    pub identifier_digest: VersionedSecretDigest,
    pub secret_digest: VersionedSecretDigest,
    pub purpose: VerificationPurpose,
    pub channel: VerificationChannel,
    pub attempt_count: u16,
    pub max_attempts: u16,
    pub created_at: TimestampMillis,
    pub expires_at: TimestampMillis,
    pub consumed_at: Option<TimestampMillis>,
    pub state: VerificationState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVerificationChallenge {
    pub user_id: Option<UserId>,
    pub initiator: Option<SessionContinuationBinding>,
    pub provisioning_revision: Option<u64>,
    pub identifier_digest: VersionedSecretDigest,
    pub secret_digest: VersionedSecretDigest,
    pub purpose: VerificationPurpose,
    pub channel: VerificationChannel,
    pub max_attempts: u16,
    pub created_at: TimestampMillis,
    pub expires_at: TimestampMillis,
}

impl VerificationChallenge {
    pub fn new(input: NewVerificationChallenge) -> Result<Self, IdentityContractError> {
        let initiator_valid = match input.purpose {
            VerificationPurpose::AccountLink => input.initiator.is_some(),
            VerificationPurpose::IdentityProvisioning
            | VerificationPurpose::EmailVerify
            | VerificationPurpose::SignIn
            | VerificationPurpose::AccountRecovery => input.initiator.is_none(),
        };
        let provisioning_valid = if input.purpose == VerificationPurpose::IdentityProvisioning {
            input
                .provisioning_revision
                .is_some_and(|revision| revision > 0)
        } else {
            input.provisioning_revision.is_none()
        };
        if input.max_attempts == 0
            || input.max_attempts > 100
            || input.expires_at <= input.created_at
            || !initiator_valid
            || !provisioning_valid
        {
            return Err(IdentityContractError::InvalidPolicy);
        }
        Ok(Self {
            id: VerificationId::new(),
            user_id: input.user_id,
            initiator: input.initiator,
            provisioning_revision: input.provisioning_revision,
            identifier_digest: input.identifier_digest,
            secret_digest: input.secret_digest,
            purpose: input.purpose,
            channel: input.channel,
            attempt_count: 0,
            max_attempts: input.max_attempts,
            created_at: input.created_at,
            expires_at: input.expires_at,
            consumed_at: None,
            state: VerificationState::Pending,
        })
    }

    pub fn attempt(&mut self, now: TimestampMillis, secret_matches: bool) -> VerificationDecision {
        match self.state {
            VerificationState::Consumed => return VerificationDecision::Replayed,
            VerificationState::Locked => return VerificationDecision::AttemptsExhausted,
            VerificationState::Expired => return VerificationDecision::Expired,
            VerificationState::Revoked => return VerificationDecision::Revoked,
            VerificationState::Pending => {}
        }
        if self.expires_at <= now {
            self.state = VerificationState::Expired;
            return VerificationDecision::Expired;
        }
        if secret_matches && now >= self.created_at {
            self.state = VerificationState::Consumed;
            self.consumed_at = Some(now);
            return self.user_id.map_or(
                VerificationDecision::Invalid,
                VerificationDecision::Verified,
            );
        }
        self.attempt_count = self.attempt_count.saturating_add(1);
        if self.attempt_count >= self.max_attempts {
            self.state = VerificationState::Locked;
            VerificationDecision::AttemptsExhausted
        } else {
            VerificationDecision::Invalid
        }
    }

    pub fn revoke(&mut self) {
        if self.state == VerificationState::Pending {
            self.state = VerificationState::Revoked;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAbuseAction {
    SessionIssue,
    IdentityProvisionIssue,
    SignInIssue,
    Verify,
    RecoverIssue,
    AccountLinkIssue,
    ApiKeyAuthenticate,
    OAuthBegin,
    OAuthExchange,
}

impl AuthAbuseAction {
    /// Verification delivery starts share one admission bucket even though
    /// their audit actions and challenge purposes remain distinct. Without
    /// this canonical bucket, signup, sign-in, and recovery can each consume
    /// the nominal global ceiling and overrun the bounded delivery dispatcher.
    pub const fn rate_limit_bucket_action(self) -> Self {
        match self {
            Self::IdentityProvisionIssue | Self::SignInIssue | Self::RecoverIssue => {
                Self::SignInIssue
            }
            action => action,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbuseDimension {
    Identifier,
    Source,
    Device,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AbuseBucketId {
    pub action: AuthAbuseAction,
    pub dimension: AbuseDimension,
    pub digest: Option<VersionedSecretDigest>,
}

impl AbuseBucketId {
    pub fn new(
        action: AuthAbuseAction,
        dimension: AbuseDimension,
        digest: Option<VersionedSecretDigest>,
    ) -> Result<Self, IdentityContractError> {
        let valid = match dimension {
            AbuseDimension::Global => digest.is_none(),
            AbuseDimension::Identifier | AbuseDimension::Source | AbuseDimension::Device => {
                digest.is_some()
            }
        };
        if !valid {
            return Err(IdentityContractError::InvalidPolicy);
        }
        Ok(Self {
            action,
            dimension,
            digest,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitPolicy {
    max_attempts: u32,
    window: DurationMillis,
    block_for: DurationMillis,
}

impl RateLimitPolicy {
    pub fn new(
        max_attempts: u32,
        window: DurationMillis,
        block_for: DurationMillis,
    ) -> Result<Self, IdentityContractError> {
        const MAX_RATE_LIMIT_DURATION_MILLIS: u64 = 24 * 60 * 60 * 1_000;
        if max_attempts == 0
            || max_attempts > 1_000_000
            || window.get() > MAX_RATE_LIMIT_DURATION_MILLIS
            || block_for.get() > MAX_RATE_LIMIT_DURATION_MILLIS
        {
            return Err(IdentityContractError::InvalidPolicy);
        }
        Ok(Self {
            max_attempts,
            window,
            block_for,
        })
    }

    #[must_use]
    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    #[must_use]
    pub const fn window(self) -> DurationMillis {
        self.window
    }

    #[must_use]
    pub const fn block_for(self) -> DurationMillis {
        self.block_for
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiRateLimitPolicy {
    pub identifier: RateLimitPolicy,
    pub source: RateLimitPolicy,
    pub device: RateLimitPolicy,
    pub global: RateLimitPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed,
    Limited { retry_at: TimestampMillis },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRateLimitBucket {
    pub id: AbuseBucketId,
    pub window_started_at: TimestampMillis,
    pub attempt_count: u32,
    pub blocked_until: Option<TimestampMillis>,
    pub updated_at: TimestampMillis,
}

impl AuthRateLimitBucket {
    #[must_use]
    pub fn new(id: AbuseBucketId, now: TimestampMillis) -> Self {
        Self {
            id,
            window_started_at: now,
            attempt_count: 0,
            blocked_until: None,
            updated_at: now,
        }
    }

    pub fn consume(
        &mut self,
        now: TimestampMillis,
        policy: RateLimitPolicy,
    ) -> Result<RateLimitDecision, IdentityContractError> {
        let now = now.max(self.updated_at);
        if let Some(blocked_until) = self.blocked_until {
            if blocked_until > now {
                self.updated_at = now;
                return Ok(RateLimitDecision::Limited {
                    retry_at: blocked_until,
                });
            }
            self.blocked_until = None;
            self.attempt_count = 0;
            self.window_started_at = now;
        }
        let window_end = self
            .window_started_at
            .checked_add(policy.window)
            .map_err(|_| IdentityContractError::InvalidPolicy)?;
        if window_end <= now {
            self.window_started_at = now;
            self.attempt_count = 0;
        }
        self.attempt_count = self.attempt_count.saturating_add(1);
        self.updated_at = now;
        if self.attempt_count > policy.max_attempts {
            let blocked_until = now
                .checked_add(policy.block_for)
                .map_err(|_| IdentityContractError::InvalidPolicy)?;
            self.blocked_until = Some(blocked_until);
            Ok(RateLimitDecision::Limited {
                retry_at: blocked_until,
            })
        } else {
            Ok(RateLimitDecision::Allowed)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantGrant {
    pub tenant_id: TenantId,
    pub role: OrganizationRole,
}

impl TenantGrant {
    #[must_use]
    pub const fn can_manage_api_keys(self) -> bool {
        matches!(self.role, OrganizationRole::Owner | OrganizationRole::Admin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalSnapshot {
    pub user_id: UserId,
    pub identity_revision: u64,
    pub tenant_grants: Vec<TenantGrant>,
}

impl PrincipalSnapshot {
    #[must_use]
    pub fn can_manage_api_keys(&self, tenant_id: TenantId) -> bool {
        self.tenant_grants
            .iter()
            .any(|grant| grant.tenant_id == tenant_id && grant.can_manage_api_keys())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedApiKeyRecord {
    pub id: ApiKeyId,
    pub owner_id: UserId,
    pub tenant_id: TenantId,
    pub key_digest: VersionedSecretDigest,
    pub scopes: Vec<ApiKeyScope>,
    pub created_at: TimestampMillis,
    pub expires_at: Option<TimestampMillis>,
    pub revoked_at: Option<TimestampMillis>,
}

impl ManagedApiKeyRecord {
    #[must_use]
    pub fn allows(&self, scope: ApiKeyScope, now: TimestampMillis) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|expires_at| expires_at > now)
            && self.scopes.contains(&scope)
    }

    pub fn revoke(&mut self, now: TimestampMillis) {
        self.revoked_at.get_or_insert(now);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    Google,
    Github,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthFlowPurpose {
    SignIn,
    AccountLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlowDecision {
    Accepted { initiated_by: Option<UserId> },
    Invalid,
    Expired,
    Replayed,
    Revoked,
}

/// Repository-revalidated binding for a continuation that began from an authenticated session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContinuationBinding {
    pub session_id: SessionId,
    pub user_id: UserId,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthFlowRecord {
    pub id: OAuthFlowId,
    pub provider: OAuthProvider,
    pub purpose: OAuthFlowPurpose,
    pub initiator: Option<SessionContinuationBinding>,
    pub state_digest: VersionedSecretDigest,
    pub pkce_digest: VersionedSecretDigest,
    pub redirect_digest: VersionedSecretDigest,
    pub audience_digest: VersionedSecretDigest,
    pub created_at: TimestampMillis,
    pub expires_at: TimestampMillis,
    pub consumed_at: Option<TimestampMillis>,
    pub revoked: bool,
}

impl OAuthFlowRecord {
    pub fn consume(
        &mut self,
        now: TimestampMillis,
        state_matches: bool,
        pkce_matches: bool,
        redirect_matches: bool,
        audience_matches: bool,
    ) -> OAuthFlowDecision {
        if self.revoked {
            return OAuthFlowDecision::Revoked;
        }
        if self.consumed_at.is_some() {
            return OAuthFlowDecision::Replayed;
        }
        if now < self.created_at || self.expires_at <= now {
            return OAuthFlowDecision::Expired;
        }
        if !(state_matches && pkce_matches && redirect_matches && audience_matches) {
            return OAuthFlowDecision::Invalid;
        }
        self.consumed_at = Some(now);
        OAuthFlowDecision::Accepted {
            initiated_by: self.initiator.map(|binding| binding.user_id),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedDeliveryEnvelope {
    pub id: AuthDeliveryId,
    payload: Vec<u8>,
    pub created_at: TimestampMillis,
}

impl SealedDeliveryEnvelope {
    pub fn new(
        payload: Vec<u8>,
        created_at: TimestampMillis,
    ) -> Result<Self, IdentityContractError> {
        if !(32..=65_536).contains(&payload.len()) {
            return Err(IdentityContractError::InvalidSecret);
        }
        Ok(Self {
            id: AuthDeliveryId::new(),
            payload,
            created_at,
        })
    }

    /// Exposes only sealed ciphertext to a trusted delivery dispatcher.
    #[must_use]
    pub fn sealed_payload(&self) -> &[u8] {
        &self.payload
    }
}

impl fmt::Debug for SealedDeliveryEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedDeliveryEnvelope")
            .field("id", &self.id)
            .field("payload", &"[sealed]")
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactBrowserOrigin(String);

impl ExactBrowserOrigin {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        let (scheme, authority) = value
            .split_once("://")
            .ok_or(IdentityContractError::InvalidOrigin)?;
        if authority.matches(':').count() > 1 {
            return Err(IdentityContractError::InvalidOrigin);
        }
        let (host, port) = authority
            .rsplit_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)));
        let valid_port = port.is_none_or(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        });
        let valid_host = !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
                    && label
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    && label
                        .as_bytes()
                        .last()
                        .is_some_and(u8::is_ascii_alphanumeric)
            });
        let local_http = scheme == "http" && matches!(host, "localhost" | "127.0.0.1");
        let valid_scheme = scheme == "https" || local_http;
        let valid_authority = !authority.is_empty()
            && authority.is_ascii()
            && !authority
                .bytes()
                .any(|byte| matches!(byte, b'/' | b'\\' | b'?' | b'#' | b'@' | b'%'))
            && authority
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
            && valid_host
            && valid_port;
        if value.len() > 255
            || !valid_scheme
            || !valid_authority
            || value != value.to_ascii_lowercase()
        {
            return Err(IdentityContractError::InvalidOrigin);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ExactBrowserOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExactBrowserOrigin")
            .field(&self.0)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactOAuthCallbackUrl(String);

impl ExactOAuthCallbackUrl {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        let scheme_end = value
            .find("://")
            .ok_or(IdentityContractError::InvalidOrigin)?
            + 3;
        let path_start = value[scheme_end..]
            .find('/')
            .map(|index| scheme_end + index)
            .ok_or(IdentityContractError::InvalidOrigin)?;
        let origin = ExactBrowserOrigin::parse(&value[..path_start])?;
        let path = &value[path_start..];
        let valid_path = path.len() > 1
            && path.len() <= 255
            && !path.contains("//")
            && !path
                .bytes()
                .any(|byte| matches!(byte, b'?' | b'#' | b'\\' | b'%'))
            && path.split('/').skip(1).all(|segment| {
                !segment.is_empty()
                    && !matches!(segment, "." | "..")
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            });
        if value.len() > 512 || !valid_path || origin.as_str().len() != path_start {
            return Err(IdentityContractError::InvalidOrigin);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ExactOAuthCallbackUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExactOAuthCallbackUrl")
            .field(&self.0)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OAuthAudience(String);

impl OAuthAudience {
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityContractError> {
        let value = value.into();
        if !(1..=255).contains(&value.len())
            || !value.is_ascii()
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(IdentityContractError::InvalidPolicy);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchSite {
    SameOrigin,
    SameSite,
    CrossSite,
    None,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSameSite {
    Strict,
    Lax,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOnlySessionCookie {
    name: String,
    path: String,
    secure: bool,
    http_only: bool,
    same_site: CookieSameSite,
    max_age: DurationMillis,
}

impl HostOnlySessionCookie {
    #[must_use]
    pub fn new(max_age: DurationMillis) -> Self {
        Self {
            name: "__Host-frame_session".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: CookieSameSite::Lax,
            max_age,
        }
    }

    #[must_use]
    pub const fn is_host_only(&self) -> bool {
        true
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn secure(&self) -> bool {
        self.secure
    }

    #[must_use]
    pub const fn http_only(&self) -> bool {
        self.http_only
    }

    #[must_use]
    pub const fn same_site(&self) -> CookieSameSite {
        self.same_site
    }

    #[must_use]
    pub const fn max_age(&self) -> DurationMillis {
        self.max_age
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAuditAction {
    SessionIssue,
    SessionAuthenticate,
    SessionRotate,
    BrowserMutationAuthenticate,
    Logout,
    LogoutAll,
    VerificationIssue,
    VerificationConsume,
    ApiKeyIssue,
    ApiKeyAuthenticate,
    ApiKeyRevoke,
    OAuthBegin,
    OAuthExchangePreflight,
    OAuthExchange,
    IdentityProvision,
    AccountLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAuditOutcome {
    Allow,
    Deny,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAuditReason {
    Issued,
    Authenticated,
    Rotated,
    LoggedOut,
    LoggedOutAll,
    VerificationAccepted,
    VerificationCompleted,
    InvalidCredential,
    Expired,
    Revoked,
    SessionVersionMismatch,
    ReplayDetected,
    CsrfMismatch,
    OriginMismatch,
    FetchMetadataMismatch,
    RateLimited,
    AttemptsExhausted,
    InsufficientRole,
    Linked,
    KeyVersionMigrated,
    AdapterFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthAuditEvent {
    pub id: AuthAuditEventId,
    pub correlation_id: CorrelationId,
    pub user_id: Option<UserId>,
    pub session_id: Option<SessionId>,
    pub client_kind: Option<AuthClientKind>,
    pub action: AuthAuditAction,
    pub outcome: AuthAuditOutcome,
    pub reason: AuthAuditReason,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuthAuditEvent {
    pub correlation_id: CorrelationId,
    pub user_id: Option<UserId>,
    pub session_id: Option<SessionId>,
    pub client_kind: Option<AuthClientKind>,
    pub action: AuthAuditAction,
    pub outcome: AuthAuditOutcome,
    pub reason: AuthAuditReason,
    pub occurred_at: TimestampMillis,
}

impl AuthAuditEvent {
    #[must_use]
    pub fn new(input: NewAuthAuditEvent) -> Self {
        Self {
            id: AuthAuditEventId::new(),
            correlation_id: input.correlation_id,
            user_id: input.user_id,
            session_id: input.session_id,
            client_kind: input.client_kind,
            action: input.action,
            outcome: input.outcome,
            reason: input.reason,
            occurred_at: input.occurred_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("valid time")
    }

    fn duration(value: u64) -> DurationMillis {
        DurationMillis::new(value).expect("valid duration")
    }

    fn digest(version: u16, value: char) -> VersionedSecretDigest {
        VersionedSecretDigest::new(
            HashKeyVersion::new(version).expect("version"),
            SecretDigest::parse_sha256(value.to_string().repeat(64)).expect("digest"),
        )
    }

    #[test]
    fn secrets_and_versioned_digests_are_strict_and_redacted() {
        let token = OpaqueAuthToken::parse("a".repeat(64)).expect("token");
        let csrf = CsrfToken::parse("b".repeat(64)).expect("csrf");
        let otp = OneTimeCode::parse("123456").expect("otp");
        let candidates =
            SecretDigestCandidates::new(digest(2, 'a'), vec![digest(1, 'b')]).expect("candidates");
        assert_eq!(format!("{token:?}"), "OpaqueAuthToken([redacted])");
        assert_eq!(format!("{csrf}"), "[redacted]");
        assert_eq!(format!("{otp:?}"), "OneTimeCode([redacted])");
        assert!(!format!("{candidates:?}").contains(&"a".repeat(64)));
        assert!(SecretDigestCandidates::new(digest(2, 'a'), vec![digest(2, 'b')]).is_err());
        assert!(HashKeyVersion::new(0).is_err());
    }

    #[test]
    fn oauth_authorization_codes_accept_provider_grammar_without_leaking() {
        let google_style = "4/P7q7W91a-oMsCeLvIaQm6bTrgtp7";
        let parsed = OAuthAuthorizationCode::parse(google_style).expect("Google-style code");
        assert_eq!(parsed.expose_for_exchange(), google_style);
        assert_eq!(format!("{parsed:?}"), "OAuthAuthorizationCode([redacted])");
        assert_eq!(format!("{parsed}"), "[redacted]");
        assert!(OAuthAuthorizationCode::parse("x").is_ok());
        assert!(OAuthAuthorizationCode::parse("short/+=_").is_ok());
        assert!(OAuthAuthorizationCode::parse("").is_err());
        assert!(OAuthAuthorizationCode::parse("contains\ncontrol").is_err());
        assert!(OAuthAuthorizationCode::parse("non-ascii-λ").is_err());
        assert!(OAuthAuthorizationCode::parse("a".repeat(2_049)).is_err());
    }

    #[test]
    fn session_rotation_is_generation_and_absolute_expiry_fenced() {
        let mut session = AuthSessionRecord::new(NewAuthSession {
            id: SessionId::new(),
            family_id: SessionFamilyId::new(),
            user_id: UserId::new(),
            client_kind: AuthClientKind::Browser,
            token_digest: digest(1, 'a'),
            csrf_digest: Some(digest(1, 'b')),
            browser_origin: Some(
                ExactBrowserOrigin::parse("https://frame.engmanager.xyz").expect("origin"),
            ),
            issued_at: time(10),
            idle_expires_at: time(20),
            absolute_expires_at: time(40),
            session_version: 3,
        })
        .expect("session");
        session
            .rotate(0, digest(2, 'c'), Some(digest(2, 'd')), time(15), time(30))
            .expect("rotate");
        assert_eq!(session.generation, 1);
        assert!(
            session
                .rotate(0, digest(2, 'e'), Some(digest(2, 'f')), time(16), time(31))
                .is_err()
        );
    }

    #[test]
    fn verification_decoys_never_mint_a_principal_and_links_require_an_initiator() {
        let mut decoy = VerificationChallenge::new(NewVerificationChallenge {
            user_id: None,
            initiator: None,
            provisioning_revision: None,
            identifier_digest: digest(2, 'a'),
            secret_digest: digest(2, 'b'),
            purpose: VerificationPurpose::SignIn,
            channel: VerificationChannel::OneTimeCode,
            max_attempts: 2,
            created_at: time(1),
            expires_at: time(10),
        })
        .expect("decoy");
        assert_eq!(decoy.attempt(time(2), true), VerificationDecision::Invalid);
        assert!(
            VerificationChallenge::new(NewVerificationChallenge {
                user_id: None,
                initiator: None,
                provisioning_revision: None,
                identifier_digest: digest(2, 'c'),
                secret_digest: digest(2, 'd'),
                purpose: VerificationPurpose::AccountLink,
                channel: VerificationChannel::MagicLink,
                max_attempts: 2,
                created_at: time(1),
                expires_at: time(10),
            })
            .is_err()
        );
    }

    #[test]
    fn rate_windows_are_exact_multidimensional_and_clock_rollback_safe() {
        let policy = RateLimitPolicy::new(2, duration(10), duration(20)).expect("policy");
        let id = AbuseBucketId::new(
            AuthAbuseAction::Verify,
            AbuseDimension::Source,
            Some(digest(2, 'a')),
        )
        .expect("bucket");
        let mut bucket = AuthRateLimitBucket::new(id, time(0));
        assert_eq!(
            bucket.consume(time(1), policy),
            Ok(RateLimitDecision::Allowed)
        );
        assert_eq!(
            bucket.consume(time(2), policy),
            Ok(RateLimitDecision::Allowed)
        );
        assert_eq!(
            bucket.consume(time(3), policy),
            Ok(RateLimitDecision::Limited { retry_at: time(23) })
        );
        assert_eq!(
            bucket.consume(time(1), policy),
            Ok(RateLimitDecision::Limited { retry_at: time(23) })
        );
        assert!(
            AbuseBucketId::new(
                AuthAbuseAction::Verify,
                AbuseDimension::Global,
                Some(digest(1, 'b'))
            )
            .is_err()
        );
        assert_eq!(
            AuthAbuseAction::IdentityProvisionIssue.rate_limit_bucket_action(),
            AuthAbuseAction::SignInIssue
        );
        assert_eq!(
            AuthAbuseAction::RecoverIssue.rate_limit_bucket_action(),
            AuthAbuseAction::SignInIssue
        );
        assert_eq!(
            AuthAbuseAction::AccountLinkIssue.rate_limit_bucket_action(),
            AuthAbuseAction::AccountLinkIssue
        );
    }

    #[test]
    fn exact_origins_cookie_and_tenant_grants_are_fail_closed() {
        let origin = ExactBrowserOrigin::parse("https://frame.engmanager.xyz").expect("origin");
        let attacker =
            ExactBrowserOrigin::parse("https://frame.engmanager.xyz.evil.test").expect("origin");
        assert_ne!(origin, attacker);
        for invalid in [
            "https://frame.engmanager.xyz@evil.test",
            "https://frame.engmanager.xyz/%2f%2fevil.test",
            "https://frame.engmanager.xyz\\@evil.test",
            "//frame.engmanager.xyz",
            "http://frame.engmanager.xyz",
            "http://localhost:evil",
            "https://frame.engmanager.xyz:0",
            "https://frame.engmanager.xyz:99999",
        ] {
            assert!(ExactBrowserOrigin::parse(invalid).is_err(), "{invalid}");
        }
        let callback = ExactOAuthCallbackUrl::parse("https://frame.engmanager.xyz/auth/callback")
            .expect("callback");
        assert_eq!(
            callback.as_str(),
            "https://frame.engmanager.xyz/auth/callback"
        );
        for invalid in [
            "https://frame.engmanager.xyz",
            "https://frame.engmanager.xyz/",
            "https://frame.engmanager.xyz@evil.test/auth/callback",
            "https://frame.engmanager.xyz/auth//callback",
            "https://frame.engmanager.xyz/auth/callback?next=https://evil.test",
            "https://frame.engmanager.xyz/auth/callback#evil",
            "https://frame.engmanager.xyz/%2e%2e/evil",
            "http://frame.engmanager.xyz/auth/callback",
        ] {
            assert!(ExactOAuthCallbackUrl::parse(invalid).is_err(), "{invalid}");
        }
        assert!(OAuthAudience::parse("frame-web/evil").is_err());
        let cookie = HostOnlySessionCookie::new(duration(60));
        assert!(cookie.is_host_only() && cookie.secure() && cookie.http_only());
        assert_eq!(cookie.name(), "__Host-frame_session");
        let tenant = TenantId::new();
        let principal = PrincipalSnapshot {
            user_id: UserId::new(),
            identity_revision: 1,
            tenant_grants: vec![TenantGrant {
                tenant_id: tenant,
                role: OrganizationRole::Viewer,
            }],
        };
        assert!(!principal.can_manage_api_keys(tenant));
    }

    #[test]
    fn oauth_flow_is_pkce_audience_redirect_bound_and_single_use() {
        let mut flow = OAuthFlowRecord {
            id: OAuthFlowId::new(),
            provider: OAuthProvider::Github,
            purpose: OAuthFlowPurpose::SignIn,
            initiator: None,
            state_digest: digest(2, 'a'),
            pkce_digest: digest(2, 'b'),
            redirect_digest: digest(2, 'c'),
            audience_digest: digest(2, 'd'),
            created_at: time(1),
            expires_at: time(10),
            consumed_at: None,
            revoked: false,
        };
        assert_eq!(
            flow.consume(time(2), true, false, true, true),
            OAuthFlowDecision::Invalid
        );
        assert!(matches!(
            flow.consume(time(3), true, true, true, true),
            OAuthFlowDecision::Accepted { .. }
        ));
        assert_eq!(
            flow.consume(time(4), true, true, true, true),
            OAuthFlowDecision::Replayed
        );
    }

    #[test]
    fn sealed_delivery_never_formats_ciphertext() {
        let envelope = SealedDeliveryEnvelope::new(vec![b'x'; 64], time(1)).expect("envelope");
        assert_eq!(envelope.sealed_payload().len(), 64);
        assert!(!format!("{envelope:?}").contains(&"x".repeat(32)));
    }
}
