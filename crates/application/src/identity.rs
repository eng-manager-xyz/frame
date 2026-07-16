use std::fmt;

use frame_domain::{
    AbuseSignal, ApiKeyId, ApiKeyScope, ApiKeySecret, AuthAuditAction, AuthClientKind,
    AuthSessionRecord, CorrelationId, CsrfToken, DeliveryDestinationRef, DurationMillis,
    ExactBrowserOrigin, ExactOAuthCallbackUrl, FetchSite, HashKeyVersion, HostOnlySessionCookie,
    ManagedApiKeyRecord, MultiRateLimitPolicy, NewAuthSession, OAuthAudience, OAuthFlowId,
    OAuthFlowPurpose, OAuthFlowRecord, OAuthProvider, OAuthState, OpaqueAuthToken, PkceChallenge,
    PkceVerifier, PrincipalSnapshot, SecretDigest, SecretDigestCandidates,
    SessionContinuationBinding, SessionFamilyId, SessionId, SessionRevocationReason, TenantId,
    TimestampMillis, UserId, VerificationChannel, VerificationDestination, VerificationPurpose,
    VerificationSecret, VersionedSecretDigest,
};
use frame_ports::{
    AbuseDigestSet, ApiKeyAuthenticationCommand, ApiKeyAuthenticationOutcome, ApiKeyIssueCommand,
    ApiKeyIssueOutcome, ApiKeyRevokeCommand, AuthDeliverySealer, AuthSecretSource,
    AuthStateRepository, BrowserBoundaryRequest, Clock, DecisionAudit, IdentityProvisionCommand,
    IdentityProvisionOutcome, IdentityProvisioningGrant, IdentityProvisioningIntent,
    LogoutAllOutcome, OAuthBeginCommand, OAuthBeginOutcome, OAuthCallback, OAuthExchangeOutcome,
    OAuthFinalizeCommand, OAuthIdentityVerifier, OAuthPreflightCommand, OAuthPreflightOutcome,
    OAuthProviderExchange, OAuthProviderResult, PrincipalIssuanceGrant,
    SessionAuthenticationCommand, SessionIssueAuthority, SessionIssueCommand, SessionIssueOutcome,
    SessionMutationGrant, SessionPresentation, SessionRevokeCommand, SessionRotationOutcome,
    SessionRotationRequest, VerificationAtomicOutcome, VerificationAttemptCommand,
    VerificationDeliveryMaterial, VerificationIssueAtomicOutcome, VerificationIssueCommand,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use frame_domain::PkceChallengeMethod;

#[derive(Clone, PartialEq, Eq)]
pub struct AuthHashKey {
    version: HashKeyVersion,
    material: Vec<u8>,
}

impl AuthHashKey {
    pub fn new(version: HashKeyVersion, material: impl Into<Vec<u8>>) -> Result<Self, AuthFailure> {
        let material = material.into();
        if !(32..=128).contains(&material.len()) {
            return Err(AuthFailure::InvalidRequest);
        }
        Ok(Self { version, material })
    }

    #[must_use]
    pub const fn version(&self) -> HashKeyVersion {
        self.version
    }
}

impl fmt::Debug for AuthHashKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthHashKey")
            .field("version", &self.version)
            .field("material", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthHashKeyRing {
    active: AuthHashKey,
    fallback: Vec<AuthHashKey>,
}

impl AuthHashKeyRing {
    pub fn new(active: AuthHashKey, fallback: Vec<AuthHashKey>) -> Result<Self, AuthFailure> {
        if fallback.len() > 4
            || fallback.iter().any(|key| key.version == active.version)
            || fallback.iter().enumerate().any(|(index, key)| {
                fallback[..index]
                    .iter()
                    .any(|other| other.version == key.version)
            })
        {
            return Err(AuthFailure::InvalidRequest);
        }
        Ok(Self { active, fallback })
    }

    #[must_use]
    pub const fn active_version(&self) -> HashKeyVersion {
        self.active.version
    }
}

impl fmt::Debug for AuthHashKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthHashKeyRing")
            .field("active_version", &self.active.version)
            .field("fallback_count", &self.fallback.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPolicy {
    session_idle_ttl: DurationMillis,
    session_absolute_ttl: DurationMillis,
    verification_ttl: DurationMillis,
    verification_max_attempts: u16,
    issue_rate_limit: MultiRateLimitPolicy,
    verify_rate_limit: MultiRateLimitPolicy,
    api_key_rate_limit: MultiRateLimitPolicy,
    oauth_rate_limit: MultiRateLimitPolicy,
    oauth_ttl: DurationMillis,
    oauth_providers: Vec<OAuthProviderPolicy>,
    browser_origin: ExactBrowserOrigin,
    session_cookie: HostOnlySessionCookie,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderPolicy {
    pub provider: OAuthProvider,
    pub callback_url: ExactOAuthCallbackUrl,
    pub audience: OAuthAudience,
}

impl AuthPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_idle_ttl: DurationMillis,
        session_absolute_ttl: DurationMillis,
        verification_ttl: DurationMillis,
        verification_max_attempts: u16,
        issue_rate_limit: MultiRateLimitPolicy,
        verify_rate_limit: MultiRateLimitPolicy,
        api_key_rate_limit: MultiRateLimitPolicy,
        oauth_rate_limit: MultiRateLimitPolicy,
        oauth_ttl: DurationMillis,
        oauth_providers: Vec<OAuthProviderPolicy>,
        browser_origin: ExactBrowserOrigin,
    ) -> Result<Self, AuthFailure> {
        if session_idle_ttl > session_absolute_ttl
            || verification_max_attempts == 0
            || verification_max_attempts > 100
            || oauth_providers.is_empty()
            || oauth_providers.iter().enumerate().any(|(index, policy)| {
                oauth_providers[index + 1..]
                    .iter()
                    .any(|candidate| candidate.provider == policy.provider)
            })
        {
            return Err(AuthFailure::InvalidRequest);
        }
        Ok(Self {
            session_idle_ttl,
            session_absolute_ttl,
            verification_ttl,
            verification_max_attempts,
            issue_rate_limit,
            verify_rate_limit,
            api_key_rate_limit,
            oauth_rate_limit,
            oauth_ttl,
            oauth_providers,
            browser_origin,
            session_cookie: HostOnlySessionCookie::new(session_absolute_ttl),
        })
    }

    #[must_use]
    pub const fn session_cookie(&self) -> &HostOnlySessionCookie {
        &self.session_cookie
    }

    fn oauth_provider(&self, provider: OAuthProvider) -> Option<&OAuthProviderPolicy> {
        self.oauth_providers
            .iter()
            .find(|policy| policy.provider == provider)
    }
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum AuthFailure {
    #[error("authentication is required")]
    Unauthenticated,
    #[error("the request was rejected")]
    RequestRejected,
    #[error("the request is invalid")]
    InvalidRequest,
    #[error("the request was rate limited")]
    RateLimited,
    #[error("the authentication service is temporarily unavailable")]
    Unavailable,
}

impl fmt::Debug for AuthFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthenticated => "Unauthenticated",
            Self::RequestRejected => "RequestRejected",
            Self::InvalidRequest => "InvalidRequest",
            Self::RateLimited => "RateLimited",
            Self::Unavailable => "Unavailable",
        })
    }
}

impl From<frame_ports::PortError> for AuthFailure {
    fn from(_: frame_ports::PortError) -> Self {
        Self::Unavailable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalAssurance {
    Verification,
    Recovery,
    OAuth,
}

/// An unforgeable application capability. Its fields and constructor are private; it is emitted
/// only by successful one-time verification or OAuth exchange.
#[derive(PartialEq, Eq)]
pub struct VerifiedPrincipal {
    snapshot: PrincipalSnapshot,
    issuance_grant: PrincipalIssuanceGrant,
    assurance: PrincipalAssurance,
}

impl VerifiedPrincipal {
    #[must_use]
    pub const fn user_id(&self) -> frame_domain::UserId {
        self.snapshot.user_id
    }

    #[must_use]
    pub const fn assurance(&self) -> PrincipalAssurance {
        self.assurance
    }
}

impl fmt::Debug for VerifiedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedPrincipal")
            .field("user_id", &self.snapshot.user_id)
            .field("assurance", &self.assurance)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedSession {
    pub session_id: SessionId,
    pub token: OpaqueAuthToken,
    pub csrf_token: Option<CsrfToken>,
    pub idle_expires_at: TimestampMillis,
    pub absolute_expires_at: TimestampMillis,
    pub generation: u64,
    pub cookie: Option<HostOnlySessionCookie>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedIdentity {
    snapshot: PrincipalSnapshot,
    client_kind: AuthClientKind,
    session_id: Option<SessionId>,
}

impl AuthenticatedIdentity {
    #[must_use]
    pub const fn user_id(&self) -> frame_domain::UserId {
        self.snapshot.user_id
    }

    #[must_use]
    pub const fn client_kind(&self) -> AuthClientKind {
        self.client_kind
    }

    #[must_use]
    pub const fn session_id(&self) -> Option<SessionId> {
        self.session_id
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BrowserMutationRequest<'a> {
    pub origin: &'a str,
    pub fetch_site: FetchSite,
    pub csrf_cookie: &'a CsrfToken,
    pub csrf_header: &'a CsrfToken,
}

/// Unforgeable proof that cookie, CSRF, exact Origin, Fetch Metadata, session state, and current
/// generation were validated together. Destructive session operations accept only this type.
pub struct ValidatedBrowserMutationProof {
    grant: SessionMutationGrant,
    principal: PrincipalSnapshot,
}

impl fmt::Debug for ValidatedBrowserMutationProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedBrowserMutationProof")
            .field("session_id", &self.grant.session_id())
            .field("user_id", &self.principal.user_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoutAllReceipt {
    pub new_session_version: u64,
    pub revoked_sessions: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct AbuseContext<'a> {
    pub source: &'a AbuseSignal,
    pub device: &'a AbuseSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationIssueReceipt {
    Accepted,
    RateLimited { retry_at: TimestampMillis },
}

impl VerificationIssueReceipt {
    #[must_use]
    pub const fn public_disposition(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::RateLimited { .. } => "rate_limited",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerificationConsumeOutcome {
    Verified(VerifiedPrincipal),
    ProvisioningAuthorized(VerifiedIdentityProvisioning),
    Linked { user_id: UserId },
    Rejected,
    RateLimited { retry_at: TimestampMillis },
}

#[derive(PartialEq, Eq)]
pub struct VerifiedIdentityProvisioning {
    grant: IdentityProvisioningGrant,
}

impl VerifiedIdentityProvisioning {
    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.grant.user_id()
    }
}

impl fmt::Debug for VerifiedIdentityProvisioning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedIdentityProvisioning")
            .field("user_id", &self.grant.user_id())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedApiKey {
    pub id: ApiKeyId,
    pub secret: ApiKeySecret,
    pub tenant_id: TenantId,
    pub scopes: Vec<ApiKeyScope>,
    pub expires_at: Option<TimestampMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStart {
    pub flow_id: OAuthFlowId,
    pub state: OAuthState,
    pub pkce_verifier: PkceVerifier,
    pub pkce_challenge: PkceChallenge,
    pub pkce_challenge_method: PkceChallengeMethod,
    pub callback_url: ExactOAuthCallbackUrl,
    pub audience: OAuthAudience,
    pub expires_at: TimestampMillis,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OAuthCompletionOutcome {
    Verified(VerifiedPrincipal),
    Linked { user_id: UserId },
}

pub struct AuthService<'a, R: ?Sized, C: ?Sized, S: ?Sized, D: ?Sized> {
    repository: &'a R,
    clock: &'a C,
    secrets: &'a S,
    delivery_sealer: &'a D,
    hash_keys: AuthHashKeyRing,
    policy: AuthPolicy,
}

impl<'a, R, C, S, D> AuthService<'a, R, C, S, D>
where
    R: AuthStateRepository + ?Sized,
    C: Clock + ?Sized,
    S: AuthSecretSource + ?Sized,
    D: AuthDeliverySealer + ?Sized,
{
    #[must_use]
    pub fn new(
        repository: &'a R,
        clock: &'a C,
        secrets: &'a S,
        delivery_sealer: &'a D,
        hash_keys: AuthHashKeyRing,
        policy: AuthPolicy,
    ) -> Self {
        Self {
            repository,
            clock,
            secrets,
            delivery_sealer,
            hash_keys,
            policy,
        }
    }

    pub async fn issue_session(
        &self,
        principal: VerifiedPrincipal,
        client_kind: AuthClientKind,
        correlation_id: CorrelationId,
    ) -> Result<IssuedSession, AuthFailure> {
        self.issue_session_for_snapshot(
            &principal.snapshot,
            SessionIssueAuthority::Verified(principal.issuance_grant),
            client_kind,
            correlation_id,
        )
        .await
    }

    pub async fn issue_additional_session(
        &self,
        proof: ValidatedBrowserMutationProof,
        client_kind: AuthClientKind,
        correlation_id: CorrelationId,
    ) -> Result<IssuedSession, AuthFailure> {
        self.issue_session_for_snapshot(
            &proof.principal,
            SessionIssueAuthority::ExistingSession(proof.grant),
            client_kind,
            correlation_id,
        )
        .await
    }

    async fn issue_session_for_snapshot(
        &self,
        principal: &PrincipalSnapshot,
        authority: SessionIssueAuthority,
        client_kind: AuthClientKind,
        correlation_id: CorrelationId,
    ) -> Result<IssuedSession, AuthFailure> {
        let now = self.now()?;
        let session_id = SessionId::new();
        let token = self.secrets.session_token().map_err(AuthFailure::from)?;
        let csrf_token = if client_kind == AuthClientKind::Browser {
            Some(self.secrets.csrf_token().map_err(AuthFailure::from)?)
        } else {
            None
        };
        let token_digest =
            self.active_digest(b"frame/session/v2", &[token.expose_for_hashing()])?;
        let csrf_digest = csrf_token
            .as_ref()
            .map(|csrf| self.active_digest(b"frame/csrf/v2", &[csrf.expose_for_hashing()]))
            .transpose()?;
        let absolute_expires_at = now
            .checked_add(self.policy.session_absolute_ttl)
            .map_err(|_| AuthFailure::InvalidRequest)?;
        let idle_expires_at = now
            .checked_add(self.policy.session_idle_ttl)
            .map_err(|_| AuthFailure::InvalidRequest)?
            .min(absolute_expires_at);
        let session = AuthSessionRecord::new(NewAuthSession {
            id: session_id,
            family_id: SessionFamilyId::new(),
            user_id: principal.user_id,
            client_kind,
            token_digest,
            csrf_digest,
            browser_origin: (client_kind == AuthClientKind::Browser)
                .then(|| self.policy.browser_origin.clone()),
            issued_at: now,
            idle_expires_at,
            absolute_expires_at,
            session_version: 0,
        })
        .map_err(|_| AuthFailure::InvalidRequest)?;
        let outcome = self
            .repository
            .issue_auth_session(SessionIssueCommand {
                principal: principal.clone(),
                authority,
                session,
                audit: self.audit(correlation_id, AuthAuditAction::SessionIssue, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        if let SessionIssueOutcome::Denied(_) = outcome {
            return Err(AuthFailure::Unauthenticated);
        }
        Ok(IssuedSession {
            session_id,
            token,
            csrf_token,
            idle_expires_at,
            absolute_expires_at,
            generation: 0,
            cookie: (client_kind == AuthClientKind::Browser)
                .then(|| self.policy.session_cookie.clone()),
        })
    }

    pub async fn authenticate(
        &self,
        token: &OpaqueAuthToken,
        correlation_id: CorrelationId,
    ) -> Result<AuthenticatedIdentity, AuthFailure> {
        let now = self.now()?;
        let presentation = self
            .repository
            .authenticate_session(SessionAuthenticationCommand {
                token_digests: self
                    .digest_candidates(b"frame/session/v2", &[token.expose_for_hashing()])?,
                browser_boundary: None,
                audit: self.audit(correlation_id, AuthAuditAction::SessionAuthenticate, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        match presentation {
            SessionPresentation::Authenticated(presentation) => Ok(AuthenticatedIdentity {
                snapshot: presentation.principal().clone(),
                client_kind: presentation.client_kind(),
                session_id: Some(presentation.session_id()),
            }),
            _ => Err(AuthFailure::Unauthenticated),
        }
    }

    pub async fn validate_browser_mutation(
        &self,
        token: &OpaqueAuthToken,
        request: BrowserMutationRequest<'_>,
        correlation_id: CorrelationId,
    ) -> Result<ValidatedBrowserMutationProof, AuthFailure> {
        let now = self.now()?;
        let csrf_header_digests = self.digest_candidates(
            b"frame/csrf/v2",
            &[request.csrf_header.expose_for_hashing()],
        )?;
        let boundary = BrowserBoundaryRequest {
            origin: ExactBrowserOrigin::parse(request.origin).ok(),
            fetch_site: request.fetch_site,
            csrf_cookie_digests: self.digest_candidates(
                b"frame/csrf/v2",
                &[request.csrf_cookie.expose_for_hashing()],
            )?,
            csrf_header_digests,
        };
        let presentation = self
            .repository
            .authenticate_session(SessionAuthenticationCommand {
                token_digests: self
                    .digest_candidates(b"frame/session/v2", &[token.expose_for_hashing()])?,
                browser_boundary: Some(boundary),
                audit: self.audit(
                    correlation_id,
                    AuthAuditAction::BrowserMutationAuthenticate,
                    now,
                ),
            })
            .await
            .map_err(AuthFailure::from)?;
        match presentation {
            SessionPresentation::Authenticated(presentation) => {
                let (principal, mutation_grant) = presentation.into_parts();
                mutation_grant.map_or(Err(AuthFailure::Unauthenticated), |grant| {
                    Ok(ValidatedBrowserMutationProof { grant, principal })
                })
            }
            SessionPresentation::BoundaryRejected(_, _) => Err(AuthFailure::RequestRejected),
            _ => Err(AuthFailure::Unauthenticated),
        }
    }

    pub async fn rotate_session(
        &self,
        proof: ValidatedBrowserMutationProof,
        correlation_id: CorrelationId,
    ) -> Result<IssuedSession, AuthFailure> {
        let now = self.now()?;
        let token = self.secrets.session_token().map_err(AuthFailure::from)?;
        let csrf = self.secrets.csrf_token().map_err(AuthFailure::from)?;
        let idle_expires_at = now
            .checked_add(self.policy.session_idle_ttl)
            .map_err(|_| AuthFailure::InvalidRequest)?;
        let outcome = self
            .repository
            .rotate_auth_session(SessionRotationRequest {
                grant: proof.grant,
                next_token_digest: self
                    .active_digest(b"frame/session/v2", &[token.expose_for_hashing()])?,
                next_csrf_digest: self
                    .active_digest(b"frame/csrf/v2", &[csrf.expose_for_hashing()])?,
                now,
                idle_expires_at,
                audit: self.audit(correlation_id, AuthAuditAction::SessionRotate, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        match outcome {
            SessionRotationOutcome::Rotated(session) => Ok(IssuedSession {
                session_id: session.id,
                token,
                csrf_token: Some(csrf),
                idle_expires_at: session.idle_expires_at,
                absolute_expires_at: session.absolute_expires_at,
                generation: session.generation,
                cookie: Some(self.policy.session_cookie.clone()),
            }),
            SessionRotationOutcome::Denied(_) => Err(AuthFailure::Unauthenticated),
        }
    }

    pub async fn logout(
        &self,
        proof: ValidatedBrowserMutationProof,
        correlation_id: CorrelationId,
    ) -> Result<(), AuthFailure> {
        let now = self.now()?;
        self.repository
            .revoke_auth_session(SessionRevokeCommand {
                grant: proof.grant,
                reason: SessionRevocationReason::UserLogout,
                audit: self.audit(correlation_id, AuthAuditAction::Logout, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        Ok(())
    }

    pub async fn logout_all(
        &self,
        proof: ValidatedBrowserMutationProof,
        correlation_id: CorrelationId,
    ) -> Result<LogoutAllReceipt, AuthFailure> {
        let now = self.now()?;
        let outcome = self
            .repository
            .revoke_all_auth_sessions(SessionRevokeCommand {
                grant: proof.grant,
                reason: SessionRevocationReason::LogoutAll,
                audit: self.audit(correlation_id, AuthAuditAction::LogoutAll, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        match outcome {
            LogoutAllOutcome::Revoked {
                new_session_version,
                revoked_sessions,
            } => Ok(LogoutAllReceipt {
                new_session_version,
                revoked_sessions,
            }),
            LogoutAllOutcome::Denied(_) => Err(AuthFailure::Unauthenticated),
        }
    }

    pub async fn issue_verification(
        &self,
        identifier: &str,
        purpose: VerificationPurpose,
        channel: VerificationChannel,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<VerificationIssueReceipt, AuthFailure> {
        if matches!(
            purpose,
            VerificationPurpose::AccountLink | VerificationPurpose::IdentityProvisioning
        ) {
            return Err(AuthFailure::InvalidRequest);
        }
        self.issue_verification_inner(
            identifier,
            purpose,
            channel,
            None,
            None,
            None,
            abuse,
            correlation_id,
        )
        .await
    }

    pub async fn issue_account_link_verification(
        &self,
        proof: ValidatedBrowserMutationProof,
        identifier: &str,
        channel: VerificationChannel,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<VerificationIssueReceipt, AuthFailure> {
        self.issue_verification_inner(
            identifier,
            VerificationPurpose::AccountLink,
            channel,
            Some(proof.principal.clone()),
            Some(proof.grant),
            None,
            abuse,
            correlation_id,
        )
        .await
    }

    pub async fn issue_identity_provisioning_verification(
        &self,
        identifier: &str,
        user_id: UserId,
        identity_revision: u64,
        channel: VerificationChannel,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<VerificationIssueReceipt, AuthFailure> {
        if identity_revision == 0 {
            return Err(AuthFailure::InvalidRequest);
        }
        self.issue_verification_inner(
            identifier,
            VerificationPurpose::IdentityProvisioning,
            channel,
            None,
            None,
            Some(IdentityProvisioningIntent {
                user_id,
                identity_revision,
            }),
            abuse,
            correlation_id,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn issue_verification_inner(
        &self,
        identifier: &str,
        purpose: VerificationPurpose,
        channel: VerificationChannel,
        initiated_by: Option<PrincipalSnapshot>,
        initiator_grant: Option<SessionMutationGrant>,
        provisioning: Option<IdentityProvisioningIntent>,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<VerificationIssueReceipt, AuthFailure> {
        let now = self.now()?;
        let normalized_identifier = normalize_identifier(identifier)?;
        let identifier_digests = self.hash_identifier(&normalized_identifier)?;
        let secret = self
            .secrets
            .verification_secret(channel)
            .map_err(AuthFailure::from)?;
        let expires_at = now
            .checked_add(self.policy.verification_ttl)
            .map_err(|_| AuthFailure::InvalidRequest)?;
        let material = VerificationDeliveryMaterial {
            destination: VerificationDestination::parse(normalized_identifier)
                .map_err(|_| AuthFailure::InvalidRequest)?,
            secret: secret.clone(),
            purpose,
            expires_at,
        };
        let sealed_delivery = self
            .delivery_sealer
            .seal(&material, now)
            .map_err(AuthFailure::from)?;
        let secret_digest = self
            .verification_secret_digests(&identifier_digests, purpose, channel, &secret)?
            .active()
            .clone();
        let outcome = self
            .repository
            .issue_verification(VerificationIssueCommand {
                identifier_digests: identifier_digests.clone(),
                secret_digest,
                purpose,
                channel,
                initiated_by,
                initiator_grant,
                provisioning,
                max_attempts: self.policy.verification_max_attempts,
                expires_at,
                sealed_delivery,
                abuse: self.hash_abuse(&identifier_digests, abuse)?,
                rate_policy: self.policy.issue_rate_limit,
                audit: self.audit(correlation_id, AuthAuditAction::VerificationIssue, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        Ok(match outcome {
            VerificationIssueAtomicOutcome::Accepted => VerificationIssueReceipt::Accepted,
            VerificationIssueAtomicOutcome::RateLimited { retry_at } => {
                VerificationIssueReceipt::RateLimited { retry_at }
            }
            VerificationIssueAtomicOutcome::Rejected(_) => {
                return Err(AuthFailure::RequestRejected);
            }
        })
    }

    pub async fn consume_verification(
        &self,
        identifier: &str,
        purpose: VerificationPurpose,
        secret: &VerificationSecret,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<VerificationConsumeOutcome, AuthFailure> {
        let now = self.now()?;
        let identifier_digests = self.hash_identifier(identifier)?;
        let secret_digests = self.verification_secret_digests(
            &identifier_digests,
            purpose,
            secret.channel(),
            secret,
        )?;
        self.repository
            .materialize_verification_deliveries(now, 100)
            .await
            .map_err(AuthFailure::from)?;
        let outcome = self
            .repository
            .attempt_verification(VerificationAttemptCommand {
                identifier_digests: identifier_digests.clone(),
                secret_digests,
                purpose,
                abuse: self.hash_abuse(&identifier_digests, abuse)?,
                rate_policy: self.policy.verify_rate_limit,
                audit: self.audit(
                    correlation_id,
                    if purpose == VerificationPurpose::AccountLink {
                        AuthAuditAction::AccountLink
                    } else {
                        AuthAuditAction::VerificationConsume
                    },
                    now,
                ),
            })
            .await
            .map_err(AuthFailure::from)?;
        Ok(match outcome {
            VerificationAtomicOutcome::Verified {
                principal: snapshot,
                issuance_grant,
            } => VerificationConsumeOutcome::Verified(VerifiedPrincipal {
                snapshot,
                issuance_grant,
                assurance: if purpose == VerificationPurpose::AccountRecovery {
                    PrincipalAssurance::Recovery
                } else {
                    PrincipalAssurance::Verification
                },
            }),
            VerificationAtomicOutcome::ProvisioningAuthorized(grant) => {
                VerificationConsumeOutcome::ProvisioningAuthorized(VerifiedIdentityProvisioning {
                    grant,
                })
            }
            VerificationAtomicOutcome::Linked { user_id } => {
                VerificationConsumeOutcome::Linked { user_id }
            }
            VerificationAtomicOutcome::Rejected(_) => VerificationConsumeOutcome::Rejected,
            VerificationAtomicOutcome::RateLimited { retry_at } => {
                VerificationConsumeOutcome::RateLimited { retry_at }
            }
        })
    }

    pub async fn provision_identity(
        &self,
        verified: VerifiedIdentityProvisioning,
        destination: DeliveryDestinationRef,
        correlation_id: CorrelationId,
    ) -> Result<UserId, AuthFailure> {
        let now = self.now()?;
        let user_id = verified.grant.user_id();
        match self
            .repository
            .provision_identity(IdentityProvisionCommand {
                grant: verified.grant,
                destination,
                audit: self.audit(correlation_id, AuthAuditAction::IdentityProvision, now),
            })
            .await
            .map_err(AuthFailure::from)?
        {
            IdentityProvisionOutcome::Created => Ok(user_id),
            IdentityProvisionOutcome::Rejected(_) => Err(AuthFailure::Unauthenticated),
        }
    }

    pub async fn issue_api_key(
        &self,
        proof: ValidatedBrowserMutationProof,
        tenant_id: TenantId,
        scopes: Vec<ApiKeyScope>,
        expires_at: Option<TimestampMillis>,
        correlation_id: CorrelationId,
    ) -> Result<IssuedApiKey, AuthFailure> {
        let now = self.now()?;
        if scopes.is_empty() || expires_at.is_some_and(|expires_at| expires_at <= now) {
            return Err(AuthFailure::InvalidRequest);
        }
        let material = self.secrets.api_key().map_err(AuthFailure::from)?;
        let id = ApiKeyId::new();
        let record = ManagedApiKeyRecord {
            id,
            owner_id: proof.principal.user_id,
            tenant_id,
            key_digest: self
                .active_digest(b"frame/api-key/v2", &[material.expose_for_hashing()])?,
            scopes: scopes.clone(),
            created_at: now,
            expires_at,
            revoked_at: None,
        };
        match self
            .repository
            .issue_api_key(ApiKeyIssueCommand {
                principal: proof.principal.clone(),
                grant: proof.grant,
                record,
                audit: self.audit(correlation_id, AuthAuditAction::ApiKeyIssue, now),
            })
            .await
            .map_err(AuthFailure::from)?
        {
            ApiKeyIssueOutcome::Issued => Ok(IssuedApiKey {
                id,
                secret: material,
                tenant_id,
                scopes,
                expires_at,
            }),
            ApiKeyIssueOutcome::Forbidden => Err(AuthFailure::RequestRejected),
        }
    }

    pub async fn authenticate_api_key(
        &self,
        secret: &ApiKeySecret,
        tenant_id: TenantId,
        required_scope: ApiKeyScope,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<AuthenticatedIdentity, AuthFailure> {
        let now = self.now()?;
        let key_digests =
            self.digest_candidates(b"frame/api-key/v2", &[secret.expose_for_hashing()])?;
        let outcome = self
            .repository
            .authenticate_api_key(ApiKeyAuthenticationCommand {
                key_digests: key_digests.clone(),
                tenant_id,
                required_scope,
                abuse: self.hash_abuse(&key_digests, abuse)?,
                rate_policy: self.policy.api_key_rate_limit,
                audit: self.audit(correlation_id, AuthAuditAction::ApiKeyAuthenticate, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        match outcome {
            ApiKeyAuthenticationOutcome::Authenticated(snapshot) => Ok(AuthenticatedIdentity {
                snapshot,
                client_kind: AuthClientKind::Api,
                session_id: None,
            }),
            ApiKeyAuthenticationOutcome::Rejected(_) => Err(AuthFailure::Unauthenticated),
            ApiKeyAuthenticationOutcome::RateLimited { .. } => Err(AuthFailure::RateLimited),
        }
    }

    pub async fn revoke_api_key(
        &self,
        proof: ValidatedBrowserMutationProof,
        key_id: ApiKeyId,
        correlation_id: CorrelationId,
    ) -> Result<bool, AuthFailure> {
        let now = self.now()?;
        self.repository
            .revoke_api_key(ApiKeyRevokeCommand {
                grant: proof.grant,
                key_id,
                audit: self.audit(correlation_id, AuthAuditAction::ApiKeyRevoke, now),
            })
            .await
            .map_err(AuthFailure::from)
    }

    pub async fn begin_oauth(
        &self,
        provider: OAuthProvider,
        purpose: OAuthFlowPurpose,
        initiator: Option<ValidatedBrowserMutationProof>,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<OAuthStart, AuthFailure> {
        if (purpose == OAuthFlowPurpose::AccountLink) != initiator.is_some() {
            return Err(AuthFailure::InvalidRequest);
        }
        let now = self.now()?;
        let provider_policy = self
            .policy
            .oauth_provider(provider)
            .cloned()
            .ok_or(AuthFailure::InvalidRequest)?;
        let state = self.secrets.oauth_state().map_err(AuthFailure::from)?;
        let pkce_verifier = self.secrets.pkce_verifier().map_err(AuthFailure::from)?;
        let pkce_challenge = pkce_s256_challenge(&pkce_verifier)?;
        let expires_at = now
            .checked_add(self.policy.oauth_ttl)
            .map_err(|_| AuthFailure::InvalidRequest)?;
        let flow = OAuthFlowRecord {
            id: OAuthFlowId::new(),
            provider,
            purpose,
            initiator: initiator.as_ref().map(|proof| SessionContinuationBinding {
                session_id: proof.grant.session_id(),
                user_id: proof.principal.user_id,
                generation: proof.grant.generation(),
            }),
            state_digest: self
                .active_digest(b"frame/oauth-state/v2", &[state.expose_for_hashing()])?,
            pkce_digest: self.active_digest(
                b"frame/oauth-pkce/v2",
                &[pkce_verifier.expose_for_hashing()],
            )?,
            redirect_digest: self.active_digest(
                b"frame/oauth-redirect/v2",
                &[provider_policy.callback_url.as_str().as_bytes()],
            )?,
            audience_digest: self.active_digest(
                b"frame/oauth-audience/v2",
                &[provider_policy.audience.as_str().as_bytes()],
            )?,
            created_at: now,
            expires_at,
            consumed_at: None,
            revoked: false,
        };
        let flow_id = flow.id;
        match self
            .repository
            .begin_oauth(OAuthBeginCommand {
                flow,
                initiator: initiator.map(|proof| proof.grant),
                abuse: self.hash_abuse(
                    &self.digest_candidates(
                        b"frame/oauth-state/v2",
                        &[state.expose_for_hashing()],
                    )?,
                    abuse,
                )?,
                rate_policy: self.policy.oauth_rate_limit,
                audit: self.audit(correlation_id, AuthAuditAction::OAuthBegin, now),
            })
            .await
            .map_err(AuthFailure::from)?
        {
            OAuthBeginOutcome::Started => {}
            OAuthBeginOutcome::RateLimited { .. } => return Err(AuthFailure::RateLimited),
            OAuthBeginOutcome::Rejected(_) => return Err(AuthFailure::RequestRejected),
        }
        Ok(OAuthStart {
            flow_id,
            state,
            pkce_verifier,
            pkce_challenge,
            pkce_challenge_method: PkceChallengeMethod::S256,
            callback_url: provider_policy.callback_url,
            audience: provider_policy.audience,
            expires_at,
        })
    }

    pub async fn exchange_oauth<V: OAuthIdentityVerifier + ?Sized>(
        &self,
        verifier: &V,
        callback: &OAuthCallback,
        state: &OAuthState,
        pkce_verifier: &PkceVerifier,
        abuse: AbuseContext<'_>,
        correlation_id: CorrelationId,
    ) -> Result<OAuthCompletionOutcome, AuthFailure> {
        let now = self.now()?;
        let provider_policy = self
            .policy
            .oauth_provider(callback.provider)
            .cloned()
            .ok_or(AuthFailure::InvalidRequest)?;
        let state_digests =
            self.digest_candidates(b"frame/oauth-state/v2", &[state.expose_for_hashing()])?;
        let preflight = self
            .repository
            .preflight_oauth_exchange(OAuthPreflightCommand {
                provider: callback.provider,
                state_digests: state_digests.clone(),
                pkce_digests: self.digest_candidates(
                    b"frame/oauth-pkce/v2",
                    &[pkce_verifier.expose_for_hashing()],
                )?,
                redirect_digests: self.digest_candidates(
                    b"frame/oauth-redirect/v2",
                    &[provider_policy.callback_url.as_str().as_bytes()],
                )?,
                audience_digests: self.digest_candidates(
                    b"frame/oauth-audience/v2",
                    &[provider_policy.audience.as_str().as_bytes()],
                )?,
                abuse: self.hash_abuse(&state_digests, abuse)?,
                rate_policy: self.policy.oauth_rate_limit,
                audit: self.audit(correlation_id, AuthAuditAction::OAuthExchangePreflight, now),
            })
            .await
            .map_err(AuthFailure::from)?;
        let reservation = match preflight {
            OAuthPreflightOutcome::Ready(reservation) => reservation,
            OAuthPreflightOutcome::Rejected(_) => return Err(AuthFailure::Unauthenticated),
            OAuthPreflightOutcome::RateLimited { .. } => return Err(AuthFailure::RateLimited),
        };
        let provider_result = verifier
            .verify(OAuthProviderExchange {
                callback,
                pkce_verifier,
                callback_url: &provider_policy.callback_url,
                audience: &provider_policy.audience,
            })
            .await;
        // Provider exchange is an unbounded external operation. Re-read the clock so the
        // repository cannot finalize a reservation with the stale preflight timestamp.
        let finalized_at = self.now()?;
        let repository_provider_result = match &provider_result {
            Ok(Some(assertion)) => OAuthProviderResult::Verified(assertion.clone()),
            Ok(None) => OAuthProviderResult::Rejected,
            Err(_) => OAuthProviderResult::AdapterFailure,
        };
        let outcome = self
            .repository
            .finalize_oauth_exchange(OAuthFinalizeCommand {
                reservation,
                provider_result: repository_provider_result,
                audit: self.audit(correlation_id, AuthAuditAction::OAuthExchange, finalized_at),
            })
            .await
            .map_err(AuthFailure::from)?;
        if provider_result.is_err() {
            return Err(AuthFailure::Unavailable);
        }
        match outcome {
            OAuthExchangeOutcome::Verified {
                principal: snapshot,
                issuance_grant,
            } => Ok(OAuthCompletionOutcome::Verified(VerifiedPrincipal {
                snapshot,
                issuance_grant,
                assurance: PrincipalAssurance::OAuth,
            })),
            OAuthExchangeOutcome::Linked { user_id } => {
                Ok(OAuthCompletionOutcome::Linked { user_id })
            }
            OAuthExchangeOutcome::Rejected(_) => Err(AuthFailure::Unauthenticated),
        }
    }

    fn now(&self) -> Result<TimestampMillis, AuthFailure> {
        self.clock.now().map_err(AuthFailure::from)
    }

    fn audit(
        &self,
        correlation_id: CorrelationId,
        action: AuthAuditAction,
        occurred_at: TimestampMillis,
    ) -> DecisionAudit {
        DecisionAudit {
            correlation_id,
            action,
            occurred_at,
        }
    }

    fn hash_identifier(&self, identifier: &str) -> Result<SecretDigestCandidates, AuthFailure> {
        let normalized = normalize_identifier(identifier)?;
        self.digest_candidates(b"frame/identifier/v2", &[normalized.as_bytes()])
    }

    fn hash_abuse(
        &self,
        identifier: &SecretDigestCandidates,
        context: AbuseContext<'_>,
    ) -> Result<AbuseDigestSet, AuthFailure> {
        Ok(AbuseDigestSet {
            identifier: identifier.clone(),
            source: self.digest_candidates(
                b"frame/abuse-source/v2",
                &[context.source.expose_for_hashing()],
            )?,
            device: self.digest_candidates(
                b"frame/abuse-device/v2",
                &[context.device.expose_for_hashing()],
            )?,
        })
    }

    fn active_digest(
        &self,
        domain: &[u8],
        parts: &[&[u8]],
    ) -> Result<VersionedSecretDigest, AuthFailure> {
        hash_with_key(&self.hash_keys.active, domain, parts)
    }

    fn verification_secret_digests(
        &self,
        identifiers: &SecretDigestCandidates,
        purpose: VerificationPurpose,
        channel: VerificationChannel,
        secret: &VerificationSecret,
    ) -> Result<SecretDigestCandidates, AuthFailure> {
        let mut digests = Vec::with_capacity(1 + self.hash_keys.fallback.len());
        for key in std::iter::once(&self.hash_keys.active).chain(self.hash_keys.fallback.iter()) {
            let identifier = identifiers
                .iter()
                .find(|digest| digest.key_version == key.version)
                .ok_or(AuthFailure::InvalidRequest)?;
            digests.push(hash_with_key(
                key,
                b"frame/verification/v2",
                &[
                    identifier.digest.expose_for_verification().as_bytes(),
                    verification_purpose_label(purpose),
                    verification_channel_label(channel),
                    secret.expose_for_hashing(),
                ],
            )?);
        }
        let mut digests = digests.into_iter();
        let active = digests.next().ok_or(AuthFailure::InvalidRequest)?;
        SecretDigestCandidates::new(active, digests.collect())
            .map_err(|_| AuthFailure::InvalidRequest)
    }

    fn digest_candidates(
        &self,
        domain: &[u8],
        parts: &[&[u8]],
    ) -> Result<SecretDigestCandidates, AuthFailure> {
        let active = hash_with_key(&self.hash_keys.active, domain, parts)?;
        let fallback = self
            .hash_keys
            .fallback
            .iter()
            .map(|key| hash_with_key(key, domain, parts))
            .collect::<Result<Vec<_>, _>>()?;
        SecretDigestCandidates::new(active, fallback).map_err(|_| AuthFailure::InvalidRequest)
    }
}

fn verification_purpose_label(purpose: VerificationPurpose) -> &'static [u8] {
    match purpose {
        VerificationPurpose::IdentityProvisioning => b"identity_provisioning",
        VerificationPurpose::EmailVerify => b"email_verify",
        VerificationPurpose::SignIn => b"sign_in",
        VerificationPurpose::AccountRecovery => b"account_recovery",
        VerificationPurpose::AccountLink => b"account_link",
    }
}

fn pkce_s256_challenge(verifier: &PkceVerifier) -> Result<PkceChallenge, AuthFailure> {
    const BASE64_URL: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let digest = Sha256::digest(verifier.expose_for_hashing());
    let mut encoded = String::with_capacity(43);
    let mut chunks = digest.chunks_exact(3);
    for chunk in &mut chunks {
        encoded.push(char::from(BASE64_URL[usize::from(chunk[0] >> 2)]));
        encoded.push(char::from(
            BASE64_URL[usize::from(((chunk[0] & 0x03) << 4) | (chunk[1] >> 4))],
        ));
        encoded.push(char::from(
            BASE64_URL[usize::from(((chunk[1] & 0x0f) << 2) | (chunk[2] >> 6))],
        ));
        encoded.push(char::from(BASE64_URL[usize::from(chunk[2] & 0x3f)]));
    }
    let remainder = chunks.remainder();
    if remainder.len() == 2 {
        encoded.push(char::from(BASE64_URL[usize::from(remainder[0] >> 2)]));
        encoded.push(char::from(
            BASE64_URL[usize::from(((remainder[0] & 0x03) << 4) | (remainder[1] >> 4))],
        ));
        encoded.push(char::from(
            BASE64_URL[usize::from((remainder[1] & 0x0f) << 2)],
        ));
    }
    PkceChallenge::parse(encoded).map_err(|_| AuthFailure::Unavailable)
}

fn verification_channel_label(channel: VerificationChannel) -> &'static [u8] {
    match channel {
        VerificationChannel::MagicLink => b"magic_link",
        VerificationChannel::OneTimeCode => b"one_time_code",
    }
}

fn normalize_identifier(identifier: &str) -> Result<String, AuthFailure> {
    let normalized = identifier.trim().to_ascii_lowercase();
    if !(3..=320).contains(&normalized.len())
        || !normalized.is_ascii()
        || normalized.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(AuthFailure::InvalidRequest);
    }
    Ok(normalized)
}

fn hash_with_key(
    key: &AuthHashKey,
    domain: &[u8],
    parts: &[&[u8]],
) -> Result<VersionedSecretDigest, AuthFailure> {
    let mut message = Vec::new();
    append_length_delimited(&mut message, domain)?;
    for part in parts {
        append_length_delimited(&mut message, part)?;
    }
    let digest = hmac_sha256(&key.material, &message);
    let digest =
        SecretDigest::parse_sha256(format!("{digest:x}")).map_err(|_| AuthFailure::Unavailable)?;
    Ok(VersionedSecretDigest::new(key.version, digest))
}

fn append_length_delimited(output: &mut Vec<u8>, value: &[u8]) -> Result<(), AuthFailure> {
    let length = u64::try_from(value.len()).map_err(|_| AuthFailure::InvalidRequest)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> sha2::digest::Output<Sha256> {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = Sha256::digest(key);
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for (index, value) in key_block.iter().enumerate() {
        inner_pad[index] ^= value;
        outer_pad[index] ^= value;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    use frame_domain::{
        DeliveryDestinationRef, OAuthAuthorizationCode, OneTimeCode, OrganizationRole,
        RateLimitPolicy, SealedDeliveryEnvelope,
    };
    use frame_ports::{ExternalIdentityAssertion, ManualClock, MemoryAuthStateRepository};

    use super::*;

    #[derive(Debug, Default)]
    struct DeterministicAuthSecretSource {
        sequence: AtomicU64,
    }

    impl DeterministicAuthSecretSource {
        fn next(&self) -> u64 {
            self.sequence.fetch_add(1, Ordering::SeqCst) + 1
        }

        fn opaque(prefix: char, value: u64) -> String {
            format!("{prefix}{value:063}")
        }
    }

    impl AuthSecretSource for DeterministicAuthSecretSource {
        fn session_token(&self) -> Result<OpaqueAuthToken, frame_ports::PortError> {
            OpaqueAuthToken::parse(Self::opaque('s', self.next()))
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        }

        fn csrf_token(&self) -> Result<CsrfToken, frame_ports::PortError> {
            CsrfToken::parse(Self::opaque('c', self.next()))
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        }

        fn api_key(&self) -> Result<ApiKeySecret, frame_ports::PortError> {
            ApiKeySecret::parse(Self::opaque('k', self.next()))
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        }

        fn oauth_state(&self) -> Result<OAuthState, frame_ports::PortError> {
            OAuthState::parse(Self::opaque('o', self.next()))
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        }

        fn pkce_verifier(&self) -> Result<PkceVerifier, frame_ports::PortError> {
            PkceVerifier::parse(Self::opaque('p', self.next()))
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        }

        fn verification_secret(
            &self,
            channel: VerificationChannel,
        ) -> Result<VerificationSecret, frame_ports::PortError> {
            let next = self.next();
            match channel {
                VerificationChannel::MagicLink => OpaqueAuthToken::parse(Self::opaque('m', next))
                    .map(VerificationSecret::MagicLink)
                    .map_err(|error| frame_ports::PortError::Adapter(error.to_string())),
                VerificationChannel::OneTimeCode => {
                    OneTimeCode::parse(format!("{:06}", next % 1_000_000))
                        .map(VerificationSecret::OneTimeCode)
                        .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
                }
            }
        }
    }

    #[derive(Debug, Default)]
    struct DeterministicDeliverySealer;

    impl AuthDeliverySealer for DeterministicDeliverySealer {
        fn seal(
            &self,
            material: &VerificationDeliveryMaterial,
            now: TimestampMillis,
        ) -> Result<SealedDeliveryEnvelope, frame_ports::PortError> {
            let marker = match material.purpose {
                VerificationPurpose::IdentityProvisioning => b'p',
                VerificationPurpose::SignIn => b'i',
                VerificationPurpose::AccountRecovery => b'r',
                VerificationPurpose::EmailVerify => b'e',
                VerificationPurpose::AccountLink => b'l',
            };
            SealedDeliveryEnvelope::new(vec![marker; 64], now)
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))
        }
    }

    struct RecordingSecrets {
        inner: DeterministicAuthSecretSource,
        last_verification: Mutex<Option<VerificationSecret>>,
    }

    impl Default for RecordingSecrets {
        fn default() -> Self {
            Self {
                inner: DeterministicAuthSecretSource::default(),
                last_verification: Mutex::new(None),
            }
        }
    }

    impl AuthSecretSource for RecordingSecrets {
        fn session_token(&self) -> Result<OpaqueAuthToken, frame_ports::PortError> {
            self.inner.session_token()
        }

        fn csrf_token(&self) -> Result<CsrfToken, frame_ports::PortError> {
            self.inner.csrf_token()
        }

        fn api_key(&self) -> Result<ApiKeySecret, frame_ports::PortError> {
            self.inner.api_key()
        }

        fn oauth_state(&self) -> Result<OAuthState, frame_ports::PortError> {
            self.inner.oauth_state()
        }

        fn pkce_verifier(&self) -> Result<PkceVerifier, frame_ports::PortError> {
            self.inner.pkce_verifier()
        }

        fn verification_secret(
            &self,
            channel: VerificationChannel,
        ) -> Result<VerificationSecret, frame_ports::PortError> {
            let generated = self.inner.verification_secret(channel)?;
            *self
                .last_verification
                .lock()
                .map_err(|error| frame_ports::PortError::Adapter(error.to_string()))? =
                Some(generated.clone());
            Ok(generated)
        }
    }

    impl RecordingSecrets {
        fn last(&self) -> VerificationSecret {
            self.last_verification
                .lock()
                .expect("lock")
                .clone()
                .expect("verification secret")
        }
    }

    #[derive(Clone)]
    struct StaticOAuthVerifier {
        assertion: ExternalIdentityAssertion,
        calls: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl OAuthIdentityVerifier for StaticOAuthVerifier {
        async fn verify(
            &self,
            exchange: OAuthProviderExchange<'_>,
        ) -> Result<Option<ExternalIdentityAssertion>, frame_ports::PortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if exchange.callback.provider != self.assertion.provider
                || exchange.callback_url.as_str() != "https://frame.engmanager.xyz/auth/callback"
                || exchange.audience.as_str() != "frame-web"
                || exchange.pkce_verifier.expose_for_hashing().is_empty()
            {
                return Err(frame_ports::PortError::InvalidRequest(
                    "provider exchange binding mismatch".into(),
                ));
            }
            Ok(Some(self.assertion.clone()))
        }
    }

    struct ClockAdvancingOAuthVerifier<'a> {
        inner: StaticOAuthVerifier,
        clock: &'a ManualClock,
        advance_to: TimestampMillis,
    }

    #[async_trait::async_trait]
    impl OAuthIdentityVerifier for ClockAdvancingOAuthVerifier<'_> {
        async fn verify(
            &self,
            exchange: OAuthProviderExchange<'_>,
        ) -> Result<Option<ExternalIdentityAssertion>, frame_ports::PortError> {
            self.clock.set(self.advance_to);
            self.inner.verify(exchange).await
        }
    }

    struct FailingOAuthVerifier {
        calls: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl OAuthIdentityVerifier for FailingOAuthVerifier {
        async fn verify(
            &self,
            _: OAuthProviderExchange<'_>,
        ) -> Result<Option<ExternalIdentityAssertion>, frame_ports::PortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(frame_ports::PortError::Adapter(
                "injected provider outage".into(),
            ))
        }
    }

    fn time(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("time")
    }

    fn duration(value: u64) -> DurationMillis {
        DurationMillis::new(value).expect("duration")
    }

    fn rate(max: u32) -> MultiRateLimitPolicy {
        let policy = RateLimitPolicy::new(max, duration(20), duration(30)).expect("policy");
        MultiRateLimitPolicy {
            identifier: policy,
            source: policy,
            device: policy,
            global: policy,
        }
    }

    fn policy() -> AuthPolicy {
        AuthPolicy::new(
            duration(10),
            duration(40),
            duration(10),
            3,
            rate(20),
            rate(20),
            rate(20),
            rate(20),
            duration(10),
            vec![OAuthProviderPolicy {
                provider: OAuthProvider::Github,
                callback_url: ExactOAuthCallbackUrl::parse(
                    "https://frame.engmanager.xyz/auth/callback",
                )
                .expect("callback"),
                audience: OAuthAudience::parse("frame-web").expect("audience"),
            }],
            ExactBrowserOrigin::parse("https://frame.engmanager.xyz").expect("origin"),
        )
        .expect("policy")
    }

    fn key(version: u16, marker: u8) -> AuthHashKey {
        AuthHashKey::new(
            HashKeyVersion::new(version).expect("version"),
            vec![marker; 32],
        )
        .expect("key")
    }

    fn ring_v1() -> AuthHashKeyRing {
        AuthHashKeyRing::new(key(1, b'a'), vec![]).expect("ring")
    }

    fn abuse_signals() -> (AbuseSignal, AbuseSignal) {
        (
            AbuseSignal::parse("source-test-signal").expect("source"),
            AbuseSignal::parse("device-test-signal").expect("device"),
        )
    }

    async fn provision_identity(
        service: &AuthService<
            '_,
            MemoryAuthStateRepository,
            ManualClock,
            RecordingSecrets,
            DeterministicDeliverySealer,
        >,
        repository: &MemoryAuthStateRepository,
        identifier: &str,
        role: OrganizationRole,
    ) -> PrincipalSnapshot {
        let _ = (repository, role);
        let user_id = frame_domain::UserId::new();
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_identity_provisioning_verification(
                    identifier,
                    user_id,
                    1,
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue signup"),
            VerificationIssueReceipt::Accepted
        );
        let authorized = service
            .consume_verification(
                identifier,
                VerificationPurpose::IdentityProvisioning,
                &service.secrets.last(),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("verify signup");
        let VerificationConsumeOutcome::ProvisioningAuthorized(authorized) = authorized else {
            panic!("provisioning grant expected");
        };
        assert_eq!(
            service
                .provision_identity(
                    authorized,
                    DeliveryDestinationRef::parse("delivery-reference-1").expect("destination"),
                    CorrelationId::new(),
                )
                .await
                .expect("provision"),
            user_id
        );
        PrincipalSnapshot {
            user_id,
            identity_revision: 1,
            tenant_grants: Vec::new(),
        }
    }

    async fn verify_principal(
        service: &AuthService<
            '_,
            MemoryAuthStateRepository,
            ManualClock,
            RecordingSecrets,
            DeterministicDeliverySealer,
        >,
        secrets: &RecordingSecrets,
        identifier: &str,
    ) -> VerifiedPrincipal {
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_verification(
                    identifier,
                    VerificationPurpose::SignIn,
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue"),
            VerificationIssueReceipt::Accepted
        );
        let outcome = service
            .consume_verification(
                identifier,
                VerificationPurpose::SignIn,
                &secrets.last(),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("consume");
        let VerificationConsumeOutcome::Verified(principal) = outcome else {
            panic!("verified principal expected");
        };
        principal
    }

    async fn provisioned_browser_session(
        service: &AuthService<
            '_,
            MemoryAuthStateRepository,
            ManualClock,
            RecordingSecrets,
            DeterministicDeliverySealer,
        >,
        repository: &MemoryAuthStateRepository,
        secrets: &RecordingSecrets,
        identifier: &str,
    ) -> (PrincipalSnapshot, IssuedSession) {
        let principal =
            provision_identity(service, repository, identifier, OrganizationRole::Owner).await;
        let verified = verify_principal(service, secrets, identifier).await;
        let session = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("browser session");
        (principal, session)
    }

    async fn browser_proof(
        service: &AuthService<
            '_,
            MemoryAuthStateRepository,
            ManualClock,
            RecordingSecrets,
            DeterministicDeliverySealer,
        >,
        session: &IssuedSession,
    ) -> ValidatedBrowserMutationProof {
        let csrf = session.csrf_token.as_ref().expect("csrf");
        service
            .validate_browser_mutation(
                &session.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("browser proof")
    }

    #[tokio::test]
    async fn only_verified_principal_can_issue_session_and_all_secrets_are_hashed() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let principal = provision_identity(
            &service,
            &repository,
            "person@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "person@example.test").await;
        assert_eq!(verified.user_id(), principal.user_id);
        let issued = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("session");
        let stored = repository
            .session(issued.session_id)
            .expect("read")
            .expect("stored");
        assert_ne!(
            stored
                .token_digest
                .digest
                .expose_for_verification()
                .as_bytes(),
            issued.token.expose_for_hashing()
        );
        assert!(!format!("{issued:?}").contains("000000"));
    }

    #[tokio::test]
    async fn expired_session_issuance_capability_is_denied_and_audited_as_expired() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "expired-grant@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "expired-grant@example.test").await;
        clock.set(time(11));
        assert_eq!(
            service
                .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
                .await,
            Err(AuthFailure::Unauthenticated)
        );
        let audit = repository.audit_events().expect("audit");
        let event = audit.last().expect("session issue audit");
        assert_eq!(event.action, AuthAuditAction::SessionIssue);
        assert_eq!(event.outcome, frame_domain::AuthAuditOutcome::Deny);
        assert_eq!(event.reason, frame_domain::AuthAuditReason::Expired);
    }

    #[tokio::test]
    async fn signup_for_an_owned_identifier_is_a_suppressed_decoy_without_a_grant() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "owned@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_identity_provisioning_verification(
                    "owned@example.test",
                    UserId::new(),
                    1,
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue decoy"),
            VerificationIssueReceipt::Accepted
        );
        assert_eq!(
            service
                .consume_verification(
                    "owned@example.test",
                    VerificationPurpose::IdentityProvisioning,
                    &secrets.last(),
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("consume decoy"),
            VerificationConsumeOutcome::Rejected
        );
    }

    #[tokio::test]
    async fn known_and_unknown_issue_receipts_are_identical_and_both_enqueue() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "known@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let baseline = repository.delivery_counts().expect("baseline deliveries");
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        let known = service
            .issue_verification(
                "known@example.test",
                VerificationPurpose::SignIn,
                VerificationChannel::MagicLink,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("known");
        let unknown = service
            .issue_verification(
                "unknown@example.test",
                VerificationPurpose::SignIn,
                VerificationChannel::MagicLink,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("unknown");
        assert_eq!(known, unknown);
        assert_eq!(known.public_disposition(), "accepted");
        assert_eq!(
            repository.delivery_counts().expect("deliveries"),
            (baseline.0 + 2, baseline.1)
        );
        repository
            .materialize_verification_deliveries(time(2), 10)
            .await
            .expect("materialize");
        assert_eq!(
            repository.delivery_counts().expect("deliveries"),
            (baseline.0 + 2, baseline.1 + 1)
        );
    }

    #[tokio::test]
    async fn destructive_session_operations_require_validated_browser_capability() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "browser@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "browser@example.test").await;
        let issued = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("session");
        let csrf = issued.csrf_token.as_ref().expect("csrf");
        let sibling = service
            .validate_browser_mutation(
                &issued.token,
                BrowserMutationRequest {
                    origin: "https://engmanager.xyz",
                    fetch_site: FetchSite::SameSite,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await;
        assert!(matches!(sibling, Err(AuthFailure::RequestRejected)));
        let proof = service
            .validate_browser_mutation(
                &issued.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("proof");
        let rotated = service
            .rotate_session(proof, CorrelationId::new())
            .await
            .expect("rotate");
        assert_eq!(rotated.generation, 1);
        assert_eq!(
            service
                .authenticate(&issued.token, CorrelationId::new())
                .await,
            Err(AuthFailure::Unauthenticated)
        );
    }

    #[tokio::test]
    async fn active_and_fallback_hash_keys_overlap_and_migrate_session() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let v1 = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &v1,
            &repository,
            "rotate@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&v1, &secrets, "rotate@example.test").await;
        let issued = v1
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("session");
        let v2 = AuthService::new(
            &repository,
            &clock,
            &secrets,
            &sealer,
            AuthHashKeyRing::new(key(2, b'b'), vec![key(1, b'a')]).expect("ring"),
            policy(),
        );
        assert!(
            v2.authenticate(&issued.token, CorrelationId::new())
                .await
                .is_ok()
        );
        assert_eq!(
            repository
                .session(issued.session_id)
                .expect("read")
                .expect("session")
                .token_digest
                .key_version,
            HashKeyVersion::new(2).expect("version")
        );
    }

    #[tokio::test]
    async fn one_time_verification_survives_hash_key_rotation_overlap() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let v1 = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &v1,
            &repository,
            "verification-rotation@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            v1.issue_verification(
                "verification-rotation@example.test",
                VerificationPurpose::SignIn,
                VerificationChannel::OneTimeCode,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("issue"),
            VerificationIssueReceipt::Accepted
        );
        let issued_secret = secrets.last();
        let v2 = AuthService::new(
            &repository,
            &clock,
            &secrets,
            &sealer,
            AuthHashKeyRing::new(key(2, b'b'), vec![key(1, b'a')]).expect("ring"),
            policy(),
        );
        assert!(matches!(
            v2.consume_verification(
                "verification-rotation@example.test",
                VerificationPurpose::SignIn,
                &issued_secret,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("consume"),
            VerificationConsumeOutcome::Verified(_)
        ));
    }

    #[tokio::test]
    async fn newly_provisioned_identity_cannot_self_grant_tenant_api_key_authority() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "admin@example.test",
            OrganizationRole::Admin,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "admin@example.test").await;
        let issued_session = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("session");
        let csrf = issued_session.csrf_token.as_ref().expect("csrf");
        let proof = service
            .validate_browser_mutation(
                &issued_session.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("proof");
        assert_eq!(
            service
                .issue_api_key(
                    proof,
                    TenantId::new(),
                    vec![ApiKeyScope::VideosRead],
                    None,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::RequestRejected)
        );
    }

    #[tokio::test]
    async fn logout_all_bumps_version_but_fresh_verified_session_can_resume() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "logout-all@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "logout-all@example.test").await;
        let old = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("old session");
        let csrf = old.csrf_token.as_ref().expect("csrf");
        let proof = service
            .validate_browser_mutation(
                &old.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("proof");
        assert_eq!(
            service
                .logout_all(proof, CorrelationId::new())
                .await
                .expect("logout all")
                .new_session_version,
            1
        );
        let verified = verify_principal(&service, &secrets, "logout-all@example.test").await;
        let fresh = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("fresh session");
        assert!(
            service
                .authenticate(&fresh.token, CorrelationId::new())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn account_link_cannot_pair_an_authenticated_user_with_someone_elses_identifier() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "first@example.test",
            OrganizationRole::Owner,
        )
        .await;
        provision_identity(
            &service,
            &repository,
            "second@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "first@example.test").await;
        let session = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("session");
        let csrf = session.csrf_token.as_ref().expect("csrf");
        let proof = service
            .validate_browser_mutation(
                &session.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("proof");
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_account_link_verification(
                    proof,
                    "second@example.test",
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue"),
            VerificationIssueReceipt::Accepted
        );
        assert_eq!(
            service
                .consume_verification(
                    "second@example.test",
                    VerificationPurpose::AccountLink,
                    &secrets.last(),
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("consume"),
            VerificationConsumeOutcome::Rejected
        );
    }

    #[tokio::test]
    async fn account_link_claims_a_new_identifier_for_the_authenticated_identity() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let snapshot = provision_identity(
            &service,
            &repository,
            "primary@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "primary@example.test").await;
        let session = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("session");
        let csrf = session.csrf_token.as_ref().expect("csrf");
        let proof = service
            .validate_browser_mutation(
                &session.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("proof");
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_account_link_verification(
                    proof,
                    "primary@example.test",
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue self-link decoy"),
            VerificationIssueReceipt::Accepted
        );
        assert_eq!(
            service
                .consume_verification(
                    "primary@example.test",
                    VerificationPurpose::AccountLink,
                    &secrets.last(),
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("consume self-link decoy"),
            VerificationConsumeOutcome::Rejected
        );
        let proof = service
            .validate_browser_mutation(
                &session.token,
                BrowserMutationRequest {
                    origin: "https://frame.engmanager.xyz",
                    fetch_site: FetchSite::SameOrigin,
                    csrf_cookie: csrf,
                    csrf_header: csrf,
                },
                CorrelationId::new(),
            )
            .await
            .expect("fresh proof");
        assert_eq!(
            service
                .issue_account_link_verification(
                    proof,
                    "linked@example.test",
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue link"),
            VerificationIssueReceipt::Accepted
        );
        let linked = service
            .consume_verification(
                "linked@example.test",
                VerificationPurpose::AccountLink,
                &secrets.last(),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("consume link");
        assert_eq!(
            linked,
            VerificationConsumeOutcome::Linked {
                user_id: snapshot.user_id
            }
        );
        // The link receipt carries no session-issuance capability; prove the new identifier now
        // resolves only through a fresh sign-in verification.
        let through_link = verify_principal(&service, &secrets, "linked@example.test").await;
        assert_eq!(through_link.user_id(), snapshot.user_id);
    }

    #[tokio::test]
    async fn account_recovery_revokes_existing_sessions_before_issuing_a_fresh_capability() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        provision_identity(
            &service,
            &repository,
            "recover@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let verified = verify_principal(&service, &secrets, "recover@example.test").await;
        let old = service
            .issue_session(verified, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("old session");
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_verification(
                    "recover@example.test",
                    VerificationPurpose::AccountRecovery,
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue recovery"),
            VerificationIssueReceipt::Accepted
        );
        let recovered = service
            .consume_verification(
                "recover@example.test",
                VerificationPurpose::AccountRecovery,
                &secrets.last(),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("recover");
        let VerificationConsumeOutcome::Verified(recovered) = recovered else {
            panic!("recovered principal expected");
        };
        assert_eq!(recovered.assurance(), PrincipalAssurance::Recovery);
        assert_eq!(
            service.authenticate(&old.token, CorrelationId::new()).await,
            Err(AuthFailure::Unauthenticated)
        );
        let fresh = service
            .issue_session(recovered, AuthClientKind::Browser, CorrelationId::new())
            .await
            .expect("fresh session");
        assert!(
            service
                .authenticate(&fresh.token, CorrelationId::new())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn logout_purges_a_pending_account_link_verification() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let (_, session) = provisioned_browser_session(
            &service,
            &repository,
            &secrets,
            "logout-link@example.test",
        )
        .await;
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        assert_eq!(
            service
                .issue_account_link_verification(
                    browser_proof(&service, &session).await,
                    "logout-pending@example.test",
                    VerificationChannel::OneTimeCode,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("issue pending link"),
            VerificationIssueReceipt::Accepted
        );
        let pending_secret = secrets.last();
        service
            .logout(
                browser_proof(&service, &session).await,
                CorrelationId::new(),
            )
            .await
            .expect("logout");
        assert_eq!(
            service
                .consume_verification(
                    "logout-pending@example.test",
                    VerificationPurpose::AccountLink,
                    &pending_secret,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("consume purged link"),
            VerificationConsumeOutcome::Rejected
        );
    }

    #[tokio::test]
    async fn logout_all_purges_a_pending_oauth_link_before_provider_io() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let (_, session) = provisioned_browser_session(
            &service,
            &repository,
            &secrets,
            "logout-all-link@example.test",
        )
        .await;
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        let start = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::AccountLink,
                Some(browser_proof(&service, &session).await),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin link");
        service
            .logout_all(
                browser_proof(&service, &session).await,
                CorrelationId::new(),
            )
            .await
            .expect("logout all");
        let calls = Arc::new(AtomicU64::new(0));
        let verifier = StaticOAuthVerifier {
            assertion: ExternalIdentityAssertion {
                provider: OAuthProvider::Github,
                subject_digests: service
                    .digest_candidates(b"test/provider-subject", &[b"logout-all-pending"])
                    .expect("subject"),
                verified_identifier_digests: None,
            },
            calls: Arc::clone(&calls),
        };
        let callback = OAuthCallback {
            provider: OAuthProvider::Github,
            code: OAuthAuthorizationCode::parse("a".repeat(64)).expect("code"),
        };
        assert_eq!(
            service
                .exchange_oauth(
                    &verifier,
                    &callback,
                    &start.state,
                    &start.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::Unauthenticated)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn account_recovery_purges_pending_otp_and_oauth_links() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let (_, session) = provisioned_browser_session(
            &service,
            &repository,
            &secrets,
            "recovery-links@example.test",
        )
        .await;
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        service
            .issue_account_link_verification(
                browser_proof(&service, &session).await,
                "recovery-pending@example.test",
                VerificationChannel::OneTimeCode,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("issue pending OTP link");
        let link_secret = secrets.last();
        let oauth = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::AccountLink,
                Some(browser_proof(&service, &session).await),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin pending OAuth link");
        service
            .issue_verification(
                "recovery-links@example.test",
                VerificationPurpose::AccountRecovery,
                VerificationChannel::OneTimeCode,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("issue recovery");
        let recovery_secret = secrets.last();
        assert!(matches!(
            service
                .consume_verification(
                    "recovery-links@example.test",
                    VerificationPurpose::AccountRecovery,
                    &recovery_secret,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("recover"),
            VerificationConsumeOutcome::Verified(_)
        ));
        assert_eq!(
            service
                .consume_verification(
                    "recovery-pending@example.test",
                    VerificationPurpose::AccountLink,
                    &link_secret,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("consume purged OTP link"),
            VerificationConsumeOutcome::Rejected
        );
        let calls = Arc::new(AtomicU64::new(0));
        let verifier = StaticOAuthVerifier {
            assertion: ExternalIdentityAssertion {
                provider: OAuthProvider::Github,
                subject_digests: service
                    .digest_candidates(b"test/provider-subject", &[b"recovery-pending"])
                    .expect("subject"),
                verified_identifier_digests: None,
            },
            calls: Arc::clone(&calls),
        };
        let callback = OAuthCallback {
            provider: OAuthProvider::Github,
            code: OAuthAuthorizationCode::parse("a".repeat(64)).expect("code"),
        };
        assert_eq!(
            service
                .exchange_oauth(
                    &verifier,
                    &callback,
                    &oauth.state,
                    &oauth.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::Unauthenticated)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn oauth_account_link_returns_only_a_link_receipt() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let (snapshot, session) =
            provisioned_browser_session(&service, &repository, &secrets, "oauth-link@example.test")
                .await;
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        let start = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::AccountLink,
                Some(browser_proof(&service, &session).await),
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin link");
        let calls = Arc::new(AtomicU64::new(0));
        let verifier = StaticOAuthVerifier {
            assertion: ExternalIdentityAssertion {
                provider: OAuthProvider::Github,
                subject_digests: service
                    .digest_candidates(b"test/provider-subject", &[b"linked-provider-user"])
                    .expect("subject"),
                verified_identifier_digests: None,
            },
            calls: Arc::clone(&calls),
        };
        let callback = OAuthCallback {
            provider: OAuthProvider::Github,
            code: OAuthAuthorizationCode::parse("a".repeat(64)).expect("code"),
        };
        assert_eq!(
            service
                .exchange_oauth(
                    &verifier,
                    &callback,
                    &start.state,
                    &start.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await
                .expect("link"),
            OAuthCompletionOutcome::Linked {
                user_id: snapshot.user_id
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn oauth_provider_adapter_failures_are_audited_explicitly() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        let start = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::SignIn,
                None,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin");
        assert_eq!(start.pkce_challenge_method, PkceChallengeMethod::S256);
        assert_eq!(start.pkce_challenge_method.as_provider_value(), "S256");
        assert_eq!(
            start.pkce_challenge,
            pkce_s256_challenge(&start.pkce_verifier).expect("challenge")
        );
        let formatted_start = format!("{start:?}");
        assert!(!formatted_start.contains(
            std::str::from_utf8(start.pkce_verifier.expose_for_hashing()).expect("ASCII verifier")
        ));
        assert!(!formatted_start.contains(start.pkce_challenge.expose_for_authorization()));
        let calls = Arc::new(AtomicU64::new(0));
        let verifier = FailingOAuthVerifier {
            calls: Arc::clone(&calls),
        };
        let callback = OAuthCallback {
            provider: OAuthProvider::Github,
            code: OAuthAuthorizationCode::parse("a".repeat(64)).expect("code"),
        };
        assert_eq!(
            service
                .exchange_oauth(
                    &verifier,
                    &callback,
                    &start.state,
                    &start.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::Unavailable)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let audit = repository.audit_events().expect("audit");
        let event = audit.last().expect("OAuth exchange audit");
        assert_eq!(event.action, AuthAuditAction::OAuthExchange);
        assert_eq!(event.outcome, frame_domain::AuthAuditOutcome::Error);
        assert_eq!(event.reason, frame_domain::AuthAuditReason::AdapterFailure);
    }

    #[tokio::test]
    async fn oauth_state_pkce_redirect_audience_and_replay_are_repository_enforced() {
        let repository = MemoryAuthStateRepository::default();
        let clock = ManualClock::new(time(1));
        let secrets = RecordingSecrets::default();
        let sealer = DeterministicDeliverySealer;
        let service = AuthService::new(&repository, &clock, &secrets, &sealer, ring_v1(), policy());
        let snapshot = provision_identity(
            &service,
            &repository,
            "oauth@example.test",
            OrganizationRole::Owner,
        )
        .await;
        let subject = service
            .digest_candidates(b"test/provider-subject", &[b"provider-user-1"])
            .expect("subject");
        let (source, device) = abuse_signals();
        let abuse = AbuseContext {
            source: &source,
            device: &device,
        };
        let start = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::SignIn,
                None,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin");
        let calls = Arc::new(AtomicU64::new(0));
        let verifier = StaticOAuthVerifier {
            assertion: ExternalIdentityAssertion {
                provider: OAuthProvider::Github,
                subject_digests: subject,
                verified_identifier_digests: Some(
                    service
                        .hash_identifier("oauth@example.test")
                        .expect("identifier"),
                ),
            },
            calls: Arc::clone(&calls),
        };
        let callback = OAuthCallback {
            provider: OAuthProvider::Github,
            code: OAuthAuthorizationCode::parse("a".repeat(64)).expect("code"),
        };
        let principal = service
            .exchange_oauth(
                &verifier,
                &callback,
                &start.state,
                &start.pkce_verifier,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("exchange");
        let OAuthCompletionOutcome::Verified(principal) = principal else {
            panic!("OAuth principal expected");
        };
        assert_eq!(principal.user_id(), snapshot.user_id);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            service
                .exchange_oauth(
                    &verifier,
                    &callback,
                    &start.state,
                    &start.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::Unauthenticated)
        );
        // State/PKCE replay is rejected by repository preflight, before provider I/O.
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let unlinked = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::SignIn,
                None,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin unlinked");
        let unverified_calls = Arc::new(AtomicU64::new(0));
        let unverified = StaticOAuthVerifier {
            assertion: ExternalIdentityAssertion {
                provider: OAuthProvider::Github,
                subject_digests: service
                    .digest_candidates(b"test/provider-subject", &[b"unlinked-provider-user"])
                    .expect("subject"),
                verified_identifier_digests: None,
            },
            calls: Arc::clone(&unverified_calls),
        };
        assert_eq!(
            service
                .exchange_oauth(
                    &unverified,
                    &callback,
                    &unlinked.state,
                    &unlinked.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::Unauthenticated)
        );
        assert_eq!(unverified_calls.load(Ordering::SeqCst), 1);

        let expiring = service
            .begin_oauth(
                OAuthProvider::Github,
                OAuthFlowPurpose::SignIn,
                None,
                abuse,
                CorrelationId::new(),
            )
            .await
            .expect("begin expiring flow");
        let expiring_calls = Arc::new(AtomicU64::new(0));
        let expiring_verifier = ClockAdvancingOAuthVerifier {
            inner: StaticOAuthVerifier {
                assertion: verifier.assertion.clone(),
                calls: Arc::clone(&expiring_calls),
            },
            clock: &clock,
            advance_to: expiring.expires_at,
        };
        assert_eq!(
            service
                .exchange_oauth(
                    &expiring_verifier,
                    &callback,
                    &expiring.state,
                    &expiring.pkce_verifier,
                    abuse,
                    CorrelationId::new(),
                )
                .await,
            Err(AuthFailure::Unauthenticated)
        );
        assert_eq!(expiring_calls.load(Ordering::SeqCst), 1);
        let audit = repository.audit_events().expect("audit");
        let event = audit.last().expect("expired OAuth audit");
        assert_eq!(event.action, AuthAuditAction::OAuthExchange);
        assert_eq!(event.outcome, frame_domain::AuthAuditOutcome::Deny);
        assert_eq!(event.reason, frame_domain::AuthAuditReason::Expired);
    }

    #[test]
    fn pkce_s256_matches_the_rfc_7636_authorization_vector() {
        let verifier =
            PkceVerifier::parse("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").expect("verifier");
        let challenge = pkce_s256_challenge(&verifier).expect("challenge");
        assert_eq!(
            challenge.expose_for_authorization(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(PkceChallengeMethod::S256.as_provider_value(), "S256");
        assert!(!format!("{challenge:?}").contains(challenge.expose_for_authorization()));
    }

    #[test]
    fn hash_keys_and_delivery_material_are_never_formatted() {
        let key = key(1, b'a');
        let ring = AuthHashKeyRing::new(key.clone(), vec![]).expect("ring");
        assert!(!format!("{key:?}").contains(&"a".repeat(32)));
        assert!(!format!("{ring:?}").contains(&"a".repeat(32)));
        let one_time_code =
            VerificationSecret::OneTimeCode(OneTimeCode::parse("123456").expect("otp"));
        let material = VerificationDeliveryMaterial {
            destination: VerificationDestination::parse("delivery@example.test")
                .expect("destination"),
            secret: one_time_code,
            purpose: VerificationPurpose::SignIn,
            expires_at: time(10),
        };
        assert!(!format!("{material:?}").contains("123456"));
    }
}
