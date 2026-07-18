PRAGMA foreign_keys = ON;

-- Exact Cap mobile-email challenges. Only the normalized-email digest and the
-- source-compatible SHA-256(code || NEXTAUTH_SECRET) token enter D1. Repeated
-- requests replace the complete challenge, matching the source upsert by its
-- `mobile:<sha256(email)>` identifier.
CREATE TABLE legacy_mobile_session_challenges_v1 (
  identifier_digest TEXT PRIMARY KEY NOT NULL CHECK (
    length(identifier_digest) = 64
    AND lower(identifier_digest) = identifier_digest
    AND identifier_digest NOT GLOB '*[^0-9a-f]*'
  ),
  token_digest TEXT NOT NULL CHECK (
    length(token_digest) = 64
    AND lower(token_digest) = token_digest
    AND token_digest NOT GLOB '*[^0-9a-f]*'
  ),
  delivery_id TEXT NOT NULL
    REFERENCES auth_delivery_provider_handoffs_v1(delivery_id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL
    CHECK (expires_at_ms = created_at_ms + 600000),
  request_operation_id TEXT NOT NULL CHECK (length(request_operation_id) = 36)
);
CREATE INDEX legacy_mobile_session_challenge_expiry_v1
  ON legacy_mobile_session_challenges_v1(expires_at_ms, identifier_digest);

CREATE TABLE legacy_mobile_session_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  action TEXT NOT NULL CHECK (action IN (
    'email_request', 'email_verify', 'session_request', 'session_revoke'
  )),
  actor_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64
    AND lower(subject_digest) = subject_digest
    AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  provider_effect TEXT NOT NULL CHECK (provider_effect IN (
    'not_requested', 'email_handoff_pending', 'stripe_sync_pending'
  )),
  state TEXT NOT NULL CHECK (state = 'complete'),
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER NOT NULL
    CHECK (completed_at_ms = created_at_ms),
  CHECK (
    (action = 'email_request' AND actor_id IS NULL
      AND provider_effect = 'email_handoff_pending')
    OR (action = 'email_verify' AND actor_id IS NOT NULL
      AND provider_effect IN ('not_requested', 'stripe_sync_pending'))
    OR (action IN ('session_request', 'session_revoke') AND actor_id IS NOT NULL
      AND provider_effect = 'not_requested')
  )
);
CREATE INDEX legacy_mobile_session_operations_actor_time_v1
  ON legacy_mobile_session_operations_v1(actor_id, created_at_ms DESC, operation_id);

CREATE TRIGGER legacy_mobile_session_operations_no_update_v1
BEFORE UPDATE ON legacy_mobile_session_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_session_operation_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_session_operations_no_delete_v1
BEFORE DELETE ON legacy_mobile_session_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_session_operation_immutable_v1');
END;

CREATE TABLE legacy_mobile_session_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_mobile_session_operations_v1(operation_id) ON DELETE RESTRICT,
  outcome TEXT NOT NULL CHECK (outcome IN (
    'challenge_replaced', 'api_key_replaced',
    'user_provisioned_provider_pending', 'api_key_revoked'
  )),
  user_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  legacy_user_id TEXT CHECK (
    legacy_user_id IS NULL OR (
      length(legacy_user_id) = 15
      AND legacy_user_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  -- Historical row identifier only: source parity requires later mobile-key
  -- replacement to physically delete this credential.
  api_key_row_id TEXT CHECK (api_key_row_id IS NULL OR length(api_key_row_id) = 36),
  delivery_id TEXT
    REFERENCES auth_delivery_provider_handoffs_v1(delivery_id) ON DELETE RESTRICT,
  affected_key_count INTEGER NOT NULL CHECK (affected_key_count BETWEEN 0 AND 1000000),
  completed_at_ms INTEGER NOT NULL
    CHECK (completed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (outcome = 'challenge_replaced' AND user_id IS NULL AND legacy_user_id IS NULL
      AND api_key_row_id IS NULL AND delivery_id IS NOT NULL AND affected_key_count = 0)
    OR (outcome = 'api_key_replaced' AND user_id IS NOT NULL AND legacy_user_id IS NOT NULL
      AND api_key_row_id IS NOT NULL AND delivery_id IS NULL AND affected_key_count >= 1)
    OR (outcome = 'user_provisioned_provider_pending' AND user_id IS NOT NULL
      AND legacy_user_id IS NOT NULL AND api_key_row_id IS NULL
      AND delivery_id IS NULL AND affected_key_count = 0)
    OR (outcome = 'api_key_revoked' AND user_id IS NOT NULL
      AND legacy_user_id IS NOT NULL AND api_key_row_id IS NULL
      AND delivery_id IS NULL)
  )
);

CREATE TABLE legacy_mobile_session_stripe_effects_v1 (
  effect_id TEXT PRIMARY KEY NOT NULL CHECK (length(effect_id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_mobile_session_operations_v1(operation_id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  normalized_email_digest TEXT NOT NULL CHECK (
    length(normalized_email_digest) = 64
    AND lower(normalized_email_digest) = normalized_email_digest
    AND normalized_email_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'delivering', 'applied', 'exhausted')),
  attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt BETWEEN 0 AND 12),
  lease_id TEXT,
  lease_expires_at_ms INTEGER,
  provider_receipt_digest TEXT,
  last_error_class TEXT,
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL
    CHECK (updated_at_ms BETWEEN created_at_ms AND 9007199254740991),
  CHECK (
    (state = 'pending' AND attempt = 0 AND lease_id IS NULL
      AND lease_expires_at_ms IS NULL AND provider_receipt_digest IS NULL
      AND last_error_class IS NULL AND updated_at_ms = created_at_ms)
    OR (state = 'delivering' AND attempt BETWEEN 1 AND 12 AND lease_id IS NOT NULL
      AND lease_expires_at_ms > updated_at_ms AND provider_receipt_digest IS NULL
      AND last_error_class IS NULL)
    OR (state = 'applied' AND attempt BETWEEN 1 AND 12 AND lease_id IS NULL
      AND lease_expires_at_ms IS NULL AND provider_receipt_digest IS NOT NULL
      AND last_error_class IS NULL)
    OR (state = 'exhausted' AND attempt = 12 AND lease_id IS NULL
      AND lease_expires_at_ms IS NULL AND provider_receipt_digest IS NULL
      AND last_error_class IS NOT NULL)
  )
);
CREATE INDEX legacy_mobile_session_stripe_effects_ready_idx_v1
  ON legacy_mobile_session_stripe_effects_v1(state, created_at_ms, effect_id);
CREATE VIEW legacy_mobile_session_stripe_effects_ready_v1 AS
SELECT effect_id, operation_id, user_id, normalized_email_digest, attempt, created_at_ms
FROM legacy_mobile_session_stripe_effects_v1
WHERE state = 'pending';

CREATE TRIGGER legacy_mobile_session_stripe_effect_payload_immutable_v1
BEFORE UPDATE OF effect_id, operation_id, user_id, normalized_email_digest, created_at_ms
ON legacy_mobile_session_stripe_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_session_stripe_effect_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_session_stripe_effect_no_delete_v1
BEFORE DELETE ON legacy_mobile_session_stripe_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_session_stripe_effect_immutable_v1');
END;

CREATE TABLE legacy_mobile_session_audit_events_v1 (
  event_id TEXT PRIMARY KEY NOT NULL CHECK (length(event_id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_mobile_session_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN (
    'email_request', 'email_verify', 'session_request', 'session_revoke'
  )),
  subject_digest TEXT NOT NULL CHECK (length(subject_digest) = 64),
  outcome TEXT NOT NULL CHECK (outcome = 'allow'),
  occurred_at_ms INTEGER NOT NULL
    CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_mobile_session_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (length(assertion_kind) BETWEEN 1 AND 80),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 1000000),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 1000000),
  PRIMARY KEY (operation_id, assertion_kind)
);
CREATE TRIGGER legacy_mobile_session_assertion_guard_v1
BEFORE INSERT ON legacy_mobile_session_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_session_assertion_failed_v1');
END;
