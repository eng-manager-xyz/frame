PRAGMA foreign_keys = ON;

-- Provider-free source fields that are not representable in the normalized
-- organization/space rows. Existing settings_json, membership, storage
-- integration, and alias tables remain the only business authorities.
ALTER TABLE organizations ADD COLUMN legacy_shareable_link_icon_key TEXT
  CHECK (
    legacy_shareable_link_icon_key IS NULL OR (
      length(legacy_shareable_link_icon_key) BETWEEN 1 AND 1024
      AND substr(legacy_shareable_link_icon_key, 1, 1) <> '/'
      AND instr(legacy_shareable_link_icon_key, '..') = 0
    )
  );
ALTER TABLE organizations ADD COLUMN legacy_workos_organization_id TEXT
  CHECK (
    legacy_workos_organization_id IS NULL OR
    length(legacy_workos_organization_id) BETWEEN 1 AND 255
  );
ALTER TABLE organizations ADD COLUMN legacy_workos_connection_id TEXT
  CHECK (
    legacy_workos_connection_id IS NULL OR
    length(legacy_workos_connection_id) BETWEEN 1 AND 255
  );
ALTER TABLE organizations ADD COLUMN legacy_organization_library_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_organization_library_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE organizations ADD COLUMN legacy_organization_library_last_operation_id TEXT
  CHECK (
    legacy_organization_library_last_operation_id IS NULL OR
    length(legacy_organization_library_last_operation_id) = 36
  );

ALTER TABLE spaces ADD COLUMN legacy_icon_key TEXT
  CHECK (
    legacy_icon_key IS NULL OR (
      length(legacy_icon_key) BETWEEN 1 AND 1024
      AND substr(legacy_icon_key, 1, 1) <> '/'
      AND instr(legacy_icon_key, '..') = 0
    )
  );
ALTER TABLE spaces ADD COLUMN legacy_organization_library_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_organization_library_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE spaces ADD COLUMN legacy_organization_library_last_operation_id TEXT
  CHECK (
    legacy_organization_library_last_operation_id IS NULL OR
    length(legacy_organization_library_last_operation_id) = 36
  );

ALTER TABLE folders ADD COLUMN legacy_organization_library_revision INTEGER NOT NULL DEFAULT 0
  CHECK (legacy_organization_library_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE folders ADD COLUMN legacy_organization_library_last_operation_id TEXT
  CHECK (
    legacy_organization_library_last_operation_id IS NULL OR
    length(legacy_organization_library_last_operation_id) = 36
  );

-- One durable replay/state-machine authority for all 20 authenticated actions.
-- Anonymous password verification remains a read plus local cookie projection
-- and is intentionally never persisted with its secret input.
CREATE TABLE legacy_organization_library_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN (
    'set_collection_logo',
    'set_space_collection_visibility',
    'delete_space',
    'get_organization_sso_data',
    'remove_organization_member',
    'update_organization_settings',
    'hide_shareable_link_cap_logo',
    'remove_shareable_link_icon',
    'select_shareable_link_branding_organization',
    'update_shareable_link_icon_preference',
    'upload_shareable_link_icon',
    'connect_organization_google_drive',
    'disconnect_organization_google_drive',
    'get_organization_storage_settings',
    'set_organization_storage_provider',
    'toggle_pro_seat',
    'update_organization_details',
    'update_organization_member_role',
    'upload_space_icon',
    'create_organization'
  )),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64
    AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'storage_pending', 'complete')),
  result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
  effects_json TEXT CHECK (effects_json IS NULL OR json_valid(effects_json)),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR
    completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND result_json IS NULL AND effects_json IS NULL
      AND completed_at_ms IS NULL)
    OR
    (state = 'storage_pending' AND result_json IS NOT NULL AND effects_json IS NOT NULL
      AND completed_at_ms IS NULL AND organization_id IS NOT NULL)
    OR
    (state = 'complete' AND result_json IS NOT NULL AND effects_json IS NOT NULL
      AND completed_at_ms IS NOT NULL AND organization_id IS NOT NULL)
  ),
  UNIQUE (actor_id, action, idempotency_key_digest)
);

CREATE INDEX legacy_organization_library_operations_state_v1
  ON legacy_organization_library_operations_v1(state, updated_at_ms, operation_id);

-- Exact R2 receipts make retry state explicit. A write uses a deterministic key;
-- deletes are recorded before/after execution and can be resumed without ever
-- fabricating a completed D1 receipt.
CREATE TABLE legacy_organization_library_r2_effects_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_organization_library_operations_v1(operation_id) ON DELETE RESTRICT,
  effect_order INTEGER NOT NULL CHECK (effect_order BETWEEN 0 AND 10000),
  effect_kind TEXT NOT NULL CHECK (effect_kind IN ('put', 'delete', 'delete_prefix')),
  object_key TEXT NOT NULL CHECK (
    length(object_key) BETWEEN 1 AND 1024
    AND substr(object_key, 1, 1) <> '/'
    AND instr(object_key, '..') = 0
  ),
  checksum_sha256 TEXT CHECK (
    checksum_sha256 IS NULL OR (
      length(checksum_sha256) = 64
      AND checksum_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  content_type TEXT CHECK (
    content_type IS NULL OR length(content_type) BETWEEN 3 AND 127
  ),
  effect_state TEXT NOT NULL CHECK (effect_state IN ('pending', 'applied')),
  applied_at_ms INTEGER CHECK (
    applied_at_ms IS NULL OR applied_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (effect_state = 'pending' AND applied_at_ms IS NULL)
    OR (effect_state = 'applied' AND applied_at_ms IS NOT NULL)
  ),
  PRIMARY KEY (operation_id, effect_order),
  UNIQUE (operation_id, effect_kind, object_key)
);
CREATE INDEX legacy_organization_library_r2_pending_v1
  ON legacy_organization_library_r2_effects_v1(effect_state, operation_id, effect_order);

CREATE TABLE legacy_organization_library_assertions_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_organization_library_operations_v1(operation_id) ON DELETE RESTRICT,
  assertion_kind TEXT NOT NULL CHECK (length(assertion_kind) BETWEEN 1 AND 64),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 100000),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 100000),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_organization_library_operation_transition_v1
BEFORE UPDATE ON legacy_organization_library_operations_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.action = NEW.action
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND (
    (OLD.state = 'claimed' AND NEW.state IN ('claimed', 'storage_pending', 'complete'))
    OR (OLD.state = 'storage_pending' AND NEW.state IN ('storage_pending', 'complete'))
    OR (OLD.state = 'complete' AND NEW.state = 'complete')
  )
  AND (
    OLD.state <> 'complete'
    OR (
      OLD.organization_id IS NEW.organization_id
      AND OLD.result_json IS NEW.result_json
      AND OLD.effects_json IS NEW.effects_json
      AND OLD.completed_at_ms IS NEW.completed_at_ms
      AND OLD.updated_at_ms = NEW.updated_at_ms
    )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_organization_library_operation_immutable_v1');
END;

CREATE TRIGGER legacy_organization_library_operation_delete_v1
BEFORE DELETE ON legacy_organization_library_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_organization_library_operation_immutable_v1');
END;

CREATE TRIGGER legacy_organization_library_r2_transition_v1
BEFORE UPDATE ON legacy_organization_library_r2_effects_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.effect_order = NEW.effect_order
  AND OLD.effect_kind = NEW.effect_kind
  AND OLD.object_key = NEW.object_key
  AND OLD.checksum_sha256 IS NEW.checksum_sha256
  AND OLD.content_type IS NEW.content_type
  AND (
    (OLD.effect_state = 'pending' AND NEW.effect_state IN ('pending', 'applied'))
    OR (OLD.effect_state = 'applied' AND NEW.effect_state = 'applied')
  )
  AND (OLD.effect_state <> 'applied' OR OLD.applied_at_ms = NEW.applied_at_ms)
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_organization_library_r2_immutable_v1');
END;

CREATE INDEX legacy_organization_library_member_authority_v1
  ON organization_members(organization_id, user_id, state, role, has_pro_seat, revision);
CREATE INDEX legacy_organization_library_space_authority_v1
  ON spaces(organization_id, id, deleted_at_ms, created_by_user_id, revision, authority_version);
CREATE INDEX legacy_organization_library_storage_provider_v1
  ON storage_integrations(organization_id, provider, state, revision, authority_version);
