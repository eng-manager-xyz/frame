PRAGMA foreign_keys = ON;

-- Exact compatibility storage for Cap's retained video-property operations.
-- Native Frame privacy remains independent while the legacy boolean is
-- shadowed explicitly, avoiding a lossy public/unlisted/private collapse.
ALTER TABLE videos ADD COLUMN legacy_public INTEGER NOT NULL DEFAULT 1
  CHECK (legacy_public IN (0, 1));
ALTER TABLE videos ADD COLUMN legacy_password_hash TEXT
  CHECK (legacy_password_hash IS NULL OR (
    length(legacy_password_hash) = 64
    AND legacy_password_hash NOT GLOB '*[^A-Za-z0-9+/=]*'
  ));
ALTER TABLE videos ADD COLUMN legacy_settings_json TEXT
  CHECK (legacy_settings_json IS NULL OR json_valid(legacy_settings_json));
-- Cap accepts any truthy JSON metadata value. Native Frame metadata is a
-- checksummed schema-versioned document, so sharing that column would either
-- reject exact Cap scalars/arrays or overwrite native concurrent edits.
ALTER TABLE videos ADD COLUMN legacy_metadata_json TEXT
  CHECK (legacy_metadata_json IS NULL OR json_valid(legacy_metadata_json));
ALTER TABLE videos ADD COLUMN legacy_property_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_property_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE videos ADD COLUMN legacy_property_last_operation_id TEXT
  CHECK (legacy_property_last_operation_id IS NULL OR length(legacy_property_last_operation_id) = 36);

UPDATE videos
SET legacy_public = CASE WHEN privacy = 'public' THEN 1 ELSE 0 END,
    legacy_metadata_json = metadata_json;

-- Cap's anonymous verifier considers the video hash first, followed by every
-- joined space hash. The space revision makes that ordered snapshot assertable
-- in the same transaction that records verification evidence.
ALTER TABLE spaces ADD COLUMN legacy_password_hash TEXT
  CHECK (legacy_password_hash IS NULL OR (
    length(legacy_password_hash) = 64
    AND legacy_password_hash NOT GLOB '*[^A-Za-z0-9+/=]*'
  ));
ALTER TABLE spaces ADD COLUMN legacy_password_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_password_revision BETWEEN 0 AND 9007199254740991);

CREATE INDEX videos_legacy_property_owner_v1
  ON videos(id, owner_id, deleted_at_ms, legacy_property_revision);
CREATE INDEX spaces_legacy_password_v1
  ON spaces(id, legacy_password_revision) WHERE legacy_password_hash IS NOT NULL;

CREATE TABLE legacy_video_property_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-2cfe7fc40a6f5a78', 'cap-v1-5fdf332d1448aedc',
    'cap-v1-b2db0e7ec51f7898', 'cap-v1-5b36dac105856ede',
    'cap-v1-96c52e9330f9a131', 'cap-v1-6e9f3d370f1ce239',
    'cap-v1-ab11637faa2de45e', 'cap-v1-455e6a1b82e647d9',
    'cap-v1-0a2c44d7a626a1fe', 'cap-v1-49dba3fbc7c4a74c'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'mobile_password', 'mobile_sharing', 'mobile_title', 'metadata_put',
    'edit_date', 'edit_title', 'remove_password', 'set_password',
    'verify_password', 'update_settings'
  )),
  principal_digest TEXT NOT NULL CHECK (
    length(principal_digest) = 64 AND principal_digest NOT GLOB '*[^0-9a-f]*'
  ),
  video_id TEXT NOT NULL,
  legacy_video_id_digest TEXT NOT NULL CHECK (
    length(legacy_video_id_digest) = 64 AND legacy_video_id_digest NOT GLOB '*[^0-9a-f]*'
  ),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64 AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (source_operation_id, principal_digest, video_id, idempotency_key_digest),
  CHECK ((state = 'claimed' AND completed_at_ms IS NULL)
      OR (state = 'complete' AND completed_at_ms IS NOT NULL))
);

CREATE TABLE legacy_video_property_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_video_property_operations_v1(operation_id) ON DELETE RESTRICT,
  result_kind TEXT NOT NULL CHECK (result_kind IN (
    'mobile_summary', 'json_true', 'success', 'password_set',
    'password_removed', 'password_verified', 'password_rejected'
  )),
  result_json TEXT CHECK (result_json IS NULL OR (
    json_valid(result_json) AND length(result_json) <= 262144
  )),
  result_digest TEXT NOT NULL CHECK (
    length(result_digest) = 64 AND result_digest NOT GLOB '*[^0-9a-f]*'
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_video_property_effects_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_video_property_operations_v1(operation_id) ON DELETE RESTRICT,
  effect_kind TEXT NOT NULL CHECK (effect_kind IN ('revalidation', 'password_cookie')),
  effect_json TEXT NOT NULL CHECK (json_valid(effect_json) AND length(effect_json) <= 4096),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, effect_kind)
);

CREATE TABLE legacy_video_property_audit_v1 (
  audit_id TEXT PRIMARY KEY NOT NULL CHECK (length(audit_id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_video_property_operations_v1(operation_id) ON DELETE RESTRICT,
  source_operation_id TEXT NOT NULL,
  principal_digest TEXT NOT NULL CHECK (length(principal_digest) = 64),
  video_id_digest TEXT NOT NULL CHECK (length(video_id_digest) = 64),
  request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
  result_digest TEXT NOT NULL CHECK (length(result_digest) = 64),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_video_property_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'operation', 'owner', 'verification', 'mutation', 'durable'
  )),
  expected_count INTEGER NOT NULL,
  actual_count INTEGER NOT NULL,
  PRIMARY KEY (operation_id, assertion_kind)
);

CREATE TRIGGER legacy_video_property_assertions_guard_v1
BEFORE INSERT ON legacy_video_property_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_property_assertion_v1');
END;

CREATE TRIGGER legacy_video_property_operations_no_update_v1
BEFORE UPDATE ON legacy_video_property_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.principal_digest = NEW.principal_digest
  AND OLD.video_id = NEW.video_id
  AND OLD.legacy_video_id_digest = NEW.legacy_video_id_digest
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.completed_at_ms IS NULL AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1');
END;

CREATE TRIGGER legacy_video_property_operations_no_delete_v1
BEFORE DELETE ON legacy_video_property_operations_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_video_property_receipts_no_update_v1
BEFORE UPDATE ON legacy_video_property_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_video_property_receipts_no_delete_v1
BEFORE DELETE ON legacy_video_property_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_video_property_effects_no_update_v1
BEFORE UPDATE ON legacy_video_property_effects_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_video_property_effects_no_delete_v1
BEFORE DELETE ON legacy_video_property_effects_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_video_property_audit_no_update_v1
BEFORE UPDATE ON legacy_video_property_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_video_property_audit_no_delete_v1
BEFORE DELETE ON legacy_video_property_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_video_property_evidence_immutable_v1'); END;
