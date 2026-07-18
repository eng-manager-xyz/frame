PRAGMA foreign_keys = ON;

-- Expand-only compatibility authority for Cap's retained download, playlist,
-- storage-object, signed-upload, and multipart routes. Provider handles never
-- cross the compatibility boundary: clients receive an opaque UUID and the R2
-- upload id remains in D1 under the same tenant/video/key authority.

ALTER TABLE videos ADD COLUMN legacy_storage_width REAL CHECK (
  legacy_storage_width IS NULL
  OR legacy_storage_width BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
);
ALTER TABLE videos ADD COLUMN legacy_storage_height REAL CHECK (
  legacy_storage_height IS NULL
  OR legacy_storage_height BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
);
ALTER TABLE videos ADD COLUMN legacy_storage_fps REAL CHECK (
  legacy_storage_fps IS NULL
  OR legacy_storage_fps BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
);

CREATE TABLE legacy_core_storage_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-f191ed86271608e3',
    'cap-v1-efc19423a62b7976',
    'cap-v1-f47512c6177fa691',
    'cap-v1-7b584d9338e8bf31',
    'cap-v1-f9deb8104204a30d',
    'cap-v1-7f87205cb7d39ee6',
    'cap-v1-c64cec46e4b828da'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'multipart_abort', 'multipart_complete', 'multipart_initiate',
    'multipart_presign_part', 'recording_complete', 'signed', 'signed_batch'
  )),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  client_idempotency INTEGER NOT NULL CHECK (client_idempotency IN (0, 1)),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  result_binding_json TEXT CHECK (
    result_binding_json IS NULL
    OR (json_valid(result_binding_json) AND length(result_binding_json) <= 1048576)
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'effect_pending', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state IN ('claimed', 'effect_pending') AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  ),
  UNIQUE (source_operation_id, actor_id, idempotency_key_digest)
);
CREATE INDEX legacy_core_storage_operations_video_time_v1
  ON legacy_core_storage_operations_v1(mapped_video_id, created_at_ms, operation_id);

CREATE TRIGGER legacy_core_storage_operations_transition_v1
BEFORE UPDATE ON legacy_core_storage_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state IN ('effect_pending', 'complete')
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.client_idempotency = NEW.client_idempotency
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND (OLD.result_binding_json IS NULL OR OLD.result_binding_json = NEW.result_binding_json)
  AND (
    (NEW.state = 'effect_pending' AND NEW.completed_at_ms IS NULL)
    OR (NEW.state = 'complete' AND NEW.completed_at_ms IS NOT NULL)
  )
) AND NOT (
  OLD.state = 'effect_pending' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id
  AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.client_idempotency = NEW.client_idempotency
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_core_storage_operation_immutable_v1');
END;

CREATE TRIGGER legacy_core_storage_operations_no_delete_v1
BEFORE DELETE ON legacy_core_storage_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_core_storage_operation_immutable_v1');
END;

CREATE TABLE legacy_core_storage_multipart_v1 (
  external_upload_id TEXT PRIMARY KEY NOT NULL CHECK (length(external_upload_id) = 36),
  provider_upload_id TEXT NOT NULL UNIQUE CHECK (length(provider_upload_id) BETWEEN 1 AND 1024),
  initiate_operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_core_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  completion_operation_id TEXT UNIQUE
    REFERENCES legacy_core_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  abort_operation_id TEXT UNIQUE
    REFERENCES legacy_core_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  storage_integration_id TEXT NOT NULL
    REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  object_prefix TEXT NOT NULL CHECK (
    length(object_prefix) BETWEEN 4 AND 1279
    AND object_prefix LIKE '%/'
    AND object_prefix NOT LIKE '/%'
    AND object_prefix NOT LIKE '%\\%'
    AND object_prefix NOT LIKE '%..%'
    AND object_prefix NOT LIKE '%//%'
  ),
  subpath TEXT NOT NULL CHECK (
    length(subpath) BETWEEN 1 AND 768
    AND subpath NOT LIKE '/%'
    AND subpath NOT LIKE '%\\%'
    AND subpath NOT LIKE '%..%'
    AND subpath NOT LIKE '%//%'
  ),
  object_key TEXT NOT NULL UNIQUE CHECK (
    object_key = object_prefix || subpath
    AND length(object_key) BETWEEN 5 AND 2048
  ),
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  state TEXT NOT NULL CHECK (state IN (
    'open', 'completion_pending', 'complete', 'abort_pending', 'aborted'
  )),
  expected_bytes INTEGER CHECK (expected_bytes BETWEEN 1 AND 9007199254740991),
  parts_digest TEXT CHECK (
    parts_digest IS NULL OR (
      length(parts_digest) = 64 AND parts_digest NOT GLOB '*[^0-9a-f]*'
    )
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (
    expires_at_ms BETWEEN created_at_ms + 1 AND 9007199254740991
  ),
  terminal_at_ms INTEGER CHECK (
    terminal_at_ms IS NULL OR terminal_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'open' AND completion_operation_id IS NULL AND abort_operation_id IS NULL
      AND expected_bytes IS NULL AND parts_digest IS NULL AND terminal_at_ms IS NULL)
    OR (state = 'completion_pending' AND completion_operation_id IS NOT NULL
      AND abort_operation_id IS NULL AND expected_bytes IS NOT NULL
      AND parts_digest IS NOT NULL AND terminal_at_ms IS NULL)
    OR (state = 'complete' AND completion_operation_id IS NOT NULL
      AND abort_operation_id IS NULL AND expected_bytes IS NOT NULL
      AND parts_digest IS NOT NULL AND terminal_at_ms IS NOT NULL)
    OR (state = 'abort_pending' AND completion_operation_id IS NULL
      AND abort_operation_id IS NOT NULL AND expected_bytes IS NULL
      AND parts_digest IS NULL AND terminal_at_ms IS NULL)
    OR (state = 'aborted' AND completion_operation_id IS NULL
      AND abort_operation_id IS NOT NULL AND expected_bytes IS NULL
      AND parts_digest IS NULL AND terminal_at_ms IS NOT NULL)
  )
);
CREATE INDEX legacy_core_storage_multipart_owner_state_v1
  ON legacy_core_storage_multipart_v1(actor_id, state, expires_at_ms, external_upload_id);

CREATE TRIGGER legacy_core_storage_multipart_transition_v1
BEFORE UPDATE ON legacy_core_storage_multipart_v1
WHEN NOT (
  OLD.state = 'open' AND NEW.state = 'completion_pending'
  AND OLD.external_upload_id = NEW.external_upload_id
  AND OLD.provider_upload_id = NEW.provider_upload_id
  AND OLD.initiate_operation_id = NEW.initiate_operation_id
  AND OLD.completion_operation_id IS NULL AND NEW.completion_operation_id IS NOT NULL
  AND OLD.abort_operation_id IS NULL AND NEW.abort_operation_id IS NULL
  AND OLD.actor_id = NEW.actor_id AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.object_prefix = NEW.object_prefix
  AND OLD.subpath = NEW.subpath AND OLD.object_key = NEW.object_key
  AND OLD.content_type = NEW.content_type
  AND NEW.expected_bytes IS NOT NULL AND NEW.parts_digest IS NOT NULL
  AND OLD.created_at_ms = NEW.created_at_ms AND OLD.expires_at_ms = NEW.expires_at_ms
  AND NEW.terminal_at_ms IS NULL
) AND NOT (
  OLD.state = 'completion_pending' AND NEW.state = 'complete'
  AND OLD.external_upload_id = NEW.external_upload_id
  AND OLD.provider_upload_id = NEW.provider_upload_id
  AND OLD.initiate_operation_id = NEW.initiate_operation_id
  AND OLD.completion_operation_id = NEW.completion_operation_id
  AND OLD.abort_operation_id IS NEW.abort_operation_id
  AND OLD.actor_id = NEW.actor_id AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.object_prefix = NEW.object_prefix
  AND OLD.subpath = NEW.subpath AND OLD.object_key = NEW.object_key
  AND OLD.content_type = NEW.content_type
  AND OLD.expected_bytes = NEW.expected_bytes AND OLD.parts_digest = NEW.parts_digest
  AND OLD.created_at_ms = NEW.created_at_ms AND OLD.expires_at_ms = NEW.expires_at_ms
  AND NEW.terminal_at_ms IS NOT NULL
) AND NOT (
  OLD.state = 'open' AND NEW.state = 'abort_pending'
  AND OLD.external_upload_id = NEW.external_upload_id
  AND OLD.provider_upload_id = NEW.provider_upload_id
  AND OLD.initiate_operation_id = NEW.initiate_operation_id
  AND OLD.completion_operation_id IS NULL AND NEW.completion_operation_id IS NULL
  AND OLD.abort_operation_id IS NULL AND NEW.abort_operation_id IS NOT NULL
  AND OLD.actor_id = NEW.actor_id AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.object_prefix = NEW.object_prefix
  AND OLD.subpath = NEW.subpath AND OLD.object_key = NEW.object_key
  AND OLD.content_type = NEW.content_type
  AND OLD.expected_bytes IS NEW.expected_bytes AND OLD.parts_digest IS NEW.parts_digest
  AND OLD.created_at_ms = NEW.created_at_ms AND OLD.expires_at_ms = NEW.expires_at_ms
  AND NEW.terminal_at_ms IS NULL
) AND NOT (
  OLD.state = 'abort_pending' AND NEW.state = 'aborted'
  AND OLD.external_upload_id = NEW.external_upload_id
  AND OLD.provider_upload_id = NEW.provider_upload_id
  AND OLD.initiate_operation_id = NEW.initiate_operation_id
  AND OLD.completion_operation_id IS NEW.completion_operation_id
  AND OLD.abort_operation_id = NEW.abort_operation_id
  AND OLD.actor_id = NEW.actor_id AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.object_prefix = NEW.object_prefix
  AND OLD.subpath = NEW.subpath AND OLD.object_key = NEW.object_key
  AND OLD.content_type = NEW.content_type
  AND OLD.expected_bytes IS NEW.expected_bytes AND OLD.parts_digest IS NEW.parts_digest
  AND OLD.created_at_ms = NEW.created_at_ms AND OLD.expires_at_ms = NEW.expires_at_ms
  AND NEW.terminal_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_core_storage_multipart_transition_v1');
END;

CREATE TRIGGER legacy_core_storage_multipart_no_delete_v1
BEFORE DELETE ON legacy_core_storage_multipart_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_core_storage_multipart_immutable_v1');
END;

CREATE TABLE legacy_core_storage_multipart_parts_v1 (
  external_upload_id TEXT NOT NULL
    REFERENCES legacy_core_storage_multipart_v1(external_upload_id) ON DELETE RESTRICT,
  part_number INTEGER NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
  provider_etag TEXT NOT NULL CHECK (length(provider_etag) BETWEEN 1 AND 256),
  bytes INTEGER NOT NULL CHECK (bytes BETWEEN 1 AND 9007199254740991),
  completion_operation_id TEXT NOT NULL
    REFERENCES legacy_core_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (external_upload_id, part_number)
);

CREATE TRIGGER legacy_core_storage_multipart_parts_no_update_v1
BEFORE UPDATE ON legacy_core_storage_multipart_parts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_part_immutable_v1'); END;
CREATE TRIGGER legacy_core_storage_multipart_parts_no_delete_v1
BEFORE DELETE ON legacy_core_storage_multipart_parts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_part_immutable_v1'); END;

-- Signed capabilities are governance intents, not proof that bytes exist.
-- Reads verify R2 directly and may promote the intent to observed exactly once.
CREATE TABLE legacy_core_storage_object_intents_v1 (
  intent_id TEXT PRIMARY KEY NOT NULL CHECK (length(intent_id) = 36),
  object_key TEXT NOT NULL CHECK (length(object_key) BETWEEN 5 AND 2048),
  operation_id TEXT NOT NULL
    REFERENCES legacy_core_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  mapped_video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  storage_integration_id TEXT NOT NULL
    REFERENCES storage_integrations(id) ON DELETE RESTRICT,
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 3 AND 127),
  object_role TEXT NOT NULL CHECK (object_role IN (
    'source', 'segment', 'thumbnail', 'preview', 'audio', 'manifest', 'export'
  )),
  method TEXT NOT NULL CHECK (method IN ('post', 'put')),
  state TEXT NOT NULL CHECK (state IN ('capability_issued', 'observed')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  observed_at_ms INTEGER CHECK (
    observed_at_ms IS NULL OR observed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'capability_issued' AND observed_at_ms IS NULL)
    OR (state = 'observed' AND observed_at_ms IS NOT NULL)
  ),
  UNIQUE (operation_id, object_key)
);
CREATE INDEX legacy_core_storage_object_video_state_v1
  ON legacy_core_storage_object_intents_v1(mapped_video_id, state, object_key);
CREATE INDEX legacy_core_storage_object_key_time_v1
  ON legacy_core_storage_object_intents_v1(object_key, created_at_ms, intent_id);

CREATE TRIGGER legacy_core_storage_object_intent_transition_v1
BEFORE UPDATE ON legacy_core_storage_object_intents_v1
WHEN NOT (
  OLD.state = 'capability_issued' AND NEW.state = 'observed'
  AND OLD.intent_id = NEW.intent_id AND OLD.object_key = NEW.object_key
  AND OLD.operation_id = NEW.operation_id
  AND OLD.actor_id = NEW.actor_id AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id = NEW.mapped_video_id AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.storage_integration_id = NEW.storage_integration_id
  AND OLD.content_type = NEW.content_type AND OLD.object_role = NEW.object_role
  AND OLD.method = NEW.method
  AND OLD.created_at_ms = NEW.created_at_ms AND NEW.observed_at_ms IS NOT NULL
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_object_intent_immutable_v1'); END;
CREATE TRIGGER legacy_core_storage_object_intent_no_delete_v1
BEFORE DELETE ON legacy_core_storage_object_intents_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_object_intent_immutable_v1'); END;

CREATE TABLE legacy_core_storage_finalize_intents_v1 (
  mapped_video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_collaboration_video_aliases_v1(legacy_video_id) ON DELETE RESTRICT,
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_core_storage_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  state TEXT NOT NULL CHECK (state IN ('provider_pending', 'complete', 'failed')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  terminal_at_ms INTEGER CHECK (
    terminal_at_ms IS NULL OR terminal_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'provider_pending' AND terminal_at_ms IS NULL)
    OR (state IN ('complete', 'failed') AND terminal_at_ms IS NOT NULL)
  )
);

CREATE TRIGGER legacy_core_storage_finalize_transition_v1
BEFORE UPDATE ON legacy_core_storage_finalize_intents_v1
WHEN NOT (
  OLD.state = 'provider_pending' AND NEW.state IN ('complete', 'failed')
  AND OLD.mapped_video_id = NEW.mapped_video_id AND OLD.legacy_video_id = NEW.legacy_video_id
  AND OLD.operation_id = NEW.operation_id AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.terminal_at_ms IS NOT NULL
)
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_finalize_immutable_v1'); END;
CREATE TRIGGER legacy_core_storage_finalize_no_delete_v1
BEFORE DELETE ON legacy_core_storage_finalize_intents_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_finalize_immutable_v1'); END;

CREATE TABLE legacy_core_storage_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'authority', 'claim', 'multipart_binding', 'parts_binding',
    'provider_pending', 'terminal'
  )),
  expected_count INTEGER NOT NULL,
  actual_count INTEGER NOT NULL,
  PRIMARY KEY (operation_id, assertion_kind)
);

CREATE TRIGGER legacy_core_storage_assertion_guard_v1
BEFORE INSERT ON legacy_core_storage_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN SELECT RAISE(ABORT, 'frame_legacy_core_storage_assertion_v1'); END;
