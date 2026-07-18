PRAGMA foreign_keys = ON;

-- Cap derives media keys from a source owner/video prefix and a five-way
-- source discriminator. Native Frame keys and lifecycle states are broader,
-- so retain the exact mobile projection instead of guessing at request time.
CREATE TABLE legacy_mobile_cap_media_v1 (
  mapped_video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  object_prefix TEXT NOT NULL CHECK (
    length(object_prefix) BETWEEN 3 AND 512
    AND substr(object_prefix, -1, 1) = '/'
    AND object_prefix NOT LIKE '/%'
    AND object_prefix NOT LIKE '%\\%'
    AND object_prefix NOT LIKE '%../%'
  ),
  source_type TEXT NOT NULL CHECK (source_type IN (
    'MediaConvert', 'local', 'desktopMP4', 'desktopSegments', 'webMP4'
  )),
  transcription_status TEXT CHECK (transcription_status IS NULL OR transcription_status IN (
    'PROCESSING', 'COMPLETE', 'ERROR', 'SKIPPED', 'NO_AUDIO'
  )),
  view_count REAL NOT NULL DEFAULT 0 CHECK (
    view_count BETWEEN 0 AND 1.7976931348623157e308
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN created_at_ms AND 9007199254740991
  )
);
CREATE INDEX legacy_mobile_cap_media_owner_org_v1
  ON legacy_mobile_cap_media_v1(owner_id, organization_id, legacy_video_id);

INSERT OR IGNORE INTO legacy_mobile_cap_media_v1(
  mapped_video_id, legacy_video_id, owner_id, organization_id, object_prefix,
  source_type, transcription_status, view_count, created_at_ms, updated_at_ms
)
SELECT
  video.id,
  video_alias.legacy_video_id,
  video.owner_id,
  video.organization_id,
  COALESCE(
    instant.storage_prefix,
    owner_alias.legacy_user_id || '/' || video_alias.legacy_video_id || '/'
  ),
  CASE
    WHEN instant.mapped_video_id IS NOT NULL THEN 'desktopMP4'
    WHEN video.playback_object_key LIKE '%.m3u8' THEN 'MediaConvert'
    WHEN video.playback_object_key LIKE '%.mp4'
      OR video.source_object_key LIKE '%.mp4' THEN 'webMP4'
    ELSE 'MediaConvert'
  END,
  NULL,
  0,
  video.created_at_ms,
  video.updated_at_ms
FROM videos video
JOIN legacy_collaboration_video_aliases_v1 video_alias
  ON video_alias.mapped_video_id = video.id
JOIN legacy_collaboration_user_aliases_v1 owner_alias
  ON owner_alias.mapped_user_id = video.owner_id
LEFT JOIN legacy_extension_instant_recordings_v1 instant
  ON instant.mapped_video_id = video.id
WHERE video.organization_id IS NOT NULL;

-- Alias creation is the common compatibility dual-write point. Instant
-- recording insertion follows it and upgrades the inferred source/prefix.
CREATE TRIGGER legacy_mobile_cap_media_alias_insert_v1
AFTER INSERT ON legacy_collaboration_video_aliases_v1
BEGIN
  INSERT OR IGNORE INTO legacy_mobile_cap_media_v1(
    mapped_video_id, legacy_video_id, owner_id, organization_id, object_prefix,
    source_type, transcription_status, view_count, created_at_ms, updated_at_ms
  )
  SELECT
    video.id, NEW.legacy_video_id, video.owner_id, video.organization_id,
    owner_alias.legacy_user_id || '/' || NEW.legacy_video_id || '/',
    CASE
      WHEN video.playback_object_key LIKE '%.m3u8' THEN 'MediaConvert'
      WHEN video.playback_object_key LIKE '%.mp4'
        OR video.source_object_key LIKE '%.mp4' THEN 'webMP4'
      ELSE 'MediaConvert'
    END,
    NULL, 0, video.created_at_ms, video.updated_at_ms
  FROM videos video
  JOIN legacy_collaboration_user_aliases_v1 owner_alias
    ON owner_alias.mapped_user_id = video.owner_id
  WHERE video.id = NEW.mapped_video_id
    AND video.organization_id IS NOT NULL;
END;

CREATE TRIGGER legacy_mobile_cap_media_instant_insert_v1
AFTER INSERT ON legacy_extension_instant_recordings_v1
BEGIN
  UPDATE legacy_mobile_cap_media_v1
  SET object_prefix = NEW.storage_prefix,
      source_type = 'desktopMP4',
      updated_at_ms = CASE
        WHEN updated_at_ms < NEW.created_at_ms THEN NEW.created_at_ms
        ELSE updated_at_ms
      END
  WHERE mapped_video_id = NEW.mapped_video_id;
END;

CREATE TRIGGER legacy_mobile_cap_media_video_source_update_v1
AFTER UPDATE OF source_object_key, playback_object_key ON videos
BEGIN
  UPDATE legacy_mobile_cap_media_v1
  SET source_type = CASE
        WHEN NEW.playback_object_key LIKE '%.m3u8' THEN 'MediaConvert'
        WHEN NEW.playback_object_key LIKE '%.mp4'
          OR NEW.source_object_key LIKE '%.mp4' THEN 'webMP4'
        ELSE source_type
      END,
      updated_at_ms = CASE
        WHEN updated_at_ms < NEW.updated_at_ms THEN NEW.updated_at_ms
        ELSE updated_at_ms
      END
  WHERE mapped_video_id = NEW.id
    AND NOT EXISTS (
      SELECT 1 FROM legacy_extension_instant_recordings_v1 instant
      WHERE instant.mapped_video_id = NEW.id
    );
END;

-- Cap has one progress row per video and keeps processing-only fields that
-- native Frame uploads do not carry. This shadow accepts exact imports while
-- deterministic triggers keep native/desktop writes visible to mobile reads.
CREATE TABLE legacy_mobile_cap_uploads_v1 (
  mapped_video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  uploaded REAL NOT NULL CHECK (
    uploaded BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  total REAL NOT NULL CHECK (
    total BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  phase TEXT NOT NULL CHECK (phase IN (
    'uploading', 'processing', 'generating_thumbnail', 'complete', 'error'
  )),
  processing_progress REAL NOT NULL DEFAULT 0 CHECK (
    processing_progress BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  processing_message TEXT CHECK (
    processing_message IS NULL OR length(processing_message) <= 255
  ),
  processing_error TEXT,
  raw_file_key TEXT CHECK (raw_file_key IS NULL OR length(raw_file_key) <= 2048),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991)
);

INSERT OR IGNORE INTO legacy_mobile_cap_uploads_v1(
  mapped_video_id, uploaded, total, phase, processing_progress,
  processing_message, processing_error, raw_file_key, updated_at_ms
)
SELECT
  upload.video_id,
  COALESCE(desktop.uploaded, CAST(upload.received_bytes AS REAL)),
  COALESCE(desktop.total, CAST(upload.expected_bytes AS REAL)),
  CASE upload.state
    WHEN 'complete' THEN 'complete'
    WHEN 'failed' THEN 'error'
    WHEN 'aborted' THEN 'error'
    WHEN 'finalizing' THEN 'processing'
    ELSE 'uploading'
  END,
  0, NULL, NULL, upload.source_object_key,
  MAX(upload.updated_at_ms, COALESCE(desktop.updated_at_ms, 0))
FROM video_uploads upload
LEFT JOIN legacy_desktop_video_uploads_v1 desktop
  ON desktop.video_id = upload.video_id
WHERE upload.id = (
  SELECT candidate.id FROM video_uploads candidate
  WHERE candidate.video_id = upload.video_id
  ORDER BY candidate.updated_at_ms DESC, candidate.id
  LIMIT 1
);

INSERT OR IGNORE INTO legacy_mobile_cap_uploads_v1(
  mapped_video_id, uploaded, total, phase, processing_progress,
  processing_message, processing_error, raw_file_key, updated_at_ms
)
SELECT video_id, uploaded, total, 'uploading', 0, NULL, NULL, NULL, updated_at_ms
FROM legacy_desktop_video_uploads_v1;

CREATE TRIGGER legacy_mobile_cap_upload_native_insert_v1
AFTER INSERT ON video_uploads
BEGIN
  INSERT INTO legacy_mobile_cap_uploads_v1(
    mapped_video_id, uploaded, total, phase, processing_progress,
    processing_message, processing_error, raw_file_key, updated_at_ms
  ) VALUES (
    NEW.video_id, CAST(NEW.received_bytes AS REAL), CAST(NEW.expected_bytes AS REAL),
    CASE NEW.state
      WHEN 'complete' THEN 'complete'
      WHEN 'failed' THEN 'error'
      WHEN 'aborted' THEN 'error'
      WHEN 'finalizing' THEN 'processing'
      ELSE 'uploading'
    END,
    0, NULL, NULL, NEW.source_object_key, NEW.updated_at_ms
  )
  ON CONFLICT(mapped_video_id) DO UPDATE SET
    uploaded = excluded.uploaded,
    total = excluded.total,
    phase = excluded.phase,
    raw_file_key = COALESCE(excluded.raw_file_key, raw_file_key),
    updated_at_ms = excluded.updated_at_ms
  WHERE excluded.updated_at_ms >= updated_at_ms;
END;

CREATE TRIGGER legacy_mobile_cap_upload_native_update_v1
AFTER UPDATE OF received_bytes, expected_bytes, state, source_object_key, updated_at_ms
ON video_uploads
BEGIN
  INSERT INTO legacy_mobile_cap_uploads_v1(
    mapped_video_id, uploaded, total, phase, processing_progress,
    processing_message, processing_error, raw_file_key, updated_at_ms
  ) VALUES (
    NEW.video_id, CAST(NEW.received_bytes AS REAL), CAST(NEW.expected_bytes AS REAL),
    CASE NEW.state
      WHEN 'complete' THEN 'complete'
      WHEN 'failed' THEN 'error'
      WHEN 'aborted' THEN 'error'
      WHEN 'finalizing' THEN 'processing'
      ELSE 'uploading'
    END,
    0, NULL, NULL, NEW.source_object_key, NEW.updated_at_ms
  )
  ON CONFLICT(mapped_video_id) DO UPDATE SET
    uploaded = excluded.uploaded,
    total = excluded.total,
    phase = excluded.phase,
    raw_file_key = COALESCE(excluded.raw_file_key, raw_file_key),
    updated_at_ms = excluded.updated_at_ms
  WHERE excluded.updated_at_ms >= updated_at_ms;
END;

CREATE TRIGGER legacy_mobile_cap_upload_desktop_insert_v1
AFTER INSERT ON legacy_desktop_video_uploads_v1
BEGIN
  INSERT INTO legacy_mobile_cap_uploads_v1(
    mapped_video_id, uploaded, total, phase, processing_progress,
    processing_message, processing_error, raw_file_key, updated_at_ms
  ) VALUES (
    NEW.video_id, NEW.uploaded, NEW.total, 'uploading', 0,
    NULL, NULL, NULL, NEW.updated_at_ms
  )
  ON CONFLICT(mapped_video_id) DO UPDATE SET
    uploaded = excluded.uploaded,
    total = excluded.total,
    updated_at_ms = excluded.updated_at_ms
  WHERE excluded.updated_at_ms >= updated_at_ms;
END;

CREATE TRIGGER legacy_mobile_cap_upload_desktop_update_v1
AFTER UPDATE OF uploaded, total, updated_at_ms ON legacy_desktop_video_uploads_v1
BEGIN
  UPDATE legacy_mobile_cap_uploads_v1
  SET uploaded = NEW.uploaded,
      total = NEW.total,
      updated_at_ms = NEW.updated_at_ms
  WHERE mapped_video_id = NEW.video_id
    AND NEW.updated_at_ms >= updated_at_ms;
END;

-- Mobile DELETE intentionally has no idempotency key. The D1 tombstone and
-- provider continuation are recorded atomically; an R2 failure therefore
-- returns 500 once and a retry observes the already-deleted 404, matching the
-- source's database-before-storage ordering without leaking another tenant.
CREATE TABLE legacy_mobile_cap_delete_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL CHECK (length(legacy_video_id) = 15),
  object_prefix TEXT NOT NULL CHECK (
    length(object_prefix) BETWEEN 3 AND 512 AND substr(object_prefix, -1, 1) = '/'
  ),
  state TEXT NOT NULL CHECK (state IN ('storage_pending', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'storage_pending' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_mobile_cap_delete_audit_v1 (
  audit_id TEXT PRIMARY KEY NOT NULL CHECK (length(audit_id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_mobile_cap_delete_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_digest TEXT NOT NULL CHECK (length(actor_digest) = 64),
  video_digest TEXT NOT NULL CHECK (length(video_digest) = 64),
  outcome TEXT NOT NULL CHECK (outcome = 'authorized_storage_pending'),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_mobile_cap_delete_assertions_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_mobile_cap_delete_operations_v1(operation_id) ON DELETE RESTRICT
    CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN ('authority', 'tombstone', 'cleanup')),
  expected_count INTEGER NOT NULL CHECK (expected_count = 1),
  actual_count INTEGER NOT NULL CHECK (actual_count = 1),
  PRIMARY KEY (operation_id, assertion_kind)
);
CREATE TRIGGER legacy_mobile_cap_delete_assertion_guard_v1
BEFORE INSERT ON legacy_mobile_cap_delete_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_assertion_v1');
END;

CREATE TRIGGER legacy_mobile_cap_delete_operation_transition_v1
BEFORE UPDATE ON legacy_mobile_cap_delete_operations_v1
WHEN NOT (
  OLD.state = 'storage_pending' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.object_prefix = NEW.object_prefix
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.completed_at_ms IS NULL AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_cap_delete_operation_no_delete_v1
BEFORE DELETE ON legacy_mobile_cap_delete_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_cap_delete_audit_no_update_v1
BEFORE UPDATE ON legacy_mobile_cap_delete_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_cap_delete_audit_no_delete_v1
BEFORE DELETE ON legacy_mobile_cap_delete_audit_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_cap_delete_assertion_no_update_v1
BEFORE UPDATE ON legacy_mobile_cap_delete_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_cap_delete_assertion_no_delete_v1
BEFORE DELETE ON legacy_mobile_cap_delete_assertions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_cap_delete_evidence_immutable_v1');
END;
