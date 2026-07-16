PRAGMA foreign_keys = ON;

-- This migration is intentionally expand-only. The v1 identity tables remain
-- readable for a later compatibility/backfill rehearsal; the capability-safe
-- repository writes only the normalized v2 tables below.
CREATE TABLE auth_identities_v2 (
  user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  identity_revision INTEGER NOT NULL CHECK (identity_revision BETWEEN 1 AND 9007199254740991),
  session_version INTEGER NOT NULL DEFAULT 0 CHECK (session_version BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36)
);

CREATE TABLE auth_identifier_digests_v2 (
  key_version INTEGER NOT NULL CHECK (key_version BETWEEN 1 AND 65535),
  digest TEXT NOT NULL CHECK (length(digest) = 64 AND digest NOT GLOB '*[^0-9a-f]*'),
  user_id TEXT NOT NULL REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  PRIMARY KEY (key_version, digest)
);
CREATE INDEX auth_identifier_digests_v2_user_idx
  ON auth_identifier_digests_v2(user_id, key_version);

CREATE TABLE auth_sessions_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  family_id TEXT NOT NULL CHECK (length(family_id) = 36),
  user_id TEXT NOT NULL REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  client_kind TEXT NOT NULL CHECK (client_kind IN ('browser', 'desktop', 'mobile', 'extension', 'api')),
  token_key_version INTEGER NOT NULL CHECK (token_key_version BETWEEN 1 AND 65535),
  token_digest TEXT NOT NULL CHECK (length(token_digest) = 64 AND token_digest NOT GLOB '*[^0-9a-f]*'),
  csrf_key_version INTEGER CHECK (csrf_key_version IS NULL OR csrf_key_version BETWEEN 1 AND 65535),
  csrf_digest TEXT CHECK (csrf_digest IS NULL OR (length(csrf_digest) = 64 AND csrf_digest NOT GLOB '*[^0-9a-f]*')),
  browser_origin TEXT CHECK (browser_origin IS NULL OR length(browser_origin) BETWEEN 8 AND 255),
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  rotated_at_ms INTEGER NOT NULL CHECK (rotated_at_ms BETWEEN 0 AND 9007199254740991),
  idle_expires_at_ms INTEGER NOT NULL CHECK (idle_expires_at_ms BETWEEN 0 AND 9007199254740991),
  absolute_expires_at_ms INTEGER NOT NULL CHECK (absolute_expires_at_ms BETWEEN 0 AND 9007199254740991),
  session_version INTEGER NOT NULL CHECK (session_version BETWEEN 0 AND 9007199254740991),
  generation INTEGER NOT NULL DEFAULT 0 CHECK (generation BETWEEN 0 AND 9007199254740991),
  state TEXT NOT NULL CHECK (state IN ('active', 'revoked')),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  revocation_reason TEXT CHECK (revocation_reason IS NULL OR revocation_reason IN (
    'user_logout', 'logout_all', 'replay_detected', 'expired',
    'session_version_changed', 'account_recovery', 'operator'
  )),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (rotated_at_ms >= issued_at_ms),
  CHECK (idle_expires_at_ms > issued_at_ms AND idle_expires_at_ms <= absolute_expires_at_ms),
  CHECK ((client_kind = 'browser') = (csrf_key_version IS NOT NULL AND csrf_digest IS NOT NULL AND browser_origin IS NOT NULL)),
  CHECK ((state = 'active' AND revoked_at_ms IS NULL AND revocation_reason IS NULL)
      OR (state = 'revoked' AND revoked_at_ms IS NOT NULL AND revocation_reason IS NOT NULL)),
  UNIQUE (token_key_version, token_digest)
);
CREATE INDEX auth_sessions_v2_user_state_idx
  ON auth_sessions_v2(user_id, state, absolute_expires_at_ms);
CREATE INDEX auth_sessions_v2_family_idx
  ON auth_sessions_v2(family_id, state);

CREATE TABLE auth_session_credentials_v2 (
  key_version INTEGER NOT NULL CHECK (key_version BETWEEN 1 AND 65535),
  digest TEXT NOT NULL CHECK (length(digest) = 64 AND digest NOT GLOB '*[^0-9a-f]*'),
  session_id TEXT NOT NULL REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  family_id TEXT NOT NULL CHECK (length(family_id) = 36),
  state TEXT NOT NULL CHECK (state IN ('current', 'rotated', 'revoked')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  PRIMARY KEY (key_version, digest)
);
CREATE INDEX auth_session_credentials_v2_session_idx
  ON auth_session_credentials_v2(session_id, state);
CREATE INDEX auth_session_credentials_v2_family_idx
  ON auth_session_credentials_v2(family_id, state);

CREATE TABLE auth_session_mutation_grants_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  session_id TEXT NOT NULL REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  generation INTEGER NOT NULL CHECK (generation BETWEEN 0 AND 9007199254740991),
  token_key_version INTEGER NOT NULL CHECK (token_key_version BETWEEN 1 AND 65535),
  token_digest TEXT NOT NULL CHECK (length(token_digest) = 64 AND token_digest NOT GLOB '*[^0-9a-f]*'),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36)
);
CREATE UNIQUE INDEX auth_session_mutation_grants_v2_one_per_session_idx
  ON auth_session_mutation_grants_v2(session_id);

CREATE TABLE auth_principal_issuance_grants_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  user_id TEXT NOT NULL REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  identity_revision INTEGER NOT NULL CHECK (identity_revision BETWEEN 1 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36)
);
CREATE INDEX auth_principal_issuance_grants_v2_expiry_idx
  ON auth_principal_issuance_grants_v2(expires_at_ms);

CREATE TABLE auth_identity_provisioning_grants_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  user_id TEXT NOT NULL CHECK (length(user_id) = 36),
  identity_revision INTEGER NOT NULL CHECK (identity_revision BETWEEN 1 AND 9007199254740991),
  identifier_key_version INTEGER NOT NULL CHECK (identifier_key_version BETWEEN 1 AND 65535),
  identifier_digest TEXT NOT NULL CHECK (length(identifier_digest) = 64 AND identifier_digest NOT GLOB '*[^0-9a-f]*'),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36)
);
CREATE INDEX auth_identity_provisioning_grants_v2_expiry_idx
  ON auth_identity_provisioning_grants_v2(expires_at_ms);

CREATE TABLE auth_pending_verifications_v2 (
  delivery_id TEXT PRIMARY KEY NOT NULL CHECK (length(delivery_id) = 36),
  identifier_candidates_json TEXT NOT NULL CHECK (json_valid(identifier_candidates_json) AND length(identifier_candidates_json) BETWEEN 86 AND 1024),
  active_identifier_key_version INTEGER NOT NULL CHECK (active_identifier_key_version BETWEEN 1 AND 65535),
  active_identifier_digest TEXT NOT NULL CHECK (length(active_identifier_digest) = 64 AND active_identifier_digest NOT GLOB '*[^0-9a-f]*'),
  secret_key_version INTEGER NOT NULL CHECK (secret_key_version BETWEEN 1 AND 65535),
  secret_digest TEXT NOT NULL CHECK (length(secret_digest) = 64 AND secret_digest NOT GLOB '*[^0-9a-f]*'),
  purpose TEXT NOT NULL CHECK (purpose IN ('identity_provisioning', 'email_verify', 'sign_in', 'account_recovery', 'account_link')),
  channel TEXT NOT NULL CHECK (channel IN ('magic_link', 'one_time_code')),
  initiator_session_id TEXT REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  initiator_user_id TEXT REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  initiator_generation INTEGER CHECK (initiator_generation IS NULL OR initiator_generation BETWEEN 0 AND 9007199254740991),
  provisioning_user_id TEXT CHECK (provisioning_user_id IS NULL OR length(provisioning_user_id) = 36),
  provisioning_revision INTEGER CHECK (provisioning_revision IS NULL OR provisioning_revision BETWEEN 1 AND 9007199254740991),
  max_attempts INTEGER NOT NULL CHECK (max_attempts BETWEEN 1 AND 100),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  sealed_payload_hex TEXT NOT NULL CHECK (
    length(sealed_payload_hex) BETWEEN 64 AND 131072
    AND length(sealed_payload_hex) % 2 = 0
    AND sealed_payload_hex NOT GLOB '*[^0-9a-f]*'
  ),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (expires_at_ms > created_at_ms),
  CHECK ((initiator_session_id IS NULL) = (initiator_user_id IS NULL)),
  CHECK ((initiator_session_id IS NULL) = (initiator_generation IS NULL)),
  CHECK ((provisioning_user_id IS NULL) = (provisioning_revision IS NULL)),
  CHECK ((purpose = 'account_link') = (initiator_session_id IS NOT NULL)),
  CHECK ((purpose = 'identity_provisioning') = (provisioning_user_id IS NOT NULL))
);
CREATE INDEX auth_pending_verifications_v2_ready_idx
  ON auth_pending_verifications_v2(created_at_ms, delivery_id);
CREATE INDEX auth_pending_verifications_v2_identifier_idx
  ON auth_pending_verifications_v2(active_identifier_key_version, active_identifier_digest, purpose);

CREATE TABLE auth_verification_challenges_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  user_id TEXT CHECK (user_id IS NULL OR length(user_id) = 36),
  initiator_session_id TEXT REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  initiator_user_id TEXT REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  initiator_generation INTEGER CHECK (initiator_generation IS NULL OR initiator_generation BETWEEN 0 AND 9007199254740991),
  provisioning_revision INTEGER CHECK (provisioning_revision IS NULL OR provisioning_revision BETWEEN 1 AND 9007199254740991),
  identifier_key_version INTEGER NOT NULL CHECK (identifier_key_version BETWEEN 1 AND 65535),
  identifier_digest TEXT NOT NULL CHECK (length(identifier_digest) = 64 AND identifier_digest NOT GLOB '*[^0-9a-f]*'),
  secret_key_version INTEGER NOT NULL CHECK (secret_key_version BETWEEN 1 AND 65535),
  secret_digest TEXT NOT NULL CHECK (length(secret_digest) = 64 AND secret_digest NOT GLOB '*[^0-9a-f]*'),
  purpose TEXT NOT NULL CHECK (purpose IN ('identity_provisioning', 'email_verify', 'sign_in', 'account_recovery', 'account_link')),
  channel TEXT NOT NULL CHECK (channel IN ('magic_link', 'one_time_code')),
  attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 0 AND 100),
  max_attempts INTEGER NOT NULL CHECK (max_attempts BETWEEN 1 AND 100),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  consumed_at_ms INTEGER CHECK (consumed_at_ms IS NULL OR consumed_at_ms BETWEEN 0 AND 9007199254740991),
  state TEXT NOT NULL CHECK (state IN ('pending', 'consumed', 'locked', 'expired', 'revoked')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (expires_at_ms > created_at_ms),
  CHECK ((initiator_session_id IS NULL) = (initiator_user_id IS NULL)),
  CHECK ((initiator_session_id IS NULL) = (initiator_generation IS NULL)),
  CHECK ((state = 'consumed') = (consumed_at_ms IS NOT NULL)),
  CHECK ((purpose = 'account_link') = (initiator_session_id IS NOT NULL)),
  CHECK ((purpose = 'identity_provisioning') = (provisioning_revision IS NOT NULL))
);
CREATE INDEX auth_verification_challenges_v2_lookup_idx
  ON auth_verification_challenges_v2(identifier_key_version, identifier_digest, purpose, created_at_ms DESC);
CREATE INDEX auth_verification_challenges_v2_expiry_idx
  ON auth_verification_challenges_v2(state, expires_at_ms);

CREATE TABLE auth_delivery_outbox_v2 (
  delivery_id TEXT PRIMARY KEY NOT NULL CHECK (length(delivery_id) = 36),
  sealed_payload_hex TEXT NOT NULL CHECK (
    length(sealed_payload_hex) BETWEEN 64 AND 131072
    AND length(sealed_payload_hex) % 2 = 0
    AND sealed_payload_hex NOT GLOB '*[^0-9a-f]*'
  ),
  suppress INTEGER NOT NULL CHECK (suppress IN (0, 1)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  next_attempt_at_ms INTEGER NOT NULL CHECK (next_attempt_at_ms BETWEEN 0 AND 9007199254740991),
  attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt BETWEEN 0 AND 12),
  lease_id TEXT CHECK (lease_id IS NULL OR length(lease_id) = 36),
  lease_expires_at_ms INTEGER CHECK (lease_expires_at_ms IS NULL OR lease_expires_at_ms BETWEEN 0 AND 9007199254740991),
  initiator_session_id TEXT REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (expires_at_ms > created_at_ms),
  CHECK ((lease_id IS NULL) = (lease_expires_at_ms IS NULL))
);
CREATE INDEX auth_delivery_outbox_v2_ready_idx
  ON auth_delivery_outbox_v2(suppress, next_attempt_at_ms, lease_expires_at_ms, created_at_ms);

CREATE TABLE auth_api_keys_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  owner_id TEXT NOT NULL REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  tenant_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  key_version INTEGER NOT NULL CHECK (key_version BETWEEN 1 AND 65535),
  key_digest TEXT NOT NULL CHECK (length(key_digest) = 64 AND key_digest NOT GLOB '*[^0-9a-f]*'),
  scopes_json TEXT NOT NULL CHECK (json_valid(scopes_json) AND json_type(scopes_json) = 'array' AND json_array_length(scopes_json) BETWEEN 1 AND 32),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER CHECK (expires_at_ms IS NULL OR expires_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  UNIQUE (key_version, key_digest)
);
CREATE INDEX auth_api_keys_v2_owner_idx ON auth_api_keys_v2(owner_id, created_at_ms DESC);
CREATE INDEX auth_api_keys_v2_tenant_idx ON auth_api_keys_v2(tenant_id, revoked_at_ms);

CREATE TABLE auth_rate_limit_buckets_v2 (
  action TEXT NOT NULL CHECK (action IN (
    'session_issue', 'identity_provision_issue', 'sign_in_issue', 'verify',
    'recover_issue', 'account_link_issue', 'api_key_authenticate', 'oauth_begin', 'oauth_exchange'
  )),
  dimension TEXT NOT NULL CHECK (dimension IN ('identifier', 'source', 'device', 'global')),
  key_version INTEGER NOT NULL CHECK (key_version BETWEEN 0 AND 65535),
  digest TEXT NOT NULL CHECK (length(digest) IN (0, 64) AND digest NOT GLOB '*[^0-9a-f]*'),
  window_started_at_ms INTEGER NOT NULL CHECK (window_started_at_ms BETWEEN 0 AND 9007199254740991),
  attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 0 AND 1000000),
  blocked_until_ms INTEGER CHECK (blocked_until_ms IS NULL OR blocked_until_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  gc_at_ms INTEGER NOT NULL CHECK (gc_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  PRIMARY KEY (action, dimension, key_version, digest),
  CHECK ((dimension = 'global' AND key_version = 0 AND digest = '')
      OR (dimension <> 'global' AND key_version BETWEEN 1 AND 65535 AND length(digest) = 64))
);
CREATE INDEX auth_rate_limit_buckets_v2_gc_idx ON auth_rate_limit_buckets_v2(gc_at_ms);
CREATE INDEX auth_rate_limit_buckets_v2_blocked_idx
  ON auth_rate_limit_buckets_v2(action, blocked_until_ms) WHERE blocked_until_ms IS NOT NULL;

-- The cardinality cap is an optimistic precondition, not a semantic provider
-- failure. A writer that loses the 4095 -> 4096 race receives the same exact,
-- repository-owned CAS sentinel as a stale row assertion, rebuilds from a
-- fresh count, and then returns the semantic rate-limit result.
CREATE TRIGGER auth_rate_limit_buckets_v2_cardinality_cap
BEFORE INSERT ON auth_rate_limit_buckets_v2
WHEN (SELECT COUNT(*) FROM auth_rate_limit_buckets_v2) >= 4096
  AND NOT EXISTS (
    SELECT 1 FROM auth_rate_limit_buckets_v2 b
    WHERE b.action = NEW.action
      AND b.dimension = NEW.dimension
      AND b.key_version = NEW.key_version
      AND b.digest = NEW.digest
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_auth_cas_conflict_v1');
END;

CREATE TABLE auth_audit_events_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36),
  user_id TEXT CHECK (user_id IS NULL OR length(user_id) = 36),
  session_id TEXT CHECK (session_id IS NULL OR length(session_id) = 36),
  client_kind TEXT CHECK (client_kind IS NULL OR client_kind IN ('browser', 'desktop', 'mobile', 'extension', 'api')),
  action TEXT NOT NULL CHECK (action IN (
    'session_issue', 'session_authenticate', 'session_rotate', 'browser_mutation_authenticate',
    'logout', 'logout_all', 'verification_issue', 'verification_consume', 'api_key_issue',
    'api_key_authenticate', 'api_key_revoke', 'oauth_begin', 'oauth_exchange_preflight',
    'oauth_exchange', 'identity_provision', 'account_link'
  )),
  outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'error')),
  reason TEXT NOT NULL CHECK (reason IN (
    'issued', 'authenticated', 'rotated', 'logged_out', 'logged_out_all',
    'verification_accepted', 'verification_completed', 'invalid_credential', 'expired',
    'revoked', 'session_version_mismatch', 'replay_detected', 'csrf_mismatch',
    'origin_mismatch', 'fetch_metadata_mismatch', 'rate_limited', 'attempts_exhausted',
    'insufficient_role', 'linked', 'key_version_migrated', 'adapter_failure'
  )),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36)
);
CREATE INDEX auth_audit_events_v2_user_time_idx
  ON auth_audit_events_v2(user_id, occurred_at_ms DESC);
CREATE INDEX auth_audit_events_v2_correlation_idx
  ON auth_audit_events_v2(correlation_id);
CREATE INDEX auth_audit_events_v2_operation_idx
  ON auth_audit_events_v2(operation_id);

CREATE TRIGGER auth_audit_events_v2_immutable_update
BEFORE UPDATE ON auth_audit_events_v2
BEGIN
  SELECT RAISE(ABORT, 'authentication audit is append-only');
END;

CREATE TRIGGER auth_audit_events_v2_immutable_delete
BEFORE DELETE ON auth_audit_events_v2
BEGIN
  SELECT RAISE(ABORT, 'authentication audit is append-only');
END;

-- Conditional writes are followed immediately by an assertion insert inside
-- one D1 batch. A zero/ambiguous row count violates this CHECK and rolls back
-- the entire batch, including its audit row.
CREATE TABLE auth_repository_assertions_v2 (
  id TEXT PRIMARY KEY NOT NULL,
  satisfied INTEGER NOT NULL CHECK (satisfied = 1)
);

-- This exact marker is the only provider error text the adapter recognizes.
-- It is owned by this repository and distinguishes stale optimistic row and
-- cardinality predicates from unrelated D1, trigger, or provider failures.
CREATE TRIGGER auth_repository_assertions_v2_conflict
BEFORE INSERT ON auth_repository_assertions_v2
WHEN NEW.satisfied <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_auth_cas_conflict_v1');
END;

-- Caller-stable, redacted mutation receipts. `operation_id` is derived only
-- from the caller correlation id and fixed operation class; the fingerprint
-- binds the complete semantic request without persisting credentials or row
-- values. Insertion is part of the same batch as the state and audit writes.
CREATE TABLE auth_repository_operations_v2 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'identity_provision', 'session_issue', 'session_authenticate',
    'session_rotate', 'session_revoke', 'session_logout_all',
    'verification_issue', 'verification_attempt', 'verification_materialize',
    'api_key_issue', 'api_key_authenticate', 'api_key_revoke',
    'delivery_claim', 'delivery_acknowledge', 'delivery_retry', 'audit'
  )),
  subject_id TEXT NOT NULL CHECK (length(subject_id) BETWEEN 1 AND 96),
  result_code TEXT NOT NULL CHECK (
    length(result_code) BETWEEN 2 AND 32
    AND result_code NOT GLOB '*[^a-z_]*'
  ),
  result_timestamp_ms INTEGER CHECK (
    result_timestamp_ms IS NULL
    OR result_timestamp_ms BETWEEN 0 AND 9007199254740991
  ),
  request_fingerprint TEXT NOT NULL CHECK (
    length(request_fingerprint) = 64
    AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX auth_repository_operations_v2_subject_idx
  ON auth_repository_operations_v2(operation_kind, subject_id);

-- A delivery acknowledgement deletes the live row, so its exact lease fence
-- must survive as a tombstone for idempotent retry and lost-ack recovery.
CREATE TABLE auth_delivery_ack_tombstones_v2 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  delivery_id TEXT NOT NULL CHECK (length(delivery_id) = 36),
  lease_id TEXT NOT NULL CHECK (length(lease_id) = 36),
  attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 12),
  lease_expires_at_ms INTEGER NOT NULL CHECK (lease_expires_at_ms BETWEEN 0 AND 9007199254740991),
  acknowledged_at_ms INTEGER NOT NULL CHECK (acknowledged_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (delivery_id, lease_id, attempt, lease_expires_at_ms)
);
