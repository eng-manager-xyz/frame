PRAGMA foreign_keys = ON;

-- Provider codes and raw OAuth state never enter D1. Only keyed digests and
-- one-time repository reservations cross this boundary.
CREATE TABLE auth_oauth_flows_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  provider TEXT NOT NULL CHECK (provider IN ('google', 'github')),
  purpose TEXT NOT NULL CHECK (purpose IN ('sign_in', 'account_link')),
  initiator_session_id TEXT REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  initiator_user_id TEXT REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  initiator_generation INTEGER CHECK (initiator_generation IS NULL OR initiator_generation BETWEEN 0 AND 9007199254740991),
  state_key_version INTEGER NOT NULL CHECK (state_key_version BETWEEN 1 AND 65535),
  state_digest TEXT NOT NULL CHECK (length(state_digest) = 64 AND state_digest NOT GLOB '*[^0-9a-f]*'),
  pkce_key_version INTEGER NOT NULL CHECK (pkce_key_version BETWEEN 1 AND 65535),
  pkce_digest TEXT NOT NULL CHECK (length(pkce_digest) = 64 AND pkce_digest NOT GLOB '*[^0-9a-f]*'),
  redirect_key_version INTEGER NOT NULL CHECK (redirect_key_version BETWEEN 1 AND 65535),
  redirect_digest TEXT NOT NULL CHECK (length(redirect_digest) = 64 AND redirect_digest NOT GLOB '*[^0-9a-f]*'),
  audience_key_version INTEGER NOT NULL CHECK (audience_key_version BETWEEN 1 AND 65535),
  audience_digest TEXT NOT NULL CHECK (length(audience_digest) = 64 AND audience_digest NOT GLOB '*[^0-9a-f]*'),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  consumed_at_ms INTEGER CHECK (consumed_at_ms IS NULL OR consumed_at_ms BETWEEN 0 AND 9007199254740991),
  revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (expires_at_ms > created_at_ms),
  CHECK ((initiator_session_id IS NULL) = (initiator_user_id IS NULL)),
  CHECK ((initiator_session_id IS NULL) = (initiator_generation IS NULL)),
  CHECK ((purpose = 'account_link') = (initiator_session_id IS NOT NULL))
);
CREATE UNIQUE INDEX auth_oauth_flows_v2_state_idx
  ON auth_oauth_flows_v2(state_key_version, state_digest);
CREATE INDEX auth_oauth_flows_v2_expiry_idx
  ON auth_oauth_flows_v2(expires_at_ms, id);

-- A fresh optimistic plan must be rebuilt when another request claims the
-- same state digest or the bounded live-flow inventory concurrently. The
-- adapter recognizes only this repository-owned trigger marker as a retryable
-- CAS conflict; arbitrary provider/constraint text still fails closed.
CREATE TRIGGER auth_oauth_flows_v2_insert_fence
BEFORE INSERT ON auth_oauth_flows_v2
WHEN EXISTS (
  SELECT 1 FROM auth_oauth_flows_v2 existing
  WHERE existing.state_key_version = NEW.state_key_version
    AND existing.state_digest = NEW.state_digest
) OR (
  SELECT COUNT(*) FROM auth_oauth_flows_v2 existing
  WHERE existing.expires_at_ms > NEW.created_at_ms
) >= 4096
BEGIN
  SELECT RAISE(ABORT, 'frame_auth_cas_conflict_v1');
END;

CREATE TABLE auth_oauth_reservations_v2 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  flow_id TEXT NOT NULL REFERENCES auth_oauth_flows_v2(id) ON DELETE CASCADE,
  provider TEXT NOT NULL CHECK (provider IN ('google', 'github')),
  initiator_session_id TEXT REFERENCES auth_sessions_v2(id) ON DELETE CASCADE,
  initiator_user_id TEXT REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  initiator_generation INTEGER CHECK (initiator_generation IS NULL OR initiator_generation BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  consumed_at_ms INTEGER CHECK (consumed_at_ms IS NULL OR consumed_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK ((initiator_session_id IS NULL) = (initiator_user_id IS NULL)),
  CHECK ((initiator_session_id IS NULL) = (initiator_generation IS NULL)),
  CHECK (expires_at_ms > created_at_ms),
  CHECK (consumed_at_ms IS NULL OR (
    consumed_at_ms >= created_at_ms AND consumed_at_ms < expires_at_ms
  ))
);
CREATE UNIQUE INDEX auth_oauth_reservations_v2_flow_idx
  ON auth_oauth_reservations_v2(flow_id);
CREATE INDEX auth_oauth_reservations_v2_expiry_idx
  ON auth_oauth_reservations_v2(expires_at_ms, id);

-- OAuth operations use a separate expand-only receipt table because the
-- released auth_repository_operations_v2 CHECK constraint cannot be widened
-- without rebuilding that table. These rows provide the same ambiguous-commit
-- recovery without weakening the existing operation-kind allowlist.
CREATE TABLE auth_oauth_operations_v2 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'oauth_begin', 'oauth_preflight', 'oauth_finalize'
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
CREATE INDEX auth_oauth_operations_v2_subject_idx
  ON auth_oauth_operations_v2(operation_kind, subject_id);

CREATE TABLE auth_external_accounts_v2 (
  provider TEXT NOT NULL CHECK (provider IN ('google', 'github')),
  subject_key_version INTEGER NOT NULL CHECK (subject_key_version BETWEEN 1 AND 65535),
  subject_digest TEXT NOT NULL CHECK (length(subject_digest) = 64 AND subject_digest NOT GLOB '*[^0-9a-f]*'),
  user_id TEXT NOT NULL REFERENCES auth_identities_v2(user_id) ON DELETE CASCADE,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  PRIMARY KEY (provider, subject_key_version, subject_digest)
);
CREATE INDEX auth_external_accounts_v2_user_idx
  ON auth_external_accounts_v2(user_id, provider);

-- Direct uploads retain a canonical logical key and a separate random R2
-- staging key. The staging capability can never overwrite the canonical key.
ALTER TABLE video_uploads ADD COLUMN transfer_mode TEXT NOT NULL DEFAULT 'brokered'
  CHECK (transfer_mode IN ('brokered', 'direct'));
ALTER TABLE video_uploads ADD COLUMN direct_staging_key TEXT
  CHECK (direct_staging_key IS NULL OR length(direct_staging_key) BETWEEN 64 AND 1024);
ALTER TABLE video_uploads ADD COLUMN direct_checksum_sha256 TEXT
  CHECK (direct_checksum_sha256 IS NULL OR (
    length(direct_checksum_sha256) = 64 AND direct_checksum_sha256 NOT GLOB '*[^0-9a-f]*'
  ));
ALTER TABLE video_uploads ADD COLUMN direct_expires_at_ms INTEGER
  CHECK (direct_expires_at_ms IS NULL OR direct_expires_at_ms BETWEEN 0 AND 9007199254740991);
CREATE UNIQUE INDEX video_uploads_direct_staging_key_v1
  ON video_uploads(direct_staging_key)
  WHERE direct_staging_key IS NOT NULL;
CREATE INDEX video_uploads_direct_expiry_v1
  ON video_uploads(direct_expires_at_ms, id)
  WHERE transfer_mode = 'direct' AND direct_staging_key IS NOT NULL;

CREATE TABLE direct_upload_staging_cleanup_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL REFERENCES video_uploads(id) ON DELETE CASCADE,
  cleaned_at_ms INTEGER NOT NULL CHECK (cleaned_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER video_uploads_direct_contract_v1
BEFORE INSERT ON video_uploads
WHEN (
  NEW.transfer_mode = 'brokered'
  AND (NEW.direct_staging_key IS NOT NULL OR NEW.direct_checksum_sha256 IS NOT NULL OR NEW.direct_expires_at_ms IS NOT NULL)
) OR (
  NEW.transfer_mode = 'direct'
  AND (
    NEW.direct_staging_key IS NULL
    OR NEW.direct_checksum_sha256 IS NULL
    OR NEW.direct_expires_at_ms IS NULL
    OR NEW.direct_expires_at_ms <= NEW.created_at_ms
    OR substr(NEW.direct_staging_key, 1, 8) <> 'uploads/'
    OR substr(NEW.direct_staging_key, 73, 9) <> '/staging/'
    OR length(substr(NEW.direct_staging_key, 9, 64)) <> 64
    OR substr(NEW.direct_staging_key, 9, 64) GLOB '*[^0-9a-f]*'
    OR substr(NEW.direct_staging_key, 82, 36) = '00000000-0000-0000-0000-000000000000'
    OR length(substr(NEW.direct_staging_key, 82, 36)) <> 36
    OR substr(NEW.direct_staging_key, 90, 1) <> '-'
    OR substr(NEW.direct_staging_key, 95, 1) <> '-'
    OR substr(NEW.direct_staging_key, 100, 1) <> '-'
    OR substr(NEW.direct_staging_key, 105, 1) <> '-'
    OR length(replace(substr(NEW.direct_staging_key, 82, 36), '-', '')) <> 32
    OR replace(substr(NEW.direct_staging_key, 82, 36), '-', '') GLOB '*[^0-9a-f]*'
    OR substr(NEW.direct_staging_key, 118, 1) <> '.'
    OR instr(substr(NEW.direct_staging_key, 82), '/') <> 0
    OR NOT (
      (NEW.content_type = 'video/mp4' AND substr(NEW.direct_staging_key, -4) = '.mp4')
      OR (NEW.content_type = 'video/webm' AND substr(NEW.direct_staging_key, -5) = '.webm')
      OR (NEW.content_type = 'video/quicktime' AND substr(NEW.direct_staging_key, -4) = '.mov')
      OR (NEW.content_type = 'video/x-matroska' AND substr(NEW.direct_staging_key, -4) = '.mkv')
    )
    OR instr(NEW.direct_staging_key, '..') <> 0
    OR instr(NEW.direct_staging_key, char(92)) <> 0
    OR instr(NEW.direct_staging_key, '?') <> 0
    OR instr(NEW.direct_staging_key, '#') <> 0
    OR instr(NEW.direct_staging_key, '%') <> 0
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_direct_upload_contract_v1');
END;

CREATE TRIGGER video_uploads_direct_immutable_v1
BEFORE UPDATE OF transfer_mode, direct_staging_key, direct_checksum_sha256, direct_expires_at_ms ON video_uploads
WHEN NEW.transfer_mode <> OLD.transfer_mode
  OR NEW.direct_staging_key IS NOT OLD.direct_staging_key
  OR NEW.direct_checksum_sha256 IS NOT OLD.direct_checksum_sha256
  OR NEW.direct_expires_at_ms IS NOT OLD.direct_expires_at_ms
BEGIN
  SELECT RAISE(ABORT, 'frame_direct_upload_contract_v1');
END;

-- R2 multipart handles and receipts are server-side only. Completion stores
-- the verified full-object checksum and trusted probe so retries are stable.
CREATE TABLE r2_multipart_sessions_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL REFERENCES video_uploads(id) ON DELETE CASCADE,
  object_key TEXT NOT NULL CHECK (length(object_key) BETWEEN 16 AND 1024),
  provider_upload_id TEXT NOT NULL CHECK (length(provider_upload_id) BETWEEN 1 AND 1024),
  state TEXT NOT NULL CHECK (state IN ('open', 'completing', 'complete', 'aborted', 'expired')),
  expected_bytes INTEGER NOT NULL CHECK (expected_bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64 AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (expires_at_ms > created_at_ms),
  CHECK ((state = 'complete') = (completed_at_ms IS NOT NULL))
);
CREATE INDEX r2_multipart_sessions_v1_expiry_idx
  ON r2_multipart_sessions_v1(state, expires_at_ms, upload_id);

CREATE TABLE r2_multipart_parts_v1 (
  upload_id TEXT NOT NULL REFERENCES r2_multipart_sessions_v1(upload_id) ON DELETE CASCADE,
  part_number INTEGER NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64 AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'),
  provider_etag TEXT NOT NULL CHECK (length(provider_etag) BETWEEN 1 AND 256),
  uploaded_at_ms INTEGER NOT NULL CHECK (uploaded_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (upload_id, part_number)
);

CREATE TABLE r2_multipart_completions_v1 (
  upload_id TEXT PRIMARY KEY NOT NULL REFERENCES video_uploads(id) ON DELETE CASCADE,
  request_parts_sha256 TEXT NOT NULL CHECK (
    length(request_parts_sha256) = 64 AND request_parts_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  provider_version TEXT NOT NULL CHECK (length(provider_version) BETWEEN 1 AND 256),
  provider_etag TEXT NOT NULL CHECK (length(provider_etag) BETWEEN 1 AND 256),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64 AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  container TEXT NOT NULL CHECK (container IN ('mp4', 'webm', 'matroska', 'quicktime')),
  video_codec TEXT NOT NULL CHECK (video_codec IN ('h264', 'h265', 'vp8', 'vp9', 'av1')),
  audio_codec TEXT NOT NULL CHECK (audio_codec IN ('aac', 'opus', 'none')),
  width INTEGER NOT NULL CHECK (width BETWEEN 1 AND 32768),
  height INTEGER NOT NULL CHECK (height BETWEEN 1 AND 32768),
  duration_ms INTEGER NOT NULL CHECK (duration_ms BETWEEN 1 AND 9007199254740991),
  frame_rate_millihertz INTEGER NOT NULL CHECK (frame_rate_millihertz BETWEEN 1 AND 1000000),
  completed_at_ms INTEGER NOT NULL CHECK (completed_at_ms BETWEEN 0 AND 9007199254740991),
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) = 36)
);
