//! Cross-surface API admission, webhook, and durable-workflow primitives.
//!
//! Runtime adapters own persistence and clocks. This module owns the security
//! invariants and state transitions so Worker, native, and test adapters cannot
//! quietly implement different retry or disclosure behavior.

use std::{collections::BTreeMap, fmt, sync::RwLock};

use async_trait::async_trait;
use frame_domain::{
    API_CONTRACT_VERSION_V1, ApiAuthClassV1, ApiErrorCodeV1, ApiErrorV1, ApiMutationEnvelopeV1,
    ApiRequestPolicyV1, ChecksumSha256, IdempotencyKey, MAX_WEBHOOK_BODY_BYTES_V1, TimestampMillis,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const WEBHOOK_SIGNATURE_PREFIX: &str = "v1=";
const WEBHOOK_MIN_SECRET_BYTES: usize = 32;
const WEBHOOK_MAX_SECRET_BYTES: usize = 128;
const MAX_WEBHOOK_SECRETS: usize = 4;
const MAX_REPLAY_ROWS: usize = 10_000;
const MAX_REPLAY_CLAIM_LIFETIME_MS: i64 = 30 * 60 * 1_000;
const MAX_WORKFLOW_ATTEMPTS: u16 = 32;
const MAX_LEASE_MS: i64 = 15 * 60 * 1000;
const MAX_RETRY_DELAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitDecisionV1 {
    Allowed,
    Rejected { retry_after_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSecurityContextV1 {
    pub authenticated: bool,
    pub authorized: bool,
    pub browser_origin_valid: bool,
    pub csrf_valid: bool,
    pub rate_limit: RateLimitDecisionV1,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ApiAdmissionV1 {
    pub schema_version: &'static str,
    pub audit_action: String,
    pub rate_limit_bucket: String,
    pub auth: ApiAuthClassV1,
    pub correlation_id: String,
}

impl fmt::Debug for ApiAdmissionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiAdmissionV1")
            .field("audit_action", &self.audit_action)
            .field("rate_limit_bucket", &self.rate_limit_bucket)
            .field("auth", &self.auth)
            .field("correlation_id", &"<redacted>")
            .finish()
    }
}

/// One admission path for validation, authentication, tenant non-disclosure,
/// CSRF/origin checks, rate limiting, and privacy-safe tracing labels.
pub struct ApiGatewayV1;

impl ApiGatewayV1 {
    pub fn admit_mutation(
        policy: &ApiRequestPolicyV1,
        envelope: &ApiMutationEnvelopeV1,
        context: RequestSecurityContextV1,
    ) -> Result<ApiAdmissionV1, ApiErrorV1> {
        envelope
            .validate(policy)
            .map_err(|_| public_error(ApiErrorCodeV1::InvalidRequest, envelope, None))?;

        let requires_auth = !matches!(
            policy.auth,
            ApiAuthClassV1::Public | ApiAuthClassV1::OptionalSession | ApiAuthClassV1::Webhook
        );
        if requires_auth && !context.authenticated {
            return Err(public_error(
                ApiErrorCodeV1::Unauthenticated,
                envelope,
                None,
            ));
        }
        // Once authentication succeeds, authorization failures collapse to a
        // 404 so this layer never reveals a cross-tenant identifier.
        if !context.authorized {
            return Err(public_error(ApiErrorCodeV1::NotFound, envelope, None));
        }
        if !context.browser_origin_valid || !context.csrf_valid {
            return Err(public_error(ApiErrorCodeV1::InvalidRequest, envelope, None));
        }
        if let RateLimitDecisionV1::Rejected { retry_after_ms } = context.rate_limit {
            return Err(public_error(
                ApiErrorCodeV1::RateLimited,
                envelope,
                Some(retry_after_ms),
            ));
        }

        Ok(ApiAdmissionV1 {
            schema_version: API_CONTRACT_VERSION_V1,
            audit_action: policy.audit_action.clone(),
            rate_limit_bucket: policy.rate_limit_bucket.clone(),
            auth: policy.auth,
            correlation_id: envelope.correlation_id.clone(),
        })
    }
}

fn public_error(
    code: ApiErrorCodeV1,
    envelope: &ApiMutationEnvelopeV1,
    retry_after_ms: Option<u64>,
) -> ApiErrorV1 {
    // Treat a malformed adapter-provided rate-limit duration as absent. Public
    // error construction must never panic on an untrusted backend response.
    let retry_after_ms =
        retry_after_ms.filter(|value| (1..=86_400_000).contains(value) && code.retryable());
    // The envelope has already validated its correlation ID in all normal
    // paths. A fixed safe token keeps malformed requests out of error details.
    ApiErrorV1::new(code, envelope.correlation_id.clone(), retry_after_ms).unwrap_or_else(|_| {
        ApiErrorV1::new(code, "invalid-correlation", retry_after_ms)
            .expect("fixed correlation token is valid")
    })
}

pub struct WebhookSecretV1 {
    key_id: String,
    secret: Vec<u8>,
    not_before_ms: i64,
    not_after_ms: i64,
}

impl WebhookSecretV1 {
    pub fn new(
        key_id: impl Into<String>,
        secret: Vec<u8>,
        not_before_ms: i64,
        not_after_ms: i64,
    ) -> Result<Self, WebhookErrorV1> {
        let key_id = key_id.into();
        if !safe_token(&key_id, 64)
            || !(WEBHOOK_MIN_SECRET_BYTES..=WEBHOOK_MAX_SECRET_BYTES).contains(&secret.len())
            || TimestampMillis::new(not_before_ms).is_err()
            || TimestampMillis::new(not_after_ms).is_err()
            || not_after_ms <= not_before_ms
        {
            return Err(WebhookErrorV1::InvalidConfiguration);
        }
        Ok(Self {
            key_id,
            secret,
            not_before_ms,
            not_after_ms,
        })
    }

    fn active_at(&self, timestamp_ms: i64) -> bool {
        (self.not_before_ms..=self.not_after_ms).contains(&timestamp_ms)
    }
}

impl fmt::Debug for WebhookSecretV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookSecretV1")
            .field("key_id", &self.key_id)
            .field("secret", &"<redacted>")
            .field("not_before_ms", &self.not_before_ms)
            .field("not_after_ms", &self.not_after_ms)
            .finish()
    }
}

pub struct WebhookKeyRingV1 {
    provider: String,
    secrets: Vec<WebhookSecretV1>,
    replay_window_ms: i64,
}

impl WebhookKeyRingV1 {
    pub fn new(
        provider: impl Into<String>,
        secrets: Vec<WebhookSecretV1>,
        replay_window_ms: i64,
    ) -> Result<Self, WebhookErrorV1> {
        let provider = provider.into();
        if !safe_token(&provider, 64)
            || secrets.is_empty()
            || secrets.len() > MAX_WEBHOOK_SECRETS
            || !(1_000..=15 * 60 * 1_000).contains(&replay_window_ms)
        {
            return Err(WebhookErrorV1::InvalidConfiguration);
        }
        for (index, left) in secrets.iter().enumerate() {
            if secrets
                .iter()
                .skip(index + 1)
                .any(|right| right.key_id == left.key_id)
            {
                return Err(WebhookErrorV1::InvalidConfiguration);
            }
        }
        Ok(Self {
            provider,
            secrets,
            replay_window_ms,
        })
    }
}

impl fmt::Debug for WebhookKeyRingV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookKeyRingV1")
            .field("provider", &self.provider)
            .field(
                "secrets",
                &format_args!("<redacted:{} keys>", self.secrets.len()),
            )
            .field("replay_window_ms", &self.replay_window_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedWebhookV1 {
    pub provider: String,
    pub key_id: String,
    pub timestamp_ms: i64,
    pub replay_digest: ChecksumSha256,
}

impl fmt::Debug for VerifiedWebhookV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedWebhookV1")
            .field("provider", &self.provider)
            .field("key_id", &self.key_id)
            .field("timestamp_ms", &self.timestamp_ms)
            .field("replay_digest", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayClaimV1 {
    Claimed,
    Duplicate,
}

#[async_trait]
pub trait WebhookReplayStoreV1: Send + Sync {
    /// Atomically insert the digest if absent. Implementations must retain it
    /// through `expires_at_ms` and return `Duplicate` for every replay.
    async fn claim_once(
        &self,
        digest: &ChecksumSha256,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<ReplayClaimV1, WebhookStoreErrorV1>;
}

#[derive(Default)]
pub struct MemoryWebhookReplayStoreV1 {
    rows: RwLock<BTreeMap<String, i64>>,
}

#[async_trait]
impl WebhookReplayStoreV1 for MemoryWebhookReplayStoreV1 {
    async fn claim_once(
        &self,
        digest: &ChecksumSha256,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<ReplayClaimV1, WebhookStoreErrorV1> {
        if TimestampMillis::new(now_ms).is_err()
            || TimestampMillis::new(expires_at_ms).is_err()
            || expires_at_ms <= now_ms
            || expires_at_ms - now_ms > MAX_REPLAY_CLAIM_LIFETIME_MS
        {
            return Err(WebhookStoreErrorV1);
        }
        let mut rows = self.rows.write().map_err(|_| WebhookStoreErrorV1)?;
        rows.retain(|_, expires| *expires >= now_ms);
        let key = digest.as_str().to_owned();
        if rows.contains_key(&key) {
            return Ok(ReplayClaimV1::Duplicate);
        }
        if rows.len() >= MAX_REPLAY_ROWS {
            return Err(WebhookStoreErrorV1);
        }
        rows.insert(key, expires_at_ms);
        Ok(ReplayClaimV1::Claimed)
    }
}

pub struct WebhookVerifierV1<S> {
    key_ring: WebhookKeyRingV1,
    replay_store: S,
}

impl<S> WebhookVerifierV1<S>
where
    S: WebhookReplayStoreV1,
{
    #[must_use]
    pub const fn new(key_ring: WebhookKeyRingV1, replay_store: S) -> Self {
        Self {
            key_ring,
            replay_store,
        }
    }

    pub async fn verify(
        &self,
        signature_header: &str,
        timestamp_ms: i64,
        body: &[u8],
        now_ms: i64,
    ) -> Result<VerifiedWebhookV1, WebhookErrorV1> {
        if body.len() as u64 > MAX_WEBHOOK_BODY_BYTES_V1
            || TimestampMillis::new(timestamp_ms).is_err()
            || TimestampMillis::new(now_ms).is_err()
            || timestamp_ms.abs_diff(now_ms) > self.key_ring.replay_window_ms as u64
        {
            return Err(WebhookErrorV1::Rejected);
        }
        let signature = parse_signature(signature_header)?;
        let signed = signed_webhook_payload(timestamp_ms, body);
        let mut matched_key_id = None;
        for secret in self
            .key_ring
            .secrets
            .iter()
            // Key activity is evaluated at receipt time. A provider-delivery
            // overlap belongs in the configured key lifetime; accepting a
            // backdated signature after expiry would extend a revoked key.
            .filter(|secret| secret.active_at(now_ms))
        {
            let expected = hmac_sha256(&secret.secret, &signed);
            if constant_time_equal(&expected, &signature) {
                matched_key_id = Some(secret.key_id.clone());
            }
        }
        let key_id = matched_key_id.ok_or(WebhookErrorV1::Rejected)?;

        let replay_digest = ChecksumSha256::parse(hex_lower(&Sha256::digest(
            [
                self.key_ring.provider.as_bytes(),
                b"\0",
                timestamp_ms.to_string().as_bytes(),
                b"\0",
                signature.as_slice(),
            ]
            .concat(),
        )))
        .map_err(|_| WebhookErrorV1::Rejected)?;
        let expires_at_ms = timestamp_ms
            .checked_add(self.key_ring.replay_window_ms)
            .ok_or(WebhookErrorV1::Rejected)?;
        if expires_at_ms <= now_ms {
            return Err(WebhookErrorV1::Rejected);
        }
        match self
            .replay_store
            .claim_once(&replay_digest, expires_at_ms, now_ms)
            .await
            .map_err(|_| WebhookErrorV1::Unavailable)?
        {
            ReplayClaimV1::Claimed => Ok(VerifiedWebhookV1 {
                provider: self.key_ring.provider.clone(),
                key_id,
                timestamp_ms,
                replay_digest,
            }),
            ReplayClaimV1::Duplicate => Err(WebhookErrorV1::Rejected),
        }
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum WebhookErrorV1 {
    #[error("webhook verifier configuration is invalid")]
    InvalidConfiguration,
    #[error("webhook request rejected")]
    Rejected,
    #[error("webhook replay authority is unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("webhook replay store is unavailable")]
pub struct WebhookStoreErrorV1;

fn parse_signature(value: &str) -> Result<[u8; 32], WebhookErrorV1> {
    let value = value
        .strip_prefix(WEBHOOK_SIGNATURE_PREFIX)
        .ok_or(WebhookErrorV1::Rejected)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(WebhookErrorV1::Rejected);
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(bytes)
}

fn hex_nibble(value: u8) -> Result<u8, WebhookErrorV1> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(WebhookErrorV1::Rejected),
    }
}

fn signed_webhook_payload(timestamp_ms: i64, body: &[u8]) -> Vec<u8> {
    let mut signed = timestamp_ms.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(body);
    signed
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for ((inner, outer), secret) in inner_pad
        .iter_mut()
        .zip(outer_pad.iter_mut())
        .zip(normalized)
    {
        *inner ^= secret;
        *outer ^= secret;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn hex_lower(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableWorkflowStateV1 {
    Queued,
    Running,
    WaitingForProvider,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderEffectStateV1 {
    NotStarted,
    Submitted,
    Confirmed,
    Rejected,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSubmissionDecisionV1 {
    Submit,
    ReconcileExisting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowClaimOutcomeV1 {
    Claimed { fence: u64, attempt: u16 },
    Busy,
    Terminal,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DurableWorkflowV1 {
    workflow_id: String,
    idempotency_key: IdempotencyKey,
    state: DurableWorkflowStateV1,
    provider_effect: ProviderEffectStateV1,
    provider_idempotency_key: String,
    fence: u64,
    attempt: u16,
    max_attempts: u16,
    checkpoint: u32,
    lease_holder: Option<String>,
    lease_expires_at_ms: Option<i64>,
    next_attempt_at_ms: i64,
    result_checksum: Option<ChecksumSha256>,
}

impl DurableWorkflowV1 {
    pub fn new(
        workflow_id: impl Into<String>,
        idempotency_key: IdempotencyKey,
        provider_idempotency_key: impl Into<String>,
        max_attempts: u16,
        now_ms: i64,
    ) -> Result<Self, WorkflowErrorV1> {
        let workflow_id = workflow_id.into();
        let provider_idempotency_key = provider_idempotency_key.into();
        if !safe_token(&workflow_id, 96)
            || !safe_token(&provider_idempotency_key, 128)
            || !(1..=MAX_WORKFLOW_ATTEMPTS).contains(&max_attempts)
            || TimestampMillis::new(now_ms).is_err()
        {
            return Err(WorkflowErrorV1::Invalid);
        }
        Ok(Self {
            workflow_id,
            idempotency_key,
            state: DurableWorkflowStateV1::Queued,
            provider_effect: ProviderEffectStateV1::NotStarted,
            provider_idempotency_key,
            fence: 0,
            attempt: 0,
            max_attempts,
            checkpoint: 0,
            lease_holder: None,
            lease_expires_at_ms: None,
            next_attempt_at_ms: now_ms,
            result_checksum: None,
        })
    }

    #[must_use]
    pub const fn state(&self) -> DurableWorkflowStateV1 {
        self.state
    }

    #[must_use]
    pub const fn provider_effect(&self) -> ProviderEffectStateV1 {
        self.provider_effect
    }

    #[must_use]
    pub const fn checkpoint(&self) -> u32 {
        self.checkpoint
    }

    #[must_use]
    pub const fn attempt(&self) -> u16 {
        self.attempt
    }

    #[must_use]
    pub const fn fence(&self) -> u64 {
        self.fence
    }

    #[must_use]
    pub fn provider_idempotency_key(&self) -> &str {
        &self.provider_idempotency_key
    }

    pub fn claim(
        &mut self,
        worker: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<WorkflowClaimOutcomeV1, WorkflowErrorV1> {
        if !safe_token(worker, 96)
            || TimestampMillis::new(now_ms).is_err()
            || !(1..=MAX_LEASE_MS).contains(&lease_ms)
        {
            return Err(WorkflowErrorV1::Invalid);
        }
        if matches!(
            self.state,
            DurableWorkflowStateV1::Succeeded
                | DurableWorkflowStateV1::Failed
                | DurableWorkflowStateV1::Cancelled
        ) {
            return Ok(WorkflowClaimOutcomeV1::Terminal);
        }
        // An indeterminate external effect is reconciled by query, never by
        // handing the normal execution step to another worker for resubmission.
        if self.state == DurableWorkflowStateV1::WaitingForProvider {
            return Ok(WorkflowClaimOutcomeV1::Busy);
        }
        if self.next_attempt_at_ms > now_ms {
            return Ok(WorkflowClaimOutcomeV1::Busy);
        }
        if self
            .lease_expires_at_ms
            .is_some_and(|expires| expires > now_ms)
        {
            return Ok(WorkflowClaimOutcomeV1::Busy);
        }
        if self.attempt >= self.max_attempts {
            self.state = DurableWorkflowStateV1::Failed;
            self.clear_lease();
            return Ok(WorkflowClaimOutcomeV1::Terminal);
        }
        let next_fence = self.fence.checked_add(1).ok_or(WorkflowErrorV1::Invalid)?;
        let lease_expires_at_ms = checked_timestamp_add(now_ms, lease_ms)?;
        self.fence = next_fence;
        self.attempt += 1;
        self.state = DurableWorkflowStateV1::Running;
        self.lease_holder = Some(worker.into());
        self.lease_expires_at_ms = Some(lease_expires_at_ms);
        Ok(WorkflowClaimOutcomeV1::Claimed {
            fence: self.fence,
            attempt: self.attempt,
        })
    }

    pub fn heartbeat(
        &mut self,
        worker: &str,
        fence: u64,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<(), WorkflowErrorV1> {
        self.require_lease(worker, fence, now_ms)?;
        if !(1..=MAX_LEASE_MS).contains(&lease_ms) {
            return Err(WorkflowErrorV1::Invalid);
        }
        self.lease_expires_at_ms = Some(checked_timestamp_add(now_ms, lease_ms)?);
        Ok(())
    }

    pub fn advance_checkpoint(
        &mut self,
        worker: &str,
        fence: u64,
        expected: u32,
        next: u32,
        now_ms: i64,
    ) -> Result<(), WorkflowErrorV1> {
        self.require_lease(worker, fence, now_ms)?;
        let expected_next = expected.checked_add(1).ok_or(WorkflowErrorV1::Invalid)?;
        if self.checkpoint != expected || next != expected_next {
            return Err(WorkflowErrorV1::Conflict);
        }
        self.checkpoint = next;
        Ok(())
    }

    pub fn plan_provider_submission(
        &mut self,
        worker: &str,
        fence: u64,
        now_ms: i64,
    ) -> Result<ProviderSubmissionDecisionV1, WorkflowErrorV1> {
        self.require_lease(worker, fence, now_ms)?;
        match self.provider_effect {
            ProviderEffectStateV1::NotStarted => {
                self.provider_effect = ProviderEffectStateV1::Submitted;
                Ok(ProviderSubmissionDecisionV1::Submit)
            }
            ProviderEffectStateV1::Submitted => Ok(ProviderSubmissionDecisionV1::ReconcileExisting),
            _ => Err(WorkflowErrorV1::Conflict),
        }
    }

    pub fn record_provider_outcome(
        &mut self,
        worker: &str,
        fence: u64,
        outcome: ProviderEffectStateV1,
        now_ms: i64,
        retry_after_ms: i64,
    ) -> Result<(), WorkflowErrorV1> {
        self.require_lease(worker, fence, now_ms)?;
        if self.provider_effect != ProviderEffectStateV1::Submitted
            || !matches!(
                outcome,
                ProviderEffectStateV1::Confirmed
                    | ProviderEffectStateV1::Rejected
                    | ProviderEffectStateV1::Indeterminate
            )
        {
            return Err(WorkflowErrorV1::Conflict);
        }
        let next_attempt_at_ms = if outcome == ProviderEffectStateV1::Indeterminate {
            validate_retry_delay(retry_after_ms)?;
            Some(checked_timestamp_add(now_ms, retry_after_ms)?)
        } else {
            None
        };
        self.provider_effect = outcome;
        match outcome {
            ProviderEffectStateV1::Indeterminate => {
                self.state = DurableWorkflowStateV1::WaitingForProvider;
                self.next_attempt_at_ms =
                    next_attempt_at_ms.expect("validated indeterminate retry");
                self.clear_lease();
            }
            ProviderEffectStateV1::Rejected => {
                self.state = DurableWorkflowStateV1::Failed;
                self.clear_lease();
            }
            ProviderEffectStateV1::Confirmed => {}
            ProviderEffectStateV1::NotStarted | ProviderEffectStateV1::Submitted => {
                return Err(WorkflowErrorV1::Conflict);
            }
        }
        Ok(())
    }

    /// Reconciliation may move an indeterminate provider effect back to a
    /// confirmed or rejected terminal fact; it may never resubmit with a new
    /// provider idempotency key.
    pub fn reconcile_provider(
        &mut self,
        outcome: ProviderEffectStateV1,
        now_ms: i64,
    ) -> Result<(), WorkflowErrorV1> {
        if self.state != DurableWorkflowStateV1::WaitingForProvider
            || self.provider_effect != ProviderEffectStateV1::Indeterminate
            || !matches!(
                outcome,
                ProviderEffectStateV1::Confirmed | ProviderEffectStateV1::Rejected
            )
            || TimestampMillis::new(now_ms).is_err()
        {
            return Err(WorkflowErrorV1::Conflict);
        }
        self.provider_effect = outcome;
        self.next_attempt_at_ms = now_ms;
        self.state = if outcome == ProviderEffectStateV1::Confirmed {
            DurableWorkflowStateV1::Queued
        } else {
            DurableWorkflowStateV1::Failed
        };
        Ok(())
    }

    pub fn retry_after_failure(
        &mut self,
        worker: &str,
        fence: u64,
        now_ms: i64,
        retry_after_ms: i64,
    ) -> Result<(), WorkflowErrorV1> {
        self.require_lease(worker, fence, now_ms)?;
        validate_retry_delay(retry_after_ms)?;
        if self.attempt >= self.max_attempts {
            self.state = DurableWorkflowStateV1::Failed;
        } else {
            let next_attempt_at_ms = checked_timestamp_add(now_ms, retry_after_ms)?;
            self.state = DurableWorkflowStateV1::Queued;
            self.next_attempt_at_ms = next_attempt_at_ms;
        }
        self.clear_lease();
        Ok(())
    }

    pub fn complete(
        &mut self,
        worker: &str,
        fence: u64,
        now_ms: i64,
        result_checksum: ChecksumSha256,
    ) -> Result<(), WorkflowErrorV1> {
        self.require_lease(worker, fence, now_ms)?;
        if matches!(
            self.provider_effect,
            ProviderEffectStateV1::Submitted | ProviderEffectStateV1::Indeterminate
        ) {
            return Err(WorkflowErrorV1::Conflict);
        }
        self.state = DurableWorkflowStateV1::Succeeded;
        self.result_checksum = Some(result_checksum);
        self.clear_lease();
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), WorkflowErrorV1> {
        if matches!(
            self.state,
            DurableWorkflowStateV1::Succeeded | DurableWorkflowStateV1::Failed
        ) || matches!(
            self.provider_effect,
            ProviderEffectStateV1::Submitted
                | ProviderEffectStateV1::Confirmed
                | ProviderEffectStateV1::Indeterminate
        ) {
            return Err(WorkflowErrorV1::Conflict);
        }
        self.state = DurableWorkflowStateV1::Cancelled;
        self.clear_lease();
        Ok(())
    }

    fn require_lease(&self, worker: &str, fence: u64, now_ms: i64) -> Result<(), WorkflowErrorV1> {
        if TimestampMillis::new(now_ms).is_err() {
            return Err(WorkflowErrorV1::Invalid);
        }
        if self.state != DurableWorkflowStateV1::Running
            || self.fence != fence
            || self.lease_holder.as_deref() != Some(worker)
            || self
                .lease_expires_at_ms
                .is_none_or(|expires| expires <= now_ms)
        {
            return Err(WorkflowErrorV1::StaleLease);
        }
        Ok(())
    }

    fn clear_lease(&mut self) {
        self.lease_holder = None;
        self.lease_expires_at_ms = None;
    }
}

impl fmt::Debug for DurableWorkflowV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableWorkflowV1")
            .field("workflow_id", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("state", &self.state)
            .field("provider_effect", &self.provider_effect)
            .field("provider_idempotency_key", &"<redacted>")
            .field("fence", &self.fence)
            .field("attempt", &self.attempt)
            .field("checkpoint", &self.checkpoint)
            .field(
                "lease_holder",
                &self.lease_holder.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum WorkflowErrorV1 {
    #[error("workflow input is invalid")]
    Invalid,
    #[error("workflow transition conflicts with durable state")]
    Conflict,
    #[error("workflow lease is stale")]
    StaleLease,
}

fn validate_retry_delay(value: i64) -> Result<(), WorkflowErrorV1> {
    if !(1..=MAX_RETRY_DELAY_MS).contains(&value) {
        return Err(WorkflowErrorV1::Invalid);
    }
    Ok(())
}

fn checked_timestamp_add(now_ms: i64, duration_ms: i64) -> Result<i64, WorkflowErrorV1> {
    let value = now_ms
        .checked_add(duration_ms)
        .ok_or(WorkflowErrorV1::Invalid)?;
    TimestampMillis::new(value).map_err(|_| WorkflowErrorV1::Invalid)?;
    Ok(value)
}

fn safe_token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

#[cfg(test)]
mod tests {
    use frame_domain::{
        ApiAuthClassV1, ApiContractErrorV1, ApiRequestPolicyV1, IdempotencyRequirementV1,
        MAX_TIMESTAMP_MS,
    };

    use super::*;

    fn checksum(byte: u8) -> ChecksumSha256 {
        ChecksumSha256::parse(format!("{byte:02x}").repeat(32)).expect("checksum")
    }

    fn webhook_secret(id: &str, byte: u8, start: i64, end: i64) -> WebhookSecretV1 {
        WebhookSecretV1::new(id, vec![byte; 32], start, end).expect("secret")
    }

    #[test]
    fn admission_collapses_forbidden_resources_and_redacts_trace() {
        let policy = ApiRequestPolicyV1 {
            auth: ApiAuthClassV1::Session,
            max_body_bytes: 100,
            accepted_content_types: vec!["application/json".into()],
            idempotency: IdempotencyRequirementV1::Required,
            rate_limit_bucket: "video_write".into(),
            audit_action: "video.update".into(),
        };
        let envelope = ApiMutationEnvelopeV1 {
            content_length: 2,
            content_type: Some("application/json".into()),
            idempotency_key: Some(IdempotencyKey::parse("request-1").expect("key")),
            correlation_id: "trace-1".into(),
        };
        let error = ApiGatewayV1::admit_mutation(
            &policy,
            &envelope,
            RequestSecurityContextV1 {
                authenticated: true,
                authorized: false,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit: RateLimitDecisionV1::Allowed,
            },
        )
        .expect_err("forbidden");
        assert_eq!(error.code, ApiErrorCodeV1::NotFound);
        assert!(!format!("{error:?}").contains("trace-1"));

        let error = ApiGatewayV1::admit_mutation(
            &policy,
            &envelope,
            RequestSecurityContextV1 {
                authenticated: true,
                authorized: true,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit: RateLimitDecisionV1::Rejected { retry_after_ms: 0 },
            },
        )
        .expect_err("invalid limiter duration fails closed without panicking");
        assert_eq!(error.code, ApiErrorCodeV1::RateLimited);
        assert_eq!(error.retry_after_ms, None);
    }

    #[test]
    fn session_or_api_key_requires_one_authenticated_transport_capability() {
        let policy = ApiRequestPolicyV1 {
            auth: ApiAuthClassV1::SessionOrApiKey,
            max_body_bytes: 0,
            accepted_content_types: Vec::new(),
            idempotency: IdempotencyRequirementV1::Optional,
            rate_limit_bucket: "mobile_compatibility_v1".into(),
            audit_action: "mobile.folder.create".into(),
        };
        let envelope = ApiMutationEnvelopeV1 {
            content_length: 0,
            content_type: None,
            idempotency_key: None,
            correlation_id: "trace-mobile-auth".into(),
        };
        let context = |authenticated| RequestSecurityContextV1 {
            authenticated,
            authorized: true,
            browser_origin_valid: true,
            csrf_valid: true,
            rate_limit: RateLimitDecisionV1::Allowed,
        };
        assert_eq!(
            ApiGatewayV1::admit_mutation(&policy, &envelope, context(false))
                .expect_err("neither credential class was authenticated")
                .code,
            ApiErrorCodeV1::Unauthenticated
        );
        assert!(ApiGatewayV1::admit_mutation(&policy, &envelope, context(true)).is_ok());
    }

    #[tokio::test]
    async fn webhook_signature_rotation_window_and_replay_are_enforced() {
        let ring = WebhookKeyRingV1::new(
            "stripe",
            vec![
                webhook_secret("previous", 7, 1_000, 20_000),
                webhook_secret("current", 9, 10_000, 40_000),
            ],
            5_000,
        )
        .expect("ring");
        let verifier = WebhookVerifierV1::new(ring, MemoryWebhookReplayStoreV1::default());
        let body = br#"{"id":"evt_fixture"}"#;
        let signed = signed_webhook_payload(15_000, body);
        let signature = format!("v1={}", hex_lower(&hmac_sha256(&[9; 32], &signed)));
        let verified = verifier
            .verify(&signature, 15_000, body, 16_000)
            .await
            .expect("verified");
        assert_eq!(verified.key_id, "current");
        assert_eq!(
            verifier.verify(&signature, 15_000, body, 16_001).await,
            Err(WebhookErrorV1::Rejected)
        );

        let old_signature = format!("v1={}", hex_lower(&hmac_sha256(&[7; 32], &signed)));
        // Both keys overlap, but a separately signed event can use the previous key.
        let previous = WebhookVerifierV1::new(
            WebhookKeyRingV1::new(
                "stripe-previous-test",
                vec![webhook_secret("previous", 7, 1_000, 20_000)],
                5_000,
            )
            .expect("ring"),
            MemoryWebhookReplayStoreV1::default(),
        )
        .verify(&old_signature, 15_000, body, 16_000)
        .await
        .expect("previous key");
        assert_eq!(previous.key_id, "previous");
    }

    #[tokio::test]
    async fn webhook_rejects_tampering_stale_events_and_oversized_bodies() {
        let ring = WebhookKeyRingV1::new(
            "provider",
            vec![webhook_secret("current", 3, 1_000, 100_000)],
            5_000,
        )
        .expect("ring");
        let verifier = WebhookVerifierV1::new(ring, MemoryWebhookReplayStoreV1::default());
        let body = b"fixture";
        let signature = format!(
            "v1={}",
            hex_lower(&hmac_sha256(
                &[3; 32],
                &signed_webhook_payload(10_000, body)
            ))
        );
        assert_eq!(
            verifier
                .verify(&signature, 10_000, b"tampered", 10_001)
                .await,
            Err(WebhookErrorV1::Rejected)
        );
        assert_eq!(
            verifier.verify(&signature, 10_000, body, 20_000).await,
            Err(WebhookErrorV1::Rejected)
        );
        assert_eq!(
            verifier.verify(&signature, 10_000, body, 15_000).await,
            Err(WebhookErrorV1::Rejected)
        );
        let oversized = vec![0; MAX_WEBHOOK_BODY_BYTES_V1 as usize + 1];
        assert_eq!(
            verifier
                .verify(&signature, 10_000, &oversized, 10_001)
                .await,
            Err(WebhookErrorV1::Rejected)
        );
    }

    #[tokio::test]
    async fn webhook_does_not_extend_an_expired_key_with_a_backdated_timestamp() {
        let ring = WebhookKeyRingV1::new(
            "provider",
            vec![webhook_secret("retired", 4, 1_000, 10_000)],
            5_000,
        )
        .expect("ring");
        let verifier = WebhookVerifierV1::new(ring, MemoryWebhookReplayStoreV1::default());
        let body = b"fixture";
        let signature = format!(
            "v1={}",
            hex_lower(&hmac_sha256(&[4; 32], &signed_webhook_payload(9_500, body)))
        );
        // The signed timestamp is fresh, but the key is no longer active when
        // the request arrives. Rotation overlap must be explicit in the ring.
        assert_eq!(
            verifier.verify(&signature, 9_500, body, 10_001).await,
            Err(WebhookErrorV1::Rejected)
        );
    }

    #[test]
    fn workflow_recovers_after_crash_with_a_new_fence() {
        let mut workflow = DurableWorkflowV1::new(
            "workflow-1",
            IdempotencyKey::parse("request-1").expect("key"),
            "provider-request-1",
            3,
            1_000,
        )
        .expect("workflow");
        assert_eq!(
            workflow.claim("worker-a", 1_000, 100),
            Ok(WorkflowClaimOutcomeV1::Claimed {
                fence: 1,
                attempt: 1
            })
        );
        workflow
            .advance_checkpoint("worker-a", 1, 0, 1, 1_001)
            .expect("checkpoint");
        assert_eq!(
            workflow.claim("worker-b", 1_050, 100),
            Ok(WorkflowClaimOutcomeV1::Busy)
        );
        assert_eq!(
            workflow.claim("worker-b", 1_101, 100),
            Ok(WorkflowClaimOutcomeV1::Claimed {
                fence: 2,
                attempt: 2
            })
        );
        assert_eq!(
            workflow.advance_checkpoint("worker-a", 1, 1, 2, 1_102),
            Err(WorkflowErrorV1::StaleLease)
        );
        assert_eq!(workflow.checkpoint(), 1);
    }

    #[test]
    fn workflow_fences_indeterminate_provider_effects_from_resubmission() {
        let mut workflow = DurableWorkflowV1::new(
            "workflow-2",
            IdempotencyKey::parse("request-2").expect("key"),
            "provider-request-2",
            4,
            1_000,
        )
        .expect("workflow");
        let WorkflowClaimOutcomeV1::Claimed { fence, .. } =
            workflow.claim("worker-a", 1_000, 100).expect("claim")
        else {
            panic!("expected claim");
        };
        assert_eq!(
            workflow.plan_provider_submission("worker-a", fence, 1_001),
            Ok(ProviderSubmissionDecisionV1::Submit)
        );
        assert_eq!(
            workflow.plan_provider_submission("worker-a", fence, 1_001),
            Ok(ProviderSubmissionDecisionV1::ReconcileExisting)
        );
        assert_eq!(workflow.cancel(), Err(WorkflowErrorV1::Conflict));
        workflow
            .record_provider_outcome(
                "worker-a",
                fence,
                ProviderEffectStateV1::Indeterminate,
                1_002,
                100,
            )
            .expect("indeterminate");
        assert_eq!(
            workflow.claim("worker-b", 1_102, 100),
            Ok(WorkflowClaimOutcomeV1::Busy)
        );
        // Crash recovery queries the provider using the same durable key.
        assert_eq!(workflow.provider_idempotency_key(), "provider-request-2");
    }

    #[test]
    fn workflow_reconciliation_handles_partial_provider_failure() {
        let mut workflow = DurableWorkflowV1::new(
            "workflow-3",
            IdempotencyKey::parse("request-3").expect("key"),
            "provider-request-3",
            3,
            1_000,
        )
        .expect("workflow");
        let WorkflowClaimOutcomeV1::Claimed { fence, .. } =
            workflow.claim("worker", 1_000, 100).expect("claim")
        else {
            panic!("expected claim");
        };
        assert_eq!(
            workflow.plan_provider_submission("worker", fence, 1_001),
            Ok(ProviderSubmissionDecisionV1::Submit)
        );
        workflow
            .record_provider_outcome(
                "worker",
                fence,
                ProviderEffectStateV1::Indeterminate,
                1_002,
                100,
            )
            .expect("indeterminate");
        workflow
            .reconcile_provider(ProviderEffectStateV1::Confirmed, 1_050)
            .expect("reconcile");
        let WorkflowClaimOutcomeV1::Claimed { fence, .. } =
            workflow.claim("worker", 1_050, 100).expect("claim")
        else {
            panic!("expected claim");
        };
        workflow
            .complete("worker", fence, 1_051, checksum(7))
            .expect("complete");
        assert_eq!(workflow.state(), DurableWorkflowStateV1::Succeeded);
        assert_eq!(workflow.provider_effect(), ProviderEffectStateV1::Confirmed);
    }

    #[test]
    fn retry_limit_and_cancel_are_terminal_and_idempotent_at_claim_boundary() {
        let mut workflow = DurableWorkflowV1::new(
            "workflow-4",
            IdempotencyKey::parse("request-4").expect("key"),
            "provider-request-4",
            1,
            1_000,
        )
        .expect("workflow");
        let WorkflowClaimOutcomeV1::Claimed { fence, .. } =
            workflow.claim("worker", 1_000, 100).expect("claim")
        else {
            panic!("expected claim");
        };
        workflow
            .retry_after_failure("worker", fence, 1_001, 10)
            .expect("failure");
        assert_eq!(workflow.state(), DurableWorkflowStateV1::Failed);
        assert_eq!(
            workflow.claim("worker", 2_000, 100),
            Ok(WorkflowClaimOutcomeV1::Terminal)
        );

        let mut cancelled = DurableWorkflowV1::new(
            "workflow-5",
            IdempotencyKey::parse("request-5").expect("key"),
            "provider-request-5",
            2,
            1_000,
        )
        .expect("workflow");
        cancelled.cancel().expect("cancel");
        assert_eq!(
            cancelled.claim("worker", 1_001, 100),
            Ok(WorkflowClaimOutcomeV1::Terminal)
        );
    }

    #[test]
    fn workflow_rejects_invalid_clock_and_checkpoint_boundaries_without_mutation() {
        let mut workflow = DurableWorkflowV1::new(
            "workflow-boundary",
            IdempotencyKey::parse("request-boundary").expect("key"),
            "provider-boundary",
            2,
            MAX_TIMESTAMP_MS,
        )
        .expect("workflow");
        assert_eq!(
            workflow.claim("worker", MAX_TIMESTAMP_MS, 1),
            Err(WorkflowErrorV1::Invalid)
        );
        assert_eq!(workflow.state(), DurableWorkflowStateV1::Queued);
        assert_eq!(workflow.fence(), 0);
        assert_eq!(workflow.attempt(), 0);

        let mut workflow = DurableWorkflowV1::new(
            "workflow-checkpoint",
            IdempotencyKey::parse("request-checkpoint").expect("key"),
            "provider-checkpoint",
            2,
            1_000,
        )
        .expect("workflow");
        let WorkflowClaimOutcomeV1::Claimed { fence, .. } =
            workflow.claim("worker", 1_000, 100).expect("claim")
        else {
            panic!("expected claim");
        };
        assert_eq!(
            workflow.advance_checkpoint("worker", fence, u32::MAX, u32::MAX, 1_001),
            Err(WorkflowErrorV1::Invalid)
        );
        assert_eq!(workflow.checkpoint(), 0);
        assert_eq!(
            workflow.heartbeat("worker", fence, -1, 100),
            Err(WorkflowErrorV1::Invalid)
        );
    }

    #[test]
    fn hmac_matches_rfc_4231_sha256_vector() {
        let key = vec![0x0b; 20];
        let digest = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex_lower(&digest),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn configuration_and_debug_never_expose_secrets() {
        let secret = webhook_secret("current", 0x5a, 1_000, 10_000);
        assert!(!format!("{secret:?}").contains("5a"));
        assert!(WebhookSecretV1::new("bad", vec![1; 8], 1_000, 10_000).is_err());
        assert!(WebhookKeyRingV1::new("provider", vec![], 5_000).is_err());
    }

    #[test]
    fn local_workflow_fault_load_preserves_fences_and_terminal_results() {
        const RUNS: u16 = 5_000;
        for index in 0..RUNS {
            let mut workflow = DurableWorkflowV1::new(
                format!("load-workflow-{index}"),
                IdempotencyKey::parse(format!("load-request-{index}")).expect("key"),
                format!("load-provider-{index}"),
                2,
                1_000,
            )
            .expect("workflow");
            let WorkflowClaimOutcomeV1::Claimed { fence, attempt } =
                workflow.claim("load-worker-a", 1_000, 10).expect("claim")
            else {
                panic!("expected claim");
            };
            assert_eq!(attempt, 1);
            workflow
                .advance_checkpoint("load-worker-a", fence, 0, 1, 1_001)
                .expect("checkpoint");

            if index % 2 == 0 {
                // Simulate a process crash. The stale fence can no longer write.
                let WorkflowClaimOutcomeV1::Claimed {
                    fence: recovered_fence,
                    attempt: recovered_attempt,
                } = workflow
                    .claim("load-worker-b", 1_011, 10)
                    .expect("recovery claim")
                else {
                    panic!("expected recovery claim");
                };
                assert_eq!(recovered_attempt, 2);
                assert!(recovered_fence > fence);
                assert_eq!(
                    workflow.complete("load-worker-a", fence, 1_012, checksum(3)),
                    Err(WorkflowErrorV1::StaleLease)
                );
                workflow
                    .complete("load-worker-b", recovered_fence, 1_012, checksum(4))
                    .expect("recovered completion");
            } else {
                workflow
                    .complete("load-worker-a", fence, 1_002, checksum(5))
                    .expect("completion");
            }
            assert_eq!(workflow.state(), DurableWorkflowStateV1::Succeeded);
            assert_eq!(
                workflow.claim("late-worker", 2_000, 10),
                Ok(WorkflowClaimOutcomeV1::Terminal)
            );
        }
    }

    #[test]
    fn domain_validation_errors_map_to_closed_public_errors() {
        let _: Option<ApiContractErrorV1> = None;
        let error =
            ApiErrorV1::new(ApiErrorCodeV1::InvalidRequest, "trace", None).expect("public error");
        assert_eq!(error.code.http_status(), 400);
    }
}
