PRAGMA foreign_keys = ON;

ALTER TABLE media_jobs ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE media_jobs ADD COLUMN selected_executor TEXT
  CHECK (selected_executor IS NULL OR selected_executor IN ('cloudflare_media', 'native_gstreamer'));
ALTER TABLE media_jobs ADD COLUMN source_version INTEGER CHECK (source_version IS NULL OR source_version > 0);
ALTER TABLE media_jobs ADD COLUMN profile_version INTEGER CHECK (profile_version IS NULL OR profile_version > 0);
ALTER TABLE media_jobs ADD COLUMN output_object_key TEXT;
ALTER TABLE media_jobs ADD COLUMN worker_id TEXT;
ALTER TABLE media_jobs ADD COLUMN lease_token_digest TEXT CHECK (lease_token_digest IS NULL OR length(lease_token_digest) = 64);
ALTER TABLE media_jobs ADD COLUMN heartbeat_at_ms INTEGER
  CHECK (heartbeat_at_ms IS NULL OR heartbeat_at_ms BETWEEN 0 AND 9007199254740991);
ALTER TABLE media_jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1));
ALTER TABLE media_jobs ADD COLUMN progress_basis_points INTEGER
  CHECK (progress_basis_points IS NULL OR progress_basis_points BETWEEN 0 AND 10000);
ALTER TABLE media_jobs ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision >= 0 AND revision <= 9007199254740991);
ALTER TABLE media_jobs ADD COLUMN error_class TEXT CHECK (error_class IS NULL OR length(error_class) <= 64);
ALTER TABLE media_jobs ADD COLUMN usage_units INTEGER
  CHECK (usage_units IS NULL OR usage_units BETWEEN 0 AND 9007199254740991);
ALTER TABLE media_jobs ADD COLUMN cost_microcredits INTEGER
  CHECK (cost_microcredits IS NULL OR cost_microcredits BETWEEN 0 AND 9007199254740991);
CREATE INDEX media_jobs_org_state_idx ON media_jobs(organization_id, state, created_at_ms);
CREATE INDEX media_jobs_lease_expiry_idx ON media_jobs(state, lease_expires_at_ms);

CREATE TABLE command_idempotency (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 128),
  command_type TEXT NOT NULL CHECK (length(command_type) BETWEEN 1 AND 64),
  request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
  response_status INTEGER,
  response_json TEXT CHECK (response_json IS NULL OR json_valid(response_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (organization_id, idempotency_key)
);
CREATE INDEX command_idempotency_expiry_idx ON command_idempotency(expires_at_ms);

CREATE TABLE media_job_attempts (
  job_id TEXT NOT NULL REFERENCES media_jobs(id) ON DELETE CASCADE,
  attempt INTEGER NOT NULL CHECK (attempt > 0),
  executor TEXT NOT NULL CHECK (executor IN ('cloudflare_media', 'native_gstreamer')),
  worker_id TEXT,
  started_at_ms INTEGER NOT NULL CHECK (started_at_ms BETWEEN 0 AND 9007199254740991),
  finished_at_ms INTEGER CHECK (finished_at_ms IS NULL OR finished_at_ms BETWEEN 0 AND 9007199254740991),
  outcome TEXT CHECK (outcome IS NULL OR outcome IN ('succeeded', 'retryable_failure', 'terminal_failure', 'cancelled', 'lost_lease')),
  error_class TEXT CHECK (error_class IS NULL OR length(error_class) <= 64),
  PRIMARY KEY (job_id, attempt)
);

CREATE TABLE media_job_dead_letters (
  job_id TEXT PRIMARY KEY NOT NULL REFERENCES media_jobs(id) ON DELETE CASCADE,
  attempt INTEGER NOT NULL CHECK (attempt > 0),
  error_class TEXT NOT NULL CHECK (length(error_class) BETWEEN 1 AND 64),
  diagnostic_code TEXT NOT NULL CHECK (length(diagnostic_code) BETWEEN 1 AND 128),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  resolved_at_ms INTEGER CHECK (resolved_at_ms IS NULL OR resolved_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE video_uploads (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK (state IN ('initiated', 'uploading', 'finalizing', 'complete', 'failed', 'aborted')),
  expected_bytes INTEGER NOT NULL CHECK (expected_bytes BETWEEN 0 AND 9007199254740991),
  received_bytes INTEGER NOT NULL DEFAULT 0 CHECK (received_bytes BETWEEN 0 AND 9007199254740991),
  idempotency_key TEXT NOT NULL,
  source_object_key TEXT NOT NULL,
  source_version INTEGER NOT NULL CHECK (source_version > 0),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  checksum_sha256 TEXT CHECK (checksum_sha256 IS NULL OR length(checksum_sha256) = 64),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991),
  CHECK (received_bytes <= expected_bytes),
  UNIQUE (organization_id, idempotency_key),
  UNIQUE (organization_id, source_object_key)
);
CREATE INDEX video_uploads_video_state_idx ON video_uploads(video_id, state);

CREATE TABLE multipart_uploads (
  id TEXT PRIMARY KEY NOT NULL,
  upload_id TEXT NOT NULL REFERENCES video_uploads(id) ON DELETE CASCADE,
  provider_upload_id_ciphertext TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('open', 'completing', 'complete', 'aborted', 'expired')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms)
);

CREATE TABLE multipart_upload_parts (
  multipart_upload_id TEXT NOT NULL REFERENCES multipart_uploads(id) ON DELETE CASCADE,
  part_number INTEGER NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  provider_etag TEXT NOT NULL CHECK (length(provider_etag) BETWEEN 1 AND 255),
  checksum_sha256 TEXT CHECK (checksum_sha256 IS NULL OR length(checksum_sha256) = 64),
  uploaded_at_ms INTEGER NOT NULL CHECK (uploaded_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (multipart_upload_id, part_number)
);

CREATE TABLE storage_integrations (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  owner_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  provider TEXT NOT NULL CHECK (provider IN ('r2', 's3_compatible', 'minio', 'google_drive')),
  state TEXT NOT NULL CHECK (state IN ('pending', 'active', 'disabled', 'revoked')),
  capabilities_json TEXT NOT NULL CHECK (json_valid(capabilities_json)),
  credential_ciphertext TEXT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0 AND revision <= 9007199254740991)
);
CREATE INDEX storage_integrations_org_provider_idx ON storage_integrations(organization_id, provider, state);

CREATE TABLE storage_objects (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  integration_id TEXT NOT NULL REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  video_id TEXT REFERENCES videos(id) ON DELETE CASCADE,
  object_key TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('source', 'segment', 'thumbnail', 'preview', 'spritesheet', 'audio', 'export', 'manifest')),
  object_version INTEGER NOT NULL CHECK (object_version > 0),
  state TEXT NOT NULL CHECK (state IN ('pending', 'available', 'quarantined', 'deleting', 'deleted', 'missing')),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 0 AND 9007199254740991),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  checksum_sha256 TEXT CHECK (checksum_sha256 IS NULL OR length(checksum_sha256) = 64),
  provider_etag TEXT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (integration_id, object_key),
  UNIQUE (organization_id, video_id, role, object_version, object_key)
);
CREATE INDEX storage_objects_video_role_idx ON storage_objects(video_id, role, object_version);
CREATE INDEX storage_objects_org_state_idx ON storage_objects(organization_id, state, created_at_ms);

ALTER TABLE object_manifests ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE object_manifests ADD COLUMN object_version INTEGER NOT NULL DEFAULT 1 CHECK (object_version > 0);
ALTER TABLE object_manifests ADD COLUMN provider_etag TEXT;
ALTER TABLE object_manifests ADD COLUMN state TEXT NOT NULL DEFAULT 'available'
  CHECK (state IN ('pending', 'available', 'quarantined', 'deleting', 'deleted', 'missing'));
ALTER TABLE object_manifests ADD COLUMN updated_at_ms INTEGER
  CHECK (updated_at_ms IS NULL OR updated_at_ms BETWEEN 0 AND 9007199254740991);
CREATE INDEX object_manifests_org_state_idx ON object_manifests(organization_id, state, created_at_ms);

CREATE TABLE imported_videos (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  video_id TEXT REFERENCES videos(id) ON DELETE SET NULL,
  provider TEXT NOT NULL CHECK (provider IN ('loom', 'file', 'google_drive', 'other')),
  external_id_digest TEXT CHECK (external_id_digest IS NULL OR length(external_id_digest) = 64),
  state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'complete', 'failed', 'cancelled')),
  idempotency_key TEXT NOT NULL,
  error_class TEXT CHECK (error_class IS NULL OR length(error_class) <= 64),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (organization_id, idempotency_key)
);

CREATE TABLE outbox_events (
  id TEXT PRIMARY KEY NOT NULL,
  organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE,
  aggregate_type TEXT NOT NULL CHECK (length(aggregate_type) BETWEEN 1 AND 64),
  aggregate_id TEXT NOT NULL,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 96),
  deduplication_key TEXT NOT NULL UNIQUE,
  payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'leased', 'delivered', 'dead_letter')),
  attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
  available_at_ms INTEGER NOT NULL CHECK (available_at_ms BETWEEN 0 AND 9007199254740991),
  lease_expires_at_ms INTEGER CHECK (lease_expires_at_ms IS NULL OR lease_expires_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  delivered_at_ms INTEGER CHECK (delivered_at_ms IS NULL OR delivered_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX outbox_events_delivery_idx ON outbox_events(state, available_at_ms);

CREATE TABLE object_retention_policies (
  organization_id TEXT PRIMARY KEY NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  source_retention_days INTEGER CHECK (source_retention_days IS NULL OR source_retention_days BETWEEN 0 AND 36500),
  derivative_retention_days INTEGER CHECK (derivative_retention_days IS NULL OR derivative_retention_days BETWEEN 0 AND 36500),
  deleted_grace_days INTEGER NOT NULL DEFAULT 30 CHECK (deleted_grace_days BETWEEN 0 AND 3650),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE object_legal_holds (
  id TEXT PRIMARY KEY NOT NULL,
  storage_object_id TEXT NOT NULL REFERENCES storage_objects(id) ON DELETE CASCADE,
  reason_code TEXT NOT NULL CHECK (length(reason_code) BETWEEN 1 AND 64),
  placed_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  placed_at_ms INTEGER NOT NULL CHECK (placed_at_ms BETWEEN 0 AND 9007199254740991),
  released_at_ms INTEGER CHECK (released_at_ms IS NULL OR released_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX object_legal_holds_active_idx ON object_legal_holds(storage_object_id) WHERE released_at_ms IS NULL;

CREATE TABLE object_deletion_jobs (
  id TEXT PRIMARY KEY NOT NULL,
  storage_object_id TEXT NOT NULL REFERENCES storage_objects(id) ON DELETE CASCADE,
  idempotency_key TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('scheduled', 'blocked_by_hold', 'deleting', 'deleted', 'failed')),
  not_before_ms INTEGER NOT NULL CHECK (not_before_ms BETWEEN 0 AND 9007199254740991),
  attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
  error_class TEXT CHECK (error_class IS NULL OR length(error_class) <= 64),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX object_deletion_jobs_ready_idx ON object_deletion_jobs(state, not_before_ms);
