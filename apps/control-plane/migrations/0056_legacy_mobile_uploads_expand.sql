PRAGMA foreign_keys = ON;

-- Durable mapping for Cap's mobile single-PUT lifecycle. Wire aliases remain
-- the only identifiers exposed to the released client; native UUIDs retain
-- tenant and referential authority. The exact raw key is minted once so a
-- completion request cannot widen an actor/video R2 prefix.
CREATE TABLE legacy_mobile_upload_records_v1 (
  mapped_video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  legacy_actor_id TEXT NOT NULL
    REFERENCES legacy_collaboration_user_aliases_v1(legacy_user_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  storage_integration_id TEXT NOT NULL
    REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  upload_id TEXT NOT NULL UNIQUE REFERENCES video_uploads(id) ON DELETE RESTRICT,
  folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
  raw_file_key TEXT NOT NULL UNIQUE CHECK (
    length(raw_file_key) BETWEEN 35 AND 512
    AND raw_file_key LIKE legacy_actor_id || '/' || legacy_video_id || '/raw-upload.%'
    AND raw_file_key NOT LIKE '%..%'
    AND raw_file_key NOT LIKE '%//%'
    AND raw_file_key NOT LIKE '%\%'
  ),
  file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 1024),
  content_type TEXT NOT NULL CHECK (
    length(content_type) BETWEEN 7 AND 127 AND substr(content_type, 1, 6) = 'video/'
  ),
  declared_content_length INTEGER CHECK (
    declared_content_length BETWEEN 0 AND 9007199254740991
  ),
  duration_seconds REAL CHECK (
    duration_seconds IS NULL OR duration_seconds BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  width REAL CHECK (
    width IS NULL OR width BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  height REAL CHECK (
    height IS NULL OR height BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  ignored_fps REAL CHECK (
    ignored_fps IS NULL OR ignored_fps BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN (
    'uploading', 'provider_pending', 'processing', 'complete', 'error'
  )),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36)
);
CREATE INDEX legacy_mobile_upload_records_actor_state_v1
  ON legacy_mobile_upload_records_v1(actor_id, lifecycle_state, created_at_ms);

CREATE TRIGGER legacy_mobile_upload_records_transition_v1
BEFORE UPDATE ON legacy_mobile_upload_records_v1
WHEN NOT (
  OLD.lifecycle_state = 'uploading' AND NEW.lifecycle_state = 'provider_pending'
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.actor_id = NEW.actor_id AND OLD.legacy_actor_id = NEW.legacy_actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.upload_id = NEW.upload_id AND OLD.folder_id IS NEW.folder_id
  AND OLD.raw_file_key = NEW.raw_file_key AND OLD.file_name = NEW.file_name
  AND OLD.content_type = NEW.content_type
  AND OLD.declared_content_length IS NEW.declared_content_length
  AND OLD.duration_seconds IS NEW.duration_seconds
  AND OLD.width IS NEW.width AND OLD.height IS NEW.height
  AND OLD.ignored_fps IS NEW.ignored_fps
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.updated_at_ms >= OLD.updated_at_ms
  AND NEW.last_operation_id <> OLD.last_operation_id
) AND NOT (
  OLD.lifecycle_state = 'provider_pending'
  AND NEW.lifecycle_state IN ('processing', 'error')
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.actor_id = NEW.actor_id AND OLD.legacy_actor_id = NEW.legacy_actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.upload_id = NEW.upload_id AND OLD.folder_id IS NEW.folder_id
  AND OLD.raw_file_key = NEW.raw_file_key AND OLD.file_name = NEW.file_name
  AND OLD.content_type = NEW.content_type
  AND OLD.declared_content_length IS NEW.declared_content_length
  AND OLD.duration_seconds IS NEW.duration_seconds
  AND OLD.width IS NEW.width AND OLD.height IS NEW.height
  AND OLD.ignored_fps IS NEW.ignored_fps
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.updated_at_ms >= OLD.updated_at_ms
  AND NEW.last_operation_id <> OLD.last_operation_id
) AND NOT (
  OLD.lifecycle_state = 'processing'
  AND NEW.lifecycle_state IN ('complete', 'error')
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.actor_id = NEW.actor_id AND OLD.legacy_actor_id = NEW.legacy_actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.upload_id = NEW.upload_id AND OLD.folder_id IS NEW.folder_id
  AND OLD.raw_file_key = NEW.raw_file_key AND OLD.file_name = NEW.file_name
  AND OLD.content_type = NEW.content_type
  AND OLD.declared_content_length IS NEW.declared_content_length
  AND OLD.duration_seconds IS NEW.duration_seconds
  AND OLD.width IS NEW.width AND OLD.height IS NEW.height
  AND OLD.ignored_fps IS NEW.ignored_fps
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.updated_at_ms >= OLD.updated_at_ms
  AND NEW.last_operation_id <> OLD.last_operation_id
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_record_transition_v1');
END;

CREATE TRIGGER legacy_mobile_upload_records_no_delete_v1
BEFORE DELETE ON legacy_mobile_upload_records_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_record_immutable_v1');
END;

-- Every carrier is intentionally non-idempotent at HTTP level because the
-- released mobile client sends no Idempotency-Key. Operations remain immutable
-- audit evidence; only completion may be provider-pending.
CREATE TABLE legacy_mobile_upload_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-b0116dd82b010477',
    'cap-v1-b43b6ede64a73798',
    'cap-v1-62469fe03e030052'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN ('create', 'complete', 'progress')),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('complete', 'provider_pending')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX legacy_mobile_upload_operations_video_time_v1
  ON legacy_mobile_upload_operations_v1(mapped_video_id, created_at_ms, operation_id);

CREATE TRIGGER legacy_mobile_upload_operations_no_update_v1
BEFORE UPDATE ON legacy_mobile_upload_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_mobile_upload_operations_no_delete_v1
BEFORE DELETE ON legacy_mobile_upload_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_evidence_immutable_v1');
END;

-- A successful R2 HEAD is evidence that bytes exist, not that the protected
-- Cap workflow was submitted. Only an independently admitted provider worker
-- may advance this intent; the HTTP adapter returns 503 while it is pending.
CREATE TABLE legacy_mobile_upload_processing_intents_v1 (
  mapped_video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_mobile_upload_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  raw_file_key TEXT NOT NULL CHECK (length(raw_file_key) BETWEEN 35 AND 512),
  observed_bytes INTEGER NOT NULL CHECK (observed_bytes BETWEEN 1 AND 9007199254740991),
  requested_content_length INTEGER CHECK (
    requested_content_length BETWEEN 0 AND 9007199254740991
  ),
  state TEXT NOT NULL CHECK (state IN ('provider_pending', 'submitted', 'complete', 'failed')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  submitted_at_ms INTEGER CHECK (
    submitted_at_ms IS NULL OR submitted_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  terminal_at_ms INTEGER CHECK (
    terminal_at_ms IS NULL OR terminal_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'provider_pending' AND submitted_at_ms IS NULL AND terminal_at_ms IS NULL)
    OR (state = 'submitted' AND submitted_at_ms IS NOT NULL AND terminal_at_ms IS NULL)
    OR (state IN ('complete', 'failed') AND submitted_at_ms IS NOT NULL AND terminal_at_ms IS NOT NULL)
  )
);

CREATE TRIGGER legacy_mobile_upload_processing_intents_transition_v1
BEFORE UPDATE ON legacy_mobile_upload_processing_intents_v1
WHEN NOT (
  OLD.state = 'provider_pending' AND NEW.state = 'submitted'
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.operation_id = NEW.operation_id AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.raw_file_key = NEW.raw_file_key AND OLD.observed_bytes = NEW.observed_bytes
  AND OLD.requested_content_length IS NEW.requested_content_length
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.submitted_at_ms IS NOT NULL AND NEW.terminal_at_ms IS NULL
) AND NOT (
  OLD.state = 'submitted' AND NEW.state IN ('complete', 'failed')
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.operation_id = NEW.operation_id AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.raw_file_key = NEW.raw_file_key AND OLD.observed_bytes = NEW.observed_bytes
  AND OLD.requested_content_length IS NEW.requested_content_length
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.submitted_at_ms = NEW.submitted_at_ms AND NEW.terminal_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_processing_intent_transition_v1');
END;

CREATE TRIGGER legacy_mobile_upload_processing_intents_no_delete_v1
BEFORE DELETE ON legacy_mobile_upload_processing_intents_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_evidence_immutable_v1');
END;

CREATE TABLE legacy_mobile_upload_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'authority', 'mutation', 'postcondition', 'provider_pending'
  )),
  expected_count INTEGER NOT NULL,
  actual_count INTEGER NOT NULL,
  PRIMARY KEY (operation_id, assertion_kind)
);

CREATE TRIGGER legacy_mobile_upload_assertions_guard_v1
BEFORE INSERT ON legacy_mobile_upload_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_mobile_upload_assertion_v1');
END;
