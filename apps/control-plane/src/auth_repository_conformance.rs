//! Closed, token-gated conformance surface for the D1 authentication adapter.
//!
//! The exact route is rejected in production before Host, method, token, or
//! body parsing. Every identifier and digest is compiled into this module;
//! callers can select only a fixed scenario and can never submit SQL, secrets,
//! identifiers, timestamps, or state transitions.

use frame_domain::{
    ApiKeyId, ApiKeyScope, AuthAuditAction, AuthAuditReason, AuthClientKind, AuthSessionRecord,
    AuthSessionState, CorrelationId, DeliveryDestinationRef, DurationMillis, HashKeyVersion,
    IdentityProvisioningGrantId, ManagedApiKeyRecord, MultiRateLimitPolicy, OAuthFlowId,
    OAuthFlowPurpose, OAuthFlowRecord, OAuthProvider, OrganizationRole, PrincipalSnapshot,
    RateLimitPolicy, SealedDeliveryEnvelope, SecretDigest, SecretDigestCandidates,
    SessionContinuationBinding, SessionFamilyId, SessionId, SessionMutationGrantId,
    SessionRevocationReason, TenantGrant, TimestampMillis, UserId, VerificationChannel,
    VerificationPurpose, VersionedSecretDigest,
};
use frame_ports::{
    AbuseDigestSet, ApiKeyAuthenticationCommand, ApiKeyAuthenticationOutcome, ApiKeyIssueCommand,
    ApiKeyIssueOutcome, ApiKeyRevokeCommand, AuthDeliveryAcknowledgeOutcome,
    AuthDeliveryRetryOutcome, AuthStateRepository, DecisionAudit, ExternalIdentityAssertion,
    IdentityProvisionCommand, IdentityProvisionOutcome, IdentityProvisioningGrant,
    LogoutAllOutcome, OAuthBeginCommand, OAuthBeginOutcome, OAuthExchangeOutcome,
    OAuthFinalizeCommand, OAuthPreflightCommand, OAuthPreflightOutcome, OAuthProviderResult,
    PortError, PrincipalIssuanceGrant, SessionAuthenticationCommand, SessionIssueAuthority,
    SessionIssueCommand, SessionMutationGrant, SessionPresentation, SessionRevokeCommand,
    SessionRotationOutcome, SessionRotationRequest, VerificationAtomicOutcome,
    VerificationAttemptCommand, VerificationIssueAtomicOutcome, VerificationIssueCommand,
};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Method, Request, Response, Result};

use crate::{
    auth_repository::D1AuthStateRepository,
    contracts::{API_SCHEMA_VERSION, constant_time_eq},
};

const TOKEN_VARIABLE: &str = "FRAME_AUTH_REPOSITORY_CONFORMANCE_TOKEN";
const TOKEN_HEADER: &str = "x-frame-auth-repository-conformance-token";
const MAX_BODY_BYTES: usize = 160;
const NOW_MS: i64 = 1_700_100_000_000;

const USER_FOUND: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a101";
const USER_SINGLE_LOGOUT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a102";
const USER_LOGOUT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a501";
const USER_PROVISION: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a901";
const USER_ROLLBACK: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690aa01";
const USER_FENCE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690a701";
const USER_OAUTH: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690ac01";

const SESSION_FOUND: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690b101";
const SESSION_SINGLE_LOGOUT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690b102";
const SESSION_LOGOUT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690b501";
const SESSION_ROLLBACK: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690ba01";
const SESSION_FENCE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690b701";
const SESSION_OAUTH: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690bc01";
const FAMILY_ROLLBACK: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690ca01";
const ROTATE_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690d101";
const SINGLE_LOGOUT_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690d102";
const LOGOUT_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690d501";
const PROVISION_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690d901";
const ROLLBACK_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690da01";
const FENCE_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690d701";
const OAUTH_GRANT: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690dc01";
const OAUTH_SIGN_IN_FLOW: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690fc01";
const OAUTH_LINK_FLOW: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690fc02";
const TENANT_API: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e601";
const TENANT_FENCE: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690e701";
const DELIVERY_EXHAUST: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690f102";
const FIXTURE_OPERATION_ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a8690ff01";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceRequest {
    schema_version: u16,
    scenario: Scenario,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    VerificationIssueNearCap,
    VerificationIssueExistingBucket,
    SessionMatrix,
    LogoutAll,
    ApiKeyRotationAndCorrupt,
    VerificationReplay,
    DeliveryLifecycle,
    ProvisionRace,
    AtomicRollback,
    AuthorityFences,
    DeliveryLeaseCleanup,
    ClaimRace,
    ContentionRetries,
    OauthSignInLinkReplay,
}

impl Scenario {
    const fn name(self) -> &'static str {
        match self {
            Self::VerificationIssueNearCap => "verification_issue_near_cap",
            Self::VerificationIssueExistingBucket => "verification_issue_existing_bucket",
            Self::SessionMatrix => "session_matrix",
            Self::LogoutAll => "logout_all",
            Self::ApiKeyRotationAndCorrupt => "api_key_rotation_and_corrupt",
            Self::VerificationReplay => "verification_replay",
            Self::DeliveryLifecycle => "delivery_lifecycle",
            Self::ProvisionRace => "provision_race",
            Self::AtomicRollback => "atomic_rollback",
            Self::AuthorityFences => "authority_fences",
            Self::DeliveryLeaseCleanup => "delivery_lease_cleanup",
            Self::ClaimRace => "claim_race",
            Self::ContentionRetries => "contention_retries",
            Self::OauthSignInLinkReplay => "oauth_sign_in_link_replay",
        }
    }
}

#[derive(Debug, Deserialize)]
struct RollbackRow {
    grant_count: i64,
    session_count: i64,
}

#[derive(Debug, Deserialize)]
struct FenceRow {
    key_version: i64,
    key_digest: String,
}

#[derive(Debug, Deserialize)]
struct DeliveryCleanupRow {
    delivery_count: i64,
    tombstone_count: i64,
}

#[derive(Debug, Deserialize)]
struct AuthorityMutationRow {
    grant_count: i64,
    issued_key_count: i64,
    existing_key_revoked: i64,
    linked_identifier_count: i64,
    link_challenge_count: i64,
}

#[derive(Debug, Deserialize)]
struct VerificationIssueArtifactRow {
    pending_count: i64,
    challenge_count: i64,
    delivery_count: i64,
}

#[derive(Debug, Deserialize)]
struct OAuthArtifactRow {
    consumed_flows: i64,
    consumed_reservations: i64,
    external_accounts: i64,
    oauth_operations: i64,
}

pub async fn response(mut request: Request, env: &Env) -> Result<Response> {
    if request.method() != Method::Post {
        return fixed_response(405, "method_not_allowed", None);
    }
    let expected = env
        .var(TOKEN_VARIABLE)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let supplied = request.headers().get(TOKEN_HEADER)?.unwrap_or_default();
    if !valid_token(&expected)
        || !valid_token(&supplied)
        || !constant_time_eq(expected.as_bytes(), supplied.as_bytes())
    {
        return fixed_response(404, "not_found", None);
    }
    let content_type = request.headers().get("content-type")?.unwrap_or_default();
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<usize>().ok());
    if content_type != "application/json"
        || content_length.is_none_or(|length| length == 0 || length > MAX_BODY_BYTES)
    {
        return fixed_response(400, "invalid_request", None);
    }
    let bytes = request.bytes().await?;
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return fixed_response(400, "invalid_request", None);
    }
    let body = match serde_json::from_slice::<ConformanceRequest>(&bytes) {
        Ok(body) if body.schema_version == API_SCHEMA_VERSION => body,
        _ => return fixed_response(400, "invalid_request", None),
    };
    let database = env.d1("DB")?;
    match run_scenario(&database, body.scenario).await {
        Ok(values) => fixed_response(
            200,
            "ok",
            Some(json!({
                "schema_version": API_SCHEMA_VERSION,
                "scenario": body.scenario.name(),
                "values": values,
            })),
        ),
        Err(_) => fixed_response(
            500,
            "auth_repository_conformance_failed",
            Some(json!({
                "schema_version": API_SCHEMA_VERSION,
                "scenario": body.scenario.name(),
            })),
        ),
    }
}

async fn run_scenario(database: &D1Database, scenario: Scenario) -> Result<Value> {
    let repository = D1AuthStateRepository::new(database);
    match scenario {
        Scenario::VerificationIssueNearCap => {
            verification_issue_near_cap(database, &repository).await
        }
        Scenario::VerificationIssueExistingBucket => {
            verification_issue_existing_bucket(database, &repository).await
        }
        Scenario::SessionMatrix => session_matrix(&repository).await,
        Scenario::LogoutAll => logout_all(&repository).await,
        Scenario::ApiKeyRotationAndCorrupt => api_key_rotation_and_corrupt(&repository).await,
        Scenario::VerificationReplay => verification_replay(&repository).await,
        Scenario::DeliveryLifecycle => delivery_lifecycle(&repository).await,
        Scenario::ProvisionRace => provision_race(&repository).await,
        Scenario::AtomicRollback => atomic_rollback(database, &repository).await,
        Scenario::AuthorityFences => authority_fences(database, &repository).await,
        Scenario::DeliveryLeaseCleanup => delivery_lease_cleanup(database, &repository).await,
        Scenario::ClaimRace => claim_race(&repository).await,
        Scenario::ContentionRetries => contention_retries(&repository).await,
        Scenario::OauthSignInLinkReplay => oauth_sign_in_link_replay(database, &repository).await,
    }
}

async fn verification_issue_near_cap(
    database: &D1Database,
    repository: &D1AuthStateRepository<'_>,
) -> Result<Value> {
    let first = verification_issue_command(
        180,
        181,
        abuse(980)?,
        VerificationPurpose::SignIn,
        VerificationChannel::OneTimeCode,
        0xa1,
        NOW_MS - 500,
    )?;
    let second = verification_issue_command(
        190,
        191,
        abuse(990)?,
        VerificationPurpose::AccountRecovery,
        VerificationChannel::MagicLink,
        0xa2,
        NOW_MS - 500,
    )?;
    let (first_outcome, second_outcome) = futures::join!(
        repository.issue_verification(first.clone()),
        repository.issue_verification(second.clone()),
    );
    let outcomes = [&first_outcome, &second_outcome];
    let accepted = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Ok(VerificationIssueAtomicOutcome::Accepted)))
        .count();
    let rate_limited = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome,
                Ok(VerificationIssueAtomicOutcome::RateLimited { .. })
            )
        })
        .count();
    if accepted != 1 || rate_limited != 1 {
        return Err(fixed_failure());
    }
    let before = verification_issue_artifacts(database, &first, &second, 180, 190).await?;
    let materialized = repository
        .materialize_verification_deliveries(time(NOW_MS)?, 1)
        .await
        .map_err(|_| fixed_failure())?;
    let after = verification_issue_artifacts(database, &first, &second, 180, 190).await?;
    let (first_replay, second_replay) = futures::join!(
        repository.issue_verification(first.clone()),
        repository.issue_verification(second.clone()),
    );
    let after_replay = verification_issue_artifacts(database, &first, &second, 180, 190).await?;
    if before.pending_count != 1
        || before.challenge_count != 0
        || before.delivery_count != 0
        || materialized != 1
        || after.pending_count != 0
        || after.challenge_count != 1
        || after.delivery_count != 1
        || after_replay.pending_count != 0
        || after_replay.challenge_count != 1
        || after_replay.delivery_count != 1
        || !same_issue_outcome(&first_outcome, &first_replay)
        || !same_issue_outcome(&second_outcome, &second_replay)
    {
        return Err(fixed_failure());
    }
    cleanup_verification_issue_artifacts(database, &first, &second, 180, 190, true).await?;
    Ok(json!({
        "accepted": 1,
        "rate_limited": 1,
        "replay_exact": true,
        "challenge_count": 1,
        "delivery_count": 1,
    }))
}

async fn verification_issue_existing_bucket(
    database: &D1Database,
    repository: &D1AuthStateRepository<'_>,
) -> Result<Value> {
    seed_existing_issue_buckets(database).await?;
    let shared_abuse = abuse(950)?;
    let first = verification_issue_command(
        150,
        160,
        shared_abuse.clone(),
        VerificationPurpose::SignIn,
        VerificationChannel::OneTimeCode,
        0xb1,
        NOW_MS - 400,
    )?;
    let second = verification_issue_command(
        151,
        161,
        shared_abuse,
        VerificationPurpose::SignIn,
        VerificationChannel::MagicLink,
        0xb2,
        NOW_MS - 400,
    )?;
    let (first_outcome, second_outcome) = futures::join!(
        repository.issue_verification(first.clone()),
        repository.issue_verification(second.clone()),
    );
    if !matches!(first_outcome, Ok(VerificationIssueAtomicOutcome::Accepted))
        || !matches!(second_outcome, Ok(VerificationIssueAtomicOutcome::Accepted))
    {
        return Err(fixed_failure());
    }
    let before = verification_issue_artifacts(database, &first, &second, 150, 151).await?;
    let materialized = repository
        .materialize_verification_deliveries(time(NOW_MS)?, 2)
        .await
        .map_err(|_| fixed_failure())?;
    let after = verification_issue_artifacts(database, &first, &second, 150, 151).await?;
    let (first_replay, second_replay) = futures::join!(
        repository.issue_verification(first.clone()),
        repository.issue_verification(second.clone()),
    );
    let after_replay = verification_issue_artifacts(database, &first, &second, 150, 151).await?;
    if before.pending_count != 2
        || before.challenge_count != 0
        || before.delivery_count != 0
        || materialized != 2
        || after.pending_count != 0
        || after.challenge_count != 2
        || after.delivery_count != 2
        || after_replay.pending_count != 0
        || after_replay.challenge_count != 2
        || after_replay.delivery_count != 2
        || !same_issue_outcome(&first_outcome, &first_replay)
        || !same_issue_outcome(&second_outcome, &second_replay)
    {
        return Err(fixed_failure());
    }
    cleanup_verification_issue_artifacts(database, &first, &second, 150, 151, false).await?;
    Ok(json!({
        "accepted": 2,
        "replay_exact": true,
        "channels": ["one_time_code", "magic_link"],
        "challenge_count": 2,
        "delivery_count": 2,
    }))
}

fn verification_issue_command(
    identifier_seed: u64,
    secret_seed: u64,
    abuse: AbuseDigestSet,
    purpose: VerificationPurpose,
    channel: VerificationChannel,
    payload_byte: u8,
    created_at_ms: i64,
) -> Result<VerificationIssueCommand> {
    Ok(VerificationIssueCommand {
        identifier_digests: candidates(digest(1, identifier_seed)?, Vec::new())?,
        secret_digest: digest(1, secret_seed)?,
        purpose,
        channel,
        initiated_by: None,
        initiator_grant: None,
        provisioning: None,
        max_attempts: 5,
        expires_at: time(NOW_MS + 10_000)?,
        sealed_delivery: SealedDeliveryEnvelope::new(vec![payload_byte; 32], time(created_at_ms)?)
            .map_err(|_| fixed_failure())?,
        abuse,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::VerificationIssue)?,
    })
}

fn same_issue_outcome(
    first: &std::result::Result<VerificationIssueAtomicOutcome, PortError>,
    second: &std::result::Result<VerificationIssueAtomicOutcome, PortError>,
) -> bool {
    match (first, second) {
        (
            Ok(VerificationIssueAtomicOutcome::Accepted),
            Ok(VerificationIssueAtomicOutcome::Accepted),
        ) => true,
        (
            Ok(VerificationIssueAtomicOutcome::RateLimited { retry_at: first }),
            Ok(VerificationIssueAtomicOutcome::RateLimited { retry_at: second }),
        ) => first == second,
        (
            Ok(VerificationIssueAtomicOutcome::Rejected(first)),
            Ok(VerificationIssueAtomicOutcome::Rejected(second)),
        ) => first == second,
        _ => false,
    }
}

async fn verification_issue_artifacts(
    database: &D1Database,
    first: &VerificationIssueCommand,
    second: &VerificationIssueCommand,
    first_identifier_seed: u64,
    second_identifier_seed: u64,
) -> Result<VerificationIssueArtifactRow> {
    database
        .prepare(
            "SELECT (SELECT COUNT(*) FROM auth_pending_verifications_v2 WHERE delivery_id IN (?1, ?2)) AS pending_count, (SELECT COUNT(*) FROM auth_verification_challenges_v2 WHERE identifier_key_version = 1 AND identifier_digest IN (?3, ?4)) AS challenge_count, (SELECT COUNT(*) FROM auth_delivery_outbox_v2 WHERE delivery_id IN (?1, ?2)) AS delivery_count",
        )
        .bind(&[
            JsValue::from_str(&first.sealed_delivery.id.to_string()),
            JsValue::from_str(&second.sealed_delivery.id.to_string()),
            JsValue::from_str(digest(1, first_identifier_seed)?.digest.expose_for_verification()),
            JsValue::from_str(digest(1, second_identifier_seed)?.digest.expose_for_verification()),
        ])?
        .first::<VerificationIssueArtifactRow>(None)
        .await?
        .ok_or_else(fixed_failure)
}

async fn cleanup_verification_issue_artifacts(
    database: &D1Database,
    first: &VerificationIssueCommand,
    second: &VerificationIssueCommand,
    first_identifier_seed: u64,
    second_identifier_seed: u64,
    cleanup_rates: bool,
) -> Result<()> {
    let bindings = [
        JsValue::from_str(&first.sealed_delivery.id.to_string()),
        JsValue::from_str(&second.sealed_delivery.id.to_string()),
        JsValue::from_str(
            digest(1, first_identifier_seed)?
                .digest
                .expose_for_verification(),
        ),
        JsValue::from_str(
            digest(1, second_identifier_seed)?
                .digest
                .expose_for_verification(),
        ),
    ];
    let mut statements = vec![
        database
            .prepare("DELETE FROM auth_delivery_outbox_v2 WHERE delivery_id IN (?1, ?2)")
            .bind(&bindings[..2])?,
        database
            .prepare("DELETE FROM auth_pending_verifications_v2 WHERE delivery_id IN (?1, ?2)")
            .bind(&bindings[..2])?,
        database
            .prepare("DELETE FROM auth_verification_challenges_v2 WHERE identifier_key_version = 1 AND identifier_digest IN (?1, ?2)")
            .bind(&bindings[2..])?,
    ];
    if cleanup_rates {
        statements.push(database.prepare("DELETE FROM auth_rate_limit_buckets_v2"));
    }
    let results = database.batch(statements).await?;
    if results.iter().any(|result| !result.success()) {
        return Err(fixed_failure());
    }
    Ok(())
}

async fn seed_existing_issue_buckets(database: &D1Database) -> Result<()> {
    let mut statements = Vec::with_capacity(4);
    for (dimension, key_version, digest_text) in [
        ("global", 0_i64, String::new()),
        (
            "source",
            1_i64,
            digest(1, 951)?.digest.expose_for_verification().to_string(),
        ),
        (
            "device",
            1_i64,
            digest(1, 952)?.digest.expose_for_verification().to_string(),
        ),
        (
            "identifier",
            1_i64,
            digest(1, 950)?.digest.expose_for_verification().to_string(),
        ),
    ] {
        statements.push(
            database
                .prepare(
                    "INSERT INTO auth_rate_limit_buckets_v2(action,dimension,key_version,digest,window_started_at_ms,attempt_count,blocked_until_ms,updated_at_ms,gc_at_ms,revision,last_operation_id) VALUES ('sign_in_issue',?1,?2,?3,?4,0,NULL,?4,?5,0,?6)",
                )
                .bind(&[
                    JsValue::from_str(dimension),
                    JsValue::from_f64(key_version as f64),
                    JsValue::from_str(&digest_text),
                    JsValue::from_f64((NOW_MS - 1_000) as f64),
                    JsValue::from_f64((NOW_MS + 60_000) as f64),
                    JsValue::from_str(FIXTURE_OPERATION_ID),
                ])?,
        );
    }
    let results = database.batch(statements).await?;
    if results.iter().any(|result| !result.success()) {
        return Err(fixed_failure());
    }
    Ok(())
}

async fn session_matrix(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let found_command = session_command(11, AuthAuditAction::SessionAuthenticate)?;
    let found = repository.authenticate_session(found_command.clone()).await;
    let found_retry = repository.authenticate_session(found_command.clone()).await;
    let mut mismatched_retry = found_command;
    mismatched_retry.token_digests = candidates(digest(1, 99)?, Vec::new())?;
    let mismatch = repository.authenticate_session(mismatched_retry).await;
    let missing = repository
        .authenticate_session(session_command(99, AuthAuditAction::SessionAuthenticate)?)
        .await;
    let expired = repository
        .authenticate_session(session_command(21, AuthAuditAction::SessionAuthenticate)?)
        .await;
    let revoked = repository
        .authenticate_session(session_command(31, AuthAuditAction::SessionAuthenticate)?)
        .await;
    let replay = repository
        .authenticate_session(session_command(41, AuthAuditAction::SessionAuthenticate)?)
        .await;
    if !matches!(found, Ok(SessionPresentation::Authenticated(_)))
        || !matches!(found_retry, Ok(SessionPresentation::Authenticated(_)))
        || !matches!(
            mismatch,
            Err(PortError::InvalidRequest(ref code))
                if code == "auth_repository_invalid_request"
        )
        || !matches!(missing, Ok(SessionPresentation::Unknown))
        || !matches!(expired, Ok(SessionPresentation::Expired(_)))
        || !matches!(revoked, Ok(SessionPresentation::Revoked(_)))
        || !matches!(replay, Ok(SessionPresentation::ReplayFamilyRevoked(_)))
    {
        return Err(fixed_failure());
    }

    let rotation_request = SessionRotationRequest {
        grant: SessionMutationGrant::from_repository(
            parse_session_mutation_grant(ROTATE_GRANT)?,
            parse_session(SESSION_FOUND)?,
            parse_user(USER_FOUND)?,
            0,
            digest(1, 11)?,
        ),
        next_token_digest: digest(2, 12)?,
        next_csrf_digest: digest(2, 13)?,
        now: time(NOW_MS)?,
        idle_expires_at: time(NOW_MS + 5_000)?,
        audit: audit(AuthAuditAction::SessionRotate)?,
    };
    let rotation = repository
        .rotate_auth_session(rotation_request.clone())
        .await;
    let rotation_retry = repository.rotate_auth_session(rotation_request).await;
    let logout = repository
        .revoke_auth_session(SessionRevokeCommand {
            grant: SessionMutationGrant::from_repository(
                parse_session_mutation_grant(SINGLE_LOGOUT_GRANT)?,
                parse_session(SESSION_SINGLE_LOGOUT)?,
                parse_user(USER_SINGLE_LOGOUT)?,
                0,
                digest(1, 16)?,
            ),
            reason: SessionRevocationReason::UserLogout,
            audit: audit(AuthAuditAction::Logout)?,
        })
        .await;
    let rotation_stable = matches!(
        (&rotation, &rotation_retry),
        (
            Ok(SessionRotationOutcome::Rotated(first)),
            Ok(SessionRotationOutcome::Rotated(second))
        ) if first == second
    );
    if !rotation_stable || !matches!(logout, Ok(true)) {
        return Err(fixed_failure());
    }
    Ok(json!({
        "found": "authenticated",
        "found_retry": "authenticated",
        "semantic_mismatch": "invalid_request",
        "not_found": "unknown",
        "expired": "expired_and_revoked",
        "revoked": "revoked",
        "replay": "family_revoked",
        "rotation": "rotated_and_reconstructed",
        "logout": "revoked",
    }))
}

async fn logout_all(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let grant = SessionMutationGrant::from_repository(
        parse_session_mutation_grant(LOGOUT_GRANT)?,
        parse_session(SESSION_LOGOUT)?,
        parse_user(USER_LOGOUT)?,
        0,
        digest(1, 51)?,
    );
    let outcome = repository
        .revoke_all_auth_sessions(SessionRevokeCommand {
            grant,
            reason: SessionRevocationReason::LogoutAll,
            audit: audit(AuthAuditAction::LogoutAll)?,
        })
        .await;
    match outcome {
        Ok(LogoutAllOutcome::Revoked {
            new_session_version: 1,
            revoked_sessions: 2,
        }) => Ok(json!({"session_version": 1, "revoked_sessions": 2})),
        _ => Err(fixed_failure()),
    }
}

async fn api_key_rotation_and_corrupt(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let tenant_id = frame_domain::TenantId::parse(TENANT_API).map_err(|_| fixed_failure())?;
    let command = ApiKeyAuthenticationCommand {
        key_digests: candidates(digest(2, 62)?, vec![digest(1, 61)?])?,
        tenant_id,
        required_scope: ApiKeyScope::VideosRead,
        abuse: abuse(610)?,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
    };
    let first = repository.authenticate_api_key(command.clone()).await;
    let retry = repository.authenticate_api_key(command).await;
    let corrupt = repository
        .authenticate_api_key(ApiKeyAuthenticationCommand {
            key_digests: candidates(digest(1, 71)?, Vec::new())?,
            tenant_id,
            required_scope: ApiKeyScope::VideosRead,
            abuse: abuse(630)?,
            rate_policy: rate_policy()?,
            audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
        })
        .await;
    Ok(json!({
        "fallback": api_key_outcome_name(&first, true),
        "retry": api_key_outcome_name(&retry, true),
        "active": "persisted",
        "corrupt": api_key_outcome_name(&corrupt, false),
    }))
}

fn api_key_outcome_name(
    outcome: &std::result::Result<ApiKeyAuthenticationOutcome, PortError>,
    migration_expected: bool,
) -> &'static str {
    match outcome {
        Ok(ApiKeyAuthenticationOutcome::Authenticated(_)) if migration_expected => "migrated",
        Ok(ApiKeyAuthenticationOutcome::Authenticated(_)) => "authenticated",
        Ok(ApiKeyAuthenticationOutcome::Rejected(_)) => "rejected",
        Ok(ApiKeyAuthenticationOutcome::RateLimited { .. }) => "rate_limited",
        Err(PortError::Adapter(code)) if code == "auth_repository_corrupt_result" => "fail_closed",
        Err(PortError::Conflict) => "conflict",
        Err(PortError::NotFound) => "not_found",
        Err(PortError::InvalidRequest(_)) => "invalid_request",
        Err(PortError::Unsupported(_)) => "unsupported",
        Err(PortError::Adapter(code)) if code == "auth_repository_unavailable" => "unavailable",
        Err(PortError::Adapter(code)) if code == "auth_repository_timeout" => "timeout",
        Err(PortError::Adapter(_)) => "adapter_error",
    }
}

async fn verification_replay(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let command = verification_command(
        candidates(digest(2, 82)?, vec![digest(1, 81)?])?,
        candidates(digest(2, 84)?, vec![digest(1, 83)?])?,
        abuse(810)?,
    )?;
    let first = repository.attempt_verification(command.clone()).await;
    let retry = repository.attempt_verification(command).await;
    let second = repository
        .attempt_verification(verification_command(
            candidates(digest(2, 82)?, Vec::new())?,
            candidates(digest(2, 84)?, Vec::new())?,
            abuse(820)?,
        )?)
        .await;
    let stable_grant = matches!(
        (&first, &retry),
        (
            Ok(VerificationAtomicOutcome::Verified { issuance_grant: first, .. }),
            Ok(VerificationAtomicOutcome::Verified { issuance_grant: second, .. })
        ) if first.id() == second.id()
    );
    if !stable_grant {
        return Err(fixed_failure());
    }
    Ok(json!({
        "first": verification_outcome_name(&first),
        "retry": verification_outcome_name(&retry),
        "stable_grant": true,
        "second": verification_outcome_name(&second),
    }))
}

fn verification_outcome_name(
    outcome: &std::result::Result<VerificationAtomicOutcome, PortError>,
) -> &'static str {
    match outcome {
        Ok(VerificationAtomicOutcome::Verified { .. }) => "verified",
        Ok(VerificationAtomicOutcome::ProvisioningAuthorized(_)) => "provisioning_authorized",
        Ok(VerificationAtomicOutcome::Linked { .. }) => "linked",
        Ok(VerificationAtomicOutcome::Rejected(AuthAuditReason::ReplayDetected)) => {
            "replay_detected"
        }
        Ok(VerificationAtomicOutcome::Rejected(_)) => "rejected",
        Ok(VerificationAtomicOutcome::RateLimited { .. }) => "rate_limited",
        Err(PortError::Conflict) => "conflict",
        Err(PortError::InvalidRequest(_)) => "invalid_request",
        Err(PortError::NotFound) => "not_found",
        Err(PortError::Unsupported(_)) => "unsupported",
        Err(PortError::Adapter(code)) if code == "auth_repository_corrupt_result" => "corrupt",
        Err(PortError::Adapter(code)) if code == "auth_repository_unavailable" => "unavailable",
        Err(PortError::Adapter(code)) if code == "auth_repository_timeout" => "timeout",
        Err(PortError::Adapter(_)) => "adapter_error",
    }
}

async fn delivery_lifecycle(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let now = time(NOW_MS)?;
    if repository
        .materialize_verification_deliveries(now, 10)
        .await
        .map_err(|_| fixed_failure())?
        != 1
    {
        return Err(fixed_failure());
    }
    let first = repository
        .claim_auth_delivery(now, duration(1_000)?)
        .await
        .map_err(|_| fixed_failure())?
        .ok_or_else(fixed_failure)?;
    let retry_at = time(NOW_MS + 100)?;
    if repository
        .retry_auth_delivery(first.clone(), now, retry_at)
        .await
        .map_err(|_| fixed_failure())?
        != AuthDeliveryRetryOutcome::Scheduled
    {
        return Err(fixed_failure());
    }
    if repository
        .retry_auth_delivery(first.clone(), now, retry_at)
        .await
        .map_err(|_| fixed_failure())?
        != AuthDeliveryRetryOutcome::Scheduled
    {
        return Err(fixed_failure());
    }
    let second = repository
        .claim_auth_delivery(retry_at, duration(1_000)?)
        .await
        .map_err(|_| fixed_failure())?
        .ok_or_else(fixed_failure)?;
    let second_attempt = second.attempt();
    let acknowledged = repository
        .acknowledge_auth_delivery(second.clone(), retry_at)
        .await
        .map_err(|_| fixed_failure())?;
    let acknowledged_retry = repository
        .acknowledge_auth_delivery(second, retry_at)
        .await
        .map_err(|_| fixed_failure())?;
    if second_attempt != 2
        || repository
            .acknowledge_auth_delivery(first, retry_at)
            .await
            .map_err(|_| fixed_failure())?
            != AuthDeliveryAcknowledgeOutcome::StaleLease
        || acknowledged != AuthDeliveryAcknowledgeOutcome::Acknowledged
        || acknowledged_retry != AuthDeliveryAcknowledgeOutcome::Acknowledged
        || repository
            .claim_auth_delivery(retry_at, duration(1_000)?)
            .await
            .map_err(|_| fixed_failure())?
            .is_some()
    {
        return Err(fixed_failure());
    }
    Ok(json!({
        "materialized": 1,
        "attempts": 2,
        "stale_lease": true,
        "retry_idempotent": true,
        "ack_idempotent": true,
    }))
}

async fn authority_fences(
    database: &D1Database,
    repository: &D1AuthStateRepository<'_>,
) -> Result<Value> {
    let membership_update = database
        .prepare(
            "UPDATE organization_members SET state = 'suspended', revision = revision + 1, updated_at_ms = ?3 WHERE organization_id = ?1 AND user_id = ?2",
        )
        .bind(&[
            JsValue::from_str(TENANT_FENCE),
            JsValue::from_str(USER_FENCE),
            JsValue::from_f64(NOW_MS as f64),
        ])?
        .run()
        .await?;
    if !membership_update.success() {
        return Err(fixed_failure());
    }
    let tenant_id = frame_domain::TenantId::parse(TENANT_FENCE).map_err(|_| fixed_failure())?;
    let membership_denied = repository
        .authenticate_api_key(ApiKeyAuthenticationCommand {
            key_digests: candidates(digest(2, 74)?, vec![digest(1, 73)?])?,
            tenant_id,
            required_scope: ApiKeyScope::VideosRead,
            abuse: abuse(710)?,
            rate_policy: rate_policy()?,
            audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
        })
        .await;

    let downgrade = database
        .prepare(
            "UPDATE organization_members SET state = 'active', role = 'viewer', revision = revision + 1, updated_at_ms = ?3 WHERE organization_id = ?1 AND user_id = ?2",
        )
        .bind(&[
            JsValue::from_str(TENANT_FENCE),
            JsValue::from_str(USER_FENCE),
            JsValue::from_f64((NOW_MS + 1) as f64),
        ])?
        .run()
        .await?;
    if !downgrade.success() {
        return Err(fixed_failure());
    }
    let grant = SessionMutationGrant::from_repository(
        parse_session_mutation_grant(FENCE_GRANT)?,
        parse_session(SESSION_FENCE)?,
        parse_user(USER_FENCE)?,
        0,
        digest(1, 72)?,
    );
    let downgraded_principal = PrincipalSnapshot {
        user_id: parse_user(USER_FENCE)?,
        identity_revision: 1,
        tenant_grants: vec![TenantGrant {
            tenant_id,
            role: OrganizationRole::Viewer,
        }],
    };
    let downgraded_issue = repository
        .issue_api_key(ApiKeyIssueCommand {
            principal: downgraded_principal,
            grant: grant.clone(),
            record: ManagedApiKeyRecord {
                id: ApiKeyId::parse("018f47a6-7b1c-7f55-8f39-8f8a8690f704")
                    .map_err(|_| fixed_failure())?,
                owner_id: parse_user(USER_FENCE)?,
                tenant_id,
                key_digest: digest(1, 75)?,
                scopes: vec![ApiKeyScope::VideosRead],
                created_at: time(NOW_MS)?,
                expires_at: Some(time(NOW_MS + 10_000)?),
                revoked_at: None,
            },
            audit: audit(AuthAuditAction::ApiKeyIssue)?,
        })
        .await;
    let downgraded_revoke = repository
        .revoke_api_key(ApiKeyRevokeCommand {
            grant,
            key_id: ApiKeyId::parse("018f47a6-7b1c-7f55-8f39-8f8a8690f702")
                .map_err(|_| fixed_failure())?,
            audit: audit(AuthAuditAction::ApiKeyRevoke)?,
        })
        .await;

    let membership_remove = database
        .prepare(
            "UPDATE organization_members SET state = 'removed', revision = revision + 1, updated_at_ms = ?3 WHERE organization_id = ?1 AND user_id = ?2",
        )
        .bind(&[
            JsValue::from_str(TENANT_FENCE),
            JsValue::from_str(USER_FENCE),
            JsValue::from_f64((NOW_MS + 1) as f64),
        ])?
        .run()
        .await?;
    if !membership_remove.success() {
        return Err(fixed_failure());
    }
    let removed_membership = repository
        .authenticate_api_key(ApiKeyAuthenticationCommand {
            key_digests: candidates(digest(2, 74)?, vec![digest(1, 73)?])?,
            tenant_id,
            required_scope: ApiKeyScope::VideosRead,
            abuse: abuse(715)?,
            rate_policy: rate_policy()?,
            audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
        })
        .await;

    let suspend_user = database
        .batch(vec![
            database
                .prepare(
                    "UPDATE organization_members SET state = 'active', revision = revision + 1, updated_at_ms = ?3 WHERE organization_id = ?1 AND user_id = ?2",
                )
                .bind(&[
                    JsValue::from_str(TENANT_FENCE),
                    JsValue::from_str(USER_FENCE),
                    JsValue::from_f64((NOW_MS + 1) as f64),
                ])?,
            database
                .prepare("UPDATE users SET status = 'suspended', updated_at_ms = ?2 WHERE id = ?1")
                .bind(&[
                    JsValue::from_str(USER_FENCE),
                    JsValue::from_f64((NOW_MS + 1) as f64),
                ])?,
        ])
        .await?;
    if suspend_user.iter().any(|result| !result.success()) {
        return Err(fixed_failure());
    }
    let suspended_session = repository
        .authenticate_session(session_command(72, AuthAuditAction::SessionAuthenticate)?)
        .await;
    let suspended_link = repository
        .attempt_verification(VerificationAttemptCommand {
            identifier_digests: candidates(digest(1, 76)?, Vec::new())?,
            secret_digests: candidates(digest(1, 77)?, Vec::new())?,
            purpose: VerificationPurpose::AccountLink,
            abuse: abuse(730)?,
            rate_policy: rate_policy()?,
            audit: audit(AuthAuditAction::AccountLink)?,
        })
        .await;

    let tombstone_org = database
        .batch(vec![
            database
                .prepare("UPDATE users SET status = 'active', updated_at_ms = ?2 WHERE id = ?1")
                .bind(&[
                    JsValue::from_str(USER_FENCE),
                    JsValue::from_f64((NOW_MS + 2) as f64),
                ])?,
            database
                .prepare(
                    "UPDATE organizations SET status = 'tombstoned', tombstoned_at_ms = ?2, revision = revision + 1, updated_at_ms = ?2 WHERE id = ?1",
                )
                .bind(&[
                    JsValue::from_str(TENANT_FENCE),
                    JsValue::from_f64((NOW_MS + 2) as f64),
                ])?,
        ])
        .await?;
    if tombstone_org.iter().any(|result| !result.success()) {
        return Err(fixed_failure());
    }
    let organization_denied = repository
        .authenticate_api_key(ApiKeyAuthenticationCommand {
            key_digests: candidates(digest(2, 74)?, vec![digest(1, 73)?])?,
            tenant_id,
            required_scope: ApiKeyScope::VideosRead,
            abuse: abuse(720)?,
            rate_policy: rate_policy()?,
            audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
        })
        .await;
    let key = database
        .prepare(
            "SELECT key_version, key_digest FROM auth_api_keys_v2 WHERE owner_id = ?1 AND tenant_id = ?2",
        )
        .bind(&[
            JsValue::from_str(USER_FENCE),
            JsValue::from_str(TENANT_FENCE),
        ])?
        .first::<FenceRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    let mutation = database
        .prepare(
            "SELECT (SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id = ?1) AS grant_count, (SELECT COUNT(*) FROM auth_api_keys_v2 WHERE id = '018f47a6-7b1c-7f55-8f39-8f8a8690f704') AS issued_key_count, (SELECT COUNT(*) FROM auth_api_keys_v2 WHERE id = '018f47a6-7b1c-7f55-8f39-8f8a8690f702' AND revoked_at_ms IS NOT NULL) AS existing_key_revoked, (SELECT COUNT(*) FROM auth_identifier_digests_v2 WHERE key_version = 1 AND digest = ?2) AS linked_identifier_count, (SELECT COUNT(*) FROM auth_verification_challenges_v2 WHERE id = '018f47a6-7b1c-7f55-8f39-8f8a8690f804') AS link_challenge_count",
        )
        .bind(&[
            JsValue::from_str(FENCE_GRANT),
            JsValue::from_str(digest(1, 76)?.digest.expose_for_verification()),
        ])?
        .first::<AuthorityMutationRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if !matches!(
        membership_denied,
        Ok(ApiKeyAuthenticationOutcome::Rejected(
            AuthAuditReason::InvalidCredential
        ))
    ) || !matches!(suspended_session, Ok(SessionPresentation::Revoked(_)))
        || !matches!(
            organization_denied,
            Ok(ApiKeyAuthenticationOutcome::Rejected(
                AuthAuditReason::InvalidCredential
            ))
        )
        || key.key_version != 1
        || key.key_digest != digest(1, 73)?.digest.expose_for_verification()
        || !matches!(downgraded_issue, Ok(ApiKeyIssueOutcome::Forbidden))
        || !matches!(downgraded_revoke, Ok(false))
        || !matches!(
            removed_membership,
            Ok(ApiKeyAuthenticationOutcome::Rejected(
                AuthAuditReason::InvalidCredential
            ))
        )
        || !matches!(
            suspended_link,
            Ok(VerificationAtomicOutcome::Rejected(
                AuthAuditReason::InvalidCredential
            ))
        )
        || mutation.grant_count != 1
        || mutation.issued_key_count != 0
        || mutation.existing_key_revoked != 0
        || mutation.linked_identifier_count != 0
        || mutation.link_challenge_count != 0
    {
        return Err(fixed_failure());
    }
    Ok(json!({
        "membership_suspension": "denied",
        "user_suspension": "session_revoked",
        "organization_tombstone": "denied",
        "key_unchanged": true,
        "membership_removal": "denied",
        "downgraded_issue": "forbidden_without_grant_spend",
        "downgraded_revoke": "forbidden_without_grant_spend",
        "suspended_link": "denied_without_identifier",
    }))
}

async fn delivery_lease_cleanup(
    database: &D1Database,
    repository: &D1AuthStateRepository<'_>,
) -> Result<Value> {
    let now = time(NOW_MS + 1_000)?;
    let claim = repository
        .claim_auth_delivery(now, duration(1_000)?)
        .await
        .map_err(|_| fixed_failure())?
        .ok_or_else(fixed_failure)?;
    if claim.delivery_id().to_string() != DELIVERY_EXHAUST || claim.attempt() != 12 {
        return Err(fixed_failure());
    }
    if repository
        .claim_auth_delivery(time(NOW_MS + 1_001)?, duration(1_000)?)
        .await
        .map_err(|_| fixed_failure())?
        .is_some()
    {
        return Err(fixed_failure());
    }
    let before = database
        .prepare(
            "SELECT (SELECT COUNT(*) FROM auth_delivery_outbox_v2 WHERE delivery_id = ?1 AND attempt = 12 AND lease_id IS NOT NULL) AS delivery_count, (SELECT COUNT(*) FROM auth_delivery_ack_tombstones_v2 WHERE delivery_id = ?1) AS tombstone_count",
        )
        .bind(&[JsValue::from_str(DELIVERY_EXHAUST)])?
        .first::<DeliveryCleanupRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    let retry_at = time(NOW_MS + 3_000)?;
    let exhausted = repository
        .retry_auth_delivery(claim.clone(), time(NOW_MS + 1_001)?, retry_at)
        .await
        .map_err(|_| fixed_failure())?;
    let exhausted_retry = repository
        .retry_auth_delivery(claim, time(NOW_MS + 1_001)?, retry_at)
        .await
        .map_err(|_| fixed_failure())?;
    let after = database
        .prepare(
            "SELECT (SELECT COUNT(*) FROM auth_delivery_outbox_v2 WHERE delivery_id = ?1) AS delivery_count, (SELECT COUNT(*) FROM auth_delivery_ack_tombstones_v2 WHERE delivery_id = ?1) AS tombstone_count",
        )
        .bind(&[JsValue::from_str(DELIVERY_EXHAUST)])?
        .first::<DeliveryCleanupRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if before.delivery_count != 1
        || before.tombstone_count != 0
        || exhausted != AuthDeliveryRetryOutcome::Exhausted
        || exhausted_retry != AuthDeliveryRetryOutcome::Exhausted
        || after.delivery_count != 0
        || after.tombstone_count != 1
    {
        return Err(fixed_failure());
    }
    Ok(json!({
        "active_attempt_twelve_lease_preserved": true,
        "exhausted": true,
        "retry_idempotent": true,
        "tombstone": true,
    }))
}

async fn claim_race(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let claim = repository
        .claim_auth_delivery(time(NOW_MS + 5_000)?, duration(1_000)?)
        .await
        .map_err(|_| fixed_failure())?;
    Ok(json!({"result": if claim.is_some() { "claimed" } else { "empty" }}))
}

async fn contention_retries(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let shared_verification_abuse = abuse(900)?;
    let first_verification = verification_command(
        candidates(digest(1, 85)?, Vec::new())?,
        candidates(digest(1, 87)?, Vec::new())?,
        shared_verification_abuse.clone(),
    )?;
    let second_verification = verification_command(
        candidates(digest(1, 86)?, Vec::new())?,
        candidates(digest(1, 88)?, Vec::new())?,
        shared_verification_abuse,
    )?;
    let (first_verified, second_verified) = futures::join!(
        repository.attempt_verification(first_verification),
        repository.attempt_verification(second_verification),
    );

    let tenant_id = frame_domain::TenantId::parse(TENANT_API).map_err(|_| fixed_failure())?;
    let shared_api_abuse = abuse(910)?;
    let first_api = ApiKeyAuthenticationCommand {
        key_digests: candidates(digest(2, 62)?, Vec::new())?,
        tenant_id,
        required_scope: ApiKeyScope::VideosRead,
        abuse: shared_api_abuse.clone(),
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
    };
    let second_api = ApiKeyAuthenticationCommand {
        audit: audit(AuthAuditAction::ApiKeyAuthenticate)?,
        ..first_api.clone()
    };
    let (first_authenticated, second_authenticated) = futures::join!(
        repository.authenticate_api_key(first_api),
        repository.authenticate_api_key(second_api),
    );
    let verification_name = |outcome: &Result<VerificationAtomicOutcome, PortError>| match outcome {
        Ok(VerificationAtomicOutcome::Verified { .. }) => "verified",
        Ok(VerificationAtomicOutcome::ProvisioningAuthorized(_)) => "provisioning_authorized",
        Ok(VerificationAtomicOutcome::Linked { .. }) => "linked",
        Ok(VerificationAtomicOutcome::Rejected(_)) => "rejected",
        Ok(VerificationAtomicOutcome::RateLimited { .. }) => "rate_limited",
        Err(PortError::Conflict) => "conflict",
        Err(PortError::InvalidRequest(_)) => "invalid",
        Err(PortError::Adapter(code)) if code == "auth_repository_unavailable" => "unavailable",
        Err(_) => "error",
    };
    let api_name = |outcome: &Result<ApiKeyAuthenticationOutcome, PortError>| match outcome {
        Ok(ApiKeyAuthenticationOutcome::Authenticated(_)) => "authenticated",
        Ok(ApiKeyAuthenticationOutcome::Rejected(_)) => "rejected",
        Ok(ApiKeyAuthenticationOutcome::RateLimited { .. }) => "rate_limited",
        Err(PortError::Conflict) => "conflict",
        Err(PortError::InvalidRequest(_)) => "invalid",
        Err(PortError::Adapter(code)) if code == "auth_repository_unavailable" => "unavailable",
        Err(_) => "error",
    };
    Ok(json!({
        "verification": [verification_name(&first_verified), verification_name(&second_verified)],
        "api_key": [api_name(&first_authenticated), api_name(&second_authenticated)],
    }))
}

async fn oauth_sign_in_link_replay(
    database: &D1Database,
    repository: &D1AuthStateRepository<'_>,
) -> Result<Value> {
    let cleanup = database
        .prepare("DELETE FROM auth_rate_limit_buckets_v2 WHERE action = 'oauth_begin'")
        .run()
        .await?;
    if !cleanup.success() {
        return Err(fixed_failure());
    }

    let sign_in_flow = oauth_flow(
        OAUTH_SIGN_IN_FLOW,
        OAuthProvider::Google,
        OAuthFlowPurpose::SignIn,
        None,
        1_203,
    )?;
    let sign_in_begin = OAuthBeginCommand {
        flow: sign_in_flow.clone(),
        initiator: None,
        abuse: abuse(1_220)?,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::OAuthBegin)?,
    };
    let sign_in_started = repository.begin_oauth(sign_in_begin.clone()).await;
    let sign_in_started_retry = repository.begin_oauth(sign_in_begin).await;
    if !matches!(sign_in_started, Ok(OAuthBeginOutcome::Started))
        || !matches!(sign_in_started_retry, Ok(OAuthBeginOutcome::Started))
    {
        return Err(fixed_failure());
    }

    let sign_in_preflight = OAuthPreflightCommand {
        provider: OAuthProvider::Google,
        state_digests: candidates(sign_in_flow.state_digest.clone(), Vec::new())?,
        pkce_digests: candidates(sign_in_flow.pkce_digest.clone(), Vec::new())?,
        redirect_digests: candidates(sign_in_flow.redirect_digest.clone(), Vec::new())?,
        audience_digests: candidates(sign_in_flow.audience_digest.clone(), Vec::new())?,
        abuse: abuse(1_240)?,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::OAuthExchangePreflight)?,
    };
    let sign_in_ready = repository
        .preflight_oauth_exchange(sign_in_preflight.clone())
        .await;
    let sign_in_ready_retry = repository.preflight_oauth_exchange(sign_in_preflight).await;
    let sign_in_reservation = match (&sign_in_ready, &sign_in_ready_retry) {
        (Ok(OAuthPreflightOutcome::Ready(first)), Ok(OAuthPreflightOutcome::Ready(second)))
            if first == second =>
        {
            first.clone()
        }
        _ => return Err(fixed_failure()),
    };
    let sign_in_result = OAuthProviderResult::Verified(ExternalIdentityAssertion {
        provider: OAuthProvider::Google,
        subject_digests: candidates(digest(2, 1_208)?, vec![digest(1, 1_207)?])?,
        verified_identifier_digests: Some(candidates(digest(1, 1_201)?, Vec::new())?),
    });
    let sign_in_finalize = OAuthFinalizeCommand {
        reservation: sign_in_reservation.clone(),
        provider_result: sign_in_result.clone(),
        audit: audit(AuthAuditAction::OAuthExchange)?,
    };
    let sign_in_verified = repository
        .finalize_oauth_exchange(sign_in_finalize.clone())
        .await;
    let sign_in_verified_retry = repository.finalize_oauth_exchange(sign_in_finalize).await;
    let stable_sign_in = matches!(
        (&sign_in_verified, &sign_in_verified_retry),
        (
            Ok(OAuthExchangeOutcome::Verified {
                principal: first_principal,
                issuance_grant: first_grant,
            }),
            Ok(OAuthExchangeOutcome::Verified {
                principal: second_principal,
                issuance_grant: second_grant,
            }),
        ) if first_principal.user_id == parse_user(USER_OAUTH)?
            && first_principal == second_principal
            && first_grant == second_grant
    );
    if !stable_sign_in {
        return Err(fixed_failure());
    }
    let consumed_replay = repository
        .finalize_oauth_exchange(OAuthFinalizeCommand {
            reservation: sign_in_reservation,
            provider_result: sign_in_result,
            audit: audit(AuthAuditAction::OAuthExchange)?,
        })
        .await;
    if !matches!(
        consumed_replay,
        Ok(OAuthExchangeOutcome::Rejected(
            AuthAuditReason::ReplayDetected
        ))
    ) {
        return Err(fixed_failure());
    }

    let binding = SessionContinuationBinding {
        session_id: parse_session(SESSION_OAUTH)?,
        user_id: parse_user(USER_OAUTH)?,
        generation: 0,
    };
    let link_flow = oauth_flow(
        OAUTH_LINK_FLOW,
        OAuthProvider::Github,
        OAuthFlowPurpose::AccountLink,
        Some(binding),
        1_210,
    )?;
    let link_begin = OAuthBeginCommand {
        flow: link_flow.clone(),
        initiator: Some(SessionMutationGrant::from_repository(
            parse_session_mutation_grant(OAUTH_GRANT)?,
            binding.session_id,
            binding.user_id,
            binding.generation,
            digest(1, 1_202)?,
        )),
        abuse: abuse(1_230)?,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::OAuthBegin)?,
    };
    let link_started = repository.begin_oauth(link_begin.clone()).await;
    let link_started_retry = repository.begin_oauth(link_begin).await;
    if !matches!(link_started, Ok(OAuthBeginOutcome::Started))
        || !matches!(link_started_retry, Ok(OAuthBeginOutcome::Started))
    {
        return Err(fixed_failure());
    }
    let link_preflight = OAuthPreflightCommand {
        provider: OAuthProvider::Github,
        state_digests: candidates(link_flow.state_digest.clone(), Vec::new())?,
        pkce_digests: candidates(link_flow.pkce_digest.clone(), Vec::new())?,
        redirect_digests: candidates(link_flow.redirect_digest.clone(), Vec::new())?,
        audience_digests: candidates(link_flow.audience_digest.clone(), Vec::new())?,
        abuse: abuse(1_250)?,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::OAuthExchangePreflight)?,
    };
    let link_ready = repository
        .preflight_oauth_exchange(link_preflight.clone())
        .await;
    let link_ready_retry = repository.preflight_oauth_exchange(link_preflight).await;
    let link_reservation = match (&link_ready, &link_ready_retry) {
        (Ok(OAuthPreflightOutcome::Ready(first)), Ok(OAuthPreflightOutcome::Ready(second)))
            if first == second =>
        {
            first.clone()
        }
        _ => return Err(fixed_failure()),
    };
    let link_finalize = OAuthFinalizeCommand {
        reservation: link_reservation,
        provider_result: OAuthProviderResult::Verified(ExternalIdentityAssertion {
            provider: OAuthProvider::Github,
            subject_digests: candidates(digest(1, 1_214)?, Vec::new())?,
            verified_identifier_digests: None,
        }),
        audit: audit(AuthAuditAction::OAuthExchange)?,
    };
    let linked = repository
        .finalize_oauth_exchange(link_finalize.clone())
        .await;
    let linked_retry = repository.finalize_oauth_exchange(link_finalize).await;
    if !matches!(
        (&linked, &linked_retry),
        (
            Ok(OAuthExchangeOutcome::Linked { user_id: first }),
            Ok(OAuthExchangeOutcome::Linked { user_id: second }),
        ) if first == second && *first == parse_user(USER_OAUTH)?
    ) {
        return Err(fixed_failure());
    }

    let artifacts = database
        .prepare(
            "SELECT (SELECT COUNT(*) FROM auth_oauth_flows_v2 WHERE id IN (?1, ?2) AND consumed_at_ms IS NOT NULL) AS consumed_flows, \
             (SELECT COUNT(*) FROM auth_oauth_reservations_v2 WHERE flow_id IN (?1, ?2) AND consumed_at_ms IS NOT NULL) AS consumed_reservations, \
             (SELECT COUNT(*) FROM auth_external_accounts_v2 WHERE user_id = ?3) AS external_accounts, \
             (SELECT COUNT(*) FROM auth_oauth_operations_v2) AS oauth_operations",
        )
        .bind(&[
            JsValue::from_str(OAUTH_SIGN_IN_FLOW),
            JsValue::from_str(OAUTH_LINK_FLOW),
            JsValue::from_str(USER_OAUTH),
        ])?
        .first::<OAuthArtifactRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    if artifacts.consumed_flows != 2
        || artifacts.consumed_reservations != 2
        || artifacts.external_accounts != 2
        || artifacts.oauth_operations != 7
    {
        return Err(fixed_failure());
    }
    Ok(json!({
        "sign_in": "verified",
        "link": "linked",
        "exact_replay": true,
        "consumed_replay": "replay_detected",
        "external_accounts": 2,
    }))
}

fn oauth_flow(
    id: &str,
    provider: OAuthProvider,
    purpose: OAuthFlowPurpose,
    initiator: Option<SessionContinuationBinding>,
    seed: u64,
) -> Result<OAuthFlowRecord> {
    Ok(OAuthFlowRecord {
        id: OAuthFlowId::parse(id).map_err(|_| fixed_failure())?,
        provider,
        purpose,
        initiator,
        state_digest: digest(1, seed)?,
        pkce_digest: digest(1, seed + 1)?,
        redirect_digest: digest(1, seed + 2)?,
        audience_digest: digest(1, seed + 3)?,
        created_at: time(NOW_MS)?,
        expires_at: time(NOW_MS + 10_000)?,
        consumed_at: None,
        revoked: false,
    })
}

async fn provision_race(repository: &D1AuthStateRepository<'_>) -> Result<Value> {
    let grant = IdentityProvisioningGrant::from_repository(
        IdentityProvisioningGrantId::parse(PROVISION_GRANT).map_err(|_| fixed_failure())?,
        parse_user(USER_PROVISION)?,
        1,
        digest(1, 91)?,
        time(NOW_MS + 10_000)?,
    );
    let outcome = repository
        .provision_identity(IdentityProvisionCommand {
            grant,
            destination: DeliveryDestinationRef::parse("closed-conformance-destination")
                .map_err(|_| fixed_failure())?,
            audit: audit(AuthAuditAction::IdentityProvision)?,
        })
        .await
        .map_err(|_| fixed_failure())?;
    match outcome {
        IdentityProvisionOutcome::Created => Ok(json!({"result": "created"})),
        IdentityProvisionOutcome::Rejected(AuthAuditReason::ReplayDetected) => {
            Ok(json!({"result": "replay_detected"}))
        }
        _ => Err(fixed_failure()),
    }
}

async fn atomic_rollback(
    database: &D1Database,
    repository: &D1AuthStateRepository<'_>,
) -> Result<Value> {
    let user_id = parse_user(USER_ROLLBACK)?;
    let principal = PrincipalSnapshot {
        user_id,
        identity_revision: 1,
        tenant_grants: Vec::new(),
    };
    let grant = PrincipalIssuanceGrant::from_repository(
        frame_domain::PrincipalIssuanceGrantId::parse(ROLLBACK_GRANT)
            .map_err(|_| fixed_failure())?,
        user_id,
        1,
        time(NOW_MS + 10_000)?,
    );
    let session = AuthSessionRecord {
        id: parse_session(SESSION_ROLLBACK)?,
        family_id: SessionFamilyId::parse(FAMILY_ROLLBACK).map_err(|_| fixed_failure())?,
        user_id,
        client_kind: AuthClientKind::Api,
        token_digest: digest(1, 101)?,
        csrf_digest: None,
        browser_origin: None,
        issued_at: time(NOW_MS)?,
        rotated_at: time(NOW_MS)?,
        idle_expires_at: time(NOW_MS + 5_000)?,
        absolute_expires_at: time(NOW_MS + 10_000)?,
        session_version: 0,
        generation: 0,
        state: AuthSessionState::Active,
        revoked_at: None,
        revocation_reason: None,
    };
    let result = repository
        .issue_auth_session(SessionIssueCommand {
            principal,
            authority: SessionIssueAuthority::Verified(grant),
            session,
            audit: audit(AuthAuditAction::SessionIssue)?,
        })
        .await;
    let row = database
        .prepare(
            "SELECT (SELECT COUNT(*) FROM auth_principal_issuance_grants_v2 WHERE id = ?1) AS grant_count, \
             (SELECT COUNT(*) FROM auth_sessions_v2 WHERE id = ?2) AS session_count",
        )
        .bind(&[
            JsValue::from_str(ROLLBACK_GRANT),
            JsValue::from_str(SESSION_ROLLBACK),
        ])?
        .first::<RollbackRow>(None)
        .await?
        .ok_or_else(fixed_failure)?;
    Ok(json!({
        "result": session_issue_result_name(&result),
        "grant_restored": row.grant_count == 1,
        "session_absent": row.session_count == 0,
    }))
}

fn session_issue_result_name(
    result: &std::result::Result<frame_ports::SessionIssueOutcome, PortError>,
) -> &'static str {
    match result {
        Ok(frame_ports::SessionIssueOutcome::Issued) => "issued",
        Ok(frame_ports::SessionIssueOutcome::Denied(_)) => "denied",
        Err(PortError::Conflict) => "conflict",
        Err(PortError::InvalidRequest(_)) => "invalid_request",
        Err(PortError::NotFound) => "not_found",
        Err(PortError::Unsupported(_)) => "unsupported",
        Err(PortError::Adapter(code)) if code == "auth_repository_unavailable" => "unavailable",
        Err(PortError::Adapter(code)) if code == "auth_repository_timeout" => "timeout",
        Err(PortError::Adapter(code)) if code == "auth_repository_corrupt_result" => "corrupt",
        Err(PortError::Adapter(_)) => "adapter_error",
    }
}

fn session_command(seed: u64, action: AuthAuditAction) -> Result<SessionAuthenticationCommand> {
    Ok(SessionAuthenticationCommand {
        token_digests: candidates(digest(1, seed)?, Vec::new())?,
        browser_boundary: None,
        audit: audit(action)?,
    })
}

fn verification_command(
    identifier_digests: SecretDigestCandidates,
    secret_digests: SecretDigestCandidates,
    abuse: AbuseDigestSet,
) -> Result<VerificationAttemptCommand> {
    Ok(VerificationAttemptCommand {
        identifier_digests,
        secret_digests,
        purpose: VerificationPurpose::SignIn,
        abuse,
        rate_policy: rate_policy()?,
        audit: audit(AuthAuditAction::VerificationConsume)?,
    })
}

fn audit(action: AuthAuditAction) -> Result<DecisionAudit> {
    Ok(DecisionAudit {
        correlation_id: CorrelationId::new(),
        action,
        occurred_at: time(NOW_MS)?,
    })
}

fn abuse(seed: u64) -> Result<AbuseDigestSet> {
    Ok(AbuseDigestSet {
        identifier: candidates(digest(1, seed)?, Vec::new())?,
        source: candidates(digest(1, seed + 1)?, Vec::new())?,
        device: candidates(digest(1, seed + 2)?, Vec::new())?,
    })
}

fn rate_policy() -> Result<MultiRateLimitPolicy> {
    let policy = RateLimitPolicy::new(100, duration(60_000)?, duration(60_000)?)
        .map_err(|_| fixed_failure())?;
    Ok(MultiRateLimitPolicy {
        identifier: policy,
        source: policy,
        device: policy,
        global: policy,
    })
}

fn candidates(
    active: VersionedSecretDigest,
    fallback: Vec<VersionedSecretDigest>,
) -> Result<SecretDigestCandidates> {
    SecretDigestCandidates::new(active, fallback).map_err(|_| fixed_failure())
}

fn digest(version: u16, seed: u64) -> Result<VersionedSecretDigest> {
    Ok(VersionedSecretDigest::new(
        HashKeyVersion::new(version).map_err(|_| fixed_failure())?,
        SecretDigest::parse_sha256(format!("{seed:064x}")).map_err(|_| fixed_failure())?,
    ))
}

fn time(value: i64) -> Result<TimestampMillis> {
    TimestampMillis::new(value).map_err(|_| fixed_failure())
}

fn duration(value: u64) -> Result<DurationMillis> {
    DurationMillis::new(value).map_err(|_| fixed_failure())
}

fn parse_user(value: &str) -> Result<UserId> {
    UserId::parse(value).map_err(|_| fixed_failure())
}

fn parse_session(value: &str) -> Result<SessionId> {
    SessionId::parse(value).map_err(|_| fixed_failure())
}

fn parse_session_mutation_grant(value: &str) -> Result<SessionMutationGrantId> {
    SessionMutationGrantId::parse(value).map_err(|_| fixed_failure())
}

fn valid_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn fixed_failure() -> worker::Error {
    worker::Error::RustError("auth repository conformance failed".into())
}

fn fixed_response(status: u16, outcome: &'static str, details: Option<Value>) -> Result<Response> {
    let mut body = json!({"outcome": outcome});
    if let (Some(object), Some(details)) = (body.as_object_mut(), details) {
        object.insert("details".into(), details);
    }
    Response::from_json(&body).map(|response| response.with_status(status))
}
