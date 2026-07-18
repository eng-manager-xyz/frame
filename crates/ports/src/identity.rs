use std::{
    collections::HashMap,
    fmt,
    sync::{
        RwLock,
        atomic::{AtomicI64, Ordering},
    },
};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU64};

use async_trait::async_trait;
use frame_domain::{
    AbuseBucketId, AbuseDimension, ApiKeyId, ApiKeyScope, ApiKeySecret, AuthAbuseAction,
    AuthAuditAction, AuthAuditEvent, AuthAuditOutcome, AuthAuditReason, AuthClientKind,
    AuthDeliveryId, AuthDeliveryLeaseId, AuthRateLimitBucket, AuthSessionDecision,
    AuthSessionRecord, AuthSessionState, CsrfToken, DeliveryDestinationRef, DurationMillis,
    ExactBrowserOrigin, ExactOAuthCallbackUrl, FetchSite, IdentityProvisioningGrantId,
    MAX_TIMESTAMP_MS, ManagedApiKeyRecord, MultiRateLimitPolicy, NewAuthAuditEvent,
    NewVerificationChallenge, OAuthAudience, OAuthAuthorizationCode, OAuthExchangeReservationId,
    OAuthFlowRecord, OAuthProvider, OAuthState, OpaqueAuthToken, PkceVerifier,
    PrincipalIssuanceGrantId, PrincipalSnapshot, RateLimitDecision, SealedDeliveryEnvelope,
    SecretDigestCandidates, SessionContinuationBinding, SessionFamilyId, SessionId,
    SessionMutationGrantId, SessionRevocationReason, TenantId, TimestampMillis, UserId,
    VerificationChallenge, VerificationChannel, VerificationDecision, VerificationDestination,
    VerificationId, VerificationPurpose, VerificationSecret, VerificationState,
    VersionedSecretDigest,
};

use crate::PortError;

pub trait Clock: Send + Sync {
    fn now(&self) -> Result<TimestampMillis, PortError>;
}

#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicI64,
}

impl ManualClock {
    pub fn new(now: TimestampMillis) -> Self {
        Self {
            now_ms: AtomicI64::new(now.get()),
        }
    }

    pub fn set(&self, now: TimestampMillis) {
        self.now_ms.store(now.get(), Ordering::SeqCst);
    }

    pub fn advance(&self, milliseconds: i64) -> Result<TimestampMillis, PortError> {
        let previous = self
            .now_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value
                    .checked_add(milliseconds)
                    .filter(|next| (0..=MAX_TIMESTAMP_MS).contains(next))
            })
            .map_err(|_| PortError::InvalidRequest("clock advance is outside range".into()))?;
        TimestampMillis::new(previous + milliseconds)
            .map_err(|error| PortError::InvalidRequest(error.to_string()))
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Result<TimestampMillis, PortError> {
        TimestampMillis::new(self.now_ms.load(Ordering::SeqCst))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }
}

/// Production implementations must draw every value from a CSPRNG.
pub trait AuthSecretSource: Send + Sync {
    fn session_token(&self) -> Result<OpaqueAuthToken, PortError>;
    fn csrf_token(&self) -> Result<CsrfToken, PortError>;
    fn api_key(&self) -> Result<ApiKeySecret, PortError>;
    fn oauth_state(&self) -> Result<OAuthState, PortError>;
    fn pkce_verifier(&self) -> Result<PkceVerifier, PortError>;
    fn verification_secret(
        &self,
        channel: VerificationChannel,
    ) -> Result<VerificationSecret, PortError>;
}

/// Deterministic material for conformance tests only. Never wire this into a deployed runtime.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct DeterministicAuthSecretSource {
    sequence: AtomicU64,
}

#[cfg(test)]
impl DeterministicAuthSecretSource {
    fn next(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn opaque(prefix: char, value: u64) -> String {
        format!("{prefix}{value:063}")
    }
}

#[cfg(test)]
impl AuthSecretSource for DeterministicAuthSecretSource {
    fn session_token(&self) -> Result<OpaqueAuthToken, PortError> {
        OpaqueAuthToken::parse(Self::opaque('s', self.next()))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn csrf_token(&self) -> Result<CsrfToken, PortError> {
        CsrfToken::parse(Self::opaque('c', self.next()))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn api_key(&self) -> Result<ApiKeySecret, PortError> {
        ApiKeySecret::parse(Self::opaque('k', self.next()))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn oauth_state(&self) -> Result<OAuthState, PortError> {
        OAuthState::parse(Self::opaque('o', self.next()))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn pkce_verifier(&self) -> Result<PkceVerifier, PortError> {
        PkceVerifier::parse(Self::opaque('p', self.next()))
            .map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn verification_secret(
        &self,
        channel: VerificationChannel,
    ) -> Result<VerificationSecret, PortError> {
        let next = self.next();
        match channel {
            VerificationChannel::MagicLink => OpaqueAuthToken::parse(Self::opaque('m', next))
                .map(VerificationSecret::MagicLink)
                .map_err(|error| PortError::Adapter(error.to_string())),
            VerificationChannel::OneTimeCode => {
                frame_domain::OneTimeCode::parse(format!("{:06}", next % 1_000_000))
                    .map(VerificationSecret::OneTimeCode)
                    .map_err(|error| PortError::Adapter(error.to_string()))
            }
        }
    }
}

#[derive(Clone)]
pub struct VerificationDeliveryMaterial {
    pub destination: VerificationDestination,
    pub secret: VerificationSecret,
    pub purpose: VerificationPurpose,
    pub expires_at: TimestampMillis,
}

impl fmt::Debug for VerificationDeliveryMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerificationDeliveryMaterial")
            .field("destination", &"[redacted]")
            .field("secret", &"[redacted]")
            .field("purpose", &self.purpose)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Seals destination and one-time material before either can enter durable state.
///
/// Production adapters must use authenticated encryption, bind the envelope to its purpose and
/// expiry, produce fixed-size padded ciphertext independent of destination/secret shape, and
/// perform equivalent cryptographic work for real and suppressed deliveries. They must never log
/// either plaintext input. The deterministic implementation below is test-only.
pub trait AuthDeliverySealer: Send + Sync {
    fn seal(
        &self,
        material: &VerificationDeliveryMaterial,
        now: TimestampMillis,
    ) -> Result<SealedDeliveryEnvelope, PortError>;
}

/// Dispatches only ciphertext claimed from the transactional outbox.
#[async_trait]
pub trait AuthDeliverySink: Send + Sync {
    async fn deliver(&self, envelope: &SealedDeliveryEnvelope) -> Result<(), PortError>;
}

/// Maximum dispatcher attempts before an outbox entry is removed as exhausted.
pub const MAX_AUTH_DELIVERY_ATTEMPTS: u16 = 12;
/// Prevents a crashed dispatcher from making authentication delivery unavailable indefinitely.
pub const MAX_AUTH_DELIVERY_LEASE_MILLIS: u64 = 15 * 60 * 1_000;
/// Hard storage bound in addition to configurable OAuth abuse limits.
pub const MAX_PENDING_OAUTH_FLOWS: usize = 4_096;
/// Hard bound preventing arbitrary identifier/source/device cardinality from exhausting storage.
pub const MAX_RATE_LIMIT_BUCKETS: usize = 4_096;

/// A fenced lease over one durable authentication-delivery outbox entry.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthDeliveryClaim {
    lease_id: AuthDeliveryLeaseId,
    envelope: SealedDeliveryEnvelope,
    lease_expires_at: TimestampMillis,
    attempt: u16,
}

impl AuthDeliveryClaim {
    /// Constructs a claim returned by a trusted repository implementation. A completion using
    /// this value succeeds only while the matching lease remains current in repository state.
    #[must_use]
    pub const fn from_repository(
        lease_id: AuthDeliveryLeaseId,
        envelope: SealedDeliveryEnvelope,
        lease_expires_at: TimestampMillis,
        attempt: u16,
    ) -> Self {
        Self {
            lease_id,
            envelope,
            lease_expires_at,
            attempt,
        }
    }

    #[must_use]
    pub const fn delivery_id(&self) -> AuthDeliveryId {
        self.envelope.id
    }

    #[must_use]
    pub const fn lease_id(&self) -> AuthDeliveryLeaseId {
        self.lease_id
    }

    #[must_use]
    pub const fn envelope(&self) -> &SealedDeliveryEnvelope {
        &self.envelope
    }

    #[must_use]
    pub const fn lease_expires_at(&self) -> TimestampMillis {
        self.lease_expires_at
    }

    #[must_use]
    pub const fn attempt(&self) -> u16 {
        self.attempt
    }
}

impl fmt::Debug for AuthDeliveryClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthDeliveryClaim")
            .field("delivery_id", &self.envelope.id)
            .field("lease_id", &self.lease_id)
            .field("lease_expires_at", &self.lease_expires_at)
            .field("attempt", &self.attempt)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDeliveryAcknowledgeOutcome {
    Acknowledged,
    StaleLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDeliveryRetryOutcome {
    Scheduled,
    Exhausted,
    StaleLease,
}

/// Test-only sealer that deliberately does not encode raw material.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct DeterministicDeliverySealer;

#[cfg(test)]
impl AuthDeliverySealer for DeterministicDeliverySealer {
    fn seal(
        &self,
        material: &VerificationDeliveryMaterial,
        now: TimestampMillis,
    ) -> Result<SealedDeliveryEnvelope, PortError> {
        let marker = match material.purpose {
            VerificationPurpose::IdentityProvisioning => b'p',
            VerificationPurpose::SignIn => b'i',
            VerificationPurpose::AccountRecovery => b'r',
            VerificationPurpose::EmailVerify => b'e',
            VerificationPurpose::AccountLink => b'l',
        };
        SealedDeliveryEnvelope::new(vec![marker; 64], now)
            .map_err(|error| PortError::Adapter(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionAudit {
    pub correlation_id: frame_domain::CorrelationId,
    pub action: AuthAuditAction,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbuseDigestSet {
    pub identifier: SecretDigestCandidates,
    pub source: SecretDigestCandidates,
    pub device: SecretDigestCandidates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAuditContext {
    pub session_id: SessionId,
    pub user_id: UserId,
    pub client_kind: AuthClientKind,
}

impl From<&AuthSessionRecord> for SessionAuditContext {
    fn from(session: &AuthSessionRecord) -> Self {
        Self {
            session_id: session.id,
            user_id: session.user_id,
            client_kind: session.client_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMutationGrant {
    id: SessionMutationGrantId,
    session_id: SessionId,
    user_id: UserId,
    generation: u64,
    token_digest: VersionedSecretDigest,
}

impl SessionMutationGrant {
    /// Constructs an opaque grant returned by a trusted repository implementation. The grant is
    /// usable only when its unpredictable identifier also exists in that repository's state.
    #[must_use]
    pub const fn from_repository(
        id: SessionMutationGrantId,
        session_id: SessionId,
        user_id: UserId,
        generation: u64,
        token_digest: VersionedSecretDigest,
    ) -> Self {
        Self {
            id,
            session_id,
            user_id,
            generation,
            token_digest,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub const fn id(&self) -> SessionMutationGrantId {
        self.id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn token_digest(&self) -> &VersionedSecretDigest {
        &self.token_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedSessionPresentation {
    session_id: SessionId,
    client_kind: AuthClientKind,
    principal: PrincipalSnapshot,
    mutation_grant: Option<SessionMutationGrant>,
}

impl AuthenticatedSessionPresentation {
    #[must_use]
    pub fn from_repository(
        session_id: SessionId,
        client_kind: AuthClientKind,
        principal: PrincipalSnapshot,
        mutation_grant: Option<SessionMutationGrant>,
    ) -> Self {
        Self {
            session_id,
            client_kind,
            principal,
            mutation_grant,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn client_kind(&self) -> AuthClientKind {
        self.client_kind
    }

    #[must_use]
    pub const fn principal(&self) -> &PrincipalSnapshot {
        &self.principal
    }

    #[must_use]
    pub fn into_parts(self) -> (PrincipalSnapshot, Option<SessionMutationGrant>) {
        (self.principal, self.mutation_grant)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPresentation {
    Authenticated(AuthenticatedSessionPresentation),
    Expired(SessionAuditContext),
    Revoked(SessionAuditContext),
    SessionVersionMismatch(SessionAuditContext),
    ReplayFamilyRevoked(SessionAuditContext),
    BoundaryRejected(SessionAuditContext, AuthAuditReason),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserBoundaryRequest {
    pub origin: Option<ExactBrowserOrigin>,
    pub fetch_site: FetchSite,
    pub csrf_cookie_digests: SecretDigestCandidates,
    pub csrf_header_digests: SecretDigestCandidates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAuthenticationCommand {
    pub token_digests: SecretDigestCandidates,
    pub browser_boundary: Option<BrowserBoundaryRequest>,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIssueCommand {
    pub principal: PrincipalSnapshot,
    pub authority: SessionIssueAuthority,
    pub session: AuthSessionRecord,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIssueAuthority {
    Verified(PrincipalIssuanceGrant),
    ExistingSession(SessionMutationGrant),
}

/// Opaque, one-time authority minted by an atomic verification or OAuth decision.
#[derive(Clone, PartialEq, Eq)]
pub struct PrincipalIssuanceGrant {
    id: PrincipalIssuanceGrantId,
    user_id: UserId,
    identity_revision: u64,
    expires_at: TimestampMillis,
}

impl PrincipalIssuanceGrant {
    /// Constructs a grant minted by a trusted repository implementation. It is usable only when
    /// the unpredictable identifier also exists in that repository's transactional state.
    #[must_use]
    pub const fn from_repository(
        id: PrincipalIssuanceGrantId,
        user_id: UserId,
        identity_revision: u64,
        expires_at: TimestampMillis,
    ) -> Self {
        Self {
            id,
            user_id,
            identity_revision,
            expires_at,
        }
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub const fn id(&self) -> PrincipalIssuanceGrantId {
        self.id
    }

    #[must_use]
    pub const fn identity_revision(&self) -> u64 {
        self.identity_revision
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }
}

impl fmt::Debug for PrincipalIssuanceGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrincipalIssuanceGrant")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIssueOutcome {
    Issued,
    Denied(AuthAuditReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRotationRequest {
    pub grant: SessionMutationGrant,
    pub next_token_digest: VersionedSecretDigest,
    pub next_csrf_digest: VersionedSecretDigest,
    pub now: TimestampMillis,
    pub idle_expires_at: TimestampMillis,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRotationOutcome {
    Rotated(Box<AuthSessionRecord>),
    Denied(AuthAuditReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRevokeCommand {
    pub grant: SessionMutationGrant,
    pub reason: SessionRevocationReason,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoutAllOutcome {
    Revoked {
        new_session_version: u64,
        revoked_sessions: u64,
    },
    Denied(AuthAuditReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationIssueCommand {
    pub identifier_digests: SecretDigestCandidates,
    pub secret_digest: VersionedSecretDigest,
    pub purpose: VerificationPurpose,
    pub channel: VerificationChannel,
    pub initiated_by: Option<PrincipalSnapshot>,
    pub initiator_grant: Option<SessionMutationGrant>,
    pub provisioning: Option<IdentityProvisioningIntent>,
    pub max_attempts: u16,
    pub expires_at: TimestampMillis,
    pub sealed_delivery: SealedDeliveryEnvelope,
    pub abuse: AbuseDigestSet,
    pub rate_policy: MultiRateLimitPolicy,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityProvisioningIntent {
    pub user_id: UserId,
    pub identity_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationIssueAtomicOutcome {
    Accepted,
    RateLimited { retry_at: TimestampMillis },
    Rejected(AuthAuditReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationAttemptCommand {
    pub identifier_digests: SecretDigestCandidates,
    pub secret_digests: SecretDigestCandidates,
    pub purpose: VerificationPurpose,
    pub abuse: AbuseDigestSet,
    pub rate_policy: MultiRateLimitPolicy,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationAtomicOutcome {
    Verified {
        principal: PrincipalSnapshot,
        issuance_grant: PrincipalIssuanceGrant,
    },
    ProvisioningAuthorized(IdentityProvisioningGrant),
    Linked {
        user_id: UserId,
    },
    Rejected(AuthAuditReason),
    RateLimited {
        retry_at: TimestampMillis,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyIssueCommand {
    pub principal: PrincipalSnapshot,
    pub grant: SessionMutationGrant,
    pub record: ManagedApiKeyRecord,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyIssueOutcome {
    Issued,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyAuthenticationCommand {
    pub key_digests: SecretDigestCandidates,
    pub tenant_id: TenantId,
    pub required_scope: ApiKeyScope,
    pub abuse: AbuseDigestSet,
    pub rate_policy: MultiRateLimitPolicy,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyAuthenticationOutcome {
    Authenticated(PrincipalSnapshot),
    Rejected(AuthAuditReason),
    RateLimited { retry_at: TimestampMillis },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRevokeCommand {
    pub grant: SessionMutationGrant,
    pub key_id: ApiKeyId,
    pub audit: DecisionAudit,
}

#[derive(Clone)]
pub struct OAuthCallback {
    pub provider: OAuthProvider,
    pub code: OAuthAuthorizationCode,
}

impl fmt::Debug for OAuthCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthCallback")
            .field("provider", &self.provider)
            .field("code", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentityAssertion {
    pub provider: OAuthProvider,
    pub subject_digests: SecretDigestCandidates,
    /// Present only when the provider adapter verified ownership/verification of this identifier.
    pub verified_identifier_digests: Option<SecretDigestCandidates>,
}

#[derive(Debug, Clone, Copy)]
pub struct OAuthProviderExchange<'a> {
    pub callback: &'a OAuthCallback,
    pub pkce_verifier: &'a PkceVerifier,
    pub callback_url: &'a ExactOAuthCallbackUrl,
    pub audience: &'a OAuthAudience,
}

/// Provider adapters submit code, PKCE verifier, and the server-configured redirect/client
/// binding. `Ok(None)` is a provider-authentication rejection; `Err` is adapter unavailability.
#[async_trait]
pub trait OAuthIdentityVerifier: Send + Sync {
    async fn verify(
        &self,
        exchange: OAuthProviderExchange<'_>,
    ) -> Result<Option<ExternalIdentityAssertion>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthBeginCommand {
    pub flow: OAuthFlowRecord,
    pub initiator: Option<SessionMutationGrant>,
    pub abuse: AbuseDigestSet,
    pub rate_policy: MultiRateLimitPolicy,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthBeginOutcome {
    Started,
    RateLimited { retry_at: TimestampMillis },
    Rejected(AuthAuditReason),
}

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthExchangeReservation {
    id: OAuthExchangeReservationId,
    flow_id: frame_domain::OAuthFlowId,
    provider: OAuthProvider,
    initiator: Option<SessionContinuationBinding>,
    expires_at: TimestampMillis,
}

impl OAuthExchangeReservation {
    #[must_use]
    pub const fn from_repository(
        id: OAuthExchangeReservationId,
        flow_id: frame_domain::OAuthFlowId,
        provider: OAuthProvider,
        initiator: Option<SessionContinuationBinding>,
        expires_at: TimestampMillis,
    ) -> Self {
        Self {
            id,
            flow_id,
            provider,
            initiator,
            expires_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> OAuthExchangeReservationId {
        self.id
    }

    #[must_use]
    pub const fn flow_id(&self) -> frame_domain::OAuthFlowId {
        self.flow_id
    }

    #[must_use]
    pub const fn provider(&self) -> OAuthProvider {
        self.provider
    }

    #[must_use]
    pub const fn initiator(&self) -> Option<SessionContinuationBinding> {
        self.initiator
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }
}

impl fmt::Debug for OAuthExchangeReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthExchangeReservation")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthPreflightCommand {
    pub provider: OAuthProvider,
    pub state_digests: SecretDigestCandidates,
    pub pkce_digests: SecretDigestCandidates,
    pub redirect_digests: SecretDigestCandidates,
    pub audience_digests: SecretDigestCandidates,
    pub abuse: AbuseDigestSet,
    pub rate_policy: MultiRateLimitPolicy,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthPreflightOutcome {
    Ready(OAuthExchangeReservation),
    Rejected(AuthAuditReason),
    RateLimited { retry_at: TimestampMillis },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthFinalizeCommand {
    pub reservation: OAuthExchangeReservation,
    pub provider_result: OAuthProviderResult,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthProviderResult {
    Verified(ExternalIdentityAssertion),
    Rejected,
    AdapterFailure,
}

/// One-time authority produced only after an unowned identifier is verified for signup.
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityProvisioningGrant {
    id: IdentityProvisioningGrantId,
    user_id: UserId,
    identity_revision: u64,
    identifier_digest: VersionedSecretDigest,
    expires_at: TimestampMillis,
}

impl IdentityProvisioningGrant {
    #[must_use]
    pub const fn from_repository(
        id: IdentityProvisioningGrantId,
        user_id: UserId,
        identity_revision: u64,
        identifier_digest: VersionedSecretDigest,
        expires_at: TimestampMillis,
    ) -> Self {
        Self {
            id,
            user_id,
            identity_revision,
            identifier_digest,
            expires_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> IdentityProvisioningGrantId {
        self.id
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub const fn identity_revision(&self) -> u64 {
        self.identity_revision
    }

    #[must_use]
    pub const fn identifier_digest(&self) -> &VersionedSecretDigest {
        &self.identifier_digest
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }
}

impl fmt::Debug for IdentityProvisioningGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityProvisioningGrant")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("identity_revision", &self.identity_revision)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityProvisionCommand {
    pub grant: IdentityProvisioningGrant,
    pub destination: DeliveryDestinationRef,
    pub audit: DecisionAudit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityProvisionOutcome {
    Created,
    Rejected(AuthAuditReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthExchangeOutcome {
    Verified {
        principal: PrincipalSnapshot,
        issuance_grant: PrincipalIssuanceGrant,
    },
    Linked {
        user_id: UserId,
    },
    Rejected(AuthAuditReason),
}

/// Every mutating method and every authentication decision must commit its audit row in the
/// same adapter transaction. Returning `Ok` without the audit row violates this port contract.
#[async_trait]
pub trait AuthStateRepository: Send + Sync {
    async fn provision_identity(
        &self,
        command: IdentityProvisionCommand,
    ) -> Result<IdentityProvisionOutcome, PortError>;
    async fn issue_auth_session(
        &self,
        command: SessionIssueCommand,
    ) -> Result<SessionIssueOutcome, PortError>;
    async fn authenticate_session(
        &self,
        command: SessionAuthenticationCommand,
    ) -> Result<SessionPresentation, PortError>;
    async fn rotate_auth_session(
        &self,
        request: SessionRotationRequest,
    ) -> Result<SessionRotationOutcome, PortError>;
    async fn revoke_auth_session(&self, command: SessionRevokeCommand) -> Result<bool, PortError>;
    async fn revoke_all_auth_sessions(
        &self,
        command: SessionRevokeCommand,
    ) -> Result<LogoutAllOutcome, PortError>;
    async fn issue_verification(
        &self,
        command: VerificationIssueCommand,
    ) -> Result<VerificationIssueAtomicOutcome, PortError>;
    async fn materialize_verification_deliveries(
        &self,
        now: TimestampMillis,
        limit: u32,
    ) -> Result<u32, PortError>;
    async fn attempt_verification(
        &self,
        command: VerificationAttemptCommand,
    ) -> Result<VerificationAtomicOutcome, PortError>;
    async fn issue_api_key(
        &self,
        command: ApiKeyIssueCommand,
    ) -> Result<ApiKeyIssueOutcome, PortError>;
    async fn authenticate_api_key(
        &self,
        command: ApiKeyAuthenticationCommand,
    ) -> Result<ApiKeyAuthenticationOutcome, PortError>;
    async fn revoke_api_key(&self, command: ApiKeyRevokeCommand) -> Result<bool, PortError>;
    async fn begin_oauth(&self, command: OAuthBeginCommand)
    -> Result<OAuthBeginOutcome, PortError>;
    async fn preflight_oauth_exchange(
        &self,
        command: OAuthPreflightCommand,
    ) -> Result<OAuthPreflightOutcome, PortError>;
    async fn finalize_oauth_exchange(
        &self,
        command: OAuthFinalizeCommand,
    ) -> Result<OAuthExchangeOutcome, PortError>;
    /// Claims the next ready delivery. Expired/suppressed entries are cleaned before selection,
    /// and a dispatcher crash becomes reclaimable after `lease_for`.
    async fn claim_auth_delivery(
        &self,
        now: TimestampMillis,
        lease_for: DurationMillis,
    ) -> Result<Option<AuthDeliveryClaim>, PortError>;
    async fn acknowledge_auth_delivery(
        &self,
        claim: AuthDeliveryClaim,
        now: TimestampMillis,
    ) -> Result<AuthDeliveryAcknowledgeOutcome, PortError>;
    async fn retry_auth_delivery(
        &self,
        claim: AuthDeliveryClaim,
        now: TimestampMillis,
        retry_at: TimestampMillis,
    ) -> Result<AuthDeliveryRetryOutcome, PortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialState {
    Current,
    Rotated,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CredentialIndex {
    session_id: SessionId,
    family_id: SessionFamilyId,
    state: CredentialState,
}

#[derive(Debug, Clone)]
struct IdentityEntry {
    principal: PrincipalSnapshot,
    identifiers: Vec<VersionedSecretDigest>,
    _destination: DeliveryDestinationRef,
}

#[derive(Debug, Clone)]
struct StoredDelivery {
    envelope: SealedDeliveryEnvelope,
    suppress: bool,
    expires_at: TimestampMillis,
    next_attempt_at: TimestampMillis,
    attempt: u16,
    lease: Option<StoredDeliveryLease>,
    initiator: Option<SessionContinuationBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredDeliveryLease {
    id: AuthDeliveryLeaseId,
    expires_at: TimestampMillis,
}

#[derive(Debug, Clone)]
struct PendingVerification {
    identifier_digests: SecretDigestCandidates,
    secret_digest: VersionedSecretDigest,
    purpose: VerificationPurpose,
    channel: VerificationChannel,
    initiator: Option<SessionContinuationBinding>,
    provisioning: Option<IdentityProvisioningIntent>,
    max_attempts: u16,
    expires_at: TimestampMillis,
    sealed_delivery: SealedDeliveryEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredPrincipalIssuanceGrant {
    user_id: UserId,
    identity_revision: u64,
    expires_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredIdentityProvisioningGrant {
    user_id: UserId,
    identity_revision: u64,
    identifier_digest: VersionedSecretDigest,
    expires_at: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredSessionMutationGrant {
    session_id: SessionId,
    user_id: UserId,
    generation: u64,
    token_digest: VersionedSecretDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredOAuthReservation {
    flow_id: frame_domain::OAuthFlowId,
    provider: OAuthProvider,
    initiator: Option<SessionContinuationBinding>,
    expires_at: TimestampMillis,
}

#[derive(Debug, Clone)]
struct StoredRateLimitBucket {
    bucket: AuthRateLimitBucket,
    gc_at: TimestampMillis,
}

#[derive(Clone, Default)]
struct MemoryAuthState {
    sessions: HashMap<SessionId, AuthSessionRecord>,
    credentials: HashMap<VersionedSecretDigest, CredentialIndex>,
    session_versions: HashMap<UserId, u64>,
    identities: HashMap<UserId, IdentityEntry>,
    identifier_index: HashMap<VersionedSecretDigest, UserId>,
    verifications: HashMap<VerificationId, VerificationChallenge>,
    pending_verifications: HashMap<frame_domain::AuthDeliveryId, PendingVerification>,
    rate_limits: HashMap<AbuseBucketId, StoredRateLimitBucket>,
    audit_events: Vec<AuthAuditEvent>,
    deliveries: HashMap<AuthDeliveryId, StoredDelivery>,
    api_keys: HashMap<ApiKeyId, ManagedApiKeyRecord>,
    api_key_index: HashMap<VersionedSecretDigest, ApiKeyId>,
    oauth_flows: HashMap<frame_domain::OAuthFlowId, OAuthFlowRecord>,
    external_accounts: HashMap<(OAuthProvider, VersionedSecretDigest), UserId>,
    issuance_grants: HashMap<PrincipalIssuanceGrantId, StoredPrincipalIssuanceGrant>,
    provisioning_grants: HashMap<IdentityProvisioningGrantId, StoredIdentityProvisioningGrant>,
    mutation_grants: HashMap<SessionMutationGrantId, StoredSessionMutationGrant>,
    oauth_reservations: HashMap<OAuthExchangeReservationId, StoredOAuthReservation>,
}

#[derive(Default)]
pub struct MemoryAuthStateRepository {
    state: RwLock<MemoryAuthState>,
    #[cfg(test)]
    fail_next_audit: AtomicBool,
}

impl fmt::Debug for MemoryAuthStateRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.read();
        formatter
            .debug_struct("MemoryAuthStateRepository")
            .field(
                "session_count",
                &state.as_ref().map_or(0, |state| state.sessions.len()),
            )
            .field(
                "identity_count",
                &state.as_ref().map_or(0, |state| state.identities.len()),
            )
            .field(
                "audit_event_count",
                &state.as_ref().map_or(0, |state| state.audit_events.len()),
            )
            .finish()
    }
}

impl MemoryAuthStateRepository {
    #[cfg(test)]
    fn provision_identity_for_test(
        &self,
        principal: PrincipalSnapshot,
        identifier_digests: Vec<VersionedSecretDigest>,
        destination: DeliveryDestinationRef,
    ) -> Result<(), PortError> {
        if identifier_digests.is_empty() {
            return Err(PortError::InvalidRequest(
                "identity requires a hashed identifier".into(),
            ));
        }
        let mut state = self.state.write().map_err(lock_error)?;
        if state.identities.contains_key(&principal.user_id)
            || identifier_digests
                .iter()
                .any(|digest| state.identifier_index.contains_key(digest))
        {
            return Err(PortError::Conflict);
        }
        for digest in &identifier_digests {
            state
                .identifier_index
                .insert(digest.clone(), principal.user_id);
        }
        state.session_versions.insert(principal.user_id, 0);
        state.identities.insert(
            principal.user_id,
            IdentityEntry {
                principal,
                identifiers: identifier_digests,
                _destination: destination,
            },
        );
        Ok(())
    }

    #[cfg(test)]
    fn fail_next_atomic_audit_for_test(&self) {
        self.fail_next_audit.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn mint_principal_issuance_grant_for_test(
        &self,
        principal: &PrincipalSnapshot,
        expires_at: TimestampMillis,
    ) -> Result<PrincipalIssuanceGrant, PortError> {
        let mut state = self.state.write().map_err(lock_error)?;
        let authoritative = authoritative_principal(&state, principal)?;
        Ok(mint_principal_issuance_grant(
            &mut state,
            &authoritative,
            TimestampMillis::new(0).map_err(|error| PortError::Adapter(error.to_string()))?,
            expires_at,
        ))
    }

    pub fn session(&self, id: SessionId) -> Result<Option<AuthSessionRecord>, PortError> {
        Ok(self
            .state
            .read()
            .map_err(lock_error)?
            .sessions
            .get(&id)
            .cloned())
    }

    pub fn audit_events(&self) -> Result<Vec<AuthAuditEvent>, PortError> {
        Ok(self.state.read().map_err(lock_error)?.audit_events.clone())
    }

    pub fn delivery_counts(&self) -> Result<(usize, usize), PortError> {
        let state = self.state.read().map_err(lock_error)?;
        Ok((
            state.deliveries.len() + state.pending_verifications.len(),
            state
                .deliveries
                .values()
                .filter(|delivery| delivery.suppress)
                .count(),
        ))
    }

    pub fn security_state_counts(&self) -> Result<(usize, usize, usize, usize), PortError> {
        let state = self.state.read().map_err(lock_error)?;
        Ok((
            state.sessions.len(),
            state.verifications.len(),
            state.rate_limits.len(),
            state.audit_events.len(),
        ))
    }

    pub fn api_key(&self, id: ApiKeyId) -> Result<Option<ManagedApiKeyRecord>, PortError> {
        Ok(self
            .state
            .read()
            .map_err(lock_error)?
            .api_keys
            .get(&id)
            .cloned())
    }

    fn transaction<T>(
        &self,
        operation: impl FnOnce(&mut MemoryAuthState) -> Result<T, PortError>,
    ) -> Result<T, PortError> {
        let mut guard = self.state.write().map_err(lock_error)?;
        #[cfg(test)]
        if self.fail_next_audit.swap(false, Ordering::SeqCst) {
            return Err(PortError::Adapter(
                "injected transactional audit failure".into(),
            ));
        }
        let mut next = guard.clone();
        let result = operation(&mut next)?;
        *guard = next;
        Ok(result)
    }
}

fn authoritative_principal(
    state: &MemoryAuthState,
    claimed: &PrincipalSnapshot,
) -> Result<PrincipalSnapshot, PortError> {
    let authoritative = state
        .identities
        .get(&claimed.user_id)
        .ok_or(PortError::NotFound)?
        .principal
        .clone();
    if authoritative != *claimed {
        return Err(PortError::Conflict);
    }
    Ok(authoritative)
}

fn require_action(audit: &DecisionAudit, expected: AuthAuditAction) -> Result<(), PortError> {
    if audit.action != expected {
        return Err(PortError::InvalidRequest(
            "authentication audit action does not match command".into(),
        ));
    }
    Ok(())
}

fn validate_grant(
    state: &mut MemoryAuthState,
    grant: &SessionMutationGrant,
    now: TimestampMillis,
) -> Result<(AuthSessionRecord, PrincipalSnapshot), PortError> {
    let stored = state
        .mutation_grants
        .get(&grant.id)
        .cloned()
        .ok_or(PortError::NotFound)?;
    if stored.session_id != grant.session_id
        || stored.user_id != grant.user_id
        || stored.generation != grant.generation
        || stored.token_digest != grant.token_digest
    {
        return Err(PortError::Conflict);
    }
    let session = state
        .sessions
        .get(&grant.session_id)
        .cloned()
        .ok_or(PortError::NotFound)?;
    let version = *state.session_versions.get(&session.user_id).unwrap_or(&0);
    if session.user_id != grant.user_id
        || session.generation != grant.generation
        || session.token_digest != grant.token_digest
        || session.evaluate(now, version) != AuthSessionDecision::Authenticated
    {
        return Err(PortError::Conflict);
    }
    let principal = state
        .identities
        .get(&session.user_id)
        .ok_or_else(|| PortError::Adapter("session identity is missing".into()))?
        .principal
        .clone();
    state.mutation_grants.remove(&grant.id);
    Ok((session, principal))
}

fn mint_session_mutation_grant(
    state: &mut MemoryAuthState,
    session: &AuthSessionRecord,
) -> SessionMutationGrant {
    // A newer browser-boundary validation supersedes every unconsumed proof for this session.
    state
        .mutation_grants
        .retain(|_, grant| grant.session_id != session.id);
    let mut id = SessionMutationGrantId::new();
    while state.mutation_grants.contains_key(&id) {
        id = SessionMutationGrantId::new();
    }
    state.mutation_grants.insert(
        id,
        StoredSessionMutationGrant {
            session_id: session.id,
            user_id: session.user_id,
            generation: session.generation,
            token_digest: session.token_digest.clone(),
        },
    );
    SessionMutationGrant::from_repository(
        id,
        session.id,
        session.user_id,
        session.generation,
        session.token_digest.clone(),
    )
}

fn mint_principal_issuance_grant(
    state: &mut MemoryAuthState,
    principal: &PrincipalSnapshot,
    now: TimestampMillis,
    expires_at: TimestampMillis,
) -> PrincipalIssuanceGrant {
    state
        .issuance_grants
        .retain(|_, grant| grant.expires_at > now);
    let mut id = PrincipalIssuanceGrantId::new();
    while state.issuance_grants.contains_key(&id) {
        id = PrincipalIssuanceGrantId::new();
    }
    state.issuance_grants.insert(
        id,
        StoredPrincipalIssuanceGrant {
            user_id: principal.user_id,
            identity_revision: principal.identity_revision,
            expires_at,
        },
    );
    PrincipalIssuanceGrant::from_repository(
        id,
        principal.user_id,
        principal.identity_revision,
        expires_at,
    )
}

fn consume_principal_issuance_grant(
    state: &mut MemoryAuthState,
    grant: &PrincipalIssuanceGrant,
    now: TimestampMillis,
) -> Result<PrincipalSnapshot, AuthAuditReason> {
    let Some(stored) = state.issuance_grants.get(&grant.id) else {
        return Err(AuthAuditReason::ReplayDetected);
    };
    if stored.user_id != grant.user_id
        || stored.identity_revision != grant.identity_revision
        || stored.expires_at != grant.expires_at
        || now >= stored.expires_at
    {
        return Err(if now >= stored.expires_at {
            AuthAuditReason::Expired
        } else {
            AuthAuditReason::InvalidCredential
        });
    }
    let principal = state
        .identities
        .get(&stored.user_id)
        .map(|identity| identity.principal.clone())
        .ok_or(AuthAuditReason::InvalidCredential)?;
    if principal.identity_revision != stored.identity_revision {
        return Err(AuthAuditReason::InvalidCredential);
    }
    state.issuance_grants.remove(&grant.id);
    Ok(principal)
}

fn mint_identity_provisioning_grant(
    state: &mut MemoryAuthState,
    intent: IdentityProvisioningIntent,
    identifier_digest: VersionedSecretDigest,
    now: TimestampMillis,
    expires_at: TimestampMillis,
) -> IdentityProvisioningGrant {
    state
        .provisioning_grants
        .retain(|_, grant| grant.expires_at > now);
    let mut id = IdentityProvisioningGrantId::new();
    while state.provisioning_grants.contains_key(&id) {
        id = IdentityProvisioningGrantId::new();
    }
    state.provisioning_grants.insert(
        id,
        StoredIdentityProvisioningGrant {
            user_id: intent.user_id,
            identity_revision: intent.identity_revision,
            identifier_digest: identifier_digest.clone(),
            expires_at,
        },
    );
    IdentityProvisioningGrant::from_repository(
        id,
        intent.user_id,
        intent.identity_revision,
        identifier_digest,
        expires_at,
    )
}

fn valid_session_continuation(
    state: &MemoryAuthState,
    binding: SessionContinuationBinding,
    now: TimestampMillis,
) -> bool {
    state
        .sessions
        .get(&binding.session_id)
        .is_some_and(|session| {
            let version = *state.session_versions.get(&session.user_id).unwrap_or(&0);
            session.user_id == binding.user_id
                && session.generation == binding.generation
                && session.evaluate(now, version) == AuthSessionDecision::Authenticated
        })
}

fn purge_session_continuations(state: &mut MemoryAuthState, session_id: SessionId) {
    state.pending_verifications.retain(|_, pending| {
        pending
            .initiator
            .is_none_or(|binding| binding.session_id != session_id)
    });
    state.verifications.retain(|_, challenge| {
        challenge
            .initiator
            .is_none_or(|binding| binding.session_id != session_id)
    });
    state.oauth_flows.retain(|_, flow| {
        flow.initiator
            .is_none_or(|binding| binding.session_id != session_id)
    });
    state.oauth_reservations.retain(|_, reservation| {
        reservation
            .initiator
            .is_none_or(|binding| binding.session_id != session_id)
    });
    state.deliveries.retain(|_, delivery| {
        delivery
            .initiator
            .is_none_or(|binding| binding.session_id != session_id)
    });
}

fn mint_oauth_reservation(
    state: &mut MemoryAuthState,
    flow: &OAuthFlowRecord,
) -> OAuthExchangeReservation {
    let mut id = OAuthExchangeReservationId::new();
    while state.oauth_reservations.contains_key(&id) {
        id = OAuthExchangeReservationId::new();
    }
    state.oauth_reservations.insert(
        id,
        StoredOAuthReservation {
            flow_id: flow.id,
            provider: flow.provider,
            initiator: flow.initiator,
            expires_at: flow.expires_at,
        },
    );
    OAuthExchangeReservation::from_repository(
        id,
        flow.id,
        flow.provider,
        flow.initiator,
        flow.expires_at,
    )
}

fn consume_oauth_reservation(
    state: &mut MemoryAuthState,
    reservation: &OAuthExchangeReservation,
    now: TimestampMillis,
) -> Result<StoredOAuthReservation, AuthAuditReason> {
    let Some(stored) = state.oauth_reservations.get(&reservation.id).cloned() else {
        return Err(AuthAuditReason::ReplayDetected);
    };
    if stored.flow_id != reservation.flow_id
        || stored.provider != reservation.provider
        || stored.initiator != reservation.initiator
        || stored.expires_at != reservation.expires_at
    {
        return Err(AuthAuditReason::InvalidCredential);
    }
    if now >= stored.expires_at {
        return Err(AuthAuditReason::Expired);
    }
    state.oauth_reservations.remove(&reservation.id);
    Ok(stored)
}

fn append_audit(
    state: &mut MemoryAuthState,
    audit: &DecisionAudit,
    context: Option<SessionAuditContext>,
    user_id: Option<UserId>,
    outcome: AuthAuditOutcome,
    reason: AuthAuditReason,
) {
    let context_user = context.map(|context| context.user_id);
    state
        .audit_events
        .push(AuthAuditEvent::new(NewAuthAuditEvent {
            correlation_id: audit.correlation_id,
            user_id: user_id.or(context_user),
            session_id: context.map(|context| context.session_id),
            client_kind: context.map(|context| context.client_kind),
            action: audit.action,
            outcome,
            reason,
            occurred_at: audit.occurred_at,
        }));
}

fn find_digest<'a, V>(
    map: &'a HashMap<VersionedSecretDigest, V>,
    candidates: &SecretDigestCandidates,
) -> Option<(VersionedSecretDigest, &'a V)> {
    candidates
        .iter()
        .find_map(|digest| map.get(digest).map(|value| (digest.clone(), value)))
}

fn revoke_family(state: &mut MemoryAuthState, family_id: SessionFamilyId, now: TimestampMillis) {
    let session_ids = state
        .sessions
        .values()
        .filter(|session| session.family_id == family_id)
        .map(|session| session.id)
        .collect::<Vec<_>>();
    for session_id in session_ids {
        revoke_one(
            state,
            session_id,
            SessionRevocationReason::ReplayDetected,
            now,
        );
    }
}

fn revoke_one(
    state: &mut MemoryAuthState,
    session_id: SessionId,
    reason: SessionRevocationReason,
    now: TimestampMillis,
) -> bool {
    let Some(session) = state.sessions.get_mut(&session_id) else {
        return false;
    };
    let was_active = session.state == AuthSessionState::Active;
    session.revoke(now, reason);
    for credential in state
        .credentials
        .values_mut()
        .filter(|credential| credential.session_id == session_id)
    {
        credential.state = CredentialState::Revoked;
    }
    state
        .mutation_grants
        .retain(|_, grant| grant.session_id != session_id);
    purge_session_continuations(state, session_id);
    was_active
}

fn target_for(
    state: &MemoryAuthState,
    candidates: &SecretDigestCandidates,
) -> Result<Option<UserId>, PortError> {
    let user_id = candidates
        .iter()
        .find_map(|digest| state.identifier_index.get(digest).copied());
    if let Some(user_id) = user_id {
        let identity = state.identities.get(&user_id).ok_or_else(|| {
            PortError::Adapter("identifier index references missing identity".into())
        })?;
        let _ = identity;
        Ok(Some(user_id))
    } else {
        Ok(None)
    }
}

fn materialize_pending_verifications(
    state: &mut MemoryAuthState,
    now: TimestampMillis,
    limit: usize,
) -> Result<u32, PortError> {
    let ids = state
        .pending_verifications
        .iter()
        .filter(|(_, pending)| pending.sealed_delivery.created_at <= now)
        .map(|(id, _)| *id)
        .take(limit)
        .collect::<Vec<_>>();
    let mut materialized = 0_u32;
    for id in ids {
        let pending = state
            .pending_verifications
            .remove(&id)
            .ok_or_else(|| PortError::Adapter("pending verification disappeared".into()))?;
        if pending.expires_at <= now {
            continue;
        }
        let identifier_owner = target_for(state, &pending.identifier_digests)?;
        let (challenge_user_id, suppress) = match pending.purpose {
            VerificationPurpose::AccountLink => {
                let initiator = pending.initiator.ok_or_else(|| {
                    PortError::Adapter("queued account link is missing its initiator".into())
                })?;
                if identifier_owner.is_none() && valid_session_continuation(state, initiator, now) {
                    (Some(initiator.user_id), false)
                } else {
                    // Already-owned identifiers (including the initiator's) are always no-op
                    // decoys so linking never becomes an account-existence oracle.
                    (None, true)
                }
            }
            VerificationPurpose::IdentityProvisioning => {
                let provisioning = pending.provisioning.ok_or_else(|| {
                    PortError::Adapter("queued signup is missing its provisioning intent".into())
                })?;
                if identifier_owner.is_none() {
                    (Some(provisioning.user_id), false)
                } else {
                    (None, true)
                }
            }
            VerificationPurpose::EmailVerify
            | VerificationPurpose::SignIn
            | VerificationPurpose::AccountRecovery => {
                (identifier_owner, identifier_owner.is_none())
            }
        };
        let challenge = VerificationChallenge::new(NewVerificationChallenge {
            user_id: challenge_user_id,
            initiator: pending.initiator,
            provisioning_revision: pending.provisioning.map(|intent| intent.identity_revision),
            identifier_digest: pending.identifier_digests.active().clone(),
            secret_digest: pending.secret_digest,
            purpose: pending.purpose,
            channel: pending.channel,
            max_attempts: pending.max_attempts,
            created_at: pending.sealed_delivery.created_at,
            expires_at: pending.expires_at,
        })
        .map_err(|error| PortError::Adapter(error.to_string()))?;
        state.verifications.insert(challenge.id, challenge);
        state.deliveries.insert(
            pending.sealed_delivery.id,
            StoredDelivery {
                next_attempt_at: pending.sealed_delivery.created_at,
                envelope: pending.sealed_delivery,
                suppress,
                expires_at: pending.expires_at,
                attempt: 0,
                lease: None,
                initiator: pending.initiator,
            },
        );
        if !matches!(
            pending.purpose,
            VerificationPurpose::AccountLink | VerificationPurpose::IdentityProvisioning
        ) && let Some(user_id) = identifier_owner
            && !state
                .identifier_index
                .contains_key(pending.identifier_digests.active())
        {
            state
                .identifier_index
                .insert(pending.identifier_digests.active().clone(), user_id);
            if let Some(identity) = state.identities.get_mut(&user_id) {
                identity
                    .identifiers
                    .push(pending.identifier_digests.active().clone());
            }
        }
        materialized = materialized.saturating_add(1);
    }
    Ok(materialized)
}

fn policy_for_dimension(
    policy: MultiRateLimitPolicy,
    dimension: AbuseDimension,
) -> frame_domain::RateLimitPolicy {
    match dimension {
        AbuseDimension::Identifier => policy.identifier,
        AbuseDimension::Source => policy.source,
        AbuseDimension::Device => policy.device,
        AbuseDimension::Global => policy.global,
    }
}

fn consume_limits(
    state: &mut MemoryAuthState,
    action: AuthAbuseAction,
    digests: &AbuseDigestSet,
    policy: MultiRateLimitPolicy,
    now: TimestampMillis,
) -> Result<Option<TimestampMillis>, PortError> {
    let action = action.rate_limit_bucket_action();
    state.rate_limits.retain(|_, stored| stored.gc_at > now);
    let dimensions = [
        // Global must be consumed first and a denial must short-circuit before attacker-chosen
        // dimensions can allocate cardinality.
        (AbuseDimension::Global, None),
        (AbuseDimension::Source, Some(&digests.source)),
        (AbuseDimension::Device, Some(&digests.device)),
        (AbuseDimension::Identifier, Some(&digests.identifier)),
    ];
    for (dimension, candidates) in dimensions {
        let dimension_policy = policy_for_dimension(policy, dimension);
        let active_digest = candidates.map(|candidates| candidates.active().clone());
        let active_id = AbuseBucketId::new(action, dimension, active_digest)
            .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
        let existing_ids = candidates.map_or_else(
            || {
                state
                    .rate_limits
                    .contains_key(&active_id)
                    .then_some(active_id.clone())
                    .into_iter()
                    .collect::<Vec<_>>()
            },
            |candidates| {
                candidates
                    .iter()
                    .filter_map(|digest| {
                        AbuseBucketId::new(action, dimension, Some(digest.clone()))
                            .ok()
                            .filter(|id| state.rate_limits.contains_key(id))
                    })
                    .collect::<Vec<_>>()
            },
        );
        let mut existing = existing_ids
            .into_iter()
            .filter_map(|id| state.rate_limits.remove(&id));
        let mut bucket = existing
            .next()
            .map(|stored| stored.bucket)
            .unwrap_or_else(|| AuthRateLimitBucket::new(active_id.clone(), now));
        for stale in existing {
            let stale = stale.bucket;
            bucket.window_started_at = bucket.window_started_at.max(stale.window_started_at);
            bucket.attempt_count = bucket.attempt_count.saturating_add(stale.attempt_count);
            bucket.blocked_until = match (bucket.blocked_until, stale.blocked_until) {
                (Some(left), Some(right)) => Some(left.max(right)),
                (left, right) => left.or(right),
            };
            bucket.updated_at = bucket.updated_at.max(stale.updated_at);
        }
        bucket.id = active_id.clone();
        if !state.rate_limits.contains_key(&active_id)
            && state.rate_limits.len() >= MAX_RATE_LIMIT_BUCKETS
        {
            let retry_at = now
                .checked_add(dimension_policy.block_for())
                .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
            return Ok(Some(retry_at));
        }
        let decision = bucket
            .consume(now, dimension_policy)
            .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
        let gc_at = bucket
            .updated_at
            .checked_add(dimension_policy.window())
            .and_then(|time| time.checked_add(dimension_policy.block_for()))
            .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
        state
            .rate_limits
            .insert(active_id, StoredRateLimitBucket { bucket, gc_at });
        if let RateLimitDecision::Limited { retry_at } = decision {
            return Ok(Some(retry_at));
        }
    }
    Ok(None)
}

fn verification_reason(decision: VerificationDecision) -> AuthAuditReason {
    match decision {
        VerificationDecision::Verified(_) => AuthAuditReason::VerificationCompleted,
        VerificationDecision::Invalid | VerificationDecision::Revoked => {
            AuthAuditReason::InvalidCredential
        }
        VerificationDecision::Expired => AuthAuditReason::Expired,
        VerificationDecision::AttemptsExhausted => AuthAuditReason::AttemptsExhausted,
        VerificationDecision::Replayed => AuthAuditReason::ReplayDetected,
    }
}

#[async_trait]
impl AuthStateRepository for MemoryAuthStateRepository {
    async fn provision_identity(
        &self,
        command: IdentityProvisionCommand,
    ) -> Result<IdentityProvisionOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::IdentityProvision)?;
        self.transaction(|state| {
            let grant = &command.grant;
            let Some(stored) = state.provisioning_grants.get(&grant.id).cloned() else {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(grant.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::ReplayDetected,
                );
                return Ok(IdentityProvisionOutcome::Rejected(
                    AuthAuditReason::ReplayDetected,
                ));
            };
            let reason = if command.audit.occurred_at >= stored.expires_at {
                Some(AuthAuditReason::Expired)
            } else if stored.user_id != grant.user_id
                || stored.identity_revision != grant.identity_revision
                || stored.identifier_digest != grant.identifier_digest
                || stored.expires_at != grant.expires_at
                || state.identities.contains_key(&stored.user_id)
                || state
                    .identifier_index
                    .contains_key(&stored.identifier_digest)
            {
                Some(AuthAuditReason::InvalidCredential)
            } else {
                None
            };
            if let Some(reason) = reason {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(stored.user_id),
                    AuthAuditOutcome::Deny,
                    reason,
                );
                return Ok(IdentityProvisionOutcome::Rejected(reason));
            }
            state.provisioning_grants.remove(&grant.id);
            state
                .identifier_index
                .insert(stored.identifier_digest.clone(), stored.user_id);
            state.session_versions.insert(stored.user_id, 0);
            let user_id = stored.user_id;
            state.identities.insert(
                user_id,
                IdentityEntry {
                    principal: PrincipalSnapshot {
                        user_id,
                        identity_revision: stored.identity_revision,
                        // Tenant/organization authority must be granted by its own workflow.
                        tenant_grants: Vec::new(),
                    },
                    identifiers: vec![stored.identifier_digest],
                    _destination: command.destination,
                },
            );
            append_audit(
                state,
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            );
            Ok(IdentityProvisionOutcome::Created)
        })
    }

    async fn issue_auth_session(
        &self,
        command: SessionIssueCommand,
    ) -> Result<SessionIssueOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::SessionIssue)?;
        self.transaction(|state| {
            let principal = match &command.authority {
                SessionIssueAuthority::Verified(grant) => {
                    match consume_principal_issuance_grant(state, grant, command.audit.occurred_at)
                    {
                        Ok(principal) => principal,
                        Err(reason) => {
                            append_audit(
                                state,
                                &command.audit,
                                None,
                                None,
                                AuthAuditOutcome::Deny,
                                reason,
                            );
                            return Ok(SessionIssueOutcome::Denied(reason));
                        }
                    }
                }
                SessionIssueAuthority::ExistingSession(grant) => {
                    match validate_grant(state, grant, command.audit.occurred_at) {
                        Ok((_, principal)) => principal,
                        Err(_) => {
                            append_audit(
                                state,
                                &command.audit,
                                None,
                                None,
                                AuthAuditOutcome::Deny,
                                AuthAuditReason::InvalidCredential,
                            );
                            return Ok(SessionIssueOutcome::Denied(
                                AuthAuditReason::InvalidCredential,
                            ));
                        }
                    }
                }
            };
            if principal != command.principal {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(principal.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(SessionIssueOutcome::Denied(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let mut session = command.session;
            session.session_version = *state.session_versions.get(&principal.user_id).unwrap_or(&0);
            if session.user_id != principal.user_id
                || state.sessions.contains_key(&session.id)
                || state.credentials.contains_key(&session.token_digest)
            {
                return Err(PortError::Conflict);
            }
            let context = SessionAuditContext::from(&session);
            state.credentials.insert(
                session.token_digest.clone(),
                CredentialIndex {
                    session_id: session.id,
                    family_id: session.family_id,
                    state: CredentialState::Current,
                },
            );
            state.sessions.insert(session.id, session);
            append_audit(
                state,
                &command.audit,
                Some(context),
                Some(principal.user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            );
            Ok(SessionIssueOutcome::Issued)
        })
    }

    async fn authenticate_session(
        &self,
        command: SessionAuthenticationCommand,
    ) -> Result<SessionPresentation, PortError> {
        let expected_action = if command.browser_boundary.is_some() {
            AuthAuditAction::BrowserMutationAuthenticate
        } else {
            AuthAuditAction::SessionAuthenticate
        };
        require_action(&command.audit, expected_action)?;
        self.transaction(|state| {
            let Some((matched_digest, credential)) =
                find_digest(&state.credentials, &command.token_digests)
            else {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(SessionPresentation::Unknown);
            };
            let credential = *credential;
            let session = state
                .sessions
                .get(&credential.session_id)
                .cloned()
                .ok_or_else(|| {
                    PortError::Adapter("credential references missing session".into())
                })?;
            let context = SessionAuditContext::from(&session);
            match credential.state {
                CredentialState::Rotated => {
                    revoke_family(state, credential.family_id, command.audit.occurred_at);
                    append_audit(
                        state,
                        &command.audit,
                        Some(context),
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::ReplayDetected,
                    );
                    return Ok(SessionPresentation::ReplayFamilyRevoked(context));
                }
                CredentialState::Revoked => {
                    append_audit(
                        state,
                        &command.audit,
                        Some(context),
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::Revoked,
                    );
                    return Ok(SessionPresentation::Revoked(context));
                }
                CredentialState::Current => {}
            }
            let version = *state.session_versions.get(&session.user_id).unwrap_or(&0);
            let decision = session.evaluate(command.audit.occurred_at, version);
            if decision != AuthSessionDecision::Authenticated {
                let (reason, presentation) = match decision {
                    AuthSessionDecision::Expired => {
                        revoke_one(
                            state,
                            session.id,
                            SessionRevocationReason::Expired,
                            command.audit.occurred_at,
                        );
                        (
                            AuthAuditReason::Expired,
                            SessionPresentation::Expired(context),
                        )
                    }
                    AuthSessionDecision::Revoked => (
                        AuthAuditReason::Revoked,
                        SessionPresentation::Revoked(context),
                    ),
                    AuthSessionDecision::SessionVersionMismatch => {
                        revoke_one(
                            state,
                            session.id,
                            SessionRevocationReason::SessionVersionChanged,
                            command.audit.occurred_at,
                        );
                        (
                            AuthAuditReason::SessionVersionMismatch,
                            SessionPresentation::SessionVersionMismatch(context),
                        )
                    }
                    AuthSessionDecision::Authenticated => unreachable!(),
                };
                append_audit(
                    state,
                    &command.audit,
                    Some(context),
                    None,
                    AuthAuditOutcome::Deny,
                    reason,
                );
                return Ok(presentation);
            }
            let mut session = session;
            let mut audit_reason = AuthAuditReason::Authenticated;
            if matched_digest != *command.token_digests.active() {
                state.credentials.remove(&matched_digest);
                session.token_digest = command.token_digests.active().clone();
                state.credentials.insert(
                    session.token_digest.clone(),
                    CredentialIndex {
                        session_id: session.id,
                        family_id: session.family_id,
                        state: CredentialState::Current,
                    },
                );
                audit_reason = AuthAuditReason::KeyVersionMigrated;
            }
            let mutation_grant = if let Some(boundary) = &command.browser_boundary {
                let boundary_reason = if session.client_kind != AuthClientKind::Browser {
                    Some(AuthAuditReason::InvalidCredential)
                } else if boundary.origin.as_ref() != session.browser_origin.as_ref() {
                    Some(AuthAuditReason::OriginMismatch)
                } else if boundary.fetch_site != FetchSite::SameOrigin {
                    Some(AuthAuditReason::FetchMetadataMismatch)
                } else {
                    let cookie_matches_header = boundary.csrf_cookie_digests.iter().any(|cookie| {
                        boundary
                            .csrf_header_digests
                            .iter()
                            .any(|header| header == cookie)
                    });
                    let stored_matches = session.csrf_digest.as_ref().is_some_and(|stored| {
                        boundary
                            .csrf_header_digests
                            .iter()
                            .any(|candidate| candidate == stored)
                    });
                    if cookie_matches_header && stored_matches {
                        session.csrf_digest = Some(boundary.csrf_header_digests.active().clone());
                        None
                    } else {
                        Some(AuthAuditReason::CsrfMismatch)
                    }
                };
                if let Some(reason) = boundary_reason {
                    append_audit(
                        state,
                        &command.audit,
                        Some(context),
                        None,
                        AuthAuditOutcome::Deny,
                        reason,
                    );
                    return Ok(SessionPresentation::BoundaryRejected(context, reason));
                }
                Some(mint_session_mutation_grant(state, &session))
            } else {
                None
            };
            let principal = state
                .identities
                .get(&session.user_id)
                .ok_or_else(|| PortError::Adapter("session identity is missing".into()))?
                .principal
                .clone();
            state.sessions.insert(session.id, session.clone());
            append_audit(
                state,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Allow,
                audit_reason,
            );
            Ok(SessionPresentation::Authenticated(
                AuthenticatedSessionPresentation::from_repository(
                    session.id,
                    session.client_kind,
                    principal,
                    mutation_grant,
                ),
            ))
        })
    }

    async fn rotate_auth_session(
        &self,
        request: SessionRotationRequest,
    ) -> Result<SessionRotationOutcome, PortError> {
        require_action(&request.audit, AuthAuditAction::SessionRotate)?;
        self.transaction(|state| {
            let existing_context = state
                .sessions
                .get(&request.grant.session_id)
                .map(SessionAuditContext::from);
            let (current, _) =
                match validate_grant(state, &request.grant, request.audit.occurred_at) {
                    Ok(validated) => validated,
                    Err(_) => {
                        append_audit(
                            state,
                            &request.audit,
                            existing_context,
                            None,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        );
                        return Ok(SessionRotationOutcome::Denied(
                            AuthAuditReason::InvalidCredential,
                        ));
                    }
                };
            let context = SessionAuditContext::from(&current);
            if request.now != request.audit.occurred_at
                || state.credentials.contains_key(&request.next_token_digest)
            {
                append_audit(
                    state,
                    &request.audit,
                    Some(context),
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(SessionRotationOutcome::Denied(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let session = state
                .sessions
                .get_mut(&current.id)
                .ok_or_else(|| PortError::Adapter("session disappeared".into()))?;
            session
                .rotate(
                    request.grant.generation,
                    request.next_token_digest.clone(),
                    Some(request.next_csrf_digest),
                    request.now,
                    request.idle_expires_at.min(current.absolute_expires_at),
                )
                .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
            let rotated = session.clone();
            state.mutation_grants.retain(|_, grant| {
                grant.session_id != rotated.id || grant.generation == rotated.generation
            });
            purge_session_continuations(state, rotated.id);
            state
                .credentials
                .get_mut(&current.token_digest)
                .ok_or_else(|| PortError::Adapter("current credential disappeared".into()))?
                .state = CredentialState::Rotated;
            state.credentials.insert(
                request.next_token_digest,
                CredentialIndex {
                    session_id: rotated.id,
                    family_id: rotated.family_id,
                    state: CredentialState::Current,
                },
            );
            append_audit(
                state,
                &request.audit,
                Some(context),
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::Rotated,
            );
            Ok(SessionRotationOutcome::Rotated(Box::new(rotated)))
        })
    }

    async fn revoke_auth_session(&self, command: SessionRevokeCommand) -> Result<bool, PortError> {
        require_action(&command.audit, AuthAuditAction::Logout)?;
        self.transaction(|state| {
            let context = state
                .sessions
                .get(&command.grant.session_id)
                .map(SessionAuditContext::from);
            if validate_grant(state, &command.grant, command.audit.occurred_at).is_err() {
                append_audit(
                    state,
                    &command.audit,
                    context,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(false);
            }
            let revoked = revoke_one(
                state,
                command.grant.session_id,
                command.reason,
                command.audit.occurred_at,
            );
            append_audit(
                state,
                &command.audit,
                context,
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::LoggedOut,
            );
            Ok(revoked)
        })
    }

    async fn revoke_all_auth_sessions(
        &self,
        command: SessionRevokeCommand,
    ) -> Result<LogoutAllOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::LogoutAll)?;
        self.transaction(|state| {
            let existing_context = state
                .sessions
                .get(&command.grant.session_id)
                .map(SessionAuditContext::from);
            let (session, _) =
                match validate_grant(state, &command.grant, command.audit.occurred_at) {
                    Ok(validated) => validated,
                    Err(_) => {
                        append_audit(
                            state,
                            &command.audit,
                            existing_context,
                            None,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        );
                        return Ok(LogoutAllOutcome::Denied(AuthAuditReason::InvalidCredential));
                    }
                };
            let context = SessionAuditContext::from(&session);
            let current = *state.session_versions.get(&session.user_id).unwrap_or(&0);
            let next = current
                .checked_add(1)
                .ok_or_else(|| PortError::InvalidRequest("session version exhausted".into()))?;
            state.session_versions.insert(session.user_id, next);
            let ids = state
                .sessions
                .values()
                .filter(|candidate| {
                    candidate.user_id == session.user_id
                        && candidate.state == AuthSessionState::Active
                })
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let mut revoked_sessions = 0_u64;
            for id in ids {
                revoked_sessions += u64::from(revoke_one(
                    state,
                    id,
                    SessionRevocationReason::LogoutAll,
                    command.audit.occurred_at,
                ));
            }
            append_audit(
                state,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::LoggedOutAll,
            );
            Ok(LogoutAllOutcome::Revoked {
                new_session_version: next,
                revoked_sessions,
            })
        })
    }

    async fn issue_verification(
        &self,
        command: VerificationIssueCommand,
    ) -> Result<VerificationIssueAtomicOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::VerificationIssue)?;
        self.transaction(|state| {
            let initiator = match (&command.initiated_by, &command.initiator_grant) {
                (Some(claimed), Some(grant)) => {
                    let authoritative = authoritative_principal(state, claimed);
                    let validated = validate_grant(state, grant, command.audit.occurred_at);
                    match (authoritative, validated) {
                        (Ok(authoritative), Ok((session, grant_principal)))
                            if authoritative == grant_principal =>
                        {
                            Some((
                                authoritative,
                                SessionContinuationBinding {
                                    session_id: session.id,
                                    user_id: session.user_id,
                                    generation: session.generation,
                                },
                            ))
                        }
                        _ => {
                            append_audit(
                                state,
                                &command.audit,
                                None,
                                None,
                                AuthAuditOutcome::Deny,
                                AuthAuditReason::InvalidCredential,
                            );
                            return Ok(VerificationIssueAtomicOutcome::Rejected(
                                AuthAuditReason::InvalidCredential,
                            ));
                        }
                    }
                }
                (None, None) => None,
                _ => {
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    );
                    return Ok(VerificationIssueAtomicOutcome::Rejected(
                        AuthAuditReason::InvalidCredential,
                    ));
                }
            };
            let valid_authority_shape = match command.purpose {
                VerificationPurpose::AccountLink => {
                    initiator.is_some() && command.provisioning.is_none()
                }
                VerificationPurpose::IdentityProvisioning => {
                    initiator.is_none()
                        && command
                            .provisioning
                            .is_some_and(|intent| intent.identity_revision > 0)
                }
                VerificationPurpose::EmailVerify
                | VerificationPurpose::SignIn
                | VerificationPurpose::AccountRecovery => {
                    initiator.is_none() && command.provisioning.is_none()
                }
            };
            let audit_user = initiator
                .as_ref()
                .map(|(principal, _)| principal.user_id)
                .or(command.provisioning.map(|intent| intent.user_id));
            if !valid_authority_shape {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    audit_user,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(VerificationIssueAtomicOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let action = match command.purpose {
                VerificationPurpose::IdentityProvisioning => {
                    AuthAbuseAction::IdentityProvisionIssue
                }
                VerificationPurpose::SignIn | VerificationPurpose::EmailVerify => {
                    AuthAbuseAction::SignInIssue
                }
                VerificationPurpose::AccountRecovery => AuthAbuseAction::RecoverIssue,
                VerificationPurpose::AccountLink => AuthAbuseAction::AccountLinkIssue,
            };
            if let Some(retry_at) = consume_limits(
                state,
                action,
                &command.abuse,
                command.rate_policy,
                command.audit.occurred_at,
            )? {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    audit_user,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                );
                return Ok(VerificationIssueAtomicOutcome::RateLimited { retry_at });
            }
            for existing in state.verifications.values_mut().filter(|challenge| {
                command
                    .identifier_digests
                    .iter()
                    .any(|digest| digest == &challenge.identifier_digest)
                    && challenge.purpose == command.purpose
                    && challenge.state == VerificationState::Pending
            }) {
                existing.revoke();
            }
            state.pending_verifications.retain(|_, pending| {
                pending.purpose != command.purpose
                    || !command
                        .identifier_digests
                        .iter()
                        .any(|digest| digest == pending.identifier_digests.active())
            });
            let pending_id = command.sealed_delivery.id;
            state.pending_verifications.insert(
                pending_id,
                PendingVerification {
                    identifier_digests: command.identifier_digests,
                    secret_digest: command.secret_digest,
                    purpose: command.purpose,
                    channel: command.channel,
                    initiator: initiator.as_ref().map(|(_, binding)| *binding),
                    provisioning: command.provisioning,
                    max_attempts: command.max_attempts,
                    expires_at: command.expires_at,
                    sealed_delivery: command.sealed_delivery,
                },
            );
            append_audit(
                state,
                &command.audit,
                None,
                audit_user,
                AuthAuditOutcome::Allow,
                AuthAuditReason::VerificationAccepted,
            );
            Ok(VerificationIssueAtomicOutcome::Accepted)
        })
    }

    async fn materialize_verification_deliveries(
        &self,
        now: TimestampMillis,
        limit: u32,
    ) -> Result<u32, PortError> {
        if limit == 0 || limit > 1_000 {
            return Err(PortError::InvalidRequest(
                "verification materialization limit is invalid".into(),
            ));
        }
        self.transaction(|state| {
            state
                .verifications
                .retain(|_, challenge| challenge.expires_at > now);
            materialize_pending_verifications(state, now, limit as usize)
        })
    }

    async fn attempt_verification(
        &self,
        command: VerificationAttemptCommand,
    ) -> Result<VerificationAtomicOutcome, PortError> {
        require_action(
            &command.audit,
            if command.purpose == VerificationPurpose::AccountLink {
                AuthAuditAction::AccountLink
            } else {
                AuthAuditAction::VerificationConsume
            },
        )?;
        self.transaction(|state| {
            if let Some(retry_at) = consume_limits(
                state,
                AuthAbuseAction::Verify,
                &command.abuse,
                command.rate_policy,
                command.audit.occurred_at,
            )? {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                );
                return Ok(VerificationAtomicOutcome::RateLimited { retry_at });
            }
            let exact = state
                .verifications
                .iter()
                .filter(|(_, challenge)| {
                    command
                        .identifier_digests
                        .iter()
                        .any(|digest| digest == &challenge.identifier_digest)
                        && command
                            .secret_digests
                            .iter()
                            .any(|digest| digest == &challenge.secret_digest)
                        && challenge.purpose == command.purpose
                })
                .max_by_key(|(_, challenge)| challenge.created_at)
                .map(|(id, _)| *id);
            let candidate = exact.or_else(|| {
                state
                    .verifications
                    .iter()
                    .filter(|(_, challenge)| {
                        command
                            .identifier_digests
                            .iter()
                            .any(|digest| digest == &challenge.identifier_digest)
                            && challenge.purpose == command.purpose
                            && challenge.state == VerificationState::Pending
                    })
                    .max_by_key(|(_, challenge)| challenge.created_at)
                    .map(|(id, _)| *id)
            });
            if command.purpose == VerificationPurpose::AccountLink
                && candidate.is_some_and(|candidate| {
                    state
                        .verifications
                        .get(&candidate)
                        .and_then(|challenge| challenge.initiator)
                        .is_none_or(|binding| {
                            !valid_session_continuation(state, binding, command.audit.occurred_at)
                        })
                })
            {
                if let Some(candidate) = candidate {
                    state.verifications.remove(&candidate);
                }
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(VerificationAtomicOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let (decision, grant_expires_at, initiator, provisioning_revision) =
                if let Some(candidate) = candidate {
                    let challenge = state
                        .verifications
                        .get_mut(&candidate)
                        .ok_or_else(|| PortError::Adapter("verification disappeared".into()))?;
                    let decision = challenge.attempt(command.audit.occurred_at, exact.is_some());
                    let expires_at = challenge.expires_at;
                    let initiator = challenge.initiator;
                    let provisioning_revision = challenge.provisioning_revision;
                    if exact.is_some() {
                        challenge.identifier_digest = command.identifier_digests.active().clone();
                        challenge.secret_digest = command.secret_digests.active().clone();
                    }
                    (decision, Some(expires_at), initiator, provisioning_revision)
                } else {
                    (VerificationDecision::Invalid, None, None, None)
                };
            let reason = verification_reason(decision);
            if let VerificationDecision::Verified(user_id) = decision {
                let expires_at = grant_expires_at.ok_or_else(|| {
                    PortError::Adapter("verified challenge expiry is missing".into())
                })?;
                if command.purpose == VerificationPurpose::IdentityProvisioning {
                    let owner = command
                        .identifier_digests
                        .iter()
                        .find_map(|digest| state.identifier_index.get(digest).copied());
                    let Some(identity_revision) = provisioning_revision else {
                        return Err(PortError::Adapter(
                            "signup challenge is missing its revision".into(),
                        ));
                    };
                    if owner.is_some() || state.identities.contains_key(&user_id) {
                        append_audit(
                            state,
                            &command.audit,
                            None,
                            Some(user_id),
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        );
                        return Ok(VerificationAtomicOutcome::Rejected(
                            AuthAuditReason::InvalidCredential,
                        ));
                    }
                    let grant = mint_identity_provisioning_grant(
                        state,
                        IdentityProvisioningIntent {
                            user_id,
                            identity_revision,
                        },
                        command.identifier_digests.active().clone(),
                        command.audit.occurred_at,
                        expires_at,
                    );
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        Some(user_id),
                        AuthAuditOutcome::Allow,
                        reason,
                    );
                    return Ok(VerificationAtomicOutcome::ProvisioningAuthorized(grant));
                }
                if command.purpose == VerificationPurpose::AccountLink {
                    let owner = command
                        .identifier_digests
                        .iter()
                        .find_map(|digest| state.identifier_index.get(digest).copied());
                    if owner.is_some()
                        || initiator.is_none_or(|binding| {
                            binding.user_id != user_id
                                || !valid_session_continuation(
                                    state,
                                    binding,
                                    command.audit.occurred_at,
                                )
                        })
                    {
                        append_audit(
                            state,
                            &command.audit,
                            None,
                            Some(user_id),
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        );
                        return Ok(VerificationAtomicOutcome::Rejected(
                            AuthAuditReason::InvalidCredential,
                        ));
                    }
                    state
                        .identifier_index
                        .insert(command.identifier_digests.active().clone(), user_id);
                    let identity = state
                        .identities
                        .get_mut(&user_id)
                        .ok_or_else(|| PortError::Adapter("linked identity is missing".into()))?;
                    if !identity
                        .identifiers
                        .contains(command.identifier_digests.active())
                    {
                        identity
                            .identifiers
                            .push(command.identifier_digests.active().clone());
                    }
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        Some(user_id),
                        AuthAuditOutcome::Allow,
                        AuthAuditReason::Linked,
                    );
                    return Ok(VerificationAtomicOutcome::Linked { user_id });
                }
                if command.purpose == VerificationPurpose::AccountRecovery {
                    let current = *state.session_versions.get(&user_id).unwrap_or(&0);
                    let next = current.checked_add(1).ok_or_else(|| {
                        PortError::InvalidRequest("session version exhausted".into())
                    })?;
                    state.session_versions.insert(user_id, next);
                    let session_ids = state
                        .sessions
                        .values()
                        .filter(|session| {
                            session.user_id == user_id && session.state == AuthSessionState::Active
                        })
                        .map(|session| session.id)
                        .collect::<Vec<_>>();
                    for session_id in session_ids {
                        revoke_one(
                            state,
                            session_id,
                            SessionRevocationReason::AccountRecovery,
                            command.audit.occurred_at,
                        );
                    }
                }
                let principal = state
                    .identities
                    .get(&user_id)
                    .ok_or_else(|| PortError::Adapter("verified identity is missing".into()))?
                    .principal
                    .clone();
                let issuance_grant = mint_principal_issuance_grant(
                    state,
                    &principal,
                    command.audit.occurred_at,
                    expires_at,
                );
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(user_id),
                    AuthAuditOutcome::Allow,
                    reason,
                );
                Ok(VerificationAtomicOutcome::Verified {
                    principal,
                    issuance_grant,
                })
            } else {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    reason,
                );
                Ok(VerificationAtomicOutcome::Rejected(reason))
            }
        })
    }

    async fn issue_api_key(
        &self,
        command: ApiKeyIssueCommand,
    ) -> Result<ApiKeyIssueOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::ApiKeyIssue)?;
        self.transaction(|state| {
            let principal = authoritative_principal(state, &command.principal)?;
            let (_, grant_principal) =
                validate_grant(state, &command.grant, command.audit.occurred_at)?;
            if grant_principal != principal {
                return Err(PortError::Conflict);
            }
            if !principal.can_manage_api_keys(command.record.tenant_id) {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(principal.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InsufficientRole,
                );
                return Ok(ApiKeyIssueOutcome::Forbidden);
            }
            if command.record.owner_id != principal.user_id
                || state.api_keys.contains_key(&command.record.id)
                || state.api_key_index.contains_key(&command.record.key_digest)
            {
                return Err(PortError::Conflict);
            }
            state
                .api_key_index
                .insert(command.record.key_digest.clone(), command.record.id);
            state.api_keys.insert(command.record.id, command.record);
            append_audit(
                state,
                &command.audit,
                None,
                Some(principal.user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            );
            Ok(ApiKeyIssueOutcome::Issued)
        })
    }

    async fn authenticate_api_key(
        &self,
        command: ApiKeyAuthenticationCommand,
    ) -> Result<ApiKeyAuthenticationOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::ApiKeyAuthenticate)?;
        self.transaction(|state| {
            if let Some(retry_at) = consume_limits(
                state,
                AuthAbuseAction::ApiKeyAuthenticate,
                &command.abuse,
                command.rate_policy,
                command.audit.occurred_at,
            )? {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                );
                return Ok(ApiKeyAuthenticationOutcome::RateLimited { retry_at });
            }
            let Some((matched, key_id)) = find_digest(&state.api_key_index, &command.key_digests)
            else {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(ApiKeyAuthenticationOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            };
            let key_id = *key_id;
            let key = state
                .api_keys
                .get(&key_id)
                .cloned()
                .ok_or_else(|| PortError::Adapter("API key index is inconsistent".into()))?;
            if key.tenant_id != command.tenant_id
                || !key.allows(command.required_scope, command.audit.occurred_at)
            {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(key.owner_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(ApiKeyAuthenticationOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            if matched != *command.key_digests.active() {
                state.api_key_index.remove(&matched);
                state
                    .api_key_index
                    .insert(command.key_digests.active().clone(), key_id);
                state
                    .api_keys
                    .get_mut(&key_id)
                    .ok_or_else(|| PortError::Adapter("API key disappeared".into()))?
                    .key_digest = command.key_digests.active().clone();
            }
            let principal = state
                .identities
                .get(&key.owner_id)
                .ok_or_else(|| PortError::Adapter("API key owner is missing".into()))?
                .principal
                .clone();
            append_audit(
                state,
                &command.audit,
                None,
                Some(key.owner_id),
                AuthAuditOutcome::Allow,
                if matched == *command.key_digests.active() {
                    AuthAuditReason::Authenticated
                } else {
                    AuthAuditReason::KeyVersionMigrated
                },
            );
            Ok(ApiKeyAuthenticationOutcome::Authenticated(principal))
        })
    }

    async fn revoke_api_key(&self, command: ApiKeyRevokeCommand) -> Result<bool, PortError> {
        require_action(&command.audit, AuthAuditAction::ApiKeyRevoke)?;
        self.transaction(|state| {
            let (session, principal) =
                match validate_grant(state, &command.grant, command.audit.occurred_at) {
                    Ok(validated) => validated,
                    Err(_) => {
                        append_audit(
                            state,
                            &command.audit,
                            None,
                            None,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        );
                        return Ok(false);
                    }
                };
            let Some(key) = state.api_keys.get_mut(&command.key_id) else {
                append_audit(
                    state,
                    &command.audit,
                    Some(SessionAuditContext::from(&session)),
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(false);
            };
            if !principal.can_manage_api_keys(key.tenant_id) {
                append_audit(
                    state,
                    &command.audit,
                    Some(SessionAuditContext::from(&session)),
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InsufficientRole,
                );
                return Ok(false);
            }
            key.revoke(command.audit.occurred_at);
            append_audit(
                state,
                &command.audit,
                Some(SessionAuditContext::from(&session)),
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::Revoked,
            );
            Ok(true)
        })
    }

    async fn begin_oauth(
        &self,
        command: OAuthBeginCommand,
    ) -> Result<OAuthBeginOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::OAuthBegin)?;
        self.transaction(|state| {
            state
                .oauth_flows
                .retain(|_, flow| flow.expires_at > command.audit.occurred_at);
            state
                .oauth_reservations
                .retain(|_, reservation| reservation.expires_at > command.audit.occurred_at);
            if state.oauth_flows.len() >= MAX_PENDING_OAUTH_FLOWS {
                let retry_at = state
                    .oauth_flows
                    .values()
                    .map(|flow| flow.expires_at)
                    .min()
                    .unwrap_or(command.flow.expires_at);
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                );
                return Ok(OAuthBeginOutcome::RateLimited { retry_at });
            }
            if let Some(retry_at) = consume_limits(
                state,
                AuthAbuseAction::OAuthBegin,
                &command.abuse,
                command.rate_policy,
                command.audit.occurred_at,
            )? {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                );
                return Ok(OAuthBeginOutcome::RateLimited { retry_at });
            }
            let initiator = match &command.initiator {
                Some(grant) => match validate_grant(state, grant, command.audit.occurred_at) {
                    Ok((session, principal)) => Some(SessionContinuationBinding {
                        session_id: session.id,
                        user_id: principal.user_id,
                        generation: session.generation,
                    }),
                    Err(_) => {
                        append_audit(
                            state,
                            &command.audit,
                            None,
                            None,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        );
                        return Ok(OAuthBeginOutcome::Rejected(
                            AuthAuditReason::InvalidCredential,
                        ));
                    }
                },
                None => None,
            };
            let purpose_matches = match command.flow.purpose {
                frame_domain::OAuthFlowPurpose::SignIn => initiator.is_none(),
                frame_domain::OAuthFlowPurpose::AccountLink => initiator.is_some(),
            };
            if !purpose_matches
                || command.flow.initiator != initiator
                || command.flow.created_at != command.audit.occurred_at
                || command.flow.expires_at <= command.audit.occurred_at
                || state.oauth_flows.contains_key(&command.flow.id)
            {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    initiator.map(|binding| binding.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(OAuthBeginOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let flow_id = command.flow.id;
            state.oauth_flows.insert(flow_id, command.flow);
            append_audit(
                state,
                &command.audit,
                None,
                initiator.map(|binding| binding.user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            );
            Ok(OAuthBeginOutcome::Started)
        })
    }

    async fn preflight_oauth_exchange(
        &self,
        command: OAuthPreflightCommand,
    ) -> Result<OAuthPreflightOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::OAuthExchangePreflight)?;
        self.transaction(|state| {
            if let Some(retry_at) = consume_limits(
                state,
                AuthAbuseAction::OAuthExchange,
                &command.abuse,
                command.rate_policy,
                command.audit.occurred_at,
            )? {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                );
                return Ok(OAuthPreflightOutcome::RateLimited { retry_at });
            }
            let flow_id = state
                .oauth_flows
                .iter()
                .find(|(_, flow)| {
                    command
                        .state_digests
                        .iter()
                        .any(|digest| digest == &flow.state_digest)
                })
                .map(|(id, _)| *id);
            let Some(flow_id) = flow_id else {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(OAuthPreflightOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            };
            let flow_initiator = state
                .oauth_flows
                .get(&flow_id)
                .and_then(|flow| flow.initiator);
            if flow_initiator.is_some_and(|binding| {
                !valid_session_continuation(state, binding, command.audit.occurred_at)
            }) {
                state.oauth_flows.remove(&flow_id);
                append_audit(
                    state,
                    &command.audit,
                    None,
                    flow_initiator.map(|binding| binding.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(OAuthPreflightOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let flow = state
                .oauth_flows
                .get_mut(&flow_id)
                .ok_or_else(|| PortError::Adapter("OAuth flow disappeared".into()))?;
            if flow.provider != command.provider {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(OAuthPreflightOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let decision = flow.consume(
                command.audit.occurred_at,
                command
                    .state_digests
                    .iter()
                    .any(|digest| digest == &flow.state_digest),
                command
                    .pkce_digests
                    .iter()
                    .any(|digest| digest == &flow.pkce_digest),
                command
                    .redirect_digests
                    .iter()
                    .any(|digest| digest == &flow.redirect_digest),
                command
                    .audience_digests
                    .iter()
                    .any(|digest| digest == &flow.audience_digest),
            );
            let reason = match decision {
                frame_domain::OAuthFlowDecision::Accepted { .. } => None,
                frame_domain::OAuthFlowDecision::Expired => Some(AuthAuditReason::Expired),
                frame_domain::OAuthFlowDecision::Replayed => Some(AuthAuditReason::ReplayDetected),
                frame_domain::OAuthFlowDecision::Invalid
                | frame_domain::OAuthFlowDecision::Revoked => {
                    Some(AuthAuditReason::InvalidCredential)
                }
            };
            if let Some(reason) = reason {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    None,
                    AuthAuditOutcome::Deny,
                    reason,
                );
                return Ok(OAuthPreflightOutcome::Rejected(reason));
            }
            let flow = state
                .oauth_flows
                .get(&flow_id)
                .cloned()
                .ok_or_else(|| PortError::Adapter("OAuth flow disappeared".into()))?;
            let reservation = mint_oauth_reservation(state, &flow);
            append_audit(
                state,
                &command.audit,
                None,
                flow.initiator.map(|binding| binding.user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            );
            Ok(OAuthPreflightOutcome::Ready(reservation))
        })
    }

    async fn finalize_oauth_exchange(
        &self,
        command: OAuthFinalizeCommand,
    ) -> Result<OAuthExchangeOutcome, PortError> {
        require_action(&command.audit, AuthAuditAction::OAuthExchange)?;
        self.transaction(|state| {
            let reservation = match consume_oauth_reservation(
                state,
                &command.reservation,
                command.audit.occurred_at,
            ) {
                Ok(reservation) => reservation,
                Err(reason) => {
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        None,
                        AuthAuditOutcome::Deny,
                        reason,
                    );
                    return Ok(OAuthExchangeOutcome::Rejected(reason));
                }
            };
            if reservation.initiator.is_some_and(|binding| {
                !valid_session_continuation(state, binding, command.audit.occurred_at)
            }) {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    reservation.initiator.map(|binding| binding.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(OAuthExchangeOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let assertion = match command.provider_result {
                OAuthProviderResult::Verified(assertion) => assertion,
                OAuthProviderResult::Rejected => {
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        reservation.initiator.map(|binding| binding.user_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    );
                    return Ok(OAuthExchangeOutcome::Rejected(
                        AuthAuditReason::InvalidCredential,
                    ));
                }
                OAuthProviderResult::AdapterFailure => {
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        reservation.initiator.map(|binding| binding.user_id),
                        AuthAuditOutcome::Error,
                        AuthAuditReason::AdapterFailure,
                    );
                    return Ok(OAuthExchangeOutcome::Rejected(
                        AuthAuditReason::AdapterFailure,
                    ));
                }
            };
            if assertion.provider != reservation.provider {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    reservation.initiator.map(|binding| binding.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                );
                return Ok(OAuthExchangeOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            let subject = assertion.subject_digests.iter().find_map(|digest| {
                state
                    .external_accounts
                    .get(&(assertion.provider, digest.clone()))
                    .copied()
                    .map(|user_id| (digest.clone(), user_id))
            });
            let identifier_owner =
                assertion
                    .verified_identifier_digests
                    .as_ref()
                    .and_then(|digests| {
                        digests
                            .iter()
                            .find_map(|digest| state.identifier_index.get(digest).copied())
                    });
            let active_subject = assertion.subject_digests.active().clone();
            let user_id = if let Some(initiator) = reservation.initiator {
                if subject
                    .as_ref()
                    .is_some_and(|(_, existing)| *existing != initiator.user_id)
                    || identifier_owner.is_some_and(|existing| existing != initiator.user_id)
                {
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        Some(initiator.user_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    );
                    return Ok(OAuthExchangeOutcome::Rejected(
                        AuthAuditReason::InvalidCredential,
                    ));
                }
                state
                    .external_accounts
                    .insert((assertion.provider, active_subject), initiator.user_id);
                initiator.user_id
            } else if let Some((matched_subject, user_id)) = subject {
                if matched_subject != active_subject {
                    state
                        .external_accounts
                        .remove(&(assertion.provider, matched_subject));
                    state
                        .external_accounts
                        .insert((assertion.provider, active_subject), user_id);
                }
                user_id
            } else {
                let Some(user_id) = identifier_owner else {
                    append_audit(
                        state,
                        &command.audit,
                        None,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    );
                    return Ok(OAuthExchangeOutcome::Rejected(
                        AuthAuditReason::InvalidCredential,
                    ));
                };
                state
                    .external_accounts
                    .insert((assertion.provider, active_subject), user_id);
                user_id
            };
            if reservation.initiator.is_some() {
                append_audit(
                    state,
                    &command.audit,
                    None,
                    Some(user_id),
                    AuthAuditOutcome::Allow,
                    AuthAuditReason::Linked,
                );
                return Ok(OAuthExchangeOutcome::Linked { user_id });
            }
            let principal = state
                .identities
                .get(&user_id)
                .ok_or_else(|| PortError::Adapter("OAuth identity is missing".into()))?
                .principal
                .clone();
            let issuance_grant = mint_principal_issuance_grant(
                state,
                &principal,
                command.audit.occurred_at,
                reservation.expires_at,
            );
            append_audit(
                state,
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Authenticated,
            );
            Ok(OAuthExchangeOutcome::Verified {
                principal,
                issuance_grant,
            })
        })
    }

    async fn claim_auth_delivery(
        &self,
        now: TimestampMillis,
        lease_for: DurationMillis,
    ) -> Result<Option<AuthDeliveryClaim>, PortError> {
        if lease_for.get() > MAX_AUTH_DELIVERY_LEASE_MILLIS {
            return Err(PortError::InvalidRequest(
                "authentication delivery lease is too long".into(),
            ));
        }
        let requested_lease_expires_at = now
            .checked_add(lease_for)
            .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
        self.transaction(|state| {
            // Materialization deliberately happens after the enumeration-safe issue response.
            materialize_pending_verifications(state, now, 100)?;
            state.deliveries.retain(|_, delivery| {
                !delivery.suppress
                    && delivery.expires_at > now
                    && delivery.attempt < MAX_AUTH_DELIVERY_ATTEMPTS
            });
            let next_id = state
                .deliveries
                .iter()
                .filter(|(_, delivery)| {
                    delivery.next_attempt_at <= now
                        && delivery.lease.is_none_or(|lease| lease.expires_at <= now)
                })
                .min_by_key(|(_, delivery)| {
                    (delivery.next_attempt_at, delivery.envelope.created_at)
                })
                .map(|(id, _)| *id);
            let Some(next_id) = next_id else {
                return Ok(None);
            };
            let delivery = state
                .deliveries
                .get_mut(&next_id)
                .ok_or_else(|| PortError::Adapter("delivery disappeared while claiming".into()))?;
            delivery.attempt = delivery
                .attempt
                .checked_add(1)
                .ok_or_else(|| PortError::Adapter("delivery attempt overflow".into()))?;
            let lease_id = AuthDeliveryLeaseId::new();
            let lease_expires_at = requested_lease_expires_at.min(delivery.expires_at);
            delivery.lease = Some(StoredDeliveryLease {
                id: lease_id,
                expires_at: lease_expires_at,
            });
            Ok(Some(AuthDeliveryClaim::from_repository(
                lease_id,
                delivery.envelope.clone(),
                lease_expires_at,
                delivery.attempt,
            )))
        })
    }

    async fn acknowledge_auth_delivery(
        &self,
        claim: AuthDeliveryClaim,
        now: TimestampMillis,
    ) -> Result<AuthDeliveryAcknowledgeOutcome, PortError> {
        self.transaction(|state| {
            let delivery_id = claim.delivery_id();
            let current = state.deliveries.get(&delivery_id);
            let is_current = current.is_some_and(|delivery| {
                delivery.attempt == claim.attempt
                    && delivery.lease.is_some_and(|lease| {
                        lease.id == claim.lease_id
                            && lease.expires_at == claim.lease_expires_at
                            && now < lease.expires_at
                    })
            });
            if !is_current {
                return Ok(AuthDeliveryAcknowledgeOutcome::StaleLease);
            }
            state.deliveries.remove(&delivery_id);
            Ok(AuthDeliveryAcknowledgeOutcome::Acknowledged)
        })
    }

    async fn retry_auth_delivery(
        &self,
        claim: AuthDeliveryClaim,
        now: TimestampMillis,
        retry_at: TimestampMillis,
    ) -> Result<AuthDeliveryRetryOutcome, PortError> {
        if retry_at <= now {
            return Err(PortError::InvalidRequest(
                "authentication delivery retry must be in the future".into(),
            ));
        }
        self.transaction(|state| {
            let delivery_id = claim.delivery_id();
            let Some(delivery) = state.deliveries.get_mut(&delivery_id) else {
                return Ok(AuthDeliveryRetryOutcome::StaleLease);
            };
            let is_current = delivery.attempt == claim.attempt
                && delivery.lease.is_some_and(|lease| {
                    lease.id == claim.lease_id
                        && lease.expires_at == claim.lease_expires_at
                        && now < lease.expires_at
                });
            if !is_current {
                return Ok(AuthDeliveryRetryOutcome::StaleLease);
            }
            if delivery.attempt >= MAX_AUTH_DELIVERY_ATTEMPTS || retry_at >= delivery.expires_at {
                state.deliveries.remove(&delivery_id);
                return Ok(AuthDeliveryRetryOutcome::Exhausted);
            }
            delivery.next_attempt_at = retry_at;
            delivery.lease = None;
            Ok(AuthDeliveryRetryOutcome::Scheduled)
        })
    }
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> PortError {
    PortError::Adapter(format!(
        "in-memory authentication adapter lock poisoned: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use frame_domain::{
        CorrelationId, DurationMillis, HashKeyVersion, NewAuthSession, OrganizationRole,
        SecretDigest, TenantGrant,
    };

    use super::*;

    fn time(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("time")
    }

    fn duration(value: u64) -> DurationMillis {
        DurationMillis::new(value).expect("duration")
    }

    fn digest(version: u16, value: char) -> VersionedSecretDigest {
        VersionedSecretDigest::new(
            HashKeyVersion::new(version).expect("version"),
            SecretDigest::parse_sha256(value.to_string().repeat(64)).expect("digest"),
        )
    }

    fn indexed_digest(value: u64) -> VersionedSecretDigest {
        VersionedSecretDigest::new(
            HashKeyVersion::new(1).expect("version"),
            SecretDigest::parse_sha256(format!("{:064x}", value + 1)).expect("digest"),
        )
    }

    fn candidates(
        active: VersionedSecretDigest,
        fallback: Vec<VersionedSecretDigest>,
    ) -> SecretDigestCandidates {
        SecretDigestCandidates::new(active, fallback).expect("candidates")
    }

    fn audit(action: AuthAuditAction, now: i64) -> DecisionAudit {
        DecisionAudit {
            correlation_id: CorrelationId::new(),
            action,
            occurred_at: time(now),
        }
    }

    fn principal(
        user_id: UserId,
        tenant_id: TenantId,
        role: OrganizationRole,
    ) -> PrincipalSnapshot {
        PrincipalSnapshot {
            user_id,
            identity_revision: 1,
            tenant_grants: vec![TenantGrant { tenant_id, role }],
        }
    }

    fn provisioned_repository() -> (
        MemoryAuthStateRepository,
        PrincipalSnapshot,
        VersionedSecretDigest,
    ) {
        let repository = MemoryAuthStateRepository::default();
        let user_id = UserId::new();
        let identifier = digest(1, '1');
        let principal = principal(user_id, TenantId::new(), OrganizationRole::Owner);
        repository
            .provision_identity_for_test(
                principal.clone(),
                vec![identifier.clone()],
                DeliveryDestinationRef::parse("delivery-user-1").expect("destination"),
            )
            .expect("provision");
        (repository, principal, identifier)
    }

    fn session(principal: &PrincipalSnapshot, token: VersionedSecretDigest) -> AuthSessionRecord {
        AuthSessionRecord::new(NewAuthSession {
            id: SessionId::new(),
            family_id: SessionFamilyId::new(),
            user_id: principal.user_id,
            client_kind: AuthClientKind::Browser,
            token_digest: token,
            csrf_digest: Some(digest(1, 'c')),
            browser_origin: Some(
                frame_domain::ExactBrowserOrigin::parse("https://frame.engmanager.xyz")
                    .expect("origin"),
            ),
            issued_at: time(1),
            idle_expires_at: time(20),
            absolute_expires_at: time(40),
            session_version: 0,
        })
        .expect("session")
    }

    fn issuance(
        repository: &MemoryAuthStateRepository,
        principal: &PrincipalSnapshot,
    ) -> SessionIssueAuthority {
        SessionIssueAuthority::Verified(
            repository
                .mint_principal_issuance_grant_for_test(principal, time(100))
                .expect("issuance grant"),
        )
    }

    fn boundary(csrf_digests: SecretDigestCandidates) -> BrowserBoundaryRequest {
        BrowserBoundaryRequest {
            origin: Some(
                ExactBrowserOrigin::parse("https://frame.engmanager.xyz").expect("origin"),
            ),
            fetch_site: FetchSite::SameOrigin,
            csrf_cookie_digests: csrf_digests.clone(),
            csrf_header_digests: csrf_digests,
        }
    }

    fn expect_mutation_grant(presentation: SessionPresentation) -> SessionMutationGrant {
        let SessionPresentation::Authenticated(presentation) = presentation else {
            panic!("authenticated presentation expected");
        };
        let (_, grant) = presentation.into_parts();
        grant.expect("mutation grant")
    }

    #[tokio::test]
    async fn audit_failure_rolls_back_session_state() {
        let (repository, principal, _) = provisioned_repository();
        let session = session(&principal, digest(1, 'a'));
        repository.fail_next_atomic_audit_for_test();
        assert!(
            repository
                .issue_auth_session(SessionIssueCommand {
                    principal: principal.clone(),
                    authority: issuance(&repository, &principal),
                    session: session.clone(),
                    audit: audit(AuthAuditAction::SessionIssue, 1),
                })
                .await
                .is_err()
        );
        assert!(repository.session(session.id).expect("read").is_none());
        assert!(repository.audit_events().expect("audit").is_empty());
    }

    #[tokio::test]
    async fn repository_minted_principal_grants_are_single_use() {
        let (repository, principal, _) = provisioned_repository();
        let SessionIssueAuthority::Verified(grant) = issuance(&repository, &principal) else {
            unreachable!("test issuance is verified");
        };
        assert_eq!(
            repository
                .issue_auth_session(SessionIssueCommand {
                    principal: principal.clone(),
                    authority: SessionIssueAuthority::Verified(grant.clone()),
                    session: session(&principal, digest(1, 'a')),
                    audit: audit(AuthAuditAction::SessionIssue, 1),
                })
                .await
                .expect("first issue"),
            SessionIssueOutcome::Issued
        );
        assert_eq!(
            repository
                .issue_auth_session(SessionIssueCommand {
                    principal: principal.clone(),
                    authority: SessionIssueAuthority::Verified(grant),
                    session: session(&principal, digest(1, 'b')),
                    audit: audit(AuthAuditAction::SessionIssue, 1),
                })
                .await
                .expect("replay decision"),
            SessionIssueOutcome::Denied(AuthAuditReason::ReplayDetected)
        );
    }

    #[tokio::test]
    async fn identity_provisioning_grants_reject_forgery_replay_and_concurrent_double_spend() {
        fn mint(
            repository: &MemoryAuthStateRepository,
            user_id: UserId,
            identifier: VersionedSecretDigest,
        ) -> IdentityProvisioningGrant {
            let mut state = repository.state.write().expect("state");
            mint_identity_provisioning_grant(
                &mut state,
                IdentityProvisioningIntent {
                    user_id,
                    identity_revision: 1,
                },
                identifier,
                time(1),
                time(10),
            )
        }

        fn command(
            grant: IdentityProvisioningGrant,
            destination: &str,
        ) -> IdentityProvisionCommand {
            IdentityProvisionCommand {
                grant,
                destination: DeliveryDestinationRef::parse(destination).expect("destination"),
                audit: audit(AuthAuditAction::IdentityProvision, 2),
            }
        }

        let repository = MemoryAuthStateRepository::default();
        let user_id = UserId::new();
        let identifier = digest(1, '7');
        let grant = mint(&repository, user_id, identifier.clone());
        let forged = IdentityProvisioningGrant::from_repository(
            IdentityProvisioningGrantId::new(),
            grant.user_id(),
            grant.identity_revision(),
            grant.identifier_digest().clone(),
            grant.expires_at(),
        );
        assert_eq!(
            repository
                .provision_identity(command(forged, "forged-destination"))
                .await
                .expect("forged decision"),
            IdentityProvisionOutcome::Rejected(AuthAuditReason::ReplayDetected)
        );

        let (first, second) = tokio::join!(
            repository.provision_identity(command(grant.clone(), "race-destination-a")),
            repository.provision_identity(command(grant.clone(), "race-destination-b")),
        );
        let outcomes = [first.expect("first race"), second.expect("second race")];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == IdentityProvisionOutcome::Created)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    **outcome == IdentityProvisionOutcome::Rejected(AuthAuditReason::ReplayDetected)
                })
                .count(),
            1
        );
        assert_eq!(
            repository
                .provision_identity(command(grant, "replay-destination"))
                .await
                .expect("replay decision"),
            IdentityProvisionOutcome::Rejected(AuthAuditReason::ReplayDetected)
        );

        let identifier_race = MemoryAuthStateRepository::default();
        let shared_identifier = digest(1, '8');
        let first_grant = mint(&identifier_race, UserId::new(), shared_identifier.clone());
        let second_grant = mint(&identifier_race, UserId::new(), shared_identifier);
        let (first, second) = tokio::join!(
            identifier_race
                .provision_identity(command(first_grant, "identifier-race-destination-a")),
            identifier_race
                .provision_identity(command(second_grant, "identifier-race-destination-b")),
        );
        let outcomes = [
            first.expect("first identifier race"),
            second.expect("second identifier race"),
        ];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == IdentityProvisionOutcome::Created)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    **outcome
                        == IdentityProvisionOutcome::Rejected(AuthAuditReason::InvalidCredential)
                })
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn fallback_key_authentication_migrates_then_replay_revokes_family() {
        let (repository, principal, _) = provisioned_repository();
        let original = session(&principal, digest(1, 'a'));
        repository
            .issue_auth_session(SessionIssueCommand {
                principal: principal.clone(),
                authority: issuance(&repository, &principal),
                session: original.clone(),
                audit: audit(AuthAuditAction::SessionIssue, 1),
            })
            .await
            .expect("issue");
        let authenticated = repository
            .authenticate_session(SessionAuthenticationCommand {
                token_digests: candidates(digest(2, 'b'), vec![digest(1, 'a')]),
                browser_boundary: Some(boundary(candidates(digest(2, 'd'), vec![digest(1, 'c')]))),
                audit: audit(AuthAuditAction::BrowserMutationAuthenticate, 2),
            })
            .await
            .expect("auth");
        let grant = expect_mutation_grant(authenticated);
        repository
            .rotate_auth_session(SessionRotationRequest {
                grant,
                next_token_digest: digest(2, 'e'),
                next_csrf_digest: digest(2, 'f'),
                now: time(3),
                idle_expires_at: time(23),
                audit: audit(AuthAuditAction::SessionRotate, 3),
            })
            .await
            .expect("rotate");
        assert!(matches!(
            repository
                .authenticate_session(SessionAuthenticationCommand {
                    token_digests: candidates(digest(2, 'b'), vec![digest(1, 'a')]),
                    browser_boundary: None,
                    audit: audit(AuthAuditAction::SessionAuthenticate, 4),
                })
                .await
                .expect("replay"),
            SessionPresentation::ReplayFamilyRevoked(_)
        ));
    }

    #[tokio::test]
    async fn mutation_grants_are_revalidated_at_the_atomic_write_boundary() {
        let (repository, principal, _) = provisioned_repository();
        let original = session(&principal, digest(1, 'a'));
        repository
            .issue_auth_session(SessionIssueCommand {
                principal: principal.clone(),
                authority: issuance(&repository, &principal),
                session: original.clone(),
                audit: audit(AuthAuditAction::SessionIssue, 1),
            })
            .await
            .expect("issue");
        let grant = expect_mutation_grant(
            repository
                .authenticate_session(SessionAuthenticationCommand {
                    token_digests: candidates(digest(1, 'a'), vec![]),
                    browser_boundary: Some(boundary(candidates(digest(1, 'c'), vec![]))),
                    audit: audit(AuthAuditAction::BrowserMutationAuthenticate, 2),
                })
                .await
                .expect("authenticate"),
        );

        assert_eq!(
            repository
                .rotate_auth_session(SessionRotationRequest {
                    grant: grant.clone(),
                    next_token_digest: digest(1, 'b'),
                    next_csrf_digest: digest(1, 'd'),
                    now: time(20),
                    idle_expires_at: time(30),
                    audit: audit(AuthAuditAction::SessionRotate, 20),
                })
                .await
                .expect("rotation decision"),
            SessionRotationOutcome::Denied(AuthAuditReason::InvalidCredential)
        );
        assert!(
            !repository
                .revoke_auth_session(SessionRevokeCommand {
                    grant: grant.clone(),
                    reason: SessionRevocationReason::UserLogout,
                    audit: audit(AuthAuditAction::Logout, 20),
                })
                .await
                .expect("logout decision")
        );
        assert_eq!(
            repository
                .revoke_all_auth_sessions(SessionRevokeCommand {
                    grant,
                    reason: SessionRevocationReason::LogoutAll,
                    audit: audit(AuthAuditAction::LogoutAll, 20),
                })
                .await
                .expect("logout-all decision"),
            LogoutAllOutcome::Denied(AuthAuditReason::InvalidCredential)
        );
        let stored = repository
            .session(original.id)
            .expect("read")
            .expect("session");
        assert_eq!(stored.generation, 0);
        assert_eq!(stored.state, AuthSessionState::Active);
        assert_eq!(repository.audit_events().expect("audit").len(), 5);
    }

    #[tokio::test]
    async fn a_new_browser_boundary_proof_supersedes_an_unconsumed_older_proof() {
        let (repository, principal, _) = provisioned_repository();
        let original = session(&principal, digest(1, 'a'));
        repository
            .issue_auth_session(SessionIssueCommand {
                principal: principal.clone(),
                authority: issuance(&repository, &principal),
                session: original,
                audit: audit(AuthAuditAction::SessionIssue, 1),
            })
            .await
            .expect("issue");
        let authenticate = |now| SessionAuthenticationCommand {
            token_digests: candidates(digest(1, 'a'), vec![]),
            browser_boundary: Some(boundary(candidates(digest(1, 'c'), vec![]))),
            audit: audit(AuthAuditAction::BrowserMutationAuthenticate, now),
        };
        let older = expect_mutation_grant(
            repository
                .authenticate_session(authenticate(2))
                .await
                .expect("older proof"),
        );
        let current = expect_mutation_grant(
            repository
                .authenticate_session(authenticate(3))
                .await
                .expect("current proof"),
        );
        assert!(
            !repository
                .revoke_auth_session(SessionRevokeCommand {
                    grant: older,
                    reason: SessionRevocationReason::UserLogout,
                    audit: audit(AuthAuditAction::Logout, 3),
                })
                .await
                .expect("stale decision")
        );
        assert!(
            repository
                .revoke_auth_session(SessionRevokeCommand {
                    grant: current,
                    reason: SessionRevocationReason::UserLogout,
                    audit: audit(AuthAuditAction::Logout, 3),
                })
                .await
                .expect("current decision")
        );
    }

    #[tokio::test]
    async fn known_and_unknown_verification_do_equivalent_durable_work() {
        let (repository, _, known_identifier) = provisioned_repository();
        let unknown_identifier = digest(1, '9');
        let sealer = DeterministicDeliverySealer;
        let policy = MultiRateLimitPolicy {
            identifier: frame_domain::RateLimitPolicy::new(10, duration(10), duration(10))
                .expect("policy"),
            source: frame_domain::RateLimitPolicy::new(10, duration(10), duration(10))
                .expect("policy"),
            device: frame_domain::RateLimitPolicy::new(10, duration(10), duration(10))
                .expect("policy"),
            global: frame_domain::RateLimitPolicy::new(10, duration(10), duration(10))
                .expect("policy"),
        };
        for (index, identifier) in [known_identifier, unknown_identifier]
            .into_iter()
            .enumerate()
        {
            let identifier = candidates(identifier, vec![]);
            let one_time_code = VerificationSecret::OneTimeCode(
                frame_domain::OneTimeCode::parse(format!("12345{index}")).expect("otp"),
            );
            let material = VerificationDeliveryMaterial {
                destination: VerificationDestination::parse(format!("queued-{index}@example.test"))
                    .expect("destination"),
                secret: one_time_code,
                purpose: VerificationPurpose::SignIn,
                expires_at: time(10),
            };
            let envelope = sealer.seal(&material, time(1)).expect("seal");
            let abuse = AbuseDigestSet {
                identifier: identifier.clone(),
                source: candidates(digest(1, 'e'), vec![]),
                device: candidates(digest(1, 'd'), vec![]),
            };
            assert_eq!(
                repository
                    .issue_verification(VerificationIssueCommand {
                        identifier_digests: identifier,
                        secret_digest: digest(1, if index == 0 { 'a' } else { 'b' }),
                        purpose: VerificationPurpose::SignIn,
                        channel: VerificationChannel::OneTimeCode,
                        initiated_by: None,
                        initiator_grant: None,
                        provisioning: None,
                        max_attempts: 3,
                        expires_at: time(10),
                        sealed_delivery: envelope,
                        abuse,
                        rate_policy: policy,
                        audit: audit(AuthAuditAction::VerificationIssue, 1),
                    })
                    .await
                    .expect("issue"),
                VerificationIssueAtomicOutcome::Accepted
            );
        }
        assert_eq!(repository.delivery_counts().expect("counts"), (2, 0));
        assert_eq!(
            repository
                .materialize_verification_deliveries(time(2), 10)
                .await
                .expect("materialize"),
            2
        );
        assert_eq!(repository.delivery_counts().expect("counts"), (2, 1));
        assert_eq!(repository.audit_events().expect("audit").len(), 2);
    }

    #[tokio::test]
    async fn delivery_leases_reclaim_crashes_fence_stale_acks_and_clean_suppressed_rows() {
        let (repository, _, known_identifier) = provisioned_repository();
        let policy =
            frame_domain::RateLimitPolicy::new(20, duration(20), duration(20)).expect("policy");
        let policy = MultiRateLimitPolicy {
            identifier: policy,
            source: policy,
            device: policy,
            global: policy,
        };
        for (index, identifier) in [known_identifier, digest(1, '9')].into_iter().enumerate() {
            let identifier = candidates(identifier, vec![]);
            assert_eq!(
                repository
                    .issue_verification(VerificationIssueCommand {
                        identifier_digests: identifier.clone(),
                        secret_digest: digest(1, if index == 0 { 'a' } else { 'b' }),
                        purpose: VerificationPurpose::SignIn,
                        channel: VerificationChannel::OneTimeCode,
                        initiated_by: None,
                        initiator_grant: None,
                        provisioning: None,
                        max_attempts: 3,
                        expires_at: time(100),
                        sealed_delivery: SealedDeliveryEnvelope::new(
                            vec![if index == 0 { b'a' } else { b'b' }; 64],
                            time(1),
                        )
                        .expect("envelope"),
                        abuse: AbuseDigestSet {
                            identifier,
                            source: candidates(
                                digest(1, if index == 0 { 'c' } else { 'd' }),
                                vec![],
                            ),
                            device: candidates(
                                digest(1, if index == 0 { 'e' } else { 'f' }),
                                vec![],
                            ),
                        },
                        rate_policy: policy,
                        audit: audit(AuthAuditAction::VerificationIssue, 1),
                    })
                    .await
                    .expect("issue"),
                VerificationIssueAtomicOutcome::Accepted
            );
        }
        repository
            .materialize_verification_deliveries(time(2), 10)
            .await
            .expect("materialize");
        assert_eq!(repository.delivery_counts().expect("counts"), (2, 1));

        let first = repository
            .claim_auth_delivery(time(2), duration(2))
            .await
            .expect("claim")
            .expect("known delivery");
        assert_eq!(first.attempt(), 1);
        // Claim-time cleanup removes suppressed enumeration decoys rather than retaining them.
        assert_eq!(repository.delivery_counts().expect("counts"), (1, 0));
        assert!(
            repository
                .claim_auth_delivery(time(3), duration(2))
                .await
                .expect("claim")
                .is_none()
        );

        let reclaimed = repository
            .claim_auth_delivery(time(4), duration(2))
            .await
            .expect("reclaim")
            .expect("expired lease is reclaimable");
        assert_eq!(reclaimed.delivery_id(), first.delivery_id());
        assert_eq!(reclaimed.attempt(), 2);
        assert_eq!(
            repository
                .acknowledge_auth_delivery(first, time(4))
                .await
                .expect("stale ack"),
            AuthDeliveryAcknowledgeOutcome::StaleLease
        );
        assert_eq!(
            repository
                .retry_auth_delivery(reclaimed, time(4), time(5))
                .await
                .expect("retry"),
            AuthDeliveryRetryOutcome::Scheduled
        );
        assert!(
            repository
                .claim_auth_delivery(time(4), duration(2))
                .await
                .expect("not ready")
                .is_none()
        );
        let retried = repository
            .claim_auth_delivery(time(5), duration(2))
            .await
            .expect("retry claim")
            .expect("scheduled delivery");
        assert_eq!(retried.attempt(), 3);
        assert_eq!(
            repository
                .acknowledge_auth_delivery(retried, time(5))
                .await
                .expect("ack"),
            AuthDeliveryAcknowledgeOutcome::Acknowledged
        );
        assert_eq!(repository.delivery_counts().expect("counts"), (0, 0));
    }

    #[tokio::test]
    async fn api_keys_require_tenant_admin_and_are_key_rotation_compatible() {
        let (repository, principal, _) = provisioned_repository();
        let tenant = principal.tenant_grants[0].tenant_id;
        let browser_session = session(&principal, digest(1, 'a'));
        repository
            .issue_auth_session(SessionIssueCommand {
                principal: principal.clone(),
                authority: issuance(&repository, &principal),
                session: browser_session,
                audit: audit(AuthAuditAction::SessionIssue, 1),
            })
            .await
            .expect("session");
        let grant = expect_mutation_grant(
            repository
                .authenticate_session(SessionAuthenticationCommand {
                    token_digests: candidates(digest(1, 'a'), vec![]),
                    browser_boundary: Some(boundary(candidates(digest(1, 'c'), vec![]))),
                    audit: audit(AuthAuditAction::BrowserMutationAuthenticate, 2),
                })
                .await
                .expect("proof"),
        );
        let record = ManagedApiKeyRecord {
            id: ApiKeyId::new(),
            owner_id: principal.user_id,
            tenant_id: tenant,
            key_digest: digest(1, 'c'),
            scopes: vec![ApiKeyScope::VideosRead],
            created_at: time(1),
            expires_at: None,
            revoked_at: None,
        };
        assert_eq!(
            repository
                .issue_api_key(ApiKeyIssueCommand {
                    principal: principal.clone(),
                    grant,
                    record,
                    audit: audit(AuthAuditAction::ApiKeyIssue, 3),
                })
                .await
                .expect("issue"),
            ApiKeyIssueOutcome::Issued
        );
        let policy =
            frame_domain::RateLimitPolicy::new(10, duration(10), duration(10)).expect("policy");
        assert!(matches!(
            repository
                .authenticate_api_key(ApiKeyAuthenticationCommand {
                    key_digests: candidates(digest(2, 'd'), vec![digest(1, 'c')]),
                    tenant_id: tenant,
                    required_scope: ApiKeyScope::VideosRead,
                    abuse: AbuseDigestSet {
                        identifier: candidates(digest(1, 'e'), vec![]),
                        source: candidates(digest(1, 'f'), vec![]),
                        device: candidates(digest(1, 'd'), vec![]),
                    },
                    rate_policy: MultiRateLimitPolicy {
                        identifier: policy,
                        source: policy,
                        device: policy,
                        global: policy,
                    },
                    audit: audit(AuthAuditAction::ApiKeyAuthenticate, 4),
                })
                .await
                .expect("auth"),
            ApiKeyAuthenticationOutcome::Authenticated(_)
        ));
    }

    fn oauth_flow(marker: char, created_at: i64, expires_at: i64) -> OAuthFlowRecord {
        OAuthFlowRecord {
            id: frame_domain::OAuthFlowId::new(),
            provider: OAuthProvider::Github,
            purpose: frame_domain::OAuthFlowPurpose::SignIn,
            initiator: None,
            state_digest: digest(1, marker),
            pkce_digest: digest(1, 'a'),
            redirect_digest: digest(1, 'b'),
            audience_digest: digest(1, 'c'),
            created_at: time(created_at),
            expires_at: time(expires_at),
            consumed_at: None,
            revoked: false,
        }
    }

    fn oauth_abuse(marker: char) -> AbuseDigestSet {
        AbuseDigestSet {
            identifier: candidates(digest(1, marker), vec![]),
            source: candidates(digest(1, 'd'), vec![]),
            device: candidates(digest(1, 'e'), vec![]),
        }
    }

    #[tokio::test]
    async fn oauth_begin_is_rate_limited_and_garbage_collects_expired_preflights() {
        let high = frame_domain::RateLimitPolicy::new(20, duration(20), duration(20))
            .expect("high policy");
        let high_policy = MultiRateLimitPolicy {
            identifier: high,
            source: high,
            device: high,
            global: high,
        };
        let repository = MemoryAuthStateRepository::default();
        assert_eq!(
            repository
                .begin_oauth(OAuthBeginCommand {
                    flow: oauth_flow('1', 1, 2),
                    initiator: None,
                    abuse: oauth_abuse('1'),
                    rate_policy: high_policy,
                    audit: audit(AuthAuditAction::OAuthBegin, 1),
                })
                .await
                .expect("begin"),
            OAuthBeginOutcome::Started
        );
        assert_eq!(
            repository
                .begin_oauth(OAuthBeginCommand {
                    flow: oauth_flow('2', 2, 10),
                    initiator: None,
                    abuse: oauth_abuse('2'),
                    rate_policy: high_policy,
                    audit: audit(AuthAuditAction::OAuthBegin, 2),
                })
                .await
                .expect("begin after expiry"),
            OAuthBeginOutcome::Started
        );
        assert_eq!(repository.state.read().expect("state").oauth_flows.len(), 1);

        let low =
            frame_domain::RateLimitPolicy::new(1, duration(20), duration(20)).expect("low policy");
        let low_policy = MultiRateLimitPolicy {
            identifier: high,
            source: high,
            device: high,
            global: low,
        };
        let limited = MemoryAuthStateRepository::default();
        assert_eq!(
            limited
                .begin_oauth(OAuthBeginCommand {
                    flow: oauth_flow('3', 1, 10),
                    initiator: None,
                    abuse: oauth_abuse('3'),
                    rate_policy: low_policy,
                    audit: audit(AuthAuditAction::OAuthBegin, 1),
                })
                .await
                .expect("first begin"),
            OAuthBeginOutcome::Started
        );
        assert!(matches!(
            limited
                .begin_oauth(OAuthBeginCommand {
                    flow: oauth_flow('4', 1, 10),
                    initiator: None,
                    abuse: oauth_abuse('4'),
                    rate_policy: low_policy,
                    audit: audit(AuthAuditAction::OAuthBegin, 1),
                })
                .await
                .expect("limited begin"),
            OAuthBeginOutcome::RateLimited { .. }
        ));
    }

    async fn assert_dimension_limits(dimension: AbuseDimension) {
        let repository = MemoryAuthStateRepository::default();
        let low =
            frame_domain::RateLimitPolicy::new(1, duration(10), duration(10)).expect("low policy");
        let high = frame_domain::RateLimitPolicy::new(10, duration(10), duration(10))
            .expect("high policy");
        let policy = MultiRateLimitPolicy {
            identifier: if dimension == AbuseDimension::Identifier {
                low
            } else {
                high
            },
            source: if dimension == AbuseDimension::Source {
                low
            } else {
                high
            },
            device: if dimension == AbuseDimension::Device {
                low
            } else {
                high
            },
            global: if dimension == AbuseDimension::Global {
                low
            } else {
                high
            },
        };
        for index in 0..2 {
            let identifier = candidates(
                digest(
                    1,
                    if dimension == AbuseDimension::Identifier || index == 0 {
                        '2'
                    } else {
                        '3'
                    },
                ),
                vec![],
            );
            let source = candidates(
                digest(
                    1,
                    if dimension == AbuseDimension::Source || index == 0 {
                        '4'
                    } else {
                        '5'
                    },
                ),
                vec![],
            );
            let device = candidates(
                digest(
                    1,
                    if dimension == AbuseDimension::Device || index == 0 {
                        '6'
                    } else {
                        '7'
                    },
                ),
                vec![],
            );
            let outcome = repository
                .issue_verification(VerificationIssueCommand {
                    identifier_digests: identifier.clone(),
                    secret_digest: digest(1, if index == 0 { 'a' } else { 'b' }),
                    purpose: VerificationPurpose::SignIn,
                    channel: VerificationChannel::MagicLink,
                    initiated_by: None,
                    initiator_grant: None,
                    provisioning: None,
                    max_attempts: 3,
                    expires_at: time(10),
                    sealed_delivery: SealedDeliveryEnvelope::new(
                        vec![if index == 0 { b'a' } else { b'b' }; 64],
                        time(1),
                    )
                    .expect("envelope"),
                    abuse: AbuseDigestSet {
                        identifier,
                        source,
                        device,
                    },
                    rate_policy: policy,
                    audit: audit(AuthAuditAction::VerificationIssue, 1),
                })
                .await
                .expect("issue");
            if index == 0 {
                assert_eq!(outcome, VerificationIssueAtomicOutcome::Accepted);
            } else {
                assert!(matches!(
                    outcome,
                    VerificationIssueAtomicOutcome::RateLimited { .. }
                ));
            }
        }
    }

    #[tokio::test]
    async fn identifier_source_device_and_global_limits_each_block_independently() {
        for dimension in [
            AbuseDimension::Identifier,
            AbuseDimension::Source,
            AbuseDimension::Device,
            AbuseDimension::Global,
        ] {
            assert_dimension_limits(dimension).await;
        }
    }

    #[test]
    fn global_limits_short_circuit_cardinality_and_rate_buckets_are_bounded_and_collected() {
        let low =
            frame_domain::RateLimitPolicy::new(1, duration(10), duration(10)).expect("low policy");
        let high = frame_domain::RateLimitPolicy::new(1_000_000, duration(10), duration(10))
            .expect("high policy");
        let mut global_first = MemoryAuthState::default();
        let global_policy = MultiRateLimitPolicy {
            identifier: high,
            source: high,
            device: high,
            global: low,
        };
        assert_eq!(
            consume_limits(
                &mut global_first,
                AuthAbuseAction::SignInIssue,
                &oauth_abuse('1'),
                global_policy,
                time(1),
            )
            .expect("first"),
            None
        );
        let allocated = global_first.rate_limits.len();
        assert!(
            consume_limits(
                &mut global_first,
                AuthAbuseAction::SignInIssue,
                &oauth_abuse('2'),
                global_policy,
                time(1),
            )
            .expect("limited")
            .is_some()
        );
        assert_eq!(global_first.rate_limits.len(), allocated);

        let mut bounded = MemoryAuthState::default();
        let high_policy = MultiRateLimitPolicy {
            identifier: high,
            source: high,
            device: high,
            global: high,
        };
        let attempts = (MAX_RATE_LIMIT_BUCKETS - 1) / 3;
        for index in 0..attempts {
            let index = u64::try_from(index).expect("index");
            let abuse = AbuseDigestSet {
                identifier: candidates(indexed_digest(index * 3), vec![]),
                source: candidates(indexed_digest(index * 3 + 1), vec![]),
                device: candidates(indexed_digest(index * 3 + 2), vec![]),
            };
            assert_eq!(
                consume_limits(
                    &mut bounded,
                    AuthAbuseAction::SignInIssue,
                    &abuse,
                    high_policy,
                    time(1),
                )
                .expect("consume"),
                None
            );
        }
        assert_eq!(bounded.rate_limits.len(), MAX_RATE_LIMIT_BUCKETS);
        let overflow = u64::try_from(attempts).expect("index") * 3;
        let overflow_abuse = AbuseDigestSet {
            identifier: candidates(indexed_digest(overflow), vec![]),
            source: candidates(indexed_digest(overflow + 1), vec![]),
            device: candidates(indexed_digest(overflow + 2), vec![]),
        };
        assert!(
            consume_limits(
                &mut bounded,
                AuthAbuseAction::SignInIssue,
                &overflow_abuse,
                high_policy,
                time(1),
            )
            .expect("bounded")
            .is_some()
        );
        assert_eq!(bounded.rate_limits.len(), MAX_RATE_LIMIT_BUCKETS);
        assert_eq!(
            consume_limits(
                &mut bounded,
                AuthAbuseAction::SignInIssue,
                &overflow_abuse,
                high_policy,
                time(21),
            )
            .expect("after ttl"),
            None
        );
        assert_eq!(bounded.rate_limits.len(), 4);
    }

    #[tokio::test]
    async fn hash_rotation_merges_rate_limit_history_instead_of_splitting_the_budget() {
        let repository = MemoryAuthStateRepository::default();
        let low =
            frame_domain::RateLimitPolicy::new(2, duration(20), duration(20)).expect("low policy");
        let high = frame_domain::RateLimitPolicy::new(20, duration(20), duration(20))
            .expect("high policy");
        let policy = MultiRateLimitPolicy {
            identifier: low,
            source: high,
            device: high,
            global: high,
        };
        let identifier_v1 = digest(1, '1');
        let identifier_v2 = digest(2, '2');
        for (index, identifier) in [identifier_v1.clone(), identifier_v2.clone()]
            .into_iter()
            .enumerate()
        {
            let identifier = candidates(identifier, vec![]);
            assert_eq!(
                repository
                    .issue_verification(VerificationIssueCommand {
                        identifier_digests: identifier.clone(),
                        secret_digest: digest(1, if index == 0 { 'a' } else { 'b' }),
                        purpose: VerificationPurpose::SignIn,
                        channel: VerificationChannel::MagicLink,
                        initiated_by: None,
                        initiator_grant: None,
                        provisioning: None,
                        max_attempts: 3,
                        expires_at: time(10),
                        sealed_delivery: SealedDeliveryEnvelope::new(
                            vec![if index == 0 { b'a' } else { b'b' }; 64],
                            time(1),
                        )
                        .expect("envelope"),
                        abuse: AbuseDigestSet {
                            identifier,
                            source: candidates(
                                digest(1, if index == 0 { '3' } else { '4' }),
                                vec![],
                            ),
                            device: candidates(
                                digest(1, if index == 0 { '5' } else { '6' }),
                                vec![],
                            ),
                        },
                        rate_policy: policy,
                        audit: audit(AuthAuditAction::VerificationIssue, 1),
                    })
                    .await
                    .expect("issue"),
                VerificationIssueAtomicOutcome::Accepted
            );
        }

        let rotated_identifier = candidates(identifier_v2, vec![identifier_v1]);
        assert!(matches!(
            repository
                .issue_verification(VerificationIssueCommand {
                    identifier_digests: rotated_identifier.clone(),
                    secret_digest: digest(1, 'c'),
                    purpose: VerificationPurpose::SignIn,
                    channel: VerificationChannel::MagicLink,
                    initiated_by: None,
                    initiator_grant: None,
                    provisioning: None,
                    max_attempts: 3,
                    expires_at: time(10),
                    sealed_delivery: SealedDeliveryEnvelope::new(vec![b'c'; 64], time(1))
                        .expect("envelope"),
                    abuse: AbuseDigestSet {
                        identifier: rotated_identifier,
                        source: candidates(digest(1, '7'), vec![]),
                        device: candidates(digest(1, '8'), vec![]),
                    },
                    rate_policy: policy,
                    audit: audit(AuthAuditAction::VerificationIssue, 1),
                })
                .await
                .expect("issue"),
            VerificationIssueAtomicOutcome::RateLimited { .. }
        ));
    }

    #[tokio::test]
    async fn audit_failure_rolls_back_verification_rate_and_outbox_together() {
        let (repository, _, identifier) = provisioned_repository();
        let identifiers = candidates(identifier, vec![]);
        let policy =
            frame_domain::RateLimitPolicy::new(10, duration(10), duration(10)).expect("policy");
        repository.fail_next_atomic_audit_for_test();
        assert!(
            repository
                .issue_verification(VerificationIssueCommand {
                    identifier_digests: identifiers.clone(),
                    secret_digest: digest(1, 'a'),
                    purpose: VerificationPurpose::SignIn,
                    channel: VerificationChannel::OneTimeCode,
                    initiated_by: None,
                    initiator_grant: None,
                    provisioning: None,
                    max_attempts: 3,
                    expires_at: time(10),
                    sealed_delivery: SealedDeliveryEnvelope::new(vec![b'x'; 64], time(1))
                        .expect("envelope"),
                    abuse: AbuseDigestSet {
                        identifier: identifiers,
                        source: candidates(digest(1, 'b'), vec![]),
                        device: candidates(digest(1, 'c'), vec![]),
                    },
                    rate_policy: MultiRateLimitPolicy {
                        identifier: policy,
                        source: policy,
                        device: policy,
                        global: policy,
                    },
                    audit: audit(AuthAuditAction::VerificationIssue, 1),
                })
                .await
                .is_err()
        );
        assert_eq!(
            repository.security_state_counts().expect("counts"),
            (0, 0, 0, 0)
        );
        assert_eq!(repository.delivery_counts().expect("delivery"), (0, 0));
    }

    #[test]
    fn injected_clock_and_secret_source_remain_explicitly_test_only() {
        let clock = ManualClock::new(time(10));
        assert_eq!(clock.advance(5).expect("advance"), time(15));
        clock.set(time(MAX_TIMESTAMP_MS));
        assert!(clock.advance(1).is_err());
        let source = DeterministicAuthSecretSource::default();
        assert_eq!(
            format!("{:?}", source.session_token().expect("token")),
            "OpaqueAuthToken([redacted])"
        );
        assert_eq!(
            format!("{:?}", source.api_key().expect("key")),
            "ApiKeySecret([redacted])"
        );
    }
}
