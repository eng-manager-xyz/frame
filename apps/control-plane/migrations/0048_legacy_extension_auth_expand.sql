PRAGMA foreign_keys = ON;

-- Cap records the credential's client source. Frame keeps credentials
-- digest-only, but retains this non-secret discriminator so the exact
-- extension lifecycle can be audited without recovering the returned UUID.
ALTER TABLE auth_api_keys ADD COLUMN legacy_source TEXT NOT NULL DEFAULT 'unknown'
  CHECK (legacy_source IN ('unknown', 'extension', 'mobile', 'desktop', 'developer'));
CREATE INDEX auth_api_keys_legacy_source_user_created_v1
  ON auth_api_keys(legacy_source, user_id, created_at_ms DESC);

-- D1 batch statements use this transient assertion relation to turn a failed
-- postcondition into a transaction abort. Successful batches delete their
-- assertion before commit, so no request-controlled value is retained here.
CREATE TABLE legacy_extension_auth_assertions_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (
    assertion_kind IN ('mint_within_hourly_limit', 'bootstrap_selection_repaired')
  ),
  accepted INTEGER NOT NULL CHECK (accepted IN (0, 1))
);

CREATE TRIGGER legacy_extension_auth_assertion_guard_v1
BEFORE INSERT ON legacy_extension_auth_assertions_v1
WHEN NEW.accepted <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_auth_assertion_failed_v1');
END;
