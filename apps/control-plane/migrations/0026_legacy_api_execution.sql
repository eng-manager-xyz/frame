PRAGMA foreign_keys = ON;

-- Central idempotency authority for evidence-enabled legacy adapters. Raw
-- tenant IDs, idempotency keys, request bodies, response bodies, and provider
-- details are never stored here; every sensitive scope/key is a bound digest.
CREATE TABLE legacy_api_execution_operations_v1 (
  scope_digest TEXT NOT NULL CHECK (
    length(scope_digest) = 64 AND scope_digest NOT GLOB '*[^0-9a-f]*'
  ),
  operation_id TEXT NOT NULL CHECK (
    length(operation_id) = 23
      AND substr(operation_id, 1, 7) = 'cap-v1-'
      AND substr(operation_id, 8) NOT GLOB '*[^0-9a-f]*'
  ),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
      AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_fingerprint TEXT NOT NULL CHECK (
    length(request_fingerprint) = 64
      AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  reservation_digest TEXT NOT NULL UNIQUE CHECK (
    length(reservation_digest) = 64
      AND reservation_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'complete')),
  response_status INTEGER CHECK (response_status IS NULL OR response_status BETWEEN 200 AND 299),
  result_digest TEXT CHECK (
    result_digest IS NULL OR (
      length(result_digest) = 64 AND result_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  PRIMARY KEY (scope_digest, operation_id, idempotency_key_digest),
  CHECK (
    (state = 'pending' AND response_status IS NULL AND result_digest IS NULL AND completed_at_ms IS NULL)
    OR
    (state = 'complete' AND response_status IS NOT NULL AND result_digest IS NOT NULL AND completed_at_ms IS NOT NULL)
  )
) WITHOUT ROWID;

CREATE INDEX legacy_api_execution_operations_v1_time_idx
  ON legacy_api_execution_operations_v1(created_at_ms, operation_id);

-- A digest-only handoff proves that the semantic adapter registered a durable
-- intent in the same batch. No report row is enabled until its concrete
-- consumer and response contract exist.
CREATE TABLE legacy_api_execution_intents_v1 (
  reservation_digest TEXT PRIMARY KEY NOT NULL CHECK (
    length(reservation_digest) = 64
      AND reservation_digest NOT GLOB '*[^0-9a-f]*'
  ),
  scope_digest TEXT NOT NULL,
  operation_id TEXT NOT NULL,
  idempotency_key_digest TEXT NOT NULL,
  request_fingerprint TEXT NOT NULL CHECK (
    length(request_fingerprint) = 64
      AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  FOREIGN KEY (scope_digest, operation_id, idempotency_key_digest)
    REFERENCES legacy_api_execution_operations_v1(
      scope_digest, operation_id, idempotency_key_digest
    ) ON DELETE RESTRICT
) WITHOUT ROWID;

-- Append-only, privacy-safe result audit. Audit insertion is guarded by the
-- winning reservation and a complete operation with its matching intent.
CREATE TABLE legacy_api_execution_audit_v1 (
  audit_id TEXT PRIMARY KEY NOT NULL CHECK (
    length(audit_id) = 64 AND audit_id NOT GLOB '*[^0-9a-f]*'
  ),
  reservation_digest TEXT NOT NULL UNIQUE
    REFERENCES legacy_api_execution_intents_v1(reservation_digest) ON DELETE RESTRICT,
  scope_digest TEXT NOT NULL CHECK (
    length(scope_digest) = 64 AND scope_digest NOT GLOB '*[^0-9a-f]*'
  ),
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 23),
  audit_action TEXT NOT NULL CHECK (length(audit_action) BETWEEN 1 AND 96),
  outcome TEXT NOT NULL CHECK (outcome = 'accepted'),
  correlation_digest TEXT NOT NULL CHECK (
    length(correlation_digest) = 64
      AND correlation_digest NOT GLOB '*[^0-9a-f]*'
  ),
  result_digest TEXT NOT NULL CHECK (
    length(result_digest) = 64 AND result_digest NOT GLOB '*[^0-9a-f]*'
  ),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
) WITHOUT ROWID;

CREATE TRIGGER legacy_api_execution_operations_v1_transition_guard
BEFORE UPDATE ON legacy_api_execution_operations_v1
WHEN OLD.scope_digest != NEW.scope_digest
  OR OLD.operation_id != NEW.operation_id
  OR OLD.idempotency_key_digest != NEW.idempotency_key_digest
  OR OLD.request_fingerprint != NEW.request_fingerprint
  OR OLD.reservation_digest != NEW.reservation_digest
  OR OLD.created_at_ms != NEW.created_at_ms
  OR OLD.state != 'pending'
  OR NEW.state != 'complete'
BEGIN
  SELECT RAISE(ABORT, 'legacy_api_execution_invalid_transition');
END;

CREATE TRIGGER legacy_api_execution_operations_v1_no_delete
BEFORE DELETE ON legacy_api_execution_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'legacy_api_execution_immutable');
END;

CREATE TRIGGER legacy_api_execution_intents_v1_no_update
BEFORE UPDATE ON legacy_api_execution_intents_v1
BEGIN
  SELECT RAISE(ABORT, 'legacy_api_execution_intent_immutable');
END;

CREATE TRIGGER legacy_api_execution_intents_v1_no_delete
BEFORE DELETE ON legacy_api_execution_intents_v1
BEGIN
  SELECT RAISE(ABORT, 'legacy_api_execution_intent_immutable');
END;

CREATE TRIGGER legacy_api_execution_audit_v1_no_update
BEFORE UPDATE ON legacy_api_execution_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'legacy_api_execution_audit_immutable');
END;

CREATE TRIGGER legacy_api_execution_audit_v1_no_delete
BEFORE DELETE ON legacy_api_execution_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'legacy_api_execution_audit_immutable');
END;
