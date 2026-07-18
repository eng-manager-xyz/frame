PRAGMA foreign_keys = ON;

-- Cap exposes one upload-progress row per video and carries a distinct start
-- timestamp. Earlier mobile compatibility only needed the latest timestamp;
-- retain the missing value explicitly before serving the Effect RPC.
ALTER TABLE legacy_mobile_cap_uploads_v1 ADD COLUMN started_at_ms INTEGER
  CHECK (started_at_ms IS NULL OR started_at_ms BETWEEN 0 AND 9007199254740991);
UPDATE legacy_mobile_cap_uploads_v1
SET started_at_ms = COALESCE(
  (SELECT MIN(upload.created_at_ms) FROM video_uploads upload
   WHERE upload.video_id = legacy_mobile_cap_uploads_v1.mapped_video_id),
  (SELECT video.created_at_ms FROM videos video
   WHERE video.id = legacy_mobile_cap_uploads_v1.mapped_video_id),
  updated_at_ms
);

-- Migrations 0052/0056 installed native, desktop, and mobile projection
-- triggers before `started_at_ms` existed. Their INSERT column lists therefore
-- continue to omit it after this ALTER. Fill the value at the projection
-- boundary so every row created after cutover remains encodable by Cap's
-- non-null `UploadProgress.startedAt` schema.
CREATE TRIGGER legacy_upload_storage_progress_started_at_insert_v1
AFTER INSERT ON legacy_mobile_cap_uploads_v1
WHEN NEW.started_at_ms IS NULL
BEGIN
  UPDATE legacy_mobile_cap_uploads_v1
  SET started_at_ms = COALESCE(
    (SELECT MIN(upload.created_at_ms) FROM video_uploads upload
     WHERE upload.video_id = NEW.mapped_video_id),
    (SELECT video.created_at_ms FROM videos video
     WHERE video.id = NEW.mapped_video_id),
    NEW.updated_at_ms
  )
  WHERE mapped_video_id = NEW.mapped_video_id AND started_at_ms IS NULL;
END;

-- Cap's videoEdits sourceKey is the exact owner/video original-media key.
-- Native edit documents do not retain it, so preserve the source projection.
CREATE TABLE legacy_upload_storage_edit_sources_v1 (
  mapped_video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  source_key TEXT NOT NULL UNIQUE CHECK (
    length(source_key) BETWEEN 35 AND 512
    AND source_key NOT LIKE '/%'
    AND source_key NOT LIKE '%//%'
    AND source_key NOT LIKE '%..%'
    AND source_key NOT LIKE '%\%'
    AND source_key LIKE '%/source/original.mp4'
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36)
);
INSERT OR IGNORE INTO legacy_upload_storage_edit_sources_v1(
  mapped_video_id, source_key, created_at_ms, last_operation_id
)
SELECT edit.video_id, media.object_prefix || 'source/original.mp4', MIN(edit.created_at_ms), NULL
FROM video_edits edit
JOIN legacy_mobile_cap_media_v1 media ON media.mapped_video_id = edit.video_id
GROUP BY edit.video_id, media.object_prefix;

-- Keep the compatibility source-key projection live for native edit writes.
-- The key exists only while at least one native edit document exists.
CREATE TRIGGER legacy_upload_storage_edit_insert_v1
AFTER INSERT ON video_edits
BEGIN
  INSERT OR IGNORE INTO legacy_upload_storage_edit_sources_v1(
    mapped_video_id, source_key, created_at_ms, last_operation_id
  )
  SELECT NEW.video_id, media.object_prefix || 'source/original.mp4', NEW.created_at_ms, NULL
  FROM legacy_mobile_cap_media_v1 media
  WHERE media.mapped_video_id = NEW.video_id;
END;
CREATE TRIGGER legacy_upload_storage_edit_media_insert_v1
AFTER INSERT ON legacy_mobile_cap_media_v1
BEGIN
  INSERT OR IGNORE INTO legacy_upload_storage_edit_sources_v1(
    mapped_video_id, source_key, created_at_ms, last_operation_id
  )
  SELECT NEW.mapped_video_id, NEW.object_prefix || 'source/original.mp4', MIN(edit.created_at_ms), NULL
  FROM video_edits edit WHERE edit.video_id = NEW.mapped_video_id
  HAVING COUNT(*) > 0;
END;
CREATE TRIGGER legacy_upload_storage_edit_media_prefix_v1
AFTER UPDATE OF object_prefix ON legacy_mobile_cap_media_v1
BEGIN
  UPDATE legacy_upload_storage_edit_sources_v1
  SET source_key = NEW.object_prefix || 'source/original.mp4'
  WHERE mapped_video_id = NEW.mapped_video_id;
END;

-- Native `shared_videos` is same-tenant and native `space_videos` is a Frame
-- placement authority. Cap can share across organizations, so preserve its
-- replacement set without weakening either native invariant.
CREATE TABLE legacy_upload_storage_organization_shares_v1 (
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  shared_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  shared_at_ms INTEGER NOT NULL CHECK (shared_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36),
  PRIMARY KEY(mapped_video_id, organization_id)
);
INSERT OR IGNORE INTO legacy_upload_storage_organization_shares_v1(
  mapped_video_id, organization_id, shared_by_user_id, shared_at_ms, last_operation_id
)
SELECT video_id, organization_id, shared_by_user_id, shared_at_ms, last_operation_id
FROM shared_videos WHERE revoked_at_ms IS NULL AND folder_id IS NULL;

CREATE TABLE legacy_upload_storage_space_shares_v1 (
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE RESTRICT,
  shared_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  shared_at_ms INTEGER NOT NULL CHECK (shared_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36),
  PRIMARY KEY(mapped_video_id, space_id)
);
INSERT OR IGNORE INTO legacy_upload_storage_space_shares_v1(
  mapped_video_id, space_id, shared_by_user_id, shared_at_ms, last_operation_id
)
SELECT video_id, space_id, added_by_user_id, added_at_ms, NULL FROM space_videos;

-- Frame adds replay identities around Cap mutations. Read-only actions and
-- RPCs remain unjournaled; their response capabilities are always short-lived.
CREATE TABLE legacy_upload_storage_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-4245d3bd72f59e22',
    'cap-v1-dd270efc913f9af9',
    'cap-v1-6ed7083eeb37e3f8',
    'cap-v1-d89571c3e0f65def',
    'cap-v1-55d41a7742153f1b'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'progress_update', 'create_upload', 'delete_result', 'reconcile_edit', 'share_cap'
  )),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64 AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'storage_pending', 'complete')),
  result_json TEXT CHECK (
    result_json IS NULL OR (json_valid(result_json) AND length(result_json) <= 1048576)
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  UNIQUE(source_operation_id, actor_id, idempotency_key_digest),
  CHECK (
    (state IN ('claimed', 'storage_pending') AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL AND result_json IS NOT NULL)
  )
);
CREATE INDEX legacy_upload_storage_operations_video_v1
  ON legacy_upload_storage_operations_v1(mapped_video_id, created_at_ms, operation_id);

CREATE TRIGGER legacy_upload_storage_operations_transition_v1
BEFORE UPDATE ON legacy_upload_storage_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state IN ('storage_pending', 'complete')
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.completed_at_ms IS NULL
  AND ((NEW.state = 'storage_pending' AND NEW.result_json IS NULL AND NEW.completed_at_ms IS NULL)
    OR (NEW.state = 'complete' AND NEW.result_json IS NOT NULL AND NEW.completed_at_ms IS NOT NULL))
) AND NOT (
  OLD.state = 'storage_pending' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.result_json IS NULL AND NEW.result_json IS NOT NULL
  AND OLD.completed_at_ms IS NULL AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_operation_transition_v1');
END;
CREATE TRIGGER legacy_upload_storage_operations_no_delete_v1
BEFORE DELETE ON legacy_upload_storage_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_evidence_immutable_v1');
END;

CREATE TABLE legacy_upload_storage_capability_intents_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_upload_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  storage_integration_id TEXT NOT NULL
    REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL CHECK (
    length(object_key) BETWEEN 33 AND 512
    AND object_key NOT LIKE '/%' AND object_key NOT LIKE '%//%'
    AND object_key NOT LIKE '%..%' AND object_key NOT LIKE '%\%'
  ),
  method TEXT NOT NULL CHECK (method = 'PUT'),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE TRIGGER legacy_upload_storage_capability_no_update_v1
BEFORE UPDATE ON legacy_upload_storage_capability_intents_v1 BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_upload_storage_capability_no_delete_v1
BEFORE DELETE ON legacy_upload_storage_capability_intents_v1 BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_evidence_immutable_v1');
END;

CREATE TABLE legacy_upload_storage_delete_intents_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_upload_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  storage_integration_id TEXT NOT NULL
    REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL CHECK (
    length(object_key) BETWEEN 33 AND 512
    AND object_key LIKE '%/result.mp4'
    AND object_key NOT LIKE '/%' AND object_key NOT LIKE '%//%'
    AND object_key NOT LIKE '%..%' AND object_key NOT LIKE '%\%'
  ),
  state TEXT NOT NULL CHECK (state IN ('storage_pending', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK ((state = 'storage_pending' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL))
);
CREATE TRIGGER legacy_upload_storage_delete_transition_v1
BEFORE UPDATE ON legacy_upload_storage_delete_intents_v1
WHEN NOT (
  OLD.state = 'storage_pending' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.object_key = NEW.object_key
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.completed_at_ms IS NULL AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_delete_transition_v1');
END;
CREATE TRIGGER legacy_upload_storage_delete_no_delete_v1
BEFORE DELETE ON legacy_upload_storage_delete_intents_v1 BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_evidence_immutable_v1');
END;

CREATE TABLE legacy_upload_storage_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'authority', 'mutation', 'postcondition', 'storage_pending'
  )),
  expected_count INTEGER NOT NULL,
  actual_count INTEGER NOT NULL,
  PRIMARY KEY(operation_id, assertion_kind)
);
CREATE TRIGGER legacy_upload_storage_assertion_guard_v1
BEFORE INSERT ON legacy_upload_storage_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_upload_storage_assertion_v1');
END;
