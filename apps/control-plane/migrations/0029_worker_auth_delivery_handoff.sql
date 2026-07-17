-- Provider-neutral, ciphertext-only handoff for Worker authentication delivery.
--
-- `auth_deliveries_v2` remains the authoritative transactional outbox. A
-- fenced dispatcher copies one claimed envelope here under the same delivery
-- ID before acknowledging the source claim. INSERT OR IGNORE plus the
-- immutable payload digest makes a crash between those operations replayable
-- without duplicating a provider handoff. Real provider execution remains a
-- protected deployment gate and may consume only rows in the ready view.

CREATE TABLE auth_delivery_provider_handoffs_v1 (
  delivery_id TEXT PRIMARY KEY
    CHECK (
      length(delivery_id) = 36
      AND lower(delivery_id) = delivery_id
      AND substr(delivery_id, 9, 1) = '-'
      AND substr(delivery_id, 14, 1) = '-'
      AND substr(delivery_id, 19, 1) = '-'
      AND substr(delivery_id, 24, 1) = '-'
    ),
  payload_hex TEXT NOT NULL
    CHECK (
      length(payload_hex) = 2142
      AND lower(payload_hex) = payload_hex
      AND payload_hex NOT GLOB '*[^0-9a-f]*'
    ),
  payload_sha256 TEXT NOT NULL
    CHECK (
      length(payload_sha256) = 64
      AND lower(payload_sha256) = payload_sha256
      AND payload_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
  state TEXT NOT NULL CHECK (state IN ('pending', 'delivering', 'delivered', 'exhausted')),
  provider_attempt INTEGER NOT NULL DEFAULT 0
    CHECK (provider_attempt BETWEEN 0 AND 12),
  provider_lease_id TEXT CHECK (
    provider_lease_id IS NULL
    OR (
      length(provider_lease_id) = 36
      AND lower(provider_lease_id) = provider_lease_id
      AND substr(provider_lease_id, 9, 1) = '-'
      AND substr(provider_lease_id, 14, 1) = '-'
      AND substr(provider_lease_id, 19, 1) = '-'
      AND substr(provider_lease_id, 24, 1) = '-'
    )
  ),
  provider_lease_expires_at_ms INTEGER CHECK (
    provider_lease_expires_at_ms IS NULL
    OR provider_lease_expires_at_ms BETWEEN 0 AND 253402300799999
  ),
  next_attempt_at_ms INTEGER NOT NULL DEFAULT 0
    CHECK (next_attempt_at_ms BETWEEN 0 AND 253402300799999),
  provider_receipt_digest TEXT,
  last_error_class TEXT,
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 253402300799999),
  updated_at_ms INTEGER NOT NULL
    CHECK (updated_at_ms BETWEEN created_at_ms AND 253402300799999),
  CHECK (
    (state = 'pending'
      AND provider_attempt BETWEEN 0 AND 11
      AND provider_lease_id IS NULL
      AND provider_lease_expires_at_ms IS NULL
      AND provider_receipt_digest IS NULL
      AND (
        (provider_attempt = 0 AND last_error_class IS NULL AND next_attempt_at_ms = 0)
        OR (provider_attempt > 0 AND last_error_class IS NOT NULL AND next_attempt_at_ms > updated_at_ms)
      ))
    OR (state = 'delivering'
      AND provider_attempt BETWEEN 1 AND 12
      AND provider_lease_id IS NOT NULL
      AND provider_lease_expires_at_ms > updated_at_ms
      AND provider_receipt_digest IS NULL
      AND last_error_class IS NULL)
    OR (state = 'delivered'
      AND provider_attempt BETWEEN 1 AND 12
      AND provider_lease_id IS NULL
      AND provider_lease_expires_at_ms IS NULL
      AND provider_receipt_digest IS NOT NULL
      AND last_error_class IS NULL)
    OR (state = 'exhausted'
      AND provider_attempt = 12
      AND provider_lease_id IS NULL
      AND provider_lease_expires_at_ms IS NULL
      AND provider_receipt_digest IS NULL
      AND last_error_class IS NOT NULL)
  ),
  CHECK (
    provider_receipt_digest IS NULL
    OR (
      length(provider_receipt_digest) = 64
      AND lower(provider_receipt_digest) = provider_receipt_digest
      AND provider_receipt_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  CHECK (
    last_error_class IS NULL
    OR last_error_class IN ('provider_unavailable', 'provider_rejected', 'invalid_ciphertext', 'expired')
  )
);

CREATE INDEX auth_delivery_provider_handoffs_v1_ready_idx
  ON auth_delivery_provider_handoffs_v1(state, next_attempt_at_ms, created_at_ms, delivery_id);

CREATE VIEW auth_delivery_provider_handoffs_ready_v1 AS
SELECT delivery_id, payload_hex, payload_sha256, provider_attempt, next_attempt_at_ms, created_at_ms
FROM auth_delivery_provider_handoffs_v1
WHERE state = 'pending';

CREATE TRIGGER auth_delivery_provider_handoffs_v1_insert_guard
BEFORE INSERT ON auth_delivery_provider_handoffs_v1
BEGIN
  SELECT CASE
    WHEN NEW.state <> 'pending'
      OR NEW.provider_attempt <> 0
      OR NEW.provider_lease_id IS NOT NULL
      OR NEW.provider_lease_expires_at_ms IS NOT NULL
      OR NEW.next_attempt_at_ms <> 0
      OR NEW.provider_receipt_digest IS NOT NULL
      OR NEW.last_error_class IS NOT NULL
      OR NEW.updated_at_ms <> NEW.created_at_ms
      THEN RAISE(ABORT, 'authentication delivery handoff initial state is invalid')
  END;
END;

CREATE TRIGGER auth_delivery_provider_handoffs_v1_payload_immutable
BEFORE UPDATE OF delivery_id, payload_hex, payload_sha256, created_at_ms
ON auth_delivery_provider_handoffs_v1
BEGIN
  SELECT RAISE(ABORT, 'authentication delivery handoff payload is immutable');
END;

CREATE TRIGGER auth_delivery_provider_handoffs_v1_transition_guard
BEFORE UPDATE OF state, provider_attempt, provider_lease_id, provider_lease_expires_at_ms,
  next_attempt_at_ms, provider_receipt_digest, last_error_class, updated_at_ms
ON auth_delivery_provider_handoffs_v1
BEGIN
  SELECT CASE
    WHEN NEW.updated_at_ms < OLD.updated_at_ms
      THEN RAISE(ABORT, 'authentication delivery handoff time regressed')
    WHEN OLD.state IN ('delivered', 'exhausted')
      THEN RAISE(ABORT, 'authentication delivery handoff is terminal')
    WHEN OLD.state = 'pending' AND (
      NEW.state <> 'delivering'
      OR NEW.provider_attempt <> OLD.provider_attempt + 1
      OR OLD.next_attempt_at_ms > NEW.updated_at_ms
      OR NEW.provider_lease_id IS NULL
      OR NEW.provider_lease_expires_at_ms <= NEW.updated_at_ms
      OR NEW.provider_receipt_digest IS NOT NULL
      OR NEW.last_error_class IS NOT NULL
    ) THEN RAISE(ABORT, 'authentication delivery handoff claim is invalid')
    WHEN OLD.state = 'delivering' AND NEW.state = 'delivering' AND (
      NEW.updated_at_ms < OLD.provider_lease_expires_at_ms
      OR NEW.provider_attempt <> OLD.provider_attempt + 1
      OR NEW.provider_lease_id IS NULL
      OR NEW.provider_lease_id = OLD.provider_lease_id
      OR NEW.provider_lease_expires_at_ms <= NEW.updated_at_ms
      OR NEW.provider_receipt_digest IS NOT NULL
      OR NEW.last_error_class IS NOT NULL
    ) THEN RAISE(ABORT, 'authentication delivery handoff takeover is invalid')
    WHEN OLD.state = 'delivering' AND NEW.state = 'pending' AND (
      NEW.updated_at_ms < OLD.provider_lease_expires_at_ms
      OR OLD.provider_attempt >= 12
      OR NEW.provider_attempt <> OLD.provider_attempt
      OR NEW.provider_lease_id IS NOT NULL
      OR NEW.provider_lease_expires_at_ms IS NOT NULL
      OR NEW.next_attempt_at_ms <= NEW.updated_at_ms
      OR NEW.provider_receipt_digest IS NOT NULL
      OR NEW.last_error_class IS NULL
    ) THEN RAISE(ABORT, 'authentication delivery handoff retry is invalid')
    WHEN OLD.state = 'delivering' AND NEW.state = 'delivered' AND (
      NEW.updated_at_ms >= OLD.provider_lease_expires_at_ms
      OR NEW.provider_attempt <> OLD.provider_attempt
      OR NEW.provider_lease_id IS NOT NULL
      OR NEW.provider_lease_expires_at_ms IS NOT NULL
      OR NEW.provider_receipt_digest IS NULL
      OR NEW.last_error_class IS NOT NULL
    ) THEN RAISE(ABORT, 'authentication delivery handoff completion is invalid')
    WHEN OLD.state = 'delivering' AND NEW.state = 'exhausted' AND (
      NEW.updated_at_ms < OLD.provider_lease_expires_at_ms
      OR OLD.provider_attempt <> 12
      OR NEW.provider_attempt <> OLD.provider_attempt
      OR NEW.provider_lease_id IS NOT NULL
      OR NEW.provider_lease_expires_at_ms IS NOT NULL
      OR NEW.provider_receipt_digest IS NOT NULL
      OR NEW.last_error_class IS NULL
    ) THEN RAISE(ABORT, 'authentication delivery handoff exhaustion is invalid')
    WHEN OLD.state = 'delivering' AND NEW.state NOT IN ('pending', 'delivering', 'delivered', 'exhausted')
      THEN RAISE(ABORT, 'authentication delivery handoff transition is invalid')
  END;
END;
