PRAGMA foreign_keys = ON;

CREATE TABLE developer_apps (
  id TEXT PRIMARY KEY NOT NULL,
  owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 160),
  environment TEXT NOT NULL CHECK (environment IN ('test', 'live')),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'deleted')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991)
);
CREATE INDEX developer_apps_owner_status_idx ON developer_apps(owner_user_id, status);

CREATE TABLE developer_app_domains (
  app_id TEXT NOT NULL REFERENCES developer_apps(id) ON DELETE CASCADE,
  domain_ascii TEXT NOT NULL COLLATE NOCASE,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  verified_at_ms INTEGER CHECK (verified_at_ms IS NULL OR verified_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (app_id, domain_ascii)
);

CREATE TABLE developer_api_keys (
  id TEXT PRIMARY KEY NOT NULL,
  app_id TEXT NOT NULL REFERENCES developer_apps(id) ON DELETE CASCADE,
  key_digest TEXT NOT NULL UNIQUE CHECK (length(key_digest) = 64),
  key_type TEXT NOT NULL CHECK (key_type IN ('publishable', 'secret')),
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  last_used_at_ms INTEGER CHECK (last_used_at_ms IS NULL OR last_used_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX developer_api_keys_app_type_idx ON developer_api_keys(app_id, key_type);

CREATE TABLE developer_videos (
  id TEXT PRIMARY KEY NOT NULL,
  app_id TEXT NOT NULL REFERENCES developer_apps(id) ON DELETE CASCADE,
  video_id TEXT REFERENCES videos(id) ON DELETE SET NULL,
  external_user_id TEXT NOT NULL CHECK (length(external_user_id) BETWEEN 1 AND 255),
  metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX developer_videos_app_created_idx ON developer_videos(app_id, created_at_ms DESC);
CREATE INDEX developer_videos_external_user_idx ON developer_videos(app_id, external_user_id);

CREATE TABLE developer_credit_accounts (
  id TEXT PRIMARY KEY NOT NULL,
  app_id TEXT NOT NULL UNIQUE REFERENCES developer_apps(id) ON DELETE CASCADE,
  balance_microcredits INTEGER NOT NULL DEFAULT 0
    CHECK (balance_microcredits BETWEEN 0 AND 9007199254740991),
  auto_top_up_enabled INTEGER NOT NULL DEFAULT 0 CHECK (auto_top_up_enabled IN (0, 1)),
  auto_top_up_threshold_microcredits INTEGER
    CHECK (auto_top_up_threshold_microcredits IS NULL OR auto_top_up_threshold_microcredits BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991)
);

CREATE TABLE developer_credit_transactions (
  id TEXT PRIMARY KEY NOT NULL,
  account_id TEXT NOT NULL REFERENCES developer_credit_accounts(id) ON DELETE RESTRICT,
  transaction_type TEXT NOT NULL CHECK (transaction_type IN ('purchase', 'usage', 'refund', 'adjustment')),
  amount_microcredits INTEGER NOT NULL
    CHECK (amount_microcredits BETWEEN -9007199254740991 AND 9007199254740991),
  balance_after_microcredits INTEGER NOT NULL
    CHECK (balance_after_microcredits BETWEEN 0 AND 9007199254740991),
  reference_type TEXT NOT NULL CHECK (length(reference_type) BETWEEN 1 AND 64),
  reference_id TEXT NOT NULL CHECK (length(reference_id) BETWEEN 1 AND 255),
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (account_id, idempotency_key),
  UNIQUE (account_id, reference_type, reference_id, transaction_type)
);
CREATE INDEX developer_credit_transactions_account_time_idx
  ON developer_credit_transactions(account_id, created_at_ms DESC);

CREATE TABLE usage_ledger (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  app_id TEXT REFERENCES developer_apps(id) ON DELETE RESTRICT,
  video_id TEXT REFERENCES videos(id) ON DELETE SET NULL,
  media_job_id TEXT REFERENCES media_jobs(id) ON DELETE SET NULL,
  usage_type TEXT NOT NULL CHECK (usage_type IN ('storage_byte_day', 'upload_byte', 'download_byte', 'transform_unit', 'compute_millisecond')),
  quantity INTEGER NOT NULL CHECK (quantity BETWEEN 0 AND 9007199254740991),
  microcredits_charged INTEGER NOT NULL DEFAULT 0
    CHECK (microcredits_charged BETWEEN 0 AND 9007199254740991),
  idempotency_key TEXT NOT NULL UNIQUE,
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  recorded_at_ms INTEGER NOT NULL CHECK (recorded_at_ms BETWEEN 0 AND 9007199254740991),
  metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
  CHECK (organization_id IS NOT NULL OR app_id IS NOT NULL)
);
CREATE INDEX usage_ledger_org_time_idx ON usage_ledger(organization_id, occurred_at_ms);
CREATE INDEX usage_ledger_app_time_idx ON usage_ledger(app_id, occurred_at_ms);

CREATE TABLE developer_daily_storage_snapshots (
  app_id TEXT NOT NULL REFERENCES developer_apps(id) ON DELETE CASCADE,
  snapshot_day TEXT NOT NULL CHECK (length(snapshot_day) = 10),
  total_bytes INTEGER NOT NULL CHECK (total_bytes BETWEEN 0 AND 9007199254740991),
  microcredits_charged INTEGER NOT NULL DEFAULT 0
    CHECK (microcredits_charged BETWEEN 0 AND 9007199254740991),
  source_checksum TEXT NOT NULL CHECK (length(source_checksum) = 64),
  processed_at_ms INTEGER CHECK (processed_at_ms IS NULL OR processed_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (app_id, snapshot_day)
);

CREATE TRIGGER developer_credit_transactions_immutable_update
BEFORE UPDATE ON developer_credit_transactions
BEGIN
  SELECT RAISE(ABORT, 'credit transactions are append-only');
END;

CREATE TRIGGER developer_credit_transactions_immutable_delete
BEFORE DELETE ON developer_credit_transactions
BEGIN
  SELECT RAISE(ABORT, 'credit transactions are append-only');
END;

CREATE TRIGGER usage_ledger_immutable_update
BEFORE UPDATE ON usage_ledger
BEGIN
  SELECT RAISE(ABORT, 'usage ledger is append-only');
END;

CREATE TRIGGER usage_ledger_immutable_delete
BEFORE DELETE ON usage_ledger
BEGIN
  SELECT RAISE(ABORT, 'usage ledger is append-only');
END;
