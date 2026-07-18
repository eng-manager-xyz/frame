PRAGMA foreign_keys = ON;

-- Expand the scaffold user row without changing or deleting any existing field.
ALTER TABLE users ADD COLUMN status TEXT NOT NULL DEFAULT 'active'
  CHECK (status IN ('active', 'suspended', 'deleted'));
ALTER TABLE users ADD COLUMN session_version INTEGER NOT NULL DEFAULT 0
  CHECK (session_version >= 0 AND session_version <= 9007199254740991);
ALTER TABLE users ADD COLUMN email_verified_at_ms INTEGER
  CHECK (email_verified_at_ms IS NULL OR email_verified_at_ms BETWEEN 0 AND 9007199254740991);
ALTER TABLE users ADD COLUMN preferences_json TEXT
  CHECK (preferences_json IS NULL OR json_valid(preferences_json));
ALTER TABLE users ADD COLUMN deleted_at_ms INTEGER
  CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991);

CREATE TABLE identity_accounts (
  id TEXT PRIMARY KEY NOT NULL,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 64),
  provider_account_id TEXT NOT NULL CHECK (length(provider_account_id) BETWEEN 1 AND 255),
  access_token_ciphertext TEXT,
  refresh_token_ciphertext TEXT,
  token_expires_at_ms INTEGER
    CHECK (token_expires_at_ms IS NULL OR token_expires_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (provider, provider_account_id)
);
CREATE INDEX identity_accounts_user_idx ON identity_accounts(user_id);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY NOT NULL,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_digest TEXT NOT NULL UNIQUE CHECK (length(token_digest) = 64),
  session_version INTEGER NOT NULL CHECK (session_version >= 0 AND session_version <= 9007199254740991),
  client_kind TEXT NOT NULL CHECK (client_kind IN ('browser', 'desktop', 'mobile', 'extension', 'api')),
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  last_seen_at_ms INTEGER CHECK (last_seen_at_ms IS NULL OR last_seen_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  revoke_reason TEXT CHECK (revoke_reason IS NULL OR length(revoke_reason) <= 64),
  CHECK (expires_at_ms > issued_at_ms)
);
CREATE INDEX sessions_user_expiry_idx ON sessions(user_id, expires_at_ms);
CREATE INDEX sessions_active_expiry_idx ON sessions(expires_at_ms) WHERE revoked_at_ms IS NULL;

CREATE TABLE verification_tokens (
  id TEXT PRIMARY KEY NOT NULL,
  user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
  identifier_digest TEXT NOT NULL CHECK (length(identifier_digest) = 64),
  token_digest TEXT NOT NULL UNIQUE CHECK (length(token_digest) = 64),
  purpose TEXT NOT NULL CHECK (purpose IN ('email_verify', 'sign_in', 'account_recovery', 'account_link')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 100),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  consumed_at_ms INTEGER CHECK (consumed_at_ms IS NULL OR consumed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms)
);
CREATE INDEX verification_tokens_identifier_idx
  ON verification_tokens(identifier_digest, purpose, expires_at_ms);

CREATE TABLE auth_api_keys (
  id TEXT PRIMARY KEY NOT NULL,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key_digest TEXT NOT NULL UNIQUE CHECK (length(key_digest) = 64),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
  scopes_json TEXT NOT NULL CHECK (json_valid(scopes_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER CHECK (expires_at_ms IS NULL OR expires_at_ms BETWEEN 0 AND 9007199254740991),
  last_used_at_ms INTEGER CHECK (last_used_at_ms IS NULL OR last_used_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX auth_api_keys_user_created_idx ON auth_api_keys(user_id, created_at_ms DESC);

CREATE TABLE auth_audit_events (
  id TEXT PRIMARY KEY NOT NULL,
  user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  correlation_id TEXT NOT NULL,
  decision_code TEXT NOT NULL CHECK (length(decision_code) BETWEEN 1 AND 64),
  outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'error')),
  client_kind TEXT CHECK (client_kind IS NULL OR client_kind IN ('browser', 'desktop', 'mobile', 'extension', 'api')),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json))
);
CREATE INDEX auth_audit_events_user_time_idx ON auth_audit_events(user_id, occurred_at_ms DESC);
CREATE INDEX auth_audit_events_correlation_idx ON auth_audit_events(correlation_id);

CREATE TABLE auth_abuse_buckets (
  identifier_digest TEXT NOT NULL CHECK (length(identifier_digest) = 64),
  action TEXT NOT NULL CHECK (action IN ('sign_in', 'verify', 'recover', 'api_key')),
  window_started_at_ms INTEGER NOT NULL CHECK (window_started_at_ms BETWEEN 0 AND 9007199254740991),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  blocked_until_ms INTEGER CHECK (blocked_until_ms IS NULL OR blocked_until_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (identifier_digest, action)
);
CREATE INDEX auth_abuse_buckets_blocked_idx ON auth_abuse_buckets(blocked_until_ms)
  WHERE blocked_until_ms IS NOT NULL;
