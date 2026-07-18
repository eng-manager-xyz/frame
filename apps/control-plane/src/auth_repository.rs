//! Capability-safe D1 persistence for the provider-free authentication core.
//!
//! All credentials and identifiers crossing this boundary are already keyed
//! digests. The adapter has no column or binding for a raw session token, CSRF
//! token, OTP, API key, delivery destination, or OAuth code. Every supported
//! authentication decision writes its privacy-safe audit event in the same D1
//! batch as the state transition and uses an immediate `changes()` assertion
//! to roll the whole transaction back on a stale optimistic predicate.

use std::{collections::BTreeSet, future::Future, time::Duration};

use async_trait::async_trait;
use frame_domain::{
    AbuseBucketId, AbuseDimension, ApiKeyId, ApiKeyScope, AuthAbuseAction, AuthAuditAction,
    AuthAuditEventId, AuthAuditOutcome, AuthAuditReason, AuthClientKind, AuthDeliveryId,
    AuthDeliveryLeaseId, AuthRateLimitBucket, AuthSessionDecision, AuthSessionRecord,
    AuthSessionState, DurationMillis, ExactBrowserOrigin, FetchSite, HashKeyVersion,
    IdentityProvisioningGrantId, ManagedApiKeyRecord, NewVerificationChallenge,
    OAuthExchangeReservationId, OAuthFlowId, OAuthFlowPurpose, OAuthProvider, OrganizationRole,
    PrincipalIssuanceGrantId, PrincipalSnapshot, SealedDeliveryEnvelope, SecretDigest,
    SecretDigestCandidates, SessionContinuationBinding, SessionFamilyId, SessionId,
    SessionMutationGrantId, SessionRevocationReason, TenantGrant, TenantId, TimestampMillis,
    UserId, VerificationChallenge, VerificationChannel, VerificationDecision, VerificationId,
    VerificationPurpose, VerificationState, VersionedSecretDigest,
};
use frame_ports::{
    AbuseDigestSet, ApiKeyAuthenticationCommand, ApiKeyAuthenticationOutcome, ApiKeyIssueCommand,
    ApiKeyIssueOutcome, ApiKeyRevokeCommand, AuthDeliveryAcknowledgeOutcome, AuthDeliveryClaim,
    AuthDeliveryRetryOutcome, AuthStateRepository, AuthenticatedSessionPresentation, DecisionAudit,
    IdentityProvisionCommand, IdentityProvisionOutcome, IdentityProvisioningGrant,
    LogoutAllOutcome, MAX_AUTH_DELIVERY_ATTEMPTS, MAX_AUTH_DELIVERY_LEASE_MILLIS,
    MAX_PENDING_OAUTH_FLOWS, MAX_RATE_LIMIT_BUCKETS, OAuthBeginCommand, OAuthBeginOutcome,
    OAuthExchangeOutcome, OAuthFinalizeCommand, OAuthPreflightCommand, OAuthPreflightOutcome,
    OAuthProviderResult, PortError, PrincipalIssuanceGrant, SessionAuditContext,
    SessionAuthenticationCommand, SessionIssueAuthority, SessionIssueCommand, SessionIssueOutcome,
    SessionMutationGrant, SessionPresentation, SessionRevokeCommand, SessionRotationOutcome,
    SessionRotationRequest, VerificationAtomicOutcome, VerificationAttemptCommand,
    VerificationIssueAtomicOutcome, VerificationIssueCommand,
};
use futures::{future::Either, future::select, pin_mut};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Delay, send::IntoSendFuture};

const DEFAULT_AUTH_QUERY_TIMEOUT_MS: u64 = 5_000;
const MAX_PRINCIPAL_GRANTS: usize = 1_000;
const MAX_CANDIDATE_JSON_BYTES: usize = 1_024;
const AUTH_CAS_CONFLICT_SENTINEL: &str = "frame_auth_cas_conflict_v1";

fn exact_d1_trigger_error(message: &str) -> bool {
    // Wrangler 4.111.0's D1 error wrapper is deliberately part of this
    // allowlist. Envelope drift must fail closed to Unavailable.
    let constraint = format!(
        "{AUTH_CAS_CONFLICT_SENTINEL}: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)"
    );
    let lines = message.lines().collect::<Vec<_>>();
    if lines.len() < 6
        || lines[0] != "D1: D1Error {"
        || lines[1] != format!("    cause: JsValue(Error: {constraint}")
        || lines[2] != format!("    Error: {constraint}")
        || lines.last() != Some(&"}")
    {
        return false;
    }
    let stack = &lines[3..lines.len() - 1];
    stack.iter().all(|line| line.starts_with("        at "))
        && stack.last().is_some_and(|line| line.ends_with("),"))
}

const PRINCIPAL_BY_USER_SQL: &str = include_str!("../queries/auth/principal_by_user.sql");
const IDENTIFIER_OWNER_SQL: &str = include_str!("../queries/auth/identifier_owner.sql");
const SESSION_BY_CREDENTIALS_SQL: &str = include_str!("../queries/auth/session_by_credentials.sql");
const SESSION_BY_ID_SQL: &str = include_str!("../queries/auth/session_by_id.sql");
const MUTATION_GRANT_SQL: &str = include_str!("../queries/auth/mutation_grant.sql");
const ISSUANCE_GRANT_SQL: &str = include_str!("../queries/auth/issuance_grant.sql");
const PROVISIONING_GRANT_SQL: &str = include_str!("../queries/auth/provisioning_grant.sql");
const PENDING_READY_SQL: &str = include_str!("../queries/auth/pending_ready.sql");
const PENDING_BY_ID_SQL: &str = include_str!("../queries/auth/pending_by_id.sql");
const VERIFICATION_CANDIDATE_SQL: &str = include_str!("../queries/auth/verification_candidate.sql");
const API_KEY_BY_CREDENTIALS_SQL: &str = include_str!("../queries/auth/api_key_by_credentials.sql");
const API_KEY_BY_ID_SQL: &str = include_str!("../queries/auth/api_key_by_id.sql");
const RATE_BUCKETS_SQL: &str = include_str!("../queries/auth/rate_buckets.sql");
const RATE_BUCKET_COUNT_SQL: &str = include_str!("../queries/auth/rate_bucket_count.sql");
const DELIVERY_NEXT_SQL: &str = include_str!("../queries/auth/delivery_next.sql");
const DELIVERY_BY_ID_SQL: &str = include_str!("../queries/auth/delivery_by_id.sql");
const DELIVERY_CLAIM_BY_OPERATION_SQL: &str =
    include_str!("../queries/auth/delivery_claim_by_operation.sql");
const DELIVERY_RETRY_SCHEDULED_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/delivery_retry_scheduled_postcondition.sql");
const DELIVERY_MATERIALIZE_LIMIT_PER_CLAIM: u32 = 1;
const DELIVERY_MATERIALIZE_CONFLICT_ATTEMPTS: usize = 1;
const DELIVERY_CLAIM_CONFLICT_ATTEMPTS: usize = 3;

const AUDIT_INSERT_SQL: &str = include_str!("../queries/auth/audit_insert.sql");
const ASSERT_CHANGES_SQL: &str = include_str!("../queries/auth/assert_changes.sql");
const ASSERTIONS_CLEANUP_SQL: &str = include_str!("../queries/auth/assertions_cleanup.sql");
const RATE_BUCKET_UPSERT_SQL: &str = include_str!("../queries/auth/rate_bucket_upsert.sql");
const RATE_BUCKET_DELETE_SQL: &str = include_str!("../queries/auth/rate_bucket_delete.sql");
const RATE_BUCKET_GC_SQL: &str = include_str!("../queries/auth/rate_bucket_gc.sql");
const USER_INSERT_SQL: &str = include_str!("../queries/auth/user_insert.sql");
const IDENTITY_INSERT_SQL: &str = include_str!("../queries/auth/identity_insert.sql");
const IDENTIFIER_INSERT_SQL: &str = include_str!("../queries/auth/identifier_insert.sql");
const PROVISIONING_GRANT_DELETE_SQL: &str =
    include_str!("../queries/auth/provisioning_grant_delete.sql");
const SESSION_INSERT_SQL: &str = include_str!("../queries/auth/session_insert.sql");
const CREDENTIAL_INSERT_SQL: &str = include_str!("../queries/auth/credential_insert.sql");
const ISSUANCE_GRANT_DELETE_SQL: &str = include_str!("../queries/auth/issuance_grant_delete.sql");
const MUTATION_GRANT_DELETE_SQL: &str = include_str!("../queries/auth/mutation_grant_delete.sql");
const MUTATION_GRANTS_DELETE_SESSION_SQL: &str =
    include_str!("../queries/auth/mutation_grants_delete_session.sql");
const MUTATION_GRANT_INSERT_SQL: &str = include_str!("../queries/auth/mutation_grant_insert.sql");
const SESSION_DIGEST_MIGRATE_SQL: &str = include_str!("../queries/auth/session_digest_migrate.sql");
const CREDENTIAL_DIGEST_MIGRATE_SQL: &str =
    include_str!("../queries/auth/credential_digest_migrate.sql");
const SESSION_CSRF_UPDATE_SQL: &str = include_str!("../queries/auth/session_csrf_update.sql");
const SESSION_ROTATE_SQL: &str = include_str!("../queries/auth/session_rotate.sql");
const CREDENTIAL_MARK_ROTATED_SQL: &str =
    include_str!("../queries/auth/credential_mark_rotated.sql");
const SESSION_REVOKE_ONE_SQL: &str = include_str!("../queries/auth/session_revoke_one.sql");
const SESSION_REVOKE_FAMILY_SQL: &str = include_str!("../queries/auth/session_revoke_family.sql");
const SESSION_REVOKE_USER_SQL: &str = include_str!("../queries/auth/session_revoke_user.sql");
const CREDENTIALS_REVOKE_SESSION_SQL: &str =
    include_str!("../queries/auth/credentials_revoke_session.sql");
const CREDENTIALS_REVOKE_FAMILY_SQL: &str =
    include_str!("../queries/auth/credentials_revoke_family.sql");
const CREDENTIALS_REVOKE_USER_SQL: &str =
    include_str!("../queries/auth/credentials_revoke_user.sql");
const IDENTITY_SESSION_VERSION_INCREMENT_SQL: &str =
    include_str!("../queries/auth/identity_session_version_increment.sql");
const SESSION_CONTINUATIONS_DELETE_SQL: &str =
    include_str!("../queries/auth/session_continuations_delete.sql");
const CHALLENGE_CONTINUATIONS_DELETE_SQL: &str =
    include_str!("../queries/auth/challenge_continuations_delete.sql");
const DELIVERY_CONTINUATIONS_DELETE_SQL: &str =
    include_str!("../queries/auth/delivery_continuations_delete.sql");
const SESSION_FAMILY_COUNTS_SQL: &str = include_str!("../queries/auth/session_family_counts.sql");
const SESSION_USER_COUNTS_SQL: &str = include_str!("../queries/auth/session_user_counts.sql");
const PENDING_REVOKE_MATCHING_SQL: &str =
    include_str!("../queries/auth/pending_revoke_matching.sql");
const CHALLENGES_REVOKE_MATCHING_SQL: &str =
    include_str!("../queries/auth/challenges_revoke_matching.sql");
const PENDING_INSERT_SQL: &str = include_str!("../queries/auth/pending_insert.sql");
const PENDING_DELETE_SQL: &str = include_str!("../queries/auth/pending_delete.sql");
const CHALLENGE_INSERT_SQL: &str = include_str!("../queries/auth/challenge_insert.sql");
const CHALLENGE_UPDATE_SQL: &str = include_str!("../queries/auth/challenge_update.sql");
const CHALLENGE_DELETE_SQL: &str = include_str!("../queries/auth/challenge_delete.sql");
const CHALLENGE_EXPIRED_CLEANUP_SQL: &str =
    include_str!("../queries/auth/challenge_expired_cleanup.sql");
const DELIVERY_INSERT_SQL: &str = include_str!("../queries/auth/delivery_insert.sql");
const DELIVERY_CLEANUP_SQL: &str = include_str!("../queries/auth/delivery_cleanup.sql");
const DELIVERY_CLAIM_SQL: &str = include_str!("../queries/auth/delivery_claim.sql");
const DELIVERY_ACKNOWLEDGE_SQL: &str = include_str!("../queries/auth/delivery_acknowledge.sql");
const DELIVERY_RETRY_SQL: &str = include_str!("../queries/auth/delivery_retry.sql");
const PROVISIONING_GRANT_INSERT_SQL: &str =
    include_str!("../queries/auth/provisioning_grant_insert.sql");
const ISSUANCE_GRANT_INSERT_SQL: &str = include_str!("../queries/auth/issuance_grant_insert.sql");
const API_KEY_INSERT_SQL: &str = include_str!("../queries/auth/api_key_insert.sql");
const API_KEY_DIGEST_MIGRATE_SQL: &str = include_str!("../queries/auth/api_key_digest_migrate.sql");
const API_KEY_REVOKE_SQL: &str = include_str!("../queries/auth/api_key_revoke.sql");
const API_KEYS_ABSENT_ASSERT_SQL: &str = include_str!("../queries/auth/api_keys_absent_assert.sql");
const API_KEY_CURRENT_ASSERT_SQL: &str = include_str!("../queries/auth/api_key_current_assert.sql");
const CREDENTIALS_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/credentials_absent_assert.sql");
const SESSION_CURRENT_ASSERT_SQL: &str = include_str!("../queries/auth/session_current_assert.sql");
const SESSION_CREDENTIAL_COUNT_SQL: &str =
    include_str!("../queries/auth/session_credential_count.sql");
const MUTATION_GRANTS_DELETE_FAMILY_SQL: &str =
    include_str!("../queries/auth/mutation_grants_delete_family.sql");
const PENDING_CONTINUATIONS_DELETE_FAMILY_SQL: &str =
    include_str!("../queries/auth/pending_continuations_delete_family.sql");
const CHALLENGE_CONTINUATIONS_DELETE_FAMILY_SQL: &str =
    include_str!("../queries/auth/challenge_continuations_delete_family.sql");
const DELIVERY_CONTINUATIONS_DELETE_FAMILY_SQL: &str =
    include_str!("../queries/auth/delivery_continuations_delete_family.sql");
const MUTATION_GRANTS_DELETE_USER_SQL: &str =
    include_str!("../queries/auth/mutation_grants_delete_user.sql");
const PENDING_CONTINUATIONS_DELETE_USER_SQL: &str =
    include_str!("../queries/auth/pending_continuations_delete_user.sql");
const CHALLENGE_CONTINUATIONS_DELETE_USER_SQL: &str =
    include_str!("../queries/auth/challenge_continuations_delete_user.sql");
const DELIVERY_CONTINUATIONS_DELETE_USER_SQL: &str =
    include_str!("../queries/auth/delivery_continuations_delete_user.sql");
const IDENTITY_EXISTS_SQL: &str = include_str!("../queries/auth/identity_exists.sql");
const IDENTITY_ABSENT_ASSERT_SQL: &str = include_str!("../queries/auth/identity_absent_assert.sql");
const IDENTITY_CURRENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/identity_current_assert.sql");
const IDENTIFIER_CANDIDATES_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/identifier_candidates_absent_assert.sql");
const SESSION_CONTINUATION_ASSERT_SQL: &str =
    include_str!("../queries/auth/session_continuation_assert.sql");
const TENANT_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/auth/tenant_authority_assert.sql");
const OPERATION_INSERT_SQL: &str = include_str!("../queries/auth/operation_insert.sql");
const OPERATION_BY_ID_SQL: &str = include_str!("../queries/auth/operation_by_id.sql");
const AUDIT_OPERATION_EXISTS_SQL: &str = include_str!("../queries/auth/audit_operation_exists.sql");
const DELIVERY_ACK_TOMBSTONE_INSERT_SQL: &str =
    include_str!("../queries/auth/delivery_ack_tombstone_insert.sql");
const DELIVERY_ACK_TOMBSTONE_BY_OPERATION_SQL: &str =
    include_str!("../queries/auth/delivery_ack_tombstone_by_operation.sql");
const IDENTITY_PROVISION_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/identity_provision_postcondition.sql");
const SESSION_ISSUE_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/session_issue_postcondition.sql");
const SESSION_AUTHENTICATION_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/session_authentication_postcondition.sql");
const SESSION_ROTATION_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/session_rotation_postcondition.sql");
const API_KEY_ISSUE_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/api_key_issue_postcondition.sql");
const API_KEY_AUTHENTICATION_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/api_key_authentication_postcondition.sql");
const API_KEY_REVOKE_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/api_key_revoke_postcondition.sql");
const SESSION_REVOKE_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/session_revoke_postcondition.sql");
const LOGOUT_ALL_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/logout_all_postcondition.sql");
const VERIFICATION_ISSUE_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/verification_issue_postcondition.sql");
const VERIFICATION_MATERIALIZATION_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/verification_materialization_postcondition.sql");
const VERIFICATION_ATTEMPT_VERIFIED_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/verification_attempt_verified_postcondition.sql");
const VERIFICATION_ATTEMPT_PROVISIONING_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/verification_attempt_provisioning_postcondition.sql");
const VERIFICATION_ATTEMPT_LINKED_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/verification_attempt_linked_postcondition.sql");
const OAUTH_OPERATION_INSERT_SQL: &str = include_str!("../queries/auth/oauth_operation_insert.sql");
const OAUTH_OPERATION_BY_ID_SQL: &str = include_str!("../queries/auth/oauth_operation_by_id.sql");
const OAUTH_FLOW_BY_STATE_SQL: &str = include_str!("../queries/auth/oauth_flow_by_state.sql");
const OAUTH_FLOW_ID_EXISTS_SQL: &str = include_str!("../queries/auth/oauth_flow_id_exists.sql");
const OAUTH_RESERVATION_BY_ID_SQL: &str =
    include_str!("../queries/auth/oauth_reservation_by_id.sql");
const OAUTH_EXTERNAL_ACCOUNT_BY_SUBJECT_SQL: &str =
    include_str!("../queries/auth/oauth_external_account_by_subject.sql");
const OAUTH_FLOW_INSERT_SQL: &str = include_str!("../queries/auth/oauth_flow_insert.sql");
const OAUTH_FLOW_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_flow_absent_assert.sql");
const OAUTH_FLOW_BEGIN_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_flow_begin_absent_assert.sql");
const OAUTH_FLOW_COLLISION_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_flow_collision_assert.sql");
const OAUTH_FLOW_CAPACITY_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_flow_capacity_assert.sql");
const OAUTH_FLOW_CURRENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_flow_current_assert.sql");
const OAUTH_FLOW_CONSUME_SQL: &str = include_str!("../queries/auth/oauth_flow_consume.sql");
const OAUTH_FLOW_DELETE_SQL: &str = include_str!("../queries/auth/oauth_flow_delete.sql");
const OAUTH_EXPIRED_FLOWS_DELETE_SQL: &str =
    include_str!("../queries/auth/oauth_expired_flows_delete.sql");
const OAUTH_RESERVATION_INSERT_SQL: &str =
    include_str!("../queries/auth/oauth_reservation_insert.sql");
const OAUTH_RESERVATION_CONSUME_SQL: &str =
    include_str!("../queries/auth/oauth_reservation_consume.sql");
const OAUTH_RESERVATION_ABSENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_reservation_absent_assert.sql");
const OAUTH_RESERVATION_CURRENT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_reservation_current_assert.sql");
const OAUTH_CONTINUATION_INVALID_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_continuation_invalid_assert.sql");
const OAUTH_MUTATION_GRANT_INVALID_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_mutation_grant_invalid_assert.sql");
const OAUTH_EXTERNAL_ACCOUNT_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_external_account_authority_assert.sql");
const OAUTH_EXTERNAL_ACCOUNT_SNAPSHOT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_external_account_snapshot_assert.sql");
const OAUTH_IDENTIFIER_AUTHORITY_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_identifier_authority_assert.sql");
const OAUTH_IDENTIFIER_SNAPSHOT_ASSERT_SQL: &str =
    include_str!("../queries/auth/oauth_identifier_snapshot_assert.sql");
const OAUTH_EXTERNAL_ACCOUNT_DELETE_FALLBACKS_SQL: &str =
    include_str!("../queries/auth/oauth_external_account_delete_fallbacks.sql");
const OAUTH_EXTERNAL_ACCOUNT_UPSERT_SQL: &str =
    include_str!("../queries/auth/oauth_external_account_upsert.sql");
const OAUTH_EXTERNAL_ACCOUNT_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/oauth_external_account_postcondition.sql");
const OAUTH_VERIFIED_POSTCONDITION_SQL: &str =
    include_str!("../queries/auth/oauth_verified_postcondition.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterFailure {
    Invalid,
    Conflict,
    Unavailable,
    Timeout,
    Corrupt,
}

impl AdapterFailure {
    const fn code(self) -> &'static str {
        match self {
            Self::Invalid => "auth_repository_invalid_request",
            Self::Conflict => "auth_repository_conflict",
            Self::Unavailable => "auth_repository_unavailable",
            Self::Timeout => "auth_repository_timeout",
            Self::Corrupt => "auth_repository_corrupt_result",
        }
    }

    fn into_port(self) -> PortError {
        match self {
            Self::Invalid => PortError::InvalidRequest(self.code().into()),
            Self::Conflict => PortError::Conflict,
            Self::Unavailable | Self::Timeout | Self::Corrupt => {
                PortError::Adapter(self.code().into())
            }
        }
    }
}

type AdapterResult<T> = Result<T, AdapterFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperationReceipt {
    operation_id: String,
    operation_kind: &'static str,
    subject_id: String,
    result_code: &'static str,
    result_timestamp: Option<TimestampMillis>,
    request_fingerprint: String,
    committed_at: TimestampMillis,
}

impl OperationReceipt {
    fn for_audit(
        operation_kind: &'static str,
        subject_id: impl Into<String>,
        audit: &DecisionAudit,
        semantic_parts: &[String],
    ) -> Self {
        let subject_id = subject_id.into();
        let correlation = audit.correlation_id.to_string();
        let operation_hash = stable_hash(&[
            b"frame/auth/operation/v1",
            correlation.as_bytes(),
            operation_kind.as_bytes(),
        ]);
        let mut fingerprint_parts = Vec::with_capacity(4 + semantic_parts.len());
        fingerprint_parts.push(b"frame/auth/request/v1".as_slice());
        fingerprint_parts.push(operation_kind.as_bytes());
        fingerprint_parts.push(subject_id.as_bytes());
        let occurred_at = audit.occurred_at.get().to_string();
        fingerprint_parts.push(occurred_at.as_bytes());
        fingerprint_parts.extend(semantic_parts.iter().map(String::as_bytes));
        let request_fingerprint = bytes_to_hex(&stable_hash(&fingerprint_parts));
        Self {
            operation_id: uuid_v8(operation_hash),
            operation_kind,
            subject_id,
            result_code: "committed",
            result_timestamp: None,
            request_fingerprint,
            committed_at: audit.occurred_at,
        }
    }

    fn internal(
        operation_kind: &'static str,
        subject_id: impl Into<String>,
        committed_at: TimestampMillis,
        semantic_parts: &[String],
    ) -> Self {
        let subject_id = subject_id.into();
        let timestamp = committed_at.get().to_string();
        let operation_hash = stable_hash(&[
            b"frame/auth/internal-operation/v1",
            operation_kind.as_bytes(),
            subject_id.as_bytes(),
            timestamp.as_bytes(),
        ]);
        let mut fingerprint_parts = Vec::with_capacity(4 + semantic_parts.len());
        fingerprint_parts.push(b"frame/auth/internal-request/v1".as_slice());
        fingerprint_parts.push(operation_kind.as_bytes());
        fingerprint_parts.push(subject_id.as_bytes());
        fingerprint_parts.push(timestamp.as_bytes());
        fingerprint_parts.extend(semantic_parts.iter().map(String::as_bytes));
        let request_fingerprint = bytes_to_hex(&stable_hash(&fingerprint_parts));
        Self {
            operation_id: uuid_v8(operation_hash),
            operation_kind,
            subject_id,
            result_code: "committed",
            result_timestamp: None,
            request_fingerprint,
            committed_at,
        }
    }

    fn with_result_code(mut self, result_code: &'static str) -> Self {
        self.result_code = result_code;
        self
    }

    fn with_result_timestamp(mut self, result_timestamp: TimestampMillis) -> Self {
        self.result_timestamp = Some(result_timestamp);
        self
    }
}

fn stable_hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().into()
}

fn uuid_v8(hash: [u8; 32]) -> String {
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn stable_child_uuid(operation_id: &str, label: &str) -> String {
    uuid_v8(stable_hash(&[
        b"frame/auth/operation-child/v1",
        operation_id.as_bytes(),
        label.as_bytes(),
    ]))
}

#[derive(Debug, Serialize)]
struct AuthRepositoryTelemetry {
    event: &'static str,
    operation: &'static str,
    outcome: &'static str,
    duration_ms: u64,
    rows: usize,
}

impl AuthRepositoryTelemetry {
    fn emit(operation: &'static str, outcome: &'static str, started_at_ms: f64, rows: usize) {
        let elapsed = js_sys::Date::now() - started_at_ms;
        let duration_ms = if elapsed.is_finite() && elapsed > 0.0 {
            elapsed.min(9_007_199_254_740_991_f64).floor() as u64
        } else {
            0
        };
        let event = Self {
            event: "d1_auth_repository",
            operation,
            outcome,
            duration_ms,
            rows,
        };
        let json = serde_json::to_string(&event).unwrap_or_else(|_| {
            "{\"event\":\"d1_auth_repository\",\"outcome\":\"telemetry_encoding_failed\"}".into()
        });
        worker::console_log!("{}", json);
    }

    fn span(operation: &'static str) -> AuthRepositoryTelemetrySpan {
        AuthRepositoryTelemetrySpan {
            operation,
            started_at_ms: js_sys::Date::now(),
            outcome: "error",
            rows: 0,
        }
    }
}

struct AuthRepositoryTelemetrySpan {
    operation: &'static str,
    started_at_ms: f64,
    outcome: &'static str,
    rows: usize,
}

impl AuthRepositoryTelemetrySpan {
    fn finish(&mut self, outcome: &'static str, rows: usize) {
        self.outcome = outcome;
        self.rows = rows;
    }
}

impl Drop for AuthRepositoryTelemetrySpan {
    fn drop(&mut self) {
        AuthRepositoryTelemetry::emit(self.operation, self.outcome, self.started_at_ms, self.rows);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DigestWire {
    key_version: u16,
    digest: String,
}

impl From<&VersionedSecretDigest> for DigestWire {
    fn from(value: &VersionedSecretDigest) -> Self {
        Self {
            key_version: value.key_version.get(),
            digest: value.digest.expose_for_verification().into(),
        }
    }
}

impl TryFrom<DigestWire> for VersionedSecretDigest {
    type Error = AdapterFailure;

    fn try_from(value: DigestWire) -> AdapterResult<Self> {
        Ok(Self::new(
            HashKeyVersion::new(value.key_version).map_err(|_| AdapterFailure::Corrupt)?,
            SecretDigest::parse_sha256(value.digest).map_err(|_| AdapterFailure::Corrupt)?,
        ))
    }
}

fn candidates_json(candidates: &SecretDigestCandidates) -> AdapterResult<String> {
    let values = candidates.iter().map(DigestWire::from).collect::<Vec<_>>();
    let json = serde_json::to_string(&values).map_err(|_| AdapterFailure::Invalid)?;
    if json.len() > MAX_CANDIDATE_JSON_BYTES {
        return Err(AdapterFailure::Invalid);
    }
    Ok(json)
}

fn parse_candidates_json(value: &str) -> AdapterResult<SecretDigestCandidates> {
    if value.len() > MAX_CANDIDATE_JSON_BYTES {
        return Err(AdapterFailure::Corrupt);
    }
    let mut values = serde_json::from_str::<Vec<DigestWire>>(value)
        .map_err(|_| AdapterFailure::Corrupt)?
        .into_iter()
        .map(VersionedSecretDigest::try_from)
        .collect::<AdapterResult<Vec<_>>>()?;
    if values.is_empty() || values.len() > 5 {
        return Err(AdapterFailure::Corrupt);
    }
    let active = values.remove(0);
    SecretDigestCandidates::new(active, values).map_err(|_| AdapterFailure::Corrupt)
}

fn digest_values(digest: &VersionedSecretDigest) -> (JsValue, JsValue) {
    (
        JsValue::from_f64(f64::from(digest.key_version.get())),
        JsValue::from_str(digest.digest.expose_for_verification()),
    )
}

fn opt_string(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn opt_i64(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64))
}

fn enum_name<T: Serialize>(value: T) -> AdapterResult<String> {
    let name = serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(AdapterFailure::Invalid)?;
    Ok(match name.as_str() {
        "o_auth_begin" => "oauth_begin".into(),
        "o_auth_exchange" => "oauth_exchange".into(),
        "o_auth_exchange_preflight" => "oauth_exchange_preflight".into(),
        _ => name,
    })
}

fn parse_enum<T: DeserializeOwned>(value: &str) -> AdapterResult<T> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| AdapterFailure::Corrupt)
}

fn rate_policy_fingerprint(policy: frame_domain::MultiRateLimitPolicy) -> String {
    format!(
        "{}:{}:{}|{}:{}:{}|{}:{}:{}|{}:{}:{}",
        policy.identifier.max_attempts(),
        policy.identifier.window().get(),
        policy.identifier.block_for().get(),
        policy.source.max_attempts(),
        policy.source.window().get(),
        policy.source.block_for().get(),
        policy.device.max_attempts(),
        policy.device.window().get(),
        policy.device.block_for().get(),
        policy.global.max_attempts(),
        policy.global.window().get(),
        policy.global.block_for().get(),
    )
}

fn digest_candidates_contain(
    candidates: &SecretDigestCandidates,
    expected: &VersionedSecretDigest,
) -> bool {
    candidates.iter().any(|candidate| candidate == expected)
}

const fn fetch_site_name(value: FetchSite) -> &'static str {
    match value {
        FetchSite::SameOrigin => "same_origin",
        FetchSite::SameSite => "same_site",
        FetchSite::CrossSite => "cross_site",
        FetchSite::None => "none",
        FetchSite::Unknown => "unknown",
    }
}

fn safe_uuid(value: &str) -> AdapterResult<()> {
    Uuid::parse_str(value)
        .ok()
        .filter(|value| !value.is_nil())
        .map(|_| ())
        .ok_or(AdapterFailure::Corrupt)
}

fn safe_revision(value: i64) -> AdapterResult<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or(AdapterFailure::Corrupt)
}

fn safe_timestamp(value: i64) -> AdapterResult<TimestampMillis> {
    TimestampMillis::new(value).map_err(|_| AdapterFailure::Corrupt)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn hex_to_bytes(value: &str) -> AdapterResult<Vec<u8>> {
    if !value.len().is_multiple_of(2)
        || !(64..=131_072).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(AdapterFailure::Corrupt);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(value: u8) -> AdapterResult<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(AdapterFailure::Corrupt),
    }
}

#[derive(Debug, Deserialize)]
struct PrincipalRow {
    user_id: String,
    identity_revision: i64,
    session_version: i64,
    identity_row_revision: i64,
    organization_id: Option<String>,
    role: Option<String>,
    member_revision: Option<i64>,
    organization_revision: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OperationRow {
    operation_id: String,
    operation_kind: String,
    subject_id: String,
    result_code: String,
    result_timestamp_ms: Option<i64>,
    request_fingerprint: String,
    committed_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct AuditOperationRow {
    present: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredOperationResult {
    code: String,
    timestamp: Option<TimestampMillis>,
}

#[derive(Debug, Deserialize)]
struct DeliveryAckTombstoneRow {
    operation_id: String,
    delivery_id: String,
    lease_id: String,
    attempt: i64,
    lease_expires_at_ms: i64,
    acknowledged_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct SessionRow {
    id: String,
    family_id: String,
    user_id: String,
    client_kind: String,
    token_key_version: i64,
    token_digest: String,
    csrf_key_version: Option<i64>,
    csrf_digest: Option<String>,
    browser_origin: Option<String>,
    issued_at_ms: i64,
    rotated_at_ms: i64,
    idle_expires_at_ms: i64,
    absolute_expires_at_ms: i64,
    session_version: i64,
    generation: i64,
    state: String,
    revoked_at_ms: Option<i64>,
    revocation_reason: Option<String>,
    session_revision: i64,
    current_session_version: i64,
    user_status: String,
}

#[derive(Debug, Deserialize)]
struct CredentialSessionRow {
    matched_key_version: i64,
    matched_digest: String,
    matched_family_id: String,
    credential_state: String,
    credential_revision: i64,
    #[serde(flatten)]
    session: SessionRow,
}

#[derive(Debug, Deserialize)]
struct MutationGrantRow {
    id: String,
    session_id: String,
    user_id: String,
    generation: i64,
    token_key_version: i64,
    token_digest: String,
    session_revision: i64,
    session_state: String,
    idle_expires_at_ms: i64,
    absolute_expires_at_ms: i64,
    session_version: i64,
    current_generation: i64,
    current_token_key_version: i64,
    current_token_digest: String,
    current_session_version: i64,
    user_status: String,
}

#[derive(Debug, Deserialize)]
struct IssuanceGrantRow {
    id: String,
    user_id: String,
    identity_revision: i64,
    expires_at_ms: i64,
    current_identity_revision: i64,
}

#[derive(Debug, Deserialize)]
struct ProvisioningGrantRow {
    id: String,
    user_id: String,
    identity_revision: i64,
    identifier_key_version: i64,
    identifier_digest: String,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct IdentifierOwnerRow {
    user_id: String,
    key_version: i64,
    digest: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthFlowRow {
    id: String,
    provider: String,
    purpose: String,
    initiator_session_id: Option<String>,
    initiator_user_id: Option<String>,
    initiator_generation: Option<i64>,
    state_key_version: i64,
    state_digest: String,
    pkce_key_version: i64,
    pkce_digest: String,
    redirect_key_version: i64,
    redirect_digest: String,
    audience_key_version: i64,
    audience_digest: String,
    created_at_ms: i64,
    expires_at_ms: i64,
    consumed_at_ms: Option<i64>,
    revoked: i64,
    revision: i64,
    last_operation_id: String,
    candidate_order: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthReservationRow {
    id: String,
    flow_id: String,
    provider: String,
    initiator_session_id: Option<String>,
    initiator_user_id: Option<String>,
    initiator_generation: Option<i64>,
    expires_at_ms: i64,
    created_at_ms: i64,
    consumed_at_ms: Option<i64>,
    revision: i64,
    last_operation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthExternalAccountRow {
    user_id: String,
    subject_key_version: i64,
    subject_digest: String,
    candidate_order: i64,
}

#[derive(Debug, Clone)]
struct StoredOAuthFlow {
    id: OAuthFlowId,
    provider: OAuthProvider,
    purpose: OAuthFlowPurpose,
    initiator: Option<SessionContinuationBinding>,
    state_digest: VersionedSecretDigest,
    pkce_digest: VersionedSecretDigest,
    redirect_digest: VersionedSecretDigest,
    audience_digest: VersionedSecretDigest,
    created_at: TimestampMillis,
    expires_at: TimestampMillis,
    consumed_at: Option<TimestampMillis>,
    revoked: bool,
    revision: u64,
}

#[derive(Debug, Clone)]
struct StoredOAuthReservation {
    id: OAuthExchangeReservationId,
    flow_id: OAuthFlowId,
    provider: OAuthProvider,
    initiator: Option<SessionContinuationBinding>,
    expires_at: TimestampMillis,
    created_at: TimestampMillis,
    consumed_at: Option<TimestampMillis>,
    revision: u64,
}

#[derive(Debug, Clone)]
struct StoredExternalAccount {
    user_id: UserId,
}

#[derive(Debug, Deserialize)]
struct PendingRow {
    delivery_id: String,
    identifier_candidates_json: String,
    active_identifier_key_version: i64,
    active_identifier_digest: String,
    secret_key_version: i64,
    secret_digest: String,
    purpose: String,
    channel: String,
    initiator_session_id: Option<String>,
    initiator_user_id: Option<String>,
    initiator_generation: Option<i64>,
    provisioning_user_id: Option<String>,
    provisioning_revision: Option<i64>,
    max_attempts: i64,
    created_at_ms: i64,
    expires_at_ms: i64,
    sealed_payload_hex: String,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct VerificationRow {
    id: String,
    user_id: Option<String>,
    initiator_session_id: Option<String>,
    initiator_user_id: Option<String>,
    initiator_generation: Option<i64>,
    provisioning_revision: Option<i64>,
    identifier_key_version: i64,
    identifier_digest: String,
    secret_key_version: i64,
    secret_digest: String,
    purpose: String,
    channel: String,
    attempt_count: i64,
    max_attempts: i64,
    created_at_ms: i64,
    expires_at_ms: i64,
    consumed_at_ms: Option<i64>,
    state: String,
    revision: i64,
    secret_matches: i64,
}

#[derive(Debug, Deserialize)]
struct ApiKeyRow {
    id: String,
    owner_id: String,
    tenant_id: String,
    key_version: i64,
    key_digest: String,
    scopes_json: String,
    created_at_ms: i64,
    expires_at_ms: Option<i64>,
    revoked_at_ms: Option<i64>,
    revision: i64,
    candidate_order: i64,
}

#[derive(Debug, Deserialize)]
struct RateBucketRow {
    action: String,
    dimension: String,
    key_version: i64,
    digest: String,
    window_started_at_ms: i64,
    attempt_count: i64,
    blocked_until_ms: Option<i64>,
    updated_at_ms: i64,
    gc_at_ms: i64,
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    bucket_count: i64,
}

#[derive(Debug, Deserialize)]
struct ExistenceRow {
    present: i64,
}

#[derive(Debug, Deserialize)]
struct AggregateCountsRow {
    active_sessions: i64,
    live_credentials: i64,
}

#[derive(Debug, Deserialize)]
struct LogoutAllResultRow {
    new_session_version: i64,
    revoked_sessions: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct DeliveryRow {
    delivery_id: String,
    sealed_payload_hex: String,
    created_at_ms: i64,
    expires_at_ms: i64,
    next_attempt_at_ms: i64,
    attempt: i64,
    lease_id: Option<String>,
    lease_expires_at_ms: Option<i64>,
    revision: i64,
}

impl SessionRow {
    fn decode(&self) -> AdapterResult<(AuthSessionRecord, u64, u64, bool)> {
        safe_uuid(&self.id)?;
        safe_uuid(&self.family_id)?;
        safe_uuid(&self.user_id)?;
        let client_kind = parse_enum::<AuthClientKind>(&self.client_kind)?;
        let token_digest = versioned_digest(self.token_key_version, &self.token_digest)?;
        let csrf_digest = match (self.csrf_key_version, self.csrf_digest.as_deref()) {
            (Some(version), Some(digest)) => Some(versioned_digest(version, digest)?),
            (None, None) => None,
            _ => return Err(AdapterFailure::Corrupt),
        };
        let browser_origin = self
            .browser_origin
            .as_ref()
            .map(|value| ExactBrowserOrigin::parse(value.clone()))
            .transpose()
            .map_err(|_| AdapterFailure::Corrupt)?;
        let issued_at = safe_timestamp(self.issued_at_ms)?;
        let rotated_at = safe_timestamp(self.rotated_at_ms)?;
        let idle_expires_at = safe_timestamp(self.idle_expires_at_ms)?;
        let absolute_expires_at = safe_timestamp(self.absolute_expires_at_ms)?;
        let revoked_at = self.revoked_at_ms.map(safe_timestamp).transpose()?;
        let state = parse_enum::<AuthSessionState>(&self.state)?;
        let user_active = match self.user_status.as_str() {
            "active" => true,
            "suspended" | "deleted" => false,
            _ => return Err(AdapterFailure::Corrupt),
        };
        let revocation_reason = self
            .revocation_reason
            .as_deref()
            .map(parse_enum::<SessionRevocationReason>)
            .transpose()?;
        if rotated_at < issued_at
            || idle_expires_at <= issued_at
            || idle_expires_at > absolute_expires_at
            || (client_kind == AuthClientKind::Browser)
                != (csrf_digest.is_some() && browser_origin.is_some())
            || (state == AuthSessionState::Active)
                != (revoked_at.is_none() && revocation_reason.is_none())
        {
            return Err(AdapterFailure::Corrupt);
        }
        Ok((
            AuthSessionRecord {
                id: SessionId::parse(&self.id).map_err(|_| AdapterFailure::Corrupt)?,
                family_id: SessionFamilyId::parse(&self.family_id)
                    .map_err(|_| AdapterFailure::Corrupt)?,
                user_id: UserId::parse(&self.user_id).map_err(|_| AdapterFailure::Corrupt)?,
                client_kind,
                token_digest,
                csrf_digest,
                browser_origin,
                issued_at,
                rotated_at,
                idle_expires_at,
                absolute_expires_at,
                session_version: safe_revision(self.session_version)?,
                generation: safe_revision(self.generation)?,
                state,
                revoked_at,
                revocation_reason,
            },
            safe_revision(self.current_session_version)?,
            safe_revision(self.session_revision)?,
            user_active,
        ))
    }
}

impl DeliveryRow {
    fn envelope(&self) -> AdapterResult<SealedDeliveryEnvelope> {
        safe_uuid(&self.delivery_id)?;
        let payload = hex_to_bytes(&self.sealed_payload_hex)?;
        let created_at = safe_timestamp(self.created_at_ms)?;
        let id = AuthDeliveryId::parse(&self.delivery_id).map_err(|_| AdapterFailure::Corrupt)?;
        persisted_envelope(id, payload, created_at)
    }

    fn validated(&self) -> AdapterResult<()> {
        let envelope = self.envelope()?;
        let expires_at = safe_timestamp(self.expires_at_ms)?;
        let next_attempt_at = safe_timestamp(self.next_attempt_at_ms)?;
        safe_revision(self.revision)?;
        let attempt = u16::try_from(self.attempt).map_err(|_| AdapterFailure::Corrupt)?;
        if attempt > MAX_AUTH_DELIVERY_ATTEMPTS
            || self.lease_id.is_some() != self.lease_expires_at_ms.is_some()
            || expires_at <= envelope.created_at
            || next_attempt_at < envelope.created_at
        {
            return Err(AdapterFailure::Corrupt);
        }
        if let Some(id) = &self.lease_id {
            safe_uuid(id)?;
        }
        if let Some(expires) = self.lease_expires_at_ms {
            let lease_expires_at = safe_timestamp(expires)?;
            if attempt == 0
                || lease_expires_at <= envelope.created_at
                || lease_expires_at > expires_at
            {
                return Err(AdapterFailure::Corrupt);
            }
        }
        Ok(())
    }
}

impl ApiKeyRow {
    fn decode(self) -> AdapterResult<(ManagedApiKeyRecord, u64, usize)> {
        safe_uuid(&self.id)?;
        safe_uuid(&self.owner_id)?;
        safe_uuid(&self.tenant_id)?;
        let scopes = serde_json::from_str::<Vec<ApiKeyScope>>(&self.scopes_json)
            .map_err(|_| AdapterFailure::Corrupt)?;
        if scopes.is_empty()
            || scopes.len() > 32
            || scopes
                .iter()
                .enumerate()
                .any(|(index, scope)| scopes[..index].iter().any(|other| other == scope))
        {
            return Err(AdapterFailure::Corrupt);
        }
        let created_at = safe_timestamp(self.created_at_ms)?;
        let expires_at = self.expires_at_ms.map(safe_timestamp).transpose()?;
        let revoked_at = self.revoked_at_ms.map(safe_timestamp).transpose()?;
        if expires_at.is_some_and(|expires| expires <= created_at)
            || revoked_at.is_some_and(|revoked| revoked < created_at)
        {
            return Err(AdapterFailure::Corrupt);
        }
        let candidate_order = usize::try_from(self.candidate_order)
            .ok()
            .filter(|order| *order < 5)
            .ok_or(AdapterFailure::Corrupt)?;
        Ok((
            ManagedApiKeyRecord {
                id: ApiKeyId::parse(&self.id).map_err(|_| AdapterFailure::Corrupt)?,
                owner_id: UserId::parse(&self.owner_id).map_err(|_| AdapterFailure::Corrupt)?,
                tenant_id: TenantId::parse(&self.tenant_id).map_err(|_| AdapterFailure::Corrupt)?,
                key_digest: versioned_digest(self.key_version, &self.key_digest)?,
                scopes,
                created_at,
                expires_at,
                revoked_at,
            },
            safe_revision(self.revision)?,
            candidate_order,
        ))
    }
}

fn decode_oauth_initiator(
    session_id: Option<String>,
    user_id: Option<String>,
    generation: Option<i64>,
) -> AdapterResult<Option<SessionContinuationBinding>> {
    match (session_id, user_id, generation) {
        (None, None, None) => Ok(None),
        (Some(session_id), Some(user_id), Some(generation)) => {
            Ok(Some(SessionContinuationBinding {
                session_id: SessionId::parse(&session_id).map_err(|_| AdapterFailure::Corrupt)?,
                user_id: UserId::parse(&user_id).map_err(|_| AdapterFailure::Corrupt)?,
                generation: safe_revision(generation)?,
            }))
        }
        _ => Err(AdapterFailure::Corrupt),
    }
}

impl OAuthFlowRow {
    fn decode(self) -> AdapterResult<StoredOAuthFlow> {
        if !(0..5).contains(&self.candidate_order) || !matches!(self.revoked, 0 | 1) {
            return Err(AdapterFailure::Corrupt);
        }
        safe_uuid(&self.last_operation_id)?;
        let created_at = safe_timestamp(self.created_at_ms)?;
        let expires_at = safe_timestamp(self.expires_at_ms)?;
        let consumed_at = self.consumed_at_ms.map(safe_timestamp).transpose()?;
        if expires_at <= created_at
            || consumed_at.is_some_and(|value| value < created_at || value >= expires_at)
        {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(StoredOAuthFlow {
            id: OAuthFlowId::parse(&self.id).map_err(|_| AdapterFailure::Corrupt)?,
            provider: parse_enum(&self.provider)?,
            purpose: parse_enum(&self.purpose)?,
            initiator: decode_oauth_initiator(
                self.initiator_session_id,
                self.initiator_user_id,
                self.initiator_generation,
            )?,
            state_digest: versioned_digest(self.state_key_version, &self.state_digest)?,
            pkce_digest: versioned_digest(self.pkce_key_version, &self.pkce_digest)?,
            redirect_digest: versioned_digest(self.redirect_key_version, &self.redirect_digest)?,
            audience_digest: versioned_digest(self.audience_key_version, &self.audience_digest)?,
            created_at,
            expires_at,
            consumed_at,
            revoked: self.revoked == 1,
            revision: safe_revision(self.revision)?,
        })
    }
}

impl OAuthReservationRow {
    fn decode(self) -> AdapterResult<StoredOAuthReservation> {
        safe_uuid(&self.last_operation_id)?;
        let created_at = safe_timestamp(self.created_at_ms)?;
        let expires_at = safe_timestamp(self.expires_at_ms)?;
        let consumed_at = self.consumed_at_ms.map(safe_timestamp).transpose()?;
        if expires_at <= created_at
            || consumed_at.is_some_and(|value| value < created_at || value >= expires_at)
        {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(StoredOAuthReservation {
            id: OAuthExchangeReservationId::parse(&self.id).map_err(|_| AdapterFailure::Corrupt)?,
            flow_id: OAuthFlowId::parse(&self.flow_id).map_err(|_| AdapterFailure::Corrupt)?,
            provider: parse_enum(&self.provider)?,
            initiator: decode_oauth_initiator(
                self.initiator_session_id,
                self.initiator_user_id,
                self.initiator_generation,
            )?,
            expires_at,
            created_at,
            consumed_at,
            revision: safe_revision(self.revision)?,
        })
    }
}

impl OAuthExternalAccountRow {
    fn decode(self) -> AdapterResult<StoredExternalAccount> {
        if !(0..5).contains(&self.candidate_order) {
            return Err(AdapterFailure::Corrupt);
        }
        versioned_digest(self.subject_key_version, &self.subject_digest)?;
        Ok(StoredExternalAccount {
            user_id: UserId::parse(&self.user_id).map_err(|_| AdapterFailure::Corrupt)?,
        })
    }
}

impl PendingRow {
    fn decode(self) -> AdapterResult<PendingState> {
        safe_uuid(&self.delivery_id)?;
        let identifier_candidates = parse_candidates_json(&self.identifier_candidates_json)?;
        let active_identifier = versioned_digest(
            self.active_identifier_key_version,
            &self.active_identifier_digest,
        )?;
        if identifier_candidates.active() != &active_identifier {
            return Err(AdapterFailure::Corrupt);
        }
        let secret_digest = versioned_digest(self.secret_key_version, &self.secret_digest)?;
        let purpose = parse_enum::<VerificationPurpose>(&self.purpose)?;
        let channel = parse_enum::<VerificationChannel>(&self.channel)?;
        let initiator = match (
            self.initiator_session_id,
            self.initiator_user_id,
            self.initiator_generation,
        ) {
            (Some(session), Some(user), Some(generation)) => Some(SessionContinuationBinding {
                session_id: SessionId::parse(&session).map_err(|_| AdapterFailure::Corrupt)?,
                user_id: UserId::parse(&user).map_err(|_| AdapterFailure::Corrupt)?,
                generation: safe_revision(generation)?,
            }),
            (None, None, None) => None,
            _ => return Err(AdapterFailure::Corrupt),
        };
        let provisioning = match (self.provisioning_user_id, self.provisioning_revision) {
            (Some(user), Some(revision)) => Some((
                UserId::parse(&user).map_err(|_| AdapterFailure::Corrupt)?,
                safe_revision(revision)?,
            )),
            (None, None) => None,
            _ => return Err(AdapterFailure::Corrupt),
        };
        if (purpose == VerificationPurpose::AccountLink) != initiator.is_some()
            || (purpose == VerificationPurpose::IdentityProvisioning) != provisioning.is_some()
        {
            return Err(AdapterFailure::Corrupt);
        }
        let max_attempts = u16::try_from(self.max_attempts)
            .ok()
            .filter(|attempts| (1..=100).contains(attempts))
            .ok_or(AdapterFailure::Corrupt)?;
        let created_at = safe_timestamp(self.created_at_ms)?;
        let expires_at = safe_timestamp(self.expires_at_ms)?;
        if expires_at <= created_at {
            return Err(AdapterFailure::Corrupt);
        }
        let sealed_payload = hex_to_bytes(&self.sealed_payload_hex)?;
        Ok(PendingState {
            delivery_id: AuthDeliveryId::parse(&self.delivery_id)
                .map_err(|_| AdapterFailure::Corrupt)?,
            identifier_candidates,
            secret_digest,
            purpose,
            channel,
            initiator,
            provisioning,
            max_attempts,
            created_at,
            expires_at,
            sealed_payload,
            revision: safe_revision(self.revision)?,
        })
    }
}

impl VerificationRow {
    fn decode(self) -> AdapterResult<DecodedVerification> {
        safe_uuid(&self.id)?;
        let purpose = parse_enum::<VerificationPurpose>(&self.purpose)?;
        let channel = parse_enum::<VerificationChannel>(&self.channel)?;
        let initiator = match (
            self.initiator_session_id,
            self.initiator_user_id,
            self.initiator_generation,
        ) {
            (Some(session), Some(user), Some(generation)) => Some(SessionContinuationBinding {
                session_id: SessionId::parse(&session).map_err(|_| AdapterFailure::Corrupt)?,
                user_id: UserId::parse(&user).map_err(|_| AdapterFailure::Corrupt)?,
                generation: safe_revision(generation)?,
            }),
            (None, None, None) => None,
            _ => return Err(AdapterFailure::Corrupt),
        };
        let attempt_count =
            u16::try_from(self.attempt_count).map_err(|_| AdapterFailure::Corrupt)?;
        let max_attempts = u16::try_from(self.max_attempts)
            .ok()
            .filter(|attempts| (1..=100).contains(attempts))
            .ok_or(AdapterFailure::Corrupt)?;
        let created_at = safe_timestamp(self.created_at_ms)?;
        let expires_at = safe_timestamp(self.expires_at_ms)?;
        let consumed_at = self.consumed_at_ms.map(safe_timestamp).transpose()?;
        let state = parse_enum::<VerificationState>(&self.state)?;
        let provisioning_revision = self.provisioning_revision.map(safe_revision).transpose()?;
        if attempt_count > max_attempts
            || expires_at <= created_at
            || (purpose == VerificationPurpose::AccountLink) != initiator.is_some()
            || (purpose == VerificationPurpose::IdentityProvisioning)
                != provisioning_revision.is_some()
            || (state == VerificationState::Consumed) != consumed_at.is_some()
            || !matches!(self.secret_matches, 0 | 1)
        {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(DecodedVerification {
            challenge: VerificationChallenge {
                id: VerificationId::parse(&self.id).map_err(|_| AdapterFailure::Corrupt)?,
                user_id: self
                    .user_id
                    .map(|value| UserId::parse(&value).map_err(|_| AdapterFailure::Corrupt))
                    .transpose()?,
                initiator,
                provisioning_revision,
                identifier_digest: versioned_digest(
                    self.identifier_key_version,
                    &self.identifier_digest,
                )?,
                secret_digest: versioned_digest(self.secret_key_version, &self.secret_digest)?,
                purpose,
                channel,
                attempt_count,
                max_attempts,
                created_at,
                expires_at,
                consumed_at,
                state,
            },
            revision: safe_revision(self.revision)?,
            secret_matches: self.secret_matches == 1,
        })
    }
}

// `SealedDeliveryEnvelope` deliberately has no public arbitrary-ID
// constructor. Deserialize is the narrowly typed persistence boundary.
fn persisted_envelope(
    id: AuthDeliveryId,
    payload: Vec<u8>,
    created_at: TimestampMillis,
) -> AdapterResult<SealedDeliveryEnvelope> {
    #[derive(Serialize)]
    struct EnvelopeWire<'a> {
        id: AuthDeliveryId,
        payload: &'a [u8],
        created_at: TimestampMillis,
    }
    let wire = EnvelopeWire {
        id,
        payload: &payload,
        created_at,
    };
    let value = serde_json::to_value(wire).map_err(|_| AdapterFailure::Corrupt)?;
    serde_json::from_value(value).map_err(|_| AdapterFailure::Corrupt)
}

fn versioned_digest(key_version: i64, digest: &str) -> AdapterResult<VersionedSecretDigest> {
    let key_version = u16::try_from(key_version).map_err(|_| AdapterFailure::Corrupt)?;
    Ok(VersionedSecretDigest::new(
        HashKeyVersion::new(key_version).map_err(|_| AdapterFailure::Corrupt)?,
        SecretDigest::parse_sha256(digest).map_err(|_| AdapterFailure::Corrupt)?,
    ))
}

/// D1 adapter for the provider-free `AuthStateRepository` capabilities.
pub struct D1AuthStateRepository<'database> {
    database: &'database D1Database,
    query_timeout_ms: u64,
}

impl<'database> D1AuthStateRepository<'database> {
    #[must_use]
    pub const fn new(database: &'database D1Database) -> Self {
        Self {
            database,
            query_timeout_ms: DEFAULT_AUTH_QUERY_TIMEOUT_MS,
        }
    }

    #[must_use]
    pub const fn with_query_timeout_ms(database: &'database D1Database, timeout_ms: u64) -> Self {
        Self {
            database,
            query_timeout_ms: timeout_ms,
        }
    }

    fn statement(&self, sql: &str, bindings: &[JsValue]) -> AdapterResult<D1PreparedStatement> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| AdapterFailure::Unavailable)
    }

    async fn await_d1<T>(
        &self,
        future: impl Future<Output = worker::Result<T>> + Send,
    ) -> AdapterResult<T> {
        if self.query_timeout_ms == 0 {
            return Err(AdapterFailure::Timeout);
        }
        // A Workers isolate is single-threaded. `IntoSendFuture` is the
        // worker-rs boundary that allows this adapter to satisfy the portable
        // `AuthStateRepository` Send future contract without moving JS state
        // between threads.
        let deadline = Delay::from(Duration::from_millis(self.query_timeout_ms)).into_send();
        pin_mut!(future);
        pin_mut!(deadline);
        match select(future, deadline).await {
            Either::Left((result, _)) => result.map_err(|_| AdapterFailure::Unavailable),
            Either::Right(((), _)) => Err(AdapterFailure::Timeout),
        }
    }

    async fn settle_d1<T>(
        &self,
        future: impl Future<Output = worker::Result<T>> + Send,
    ) -> worker::Result<T> {
        // Mutations must settle. Dropping a JS/D1 promise at a local deadline
        // can report Timeout while the database later commits the write.
        future.await
    }

    fn mutation_error(error: &worker::Error) -> AdapterFailure {
        let message = error.to_string();
        if exact_d1_trigger_error(&message) {
            AdapterFailure::Conflict
        } else {
            AdapterFailure::Unavailable
        }
    }

    async fn rows<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> AdapterResult<Vec<T>> {
        let statement = self.statement(sql, bindings)?;
        let result = self.await_d1(statement.all().into_send()).await?;
        if !result.success() {
            return Err(AdapterFailure::Unavailable);
        }
        let values = result
            .results::<serde_json::Value>()
            .map_err(|_| AdapterFailure::Unavailable)?;
        values
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(|_| AdapterFailure::Corrupt))
            .collect()
    }

    async fn one<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> AdapterResult<Option<T>> {
        let rows = self.rows(sql, bindings).await?;
        if rows.len() > 1 {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(rows.into_iter().next())
    }

    async fn batch(&self, statements: Vec<D1PreparedStatement>) -> AdapterResult<()> {
        let results = self
            .settle_d1(self.database.batch(statements).into_send())
            .await
            .map_err(|error| Self::mutation_error(&error))?;
        if results.iter().all(worker::D1Result::success) {
            Ok(())
        } else {
            Err(AdapterFailure::Unavailable)
        }
    }

    async fn settled_rows<T: DeserializeOwned>(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> AdapterResult<Vec<T>> {
        let statement = self.statement(sql, bindings)?;
        let result = self
            .settle_d1(statement.all().into_send())
            .await
            .map_err(|_| AdapterFailure::Unavailable)?;
        if !result.success() {
            return Err(AdapterFailure::Unavailable);
        }
        result
            .results::<serde_json::Value>()
            .map_err(|_| AdapterFailure::Unavailable)?
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(|_| AdapterFailure::Corrupt))
            .collect()
    }

    async fn operation_receipt_matches(&self, receipt: &OperationReceipt) -> AdapterResult<bool> {
        let mut rows = self
            .settled_rows::<OperationRow>(
                OPERATION_BY_ID_SQL,
                &[JsValue::from_str(&receipt.operation_id)],
            )
            .await?;
        if rows.len() > 1 {
            return Err(AdapterFailure::Corrupt);
        }
        let Some(row) = rows.pop() else {
            return Ok(false);
        };
        if row.operation_id != receipt.operation_id
            || row.operation_kind != receipt.operation_kind
            || row.result_code != receipt.result_code
            || row.result_timestamp_ms != receipt.result_timestamp.map(TimestampMillis::get)
            || row.committed_at_ms != receipt.committed_at.get()
        {
            return Err(AdapterFailure::Corrupt);
        }
        if row.subject_id != receipt.subject_id
            || row.request_fingerprint != receipt.request_fingerprint
        {
            return Err(AdapterFailure::Invalid);
        }
        Ok(true)
    }

    async fn operation_result(
        &self,
        request: &OperationReceipt,
    ) -> AdapterResult<Option<StoredOperationResult>> {
        let rows = self
            .settled_rows::<OperationRow>(
                OPERATION_BY_ID_SQL,
                &[JsValue::from_str(&request.operation_id)],
            )
            .await?;
        let row = match rows.as_slice() {
            [row] => row,
            [] => return Ok(None),
            _ => return Err(AdapterFailure::Corrupt),
        };
        if row.operation_id != request.operation_id
            || row.operation_kind != request.operation_kind
            || row.committed_at_ms != request.committed_at.get()
        {
            return Err(AdapterFailure::Corrupt);
        }
        if row.subject_id != request.subject_id
            || row.request_fingerprint != request.request_fingerprint
        {
            return Err(AdapterFailure::Invalid);
        }
        Ok(Some(StoredOperationResult {
            code: row.result_code.clone(),
            timestamp: row.result_timestamp_ms.map(safe_timestamp).transpose()?,
        }))
    }

    async fn audit_operation_exists(
        &self,
        operation_id: &str,
        action: AuthAuditAction,
        outcome: AuthAuditOutcome,
        reason: AuthAuditReason,
    ) -> AdapterResult<bool> {
        let action = enum_name(action)?;
        let outcome = enum_name(outcome)?;
        let reason = enum_name(reason)?;
        let rows = self
            .settled_rows::<AuditOperationRow>(
                AUDIT_OPERATION_EXISTS_SQL,
                &[
                    JsValue::from_str(operation_id),
                    JsValue::from_str(&action),
                    JsValue::from_str(&outcome),
                    JsValue::from_str(&reason),
                ],
            )
            .await?;
        match rows.as_slice() {
            [AuditOperationRow { present: 1 }] => Ok(true),
            [AuditOperationRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn identity_provision_committed(
        &self,
        receipt: &OperationReceipt,
        grant: &IdentityProvisioningGrant,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let (version, digest) = digest_values(grant.identifier_digest());
        let rows = self
            .settled_rows::<ExistenceRow>(
                IDENTITY_PROVISION_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&grant.user_id().to_string()),
                    JsValue::from_f64(grant.identity_revision() as f64),
                    JsValue::from_str(&receipt.operation_id),
                    version,
                    digest,
                    JsValue::from_str(&grant.id().to_string()),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn session_issue_committed(
        &self,
        receipt: &OperationReceipt,
        command: &SessionIssueCommand,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let session = &command.session;
        let (token_version, token_digest) = digest_values(&session.token_digest);
        let (csrf_version, csrf_digest) = session
            .csrf_digest
            .as_ref()
            .map_or((JsValue::NULL, JsValue::NULL), digest_values);
        let (authority_kind, authority_id) = match &command.authority {
            SessionIssueAuthority::Verified(grant) => ("issuance", grant.id().to_string()),
            SessionIssueAuthority::ExistingSession(grant) => ("mutation", grant.id().to_string()),
        };
        let rows = self
            .settled_rows::<ExistenceRow>(
                SESSION_ISSUE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&session.id.to_string()),
                    JsValue::from_str(&session.family_id.to_string()),
                    JsValue::from_str(&session.user_id.to_string()),
                    JsValue::from_str(&enum_name(session.client_kind)?),
                    token_version,
                    token_digest,
                    csrf_version,
                    csrf_digest,
                    opt_string(
                        session
                            .browser_origin
                            .as_ref()
                            .map(ExactBrowserOrigin::as_str),
                    ),
                    JsValue::from_f64(session.issued_at.get() as f64),
                    JsValue::from_f64(session.idle_expires_at.get() as f64),
                    JsValue::from_f64(session.absolute_expires_at.get() as f64),
                    JsValue::from_str(&receipt.operation_id),
                    JsValue::from_str(authority_kind),
                    JsValue::from_str(&authority_id),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn session_authentication_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &SessionAuthenticationCommand,
    ) -> AdapterResult<Option<SessionPresentation>> {
        let Some(result) = self.operation_result(base_receipt).await? else {
            return Ok(None);
        };
        if result.timestamp.is_some() {
            return Err(AdapterFailure::Corrupt);
        }
        let result_code = match result.code.as_str() {
            "unknown" => "unknown",
            "revoked" => "revoked",
            "replay_family_revoked" => "replay_family_revoked",
            "expired" => "expired",
            "version_mismatch" => "version_mismatch",
            "boundary_invalid" => "boundary_invalid",
            "boundary_origin" => "boundary_origin",
            "boundary_fetch" => "boundary_fetch",
            "boundary_csrf" => "boundary_csrf",
            "authenticated" => "authenticated",
            "token_migrated" => "token_migrated",
            "csrf_migrated" => "csrf_migrated",
            _ => return Err(AdapterFailure::Corrupt),
        };
        let receipt = base_receipt.clone().with_result_code(result_code);
        if !self.operation_receipt_matches(&receipt).await? {
            return Err(AdapterFailure::Unavailable);
        }
        if result_code == "unknown" {
            if !self
                .audit_operation_exists(
                    &receipt.operation_id,
                    command.audit.action,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )
                .await?
            {
                return Err(AdapterFailure::Unavailable);
            }
            return Ok(Some(SessionPresentation::Unknown));
        }
        let credential = self
            .settled_session_by_credentials(&command.token_digests)
            .await?
            .ok_or(AdapterFailure::Unavailable)?;
        let context = SessionAuditContext::from(&credential.session.record);
        let denied = match result_code {
            "revoked" => Some((
                AuthAuditReason::Revoked,
                SessionPresentation::Revoked(context),
            )),
            "replay_family_revoked" => Some((
                AuthAuditReason::ReplayDetected,
                SessionPresentation::ReplayFamilyRevoked(context),
            )),
            "expired" => Some((
                AuthAuditReason::Expired,
                SessionPresentation::Expired(context),
            )),
            "version_mismatch" => Some((
                AuthAuditReason::SessionVersionMismatch,
                SessionPresentation::SessionVersionMismatch(context),
            )),
            "boundary_invalid" => Some((
                AuthAuditReason::InvalidCredential,
                SessionPresentation::BoundaryRejected(context, AuthAuditReason::InvalidCredential),
            )),
            "boundary_origin" => Some((
                AuthAuditReason::OriginMismatch,
                SessionPresentation::BoundaryRejected(context, AuthAuditReason::OriginMismatch),
            )),
            "boundary_fetch" => Some((
                AuthAuditReason::FetchMetadataMismatch,
                SessionPresentation::BoundaryRejected(
                    context,
                    AuthAuditReason::FetchMetadataMismatch,
                ),
            )),
            "boundary_csrf" => Some((
                AuthAuditReason::CsrfMismatch,
                SessionPresentation::BoundaryRejected(context, AuthAuditReason::CsrfMismatch),
            )),
            _ => None,
        };
        if let Some((reason, presentation)) = denied {
            if !self
                .audit_operation_exists(
                    &receipt.operation_id,
                    command.audit.action,
                    AuthAuditOutcome::Deny,
                    reason,
                )
                .await?
            {
                return Err(AdapterFailure::Unavailable);
            }
            return Ok(Some(presentation));
        }

        if !credential.session.user_active
            || credential.credential_state != CredentialState::Current
            || credential.session.record.evaluate(
                command.audit.occurred_at,
                credential.session.current_session_version,
            ) != AuthSessionDecision::Authenticated
            || credential.session.record.token_digest != *command.token_digests.active()
        {
            return Err(AdapterFailure::Unavailable);
        }
        let principal = self
            .settled_principal(credential.session.record.user_id)
            .await?
            .ok_or(AdapterFailure::Unavailable)?;
        let grant_id = command
            .browser_boundary
            .as_ref()
            .map(|_| {
                SessionMutationGrantId::parse(&stable_child_uuid(
                    &receipt.operation_id,
                    "mutation-grant",
                ))
                .map_err(|_| AdapterFailure::Corrupt)
            })
            .transpose()?;
        let (token_version, token_digest) = digest_values(command.token_digests.active());
        let expected_csrf = command
            .browser_boundary
            .as_ref()
            .map(|boundary| boundary.csrf_header_digests.active());
        let (csrf_version, csrf_digest) =
            expected_csrf.map_or((JsValue::NULL, JsValue::NULL), digest_values);
        let migration = match result_code {
            "authenticated" => "unchanged",
            "token_migrated" => "token_migrated",
            "csrf_migrated" => "csrf_migrated",
            _ => return Err(AdapterFailure::Corrupt),
        };
        let audit_reason = if result_code == "token_migrated" {
            AuthAuditReason::KeyVersionMigrated
        } else {
            AuthAuditReason::Authenticated
        };
        let rows = self
            .settled_rows::<ExistenceRow>(
                SESSION_AUTHENTICATION_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&credential.session.record.id.to_string()),
                    token_version,
                    token_digest,
                    csrf_version,
                    csrf_digest,
                    JsValue::from_str(migration),
                    JsValue::from_str(&receipt.operation_id),
                    opt_string(grant_id.map(|value| value.to_string()).as_deref()),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&enum_name(command.audit.action)?),
                    JsValue::from_str(&enum_name(audit_reason)?),
                ],
            )
            .await?;
        if !matches!(rows.as_slice(), [ExistenceRow { present: 1 }]) {
            return if matches!(rows.as_slice(), [ExistenceRow { present: 0 }]) {
                Err(AdapterFailure::Unavailable)
            } else {
                Err(AdapterFailure::Corrupt)
            };
        }
        let mutation_grant = grant_id.map(|grant_id| {
            SessionMutationGrant::from_repository(
                grant_id,
                credential.session.record.id,
                credential.session.record.user_id,
                credential.session.record.generation,
                command.token_digests.active().clone(),
            )
        });
        Ok(Some(SessionPresentation::Authenticated(
            AuthenticatedSessionPresentation::from_repository(
                credential.session.record.id,
                credential.session.record.client_kind,
                principal.snapshot,
                mutation_grant,
            ),
        )))
    }

    async fn commit_session_authentication(
        &self,
        statements: Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
        command: &SessionAuthenticationCommand,
    ) -> AdapterResult<SessionPresentation> {
        self.batch_mutation(statements, receipt).await?;
        self.session_authentication_committed(receipt, command)
            .await?
            .ok_or(AdapterFailure::Unavailable)
    }

    async fn session_rotation_committed(
        &self,
        receipt: &OperationReceipt,
        request: &SessionRotationRequest,
    ) -> AdapterResult<Option<AuthSessionRecord>> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(None);
        }
        let next_generation = request
            .grant
            .generation()
            .checked_add(1)
            .ok_or(AdapterFailure::Invalid)?;
        let (next_version, next_digest) = digest_values(&request.next_token_digest);
        let existing = self
            .settled_rows::<SessionRow>(
                SESSION_BY_ID_SQL,
                &[JsValue::from_str(&request.grant.session_id().to_string())],
            )
            .await?;
        let session = match existing.as_slice() {
            [row] => SessionState::decode(row.clone())?,
            [] => return Ok(None),
            _ => return Err(AdapterFailure::Corrupt),
        };
        let (csrf_version, csrf_digest) = if session.record.client_kind == AuthClientKind::Browser {
            digest_values(&request.next_csrf_digest)
        } else {
            (JsValue::NULL, JsValue::NULL)
        };
        let (old_version, old_digest) = digest_values(request.grant.token_digest());
        let rows = self
            .settled_rows::<ExistenceRow>(
                SESSION_ROTATION_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&request.grant.session_id().to_string()),
                    JsValue::from_f64(next_generation as f64),
                    next_version,
                    next_digest,
                    csrf_version,
                    csrf_digest,
                    JsValue::from_str(&receipt.operation_id),
                    old_version,
                    old_digest,
                    JsValue::from_str(&request.grant.id().to_string()),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }]
                if session.user_active
                    && session.record.generation == next_generation
                    && session.record.token_digest == request.next_token_digest
                    && (session.record.client_kind != AuthClientKind::Browser
                        || session.record.csrf_digest.as_ref()
                            == Some(&request.next_csrf_digest)) =>
            {
                Ok(Some(session.record))
            }
            [ExistenceRow { present: 0 }] => Ok(None),
            [_] => Err(AdapterFailure::Unavailable),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn api_key_issue_committed(
        &self,
        receipt: &OperationReceipt,
        command: &ApiKeyIssueCommand,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let record = &command.record;
        let (version, digest) = digest_values(&record.key_digest);
        let scopes = serde_json::to_string(&record.scopes).map_err(|_| AdapterFailure::Invalid)?;
        let rows = self
            .settled_rows::<ExistenceRow>(
                API_KEY_ISSUE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&record.id.to_string()),
                    JsValue::from_str(&record.owner_id.to_string()),
                    JsValue::from_str(&record.tenant_id.to_string()),
                    version,
                    digest,
                    JsValue::from_str(&scopes),
                    JsValue::from_f64(record.created_at.get() as f64),
                    opt_i64(record.expires_at.map(TimestampMillis::get)),
                    JsValue::from_str(&receipt.operation_id),
                    JsValue::from_str(&command.grant.id().to_string()),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn api_key_authentication_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &ApiKeyAuthenticationCommand,
    ) -> AdapterResult<Option<ApiKeyAuthenticationOutcome>> {
        let Some(result) = self.operation_result(base_receipt).await? else {
            return Ok(None);
        };
        let (receipt, outcome_kind) = match result.code.as_str() {
            "rate_limited" => {
                let retry_at = result.timestamp.ok_or(AdapterFailure::Corrupt)?;
                (
                    base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at),
                    ApiKeyAuthenticationReplay::RateLimited(retry_at),
                )
            }
            "rejected_invalid" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_invalid"),
                ApiKeyAuthenticationReplay::Rejected,
            ),
            "authenticated" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("authenticated"),
                ApiKeyAuthenticationReplay::Authenticated(false),
            ),
            "key_migrated" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("key_migrated"),
                ApiKeyAuthenticationReplay::Authenticated(true),
            ),
            _ => return Err(AdapterFailure::Corrupt),
        };
        if !self.operation_receipt_matches(&receipt).await? {
            return Err(AdapterFailure::Unavailable);
        }
        match outcome_kind {
            ApiKeyAuthenticationReplay::RateLimited(retry_at) => {
                if !self
                    .audit_operation_exists(
                        &receipt.operation_id,
                        command.audit.action,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .await?
                {
                    return Err(AdapterFailure::Unavailable);
                }
                Ok(Some(ApiKeyAuthenticationOutcome::RateLimited { retry_at }))
            }
            ApiKeyAuthenticationReplay::Rejected => {
                if !self
                    .audit_operation_exists(
                        &receipt.operation_id,
                        command.audit.action,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .await?
                {
                    return Err(AdapterFailure::Unavailable);
                }
                Ok(Some(ApiKeyAuthenticationOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                )))
            }
            ApiKeyAuthenticationReplay::Authenticated(migrated) => {
                let (key, _revision, _) = self
                    .settled_api_key_by_candidates(&command.key_digests)
                    .await?
                    .ok_or(AdapterFailure::Unavailable)?;
                if key.key_digest != *command.key_digests.active()
                    || key.tenant_id != command.tenant_id
                    || !key.allows(command.required_scope, command.audit.occurred_at)
                {
                    return Err(AdapterFailure::Unavailable);
                }
                let principal = self
                    .settled_principal(key.owner_id)
                    .await?
                    .ok_or(AdapterFailure::Unavailable)?;
                if Self::tenant_membership(&principal, key.tenant_id).is_none() {
                    return Err(AdapterFailure::Unavailable);
                }
                let (version, digest) = digest_values(command.key_digests.active());
                let audit_reason = if migrated {
                    AuthAuditReason::KeyVersionMigrated
                } else {
                    AuthAuditReason::Authenticated
                };
                let rows = self
                    .settled_rows::<ExistenceRow>(
                        API_KEY_AUTHENTICATION_POSTCONDITION_SQL,
                        &[
                            JsValue::from_str(&key.id.to_string()),
                            JsValue::from_str(&command.tenant_id.to_string()),
                            version,
                            digest,
                            JsValue::from_str(if migrated { "migrated" } else { "unchanged" }),
                            JsValue::from_str(&receipt.operation_id),
                            JsValue::from_str(&enum_name(audit_reason)?),
                        ],
                    )
                    .await?;
                if !matches!(rows.as_slice(), [ExistenceRow { present: 1 }]) {
                    return if matches!(rows.as_slice(), [ExistenceRow { present: 0 }]) {
                        Err(AdapterFailure::Unavailable)
                    } else {
                        Err(AdapterFailure::Corrupt)
                    };
                }
                Ok(Some(ApiKeyAuthenticationOutcome::Authenticated(
                    principal.snapshot,
                )))
            }
        }
    }

    async fn commit_api_key_authentication(
        &self,
        statements: Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
        command: &ApiKeyAuthenticationCommand,
    ) -> AdapterResult<ApiKeyAuthenticationOutcome> {
        self.batch_mutation(statements, receipt).await?;
        self.api_key_authentication_committed(receipt, command)
            .await?
            .ok_or(AdapterFailure::Unavailable)
    }

    async fn api_key_revoke_committed(
        &self,
        receipt: &OperationReceipt,
        command: &ApiKeyRevokeCommand,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let rows = self
            .settled_rows::<ExistenceRow>(
                API_KEY_REVOKE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&command.key_id.to_string()),
                    JsValue::from_str(&receipt.operation_id),
                    JsValue::from_str(&command.grant.id().to_string()),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn delivery_acknowledged(
        &self,
        receipt: &OperationReceipt,
        claim: &AuthDeliveryClaim,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let rows = self
            .settled_rows::<DeliveryAckTombstoneRow>(
                DELIVERY_ACK_TOMBSTONE_BY_OPERATION_SQL,
                &[JsValue::from_str(&receipt.operation_id)],
            )
            .await?;
        let exact = match rows.as_slice() {
            [row] => {
                row.operation_id == receipt.operation_id
                    && row.delivery_id == claim.delivery_id().to_string()
                    && row.lease_id == claim.lease_id().to_string()
                    && row.attempt == i64::from(claim.attempt())
                    && row.lease_expires_at_ms == claim.lease_expires_at().get()
                    && safe_timestamp(row.acknowledged_at_ms)? < claim.lease_expires_at()
            }
            [] => false,
            _ => return Err(AdapterFailure::Corrupt),
        };
        if !exact {
            return Ok(false);
        }
        Ok(self
            .settled_rows::<DeliveryRow>(
                DELIVERY_BY_ID_SQL,
                &[JsValue::from_str(&claim.delivery_id().to_string())],
            )
            .await?
            .is_empty())
    }

    async fn delivery_claim_committed(
        &self,
        receipt: &OperationReceipt,
        requested_lease_expires_at: TimestampMillis,
    ) -> AdapterResult<Option<AuthDeliveryClaim>> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(None);
        }
        let rows = self
            .settled_rows::<DeliveryRow>(
                DELIVERY_CLAIM_BY_OPERATION_SQL,
                &[JsValue::from_str(&receipt.operation_id)],
            )
            .await?;
        let row = match rows.as_slice() {
            [row] => row,
            [] => return Ok(None),
            _ => return Err(AdapterFailure::Corrupt),
        };
        row.validated()?;
        let expected_lease_id =
            AuthDeliveryLeaseId::parse(&stable_child_uuid(&receipt.operation_id, "delivery-lease"))
                .map_err(|_| AdapterFailure::Corrupt)?;
        let expires_at = safe_timestamp(row.expires_at_ms)?;
        let expected_lease_expires_at = requested_lease_expires_at.min(expires_at);
        let attempt = u16::try_from(row.attempt)
            .ok()
            .filter(|value| (1..=MAX_AUTH_DELIVERY_ATTEMPTS).contains(value))
            .ok_or(AdapterFailure::Corrupt)?;
        if row.lease_id.as_deref() != Some(&expected_lease_id.to_string())
            || row.lease_expires_at_ms != Some(expected_lease_expires_at.get())
        {
            return Err(AdapterFailure::Unavailable);
        }
        Ok(Some(AuthDeliveryClaim::from_repository(
            expected_lease_id,
            row.envelope()?,
            expected_lease_expires_at,
            attempt,
        )))
    }

    async fn delivery_retry_committed(
        &self,
        base_receipt: &OperationReceipt,
        claim: &AuthDeliveryClaim,
    ) -> AdapterResult<Option<AuthDeliveryRetryOutcome>> {
        let Some(result) = self.operation_result(base_receipt).await? else {
            return Ok(None);
        };
        match result.code.as_str() {
            "scheduled" => {
                let retry_at = result.timestamp.ok_or(AdapterFailure::Corrupt)?;
                let receipt = base_receipt
                    .clone()
                    .with_result_code("scheduled")
                    .with_result_timestamp(retry_at);
                if !self.operation_receipt_matches(&receipt).await? {
                    return Err(AdapterFailure::Unavailable);
                }
                let rows = self
                    .settled_rows::<ExistenceRow>(
                        DELIVERY_RETRY_SCHEDULED_POSTCONDITION_SQL,
                        &[
                            JsValue::from_str(&claim.delivery_id().to_string()),
                            JsValue::from_f64(f64::from(claim.attempt())),
                            JsValue::from_f64(retry_at.get() as f64),
                            JsValue::from_str(&receipt.operation_id),
                        ],
                    )
                    .await?;
                match rows.as_slice() {
                    [ExistenceRow { present: 1 }] => Ok(Some(AuthDeliveryRetryOutcome::Scheduled)),
                    [ExistenceRow { present: 0 }] => Err(AdapterFailure::Unavailable),
                    _ => Err(AdapterFailure::Corrupt),
                }
            }
            "exhausted" if result.timestamp.is_none() => {
                let receipt = base_receipt.clone().with_result_code("exhausted");
                if self.delivery_acknowledged(&receipt, claim).await? {
                    Ok(Some(AuthDeliveryRetryOutcome::Exhausted))
                } else {
                    Err(AdapterFailure::Unavailable)
                }
            }
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn session_revoke_committed(
        &self,
        receipt: &OperationReceipt,
        command: &SessionRevokeCommand,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let rows = self
            .settled_rows::<ExistenceRow>(
                SESSION_REVOKE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&command.grant.session_id().to_string()),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&enum_name(command.reason)?),
                    JsValue::from_str(&receipt.operation_id),
                    JsValue::from_str(&command.grant.id().to_string()),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn logout_all_committed(
        &self,
        receipt: &OperationReceipt,
        command: &SessionRevokeCommand,
    ) -> AdapterResult<Option<(u64, u64)>> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(None);
        }
        let rows = self
            .settled_rows::<LogoutAllResultRow>(
                LOGOUT_ALL_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&command.grant.user_id().to_string()),
                    JsValue::from_str(&receipt.operation_id),
                    JsValue::from_str(&command.grant.id().to_string()),
                ],
            )
            .await?;
        match rows.as_slice() {
            [row] => Ok(Some((
                safe_revision(row.new_session_version)?,
                safe_revision(row.revoked_sessions)?,
            ))),
            [] => Ok(None),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn verification_issue_committed(
        &self,
        receipt: &OperationReceipt,
        command: &VerificationIssueCommand,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let grant_id = command
            .initiator_grant
            .as_ref()
            .map(|grant| grant.id().to_string());
        let (identifier_version, identifier_digest) =
            digest_values(command.identifier_digests.active());
        let (secret_version, secret_digest) = digest_values(&command.secret_digest);
        let rows = self
            .settled_rows::<ExistenceRow>(
                VERIFICATION_ISSUE_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&command.sealed_delivery.id.to_string()),
                    JsValue::from_str(&receipt.operation_id),
                    opt_string(grant_id.as_deref()),
                    identifier_version,
                    identifier_digest,
                    secret_version,
                    secret_digest,
                    JsValue::from_str(&enum_name(command.purpose)?),
                    JsValue::from_str(&enum_name(command.channel)?),
                    JsValue::from_f64(f64::from(command.max_attempts)),
                    JsValue::from_f64(command.sealed_delivery.created_at.get() as f64),
                    JsValue::from_f64(command.expires_at.get() as f64),
                    JsValue::from_str(&bytes_to_hex(command.sealed_delivery.sealed_payload())),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn verification_issue_outcome_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &VerificationIssueCommand,
    ) -> AdapterResult<Option<VerificationIssueAtomicOutcome>> {
        let Some(result) = self.operation_result(base_receipt).await? else {
            return Ok(None);
        };
        match result.code.as_str() {
            "accepted" if result.timestamp.is_none() => {
                let receipt = base_receipt.clone().with_result_code("accepted");
                if self.verification_issue_committed(&receipt, command).await? {
                    Ok(Some(VerificationIssueAtomicOutcome::Accepted))
                } else {
                    Err(AdapterFailure::Unavailable)
                }
            }
            "rejected_invalid" if result.timestamp.is_none() => {
                let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                if self.operation_receipt_matches(&receipt).await?
                    && self
                        .audit_operation_exists(
                            &receipt.operation_id,
                            command.audit.action,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        )
                        .await?
                {
                    Ok(Some(VerificationIssueAtomicOutcome::Rejected(
                        AuthAuditReason::InvalidCredential,
                    )))
                } else {
                    Err(AdapterFailure::Unavailable)
                }
            }
            "rate_limited" => {
                let retry_at = result.timestamp.ok_or(AdapterFailure::Corrupt)?;
                let receipt = base_receipt
                    .clone()
                    .with_result_code("rate_limited")
                    .with_result_timestamp(retry_at);
                if self.operation_receipt_matches(&receipt).await?
                    && self
                        .audit_operation_exists(
                            &receipt.operation_id,
                            command.audit.action,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::RateLimited,
                        )
                        .await?
                {
                    Ok(Some(VerificationIssueAtomicOutcome::RateLimited {
                        retry_at,
                    }))
                } else {
                    Err(AdapterFailure::Unavailable)
                }
            }
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn verification_materialization_committed(
        &self,
        receipt: &OperationReceipt,
        delivery_id: AuthDeliveryId,
        challenge_id: Option<VerificationId>,
        suppress: Option<bool>,
    ) -> AdapterResult<bool> {
        if !self.operation_receipt_matches(receipt).await? {
            return Ok(false);
        }
        let rows = self
            .settled_rows::<ExistenceRow>(
                VERIFICATION_MATERIALIZATION_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&delivery_id.to_string()),
                    JsValue::from_str(receipt.result_code),
                    opt_string(challenge_id.map(|value| value.to_string()).as_deref()),
                    JsValue::from_str(&receipt.operation_id),
                    suppress.map_or(JsValue::NULL, |value| {
                        JsValue::from_f64(if value { 1.0 } else { 0.0 })
                    }),
                ],
            )
            .await?;
        match rows.as_slice() {
            [ExistenceRow { present: 1 }] => Ok(true),
            [ExistenceRow { present: 0 }] => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    fn push_operation_receipt(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OPERATION_INSERT_SQL,
            &[
                JsValue::from_str(&receipt.operation_id),
                JsValue::from_str(receipt.operation_kind),
                JsValue::from_str(&receipt.subject_id),
                JsValue::from_str(receipt.result_code),
                opt_i64(receipt.result_timestamp.map(TimestampMillis::get)),
                JsValue::from_str(&receipt.request_fingerprint),
                JsValue::from_f64(receipt.committed_at.get() as f64),
            ],
        )
    }

    async fn batch_mutation(
        &self,
        mut statements: Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
    ) -> AdapterResult<()> {
        self.push_operation_receipt(&mut statements, receipt)?;
        match self.batch(statements).await {
            Ok(()) => Ok(()),
            Err(failure) => match self.operation_receipt_matches(receipt).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(failure),
                Err(AdapterFailure::Invalid) => Err(AdapterFailure::Invalid),
                Err(_) => Err(AdapterFailure::Unavailable),
            },
        }
    }

    fn push_statement(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        sql: &str,
        bindings: &[JsValue],
    ) -> AdapterResult<()> {
        statements.push(self.statement(sql, bindings)?);
        Ok(())
    }

    fn push_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        label: &str,
        expected_changes: usize,
    ) -> AdapterResult<()> {
        let id = format!("{operation_id}:{label}");
        self.push_statement(
            statements,
            ASSERT_CHANGES_SQL,
            &[
                JsValue::from_str(&id),
                JsValue::from_f64(expected_changes as f64),
            ],
        )
    }

    fn push_cleanup(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            ASSERTIONS_CLEANUP_SQL,
            &[JsValue::from_str(&format!("{operation_id}:%"))],
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the audit tuple is kept explicit at every atomic D1 write callsite"
    )]
    fn push_audit(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        audit: &DecisionAudit,
        context: Option<SessionAuditContext>,
        user_id: Option<UserId>,
        outcome: AuthAuditOutcome,
        reason: AuthAuditReason,
    ) -> AdapterResult<()> {
        let audit_id = AuthAuditEventId::new().to_string();
        let context_user = context.map(|value| value.user_id);
        let user_id = user_id.or(context_user).map(|value| value.to_string());
        let session_id = context.map(|value| value.session_id.to_string());
        let client_kind = context
            .map(|value| enum_name(value.client_kind))
            .transpose()?;
        self.push_statement(
            statements,
            AUDIT_INSERT_SQL,
            &[
                JsValue::from_str(&audit_id),
                JsValue::from_str(&audit.correlation_id.to_string()),
                opt_string(user_id.as_deref()),
                opt_string(session_id.as_deref()),
                opt_string(client_kind.as_deref()),
                JsValue::from_str(&enum_name(audit.action)?),
                JsValue::from_str(&enum_name(outcome)?),
                JsValue::from_str(&enum_name(reason)?),
                JsValue::from_f64(audit.occurred_at.get() as f64),
                JsValue::from_str(operation_id),
            ],
        )
    }

    async fn principal(&self, user_id: UserId) -> AdapterResult<Option<PrincipalState>> {
        let rows = self
            .rows::<PrincipalRow>(
                PRINCIPAL_BY_USER_SQL,
                &[JsValue::from_str(&user_id.to_string())],
            )
            .await?;
        Self::decode_principal(user_id, rows)
    }

    async fn settled_principal(&self, user_id: UserId) -> AdapterResult<Option<PrincipalState>> {
        let rows = self
            .settled_rows::<PrincipalRow>(
                PRINCIPAL_BY_USER_SQL,
                &[JsValue::from_str(&user_id.to_string())],
            )
            .await?;
        Self::decode_principal(user_id, rows)
    }

    fn decode_principal(
        user_id: UserId,
        rows: Vec<PrincipalRow>,
    ) -> AdapterResult<Option<PrincipalState>> {
        if rows.is_empty() {
            return Ok(None);
        }
        if rows.len() > MAX_PRINCIPAL_GRANTS {
            return Err(AdapterFailure::Corrupt);
        }
        let first_user_id = rows[0].user_id.clone();
        if first_user_id != user_id.to_string() {
            return Err(AdapterFailure::Corrupt);
        }
        let identity_revision = safe_revision(rows[0].identity_revision)?;
        if identity_revision == 0 {
            return Err(AdapterFailure::Corrupt);
        }
        let session_version = safe_revision(rows[0].session_version)?;
        let row_revision = safe_revision(rows[0].identity_row_revision)?;
        let mut tenant_grants = Vec::new();
        let mut authority_fences = Vec::new();
        let mut seen = BTreeSet::new();
        for row in rows {
            if row.user_id != first_user_id
                || safe_revision(row.identity_revision)? != identity_revision
                || safe_revision(row.session_version)? != session_version
                || safe_revision(row.identity_row_revision)? != row_revision
            {
                return Err(AdapterFailure::Corrupt);
            }
            match (
                row.organization_id,
                row.role,
                row.member_revision,
                row.organization_revision,
            ) {
                (Some(tenant), Some(role), Some(member_revision), Some(organization_revision)) => {
                    let tenant_id =
                        TenantId::parse(&tenant).map_err(|_| AdapterFailure::Corrupt)?;
                    if !seen.insert(tenant_id.to_string()) {
                        return Err(AdapterFailure::Corrupt);
                    }
                    let role = parse_enum::<OrganizationRole>(&role)?;
                    tenant_grants.push(TenantGrant { tenant_id, role });
                    authority_fences.push(TenantAuthorityFence {
                        tenant_id,
                        role,
                        member_revision: safe_revision(member_revision)?,
                        organization_revision: safe_revision(organization_revision)?,
                    });
                }
                (None, None, None, None) => {}
                _ => return Err(AdapterFailure::Corrupt),
            }
        }
        Ok(Some(PrincipalState {
            snapshot: PrincipalSnapshot {
                user_id,
                identity_revision,
                tenant_grants,
            },
            session_version,
            row_revision,
            authority_fences,
        }))
    }

    async fn identifier_owner(
        &self,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<UserId>> {
        let json = candidates_json(candidates)?;
        let rows = self
            .rows::<IdentifierOwnerRow>(IDENTIFIER_OWNER_SQL, &[JsValue::from_str(&json)])
            .await?;
        if rows.len() > 1 {
            let owners = rows
                .iter()
                .map(|row| row.user_id.as_str())
                .collect::<BTreeSet<_>>();
            if owners.len() > 1 {
                return Err(AdapterFailure::Corrupt);
            }
        }
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        versioned_digest(row.key_version, &row.digest)?;
        UserId::parse(&row.user_id)
            .map(Some)
            .map_err(|_| AdapterFailure::Corrupt)
    }

    async fn oauth_flow_by_state(
        &self,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<StoredOAuthFlow>> {
        let rows = self
            .rows::<OAuthFlowRow>(
                OAUTH_FLOW_BY_STATE_SQL,
                &[JsValue::from_str(&candidates_json(candidates)?)],
            )
            .await?;
        match rows.len() {
            0 => Ok(None),
            1 => rows
                .into_iter()
                .next()
                .ok_or(AdapterFailure::Corrupt)?
                .decode()
                .map(Some),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn oauth_flow_id_exists(
        &self,
        id: OAuthFlowId,
        now: TimestampMillis,
    ) -> AdapterResult<bool> {
        match self
            .one::<ExistenceRow>(
                OAUTH_FLOW_ID_EXISTS_SQL,
                &[
                    JsValue::from_str(&id.to_string()),
                    JsValue::from_f64(now.get() as f64),
                ],
            )
            .await?
        {
            Some(ExistenceRow { present: 1 }) => Ok(true),
            Some(ExistenceRow { present: 0 }) => Ok(false),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn oauth_reservation(
        &self,
        id: OAuthExchangeReservationId,
    ) -> AdapterResult<Option<StoredOAuthReservation>> {
        self.one::<OAuthReservationRow>(
            OAUTH_RESERVATION_BY_ID_SQL,
            &[JsValue::from_str(&id.to_string())],
        )
        .await?
        .map(OAuthReservationRow::decode)
        .transpose()
    }

    async fn oauth_external_account(
        &self,
        provider: OAuthProvider,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<StoredExternalAccount>> {
        let rows = self
            .rows::<OAuthExternalAccountRow>(
                OAUTH_EXTERNAL_ACCOUNT_BY_SUBJECT_SQL,
                &[
                    JsValue::from_str(&enum_name(provider)?),
                    JsValue::from_str(&candidates_json(candidates)?),
                ],
            )
            .await?;
        let mut decoded = rows
            .into_iter()
            .map(OAuthExternalAccountRow::decode)
            .collect::<AdapterResult<Vec<_>>>()?;
        if decoded
            .first()
            .is_some_and(|first| decoded.iter().any(|row| row.user_id != first.user_id))
        {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(decoded.drain(..).next())
    }

    async fn settled_oauth_external_account(
        &self,
        provider: OAuthProvider,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<StoredExternalAccount>> {
        let rows = self
            .settled_rows::<OAuthExternalAccountRow>(
                OAUTH_EXTERNAL_ACCOUNT_BY_SUBJECT_SQL,
                &[
                    JsValue::from_str(&enum_name(provider)?),
                    JsValue::from_str(&candidates_json(candidates)?),
                ],
            )
            .await?;
        let mut decoded = rows
            .into_iter()
            .map(OAuthExternalAccountRow::decode)
            .collect::<AdapterResult<Vec<_>>>()?;
        if decoded
            .first()
            .is_some_and(|first| decoded.iter().any(|row| row.user_id != first.user_id))
        {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(decoded.drain(..).next())
    }

    async fn oauth_pending_capacity_retry(
        &self,
        now: TimestampMillis,
    ) -> AdapterResult<Option<TimestampMillis>> {
        #[derive(Deserialize)]
        struct PendingOAuthCapacityRow {
            pending_count: i64,
            retry_at_ms: Option<i64>,
        }
        let row = self
            .one::<PendingOAuthCapacityRow>(
                "SELECT COUNT(*) pending_count,MIN(expires_at_ms) retry_at_ms \
                   FROM auth_oauth_flows_v2 WHERE expires_at_ms>?1",
                &[JsValue::from_f64(now.get() as f64)],
            )
            .await?
            .ok_or(AdapterFailure::Corrupt)?;
        let count = usize::try_from(row.pending_count).map_err(|_| AdapterFailure::Corrupt)?;
        if count < MAX_PENDING_OAUTH_FLOWS {
            Ok(None)
        } else {
            row.retry_at_ms
                .map(safe_timestamp)
                .transpose()?
                .ok_or(AdapterFailure::Corrupt)
                .map(Some)
        }
    }

    async fn oauth_operation_result(
        &self,
        request: &OperationReceipt,
    ) -> AdapterResult<Option<StoredOperationResult>> {
        let rows = self
            .settled_rows::<OperationRow>(
                OAUTH_OPERATION_BY_ID_SQL,
                &[JsValue::from_str(&request.operation_id)],
            )
            .await?;
        let row = match rows.as_slice() {
            [row] => row,
            [] => return Ok(None),
            _ => return Err(AdapterFailure::Corrupt),
        };
        if row.operation_id != request.operation_id
            || row.operation_kind != request.operation_kind
            || row.committed_at_ms != request.committed_at.get()
        {
            return Err(AdapterFailure::Corrupt);
        }
        if row.subject_id != request.subject_id
            || row.request_fingerprint != request.request_fingerprint
        {
            return Err(AdapterFailure::Invalid);
        }
        Ok(Some(StoredOperationResult {
            code: row.result_code.clone(),
            timestamp: row.result_timestamp_ms.map(safe_timestamp).transpose()?,
        }))
    }

    async fn oauth_operation_receipt_matches(
        &self,
        receipt: &OperationReceipt,
    ) -> AdapterResult<bool> {
        let Some(result) = self.oauth_operation_result(receipt).await? else {
            return Ok(false);
        };
        if result.code != receipt.result_code || result.timestamp != receipt.result_timestamp {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(true)
    }

    fn push_oauth_operation_receipt(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OAUTH_OPERATION_INSERT_SQL,
            &[
                JsValue::from_str(&receipt.operation_id),
                JsValue::from_str(receipt.operation_kind),
                JsValue::from_str(&receipt.subject_id),
                JsValue::from_str(receipt.result_code),
                opt_i64(receipt.result_timestamp.map(TimestampMillis::get)),
                JsValue::from_str(&receipt.request_fingerprint),
                JsValue::from_f64(receipt.committed_at.get() as f64),
            ],
        )
    }

    async fn oauth_batch_mutation(
        &self,
        mut statements: Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
    ) -> AdapterResult<()> {
        self.push_oauth_operation_receipt(&mut statements, receipt)?;
        match self.batch(statements).await {
            Ok(()) => Ok(()),
            Err(failure) => match self.oauth_operation_receipt_matches(receipt).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(failure),
                Err(AdapterFailure::Invalid) => Err(AdapterFailure::Invalid),
                Err(_) => Err(AdapterFailure::Unavailable),
            },
        }
    }

    async fn commit_oauth_decision(
        &self,
        mut statements: Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
        audit: &DecisionAudit,
        user_id: Option<UserId>,
        outcome: AuthAuditOutcome,
        reason: AuthAuditReason,
    ) -> AdapterResult<()> {
        self.push_audit(
            &mut statements,
            &receipt.operation_id,
            audit,
            None,
            user_id,
            outcome,
            reason,
        )?;
        self.push_cleanup(&mut statements, &receipt.operation_id)?;
        self.oauth_batch_mutation(statements, receipt).await
    }

    fn push_oauth_flow_current_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        flow: &StoredOAuthFlow,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OAUTH_FLOW_CURRENT_ASSERT_SQL,
            &[
                JsValue::from_str(&flow.id.to_string()),
                JsValue::from_f64(flow.revision as f64),
                opt_i64(flow.consumed_at.map(TimestampMillis::get)),
                JsValue::from_str(&format!("{operation_id}:oauth_flow_current")),
            ],
        )
    }

    fn push_oauth_reservation_current_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        reservation: &StoredOAuthReservation,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OAUTH_RESERVATION_CURRENT_ASSERT_SQL,
            &[
                JsValue::from_str(&reservation.id.to_string()),
                JsValue::from_f64(reservation.revision as f64),
                opt_i64(reservation.consumed_at.map(TimestampMillis::get)),
                JsValue::from_str(&format!("{operation_id}:oauth_reservation_current")),
            ],
        )
    }

    fn push_oauth_continuation_invalid_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        binding: SessionContinuationBinding,
        now: TimestampMillis,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OAUTH_CONTINUATION_INVALID_ASSERT_SQL,
            &[
                JsValue::from_str(&binding.session_id.to_string()),
                JsValue::from_str(&binding.user_id.to_string()),
                JsValue::from_f64(binding.generation as f64),
                JsValue::from_f64(now.get() as f64),
                JsValue::from_str(&format!("{operation_id}:continuation_invalid")),
            ],
        )
    }

    fn push_oauth_mutation_grant_invalid_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        grant: &SessionMutationGrant,
        now: TimestampMillis,
    ) -> AdapterResult<()> {
        let (version, digest) = digest_values(grant.token_digest());
        self.push_statement(
            statements,
            OAUTH_MUTATION_GRANT_INVALID_ASSERT_SQL,
            &[
                JsValue::from_str(&grant.id().to_string()),
                JsValue::from_str(&grant.session_id().to_string()),
                JsValue::from_str(&grant.user_id().to_string()),
                JsValue::from_f64(grant.generation() as f64),
                version,
                digest,
                JsValue::from_f64(now.get() as f64),
                JsValue::from_str(&format!("{operation_id}:oauth_grant_invalid")),
            ],
        )
    }

    fn push_oauth_external_account_snapshot_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        provider: OAuthProvider,
        candidates: &SecretDigestCandidates,
        expected_user_id: Option<UserId>,
    ) -> AdapterResult<()> {
        let expected_user_id = expected_user_id.map(|value| value.to_string());
        self.push_statement(
            statements,
            OAUTH_EXTERNAL_ACCOUNT_SNAPSHOT_ASSERT_SQL,
            &[
                JsValue::from_str(&enum_name(provider)?),
                JsValue::from_str(&candidates_json(candidates)?),
                opt_string(expected_user_id.as_deref()),
                JsValue::from_str(&format!("{operation_id}:oauth_subject_snapshot")),
            ],
        )
    }

    fn push_oauth_identifier_snapshot_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        candidates: &SecretDigestCandidates,
        expected_user_id: Option<UserId>,
    ) -> AdapterResult<()> {
        let expected_user_id = expected_user_id.map(|value| value.to_string());
        self.push_statement(
            statements,
            OAUTH_IDENTIFIER_SNAPSHOT_ASSERT_SQL,
            &[
                JsValue::from_str(&candidates_json(candidates)?),
                opt_string(expected_user_id.as_deref()),
                JsValue::from_str(&format!("{operation_id}:oauth_identifier_snapshot")),
            ],
        )
    }

    fn push_oauth_external_account_authority_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        provider: OAuthProvider,
        candidates: &SecretDigestCandidates,
        user_id: UserId,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OAUTH_EXTERNAL_ACCOUNT_AUTHORITY_ASSERT_SQL,
            &[
                JsValue::from_str(&enum_name(provider)?),
                JsValue::from_str(&candidates_json(candidates)?),
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_str(&format!("{operation_id}:oauth_subject_authority")),
            ],
        )
    }

    fn push_oauth_identifier_authority_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        candidates: &SecretDigestCandidates,
        user_id: UserId,
        allow_absent: bool,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            OAUTH_IDENTIFIER_AUTHORITY_ASSERT_SQL,
            &[
                JsValue::from_str(&candidates_json(candidates)?),
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_f64(if allow_absent { 1.0 } else { 0.0 }),
                JsValue::from_str(&format!("{operation_id}:oauth_identifier_authority")),
            ],
        )
    }

    fn push_oauth_external_account_write(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        provider: OAuthProvider,
        candidates: &SecretDigestCandidates,
        user_id: UserId,
        now: TimestampMillis,
    ) -> AdapterResult<()> {
        let provider = enum_name(provider)?;
        let candidates_json = candidates_json(candidates)?;
        let active = candidates.active();
        let (active_version, active_digest) = digest_values(active);
        self.push_statement(
            statements,
            OAUTH_EXTERNAL_ACCOUNT_DELETE_FALLBACKS_SQL,
            &[
                JsValue::from_str(&provider),
                JsValue::from_str(&user_id.to_string()),
                active_version.clone(),
                active_digest.clone(),
                JsValue::from_str(&candidates_json),
            ],
        )?;
        self.push_statement(
            statements,
            OAUTH_EXTERNAL_ACCOUNT_UPSERT_SQL,
            &[
                JsValue::from_str(&provider),
                active_version.clone(),
                active_digest.clone(),
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_f64(now.get() as f64),
                JsValue::from_str(operation_id),
            ],
        )?;
        self.push_assertion(statements, operation_id, "oauth_subject_upsert", 1)?;
        self.push_statement(
            statements,
            OAUTH_EXTERNAL_ACCOUNT_POSTCONDITION_SQL,
            &[
                JsValue::from_str(&provider),
                active_version,
                active_digest,
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_str(operation_id),
                JsValue::from_str(&candidates_json),
                JsValue::from_str(&format!("{operation_id}:oauth_subject_postcondition")),
            ],
        )
    }

    async fn oauth_begin_outcome_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &OAuthBeginCommand,
    ) -> AdapterResult<Option<OAuthBeginOutcome>> {
        let Some(result) = self.oauth_operation_result(base_receipt).await? else {
            return Ok(None);
        };
        let (receipt, outcome, audit_outcome, reason) = match result.code.as_str() {
            "started" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("started"),
                OAuthBeginOutcome::Started,
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            ),
            "rejected_invalid" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_invalid"),
                OAuthBeginOutcome::Rejected(AuthAuditReason::InvalidCredential),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            ),
            "rate_limited" => {
                let retry_at = result.timestamp.ok_or(AdapterFailure::Corrupt)?;
                (
                    base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at),
                    OAuthBeginOutcome::RateLimited { retry_at },
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                )
            }
            _ => return Err(AdapterFailure::Corrupt),
        };
        if !self.oauth_operation_receipt_matches(&receipt).await?
            || !self
                .audit_operation_exists(
                    &receipt.operation_id,
                    command.audit.action,
                    audit_outcome,
                    reason,
                )
                .await?
        {
            return Err(AdapterFailure::Unavailable);
        }
        Ok(Some(outcome))
    }

    async fn oauth_preflight_outcome_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &OAuthPreflightCommand,
    ) -> AdapterResult<Option<OAuthPreflightOutcome>> {
        let Some(result) = self.oauth_operation_result(base_receipt).await? else {
            return Ok(None);
        };
        let (receipt, denied, audit_outcome, reason) = match result.code.as_str() {
            "ready" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("ready"),
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::Issued,
            ),
            "rejected_invalid" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_invalid"),
                Some(OAuthPreflightOutcome::Rejected(
                    AuthAuditReason::InvalidCredential,
                )),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            ),
            "rejected_expired" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_expired"),
                Some(OAuthPreflightOutcome::Rejected(AuthAuditReason::Expired)),
                AuthAuditOutcome::Deny,
                AuthAuditReason::Expired,
            ),
            "rejected_replay" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_replay"),
                Some(OAuthPreflightOutcome::Rejected(
                    AuthAuditReason::ReplayDetected,
                )),
                AuthAuditOutcome::Deny,
                AuthAuditReason::ReplayDetected,
            ),
            "rate_limited" => {
                let retry_at = result.timestamp.ok_or(AdapterFailure::Corrupt)?;
                (
                    base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at),
                    Some(OAuthPreflightOutcome::RateLimited { retry_at }),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::RateLimited,
                )
            }
            _ => return Err(AdapterFailure::Corrupt),
        };
        if !self.oauth_operation_receipt_matches(&receipt).await?
            || !self
                .audit_operation_exists(
                    &receipt.operation_id,
                    command.audit.action,
                    audit_outcome,
                    reason,
                )
                .await?
        {
            return Err(AdapterFailure::Unavailable);
        }
        if let Some(outcome) = denied {
            return Ok(Some(outcome));
        }
        let reservation_id = OAuthExchangeReservationId::parse(&stable_child_uuid(
            &receipt.operation_id,
            "oauth-reservation",
        ))
        .map_err(|_| AdapterFailure::Corrupt)?;
        let stored = self
            .oauth_reservation(reservation_id)
            .await?
            .ok_or(AdapterFailure::Unavailable)?;
        if stored.id != reservation_id || stored.provider != command.provider {
            return Err(AdapterFailure::Unavailable);
        }
        Ok(Some(OAuthPreflightOutcome::Ready(
            frame_ports::OAuthExchangeReservation::from_repository(
                stored.id,
                stored.flow_id,
                stored.provider,
                stored.initiator,
                stored.expires_at,
            ),
        )))
    }

    async fn oauth_finalize_outcome_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &OAuthFinalizeCommand,
    ) -> AdapterResult<Option<OAuthExchangeOutcome>> {
        let Some(result) = self.oauth_operation_result(base_receipt).await? else {
            return Ok(None);
        };
        let (receipt, rejected, audit_outcome, reason) = match result.code.as_str() {
            "linked" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("linked"),
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::Linked,
            ),
            "verified" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("verified"),
                None,
                AuthAuditOutcome::Allow,
                AuthAuditReason::Authenticated,
            ),
            "rejected_invalid" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_invalid"),
                Some(AuthAuditReason::InvalidCredential),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            ),
            "rejected_expired" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_expired"),
                Some(AuthAuditReason::Expired),
                AuthAuditOutcome::Deny,
                AuthAuditReason::Expired,
            ),
            "rejected_replay" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_replay"),
                Some(AuthAuditReason::ReplayDetected),
                AuthAuditOutcome::Deny,
                AuthAuditReason::ReplayDetected,
            ),
            "adapter_failure" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("adapter_failure"),
                Some(AuthAuditReason::AdapterFailure),
                AuthAuditOutcome::Error,
                AuthAuditReason::AdapterFailure,
            ),
            _ => return Err(AdapterFailure::Corrupt),
        };
        if !self.oauth_operation_receipt_matches(&receipt).await?
            || !self
                .audit_operation_exists(
                    &receipt.operation_id,
                    command.audit.action,
                    audit_outcome,
                    reason,
                )
                .await?
        {
            return Err(AdapterFailure::Unavailable);
        }
        if let Some(reason) = rejected {
            return Ok(Some(OAuthExchangeOutcome::Rejected(reason)));
        }
        let OAuthProviderResult::Verified(assertion) = &command.provider_result else {
            return Err(AdapterFailure::Corrupt);
        };
        if assertion.provider != command.reservation.provider() {
            return Err(AdapterFailure::Corrupt);
        }
        let account = self
            .settled_oauth_external_account(assertion.provider, &assertion.subject_digests)
            .await?
            .ok_or(AdapterFailure::Unavailable)?;
        if result.code == "linked" {
            let initiator = command
                .reservation
                .initiator()
                .ok_or(AdapterFailure::Corrupt)?;
            if account.user_id != initiator.user_id {
                return Err(AdapterFailure::Unavailable);
            }
            return Ok(Some(OAuthExchangeOutcome::Linked {
                user_id: initiator.user_id,
            }));
        }
        if command.reservation.initiator().is_some() {
            return Err(AdapterFailure::Corrupt);
        }
        let principal = self
            .settled_principal(account.user_id)
            .await?
            .ok_or(AdapterFailure::Unavailable)?;
        let grant_id = PrincipalIssuanceGrantId::parse(&stable_child_uuid(
            &receipt.operation_id,
            "issuance-grant",
        ))
        .map_err(|_| AdapterFailure::Corrupt)?;
        let rows = self
            .settled_rows::<ExistenceRow>(
                OAUTH_VERIFIED_POSTCONDITION_SQL,
                &[
                    JsValue::from_str(&grant_id.to_string()),
                    JsValue::from_str(&account.user_id.to_string()),
                    JsValue::from_f64(principal.snapshot.identity_revision as f64),
                    JsValue::from_f64(command.reservation.expires_at().get() as f64),
                    JsValue::from_str(&receipt.operation_id),
                ],
            )
            .await?;
        if !matches!(rows.as_slice(), [ExistenceRow { present: 1 }]) {
            return if matches!(rows.as_slice(), [ExistenceRow { present: 0 }]) {
                Err(AdapterFailure::Unavailable)
            } else {
                Err(AdapterFailure::Corrupt)
            };
        }
        Ok(Some(OAuthExchangeOutcome::Verified {
            principal: principal.snapshot.clone(),
            issuance_grant: PrincipalIssuanceGrant::from_repository(
                grant_id,
                account.user_id,
                principal.snapshot.identity_revision,
                command.reservation.expires_at(),
            ),
        }))
    }

    async fn session_by_id(&self, id: SessionId) -> AdapterResult<Option<SessionState>> {
        self.one::<SessionRow>(SESSION_BY_ID_SQL, &[JsValue::from_str(&id.to_string())])
            .await?
            .map(SessionState::decode)
            .transpose()
    }

    async fn session_by_credentials(
        &self,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<CredentialSessionState>> {
        let json = candidates_json(candidates)?;
        let rows = self
            .rows::<CredentialSessionRow>(SESSION_BY_CREDENTIALS_SQL, &[JsValue::from_str(&json)])
            .await?;
        Self::decode_credential_session_rows(candidates, rows)
    }

    async fn settled_session_by_credentials(
        &self,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<CredentialSessionState>> {
        let json = candidates_json(candidates)?;
        let rows = self
            .settled_rows::<CredentialSessionRow>(
                SESSION_BY_CREDENTIALS_SQL,
                &[JsValue::from_str(&json)],
            )
            .await?;
        Self::decode_credential_session_rows(candidates, rows)
    }

    fn decode_credential_session_rows(
        candidates: &SecretDigestCandidates,
        rows: Vec<CredentialSessionRow>,
    ) -> AdapterResult<Option<CredentialSessionState>> {
        if rows.is_empty() {
            return Ok(None);
        }
        let session_ids = rows
            .iter()
            .map(|row| row.session.id.as_str())
            .collect::<BTreeSet<_>>();
        if session_ids.len() != 1 {
            return Err(AdapterFailure::Corrupt);
        }
        let row = rows.into_iter().next().ok_or(AdapterFailure::Corrupt)?;
        if row.matched_family_id != row.session.family_id {
            return Err(AdapterFailure::Corrupt);
        }
        let matched = versioned_digest(row.matched_key_version, &row.matched_digest)?;
        if !candidates.iter().any(|candidate| candidate == &matched) {
            return Err(AdapterFailure::Corrupt);
        }
        let credential_state = match row.credential_state.as_str() {
            "current" => CredentialState::Current,
            "rotated" => CredentialState::Rotated,
            "revoked" => CredentialState::Revoked,
            _ => return Err(AdapterFailure::Corrupt),
        };
        let session = SessionState::decode(row.session)?;
        if credential_state == CredentialState::Current && session.record.token_digest != matched {
            return Err(AdapterFailure::Corrupt);
        }
        Ok(Some(CredentialSessionState {
            matched,
            credential_state,
            credential_revision: safe_revision(row.credential_revision)?,
            session,
        }))
    }

    fn push_current_session_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        credential: &CredentialSessionState,
        now: TimestampMillis,
    ) -> AdapterResult<()> {
        let (key_version, digest) = digest_values(&credential.matched);
        self.push_statement(
            statements,
            SESSION_CURRENT_ASSERT_SQL,
            &[
                JsValue::from_str(&credential.session.record.id.to_string()),
                JsValue::from_f64(credential.session.row_revision as f64),
                JsValue::from_f64(credential.session.record.generation as f64),
                JsValue::from_str(&enum_name(credential.session.record.state)?),
                key_version,
                digest,
                JsValue::from_str(match credential.credential_state {
                    CredentialState::Current => "current",
                    CredentialState::Rotated => "rotated",
                    CredentialState::Revoked => "revoked",
                }),
                JsValue::from_f64(credential.credential_revision as f64),
                JsValue::from_str(&format!("{operation_id}:current")),
                JsValue::from_f64(now.get() as f64),
            ],
        )
    }

    async fn mutation_grant(
        &self,
        grant: &SessionMutationGrant,
        now: TimestampMillis,
    ) -> AdapterResult<Option<ValidatedMutationGrant>> {
        let row = self
            .one::<MutationGrantRow>(
                MUTATION_GRANT_SQL,
                &[JsValue::from_str(&grant.id().to_string())],
            )
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let stored_digest = versioned_digest(row.token_key_version, &row.token_digest)?;
        let current_digest =
            versioned_digest(row.current_token_key_version, &row.current_token_digest)?;
        safe_uuid(&row.id)?;
        if row.id != grant.id().to_string()
            || row.session_id != grant.session_id().to_string()
            || row.user_id != grant.user_id().to_string()
            || safe_revision(row.generation)? != grant.generation()
            || &stored_digest != grant.token_digest()
            || safe_revision(row.current_generation)? != grant.generation()
            || current_digest != stored_digest
            || row.session_state != "active"
            || row.user_status != "active"
            || safe_timestamp(row.idle_expires_at_ms)? <= now
            || safe_timestamp(row.absolute_expires_at_ms)? <= now
            || safe_revision(row.session_version)? != safe_revision(row.current_session_version)?
        {
            return Ok(None);
        }
        let session = self
            .session_by_id(grant.session_id())
            .await?
            .ok_or(AdapterFailure::Corrupt)?;
        let Some(principal) = self.principal(grant.user_id()).await? else {
            return Ok(None);
        };
        Ok(Some(ValidatedMutationGrant {
            session,
            principal,
            session_revision: safe_revision(row.session_revision)?,
        }))
    }

    async fn valid_continuation(
        &self,
        binding: SessionContinuationBinding,
        now: TimestampMillis,
    ) -> AdapterResult<bool> {
        let Some(session) = self.session_by_id(binding.session_id).await? else {
            return Ok(false);
        };
        Ok(session.record.user_id == binding.user_id
            && session.user_active
            && session.record.generation == binding.generation
            && session
                .record
                .evaluate(now, session.current_session_version)
                == AuthSessionDecision::Authenticated)
    }

    fn push_continuation_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        binding: SessionContinuationBinding,
        now: TimestampMillis,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            SESSION_CONTINUATION_ASSERT_SQL,
            &[
                JsValue::from_str(&binding.session_id.to_string()),
                JsValue::from_str(&binding.user_id.to_string()),
                JsValue::from_f64(binding.generation as f64),
                JsValue::from_f64(now.get() as f64),
                JsValue::from_str(&format!("{operation_id}:continuation")),
            ],
        )
    }

    fn push_identifier_absence_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            IDENTIFIER_CANDIDATES_ABSENT_ASSERT_SQL,
            &[
                JsValue::from_str(&candidates_json(candidates)?),
                JsValue::from_str(&format!("{operation_id}:identifier_absent")),
            ],
        )
    }

    async fn identity_exists(&self, user_id: UserId) -> AdapterResult<bool> {
        let row = self
            .one::<ExistenceRow>(
                IDENTITY_EXISTS_SQL,
                &[JsValue::from_str(&user_id.to_string())],
            )
            .await?;
        match row {
            None => Ok(false),
            Some(ExistenceRow { present: 1 }) => Ok(true),
            Some(_) => Err(AdapterFailure::Corrupt),
        }
    }

    fn push_identity_absence_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        user_id: UserId,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            IDENTITY_ABSENT_ASSERT_SQL,
            &[
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_str(&format!("{operation_id}:identity_absent")),
            ],
        )
    }

    fn push_identity_current_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        identity: &PrincipalState,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            IDENTITY_CURRENT_ASSERT_SQL,
            &[
                JsValue::from_str(&identity.snapshot.user_id.to_string()),
                JsValue::from_f64(identity.snapshot.identity_revision as f64),
                JsValue::from_f64(identity.row_revision as f64),
                JsValue::from_str(&format!("{operation_id}:identity_current")),
            ],
        )
    }

    fn tenant_authority(
        principal: &PrincipalState,
        tenant_id: TenantId,
    ) -> Option<TenantAuthorityFence> {
        principal.authority_fences.iter().copied().find(|fence| {
            fence.tenant_id == tenant_id
                && matches!(
                    fence.role,
                    OrganizationRole::Owner | OrganizationRole::Admin
                )
        })
    }

    fn tenant_membership(
        principal: &PrincipalState,
        tenant_id: TenantId,
    ) -> Option<TenantAuthorityFence> {
        principal
            .authority_fences
            .iter()
            .copied()
            .find(|fence| fence.tenant_id == tenant_id)
    }

    fn push_tenant_authority_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        principal: &PrincipalState,
        fence: TenantAuthorityFence,
    ) -> AdapterResult<()> {
        self.push_statement(
            statements,
            TENANT_AUTHORITY_ASSERT_SQL,
            &[
                JsValue::from_str(&principal.snapshot.user_id.to_string()),
                JsValue::from_f64(principal.snapshot.identity_revision as f64),
                JsValue::from_f64(principal.row_revision as f64),
                JsValue::from_str(&fence.tenant_id.to_string()),
                JsValue::from_str(&enum_name(fence.role)?),
                JsValue::from_f64(fence.member_revision as f64),
                JsValue::from_f64(fence.organization_revision as f64),
                JsValue::from_str(&format!("{operation_id}:tenant_authority")),
            ],
        )
    }

    async fn pending_by_id(&self, id: AuthDeliveryId) -> AdapterResult<Option<PendingState>> {
        self.one::<PendingRow>(PENDING_BY_ID_SQL, &[JsValue::from_str(&id.to_string())])
            .await?
            .map(PendingRow::decode)
            .transpose()
    }

    async fn verification_candidate(
        &self,
        identifiers: &SecretDigestCandidates,
        secrets: &SecretDigestCandidates,
        purpose: VerificationPurpose,
    ) -> AdapterResult<Option<DecodedVerification>> {
        let identifier_json = candidates_json(identifiers)?;
        let secret_json = candidates_json(secrets)?;
        self.one::<VerificationRow>(
            VERIFICATION_CANDIDATE_SQL,
            &[
                JsValue::from_str(&identifier_json),
                JsValue::from_str(&secret_json),
                JsValue::from_str(&enum_name(purpose)?),
            ],
        )
        .await?
        .map(VerificationRow::decode)
        .transpose()
    }

    async fn settled_verification_candidate(
        &self,
        identifiers: &SecretDigestCandidates,
        secrets: &SecretDigestCandidates,
        purpose: VerificationPurpose,
    ) -> AdapterResult<Option<DecodedVerification>> {
        let identifier_json = candidates_json(identifiers)?;
        let secret_json = candidates_json(secrets)?;
        let rows = self
            .settled_rows::<VerificationRow>(
                VERIFICATION_CANDIDATE_SQL,
                &[
                    JsValue::from_str(&identifier_json),
                    JsValue::from_str(&secret_json),
                    JsValue::from_str(&enum_name(purpose)?),
                ],
            )
            .await?;
        match rows.len() {
            0 => Ok(None),
            1 => rows
                .into_iter()
                .next()
                .ok_or(AdapterFailure::Corrupt)?
                .decode()
                .map(Some),
            _ => Err(AdapterFailure::Corrupt),
        }
    }

    async fn verification_attempt_committed(
        &self,
        base_receipt: &OperationReceipt,
        command: &VerificationAttemptCommand,
    ) -> AdapterResult<Option<VerificationAtomicOutcome>> {
        let Some(result) = self.operation_result(base_receipt).await? else {
            return Ok(None);
        };
        let (receipt, replay) = match result.code.as_str() {
            "verified" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("verified"),
                VerificationAttemptReplay::Verified,
            ),
            "provisioning_authorized" if result.timestamp.is_none() => (
                base_receipt
                    .clone()
                    .with_result_code("provisioning_authorized"),
                VerificationAttemptReplay::ProvisioningAuthorized,
            ),
            "linked" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("linked"),
                VerificationAttemptReplay::Linked,
            ),
            "rejected_invalid" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_invalid"),
                VerificationAttemptReplay::Rejected(AuthAuditReason::InvalidCredential),
            ),
            "rejected_expired" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_expired"),
                VerificationAttemptReplay::Rejected(AuthAuditReason::Expired),
            ),
            "rejected_attempts" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_attempts"),
                VerificationAttemptReplay::Rejected(AuthAuditReason::AttemptsExhausted),
            ),
            "rejected_replay" if result.timestamp.is_none() => (
                base_receipt.clone().with_result_code("rejected_replay"),
                VerificationAttemptReplay::Rejected(AuthAuditReason::ReplayDetected),
            ),
            "rate_limited" => {
                let retry_at = result.timestamp.ok_or(AdapterFailure::Corrupt)?;
                (
                    base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at),
                    VerificationAttemptReplay::RateLimited(retry_at),
                )
            }
            _ => return Err(AdapterFailure::Corrupt),
        };
        if !self.operation_receipt_matches(&receipt).await? {
            return Err(AdapterFailure::Unavailable);
        }
        match replay {
            VerificationAttemptReplay::Rejected(reason) => {
                if !self
                    .audit_operation_exists(
                        &receipt.operation_id,
                        command.audit.action,
                        AuthAuditOutcome::Deny,
                        reason,
                    )
                    .await?
                {
                    return Err(AdapterFailure::Unavailable);
                }
                Ok(Some(VerificationAtomicOutcome::Rejected(reason)))
            }
            VerificationAttemptReplay::RateLimited(retry_at) => {
                if !self
                    .audit_operation_exists(
                        &receipt.operation_id,
                        command.audit.action,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .await?
                {
                    return Err(AdapterFailure::Unavailable);
                }
                Ok(Some(VerificationAtomicOutcome::RateLimited { retry_at }))
            }
            success => {
                let decoded = self
                    .settled_verification_candidate(
                        &command.identifier_digests,
                        &command.secret_digests,
                        command.purpose,
                    )
                    .await?
                    .ok_or(AdapterFailure::Unavailable)?;
                let challenge = decoded.challenge;
                if !decoded.secret_matches
                    || challenge.state != VerificationState::Consumed
                    || challenge.consumed_at != Some(command.audit.occurred_at)
                    || challenge.identifier_digest != *command.identifier_digests.active()
                    || challenge.secret_digest != *command.secret_digests.active()
                {
                    return Err(AdapterFailure::Unavailable);
                }
                let user_id = challenge.user_id.ok_or(AdapterFailure::Corrupt)?;
                let (identifier_version, identifier_digest) =
                    digest_values(&challenge.identifier_digest);
                let (secret_version, secret_digest) = digest_values(&challenge.secret_digest);
                match success {
                    VerificationAttemptReplay::Verified => {
                        let grant_id = PrincipalIssuanceGrantId::parse(&stable_child_uuid(
                            &receipt.operation_id,
                            "issuance-grant",
                        ))
                        .map_err(|_| AdapterFailure::Corrupt)?;
                        let principal = self
                            .settled_principal(user_id)
                            .await?
                            .ok_or(AdapterFailure::Unavailable)?;
                        let rows = self
                            .settled_rows::<ExistenceRow>(
                                VERIFICATION_ATTEMPT_VERIFIED_POSTCONDITION_SQL,
                                &[
                                    JsValue::from_str(&challenge.id.to_string()),
                                    JsValue::from_str(&user_id.to_string()),
                                    identifier_version,
                                    identifier_digest,
                                    secret_version,
                                    secret_digest,
                                    JsValue::from_str(&enum_name(command.purpose)?),
                                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                                    JsValue::from_str(&receipt.operation_id),
                                    JsValue::from_str(&grant_id.to_string()),
                                ],
                            )
                            .await?;
                        if !matches!(rows.as_slice(), [ExistenceRow { present: 1 }]) {
                            return if matches!(rows.as_slice(), [ExistenceRow { present: 0 }]) {
                                Err(AdapterFailure::Unavailable)
                            } else {
                                Err(AdapterFailure::Corrupt)
                            };
                        }
                        let identity_revision = principal.snapshot.identity_revision;
                        Ok(Some(VerificationAtomicOutcome::Verified {
                            principal: principal.snapshot,
                            issuance_grant: PrincipalIssuanceGrant::from_repository(
                                grant_id,
                                user_id,
                                identity_revision,
                                challenge.expires_at,
                            ),
                        }))
                    }
                    VerificationAttemptReplay::ProvisioningAuthorized => {
                        let identity_revision = challenge
                            .provisioning_revision
                            .ok_or(AdapterFailure::Corrupt)?;
                        let grant_id = IdentityProvisioningGrantId::parse(&stable_child_uuid(
                            &receipt.operation_id,
                            "provisioning-grant",
                        ))
                        .map_err(|_| AdapterFailure::Corrupt)?;
                        let rows = self
                            .settled_rows::<ExistenceRow>(
                                VERIFICATION_ATTEMPT_PROVISIONING_POSTCONDITION_SQL,
                                &[
                                    JsValue::from_str(&challenge.id.to_string()),
                                    JsValue::from_str(&user_id.to_string()),
                                    identifier_version,
                                    identifier_digest,
                                    secret_version,
                                    secret_digest,
                                    JsValue::from_f64(identity_revision as f64),
                                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                                    JsValue::from_str(&receipt.operation_id),
                                    JsValue::from_str(&grant_id.to_string()),
                                ],
                            )
                            .await?;
                        if !matches!(rows.as_slice(), [ExistenceRow { present: 1 }]) {
                            return if matches!(rows.as_slice(), [ExistenceRow { present: 0 }]) {
                                Err(AdapterFailure::Unavailable)
                            } else {
                                Err(AdapterFailure::Corrupt)
                            };
                        }
                        Ok(Some(VerificationAtomicOutcome::ProvisioningAuthorized(
                            IdentityProvisioningGrant::from_repository(
                                grant_id,
                                user_id,
                                identity_revision,
                                challenge.identifier_digest,
                                challenge.expires_at,
                            ),
                        )))
                    }
                    VerificationAttemptReplay::Linked => {
                        let rows = self
                            .settled_rows::<ExistenceRow>(
                                VERIFICATION_ATTEMPT_LINKED_POSTCONDITION_SQL,
                                &[
                                    JsValue::from_str(&challenge.id.to_string()),
                                    JsValue::from_str(&user_id.to_string()),
                                    identifier_version,
                                    identifier_digest,
                                    secret_version,
                                    secret_digest,
                                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                                    JsValue::from_str(&receipt.operation_id),
                                ],
                            )
                            .await?;
                        if !matches!(rows.as_slice(), [ExistenceRow { present: 1 }]) {
                            return if matches!(rows.as_slice(), [ExistenceRow { present: 0 }]) {
                                Err(AdapterFailure::Unavailable)
                            } else {
                                Err(AdapterFailure::Corrupt)
                            };
                        }
                        Ok(Some(VerificationAtomicOutcome::Linked { user_id }))
                    }
                    VerificationAttemptReplay::Rejected(_)
                    | VerificationAttemptReplay::RateLimited(_) => Err(AdapterFailure::Corrupt),
                }
            }
        }
    }

    async fn commit_verification_attempt(
        &self,
        statements: Vec<D1PreparedStatement>,
        receipt: &OperationReceipt,
        command: &VerificationAttemptCommand,
    ) -> AdapterResult<VerificationAtomicOutcome> {
        self.batch_mutation(statements, receipt).await?;
        self.verification_attempt_committed(receipt, command)
            .await?
            .ok_or(AdapterFailure::Unavailable)
    }

    async fn delivery_by_id(&self, id: AuthDeliveryId) -> AdapterResult<Option<DeliveryRow>> {
        let row = self
            .one::<DeliveryRow>(DELIVERY_BY_ID_SQL, &[JsValue::from_str(&id.to_string())])
            .await?;
        if let Some(row) = &row {
            row.validated()?;
        }
        Ok(row)
    }

    fn push_challenge_insert(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        challenge: &VerificationChallenge,
    ) -> AdapterResult<()> {
        let user_id = challenge.user_id.map(|value| value.to_string());
        let initiator_session = challenge
            .initiator
            .map(|binding| binding.session_id.to_string());
        let initiator_user = challenge
            .initiator
            .map(|binding| binding.user_id.to_string());
        let (identifier_version, identifier_digest) = digest_values(&challenge.identifier_digest);
        let (secret_version, secret_digest) = digest_values(&challenge.secret_digest);
        self.push_statement(
            statements,
            CHALLENGE_INSERT_SQL,
            &[
                JsValue::from_str(&challenge.id.to_string()),
                opt_string(user_id.as_deref()),
                opt_string(initiator_session.as_deref()),
                opt_string(initiator_user.as_deref()),
                opt_i64(
                    challenge
                        .initiator
                        .map(|binding| i64::try_from(binding.generation).unwrap_or(i64::MAX)),
                ),
                opt_i64(
                    challenge
                        .provisioning_revision
                        .map(|revision| i64::try_from(revision).unwrap_or(i64::MAX)),
                ),
                identifier_version,
                identifier_digest,
                secret_version,
                secret_digest,
                JsValue::from_str(&enum_name(challenge.purpose)?),
                JsValue::from_str(&enum_name(challenge.channel)?),
                JsValue::from_f64(f64::from(challenge.attempt_count)),
                JsValue::from_f64(f64::from(challenge.max_attempts)),
                JsValue::from_f64(challenge.created_at.get() as f64),
                JsValue::from_f64(challenge.expires_at.get() as f64),
                opt_i64(challenge.consumed_at.map(TimestampMillis::get)),
                JsValue::from_str(&enum_name(challenge.state)?),
                JsValue::from_str(operation_id),
            ],
        )
    }

    fn push_challenge_update(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        challenge: &VerificationChallenge,
        expected_revision: u64,
        expected_state: VerificationState,
    ) -> AdapterResult<()> {
        let (identifier_version, identifier_digest) = digest_values(&challenge.identifier_digest);
        let (secret_version, secret_digest) = digest_values(&challenge.secret_digest);
        self.push_statement(
            statements,
            CHALLENGE_UPDATE_SQL,
            &[
                JsValue::from_str(&challenge.id.to_string()),
                JsValue::from_f64(expected_revision as f64),
                JsValue::from_str(&enum_name(expected_state)?),
                identifier_version,
                identifier_digest,
                secret_version,
                secret_digest,
                JsValue::from_f64(f64::from(challenge.attempt_count)),
                opt_i64(challenge.consumed_at.map(TimestampMillis::get)),
                JsValue::from_str(&enum_name(challenge.state)?),
                JsValue::from_str(operation_id),
            ],
        )?;
        self.push_assertion(statements, operation_id, "challenge", 1)
    }

    async fn aggregate_counts(&self, sql: &str, identifier: &str) -> AdapterResult<(usize, usize)> {
        let row = self
            .one::<AggregateCountsRow>(sql, &[JsValue::from_str(identifier)])
            .await?
            .ok_or(AdapterFailure::Corrupt)?;
        let sessions = usize::try_from(row.active_sessions).map_err(|_| AdapterFailure::Corrupt)?;
        let credentials =
            usize::try_from(row.live_credentials).map_err(|_| AdapterFailure::Corrupt)?;
        Ok((sessions, credentials))
    }

    async fn api_key_by_candidates(
        &self,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<(ManagedApiKeyRecord, u64, usize)>> {
        let json = candidates_json(candidates)?;
        let rows = self
            .rows::<ApiKeyRow>(API_KEY_BY_CREDENTIALS_SQL, &[JsValue::from_str(&json)])
            .await?;
        Self::decode_api_key_rows(rows)
    }

    async fn settled_api_key_by_candidates(
        &self,
        candidates: &SecretDigestCandidates,
    ) -> AdapterResult<Option<(ManagedApiKeyRecord, u64, usize)>> {
        let json = candidates_json(candidates)?;
        let rows = self
            .settled_rows::<ApiKeyRow>(API_KEY_BY_CREDENTIALS_SQL, &[JsValue::from_str(&json)])
            .await?;
        Self::decode_api_key_rows(rows)
    }

    fn decode_api_key_rows(
        rows: Vec<ApiKeyRow>,
    ) -> AdapterResult<Option<(ManagedApiKeyRecord, u64, usize)>> {
        if rows.is_empty() {
            return Ok(None);
        }
        let ids = rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != 1 {
            return Err(AdapterFailure::Corrupt);
        }
        rows.into_iter()
            .next()
            .ok_or(AdapterFailure::Corrupt)?
            .decode()
            .map(Some)
    }

    async fn api_key_by_id(
        &self,
        id: ApiKeyId,
    ) -> AdapterResult<Option<(ManagedApiKeyRecord, u64)>> {
        self.one::<ApiKeyRow>(API_KEY_BY_ID_SQL, &[JsValue::from_str(&id.to_string())])
            .await?
            .map(|row| row.decode().map(|(record, revision, _)| (record, revision)))
            .transpose()
    }

    fn push_api_key_assertion(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        record: &ManagedApiKeyRecord,
        revision: u64,
    ) -> AdapterResult<()> {
        let (version, digest) = digest_values(&record.key_digest);
        self.push_statement(
            statements,
            API_KEY_CURRENT_ASSERT_SQL,
            &[
                JsValue::from_str(&record.id.to_string()),
                JsValue::from_f64(revision as f64),
                version,
                digest,
                opt_i64(record.revoked_at.map(TimestampMillis::get)),
                JsValue::from_str(&format!("{operation_id}:api_key_current")),
            ],
        )
    }

    fn push_mutation_grant_consume(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        grant: &SessionMutationGrant,
        now: TimestampMillis,
    ) -> AdapterResult<()> {
        let (version, digest) = digest_values(grant.token_digest());
        self.push_statement(
            statements,
            MUTATION_GRANT_DELETE_SQL,
            &[
                JsValue::from_str(&grant.id().to_string()),
                JsValue::from_str(&grant.session_id().to_string()),
                JsValue::from_str(&grant.user_id().to_string()),
                JsValue::from_f64(grant.generation() as f64),
                version,
                digest,
                JsValue::from_f64(now.get() as f64),
            ],
        )?;
        self.push_assertion(statements, operation_id, "grant", 1)
    }

    async fn append_rate_limit_plan(
        &self,
        statements: &mut Vec<D1PreparedStatement>,
        operation_id: &str,
        action: AuthAbuseAction,
        abuse: &AbuseDigestSet,
        policy: frame_domain::MultiRateLimitPolicy,
        now: TimestampMillis,
    ) -> AdapterResult<Option<TimestampMillis>> {
        let action = action.rate_limit_bucket_action();
        self.push_statement(
            statements,
            RATE_BUCKET_GC_SQL,
            &[JsValue::from_f64(now.get() as f64)],
        )?;
        let count = self
            .one::<CountRow>(
                RATE_BUCKET_COUNT_SQL,
                &[JsValue::from_f64(now.get() as f64)],
            )
            .await?
            .ok_or(AdapterFailure::Corrupt)?;
        let mut active_count = usize::try_from(count.bucket_count)
            .ok()
            .filter(|count| *count <= MAX_RATE_LIMIT_BUCKETS)
            .ok_or(AdapterFailure::Corrupt)?;
        let dimensions = [
            (AbuseDimension::Global, None),
            (AbuseDimension::Source, Some(&abuse.source)),
            (AbuseDimension::Device, Some(&abuse.device)),
            (AbuseDimension::Identifier, Some(&abuse.identifier)),
        ];
        let action_name = enum_name(action)?;
        for (index, (dimension, candidates)) in dimensions.into_iter().enumerate() {
            let dimension_name = enum_name(dimension)?;
            let dimension_policy = match dimension {
                AbuseDimension::Identifier => policy.identifier,
                AbuseDimension::Source => policy.source,
                AbuseDimension::Device => policy.device,
                AbuseDimension::Global => policy.global,
            };
            let candidate_json = candidates.map_or_else(|| Ok("[]".into()), candidates_json)?;
            let rows = self
                .rows::<RateBucketRow>(
                    RATE_BUCKETS_SQL,
                    &[
                        JsValue::from_str(&action_name),
                        JsValue::from_str(&dimension_name),
                        JsValue::from_str(&candidate_json),
                        JsValue::from_f64(now.get() as f64),
                    ],
                )
                .await?;
            let active_digest = candidates.map(|values| values.active().clone());
            let active_id = AbuseBucketId::new(action, dimension, active_digest.clone())
                .map_err(|_| AdapterFailure::Invalid)?;
            let mut decoded = Vec::with_capacity(rows.len());
            for row in rows {
                if row.action != action_name
                    || row.dimension != dimension_name
                    || safe_timestamp(row.gc_at_ms)? <= now
                {
                    return Err(AdapterFailure::Corrupt);
                }
                let digest = if dimension == AbuseDimension::Global {
                    if row.key_version != 0 || !row.digest.is_empty() {
                        return Err(AdapterFailure::Corrupt);
                    }
                    None
                } else {
                    Some(versioned_digest(row.key_version, &row.digest)?)
                };
                let id = AbuseBucketId::new(action, dimension, digest.clone())
                    .map_err(|_| AdapterFailure::Corrupt)?;
                let attempt_count = u32::try_from(row.attempt_count)
                    .ok()
                    .filter(|count| *count <= 1_000_000)
                    .ok_or(AdapterFailure::Corrupt)?;
                let bucket = AuthRateLimitBucket {
                    id,
                    window_started_at: safe_timestamp(row.window_started_at_ms)?,
                    attempt_count,
                    blocked_until: row.blocked_until_ms.map(safe_timestamp).transpose()?,
                    updated_at: safe_timestamp(row.updated_at_ms)?,
                };
                decoded.push(StoredRateBucket {
                    bucket,
                    revision: safe_revision(row.revision)?,
                    key_version: row.key_version,
                    digest_text: row.digest,
                });
            }
            let active_position = decoded
                .iter()
                .position(|stored| stored.bucket.id == active_id);
            let mut bucket = if let Some(position) = active_position {
                decoded[position].bucket.clone()
            } else if let Some(first) = decoded.first() {
                first.bucket.clone()
            } else {
                AuthRateLimitBucket::new(active_id.clone(), now)
            };
            for (position, stored) in decoded.iter().enumerate() {
                if Some(position) == active_position || (active_position.is_none() && position == 0)
                {
                    continue;
                }
                bucket.window_started_at = bucket
                    .window_started_at
                    .max(stored.bucket.window_started_at);
                bucket.attempt_count = bucket
                    .attempt_count
                    .saturating_add(stored.bucket.attempt_count);
                bucket.blocked_until = match (bucket.blocked_until, stored.bucket.blocked_until) {
                    (Some(left), Some(right)) => Some(left.max(right)),
                    (left, right) => left.or(right),
                };
                bucket.updated_at = bucket.updated_at.max(stored.bucket.updated_at);
            }
            bucket.id = active_id;
            let active_exists = active_position.is_some();
            let removed = decoded.len().saturating_sub(usize::from(active_exists));
            let effective_count = active_count.saturating_sub(removed);
            if !active_exists && effective_count >= MAX_RATE_LIMIT_BUCKETS {
                return now
                    .checked_add(dimension_policy.block_for())
                    .map(Some)
                    .map_err(|_| AdapterFailure::Invalid);
            }
            for (position, stale) in decoded.iter().enumerate() {
                if Some(position) == active_position {
                    continue;
                }
                self.push_statement(
                    statements,
                    RATE_BUCKET_DELETE_SQL,
                    &[
                        JsValue::from_str(&action_name),
                        JsValue::from_str(&dimension_name),
                        JsValue::from_f64(stale.key_version as f64),
                        JsValue::from_str(&stale.digest_text),
                        JsValue::from_f64(stale.revision as f64),
                    ],
                )?;
                self.push_assertion(
                    statements,
                    operation_id,
                    &format!("rate_{index}_stale_{position}"),
                    1,
                )?;
            }
            let decision = bucket
                .consume(now, dimension_policy)
                .map_err(|_| AdapterFailure::Invalid)?;
            let gc_at = bucket
                .updated_at
                .checked_add(dimension_policy.window())
                .and_then(|time| time.checked_add(dimension_policy.block_for()))
                .map_err(|_| AdapterFailure::Invalid)?;
            let (key_version, digest) = active_digest.as_ref().map_or((0, ""), |digest| {
                (
                    i64::from(digest.key_version.get()),
                    digest.digest.expose_for_verification(),
                )
            });
            let expected_revision = active_position
                .map(|position| decoded[position].revision as i64)
                .unwrap_or(-1);
            self.push_statement(
                statements,
                RATE_BUCKET_UPSERT_SQL,
                &[
                    JsValue::from_str(&action_name),
                    JsValue::from_str(&dimension_name),
                    JsValue::from_f64(key_version as f64),
                    JsValue::from_str(digest),
                    JsValue::from_f64(bucket.window_started_at.get() as f64),
                    JsValue::from_f64(f64::from(bucket.attempt_count)),
                    opt_i64(bucket.blocked_until.map(TimestampMillis::get)),
                    JsValue::from_f64(bucket.updated_at.get() as f64),
                    JsValue::from_f64(gc_at.get() as f64),
                    JsValue::from_f64(expected_revision as f64),
                    JsValue::from_str(operation_id),
                ],
            )?;
            self.push_assertion(statements, operation_id, &format!("rate_{index}_active"), 1)?;
            active_count = effective_count + usize::from(!active_exists);
            if let frame_domain::RateLimitDecision::Limited { retry_at } = decision {
                return Ok(Some(retry_at));
            }
        }
        Ok(None)
    }

    async fn attempt_verification_once(
        &self,
        command: &VerificationAttemptCommand,
    ) -> AdapterResult<VerificationAtomicOutcome> {
        let policy = command.rate_policy;
        let semantic = vec![
            candidates_json(&command.identifier_digests)?,
            candidates_json(&command.secret_digests)?,
            enum_name(command.purpose)?,
            candidates_json(&command.abuse.identifier)?,
            candidates_json(&command.abuse.source)?,
            candidates_json(&command.abuse.device)?,
            format!(
                "{}:{}:{}|{}:{}:{}|{}:{}:{}|{}:{}:{}",
                policy.identifier.max_attempts(),
                policy.identifier.window().get(),
                policy.identifier.block_for().get(),
                policy.source.max_attempts(),
                policy.source.window().get(),
                policy.source.block_for().get(),
                policy.device.max_attempts(),
                policy.device.window().get(),
                policy.device.block_for().get(),
                policy.global.max_attempts(),
                policy.global.window().get(),
                policy.global.block_for().get(),
            ),
        ];
        let base_receipt = OperationReceipt::for_audit(
            "verification_attempt",
            command.audit.correlation_id.to_string(),
            &command.audit,
            &semantic,
        );
        if let Some(outcome) = self
            .verification_attempt_committed(&base_receipt, command)
            .await?
        {
            return Ok(outcome);
        }
        let operation_id = base_receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(32);
        if let Some(retry_at) = self
            .append_rate_limit_plan(
                &mut statements,
                &operation_id,
                AuthAbuseAction::Verify,
                &command.abuse,
                command.rate_policy,
                command.audit.occurred_at,
            )
            .await?
        {
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::RateLimited,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt
                .clone()
                .with_result_code("rate_limited")
                .with_result_timestamp(retry_at);
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        }

        let candidate = self
            .verification_candidate(
                &command.identifier_digests,
                &command.secret_digests,
                command.purpose,
            )
            .await?;
        if command.purpose == VerificationPurpose::AccountLink
            && let Some(decoded) = &candidate
            && decoded.challenge.initiator.is_none_or(|binding| {
                // The asynchronous authoritative check follows immediately;
                // this branch only handles malformed persisted shape.
                binding.user_id != decoded.challenge.user_id.unwrap_or(binding.user_id)
            })
        {
            self.push_statement(
                &mut statements,
                CHALLENGE_DELETE_SQL,
                &[
                    JsValue::from_str(&decoded.challenge.id.to_string()),
                    JsValue::from_f64(decoded.revision as f64),
                ],
            )?;
            self.push_assertion(&mut statements, &operation_id, "challenge", 1)?;
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                decoded.challenge.user_id,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt.clone().with_result_code("rejected_invalid");
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        }
        if command.purpose == VerificationPurpose::AccountLink
            && let Some(decoded) = &candidate
            && let Some(binding) = decoded.challenge.initiator
            && !self
                .valid_continuation(binding, command.audit.occurred_at)
                .await?
        {
            self.push_statement(
                &mut statements,
                CHALLENGE_DELETE_SQL,
                &[
                    JsValue::from_str(&decoded.challenge.id.to_string()),
                    JsValue::from_f64(decoded.revision as f64),
                ],
            )?;
            self.push_assertion(&mut statements, &operation_id, "challenge", 1)?;
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                decoded.challenge.user_id,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt.clone().with_result_code("rejected_invalid");
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        }

        let Some(mut decoded) = candidate else {
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt.clone().with_result_code("rejected_invalid");
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        };

        let expected_state = decoded.challenge.state;
        let decision = decoded
            .challenge
            .attempt(command.audit.occurred_at, decoded.secret_matches);
        if decoded.secret_matches {
            decoded.challenge.identifier_digest = command.identifier_digests.active().clone();
            decoded.challenge.secret_digest = command.secret_digests.active().clone();
        }
        self.push_challenge_update(
            &mut statements,
            &operation_id,
            &decoded.challenge,
            decoded.revision,
            expected_state,
        )?;
        let reason = verification_reason(decision);
        let VerificationDecision::Verified(user_id) = decision else {
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                None,
                AuthAuditOutcome::Deny,
                reason,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt
                .clone()
                .with_result_code(verification_rejection_result(reason)?);
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        };
        let grant_expires_at = decoded.challenge.expires_at;

        if command.purpose == VerificationPurpose::IdentityProvisioning {
            let owner = self.identifier_owner(&command.identifier_digests).await?;
            let identity_exists = self.identity_exists(user_id).await?;
            let Some(identity_revision) = decoded.challenge.provisioning_revision else {
                return Err(AdapterFailure::Corrupt);
            };
            if owner.is_some() || identity_exists {
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    None,
                    Some(user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )?;
                self.push_cleanup(&mut statements, &operation_id)?;
                let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                return self
                    .commit_verification_attempt(statements, &receipt, command)
                    .await;
            }
            self.push_identifier_absence_assertion(
                &mut statements,
                &operation_id,
                &command.identifier_digests,
            )?;
            self.push_identity_absence_assertion(&mut statements, &operation_id, user_id)?;
            let grant_id = IdentityProvisioningGrantId::parse(&stable_child_uuid(
                &operation_id,
                "provisioning-grant",
            ))
            .map_err(|_| AdapterFailure::Corrupt)?;
            let active_identifier = command.identifier_digests.active().clone();
            let (identifier_version, identifier_digest) = digest_values(&active_identifier);
            self.push_statement(
                &mut statements,
                PROVISIONING_GRANT_INSERT_SQL,
                &[
                    JsValue::from_str(&grant_id.to_string()),
                    JsValue::from_str(&user_id.to_string()),
                    JsValue::from_f64(identity_revision as f64),
                    identifier_version,
                    identifier_digest,
                    JsValue::from_f64(grant_expires_at.get() as f64),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&operation_id),
                ],
            )?;
            self.push_assertion(&mut statements, &operation_id, "provisioning_grant", 1)?;
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Allow,
                reason,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt
                .clone()
                .with_result_code("provisioning_authorized");
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        }

        if command.purpose == VerificationPurpose::AccountLink {
            let owner = self.identifier_owner(&command.identifier_digests).await?;
            let binding = decoded.challenge.initiator.ok_or(AdapterFailure::Corrupt)?;
            let continuation_valid = binding.user_id == user_id
                && self
                    .valid_continuation(binding, command.audit.occurred_at)
                    .await?;
            if owner.is_some() || !continuation_valid {
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    None,
                    Some(user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )?;
                self.push_cleanup(&mut statements, &operation_id)?;
                let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                return self
                    .commit_verification_attempt(statements, &receipt, command)
                    .await;
            }
            self.push_continuation_assertion(
                &mut statements,
                &operation_id,
                binding,
                command.audit.occurred_at,
            )?;
            self.push_identifier_absence_assertion(
                &mut statements,
                &operation_id,
                &command.identifier_digests,
            )?;
            let (identifier_version, identifier_digest) =
                digest_values(command.identifier_digests.active());
            self.push_statement(
                &mut statements,
                IDENTIFIER_INSERT_SQL,
                &[
                    identifier_version,
                    identifier_digest,
                    JsValue::from_str(&user_id.to_string()),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&operation_id),
                ],
            )?;
            self.push_assertion(&mut statements, &operation_id, "identifier", 1)?;
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Allow,
                AuthAuditReason::Linked,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt.clone().with_result_code("linked");
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        }

        let Some(principal) = self.principal(user_id).await? else {
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )?;
            self.push_cleanup(&mut statements, &operation_id)?;
            let receipt = base_receipt.clone().with_result_code("rejected_invalid");
            return self
                .commit_verification_attempt(statements, &receipt, command)
                .await;
        };
        if command.purpose == VerificationPurpose::AccountRecovery {
            let (active_sessions, live_credentials) = self
                .aggregate_counts(SESSION_USER_COUNTS_SQL, &user_id.to_string())
                .await?;
            self.push_statement(
                &mut statements,
                IDENTITY_SESSION_VERSION_INCREMENT_SQL,
                &[
                    JsValue::from_str(&user_id.to_string()),
                    JsValue::from_f64(principal.row_revision as f64),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&operation_id),
                ],
            )?;
            self.push_assertion(&mut statements, &operation_id, "identity", 1)?;
            self.push_statement(
                &mut statements,
                SESSION_REVOKE_USER_SQL,
                &[
                    JsValue::from_str(&user_id.to_string()),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str("account_recovery"),
                    JsValue::from_str(&operation_id),
                ],
            )?;
            self.push_assertion(&mut statements, &operation_id, "sessions", active_sessions)?;
            self.push_statement(
                &mut statements,
                CREDENTIALS_REVOKE_USER_SQL,
                &[
                    JsValue::from_str(&user_id.to_string()),
                    JsValue::from_str(&operation_id),
                ],
            )?;
            self.push_assertion(
                &mut statements,
                &operation_id,
                "credentials",
                live_credentials,
            )?;
            for sql in [
                MUTATION_GRANTS_DELETE_USER_SQL,
                PENDING_CONTINUATIONS_DELETE_USER_SQL,
                CHALLENGE_CONTINUATIONS_DELETE_USER_SQL,
                DELIVERY_CONTINUATIONS_DELETE_USER_SQL,
            ] {
                self.push_statement(
                    &mut statements,
                    sql,
                    &[JsValue::from_str(&user_id.to_string())],
                )?;
            }
        } else {
            self.push_identity_current_assertion(&mut statements, &operation_id, &principal)?;
        }
        let grant_id =
            PrincipalIssuanceGrantId::parse(&stable_child_uuid(&operation_id, "issuance-grant"))
                .map_err(|_| AdapterFailure::Corrupt)?;
        self.push_statement(
            &mut statements,
            ISSUANCE_GRANT_INSERT_SQL,
            &[
                JsValue::from_str(&grant_id.to_string()),
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_f64(principal.snapshot.identity_revision as f64),
                JsValue::from_f64(grant_expires_at.get() as f64),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
                JsValue::from_str(&operation_id),
            ],
        )?;
        self.push_assertion(&mut statements, &operation_id, "issuance_grant", 1)?;
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            None,
            Some(user_id),
            AuthAuditOutcome::Allow,
            reason,
        )?;
        self.push_cleanup(&mut statements, &operation_id)?;
        let receipt = base_receipt.clone().with_result_code("verified");
        self.commit_verification_attempt(statements, &receipt, command)
            .await
    }

    async fn commit_audit(
        &self,
        audit: &DecisionAudit,
        context: Option<SessionAuditContext>,
        user_id: Option<UserId>,
        outcome: AuthAuditOutcome,
        reason: AuthAuditReason,
    ) -> AdapterResult<()> {
        let subject = user_id
            .map(|value| value.to_string())
            .or_else(|| context.map(|value| value.user_id.to_string()))
            .unwrap_or_else(|| audit.correlation_id.to_string());
        let semantic = vec![
            enum_name(audit.action)?,
            enum_name(outcome)?,
            enum_name(reason)?,
            context
                .map(|value| value.session_id.to_string())
                .unwrap_or_default(),
        ];
        let receipt = OperationReceipt::for_audit("audit", subject, audit, &semantic);
        if self.operation_receipt_matches(&receipt).await? {
            return self
                .audit_operation_exists(&receipt.operation_id, audit.action, outcome, reason)
                .await?
                .then_some(())
                .ok_or(AdapterFailure::Unavailable);
        }
        let mut statements = Vec::with_capacity(2);
        self.push_audit(
            &mut statements,
            &receipt.operation_id,
            audit,
            context,
            user_id,
            outcome,
            reason,
        )?;
        self.batch_mutation(statements, &receipt).await?;
        if self
            .audit_operation_exists(&receipt.operation_id, audit.action, outcome, reason)
            .await?
        {
            Ok(())
        } else {
            Err(AdapterFailure::Unavailable)
        }
    }
}

#[derive(Debug, Clone)]
struct PrincipalState {
    snapshot: PrincipalSnapshot,
    session_version: u64,
    row_revision: u64,
    authority_fences: Vec<TenantAuthorityFence>,
}

#[derive(Debug, Clone, Copy)]
struct TenantAuthorityFence {
    tenant_id: TenantId,
    role: OrganizationRole,
    member_revision: u64,
    organization_revision: u64,
}

#[derive(Debug, Clone)]
struct SessionState {
    record: AuthSessionRecord,
    current_session_version: u64,
    row_revision: u64,
    user_active: bool,
}

impl SessionState {
    fn decode(row: SessionRow) -> AdapterResult<Self> {
        let (record, current_session_version, row_revision, user_active) = row.decode()?;
        Ok(Self {
            record,
            current_session_version,
            row_revision,
            user_active,
        })
    }
}

#[derive(Debug, Clone)]
struct ValidatedMutationGrant {
    session: SessionState,
    principal: PrincipalState,
    session_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialState {
    Current,
    Rotated,
    Revoked,
}

#[derive(Debug, Clone)]
struct CredentialSessionState {
    matched: VersionedSecretDigest,
    credential_state: CredentialState,
    credential_revision: u64,
    session: SessionState,
}

#[derive(Debug, Clone)]
struct StoredRateBucket {
    bucket: AuthRateLimitBucket,
    revision: u64,
    key_version: i64,
    digest_text: String,
}

#[derive(Debug, Clone)]
struct PendingState {
    delivery_id: AuthDeliveryId,
    identifier_candidates: SecretDigestCandidates,
    secret_digest: VersionedSecretDigest,
    purpose: VerificationPurpose,
    channel: VerificationChannel,
    initiator: Option<SessionContinuationBinding>,
    provisioning: Option<(UserId, u64)>,
    max_attempts: u16,
    created_at: TimestampMillis,
    expires_at: TimestampMillis,
    sealed_payload: Vec<u8>,
    revision: u64,
}

#[derive(Debug, Clone)]
struct DecodedVerification {
    challenge: VerificationChallenge,
    revision: u64,
    secret_matches: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationAttemptReplay {
    Verified,
    ProvisioningAuthorized,
    Linked,
    Rejected(AuthAuditReason),
    RateLimited(TimestampMillis),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiKeyAuthenticationReplay {
    RateLimited(TimestampMillis),
    Rejected,
    Authenticated(bool),
}

fn verification_rejection_result(reason: AuthAuditReason) -> AdapterResult<&'static str> {
    match reason {
        AuthAuditReason::InvalidCredential => Ok("rejected_invalid"),
        AuthAuditReason::Expired => Ok("rejected_expired"),
        AuthAuditReason::AttemptsExhausted => Ok("rejected_attempts"),
        AuthAuditReason::ReplayDetected => Ok("rejected_replay"),
        _ => Err(AdapterFailure::Corrupt),
    }
}

fn require_action(audit: &DecisionAudit, expected: AuthAuditAction) -> AdapterResult<()> {
    if audit.action == expected {
        Ok(())
    } else {
        Err(AdapterFailure::Invalid)
    }
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
impl AuthStateRepository for D1AuthStateRepository<'_> {
    async fn provision_identity(
        &self,
        command: IdentityProvisionCommand,
    ) -> Result<IdentityProvisionOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("identity_provision");
        require_action(&command.audit, AuthAuditAction::IdentityProvision)
            .map_err(AdapterFailure::into_port)?;
        let grant_id = command.grant.id();
        let user_id = command.grant.user_id();
        let semantic = vec![
            grant_id.to_string(),
            user_id.to_string(),
            command.grant.identity_revision().to_string(),
            serde_json::to_string(&DigestWire::from(command.grant.identifier_digest()))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            command.grant.expires_at().get().to_string(),
        ];
        let receipt = OperationReceipt::for_audit(
            "identity_provision",
            user_id.to_string(),
            &command.audit,
            &semantic,
        );
        if self
            .identity_provision_committed(&receipt, &command.grant)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("created", 1);
            return Ok(IdentityProvisionOutcome::Created);
        }
        let row = self
            .one::<ProvisioningGrantRow>(
                PROVISIONING_GRANT_SQL,
                &[JsValue::from_str(&grant_id.to_string())],
            )
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some(row) = row else {
            self.commit_audit(
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::ReplayDetected,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("replay_detected", 0);
            return Ok(IdentityProvisionOutcome::Rejected(
                AuthAuditReason::ReplayDetected,
            ));
        };
        let stored_digest = versioned_digest(row.identifier_key_version, &row.identifier_digest)
            .map_err(AdapterFailure::into_port)?;
        let stored_expiry = safe_timestamp(row.expires_at_ms).map_err(AdapterFailure::into_port)?;
        let reason = if command.audit.occurred_at >= stored_expiry {
            Some(AuthAuditReason::Expired)
        } else if row.id != grant_id.to_string()
            || row.user_id != user_id.to_string()
            || safe_revision(row.identity_revision).map_err(AdapterFailure::into_port)?
                != command.grant.identity_revision()
            || stored_digest != *command.grant.identifier_digest()
            || stored_expiry != command.grant.expires_at()
        {
            Some(AuthAuditReason::InvalidCredential)
        } else {
            let candidates = SecretDigestCandidates::new(stored_digest.clone(), Vec::new())
                .map_err(|_| AdapterFailure::Corrupt.into_port())?;
            if self
                .principal(user_id)
                .await
                .map_err(AdapterFailure::into_port)?
                .is_some()
                || self
                    .identifier_owner(&candidates)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .is_some()
            {
                Some(AuthAuditReason::InvalidCredential)
            } else {
                None
            }
        };
        if let Some(reason) = reason {
            self.commit_audit(
                &command.audit,
                None,
                Some(user_id),
                AuthAuditOutcome::Deny,
                reason,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(IdentityProvisionOutcome::Rejected(reason));
        }

        // `destination` is deliberately consumed at the application/sealed-delivery
        // boundary. D1 stores only the verified identifier digest and never the
        // raw delivery destination or a reversible derivative of it.
        let _sealed_destination_capability = command.destination;
        let operation_id = receipt.operation_id.clone();
        let user = user_id.to_string();
        let placeholder_email = format!("auth-{user}@invalid.invalid");
        let (identifier_version, identifier_digest) = digest_values(&stored_digest);
        let mut statements = Vec::with_capacity(11);
        self.push_statement(
            &mut statements,
            PROVISIONING_GRANT_DELETE_SQL,
            &[
                JsValue::from_str(&grant_id.to_string()),
                JsValue::from_str(&user),
                JsValue::from_f64(command.grant.identity_revision() as f64),
                identifier_version.clone(),
                identifier_digest.clone(),
                JsValue::from_f64(command.grant.expires_at().get() as f64),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "grant", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            USER_INSERT_SQL,
            &[
                JsValue::from_str(&user),
                JsValue::from_str(&placeholder_email),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "user", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            IDENTITY_INSERT_SQL,
            &[
                JsValue::from_str(&user),
                JsValue::from_f64(command.grant.identity_revision() as f64),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "identity", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            IDENTIFIER_INSERT_SQL,
            &[
                identifier_version,
                identifier_digest,
                JsValue::from_str(&user),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "identifier", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            None,
            Some(user_id),
            AuthAuditOutcome::Allow,
            AuthAuditReason::Issued,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {
                if !self
                    .identity_provision_committed(&receipt, &command.grant)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    return Err(AdapterFailure::Unavailable.into_port());
                }
                telemetry.finish("created", 1);
                Ok(IdentityProvisionOutcome::Created)
            }
            Err(failure @ (AdapterFailure::Conflict | AdapterFailure::Unavailable)) => {
                // A D1 constraint can surface either as an unsuccessful batch
                // result or as a rejected batch promise. Disambiguate a real
                // adapter outage from a concurrent, successfully committed
                // grant spend before returning the replay decision.
                let grant_absent = self
                    .one::<ProvisioningGrantRow>(
                        PROVISIONING_GRANT_SQL,
                        &[JsValue::from_str(&grant_id.to_string())],
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .is_none();
                let candidates = SecretDigestCandidates::new(stored_digest, Vec::new())
                    .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                let committed = grant_absent
                    && self
                        .identity_exists(user_id)
                        .await
                        .map_err(AdapterFailure::into_port)?
                    && self
                        .identifier_owner(&candidates)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        == Some(user_id);
                if !committed {
                    return Err(failure.into_port());
                }
                self.commit_audit(
                    &command.audit,
                    None,
                    Some(user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::ReplayDetected,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("replay_detected", 0);
                Ok(IdentityProvisionOutcome::Rejected(
                    AuthAuditReason::ReplayDetected,
                ))
            }
            Err(failure) => Err(failure.into_port()),
        }
    }

    async fn issue_auth_session(
        &self,
        command: SessionIssueCommand,
    ) -> Result<SessionIssueOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("session_issue");
        require_action(&command.audit, AuthAuditAction::SessionIssue)
            .map_err(AdapterFailure::into_port)?;
        let (authority_kind, authority_id) = match &command.authority {
            SessionIssueAuthority::Verified(grant) => ("issuance", grant.id().to_string()),
            SessionIssueAuthority::ExistingSession(grant) => ("mutation", grant.id().to_string()),
        };
        let semantic = vec![
            serde_json::to_string(&command.principal)
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            serde_json::to_string(&command.session)
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            authority_kind.into(),
            authority_id,
        ];
        let receipt = OperationReceipt::for_audit(
            "session_issue",
            command.session.id.to_string(),
            &command.audit,
            &semantic,
        );
        if self
            .session_issue_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("issued", 1);
            return Ok(SessionIssueOutcome::Issued);
        }
        let principal = self
            .principal(command.principal.user_id)
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some(principal) = principal else {
            self.commit_audit(
                &command.audit,
                None,
                Some(command.principal.user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(SessionIssueOutcome::Denied(
                AuthAuditReason::InvalidCredential,
            ));
        };
        let mut statements = Vec::with_capacity(9);
        let operation_id = receipt.operation_id.clone();
        let authority_valid = match &command.authority {
            SessionIssueAuthority::Verified(grant) => {
                let row = self
                    .one::<IssuanceGrantRow>(
                        ISSUANCE_GRANT_SQL,
                        &[JsValue::from_str(&grant.id().to_string())],
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let valid = row.as_ref().is_some_and(|row| {
                    row.id == grant.id().to_string()
                        && row.user_id == grant.user_id().to_string()
                        && safe_revision(row.identity_revision).ok()
                            == Some(grant.identity_revision())
                        && safe_revision(row.current_identity_revision).ok()
                            == Some(grant.identity_revision())
                        && safe_timestamp(row.expires_at_ms).ok() == Some(grant.expires_at())
                        && command.audit.occurred_at < grant.expires_at()
                });
                if valid {
                    self.push_statement(
                        &mut statements,
                        ISSUANCE_GRANT_DELETE_SQL,
                        &[
                            JsValue::from_str(&grant.id().to_string()),
                            JsValue::from_str(&grant.user_id().to_string()),
                            JsValue::from_f64(grant.identity_revision() as f64),
                            JsValue::from_f64(grant.expires_at().get() as f64),
                            JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_assertion(&mut statements, &operation_id, "authority", 1)
                        .map_err(AdapterFailure::into_port)?;
                }
                valid
            }
            SessionIssueAuthority::ExistingSession(grant) => {
                let validated = self
                    .mutation_grant(grant, command.audit.occurred_at)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let valid = validated
                    .as_ref()
                    .is_some_and(|validated| validated.principal.snapshot == principal.snapshot);
                if valid {
                    let (token_version, token_digest) = digest_values(grant.token_digest());
                    self.push_statement(
                        &mut statements,
                        MUTATION_GRANT_DELETE_SQL,
                        &[
                            JsValue::from_str(&grant.id().to_string()),
                            JsValue::from_str(&grant.session_id().to_string()),
                            JsValue::from_str(&grant.user_id().to_string()),
                            JsValue::from_f64(grant.generation() as f64),
                            token_version,
                            token_digest,
                            JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_assertion(&mut statements, &operation_id, "authority", 1)
                        .map_err(AdapterFailure::into_port)?;
                }
                valid
            }
        };
        if !authority_valid || principal.snapshot != command.principal {
            self.commit_audit(
                &command.audit,
                None,
                Some(command.principal.user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(SessionIssueOutcome::Denied(
                AuthAuditReason::InvalidCredential,
            ));
        }
        let mut session = command.session.clone();
        session.session_version = principal.session_version;
        if session.user_id != principal.snapshot.user_id
            || session.state != AuthSessionState::Active
            || session.generation != 0
            || session.rotated_at != session.issued_at
            || session.revoked_at.is_some()
            || session.revocation_reason.is_some()
            || session.idle_expires_at <= session.issued_at
            || session.idle_expires_at > session.absolute_expires_at
            || (session.client_kind == AuthClientKind::Browser)
                != (session.csrf_digest.is_some() && session.browser_origin.is_some())
        {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let (token_version, token_digest) = digest_values(&session.token_digest);
        let (csrf_version, csrf_digest) = session
            .csrf_digest
            .as_ref()
            .map_or((JsValue::NULL, JsValue::NULL), |digest| {
                digest_values(digest)
            });
        self.push_identity_current_assertion(&mut statements, &operation_id, &principal)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            SESSION_INSERT_SQL,
            &[
                JsValue::from_str(&session.id.to_string()),
                JsValue::from_str(&session.family_id.to_string()),
                JsValue::from_str(&session.user_id.to_string()),
                JsValue::from_str(
                    &enum_name(session.client_kind).map_err(AdapterFailure::into_port)?,
                ),
                token_version.clone(),
                token_digest.clone(),
                csrf_version,
                csrf_digest,
                opt_string(
                    session
                        .browser_origin
                        .as_ref()
                        .map(ExactBrowserOrigin::as_str),
                ),
                JsValue::from_f64(session.issued_at.get() as f64),
                JsValue::from_f64(session.idle_expires_at.get() as f64),
                JsValue::from_f64(session.absolute_expires_at.get() as f64),
                JsValue::from_f64(principal.session_version as f64),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "session", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            CREDENTIAL_INSERT_SQL,
            &[
                token_version,
                token_digest,
                JsValue::from_str(&session.id.to_string()),
                JsValue::from_str(&session.family_id.to_string()),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "credential", 1)
            .map_err(AdapterFailure::into_port)?;
        let context = SessionAuditContext::from(&session);
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            Some(context),
            Some(principal.snapshot.user_id),
            AuthAuditOutcome::Allow,
            AuthAuditReason::Issued,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {}
            Err(AdapterFailure::Conflict) => {
                self.commit_audit(
                    &command.audit,
                    None,
                    Some(command.principal.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("denied", 0);
                return Ok(SessionIssueOutcome::Denied(
                    AuthAuditReason::InvalidCredential,
                ));
            }
            Err(failure) => return Err(failure.into_port()),
        }
        if !self
            .session_issue_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            return Err(AdapterFailure::Unavailable.into_port());
        }
        telemetry.finish("issued", 1);
        Ok(SessionIssueOutcome::Issued)
    }

    async fn authenticate_session(
        &self,
        command: SessionAuthenticationCommand,
    ) -> Result<SessionPresentation, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("session_authenticate");
        let expected_action = if command.browser_boundary.is_some() {
            AuthAuditAction::BrowserMutationAuthenticate
        } else {
            AuthAuditAction::SessionAuthenticate
        };
        require_action(&command.audit, expected_action).map_err(AdapterFailure::into_port)?;
        let boundary_semantic = if let Some(boundary) = &command.browser_boundary {
            format!(
                "{}|{}|{}|{}",
                boundary
                    .origin
                    .as_ref()
                    .map_or("", ExactBrowserOrigin::as_str),
                fetch_site_name(boundary.fetch_site),
                candidates_json(&boundary.csrf_cookie_digests)
                    .map_err(AdapterFailure::into_port)?,
                candidates_json(&boundary.csrf_header_digests)
                    .map_err(AdapterFailure::into_port)?,
            )
        } else {
            String::new()
        };
        let semantic = vec![
            candidates_json(&command.token_digests).map_err(AdapterFailure::into_port)?,
            boundary_semantic,
        ];
        let base_receipt = OperationReceipt::for_audit(
            "session_authenticate",
            command
                .token_digests
                .active()
                .digest
                .expose_for_verification(),
            &command.audit,
            &semantic,
        );
        if let Some(presentation) = self
            .session_authentication_committed(&base_receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish(
                "replayed",
                usize::from(matches!(
                    presentation,
                    SessionPresentation::Authenticated(_)
                )),
            );
            return Ok(presentation);
        }
        let operation_id = base_receipt.operation_id.clone();
        let credential = self
            .session_by_credentials(&command.token_digests)
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some(mut credential) = credential else {
            let candidates =
                candidates_json(&command.token_digests).map_err(AdapterFailure::into_port)?;
            let mut statements = Vec::with_capacity(3);
            self.push_statement(
                &mut statements,
                CREDENTIALS_ABSENT_ASSERT_SQL,
                &[
                    JsValue::from_str(&candidates),
                    JsValue::from_str(&format!("{operation_id}:absent")),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                None,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            let receipt = base_receipt.clone().with_result_code("unknown");
            let presentation = self
                .commit_session_authentication(statements, &receipt, &command)
                .await
                .map_err(AdapterFailure::into_port)?;
            telemetry.finish("unknown", 0);
            return Ok(presentation);
        };
        let context = SessionAuditContext::from(&credential.session.record);

        if !credential.session.user_active {
            let mut statements = Vec::with_capacity(3);
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::Revoked,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            let receipt = base_receipt.clone().with_result_code("revoked");
            let presentation = self
                .commit_session_authentication(statements, &receipt, &command)
                .await
                .map_err(AdapterFailure::into_port)?;
            telemetry.finish("revoked", 0);
            return Ok(presentation);
        }

        if credential.credential_state == CredentialState::Rotated {
            let family_id = credential.session.record.family_id.to_string();
            let (active_sessions, live_credentials) = self
                .aggregate_counts(SESSION_FAMILY_COUNTS_SQL, &family_id)
                .await
                .map_err(AdapterFailure::into_port)?;
            let mut statements = Vec::with_capacity(13);
            self.push_statement(
                &mut statements,
                SESSION_REVOKE_FAMILY_SQL,
                &[
                    JsValue::from_str(&family_id),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str("replay_detected"),
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(
                &mut statements,
                &operation_id,
                "family_sessions",
                active_sessions,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_statement(
                &mut statements,
                CREDENTIALS_REVOKE_FAMILY_SQL,
                &[
                    JsValue::from_str(&family_id),
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(
                &mut statements,
                &operation_id,
                "family_credentials",
                live_credentials,
            )
            .map_err(AdapterFailure::into_port)?;
            for sql in [
                MUTATION_GRANTS_DELETE_FAMILY_SQL,
                PENDING_CONTINUATIONS_DELETE_FAMILY_SQL,
                CHALLENGE_CONTINUATIONS_DELETE_FAMILY_SQL,
                DELIVERY_CONTINUATIONS_DELETE_FAMILY_SQL,
            ] {
                self.push_statement(&mut statements, sql, &[JsValue::from_str(&family_id)])
                    .map_err(AdapterFailure::into_port)?;
            }
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::ReplayDetected,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            let receipt = base_receipt
                .clone()
                .with_result_code("replay_family_revoked");
            let presentation = self
                .commit_session_authentication(statements, &receipt, &command)
                .await
                .map_err(AdapterFailure::into_port)?;
            telemetry.finish("replay_detected", 0);
            return Ok(presentation);
        }
        if credential.credential_state == CredentialState::Revoked {
            let mut statements = Vec::with_capacity(3);
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::Revoked,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            let receipt = base_receipt.clone().with_result_code("revoked");
            let presentation = self
                .commit_session_authentication(statements, &receipt, &command)
                .await
                .map_err(AdapterFailure::into_port)?;
            telemetry.finish("revoked", 0);
            return Ok(presentation);
        }

        let decision = credential.session.record.evaluate(
            command.audit.occurred_at,
            credential.session.current_session_version,
        );
        if decision != AuthSessionDecision::Authenticated {
            let (reason, revoke_reason) = match decision {
                AuthSessionDecision::Expired => (
                    AuthAuditReason::Expired,
                    Some(SessionRevocationReason::Expired),
                ),
                AuthSessionDecision::Revoked => (AuthAuditReason::Revoked, None),
                AuthSessionDecision::SessionVersionMismatch => (
                    AuthAuditReason::SessionVersionMismatch,
                    Some(SessionRevocationReason::SessionVersionChanged),
                ),
                AuthSessionDecision::Authenticated => unreachable!(),
            };
            if let Some(revoke_reason) = revoke_reason {
                let count = self
                    .one::<CountRow>(
                        SESSION_CREDENTIAL_COUNT_SQL,
                        &[JsValue::from_str(&credential.session.record.id.to_string())],
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
                let live_credentials = usize::try_from(count.bucket_count)
                    .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                let mut statements = Vec::with_capacity(12);
                self.push_statement(
                    &mut statements,
                    SESSION_REVOKE_ONE_SQL,
                    &[
                        JsValue::from_str(&credential.session.record.id.to_string()),
                        JsValue::from_f64(credential.session.row_revision as f64),
                        JsValue::from_f64(credential.session.record.generation as f64),
                        JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        JsValue::from_str(
                            &enum_name(revoke_reason).map_err(AdapterFailure::into_port)?,
                        ),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "session", 1)
                    .map_err(AdapterFailure::into_port)?;
                self.push_statement(
                    &mut statements,
                    CREDENTIALS_REVOKE_SESSION_SQL,
                    &[
                        JsValue::from_str(&credential.session.record.id.to_string()),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(
                    &mut statements,
                    &operation_id,
                    "credentials",
                    live_credentials,
                )
                .map_err(AdapterFailure::into_port)?;
                for sql in [
                    MUTATION_GRANTS_DELETE_SESSION_SQL,
                    SESSION_CONTINUATIONS_DELETE_SQL,
                    CHALLENGE_CONTINUATIONS_DELETE_SQL,
                    DELIVERY_CONTINUATIONS_DELETE_SQL,
                ] {
                    self.push_statement(
                        &mut statements,
                        sql,
                        &[JsValue::from_str(&credential.session.record.id.to_string())],
                    )
                    .map_err(AdapterFailure::into_port)?;
                }
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    Some(context),
                    None,
                    AuthAuditOutcome::Deny,
                    reason,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let result_code = match reason {
                    AuthAuditReason::Expired => "expired",
                    AuthAuditReason::SessionVersionMismatch => "version_mismatch",
                    _ => return Err(AdapterFailure::Corrupt.into_port()),
                };
                let receipt = base_receipt.clone().with_result_code(result_code);
                let committed = self
                    .commit_session_authentication(statements, &receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                telemetry.finish(result_code, 0);
                return Ok(committed);
            } else {
                let mut statements = Vec::with_capacity(3);
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    Some(context),
                    None,
                    AuthAuditOutcome::Deny,
                    reason,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let receipt = base_receipt.clone().with_result_code("revoked");
                let committed = self
                    .commit_session_authentication(statements, &receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                telemetry.finish("revoked", 0);
                return Ok(committed);
            }
        }

        let boundary_reason = command.browser_boundary.as_ref().and_then(|boundary| {
            if credential.session.record.client_kind != AuthClientKind::Browser {
                Some(AuthAuditReason::InvalidCredential)
            } else if boundary.origin.as_ref() != credential.session.record.browser_origin.as_ref()
            {
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
                let stored_matches =
                    credential
                        .session
                        .record
                        .csrf_digest
                        .as_ref()
                        .is_some_and(|stored| {
                            boundary
                                .csrf_header_digests
                                .iter()
                                .any(|candidate| candidate == stored)
                        });
                (!cookie_matches_header || !stored_matches).then_some(AuthAuditReason::CsrfMismatch)
            }
        });
        if let Some(reason) = boundary_reason {
            let mut statements = Vec::with_capacity(3);
            self.push_current_session_assertion(
                &mut statements,
                &operation_id,
                &credential,
                command.audit.occurred_at,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Deny,
                reason,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            let result_code = match reason {
                AuthAuditReason::InvalidCredential => "boundary_invalid",
                AuthAuditReason::OriginMismatch => "boundary_origin",
                AuthAuditReason::FetchMetadataMismatch => "boundary_fetch",
                AuthAuditReason::CsrfMismatch => "boundary_csrf",
                _ => return Err(AdapterFailure::Corrupt.into_port()),
            };
            let receipt = base_receipt.clone().with_result_code(result_code);
            let presentation = self
                .commit_session_authentication(statements, &receipt, &command)
                .await
                .map_err(AdapterFailure::into_port)?;
            telemetry.finish("boundary_rejected", 0);
            return Ok(presentation);
        }

        let Some(principal) = self
            .principal(credential.session.record.user_id)
            .await
            .map_err(AdapterFailure::into_port)?
        else {
            let mut statements = Vec::with_capacity(3);
            self.push_audit(
                &mut statements,
                &operation_id,
                &command.audit,
                Some(context),
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::Revoked,
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            let receipt = base_receipt.clone().with_result_code("revoked");
            let presentation = self
                .commit_session_authentication(statements, &receipt, &command)
                .await
                .map_err(AdapterFailure::into_port)?;
            telemetry.finish("revoked", 0);
            return Ok(presentation);
        };
        let mut statements = Vec::with_capacity(10);
        let active_token = command.token_digests.active().clone();
        let next_csrf = command
            .browser_boundary
            .as_ref()
            .map(|boundary| boundary.csrf_header_digests.active().clone());
        let token_migrates = credential.matched != active_token;
        let csrf_migrates = next_csrf
            .as_ref()
            .is_some_and(|next| credential.session.record.csrf_digest.as_ref() != Some(next));
        if token_migrates {
            let (next_token_version, next_token_digest) = digest_values(&active_token);
            let (next_csrf_version, next_csrf_digest) = next_csrf
                .as_ref()
                .map_or((JsValue::NULL, JsValue::NULL), |digest| {
                    digest_values(digest)
                });
            self.push_statement(
                &mut statements,
                SESSION_DIGEST_MIGRATE_SQL,
                &[
                    JsValue::from_str(&credential.session.record.id.to_string()),
                    JsValue::from_f64(credential.session.row_revision as f64),
                    JsValue::from_f64(credential.session.record.generation as f64),
                    next_token_version.clone(),
                    next_token_digest.clone(),
                    next_csrf_version,
                    next_csrf_digest,
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(&mut statements, &operation_id, "session", 1)
                .map_err(AdapterFailure::into_port)?;
            let (matched_version, matched_digest) = digest_values(&credential.matched);
            self.push_statement(
                &mut statements,
                CREDENTIAL_DIGEST_MIGRATE_SQL,
                &[
                    matched_version,
                    matched_digest,
                    JsValue::from_str(&credential.session.record.id.to_string()),
                    JsValue::from_f64(credential.credential_revision as f64),
                    next_token_version,
                    next_token_digest,
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(&mut statements, &operation_id, "credential", 1)
                .map_err(AdapterFailure::into_port)?;
            credential.session.record.token_digest = active_token.clone();
            credential.matched = active_token.clone();
            credential.session.row_revision = credential.session.row_revision.saturating_add(1);
            credential.credential_revision = credential.credential_revision.saturating_add(1);
            if let Some(next) = next_csrf.clone() {
                credential.session.record.csrf_digest = Some(next);
            }
        } else if csrf_migrates {
            let next = next_csrf
                .clone()
                .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
            let (version, digest) = digest_values(&next);
            self.push_statement(
                &mut statements,
                SESSION_CSRF_UPDATE_SQL,
                &[
                    JsValue::from_str(&credential.session.record.id.to_string()),
                    JsValue::from_f64(credential.session.row_revision as f64),
                    JsValue::from_f64(credential.session.record.generation as f64),
                    version,
                    digest,
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(&mut statements, &operation_id, "session", 1)
                .map_err(AdapterFailure::into_port)?;
            credential.session.record.csrf_digest = Some(next);
            credential.session.row_revision = credential.session.row_revision.saturating_add(1);
        } else {
            self.push_current_session_assertion(
                &mut statements,
                &operation_id,
                &credential,
                command.audit.occurred_at,
            )
            .map_err(AdapterFailure::into_port)?;
        }

        self.push_identity_current_assertion(&mut statements, &operation_id, &principal)
            .map_err(AdapterFailure::into_port)?;
        if command.browser_boundary.is_some() {
            self.push_statement(
                &mut statements,
                MUTATION_GRANTS_DELETE_SESSION_SQL,
                &[JsValue::from_str(&credential.session.record.id.to_string())],
            )
            .map_err(AdapterFailure::into_port)?;
            let grant_id =
                SessionMutationGrantId::parse(&stable_child_uuid(&operation_id, "mutation-grant"))
                    .map_err(|_| AdapterFailure::Corrupt.into_port())?;
            let (token_version, token_digest) = digest_values(&active_token);
            self.push_statement(
                &mut statements,
                MUTATION_GRANT_INSERT_SQL,
                &[
                    JsValue::from_str(&grant_id.to_string()),
                    JsValue::from_str(&credential.session.record.id.to_string()),
                    JsValue::from_str(&credential.session.record.user_id.to_string()),
                    JsValue::from_f64(credential.session.record.generation as f64),
                    token_version,
                    token_digest,
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(&mut statements, &operation_id, "mutation_grant", 1)
                .map_err(AdapterFailure::into_port)?;
        }
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            Some(context),
            None,
            AuthAuditOutcome::Allow,
            if token_migrates {
                AuthAuditReason::KeyVersionMigrated
            } else {
                AuthAuditReason::Authenticated
            },
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        let result_code = if token_migrates {
            "token_migrated"
        } else if csrf_migrates {
            "csrf_migrated"
        } else {
            "authenticated"
        };
        let receipt = base_receipt.clone().with_result_code(result_code);
        let presentation = self
            .commit_session_authentication(statements, &receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?;
        telemetry.finish(
            if token_migrates || csrf_migrates {
                "migrated"
            } else {
                "authenticated"
            },
            1,
        );
        Ok(presentation)
    }

    async fn rotate_auth_session(
        &self,
        request: SessionRotationRequest,
    ) -> Result<SessionRotationOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("session_rotate");
        require_action(&request.audit, AuthAuditAction::SessionRotate)
            .map_err(AdapterFailure::into_port)?;
        if request.now != request.audit.occurred_at {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let semantic = vec![
            request.grant.id().to_string(),
            request.grant.session_id().to_string(),
            request.grant.generation().to_string(),
            serde_json::to_string(&DigestWire::from(request.grant.token_digest()))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            serde_json::to_string(&DigestWire::from(&request.next_token_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            serde_json::to_string(&DigestWire::from(&request.next_csrf_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            request.idle_expires_at.get().to_string(),
        ];
        let receipt = OperationReceipt::for_audit(
            "session_rotate",
            request.grant.session_id().to_string(),
            &request.audit,
            &semantic,
        );
        if let Some(rotated) = self
            .session_rotation_committed(&receipt, &request)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("rotated", 1);
            return Ok(SessionRotationOutcome::Rotated(Box::new(rotated)));
        }
        let existing = self
            .session_by_id(request.grant.session_id())
            .await
            .map_err(AdapterFailure::into_port)?;
        let context = existing
            .as_ref()
            .map(|state| SessionAuditContext::from(&state.record));
        let Some(validated) = self
            .mutation_grant(&request.grant, request.audit.occurred_at)
            .await
            .map_err(AdapterFailure::into_port)?
        else {
            self.commit_audit(
                &request.audit,
                context,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(SessionRotationOutcome::Denied(
                AuthAuditReason::InvalidCredential,
            ));
        };
        let next_candidates =
            SecretDigestCandidates::new(request.next_token_digest.clone(), vec![])
                .map_err(|_| AdapterFailure::Invalid.into_port())?;
        if self
            .session_by_credentials(&next_candidates)
            .await
            .map_err(AdapterFailure::into_port)?
            .is_some()
        {
            self.commit_audit(
                &request.audit,
                context,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(SessionRotationOutcome::Denied(
                AuthAuditReason::InvalidCredential,
            ));
        }
        let mut rotated = validated.session.record.clone();
        let next_csrf_digest = (rotated.client_kind == AuthClientKind::Browser)
            .then(|| request.next_csrf_digest.clone());
        rotated
            .rotate(
                request.grant.generation(),
                request.next_token_digest.clone(),
                next_csrf_digest,
                request.now,
                request.idle_expires_at.min(rotated.absolute_expires_at),
            )
            .map_err(|_| AdapterFailure::Invalid.into_port())?;
        let operation_id = receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(15);
        let (old_version, old_digest) = digest_values(request.grant.token_digest());
        self.push_statement(
            &mut statements,
            MUTATION_GRANT_DELETE_SQL,
            &[
                JsValue::from_str(&request.grant.id().to_string()),
                JsValue::from_str(&request.grant.session_id().to_string()),
                JsValue::from_str(&request.grant.user_id().to_string()),
                JsValue::from_f64(request.grant.generation() as f64),
                old_version.clone(),
                old_digest.clone(),
                JsValue::from_f64(request.now.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "grant", 1)
            .map_err(AdapterFailure::into_port)?;
        let (next_version, next_digest) = digest_values(&request.next_token_digest);
        let (next_csrf_version, next_csrf_digest) =
            if rotated.client_kind == AuthClientKind::Browser {
                digest_values(&request.next_csrf_digest)
            } else {
                (JsValue::NULL, JsValue::NULL)
            };
        self.push_statement(
            &mut statements,
            SESSION_ROTATE_SQL,
            &[
                JsValue::from_str(&rotated.id.to_string()),
                JsValue::from_f64(validated.session_revision as f64),
                JsValue::from_f64(request.grant.generation() as f64),
                next_version.clone(),
                next_digest.clone(),
                next_csrf_version,
                next_csrf_digest,
                JsValue::from_f64(request.now.get() as f64),
                JsValue::from_f64(rotated.idle_expires_at.get() as f64),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "session", 1)
            .map_err(AdapterFailure::into_port)?;
        // The current credential must still be exactly the grant-bound row.
        let credential = self
            .session_by_credentials(
                &SecretDigestCandidates::new(request.grant.token_digest().clone(), vec![])
                    .map_err(|_| AdapterFailure::Invalid.into_port())?,
            )
            .await
            .map_err(AdapterFailure::into_port)?
            .ok_or_else(|| AdapterFailure::Conflict.into_port())?;
        self.push_statement(
            &mut statements,
            CREDENTIAL_MARK_ROTATED_SQL,
            &[
                old_version,
                old_digest,
                JsValue::from_str(&rotated.id.to_string()),
                JsValue::from_f64(credential.credential_revision as f64),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "old_credential", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            CREDENTIAL_INSERT_SQL,
            &[
                next_version,
                next_digest,
                JsValue::from_str(&rotated.id.to_string()),
                JsValue::from_str(&rotated.family_id.to_string()),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "new_credential", 1)
            .map_err(AdapterFailure::into_port)?;
        for sql in [
            MUTATION_GRANTS_DELETE_SESSION_SQL,
            SESSION_CONTINUATIONS_DELETE_SQL,
            CHALLENGE_CONTINUATIONS_DELETE_SQL,
            DELIVERY_CONTINUATIONS_DELETE_SQL,
        ] {
            self.push_statement(
                &mut statements,
                sql,
                &[JsValue::from_str(&rotated.id.to_string())],
            )
            .map_err(AdapterFailure::into_port)?;
        }
        self.push_identity_current_assertion(&mut statements, &operation_id, &validated.principal)
            .map_err(AdapterFailure::into_port)?;
        self.push_audit(
            &mut statements,
            &operation_id,
            &request.audit,
            Some(SessionAuditContext::from(&validated.session.record)),
            None,
            AuthAuditOutcome::Allow,
            AuthAuditReason::Rotated,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {
                let committed = self
                    .session_rotation_committed(&receipt, &request)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                telemetry.finish("rotated", 1);
                Ok(SessionRotationOutcome::Rotated(Box::new(committed)))
            }
            Err(AdapterFailure::Conflict) => {
                self.commit_audit(
                    &request.audit,
                    context,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("denied", 0);
                Ok(SessionRotationOutcome::Denied(
                    AuthAuditReason::InvalidCredential,
                ))
            }
            Err(failure) => Err(failure.into_port()),
        }
    }

    async fn revoke_auth_session(&self, command: SessionRevokeCommand) -> Result<bool, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("session_revoke");
        require_action(&command.audit, AuthAuditAction::Logout)
            .map_err(AdapterFailure::into_port)?;
        let semantic = vec![
            command.grant.id().to_string(),
            command.grant.session_id().to_string(),
            command.grant.generation().to_string(),
            enum_name(command.reason).map_err(AdapterFailure::into_port)?,
        ];
        let receipt = OperationReceipt::for_audit(
            "session_revoke",
            command.grant.session_id().to_string(),
            &command.audit,
            &semantic,
        );
        if self
            .session_revoke_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("revoked", 1);
            return Ok(true);
        }
        let existing = self
            .session_by_id(command.grant.session_id())
            .await
            .map_err(AdapterFailure::into_port)?;
        let context = existing
            .as_ref()
            .map(|state| SessionAuditContext::from(&state.record));
        let Some(validated) = self
            .mutation_grant(&command.grant, command.audit.occurred_at)
            .await
            .map_err(AdapterFailure::into_port)?
        else {
            self.commit_audit(
                &command.audit,
                context,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(false);
        };
        let count = self
            .one::<CountRow>(
                SESSION_CREDENTIAL_COUNT_SQL,
                &[JsValue::from_str(&command.grant.session_id().to_string())],
            )
            .await
            .map_err(AdapterFailure::into_port)?
            .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
        let live_credentials =
            usize::try_from(count.bucket_count).map_err(|_| AdapterFailure::Corrupt.into_port())?;
        let operation_id = receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(14);
        let (token_version, token_digest) = digest_values(command.grant.token_digest());
        self.push_statement(
            &mut statements,
            MUTATION_GRANT_DELETE_SQL,
            &[
                JsValue::from_str(&command.grant.id().to_string()),
                JsValue::from_str(&command.grant.session_id().to_string()),
                JsValue::from_str(&command.grant.user_id().to_string()),
                JsValue::from_f64(command.grant.generation() as f64),
                token_version,
                token_digest,
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "grant", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            SESSION_REVOKE_ONE_SQL,
            &[
                JsValue::from_str(&command.grant.session_id().to_string()),
                JsValue::from_f64(validated.session_revision as f64),
                JsValue::from_f64(command.grant.generation() as f64),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
                JsValue::from_str(&enum_name(command.reason).map_err(AdapterFailure::into_port)?),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "session", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            CREDENTIALS_REVOKE_SESSION_SQL,
            &[
                JsValue::from_str(&command.grant.session_id().to_string()),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(
            &mut statements,
            &operation_id,
            "credentials",
            live_credentials,
        )
        .map_err(AdapterFailure::into_port)?;
        for sql in [
            MUTATION_GRANTS_DELETE_SESSION_SQL,
            SESSION_CONTINUATIONS_DELETE_SQL,
            CHALLENGE_CONTINUATIONS_DELETE_SQL,
            DELIVERY_CONTINUATIONS_DELETE_SQL,
        ] {
            self.push_statement(
                &mut statements,
                sql,
                &[JsValue::from_str(&command.grant.session_id().to_string())],
            )
            .map_err(AdapterFailure::into_port)?;
        }
        self.push_identity_current_assertion(&mut statements, &operation_id, &validated.principal)
            .map_err(AdapterFailure::into_port)?;
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            Some(SessionAuditContext::from(&validated.session.record)),
            None,
            AuthAuditOutcome::Allow,
            AuthAuditReason::LoggedOut,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {
                if !self
                    .session_revoke_committed(&receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    return Err(AdapterFailure::Unavailable.into_port());
                }
                telemetry.finish("revoked", 1);
                Ok(true)
            }
            Err(AdapterFailure::Conflict) => {
                self.commit_audit(
                    &command.audit,
                    context,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("denied", 0);
                Ok(false)
            }
            Err(failure) => Err(failure.into_port()),
        }
    }

    async fn revoke_all_auth_sessions(
        &self,
        command: SessionRevokeCommand,
    ) -> Result<LogoutAllOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("session_logout_all");
        require_action(&command.audit, AuthAuditAction::LogoutAll)
            .map_err(AdapterFailure::into_port)?;
        if command.reason != SessionRevocationReason::LogoutAll {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let semantic = vec![
            command.grant.id().to_string(),
            command.grant.session_id().to_string(),
            command.grant.user_id().to_string(),
            command.grant.generation().to_string(),
            "logout_all".into(),
        ];
        let receipt = OperationReceipt::for_audit(
            "session_logout_all",
            command.grant.user_id().to_string(),
            &command.audit,
            &semantic,
        );
        if let Some((new_session_version, revoked_sessions)) = self
            .logout_all_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("revoked", revoked_sessions as usize);
            return Ok(LogoutAllOutcome::Revoked {
                new_session_version,
                revoked_sessions,
            });
        }
        let existing = self
            .session_by_id(command.grant.session_id())
            .await
            .map_err(AdapterFailure::into_port)?;
        let context = existing
            .as_ref()
            .map(|state| SessionAuditContext::from(&state.record));
        let Some(validated) = self
            .mutation_grant(&command.grant, command.audit.occurred_at)
            .await
            .map_err(AdapterFailure::into_port)?
        else {
            self.commit_audit(
                &command.audit,
                context,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(LogoutAllOutcome::Denied(AuthAuditReason::InvalidCredential));
        };
        let user_id = validated.session.record.user_id;
        let (active_sessions, live_credentials) = self
            .aggregate_counts(SESSION_USER_COUNTS_SQL, &user_id.to_string())
            .await
            .map_err(AdapterFailure::into_port)?;
        let new_session_version = validated
            .principal
            .session_version
            .checked_add(1)
            .filter(|value| *value <= 9_007_199_254_740_991)
            .ok_or_else(|| AdapterFailure::Invalid.into_port())?;
        let operation_id = receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(16);
        let (token_version, token_digest) = digest_values(command.grant.token_digest());
        self.push_statement(
            &mut statements,
            MUTATION_GRANT_DELETE_SQL,
            &[
                JsValue::from_str(&command.grant.id().to_string()),
                JsValue::from_str(&command.grant.session_id().to_string()),
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_f64(command.grant.generation() as f64),
                token_version,
                token_digest,
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "grant", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            IDENTITY_SESSION_VERSION_INCREMENT_SQL,
            &[
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_f64(validated.principal.row_revision as f64),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "identity", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            SESSION_REVOKE_USER_SQL,
            &[
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_f64(command.audit.occurred_at.get() as f64),
                JsValue::from_str("logout_all"),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "sessions", active_sessions)
            .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            CREDENTIALS_REVOKE_USER_SQL,
            &[
                JsValue::from_str(&user_id.to_string()),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(
            &mut statements,
            &operation_id,
            "credentials",
            live_credentials,
        )
        .map_err(AdapterFailure::into_port)?;
        for sql in [
            MUTATION_GRANTS_DELETE_USER_SQL,
            PENDING_CONTINUATIONS_DELETE_USER_SQL,
            CHALLENGE_CONTINUATIONS_DELETE_USER_SQL,
            DELIVERY_CONTINUATIONS_DELETE_USER_SQL,
        ] {
            self.push_statement(
                &mut statements,
                sql,
                &[JsValue::from_str(&user_id.to_string())],
            )
            .map_err(AdapterFailure::into_port)?;
        }
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            Some(SessionAuditContext::from(&validated.session.record)),
            None,
            AuthAuditOutcome::Allow,
            AuthAuditReason::LoggedOutAll,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {
                let (committed_version, committed_sessions) = self
                    .logout_all_committed(&receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                if committed_version != new_session_version
                    || committed_sessions != active_sessions as u64
                {
                    return Err(AdapterFailure::Unavailable.into_port());
                }
                telemetry.finish("revoked", committed_sessions as usize);
                Ok(LogoutAllOutcome::Revoked {
                    new_session_version: committed_version,
                    revoked_sessions: committed_sessions,
                })
            }
            Err(AdapterFailure::Conflict) => {
                self.commit_audit(
                    &command.audit,
                    context,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InvalidCredential,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("denied", 0);
                Ok(LogoutAllOutcome::Denied(AuthAuditReason::InvalidCredential))
            }
            Err(failure) => Err(failure.into_port()),
        }
    }

    async fn issue_verification(
        &self,
        command: VerificationIssueCommand,
    ) -> Result<VerificationIssueAtomicOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("verification_issue");
        require_action(&command.audit, AuthAuditAction::VerificationIssue)
            .map_err(AdapterFailure::into_port)?;
        if command.max_attempts == 0
            || command.max_attempts > 100
            || command.expires_at <= command.sealed_delivery.created_at
            || command.sealed_delivery.sealed_payload().len() > 65_536
        {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let payload_fingerprint = bytes_to_hex(&stable_hash(&[
            b"frame/auth/sealed-delivery/v1",
            command.sealed_delivery.sealed_payload(),
        ]));
        let semantic = vec![
            candidates_json(&command.identifier_digests).map_err(AdapterFailure::into_port)?,
            serde_json::to_string(&DigestWire::from(&command.secret_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            enum_name(command.purpose).map_err(AdapterFailure::into_port)?,
            enum_name(command.channel).map_err(AdapterFailure::into_port)?,
            command
                .initiated_by
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| AdapterFailure::Invalid.into_port())?
                .unwrap_or_default(),
            command
                .initiator_grant
                .as_ref()
                .map(|grant| grant.id().to_string())
                .unwrap_or_default(),
            command
                .provisioning
                .map(|intent| format!("{}:{}", intent.user_id, intent.identity_revision))
                .unwrap_or_default(),
            command.max_attempts.to_string(),
            command.expires_at.get().to_string(),
            payload_fingerprint,
        ];
        let receipt = OperationReceipt::for_audit(
            "verification_issue",
            command.sealed_delivery.id.to_string(),
            &command.audit,
            &semantic,
        );
        for attempt in 0..3 {
            let outcome: Result<VerificationIssueAtomicOutcome, PortError> = async {
                if let Some(outcome) = self
                    .verification_issue_outcome_committed(&receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    telemetry.finish(
                        match outcome {
                            VerificationIssueAtomicOutcome::Accepted => "accepted",
                            VerificationIssueAtomicOutcome::RateLimited { .. } => "rate_limited",
                            VerificationIssueAtomicOutcome::Rejected(_) => "rejected",
                        },
                        usize::from(outcome == VerificationIssueAtomicOutcome::Accepted),
                    );
                    return Ok(outcome);
                }
                let operation_id = receipt.operation_id.clone();
                let mut statements = Vec::with_capacity(28);
                let mut authority_rejected = false;
                let mut initiator_fence = None;
                let initiator = match (&command.initiated_by, &command.initiator_grant) {
                    (Some(claimed), Some(grant)) => {
                        let authoritative = self
                            .principal(claimed.user_id)
                            .await
                            .map_err(AdapterFailure::into_port)?;
                        let validated = self
                            .mutation_grant(grant, command.audit.occurred_at)
                            .await
                            .map_err(AdapterFailure::into_port)?;
                        match (authoritative, validated) {
                            (Some(authoritative), Some(validated))
                                if authoritative.snapshot == *claimed
                                    && validated.principal.snapshot == authoritative.snapshot =>
                            {
                                initiator_fence = Some(authoritative.clone());
                                Some((
                                    authoritative.snapshot,
                                    SessionContinuationBinding {
                                        session_id: validated.session.record.id,
                                        user_id: validated.session.record.user_id,
                                        generation: validated.session.record.generation,
                                    },
                                ))
                            }
                            _ => {
                                authority_rejected = true;
                                None
                            }
                        }
                    }
                    (None, None) => None,
                    _ => {
                        authority_rejected = true;
                        None
                    }
                };
                let valid_shape = match command.purpose {
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
                if authority_rejected || !valid_shape {
                    self.push_audit(
                        &mut statements,
                        &operation_id,
                        &command.audit,
                        None,
                        audit_user,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let rejected_receipt = receipt.clone().with_result_code("rejected_invalid");
                    self.batch_mutation(statements, &rejected_receipt)
                        .await
                        .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .verification_issue_outcome_committed(&rejected_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
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
                if let Some(retry_at) = self
                    .append_rate_limit_plan(
                        &mut statements,
                        &operation_id,
                        action,
                        &command.abuse,
                        command.rate_policy,
                        command.audit.occurred_at,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    self.push_audit(
                        &mut statements,
                        &operation_id,
                        &command.audit,
                        None,
                        audit_user,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let limited_receipt = receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at);
                    self.batch_mutation(statements, &limited_receipt)
                        .await
                        .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .verification_issue_outcome_committed(&limited_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rate_limited", 0);
                    return Ok(outcome);
                }
                if let Some(grant) = command.initiator_grant.as_ref() {
                    self.push_mutation_grant_consume(
                        &mut statements,
                        &operation_id,
                        grant,
                        command.audit.occurred_at,
                    )
                    .map_err(AdapterFailure::into_port)?;
                }
                let identifier_json = candidates_json(&command.identifier_digests)
                    .map_err(AdapterFailure::into_port)?;
                let purpose = enum_name(command.purpose).map_err(AdapterFailure::into_port)?;
                self.push_statement(
                    &mut statements,
                    CHALLENGES_REVOKE_MATCHING_SQL,
                    &[
                        JsValue::from_str(&purpose),
                        JsValue::from_str(&identifier_json),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_statement(
                    &mut statements,
                    PENDING_REVOKE_MATCHING_SQL,
                    &[
                        JsValue::from_str(&purpose),
                        JsValue::from_str(&identifier_json),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                let initiator_binding = initiator.as_ref().map(|(_, binding)| *binding);
                let provisioning = command.provisioning;
                let active_identifier = command.identifier_digests.active();
                let (identifier_version, identifier_digest) = digest_values(active_identifier);
                let (secret_version, secret_digest) = digest_values(&command.secret_digest);
                self.push_statement(
                    &mut statements,
                    PENDING_INSERT_SQL,
                    &[
                        JsValue::from_str(&command.sealed_delivery.id.to_string()),
                        JsValue::from_str(&identifier_json),
                        identifier_version,
                        identifier_digest,
                        secret_version,
                        secret_digest,
                        JsValue::from_str(&purpose),
                        JsValue::from_str(
                            &enum_name(command.channel).map_err(AdapterFailure::into_port)?,
                        ),
                        opt_string(
                            initiator_binding
                                .as_ref()
                                .map(|binding| binding.session_id.to_string())
                                .as_deref(),
                        ),
                        opt_string(
                            initiator_binding
                                .as_ref()
                                .map(|binding| binding.user_id.to_string())
                                .as_deref(),
                        ),
                        opt_i64(
                            initiator_binding.map(|binding| {
                                i64::try_from(binding.generation).unwrap_or(i64::MAX)
                            }),
                        ),
                        opt_string(
                            provisioning
                                .map(|intent| intent.user_id.to_string())
                                .as_deref(),
                        ),
                        opt_i64(provisioning.map(|intent| {
                            i64::try_from(intent.identity_revision).unwrap_or(i64::MAX)
                        })),
                        JsValue::from_f64(f64::from(command.max_attempts)),
                        JsValue::from_f64(command.sealed_delivery.created_at.get() as f64),
                        JsValue::from_f64(command.expires_at.get() as f64),
                        JsValue::from_str(&bytes_to_hex(command.sealed_delivery.sealed_payload())),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "pending", 1)
                    .map_err(AdapterFailure::into_port)?;
                if let Some(identity) = initiator_fence.as_ref() {
                    self.push_identity_current_assertion(&mut statements, &operation_id, identity)
                        .map_err(AdapterFailure::into_port)?;
                }
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    None,
                    audit_user,
                    AuthAuditOutcome::Allow,
                    AuthAuditReason::VerificationAccepted,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let accepted_receipt = receipt.clone().with_result_code("accepted");
                self.batch_mutation(statements, &accepted_receipt)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let outcome = self
                    .verification_issue_outcome_committed(&accepted_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                telemetry.finish("accepted", 1);
                Ok(outcome)
            }
            .await;
            match outcome {
                Err(PortError::Conflict) if attempt < 2 => {}
                outcome => return outcome,
            }
        }
        Err(PortError::Conflict)
    }

    async fn materialize_verification_deliveries(
        &self,
        now: TimestampMillis,
        limit: u32,
    ) -> Result<u32, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("verification_materialize");
        if limit == 0 || limit > 1_000 {
            return Err(AdapterFailure::Invalid.into_port());
        }

        let mut expired_cleanup = Vec::with_capacity(1);
        self.push_statement(
            &mut expired_cleanup,
            CHALLENGE_EXPIRED_CLEANUP_SQL,
            &[JsValue::from_f64(now.get() as f64)],
        )
        .map_err(AdapterFailure::into_port)?;
        self.batch(expired_cleanup)
            .await
            .map_err(AdapterFailure::into_port)?;

        let mut materialized = 0_u32;
        for round in 0..DELIVERY_MATERIALIZE_CONFLICT_ATTEMPTS {
            let pending = self
                .rows::<PendingRow>(
                    PENDING_READY_SQL,
                    &[
                        JsValue::from_f64(now.get() as f64),
                        JsValue::from_f64(f64::from(limit)),
                    ],
                )
                .await
                .map_err(AdapterFailure::into_port)?;
            let mut retry_needed = false;
            for row in pending {
                let pending = row.decode().map_err(AdapterFailure::into_port)?;
                let payload_fingerprint = bytes_to_hex(&stable_hash(&[
                    b"frame/auth/materialized-delivery/v1",
                    &pending.sealed_payload,
                ]));
                let semantic = vec![
                    candidates_json(&pending.identifier_candidates)
                        .map_err(AdapterFailure::into_port)?,
                    serde_json::to_string(&DigestWire::from(&pending.secret_digest))
                        .map_err(|_| AdapterFailure::Invalid.into_port())?,
                    enum_name(pending.purpose).map_err(AdapterFailure::into_port)?,
                    enum_name(pending.channel).map_err(AdapterFailure::into_port)?,
                    pending
                        .initiator
                        .map(|binding| {
                            format!(
                                "{}:{}:{}",
                                binding.session_id, binding.user_id, binding.generation
                            )
                        })
                        .unwrap_or_default(),
                    pending
                        .provisioning
                        .map(|(user_id, revision)| format!("{user_id}:{revision}"))
                        .unwrap_or_default(),
                    pending.max_attempts.to_string(),
                    pending.expires_at.get().to_string(),
                    payload_fingerprint,
                ];
                let base_receipt = OperationReceipt::internal(
                    "verification_materialize",
                    pending.delivery_id.to_string(),
                    pending.created_at,
                    &semantic,
                );
                let operation_id = base_receipt.operation_id.clone();
                let mut statements = Vec::with_capacity(12);
                if pending.expires_at <= now {
                    self.push_statement(
                        &mut statements,
                        PENDING_DELETE_SQL,
                        &[
                            JsValue::from_str(&pending.delivery_id.to_string()),
                            JsValue::from_f64(pending.revision as f64),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_assertion(&mut statements, &operation_id, "pending", 1)
                        .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("expired_deleted");
                    match self.batch_mutation(statements, &receipt).await {
                        Ok(())
                            if self
                                .verification_materialization_committed(
                                    &receipt,
                                    pending.delivery_id,
                                    None,
                                    None,
                                )
                                .await
                                .map_err(AdapterFailure::into_port)? => {}
                        Ok(()) => return Err(AdapterFailure::Unavailable.into_port()),
                        Err(AdapterFailure::Conflict)
                            if self
                                .pending_by_id(pending.delivery_id)
                                .await
                                .map_err(AdapterFailure::into_port)?
                                .is_none() => {}
                        Err(AdapterFailure::Conflict) => retry_needed = true,
                        Err(failure) => return Err(failure.into_port()),
                    }
                    continue;
                }

                let identifier_owner = self
                    .identifier_owner(&pending.identifier_candidates)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let (challenge_user_id, suppress) = match pending.purpose {
                    VerificationPurpose::AccountLink => {
                        let initiator = pending
                            .initiator
                            .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
                        if identifier_owner.is_none()
                            && self
                                .valid_continuation(initiator, now)
                                .await
                                .map_err(AdapterFailure::into_port)?
                        {
                            self.push_continuation_assertion(
                                &mut statements,
                                &operation_id,
                                initiator,
                                now,
                            )
                            .map_err(AdapterFailure::into_port)?;
                            (Some(initiator.user_id), false)
                        } else {
                            (None, true)
                        }
                    }
                    VerificationPurpose::IdentityProvisioning => {
                        let (user_id, _) = pending
                            .provisioning
                            .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
                        if identifier_owner.is_none() {
                            (Some(user_id), false)
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
                let mut challenge = VerificationChallenge::new(NewVerificationChallenge {
                    user_id: challenge_user_id,
                    initiator: pending.initiator,
                    provisioning_revision: pending.provisioning.map(|(_, revision)| revision),
                    identifier_digest: pending.identifier_candidates.active().clone(),
                    secret_digest: pending.secret_digest.clone(),
                    purpose: pending.purpose,
                    channel: pending.channel,
                    max_attempts: pending.max_attempts,
                    created_at: pending.created_at,
                    expires_at: pending.expires_at,
                })
                .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                challenge.id = VerificationId::parse(&stable_child_uuid(
                    &operation_id,
                    "verification-challenge",
                ))
                .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                self.push_challenge_insert(&mut statements, &operation_id, &challenge)
                    .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "challenge", 1)
                    .map_err(AdapterFailure::into_port)?;
                let initiator_session = pending
                    .initiator
                    .map(|binding| binding.session_id.to_string());
                self.push_statement(
                    &mut statements,
                    DELIVERY_INSERT_SQL,
                    &[
                        JsValue::from_str(&pending.delivery_id.to_string()),
                        JsValue::from_str(&bytes_to_hex(&pending.sealed_payload)),
                        JsValue::from_f64(if suppress { 1.0 } else { 0.0 }),
                        JsValue::from_f64(pending.created_at.get() as f64),
                        JsValue::from_f64(pending.expires_at.get() as f64),
                        opt_string(initiator_session.as_deref()),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "delivery", 1)
                    .map_err(AdapterFailure::into_port)?;

                if !matches!(
                    pending.purpose,
                    VerificationPurpose::AccountLink | VerificationPurpose::IdentityProvisioning
                ) && let Some(user_id) = identifier_owner
                {
                    let active_candidates = SecretDigestCandidates::new(
                        pending.identifier_candidates.active().clone(),
                        Vec::new(),
                    )
                    .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                    if self
                        .identifier_owner(&active_candidates)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .is_none()
                    {
                        let (version, digest) =
                            digest_values(pending.identifier_candidates.active());
                        self.push_statement(
                            &mut statements,
                            IDENTIFIER_INSERT_SQL,
                            &[
                                version,
                                digest,
                                JsValue::from_str(&user_id.to_string()),
                                JsValue::from_f64(now.get() as f64),
                                JsValue::from_str(&operation_id),
                            ],
                        )
                        .map_err(AdapterFailure::into_port)?;
                        self.push_assertion(&mut statements, &operation_id, "identifier", 1)
                            .map_err(AdapterFailure::into_port)?;
                    }
                }
                self.push_statement(
                    &mut statements,
                    PENDING_DELETE_SQL,
                    &[
                        JsValue::from_str(&pending.delivery_id.to_string()),
                        JsValue::from_f64(pending.revision as f64),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "pending", 1)
                    .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let receipt = base_receipt.clone().with_result_code("materialized");
                match self.batch_mutation(statements, &receipt).await {
                    Ok(())
                        if self
                            .verification_materialization_committed(
                                &receipt,
                                pending.delivery_id,
                                Some(challenge.id),
                                Some(suppress),
                            )
                            .await
                            .map_err(AdapterFailure::into_port)? =>
                    {
                        materialized = materialized.saturating_add(1);
                    }
                    Ok(()) => return Err(AdapterFailure::Unavailable.into_port()),
                    Err(AdapterFailure::Conflict)
                        if self
                            .pending_by_id(pending.delivery_id)
                            .await
                            .map_err(AdapterFailure::into_port)?
                            .is_none() => {}
                    Err(AdapterFailure::Conflict) => retry_needed = true,
                    Err(failure) => return Err(failure.into_port()),
                }
            }
            if !retry_needed {
                telemetry.finish("materialized", materialized as usize);
                return Ok(materialized);
            }
            if round + 1 == DELIVERY_MATERIALIZE_CONFLICT_ATTEMPTS {
                return Err(AdapterFailure::Conflict.into_port());
            }
        }
        Err(AdapterFailure::Conflict.into_port())
    }

    async fn attempt_verification(
        &self,
        command: VerificationAttemptCommand,
    ) -> Result<VerificationAtomicOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("verification_attempt");
        require_action(
            &command.audit,
            if command.purpose == VerificationPurpose::AccountLink {
                AuthAuditAction::AccountLink
            } else {
                AuthAuditAction::VerificationConsume
            },
        )
        .map_err(AdapterFailure::into_port)?;
        for attempt in 0..3 {
            match self.attempt_verification_once(&command).await {
                Ok(outcome) => {
                    let (name, rows) = match &outcome {
                        VerificationAtomicOutcome::Verified { .. }
                        | VerificationAtomicOutcome::ProvisioningAuthorized(_)
                        | VerificationAtomicOutcome::Linked { .. } => ("verified", 1),
                        VerificationAtomicOutcome::Rejected(_) => ("rejected", 0),
                        VerificationAtomicOutcome::RateLimited { .. } => ("rate_limited", 0),
                    };
                    telemetry.finish(name, rows);
                    return Ok(outcome);
                }
                Err(AdapterFailure::Conflict) if attempt < 2 => {}
                Err(failure) => return Err(failure.into_port()),
            }
        }
        Err(AdapterFailure::Conflict.into_port())
    }

    async fn issue_api_key(
        &self,
        command: ApiKeyIssueCommand,
    ) -> Result<ApiKeyIssueOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("api_key_issue");
        require_action(&command.audit, AuthAuditAction::ApiKeyIssue)
            .map_err(AdapterFailure::into_port)?;
        let semantic = vec![
            serde_json::to_string(&command.principal)
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            command.grant.id().to_string(),
            command.grant.session_id().to_string(),
            command.grant.generation().to_string(),
            serde_json::to_string(&command.record)
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
        ];
        let receipt = OperationReceipt::for_audit(
            "api_key_issue",
            command.record.id.to_string(),
            &command.audit,
            &semantic,
        );
        if self
            .api_key_issue_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("issued", 1);
            return Ok(ApiKeyIssueOutcome::Issued);
        }
        let authoritative = self
            .principal(command.principal.user_id)
            .await
            .map_err(AdapterFailure::into_port)?;
        let validated = self
            .mutation_grant(&command.grant, command.audit.occurred_at)
            .await
            .map_err(AdapterFailure::into_port)?;
        let (Some(authoritative), Some(validated)) = (authoritative, validated) else {
            self.commit_audit(
                &command.audit,
                None,
                Some(command.principal.user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("forbidden", 0);
            return Ok(ApiKeyIssueOutcome::Forbidden);
        };
        if authoritative.snapshot != command.principal
            || validated.principal.snapshot != authoritative.snapshot
            || command.record.owner_id != authoritative.snapshot.user_id
        {
            self.commit_audit(
                &command.audit,
                Some(SessionAuditContext::from(&validated.session.record)),
                Some(command.principal.user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("forbidden", 0);
            return Ok(ApiKeyIssueOutcome::Forbidden);
        }
        let context = Some(SessionAuditContext::from(&validated.session.record));
        let authority = Self::tenant_authority(&authoritative, command.record.tenant_id);
        let Some(authority) = authority else {
            self.commit_audit(
                &command.audit,
                context,
                Some(authoritative.snapshot.user_id),
                AuthAuditOutcome::Deny,
                AuthAuditReason::InsufficientRole,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("forbidden", 0);
            return Ok(ApiKeyIssueOutcome::Forbidden);
        };
        let operation_id = receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(9);
        self.push_mutation_grant_consume(
            &mut statements,
            &operation_id,
            &command.grant,
            command.audit.occurred_at,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_tenant_authority_assertion(
            &mut statements,
            &operation_id,
            &authoritative,
            authority,
        )
        .map_err(AdapterFailure::into_port)?;
        if command.record.scopes.is_empty()
            || command.record.scopes.len() > 32
            || command
                .record
                .scopes
                .iter()
                .enumerate()
                .any(|(index, scope)| {
                    command.record.scopes[..index]
                        .iter()
                        .any(|other| other == scope)
                })
            || command
                .record
                .expires_at
                .is_some_and(|expires| expires <= command.record.created_at)
        {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let (key_version, key_digest) = digest_values(&command.record.key_digest);
        let scopes_json = serde_json::to_string(&command.record.scopes)
            .map_err(|_| AdapterFailure::Invalid.into_port())?;
        self.push_statement(
            &mut statements,
            API_KEY_INSERT_SQL,
            &[
                JsValue::from_str(&command.record.id.to_string()),
                JsValue::from_str(&command.record.owner_id.to_string()),
                JsValue::from_str(&command.record.tenant_id.to_string()),
                key_version,
                key_digest,
                JsValue::from_str(&scopes_json),
                JsValue::from_f64(command.record.created_at.get() as f64),
                opt_i64(command.record.expires_at.map(TimestampMillis::get)),
                JsValue::from_str(&operation_id),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "api_key", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            context,
            Some(authoritative.snapshot.user_id),
            AuthAuditOutcome::Allow,
            AuthAuditReason::Issued,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {}
            Err(AdapterFailure::Conflict) => {
                self.commit_audit(
                    &command.audit,
                    context,
                    Some(command.principal.user_id),
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InsufficientRole,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("forbidden", 0);
                return Ok(ApiKeyIssueOutcome::Forbidden);
            }
            Err(failure) => return Err(failure.into_port()),
        }
        if !self
            .api_key_issue_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            return Err(AdapterFailure::Unavailable.into_port());
        }
        telemetry.finish("issued", 1);
        Ok(ApiKeyIssueOutcome::Issued)
    }

    async fn authenticate_api_key(
        &self,
        command: ApiKeyAuthenticationCommand,
    ) -> Result<ApiKeyAuthenticationOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("api_key_authenticate");
        require_action(&command.audit, AuthAuditAction::ApiKeyAuthenticate)
            .map_err(AdapterFailure::into_port)?;
        for attempt in 0..3 {
            let outcome: Result<ApiKeyAuthenticationOutcome, PortError> = async {
                let policy = command.rate_policy;
                let semantic = vec![
                    candidates_json(&command.key_digests).map_err(AdapterFailure::into_port)?,
                    command.tenant_id.to_string(),
                    enum_name(command.required_scope).map_err(AdapterFailure::into_port)?,
                    candidates_json(&command.abuse.identifier)
                        .map_err(AdapterFailure::into_port)?,
                    candidates_json(&command.abuse.source).map_err(AdapterFailure::into_port)?,
                    candidates_json(&command.abuse.device).map_err(AdapterFailure::into_port)?,
                    format!(
                        "{}:{}:{}|{}:{}:{}|{}:{}:{}|{}:{}:{}",
                        policy.identifier.max_attempts(),
                        policy.identifier.window().get(),
                        policy.identifier.block_for().get(),
                        policy.source.max_attempts(),
                        policy.source.window().get(),
                        policy.source.block_for().get(),
                        policy.device.max_attempts(),
                        policy.device.window().get(),
                        policy.device.block_for().get(),
                        policy.global.max_attempts(),
                        policy.global.window().get(),
                        policy.global.block_for().get(),
                    ),
                ];
                let base_receipt = OperationReceipt::for_audit(
                    "api_key_authenticate",
                    command
                        .key_digests
                        .active()
                        .digest
                        .expose_for_verification(),
                    &command.audit,
                    &semantic,
                );
                if let Some(outcome) = self
                    .api_key_authentication_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    telemetry.finish(
                        "replayed",
                        usize::from(matches!(
                            outcome,
                            ApiKeyAuthenticationOutcome::Authenticated(_)
                        )),
                    );
                    return Ok(outcome);
                }
                let operation_id = base_receipt.operation_id.clone();
                let mut statements = Vec::with_capacity(22);
                if let Some(retry_at) = self
                    .append_rate_limit_plan(
                        &mut statements,
                        &operation_id,
                        AuthAbuseAction::ApiKeyAuthenticate,
                        &command.abuse,
                        command.rate_policy,
                        command.audit.occurred_at,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    self.push_audit(
                        &mut statements,
                        &operation_id,
                        &command.audit,
                        None,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at);
                    let outcome = self
                        .commit_api_key_authentication(statements, &receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?;
                    telemetry.finish("rate_limited", 0);
                    return Ok(outcome);
                }
                let key = self
                    .api_key_by_candidates(&command.key_digests)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let Some((mut key, mut revision, _candidate_order)) = key else {
                    let json =
                        candidates_json(&command.key_digests).map_err(AdapterFailure::into_port)?;
                    self.push_statement(
                        &mut statements,
                        API_KEYS_ABSENT_ASSERT_SQL,
                        &[
                            JsValue::from_str(&json),
                            JsValue::from_str(&format!("{operation_id}:api_key_absent")),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_audit(
                        &mut statements,
                        &operation_id,
                        &command.audit,
                        None,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    let outcome = self
                        .commit_api_key_authentication(statements, &receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                };
                if key.tenant_id != command.tenant_id
                    || !key.allows(command.required_scope, command.audit.occurred_at)
                {
                    self.push_api_key_assertion(&mut statements, &operation_id, &key, revision)
                        .map_err(AdapterFailure::into_port)?;
                    self.push_audit(
                        &mut statements,
                        &operation_id,
                        &command.audit,
                        None,
                        Some(key.owner_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    let outcome = self
                        .commit_api_key_authentication(statements, &receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                let principal = self
                    .principal(key.owner_id)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let membership = principal
                    .as_ref()
                    .and_then(|principal| Self::tenant_membership(principal, key.tenant_id));
                let (Some(principal), Some(membership)) = (principal, membership) else {
                    self.push_api_key_assertion(&mut statements, &operation_id, &key, revision)
                        .map_err(AdapterFailure::into_port)?;
                    self.push_audit(
                        &mut statements,
                        &operation_id,
                        &command.audit,
                        None,
                        Some(key.owner_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_cleanup(&mut statements, &operation_id)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    let outcome = self
                        .commit_api_key_authentication(statements, &receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                };
                let migrates = key.key_digest != *command.key_digests.active();
                if migrates {
                    let previous_version = key.key_digest.key_version.get();
                    let (next_version, next_digest) = digest_values(command.key_digests.active());
                    self.push_statement(
                        &mut statements,
                        API_KEY_DIGEST_MIGRATE_SQL,
                        &[
                            JsValue::from_str(&key.id.to_string()),
                            JsValue::from_f64(revision as f64),
                            JsValue::from_f64(f64::from(previous_version)),
                            next_version,
                            next_digest,
                            JsValue::from_str(&operation_id),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_assertion(&mut statements, &operation_id, "api_key", 1)
                        .map_err(AdapterFailure::into_port)?;
                    key.key_digest = command.key_digests.active().clone();
                    revision = revision.saturating_add(1);
                } else {
                    self.push_api_key_assertion(&mut statements, &operation_id, &key, revision)
                        .map_err(AdapterFailure::into_port)?;
                }
                self.push_tenant_authority_assertion(
                    &mut statements,
                    &operation_id,
                    &principal,
                    membership,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    None,
                    Some(key.owner_id),
                    AuthAuditOutcome::Allow,
                    if migrates {
                        AuthAuditReason::KeyVersionMigrated
                    } else {
                        AuthAuditReason::Authenticated
                    },
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let receipt = base_receipt.clone().with_result_code(if migrates {
                    "key_migrated"
                } else {
                    "authenticated"
                });
                let outcome = self
                    .commit_api_key_authentication(statements, &receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let _final_revision = revision;
                telemetry.finish(
                    if migrates {
                        "migrated"
                    } else {
                        "authenticated"
                    },
                    1,
                );
                let _ = principal;
                Ok(outcome)
            }
            .await;
            match outcome {
                Err(PortError::Conflict) if attempt < 2 => {}
                outcome => return outcome,
            }
        }
        Err(PortError::Conflict)
    }

    async fn revoke_api_key(&self, command: ApiKeyRevokeCommand) -> Result<bool, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("api_key_revoke");
        require_action(&command.audit, AuthAuditAction::ApiKeyRevoke)
            .map_err(AdapterFailure::into_port)?;
        let semantic = vec![
            command.grant.id().to_string(),
            command.grant.session_id().to_string(),
            command.grant.generation().to_string(),
            command.key_id.to_string(),
        ];
        let receipt = OperationReceipt::for_audit(
            "api_key_revoke",
            command.key_id.to_string(),
            &command.audit,
            &semantic,
        );
        if self
            .api_key_revoke_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("revoked", 1);
            return Ok(true);
        }
        let validated = self
            .mutation_grant(&command.grant, command.audit.occurred_at)
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some(validated) = validated else {
            self.commit_audit(
                &command.audit,
                None,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(false);
        };
        let context = Some(SessionAuditContext::from(&validated.session.record));
        let key = self
            .api_key_by_id(command.key_id)
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some((mut key, revision)) = key else {
            self.commit_audit(
                &command.audit,
                context,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InvalidCredential,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("denied", 0);
            return Ok(false);
        };
        let authority = Self::tenant_authority(&validated.principal, key.tenant_id);
        let Some(authority) = authority else {
            self.commit_audit(
                &command.audit,
                context,
                None,
                AuthAuditOutcome::Deny,
                AuthAuditReason::InsufficientRole,
            )
            .await
            .map_err(AdapterFailure::into_port)?;
            telemetry.finish("forbidden", 0);
            return Ok(false);
        };
        let operation_id = receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(9);
        self.push_mutation_grant_consume(
            &mut statements,
            &operation_id,
            &command.grant,
            command.audit.occurred_at,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_tenant_authority_assertion(
            &mut statements,
            &operation_id,
            &validated.principal,
            authority,
        )
        .map_err(AdapterFailure::into_port)?;
        if key.revoked_at.is_none() {
            self.push_statement(
                &mut statements,
                API_KEY_REVOKE_SQL,
                &[
                    JsValue::from_str(&key.id.to_string()),
                    JsValue::from_f64(revision as f64),
                    JsValue::from_f64(command.audit.occurred_at.get() as f64),
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(&mut statements, &operation_id, "api_key", 1)
                .map_err(AdapterFailure::into_port)?;
            key.revoked_at = Some(command.audit.occurred_at);
        } else {
            self.push_api_key_assertion(&mut statements, &operation_id, &key, revision)
                .map_err(AdapterFailure::into_port)?;
        }
        self.push_audit(
            &mut statements,
            &operation_id,
            &command.audit,
            context,
            None,
            AuthAuditOutcome::Allow,
            AuthAuditReason::Revoked,
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {}
            Err(AdapterFailure::Conflict) => {
                self.commit_audit(
                    &command.audit,
                    context,
                    None,
                    AuthAuditOutcome::Deny,
                    AuthAuditReason::InsufficientRole,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                telemetry.finish("forbidden", 0);
                return Ok(false);
            }
            Err(failure) => return Err(failure.into_port()),
        }
        if !self
            .api_key_revoke_committed(&receipt, &command)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            return Err(AdapterFailure::Unavailable.into_port());
        }
        telemetry.finish("revoked", 1);
        Ok(true)
    }

    async fn begin_oauth(
        &self,
        command: OAuthBeginCommand,
    ) -> Result<OAuthBeginOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("oauth_begin");
        require_action(&command.audit, AuthAuditAction::OAuthBegin)
            .map_err(AdapterFailure::into_port)?;
        let flow = &command.flow;
        let semantic = vec![
            enum_name(flow.provider).map_err(AdapterFailure::into_port)?,
            enum_name(flow.purpose).map_err(AdapterFailure::into_port)?,
            flow.initiator
                .map(|binding| {
                    format!(
                        "{}:{}:{}",
                        binding.session_id, binding.user_id, binding.generation
                    )
                })
                .unwrap_or_default(),
            command
                .initiator
                .as_ref()
                .map(|grant| grant.id().to_string())
                .unwrap_or_default(),
            serde_json::to_string(&DigestWire::from(&flow.state_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            serde_json::to_string(&DigestWire::from(&flow.pkce_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            serde_json::to_string(&DigestWire::from(&flow.redirect_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            serde_json::to_string(&DigestWire::from(&flow.audience_digest))
                .map_err(|_| AdapterFailure::Invalid.into_port())?,
            flow.created_at.get().to_string(),
            flow.expires_at.get().to_string(),
            candidates_json(&command.abuse.identifier).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.abuse.source).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.abuse.device).map_err(AdapterFailure::into_port)?,
            rate_policy_fingerprint(command.rate_policy),
        ];
        let base_receipt = OperationReceipt::for_audit(
            "oauth_begin",
            flow.id.to_string(),
            &command.audit,
            &semantic,
        );
        let state_candidates = SecretDigestCandidates::new(flow.state_digest.clone(), Vec::new())
            .map_err(|_| AdapterFailure::Invalid.into_port())?;
        for attempt in 0..3 {
            let outcome: Result<OAuthBeginOutcome, PortError> = async {
                if let Some(outcome) = self
                    .oauth_begin_outcome_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    telemetry.finish(
                        "replayed",
                        usize::from(outcome == OAuthBeginOutcome::Started),
                    );
                    return Ok(outcome);
                }
                let operation_id = base_receipt.operation_id.clone();
                let mut statements = Vec::with_capacity(24);
                self.push_statement(
                    &mut statements,
                    OAUTH_EXPIRED_FLOWS_DELETE_SQL,
                    &[JsValue::from_f64(command.audit.occurred_at.get() as f64)],
                )
                .map_err(AdapterFailure::into_port)?;
                if let Some(retry_at) = self
                    .oauth_pending_capacity_retry(command.audit.occurred_at)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    self.push_statement(
                        &mut statements,
                        OAUTH_FLOW_CAPACITY_ASSERT_SQL,
                        &[
                            JsValue::from_f64(command.audit.occurred_at.get() as f64),
                            JsValue::from_f64(retry_at.get() as f64),
                            JsValue::from_str(&format!("{operation_id}:oauth_flow_capacity")),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at);
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_begin_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rate_limited", 0);
                    return Ok(outcome);
                }
                if let Some(retry_at) = self
                    .append_rate_limit_plan(
                        &mut statements,
                        &operation_id,
                        AuthAbuseAction::OAuthBegin,
                        &command.abuse,
                        command.rate_policy,
                        command.audit.occurred_at,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    let receipt = base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at);
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_begin_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rate_limited", 0);
                    return Ok(outcome);
                }
                let validated = match command.initiator.as_ref() {
                    Some(grant) => self
                        .mutation_grant(grant, command.audit.occurred_at)
                        .await
                        .map_err(AdapterFailure::into_port)?,
                    None => None,
                };
                if let Some(grant) = command.initiator.as_ref()
                    && validated.is_none()
                {
                    self.push_oauth_mutation_grant_invalid_assertion(
                        &mut statements,
                        &operation_id,
                        grant,
                        command.audit.occurred_at,
                    )
                    .map_err(AdapterFailure::into_port)?;
                }
                let validated_binding =
                    validated.as_ref().map(|value| SessionContinuationBinding {
                        session_id: value.session.record.id,
                        user_id: value.session.record.user_id,
                        generation: value.session.record.generation,
                    });
                let initiator_shape_matches = match (&command.initiator, validated_binding) {
                    (None, None) => true,
                    (Some(_), Some(binding)) => flow.initiator == Some(binding),
                    _ => false,
                };
                let purpose_matches = match flow.purpose {
                    OAuthFlowPurpose::SignIn => validated_binding.is_none(),
                    OAuthFlowPurpose::AccountLink => validated_binding.is_some(),
                };
                let valid_shape = initiator_shape_matches
                    && purpose_matches
                    && flow.created_at == command.audit.occurred_at
                    && flow.expires_at > command.audit.occurred_at
                    && flow.consumed_at.is_none()
                    && !flow.revoked;
                if let (Some(grant), Some(_)) = (command.initiator.as_ref(), validated.as_ref()) {
                    self.push_mutation_grant_consume(
                        &mut statements,
                        &operation_id,
                        grant,
                        command.audit.occurred_at,
                    )
                    .map_err(AdapterFailure::into_port)?;
                }
                if !valid_shape {
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        validated_binding.map(|binding| binding.user_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_begin_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                let state_collision = self
                    .oauth_flow_by_state(&state_candidates)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .is_some_and(|existing| existing.expires_at > command.audit.occurred_at);
                let id_collision = self
                    .oauth_flow_id_exists(flow.id, command.audit.occurred_at)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                if state_collision || id_collision {
                    self.push_statement(
                        &mut statements,
                        OAUTH_FLOW_COLLISION_ASSERT_SQL,
                        &[
                            JsValue::from_str(&flow.id.to_string()),
                            JsValue::from_str(
                                &candidates_json(&state_candidates)
                                    .map_err(AdapterFailure::into_port)?,
                            ),
                            JsValue::from_f64(command.audit.occurred_at.get() as f64),
                            JsValue::from_str(&format!("{operation_id}:oauth_flow_collision")),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        validated_binding.map(|binding| binding.user_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_begin_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                self.push_statement(
                    &mut statements,
                    OAUTH_FLOW_BEGIN_ABSENT_ASSERT_SQL,
                    &[
                        JsValue::from_str(&flow.id.to_string()),
                        JsValue::from_str(
                            &candidates_json(&state_candidates)
                                .map_err(AdapterFailure::into_port)?,
                        ),
                        JsValue::from_str(&format!("{operation_id}:oauth_flow_absent")),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                let (state_version, state_digest) = digest_values(&flow.state_digest);
                let (pkce_version, pkce_digest) = digest_values(&flow.pkce_digest);
                let (redirect_version, redirect_digest) = digest_values(&flow.redirect_digest);
                let (audience_version, audience_digest) = digest_values(&flow.audience_digest);
                self.push_statement(
                    &mut statements,
                    OAUTH_FLOW_INSERT_SQL,
                    &[
                        JsValue::from_str(&flow.id.to_string()),
                        JsValue::from_str(
                            &enum_name(flow.provider).map_err(AdapterFailure::into_port)?,
                        ),
                        JsValue::from_str(
                            &enum_name(flow.purpose).map_err(AdapterFailure::into_port)?,
                        ),
                        opt_string(
                            validated_binding
                                .map(|binding| binding.session_id.to_string())
                                .as_deref(),
                        ),
                        opt_string(
                            validated_binding
                                .map(|binding| binding.user_id.to_string())
                                .as_deref(),
                        ),
                        opt_i64(
                            validated_binding.map(|binding| {
                                i64::try_from(binding.generation).unwrap_or(i64::MAX)
                            }),
                        ),
                        state_version,
                        state_digest,
                        pkce_version,
                        pkce_digest,
                        redirect_version,
                        redirect_digest,
                        audience_version,
                        audience_digest,
                        JsValue::from_f64(flow.created_at.get() as f64),
                        JsValue::from_f64(flow.expires_at.get() as f64),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "oauth_flow", 1)
                    .map_err(AdapterFailure::into_port)?;
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    None,
                    validated_binding.map(|binding| binding.user_id),
                    AuthAuditOutcome::Allow,
                    AuthAuditReason::Issued,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let receipt = base_receipt.clone().with_result_code("started");
                self.oauth_batch_mutation(statements, &receipt)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let outcome = self
                    .oauth_begin_outcome_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                telemetry.finish("started", 1);
                Ok(outcome)
            }
            .await;
            match outcome {
                Err(PortError::Conflict) if attempt < 2 => {}
                outcome => return outcome,
            }
        }
        Err(PortError::Conflict)
    }

    async fn preflight_oauth_exchange(
        &self,
        command: OAuthPreflightCommand,
    ) -> Result<OAuthPreflightOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("oauth_preflight");
        require_action(&command.audit, AuthAuditAction::OAuthExchangePreflight)
            .map_err(AdapterFailure::into_port)?;
        let semantic = vec![
            enum_name(command.provider).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.state_digests).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.pkce_digests).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.redirect_digests).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.audience_digests).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.abuse.identifier).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.abuse.source).map_err(AdapterFailure::into_port)?,
            candidates_json(&command.abuse.device).map_err(AdapterFailure::into_port)?,
            rate_policy_fingerprint(command.rate_policy),
        ];
        let base_receipt = OperationReceipt::for_audit(
            "oauth_preflight",
            enum_name(command.provider).map_err(AdapterFailure::into_port)?,
            &command.audit,
            &semantic,
        );
        for attempt in 0..3 {
            let outcome: Result<OAuthPreflightOutcome, PortError> = async {
                if let Some(outcome) = self
                    .oauth_preflight_outcome_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    telemetry.finish(
                        "replayed",
                        usize::from(matches!(outcome, OAuthPreflightOutcome::Ready(_))),
                    );
                    return Ok(outcome);
                }
                let operation_id = base_receipt.operation_id.clone();
                let mut statements = Vec::with_capacity(24);
                if let Some(retry_at) = self
                    .append_rate_limit_plan(
                        &mut statements,
                        &operation_id,
                        AuthAbuseAction::OAuthExchange,
                        &command.abuse,
                        command.rate_policy,
                        command.audit.occurred_at,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    let receipt = base_receipt
                        .clone()
                        .with_result_code("rate_limited")
                        .with_result_timestamp(retry_at);
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::RateLimited,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_preflight_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rate_limited", 0);
                    return Ok(outcome);
                }
                let Some(flow) = self
                    .oauth_flow_by_state(&command.state_digests)
                    .await
                    .map_err(AdapterFailure::into_port)?
                else {
                    self.push_statement(
                        &mut statements,
                        OAUTH_FLOW_ABSENT_ASSERT_SQL,
                        &[
                            JsValue::from_str(
                                &candidates_json(&command.state_digests)
                                    .map_err(AdapterFailure::into_port)?,
                            ),
                            JsValue::from_str(&format!("{operation_id}:oauth_flow_absent")),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_preflight_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                };
                if let Some(binding) = flow.initiator
                    && !self
                        .valid_continuation(binding, command.audit.occurred_at)
                        .await
                        .map_err(AdapterFailure::into_port)?
                {
                    self.push_oauth_continuation_invalid_assertion(
                        &mut statements,
                        &operation_id,
                        binding,
                        command.audit.occurred_at,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_statement(
                        &mut statements,
                        OAUTH_FLOW_DELETE_SQL,
                        &[
                            JsValue::from_str(&flow.id.to_string()),
                            JsValue::from_f64(flow.revision as f64),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    self.push_assertion(&mut statements, &operation_id, "oauth_flow", 1)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        Some(binding.user_id),
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_preflight_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                let purpose_matches = match flow.purpose {
                    OAuthFlowPurpose::SignIn => flow.initiator.is_none(),
                    OAuthFlowPurpose::AccountLink => flow.initiator.is_some(),
                };
                let invalid = !purpose_matches
                    || flow.provider != command.provider
                    || flow.revoked
                    || !digest_candidates_contain(&command.state_digests, &flow.state_digest)
                    || !digest_candidates_contain(&command.pkce_digests, &flow.pkce_digest)
                    || !digest_candidates_contain(&command.redirect_digests, &flow.redirect_digest)
                    || !digest_candidates_contain(&command.audience_digests, &flow.audience_digest);
                let rejection = if invalid {
                    Some(("rejected_invalid", AuthAuditReason::InvalidCredential))
                } else if flow.consumed_at.is_some() {
                    Some(("rejected_replay", AuthAuditReason::ReplayDetected))
                } else if command.audit.occurred_at < flow.created_at
                    || command.audit.occurred_at >= flow.expires_at
                {
                    Some(("rejected_expired", AuthAuditReason::Expired))
                } else {
                    None
                };
                if let Some((result_code, reason)) = rejection {
                    self.push_oauth_flow_current_assertion(&mut statements, &operation_id, &flow)
                        .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code(result_code);
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        flow.initiator.map(|binding| binding.user_id),
                        AuthAuditOutcome::Deny,
                        reason,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_preflight_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                self.push_statement(
                    &mut statements,
                    OAUTH_FLOW_CONSUME_SQL,
                    &[
                        JsValue::from_str(&flow.id.to_string()),
                        JsValue::from_f64(flow.revision as f64),
                        JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "oauth_flow", 1)
                    .map_err(AdapterFailure::into_port)?;
                if let Some(binding) = flow.initiator {
                    self.push_continuation_assertion(
                        &mut statements,
                        &operation_id,
                        binding,
                        command.audit.occurred_at,
                    )
                    .map_err(AdapterFailure::into_port)?;
                }
                let reservation_id = OAuthExchangeReservationId::parse(&stable_child_uuid(
                    &operation_id,
                    "oauth-reservation",
                ))
                .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                let initiator_session_id =
                    flow.initiator.map(|binding| binding.session_id.to_string());
                let initiator_user_id = flow.initiator.map(|binding| binding.user_id.to_string());
                self.push_statement(
                    &mut statements,
                    OAUTH_RESERVATION_INSERT_SQL,
                    &[
                        JsValue::from_str(&reservation_id.to_string()),
                        JsValue::from_str(&flow.id.to_string()),
                        JsValue::from_str(
                            &enum_name(flow.provider).map_err(AdapterFailure::into_port)?,
                        ),
                        opt_string(initiator_session_id.as_deref()),
                        opt_string(initiator_user_id.as_deref()),
                        opt_i64(
                            flow.initiator.map(|binding| {
                                i64::try_from(binding.generation).unwrap_or(i64::MAX)
                            }),
                        ),
                        JsValue::from_f64(flow.expires_at.get() as f64),
                        JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "oauth_reservation", 1)
                    .map_err(AdapterFailure::into_port)?;
                self.push_audit(
                    &mut statements,
                    &operation_id,
                    &command.audit,
                    None,
                    flow.initiator.map(|binding| binding.user_id),
                    AuthAuditOutcome::Allow,
                    AuthAuditReason::Issued,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_cleanup(&mut statements, &operation_id)
                    .map_err(AdapterFailure::into_port)?;
                let receipt = base_receipt.clone().with_result_code("ready");
                self.oauth_batch_mutation(statements, &receipt)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let outcome = self
                    .oauth_preflight_outcome_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                telemetry.finish("ready", 1);
                Ok(outcome)
            }
            .await;
            match outcome {
                Err(PortError::Conflict) if attempt < 2 => {}
                outcome => return outcome,
            }
        }
        Err(PortError::Conflict)
    }

    async fn finalize_oauth_exchange(
        &self,
        command: OAuthFinalizeCommand,
    ) -> Result<OAuthExchangeOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("oauth_finalize");
        require_action(&command.audit, AuthAuditAction::OAuthExchange)
            .map_err(AdapterFailure::into_port)?;
        let reservation = &command.reservation;
        let initiator = reservation.initiator();
        let mut semantic = vec![
            reservation.id().to_string(),
            reservation.flow_id().to_string(),
            enum_name(reservation.provider()).map_err(AdapterFailure::into_port)?,
            initiator
                .map(|binding| {
                    format!(
                        "{}:{}:{}",
                        binding.session_id, binding.user_id, binding.generation
                    )
                })
                .unwrap_or_default(),
            reservation.expires_at().get().to_string(),
        ];
        match &command.provider_result {
            OAuthProviderResult::Verified(assertion) => {
                semantic.push("verified".into());
                semantic.push(enum_name(assertion.provider).map_err(AdapterFailure::into_port)?);
                semantic.push(
                    candidates_json(&assertion.subject_digests)
                        .map_err(AdapterFailure::into_port)?,
                );
                semantic.push(
                    assertion
                        .verified_identifier_digests
                        .as_ref()
                        .map(candidates_json)
                        .transpose()
                        .map_err(AdapterFailure::into_port)?
                        .unwrap_or_default(),
                );
            }
            OAuthProviderResult::Rejected => semantic.push("rejected".into()),
            OAuthProviderResult::AdapterFailure => semantic.push("adapter_failure".into()),
        }
        let base_receipt = OperationReceipt::for_audit(
            "oauth_finalize",
            reservation.id().to_string(),
            &command.audit,
            &semantic,
        );
        for attempt in 0..3 {
            let outcome: Result<OAuthExchangeOutcome, PortError> = async {
                if let Some(outcome) = self
                    .oauth_finalize_outcome_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    telemetry.finish(
                        "replayed",
                        usize::from(matches!(
                            outcome,
                            OAuthExchangeOutcome::Verified { .. }
                                | OAuthExchangeOutcome::Linked { .. }
                        )),
                    );
                    return Ok(outcome);
                }
                let operation_id = base_receipt.operation_id.clone();
                let mut statements = Vec::with_capacity(24);
                let Some(stored) = self
                    .oauth_reservation(reservation.id())
                    .await
                    .map_err(AdapterFailure::into_port)?
                else {
                    self.push_statement(
                        &mut statements,
                        OAUTH_RESERVATION_ABSENT_ASSERT_SQL,
                        &[
                            JsValue::from_str(&reservation.id().to_string()),
                            JsValue::from_str(&format!("{operation_id}:oauth_reservation_absent")),
                        ],
                    )
                    .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("rejected_replay");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::ReplayDetected,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_finalize_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                };
                let stored_user = stored.initiator.map(|binding| binding.user_id);
                let rejection = if stored.consumed_at.is_some() {
                    Some(("rejected_replay", AuthAuditReason::ReplayDetected))
                } else if stored.flow_id != reservation.flow_id()
                    || stored.provider != reservation.provider()
                    || stored.initiator != reservation.initiator()
                    || stored.expires_at != reservation.expires_at()
                    || command.audit.occurred_at < stored.created_at
                {
                    Some(("rejected_invalid", AuthAuditReason::InvalidCredential))
                } else if command.audit.occurred_at >= stored.expires_at {
                    Some(("rejected_expired", AuthAuditReason::Expired))
                } else {
                    None
                };
                if let Some((result_code, reason)) = rejection {
                    self.push_oauth_reservation_current_assertion(
                        &mut statements,
                        &operation_id,
                        &stored,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code(result_code);
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        stored_user,
                        AuthAuditOutcome::Deny,
                        reason,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_finalize_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                self.push_statement(
                    &mut statements,
                    OAUTH_RESERVATION_CONSUME_SQL,
                    &[
                        JsValue::from_str(&stored.id.to_string()),
                        JsValue::from_f64(stored.revision as f64),
                        JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "oauth_reservation", 1)
                    .map_err(AdapterFailure::into_port)?;
                if let Some(binding) = stored.initiator {
                    if self
                        .valid_continuation(binding, command.audit.occurred_at)
                        .await
                        .map_err(AdapterFailure::into_port)?
                    {
                        self.push_continuation_assertion(
                            &mut statements,
                            &operation_id,
                            binding,
                            command.audit.occurred_at,
                        )
                        .map_err(AdapterFailure::into_port)?;
                    } else {
                        self.push_oauth_continuation_invalid_assertion(
                            &mut statements,
                            &operation_id,
                            binding,
                            command.audit.occurred_at,
                        )
                        .map_err(AdapterFailure::into_port)?;
                        let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                        self.commit_oauth_decision(
                            statements,
                            &receipt,
                            &command.audit,
                            Some(binding.user_id),
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        )
                        .await
                        .map_err(AdapterFailure::into_port)?;
                        let outcome = self
                            .oauth_finalize_outcome_committed(&base_receipt, &command)
                            .await
                            .map_err(AdapterFailure::into_port)?
                            .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                        telemetry.finish("rejected", 0);
                        return Ok(outcome);
                    }
                }
                let assertion = match &command.provider_result {
                    OAuthProviderResult::Verified(assertion)
                        if assertion.provider == stored.provider =>
                    {
                        assertion
                    }
                    OAuthProviderResult::Verified(_) | OAuthProviderResult::Rejected => {
                        let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                        self.commit_oauth_decision(
                            statements,
                            &receipt,
                            &command.audit,
                            stored_user,
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        )
                        .await
                        .map_err(AdapterFailure::into_port)?;
                        let outcome = self
                            .oauth_finalize_outcome_committed(&base_receipt, &command)
                            .await
                            .map_err(AdapterFailure::into_port)?
                            .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                        telemetry.finish("rejected", 0);
                        return Ok(outcome);
                    }
                    OAuthProviderResult::AdapterFailure => {
                        let receipt = base_receipt.clone().with_result_code("adapter_failure");
                        self.commit_oauth_decision(
                            statements,
                            &receipt,
                            &command.audit,
                            stored_user,
                            AuthAuditOutcome::Error,
                            AuthAuditReason::AdapterFailure,
                        )
                        .await
                        .map_err(AdapterFailure::into_port)?;
                        let outcome = self
                            .oauth_finalize_outcome_committed(&base_receipt, &command)
                            .await
                            .map_err(AdapterFailure::into_port)?
                            .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                        telemetry.finish("adapter_failure", 0);
                        return Ok(outcome);
                    }
                };
                let subject = self
                    .oauth_external_account(assertion.provider, &assertion.subject_digests)
                    .await
                    .map_err(AdapterFailure::into_port)?;
                let subject_user = subject.as_ref().map(|account| account.user_id);
                let identifier_user = match &assertion.verified_identifier_digests {
                    Some(candidates) => self
                        .identifier_owner(candidates)
                        .await
                        .map_err(AdapterFailure::into_port)?,
                    None => None,
                };
                if let Some(binding) = stored.initiator {
                    let ownership_conflict = subject_user
                        .is_some_and(|user_id| user_id != binding.user_id)
                        || identifier_user.is_some_and(|user_id| user_id != binding.user_id);
                    if ownership_conflict {
                        self.push_oauth_external_account_snapshot_assertion(
                            &mut statements,
                            &operation_id,
                            assertion.provider,
                            &assertion.subject_digests,
                            subject_user,
                        )
                        .map_err(AdapterFailure::into_port)?;
                        if let Some(candidates) = &assertion.verified_identifier_digests {
                            self.push_oauth_identifier_snapshot_assertion(
                                &mut statements,
                                &operation_id,
                                candidates,
                                identifier_user,
                            )
                            .map_err(AdapterFailure::into_port)?;
                        }
                        let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                        self.commit_oauth_decision(
                            statements,
                            &receipt,
                            &command.audit,
                            Some(binding.user_id),
                            AuthAuditOutcome::Deny,
                            AuthAuditReason::InvalidCredential,
                        )
                        .await
                        .map_err(AdapterFailure::into_port)?;
                        let outcome = self
                            .oauth_finalize_outcome_committed(&base_receipt, &command)
                            .await
                            .map_err(AdapterFailure::into_port)?
                            .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                        telemetry.finish("rejected", 0);
                        return Ok(outcome);
                    }
                    self.push_oauth_external_account_authority_assertion(
                        &mut statements,
                        &operation_id,
                        assertion.provider,
                        &assertion.subject_digests,
                        binding.user_id,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    if let Some(candidates) = &assertion.verified_identifier_digests {
                        self.push_oauth_identifier_authority_assertion(
                            &mut statements,
                            &operation_id,
                            candidates,
                            binding.user_id,
                            identifier_user.is_none(),
                        )
                        .map_err(AdapterFailure::into_port)?;
                    }
                    self.push_oauth_external_account_write(
                        &mut statements,
                        &operation_id,
                        assertion.provider,
                        &assertion.subject_digests,
                        binding.user_id,
                        command.audit.occurred_at,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    let receipt = base_receipt.clone().with_result_code("linked");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        Some(binding.user_id),
                        AuthAuditOutcome::Allow,
                        AuthAuditReason::Linked,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_finalize_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("linked", 1);
                    return Ok(outcome);
                }
                let ownership_conflict = matches!(
                    (subject_user, identifier_user),
                    (Some(subject_user), Some(identifier_user)) if subject_user != identifier_user
                );
                let user_id = subject_user.or(identifier_user);
                if ownership_conflict || user_id.is_none() {
                    self.push_oauth_external_account_snapshot_assertion(
                        &mut statements,
                        &operation_id,
                        assertion.provider,
                        &assertion.subject_digests,
                        subject_user,
                    )
                    .map_err(AdapterFailure::into_port)?;
                    if let Some(candidates) = &assertion.verified_identifier_digests {
                        self.push_oauth_identifier_snapshot_assertion(
                            &mut statements,
                            &operation_id,
                            candidates,
                            identifier_user,
                        )
                        .map_err(AdapterFailure::into_port)?;
                    }
                    let receipt = base_receipt.clone().with_result_code("rejected_invalid");
                    self.commit_oauth_decision(
                        statements,
                        &receipt,
                        &command.audit,
                        None,
                        AuthAuditOutcome::Deny,
                        AuthAuditReason::InvalidCredential,
                    )
                    .await
                    .map_err(AdapterFailure::into_port)?;
                    let outcome = self
                        .oauth_finalize_outcome_committed(&base_receipt, &command)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("rejected", 0);
                    return Ok(outcome);
                }
                let user_id = user_id.ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
                let principal = self
                    .principal(user_id)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
                self.push_oauth_external_account_authority_assertion(
                    &mut statements,
                    &operation_id,
                    assertion.provider,
                    &assertion.subject_digests,
                    user_id,
                )
                .map_err(AdapterFailure::into_port)?;
                if let Some(candidates) = &assertion.verified_identifier_digests {
                    self.push_oauth_identifier_authority_assertion(
                        &mut statements,
                        &operation_id,
                        candidates,
                        user_id,
                        identifier_user.is_none(),
                    )
                    .map_err(AdapterFailure::into_port)?;
                }
                self.push_oauth_external_account_write(
                    &mut statements,
                    &operation_id,
                    assertion.provider,
                    &assertion.subject_digests,
                    user_id,
                    command.audit.occurred_at,
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_identity_current_assertion(&mut statements, &operation_id, &principal)
                    .map_err(AdapterFailure::into_port)?;
                let grant_id = PrincipalIssuanceGrantId::parse(&stable_child_uuid(
                    &operation_id,
                    "issuance-grant",
                ))
                .map_err(|_| AdapterFailure::Corrupt.into_port())?;
                self.push_statement(
                    &mut statements,
                    ISSUANCE_GRANT_INSERT_SQL,
                    &[
                        JsValue::from_str(&grant_id.to_string()),
                        JsValue::from_str(&user_id.to_string()),
                        JsValue::from_f64(principal.snapshot.identity_revision as f64),
                        JsValue::from_f64(stored.expires_at.get() as f64),
                        JsValue::from_f64(command.audit.occurred_at.get() as f64),
                        JsValue::from_str(&operation_id),
                    ],
                )
                .map_err(AdapterFailure::into_port)?;
                self.push_assertion(&mut statements, &operation_id, "issuance_grant", 1)
                    .map_err(AdapterFailure::into_port)?;
                let receipt = base_receipt.clone().with_result_code("verified");
                self.commit_oauth_decision(
                    statements,
                    &receipt,
                    &command.audit,
                    Some(user_id),
                    AuthAuditOutcome::Allow,
                    AuthAuditReason::Authenticated,
                )
                .await
                .map_err(AdapterFailure::into_port)?;
                let outcome = self
                    .oauth_finalize_outcome_committed(&base_receipt, &command)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                telemetry.finish("verified", 1);
                Ok(outcome)
            }
            .await;
            match outcome {
                Err(PortError::Conflict) if attempt < 2 => {}
                outcome => return outcome,
            }
        }
        Err(PortError::Conflict)
    }

    async fn claim_auth_delivery(
        &self,
        now: TimestampMillis,
        lease_for: DurationMillis,
    ) -> Result<Option<AuthDeliveryClaim>, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("delivery_claim");
        if lease_for.get() > MAX_AUTH_DELIVERY_LEASE_MILLIS {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let requested_lease_expires_at = now
            .checked_add(lease_for)
            .map_err(|_| AdapterFailure::Invalid.into_port())?;
        let semantic = vec![
            lease_for.get().to_string(),
            requested_lease_expires_at.get().to_string(),
        ];
        let invocation_id = Uuid::now_v7().to_string();
        let receipt = OperationReceipt::internal("delivery_claim", invocation_id, now, &semantic)
            .with_result_code("claimed");
        if let Some(claim) = self
            .delivery_claim_committed(&receipt, requested_lease_expires_at)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("claimed", 1);
            return Ok(Some(claim));
        }
        self.materialize_verification_deliveries(now, DELIVERY_MATERIALIZE_LIMIT_PER_CLAIM)
            .await?;
        let mut cleanup = Vec::with_capacity(1);
        self.push_statement(
            &mut cleanup,
            DELIVERY_CLEANUP_SQL,
            &[JsValue::from_f64(now.get() as f64)],
        )
        .map_err(AdapterFailure::into_port)?;
        self.batch(cleanup)
            .await
            .map_err(AdapterFailure::into_port)?;

        for _ in 0..DELIVERY_CLAIM_CONFLICT_ATTEMPTS {
            let row = self
                .one::<DeliveryRow>(DELIVERY_NEXT_SQL, &[JsValue::from_f64(now.get() as f64)])
                .await
                .map_err(AdapterFailure::into_port)?;
            let Some(row) = row else {
                telemetry.finish("empty", 0);
                return Ok(None);
            };
            row.validated().map_err(AdapterFailure::into_port)?;
            let envelope = row.envelope().map_err(AdapterFailure::into_port)?;
            let attempt = u16::try_from(row.attempt)
                .ok()
                .and_then(|attempt| attempt.checked_add(1))
                .filter(|attempt| *attempt <= MAX_AUTH_DELIVERY_ATTEMPTS)
                .ok_or_else(|| AdapterFailure::Corrupt.into_port())?;
            let expires_at =
                safe_timestamp(row.expires_at_ms).map_err(AdapterFailure::into_port)?;
            let lease_expires_at = requested_lease_expires_at.min(expires_at);
            let lease_id = AuthDeliveryLeaseId::parse(&stable_child_uuid(
                &receipt.operation_id,
                "delivery-lease",
            ))
            .map_err(|_| AdapterFailure::Corrupt.into_port())?;
            let operation_id = receipt.operation_id.clone();
            let mut statements = Vec::with_capacity(3);
            self.push_statement(
                &mut statements,
                DELIVERY_CLAIM_SQL,
                &[
                    JsValue::from_str(&row.delivery_id),
                    JsValue::from_f64(row.revision as f64),
                    JsValue::from_f64(row.attempt as f64),
                    JsValue::from_str(&lease_id.to_string()),
                    JsValue::from_f64(requested_lease_expires_at.get() as f64),
                    JsValue::from_str(&operation_id),
                    JsValue::from_f64(now.get() as f64),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_assertion(&mut statements, &operation_id, "delivery", 1)
                .map_err(AdapterFailure::into_port)?;
            self.push_cleanup(&mut statements, &operation_id)
                .map_err(AdapterFailure::into_port)?;
            match self.batch_mutation(statements, &receipt).await {
                Ok(()) => {
                    let committed = self
                        .delivery_claim_committed(&receipt, requested_lease_expires_at)
                        .await
                        .map_err(AdapterFailure::into_port)?
                        .ok_or_else(|| AdapterFailure::Unavailable.into_port())?;
                    telemetry.finish("claimed", 1);
                    let _ = (lease_id, envelope, lease_expires_at, attempt);
                    return Ok(Some(committed));
                }
                Err(AdapterFailure::Conflict) => {}
                Err(failure) => return Err(failure.into_port()),
            }
        }
        Err(AdapterFailure::Conflict.into_port())
    }

    async fn acknowledge_auth_delivery(
        &self,
        claim: AuthDeliveryClaim,
        now: TimestampMillis,
    ) -> Result<AuthDeliveryAcknowledgeOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("delivery_acknowledge");
        let semantic = vec![
            claim.delivery_id().to_string(),
            claim.lease_id().to_string(),
            claim.attempt().to_string(),
            claim.lease_expires_at().get().to_string(),
        ];
        let receipt = OperationReceipt::internal(
            "delivery_acknowledge",
            claim.delivery_id().to_string(),
            claim.lease_expires_at(),
            &semantic,
        );
        if self
            .delivery_acknowledged(&receipt, &claim)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish("acknowledged", 1);
            return Ok(AuthDeliveryAcknowledgeOutcome::Acknowledged);
        }
        let row = self
            .delivery_by_id(claim.delivery_id())
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some(row) = row else {
            telemetry.finish("stale", 0);
            return Ok(AuthDeliveryAcknowledgeOutcome::StaleLease);
        };
        let current_envelope = row.envelope().map_err(AdapterFailure::into_port)?;
        if row.attempt != i64::from(claim.attempt())
            || row.lease_id.as_deref() != Some(&claim.lease_id().to_string())
            || row.lease_expires_at_ms != Some(claim.lease_expires_at().get())
            || now >= claim.lease_expires_at()
            || current_envelope != *claim.envelope()
        {
            telemetry.finish("stale", 0);
            return Ok(AuthDeliveryAcknowledgeOutcome::StaleLease);
        }
        let operation_id = receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(5);
        self.push_statement(
            &mut statements,
            DELIVERY_ACK_TOMBSTONE_INSERT_SQL,
            &[
                JsValue::from_str(&operation_id),
                JsValue::from_str(&row.delivery_id),
                JsValue::from_str(&claim.lease_id().to_string()),
                JsValue::from_f64(f64::from(claim.attempt())),
                JsValue::from_f64(claim.lease_expires_at().get() as f64),
                JsValue::from_f64(now.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_statement(
            &mut statements,
            DELIVERY_ACKNOWLEDGE_SQL,
            &[
                JsValue::from_str(&row.delivery_id),
                JsValue::from_f64(row.revision as f64),
                JsValue::from_f64(row.attempt as f64),
                JsValue::from_str(&claim.lease_id().to_string()),
                JsValue::from_f64(claim.lease_expires_at().get() as f64),
                JsValue::from_f64(now.get() as f64),
            ],
        )
        .map_err(AdapterFailure::into_port)?;
        self.push_assertion(&mut statements, &operation_id, "delivery", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) => {
                if !self
                    .delivery_acknowledged(&receipt, &claim)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    return Err(AdapterFailure::Unavailable.into_port());
                }
                telemetry.finish("acknowledged", 1);
                Ok(AuthDeliveryAcknowledgeOutcome::Acknowledged)
            }
            Err(AdapterFailure::Conflict) => {
                telemetry.finish("stale", 0);
                Ok(AuthDeliveryAcknowledgeOutcome::StaleLease)
            }
            Err(failure) => Err(failure.into_port()),
        }
    }

    async fn retry_auth_delivery(
        &self,
        claim: AuthDeliveryClaim,
        now: TimestampMillis,
        retry_at: TimestampMillis,
    ) -> Result<AuthDeliveryRetryOutcome, PortError> {
        let mut telemetry = AuthRepositoryTelemetry::span("delivery_retry");
        if retry_at <= now {
            return Err(AdapterFailure::Invalid.into_port());
        }
        let payload_fingerprint = bytes_to_hex(&stable_hash(&[
            b"frame/auth/delivery-retry/v1",
            claim.envelope().sealed_payload(),
        ]));
        let semantic = vec![
            claim.delivery_id().to_string(),
            claim.lease_id().to_string(),
            claim.attempt().to_string(),
            claim.lease_expires_at().get().to_string(),
            retry_at.get().to_string(),
            payload_fingerprint,
        ];
        let base_receipt = OperationReceipt::internal(
            "delivery_retry",
            claim.delivery_id().to_string(),
            claim.lease_expires_at(),
            &semantic,
        );
        if let Some(outcome) = self
            .delivery_retry_committed(&base_receipt, &claim)
            .await
            .map_err(AdapterFailure::into_port)?
        {
            telemetry.finish(
                match outcome {
                    AuthDeliveryRetryOutcome::Scheduled => "scheduled",
                    AuthDeliveryRetryOutcome::Exhausted => "exhausted",
                    AuthDeliveryRetryOutcome::StaleLease => "stale",
                },
                usize::from(outcome != AuthDeliveryRetryOutcome::StaleLease),
            );
            return Ok(outcome);
        }
        let row = self
            .delivery_by_id(claim.delivery_id())
            .await
            .map_err(AdapterFailure::into_port)?;
        let Some(row) = row else {
            telemetry.finish("stale", 0);
            return Ok(AuthDeliveryRetryOutcome::StaleLease);
        };
        let current_envelope = row.envelope().map_err(AdapterFailure::into_port)?;
        if row.attempt != i64::from(claim.attempt())
            || row.lease_id.as_deref() != Some(&claim.lease_id().to_string())
            || row.lease_expires_at_ms != Some(claim.lease_expires_at().get())
            || now >= claim.lease_expires_at()
            || current_envelope != *claim.envelope()
        {
            telemetry.finish("stale", 0);
            return Ok(AuthDeliveryRetryOutcome::StaleLease);
        }
        let expires_at = safe_timestamp(row.expires_at_ms).map_err(AdapterFailure::into_port)?;
        let exhausted = claim.attempt() >= MAX_AUTH_DELIVERY_ATTEMPTS || retry_at >= expires_at;
        let operation_id = base_receipt.operation_id.clone();
        let mut statements = Vec::with_capacity(5);
        if exhausted {
            self.push_statement(
                &mut statements,
                DELIVERY_ACK_TOMBSTONE_INSERT_SQL,
                &[
                    JsValue::from_str(&operation_id),
                    JsValue::from_str(&row.delivery_id),
                    JsValue::from_str(&claim.lease_id().to_string()),
                    JsValue::from_f64(f64::from(claim.attempt())),
                    JsValue::from_f64(claim.lease_expires_at().get() as f64),
                    JsValue::from_f64(now.get() as f64),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
            self.push_statement(
                &mut statements,
                DELIVERY_ACKNOWLEDGE_SQL,
                &[
                    JsValue::from_str(&row.delivery_id),
                    JsValue::from_f64(row.revision as f64),
                    JsValue::from_f64(row.attempt as f64),
                    JsValue::from_str(&claim.lease_id().to_string()),
                    JsValue::from_f64(claim.lease_expires_at().get() as f64),
                    JsValue::from_f64(now.get() as f64),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
        } else {
            self.push_statement(
                &mut statements,
                DELIVERY_RETRY_SQL,
                &[
                    JsValue::from_str(&row.delivery_id),
                    JsValue::from_f64(row.revision as f64),
                    JsValue::from_f64(row.attempt as f64),
                    JsValue::from_str(&claim.lease_id().to_string()),
                    JsValue::from_f64(claim.lease_expires_at().get() as f64),
                    JsValue::from_f64(now.get() as f64),
                    JsValue::from_f64(retry_at.get() as f64),
                    JsValue::from_str(&operation_id),
                ],
            )
            .map_err(AdapterFailure::into_port)?;
        }
        self.push_assertion(&mut statements, &operation_id, "delivery", 1)
            .map_err(AdapterFailure::into_port)?;
        self.push_cleanup(&mut statements, &operation_id)
            .map_err(AdapterFailure::into_port)?;
        let receipt = if exhausted {
            base_receipt.clone().with_result_code("exhausted")
        } else {
            base_receipt
                .clone()
                .with_result_code("scheduled")
                .with_result_timestamp(retry_at)
        };
        match self.batch_mutation(statements, &receipt).await {
            Ok(()) if exhausted => {
                if self
                    .delivery_retry_committed(&receipt, &claim)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    != Some(AuthDeliveryRetryOutcome::Exhausted)
                {
                    return Err(AdapterFailure::Unavailable.into_port());
                }
                telemetry.finish("exhausted", 1);
                Ok(AuthDeliveryRetryOutcome::Exhausted)
            }
            Ok(()) => {
                if self
                    .delivery_retry_committed(&receipt, &claim)
                    .await
                    .map_err(AdapterFailure::into_port)?
                    != Some(AuthDeliveryRetryOutcome::Scheduled)
                {
                    return Err(AdapterFailure::Unavailable.into_port());
                }
                telemetry.finish("scheduled", 1);
                Ok(AuthDeliveryRetryOutcome::Scheduled)
            }
            Err(AdapterFailure::Conflict) => {
                if let Some(outcome) = self
                    .delivery_retry_committed(&receipt, &claim)
                    .await
                    .map_err(AdapterFailure::into_port)?
                {
                    telemetry.finish(
                        if outcome == AuthDeliveryRetryOutcome::Exhausted {
                            "exhausted"
                        } else {
                            "scheduled"
                        },
                        1,
                    );
                    return Ok(outcome);
                }
                telemetry.finish("stale", 0);
                Ok(AuthDeliveryRetryOutcome::StaleLease)
            }
            Err(failure) => Err(failure.into_port()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(version: u16, marker: char) -> VersionedSecretDigest {
        VersionedSecretDigest::new(
            HashKeyVersion::new(version).expect("version"),
            SecretDigest::parse_sha256(marker.to_string().repeat(64)).expect("digest"),
        )
    }

    #[test]
    fn digest_candidate_wire_is_bounded_canonical_and_secret_free() {
        let candidates =
            SecretDigestCandidates::new(digest(2, 'b'), vec![digest(1, 'a')]).expect("candidates");
        let json = candidates_json(&candidates).expect("json");
        assert!(json.len() <= MAX_CANDIDATE_JSON_BYTES);
        assert_eq!(parse_candidates_json(&json).expect("parse"), candidates);
        assert!(!json.contains("token"));
        assert!(!json.contains("cookie"));
        assert!(!json.contains("otp"));
    }

    #[test]
    fn checked_in_queries_bind_all_external_values() {
        for sql in [
            PRINCIPAL_BY_USER_SQL,
            IDENTIFIER_OWNER_SQL,
            SESSION_BY_CREDENTIALS_SQL,
            MUTATION_GRANT_SQL,
            VERIFICATION_CANDIDATE_SQL,
            API_KEY_BY_CREDENTIALS_SQL,
            RATE_BUCKETS_SQL,
            AUDIT_INSERT_SQL,
            RATE_BUCKET_UPSERT_SQL,
            SESSION_INSERT_SQL,
            PENDING_INSERT_SQL,
            OAUTH_FLOW_INSERT_SQL,
            OAUTH_RESERVATION_INSERT_SQL,
            OAUTH_EXTERNAL_ACCOUNT_UPSERT_SQL,
        ] {
            assert!(sql.contains("?1"));
            assert!(!sql.contains("raw-session"));
            assert!(!sql.contains("raw-api-key"));
        }
    }

    #[test]
    fn oauth_enum_names_match_the_persisted_contract() {
        assert_eq!(
            enum_name(AuthAbuseAction::OAuthBegin).expect("abuse action"),
            "oauth_begin"
        );
        assert_eq!(
            enum_name(AuthAbuseAction::OAuthExchange).expect("abuse action"),
            "oauth_exchange"
        );
        assert_eq!(
            enum_name(AuthAuditAction::OAuthExchangePreflight).expect("audit action"),
            "oauth_exchange_preflight"
        );
    }

    #[test]
    fn adapter_errors_and_telemetry_are_stable_and_privacy_safe() {
        for failure in [
            AdapterFailure::Invalid,
            AdapterFailure::Conflict,
            AdapterFailure::Unavailable,
            AdapterFailure::Timeout,
            AdapterFailure::Corrupt,
        ] {
            assert!(failure.code().starts_with("auth_repository_"));
            assert!(!failure.code().contains('@'));
        }
        let event = AuthRepositoryTelemetry {
            event: "d1_auth_repository",
            operation: "session_authenticate",
            outcome: "deny",
            duration_ms: 4,
            rows: 1,
        };
        let json = serde_json::to_string(&event).expect("telemetry");
        for forbidden in [
            "SELECT",
            "token_digest",
            "identifier_digest",
            "example.test",
        ] {
            assert!(!json.contains(forbidden));
        }
    }

    #[test]
    fn cas_conflict_parser_requires_the_exact_structured_trigger_message() {
        let exact = "D1: D1Error {\n    cause: JsValue(Error: frame_auth_cas_conflict_v1: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)\n    Error: frame_auth_cas_conflict_v1: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)\n        at D1DatabaseSessionAlwaysPrimary._sendOrThrow (cloudflare-internal:d1-api:147:24)\n        at async cloudflare-internal:d1-api:97:27),\n}";
        let spoof = "D1: D1Error {\n    cause: JsValue(Error: provider frame_auth_cas_conflict_v1 spoof: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)\n    Error: provider frame_auth_cas_conflict_v1 spoof: SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)\n        at D1DatabaseSessionAlwaysPrimary._sendOrThrow (cloudflare-internal:d1-api:147:24)\n        at async cloudflare-internal:d1-api:97:27),\n}";
        let suffix_spoof = exact.replace(
            "frame_auth_cas_conflict_v1: SQLITE_CONSTRAINT",
            "frame_auth_cas_conflict_v1 suffix: SQLITE_CONSTRAINT",
        );
        let check_constraint = exact.replace(
            "SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)",
            "SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_CHECK)",
        );
        let unique_constraint = exact.replace(
            "SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_TRIGGER)",
            "SQLITE_CONSTRAINT (extended: SQLITE_CONSTRAINT_UNIQUE)",
        );
        assert!(exact_d1_trigger_error(exact));
        assert!(!exact_d1_trigger_error(spoof));
        assert!(!exact_d1_trigger_error(&suffix_spoof));
        assert!(!exact_d1_trigger_error(&check_constraint));
        assert!(!exact_d1_trigger_error(&unique_constraint));
        assert!(!exact_d1_trigger_error(AUTH_CAS_CONFLICT_SENTINEL));
        assert!(!exact_d1_trigger_error(&format!("provider: {exact}")));
        assert_eq!(
            D1AuthStateRepository::mutation_error(&worker::Error::RustError(exact.into())),
            AdapterFailure::Conflict
        );
        for unknown_envelope in [
            AUTH_CAS_CONFLICT_SENTINEL.to_string(),
            format!("D1_ERROR: {AUTH_CAS_CONFLICT_SENTINEL}"),
            format!("D1_ERROR: {AUTH_CAS_CONFLICT_SENTINEL}: SQLITE_CONSTRAINT"),
            spoof.to_string(),
            suffix_spoof,
            check_constraint,
            unique_constraint,
        ] {
            assert_eq!(
                D1AuthStateRepository::mutation_error(&worker::Error::RustError(unknown_envelope)),
                AdapterFailure::Unavailable
            );
        }
    }

    #[test]
    fn migration_is_expand_only_and_has_no_plaintext_secret_columns() {
        let migration = include_str!("../migrations/0009_auth_repository_expand.sql");
        let oauth_migration = include_str!("../migrations/0023_auth_oauth_direct_upload.sql");
        for destructive in [
            "DROP TABLE",
            "DROP COLUMN",
            "DELETE FROM users",
            "RENAME TO",
        ] {
            assert!(!migration.contains(destructive));
        }
        for forbidden in ["raw_token", "raw_otp", "api_key_plaintext", "oauth_code"] {
            assert!(!migration.contains(forbidden));
            assert!(!oauth_migration.contains(forbidden));
        }
        for forbidden in ["authorization_code", "client_secret", "raw_state"] {
            assert!(!oauth_migration.contains(forbidden));
        }
        assert!(migration.contains("auth_repository_assertions_v2"));
        assert!(oauth_migration.contains("auth_oauth_operations_v2"));
        assert!(oauth_migration.contains("auth_external_accounts_v2"));
        assert!(migration.contains("authentication audit is append-only"));
        assert_eq!(migration.matches(AUTH_CAS_CONFLICT_SENTINEL).count(), 2);
        assert!(!migration.contains("authentication rate bucket capacity reached"));
    }
}
