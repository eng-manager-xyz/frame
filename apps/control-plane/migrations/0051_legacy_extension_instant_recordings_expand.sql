PRAGMA foreign_keys = ON;

-- Lossless fields accepted by Cap's InstantRecordingCreateInput. Native Frame
-- retains the collision-safe UUID video while the extension receives the
-- immutable 15-character alias recorded in legacy_collaboration_video_aliases_v1.
ALTER TABLE videos ADD COLUMN legacy_instant_recording INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_instant_recording IN (0, 1));
ALTER TABLE videos ADD COLUMN legacy_instant_resolution TEXT
  CHECK (legacy_instant_resolution IS NULL OR length(legacy_instant_resolution) <= 4096);
ALTER TABLE videos ADD COLUMN legacy_instant_width REAL
  CHECK (legacy_instant_width IS NULL OR legacy_instant_width BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308);
ALTER TABLE videos ADD COLUMN legacy_instant_height REAL
  CHECK (legacy_instant_height IS NULL OR legacy_instant_height BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308);
ALTER TABLE videos ADD COLUMN legacy_instant_video_codec TEXT
  CHECK (legacy_instant_video_codec IS NULL OR length(legacy_instant_video_codec) <= 4096);
ALTER TABLE videos ADD COLUMN legacy_instant_audio_codec TEXT
  CHECK (legacy_instant_audio_codec IS NULL OR length(legacy_instant_audio_codec) <= 4096);
ALTER TABLE videos ADD COLUMN legacy_instant_supports_progress INTEGER
  CHECK (legacy_instant_supports_progress IS NULL OR legacy_instant_supports_progress IN (0, 1));

CREATE TABLE legacy_extension_instant_recordings_v1 (
  legacy_video_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL UNIQUE REFERENCES videos(id) ON DELETE RESTRICT,
  upload_id TEXT UNIQUE REFERENCES video_uploads(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  storage_integration_id TEXT NOT NULL REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  storage_prefix TEXT NOT NULL UNIQUE CHECK (
    storage_prefix = actor_id || '/' || legacy_video_id || '/'
    AND length(storage_prefix) BETWEEN 18 AND 512
  ),
  source_object_key TEXT NOT NULL UNIQUE CHECK (
    source_object_key = storage_prefix || 'result.mp4'
  ),
  supports_upload_progress INTEGER NOT NULL CHECK (supports_upload_progress IN (0, 1)),
  lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN ('active', 'deleting', 'deleted')),
  storage_cleanup_state TEXT NOT NULL CHECK (
    storage_cleanup_state IN ('not_requested', 'pending', 'complete')
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  delete_started_at_ms INTEGER CHECK (delete_started_at_ms BETWEEN 0 AND 9007199254740991),
  deleted_at_ms INTEGER CHECK (deleted_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  CHECK (
    (lifecycle_state = 'active' AND storage_cleanup_state = 'not_requested'
      AND delete_started_at_ms IS NULL AND deleted_at_ms IS NULL)
    OR (lifecycle_state = 'deleting' AND storage_cleanup_state = 'pending'
      AND delete_started_at_ms IS NOT NULL AND deleted_at_ms IS NULL)
    OR (lifecycle_state = 'deleted' AND storage_cleanup_state = 'complete'
      AND delete_started_at_ms IS NOT NULL AND deleted_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_extension_instant_actor_state_v1
  ON legacy_extension_instant_recordings_v1(actor_id, lifecycle_state, legacy_video_id);
CREATE INDEX legacy_extension_instant_org_state_v1
  ON legacy_extension_instant_recordings_v1(organization_id, lifecycle_state, legacy_video_id);

CREATE TRIGGER legacy_extension_instant_recording_transition_v1
BEFORE UPDATE ON legacy_extension_instant_recordings_v1
WHEN NOT (
  OLD.lifecycle_state = 'active' AND NEW.lifecycle_state = 'active'
  AND ((OLD.upload_id IS NULL AND NEW.upload_id IS NOT NULL)
    OR OLD.upload_id IS NEW.upload_id)
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.storage_prefix = NEW.storage_prefix
  AND OLD.source_object_key = NEW.source_object_key
  AND OLD.supports_upload_progress = NEW.supports_upload_progress
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.delete_started_at_ms IS NULL AND NEW.delete_started_at_ms IS NULL
  AND OLD.deleted_at_ms IS NULL AND NEW.deleted_at_ms IS NULL
  AND NEW.storage_cleanup_state = 'not_requested'
) AND NOT (
  OLD.lifecycle_state = 'active' AND NEW.lifecycle_state = 'deleting'
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.upload_id IS NEW.upload_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.storage_prefix = NEW.storage_prefix
  AND OLD.source_object_key = NEW.source_object_key
  AND OLD.supports_upload_progress = NEW.supports_upload_progress
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.storage_cleanup_state = 'pending'
  AND NEW.delete_started_at_ms IS NOT NULL AND NEW.deleted_at_ms IS NULL
) AND NOT (
  OLD.lifecycle_state = 'deleting' AND NEW.lifecycle_state = 'deleted'
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.upload_id IS NEW.upload_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.storage_prefix = NEW.storage_prefix
  AND OLD.source_object_key = NEW.source_object_key
  AND OLD.supports_upload_progress = NEW.supports_upload_progress
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.delete_started_at_ms = NEW.delete_started_at_ms
  AND NEW.storage_cleanup_state = 'complete' AND NEW.deleted_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_instant_transition_v1');
END;

CREATE TRIGGER legacy_extension_instant_recording_no_delete_v1
BEFORE DELETE ON legacy_extension_instant_recordings_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_instant_alias_immutable_v1');
END;

CREATE TABLE legacy_extension_instant_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-00422c50f4d39053',
    'cap-v1-8fd4741d6e52465e',
    'cap-v1-82dec55d0fbea3db'
  )),
  action TEXT NOT NULL CHECK (action IN ('create', 'progress', 'delete')),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  uploaded INTEGER CHECK (uploaded BETWEEN 0 AND 9007199254740991),
  total INTEGER CHECK (total BETWEEN 0 AND 9007199254740991),
  source_updated_at_ms INTEGER CHECK (source_updated_at_ms BETWEEN 0 AND 9007199254740991),
  applied INTEGER NOT NULL CHECK (applied IN (0, 1)),
  state TEXT NOT NULL CHECK (state IN ('pending_storage', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (completed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (action = 'progress' AND uploaded IS NOT NULL AND total IS NOT NULL
      AND source_updated_at_ms IS NOT NULL AND state = 'complete'
      AND completed_at_ms IS NOT NULL)
    OR (action = 'create' AND uploaded IS NULL AND total IS NULL
      AND source_updated_at_ms IS NULL AND applied = 1 AND state = 'complete'
      AND completed_at_ms IS NOT NULL)
    OR (action = 'delete' AND uploaded IS NULL AND total IS NULL
      AND source_updated_at_ms IS NULL
      AND ((state = 'pending_storage' AND applied = 0 AND completed_at_ms IS NULL)
        OR (state = 'complete' AND applied = 1 AND completed_at_ms IS NOT NULL)))
  )
);
CREATE INDEX legacy_extension_instant_operations_video_time_v1
  ON legacy_extension_instant_operations_v1(legacy_video_id, created_at_ms, operation_id);

CREATE TRIGGER legacy_extension_instant_operation_transition_v1
BEFORE UPDATE ON legacy_extension_instant_operations_v1
WHEN NOT (
  OLD.action = 'delete' AND OLD.state = 'pending_storage'
  AND NEW.state = 'complete' AND NEW.applied = 1 AND NEW.completed_at_ms IS NOT NULL
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.action = NEW.action
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.request_digest = NEW.request_digest
  AND OLD.uploaded IS NEW.uploaded AND OLD.total IS NEW.total
  AND OLD.source_updated_at_ms IS NEW.source_updated_at_ms
  AND OLD.created_at_ms = NEW.created_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_instant_operation_immutable_v1');
END;

CREATE TRIGGER legacy_extension_instant_operation_no_delete_v1
BEFORE DELETE ON legacy_extension_instant_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_instant_operation_immutable_v1');
END;

-- Frame tightens Cap's timestamp-only guard: an accepted update may never
-- reduce uploaded or total bytes. Stale and regressing calls still return
-- success=true but remain durable no-ops.
CREATE TRIGGER legacy_extension_instant_progress_monotonic_v1
BEFORE UPDATE OF received_bytes, expected_bytes, updated_at_ms ON video_uploads
WHEN EXISTS (
  SELECT 1 FROM legacy_extension_instant_recordings_v1 instant
  WHERE instant.upload_id = OLD.id
)
AND (
  NEW.received_bytes < OLD.received_bytes
  OR NEW.expected_bytes < OLD.expected_bytes
  OR NEW.updated_at_ms < OLD.updated_at_ms
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_instant_progress_regression_v1');
END;

CREATE TABLE legacy_extension_instant_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'create_authority', 'create_postcondition', 'progress_authority',
    'delete_authority', 'delete_cleanup'
  )),
  accepted INTEGER NOT NULL CHECK (accepted IN (0, 1)),
  PRIMARY KEY (operation_id, assertion_kind)
);

CREATE TRIGGER legacy_extension_instant_assertion_guard_v1
BEFORE INSERT ON legacy_extension_instant_assertions_v1
WHEN NEW.accepted <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_extension_instant_assertion_failed_v1');
END;
