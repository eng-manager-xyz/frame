PRAGMA foreign_keys = ON;

-- Lossless desktop branding shadow. Cap's organization metadata is a distinct
-- JSON object and cannot be merged into Frame's native settings document.
-- Uploaded one-MiB logos are retained as data URLs so this compatibility
-- route remains provider-independent and the desktop receives an immediately
-- renderable `iconUrl` without exposing private R2 credentials.
ALTER TABLE organizations ADD COLUMN legacy_desktop_metadata_json TEXT NOT NULL DEFAULT '{}'
  CHECK (
    json_valid(legacy_desktop_metadata_json)
    AND json_type(legacy_desktop_metadata_json) = 'object'
    AND length(legacy_desktop_metadata_json) <= 262144
  );
ALTER TABLE organizations ADD COLUMN legacy_desktop_icon_url TEXT
  CHECK (
    legacy_desktop_icon_url IS NULL
    OR length(legacy_desktop_icon_url) <= 1500000
  );
ALTER TABLE organizations ADD COLUMN legacy_desktop_branding_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_desktop_branding_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE organizations ADD COLUMN legacy_desktop_branding_last_operation_id TEXT
  CHECK (
    legacy_desktop_branding_last_operation_id IS NULL
    OR length(legacy_desktop_branding_last_operation_id) = 36
  );
UPDATE organizations SET legacy_desktop_icon_url = legacy_icon_key
WHERE legacy_icon_key IS NOT NULL;

-- Cap permits personal storage integrations with no organization. Frame's
-- native integration is tenant-required, so retain the personal selection as
-- an explicit actor-owned projection rather than mutating another tenant.
CREATE TABLE legacy_desktop_personal_storage_integrations_v1 (
  integration_id TEXT PRIMARY KEY NOT NULL
    REFERENCES storage_integrations(id) ON DELETE CASCADE,
  owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  provider TEXT NOT NULL CHECK (provider = 'googleDrive'),
  status TEXT NOT NULL CHECK (status IN ('active', 'disconnected', 'error')),
  active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (
    last_operation_id IS NULL OR length(last_operation_id) = 36
  )
);
CREATE INDEX legacy_desktop_personal_storage_owner_v1
  ON legacy_desktop_personal_storage_integrations_v1(
    owner_user_id, provider, status, active DESC, updated_at_ms DESC
  );

-- Cap's progress row has one row per video and source timestamps arbitrate
-- races. Keep that projection independent from Frame's authority-fenced
-- upload lifecycle; deleting a source singlepart progress row must not erase a
-- native finalize intent.
CREATE TABLE legacy_desktop_video_uploads_v1 (
  video_id TEXT PRIMARY KEY NOT NULL REFERENCES videos(id) ON DELETE RESTRICT,
  uploaded REAL NOT NULL CHECK (
    uploaded BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  total REAL NOT NULL CHECK (
    total BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  mode TEXT CHECK (mode IS NULL OR mode IN ('singlepart', 'multipart')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT CHECK (
    last_operation_id IS NULL OR length(last_operation_id) = 36
  )
);
INSERT OR IGNORE INTO legacy_desktop_video_uploads_v1(
  video_id, uploaded, total, updated_at_ms, mode, revision, last_operation_id
)
SELECT
  video_id,
  CAST(received_bytes AS REAL),
  CAST(expected_bytes AS REAL),
  updated_at_ms,
  CASE transfer_mode WHEN 'multipart' THEN 'multipart' ELSE 'singlepart' END,
  revision,
  last_operation_id
FROM video_uploads;

-- Optional client idempotency is durable when supplied. Absent keys are
-- server-generated per execution so released-client retry behaviour remains
-- source-compatible. Every mutation still records an immutable receipt and
-- audit row.
CREATE TABLE legacy_desktop_compatibility_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-cdfdf7db0f5cb243',
    'cap-v1-a77171e54b2ba955',
    'cap-v1-acc98d2d5e8ff345',
    'cap-v1-117b0cb801816693'
  )),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'organization_branding', 'storage_set_active', 'video_delete', 'video_progress'
  )),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  target_id TEXT CHECK (target_id IS NULL OR length(target_id) <= 2048),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'effect_pending', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  UNIQUE (source_operation_id, actor_id, idempotency_key_digest),
  CHECK (
    (state IN ('claimed', 'effect_pending') AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_desktop_compatibility_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_desktop_compatibility_operations_v1(operation_id) ON DELETE RESTRICT,
  status INTEGER NOT NULL CHECK (status BETWEEN 200 AND 599),
  result_kind TEXT NOT NULL CHECK (result_kind IN (
    'organization', 'storage_success', 'json_true'
  )),
  result_json TEXT NOT NULL CHECK (
    json_valid(result_json) AND length(result_json) <= 1600000
  ),
  result_digest TEXT NOT NULL CHECK (
    length(result_digest) = 64 AND result_digest NOT GLOB '*[^0-9a-f]*'
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

-- Provider continuation retained before the video row becomes externally
-- deleted. Known manifest keys and Cap's owner/video prefix are both recorded;
-- an interrupted R2 delete can resume under a supplied idempotency key.
CREATE TABLE legacy_desktop_video_delete_objects_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_desktop_compatibility_operations_v1(operation_id) ON DELETE RESTRICT,
  object_key TEXT NOT NULL CHECK (length(object_key) BETWEEN 1 AND 2048),
  state TEXT NOT NULL CHECK (state IN ('pending', 'deleted')),
  deleted_at_ms INTEGER CHECK (
    deleted_at_ms IS NULL OR deleted_at_ms BETWEEN 0 AND 9007199254740991
  ),
  PRIMARY KEY (operation_id, object_key),
  CHECK (
    (state = 'pending' AND deleted_at_ms IS NULL)
    OR (state = 'deleted' AND deleted_at_ms IS NOT NULL)
  )
);

CREATE TABLE legacy_desktop_compatibility_audit_v1 (
  audit_id TEXT PRIMARY KEY NOT NULL CHECK (length(audit_id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_desktop_compatibility_operations_v1(operation_id) ON DELETE RESTRICT,
  source_operation_id TEXT NOT NULL,
  actor_digest TEXT NOT NULL CHECK (length(actor_digest) = 64),
  target_digest TEXT NOT NULL CHECK (length(target_digest) = 64),
  request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
  result_digest TEXT NOT NULL CHECK (length(result_digest) = 64),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_desktop_compatibility_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'claim', 'authority', 'mutation', 'effect', 'durable'
  )),
  expected_count INTEGER NOT NULL,
  actual_count INTEGER NOT NULL,
  PRIMARY KEY (operation_id, assertion_kind)
);

CREATE TRIGGER legacy_desktop_compatibility_assertion_guard_v1
BEFORE INSERT ON legacy_desktop_compatibility_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_assertion_v1');
END;

CREATE TRIGGER legacy_desktop_compatibility_operations_guard_v1
BEFORE UPDATE ON legacy_desktop_compatibility_operations_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.operation_kind = NEW.operation_kind
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id IS NEW.organization_id
  AND OLD.target_id IS NEW.target_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.completed_at_ms IS NULL
  AND (
    (OLD.state = 'claimed' AND NEW.state = 'effect_pending' AND NEW.completed_at_ms IS NULL)
    OR (OLD.state IN ('claimed','effect_pending') AND NEW.state = 'complete'
        AND NEW.completed_at_ms IS NOT NULL)
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_evidence_immutable_v1');
END;

CREATE TRIGGER legacy_desktop_compatibility_operations_no_delete_v1
BEFORE DELETE ON legacy_desktop_compatibility_operations_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_desktop_compatibility_receipts_no_update_v1
BEFORE UPDATE ON legacy_desktop_compatibility_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_desktop_compatibility_receipts_no_delete_v1
BEFORE DELETE ON legacy_desktop_compatibility_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_desktop_compatibility_audit_no_update_v1
BEFORE UPDATE ON legacy_desktop_compatibility_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_evidence_immutable_v1'); END;
CREATE TRIGGER legacy_desktop_compatibility_audit_no_delete_v1
BEFORE DELETE ON legacy_desktop_compatibility_audit_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_desktop_compatibility_evidence_immutable_v1'); END;
