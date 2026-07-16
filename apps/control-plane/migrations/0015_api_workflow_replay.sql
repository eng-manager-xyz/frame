-- Durable, privacy-safe replay claims for signed inbound callbacks.
-- The digest already binds provider, signed timestamp, and signature; raw
-- headers, bodies, event IDs, tenant IDs, and secrets are never stored here.
CREATE TABLE api_webhook_replay_claims_v1 (
  replay_digest TEXT PRIMARY KEY CHECK (
    length(replay_digest) = 64
      AND lower(replay_digest) = replay_digest
      AND replay_digest NOT GLOB '*[^0-9a-f]*'
  ),
  claimed_at_ms INTEGER NOT NULL CHECK (
    claimed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  expires_at_ms INTEGER NOT NULL CHECK (
    expires_at_ms BETWEEN 1 AND 9007199254740991
      AND expires_at_ms > claimed_at_ms
      AND expires_at_ms - claimed_at_ms <= 1800000
  )
) WITHOUT ROWID;

CREATE INDEX api_webhook_replay_claims_v1_expiry_idx
  ON api_webhook_replay_claims_v1(expires_at_ms, replay_digest);

CREATE TRIGGER api_webhook_replay_claims_v1_no_update
BEFORE UPDATE ON api_webhook_replay_claims_v1
BEGIN
  SELECT RAISE(ABORT, 'api_webhook_replay_claim_immutable');
END;
