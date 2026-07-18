PRAGMA foreign_keys = ON;

-- Cap represents three folder namespaces: a user's personal library, the
-- whole-organization library (where spaceId equals organizationId), and a
-- concrete space. Frame's original nullable space_id cannot distinguish the
-- first two, so this migration records that distinction without rewriting any
-- existing row. Existing folders are conservatively classified as personal.
ALTER TABLE folders ADD COLUMN legacy_folder_id TEXT
  CHECK (
    legacy_folder_id IS NULL OR (
      length(legacy_folder_id) = 15
      AND legacy_folder_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  );
-- Exact Cap/MySQL names may be empty. The original Frame column has a
-- length(name) >= 1 CHECK, so compatibility rows retain their exact value
-- here. Current folder projections prefer this column; native rows leave it
-- NULL and continue to use name unchanged.
ALTER TABLE folders ADD COLUMN legacy_name TEXT
  CHECK (legacy_name IS NULL OR length(legacy_name) <= 255);
ALTER TABLE folders ADD COLUMN legacy_color TEXT NOT NULL DEFAULT 'normal'
  CHECK (legacy_color IN ('normal', 'blue', 'red', 'yellow'));
ALTER TABLE folders ADD COLUMN legacy_scope_kind TEXT NOT NULL DEFAULT 'personal'
  CHECK (legacy_scope_kind IN ('personal', 'organization', 'space'));
ALTER TABLE folders ADD COLUMN legacy_scope_id TEXT;

CREATE UNIQUE INDEX folders_legacy_folder_id_v1
  ON folders(legacy_folder_id) WHERE legacy_folder_id IS NOT NULL;
CREATE INDEX folders_legacy_scope_parent_v1
  ON folders(organization_id, legacy_scope_kind, legacy_scope_id, parent_id);

-- New compatibility writes must have a coherent namespace and hierarchy.
-- This deliberately prevents cross-tenant and cross-namespace parent edges.
CREATE TRIGGER legacy_folder_crud_insert_scope_guard_v1
BEFORE INSERT ON folders
WHEN NOT (
  (
    -- Native Frame folders predate the legacy namespace columns. They keep
    -- the migration defaults, but a space folder must still name a live space
    -- in the same tenant.
    NEW.legacy_folder_id IS NULL
    AND NEW.legacy_scope_kind = 'personal'
    AND NEW.legacy_scope_id IS NULL
    AND (
      NEW.space_id IS NULL OR EXISTS (
        SELECT 1 FROM spaces native_space
        WHERE native_space.id = NEW.space_id
          AND native_space.organization_id = NEW.organization_id
          AND native_space.deleted_at_ms IS NULL
      )
    )
  )
  OR (
    NEW.legacy_folder_id IS NOT NULL
    AND (
      (
        NEW.legacy_scope_kind = 'personal'
        AND NEW.legacy_scope_id IS NULL
        AND NEW.space_id IS NULL
      )
      OR (
        NEW.legacy_scope_kind = 'organization'
        AND NEW.legacy_scope_id = NEW.organization_id
        AND NEW.space_id IS NULL
      )
      OR (
        NEW.legacy_scope_kind = 'space'
        AND NEW.legacy_scope_id = NEW.space_id
        AND NEW.space_id IS NOT NULL
        AND EXISTS (
          SELECT 1 FROM spaces s
          WHERE s.id = NEW.space_id
            AND s.organization_id = NEW.organization_id
            AND s.deleted_at_ms IS NULL
        )
      )
    )
  )
)
OR (
  NEW.parent_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM folders parent
    WHERE parent.id = NEW.parent_id
      AND parent.organization_id = NEW.organization_id
      AND parent.deleted_at_ms IS NULL
      AND (
        (
          NEW.legacy_folder_id IS NULL
          AND parent.legacy_folder_id IS NULL
          AND parent.space_id IS NEW.space_id
        )
        OR (
          NEW.legacy_folder_id IS NOT NULL
          AND parent.legacy_scope_kind = NEW.legacy_scope_kind
          AND parent.legacy_scope_id IS NEW.legacy_scope_id
        )
      )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_scope_v1');
END;

CREATE TRIGGER legacy_folder_crud_update_scope_guard_v1
BEFORE UPDATE OF
  organization_id, space_id, parent_id, legacy_folder_id,
  legacy_scope_kind, legacy_scope_id
ON folders
WHEN NOT (
  (
    NEW.legacy_folder_id IS NULL
    AND NEW.legacy_scope_kind = 'personal'
    AND NEW.legacy_scope_id IS NULL
    AND (
      NEW.space_id IS NULL OR EXISTS (
        SELECT 1 FROM spaces native_space
        WHERE native_space.id = NEW.space_id
          AND native_space.organization_id = NEW.organization_id
          AND native_space.deleted_at_ms IS NULL
      )
    )
  )
  OR (
    NEW.legacy_folder_id IS NOT NULL
    AND (
      (
        NEW.legacy_scope_kind = 'personal'
        AND NEW.legacy_scope_id IS NULL
        AND NEW.space_id IS NULL
      )
      OR (
        NEW.legacy_scope_kind = 'organization'
        AND NEW.legacy_scope_id = NEW.organization_id
        AND NEW.space_id IS NULL
      )
      OR (
        NEW.legacy_scope_kind = 'space'
        AND NEW.legacy_scope_id = NEW.space_id
        AND NEW.space_id IS NOT NULL
        AND EXISTS (
          SELECT 1 FROM spaces s
          WHERE s.id = NEW.space_id
            AND s.organization_id = NEW.organization_id
            AND s.deleted_at_ms IS NULL
        )
      )
    )
  )
)
OR (
  NEW.parent_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM folders parent
    WHERE parent.id = NEW.parent_id
      AND parent.organization_id = NEW.organization_id
      AND parent.deleted_at_ms IS NULL
      AND (
        (
          NEW.legacy_folder_id IS NULL
          AND parent.legacy_folder_id IS NULL
          AND parent.space_id IS NEW.space_id
        )
        OR (
          NEW.legacy_folder_id IS NOT NULL
          AND parent.legacy_scope_kind = NEW.legacy_scope_kind
          AND parent.legacy_scope_id IS NEW.legacy_scope_id
        )
      )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_scope_v1');
END;

-- Raw idempotency keys never enter D1. The unique key is scoped by tenant,
-- principal, and exact source operation identity.
CREATE TABLE legacy_folder_crud_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-7160c4389375c682',
    'cap-v1-9e125712cee9ce5a',
    'cap-v1-eea1796482b3af28',
    'cap-v1-a193e9e08b2c3f7d'
  )),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64
    AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  ),
  UNIQUE (organization_id, actor_id, source_operation_id, idempotency_key_digest)
);

CREATE TRIGGER legacy_folder_crud_operation_transition_v1
BEFORE UPDATE ON legacy_folder_crud_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_operation_immutable_v1');
END;

CREATE TRIGGER legacy_folder_crud_operation_delete_v1
BEFORE DELETE ON legacy_folder_crud_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_operation_immutable_v1');
END;

-- A recursive delete stages the exact subtree before the root delete invokes
-- the existing ON DELETE CASCADE hierarchy. There is intentionally no folder
-- foreign key because these rows are retained as durable deletion evidence.
CREATE TABLE legacy_folder_crud_delete_targets_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_folder_crud_operations_v1(operation_id) ON DELETE RESTRICT,
  folder_id TEXT NOT NULL CHECK (length(folder_id) = 36),
  depth INTEGER NOT NULL CHECK (depth BETWEEN 0 AND 32),
  PRIMARY KEY (operation_id, folder_id)
);

CREATE TABLE legacy_folder_crud_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_folder_crud_operations_v1(operation_id) ON DELETE RESTRICT,
  result_kind TEXT NOT NULL CHECK (result_kind IN ('mobile_created', 'rpc_void')),
  mutation_kind TEXT NOT NULL CHECK (mutation_kind IN ('create', 'update', 'delete')),
  folder_id TEXT NOT NULL CHECK (length(folder_id) = 36),
  legacy_folder_id TEXT CHECK (
    legacy_folder_id IS NULL OR (
      length(legacy_folder_id) = 15
      AND legacy_folder_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  name TEXT,
  color TEXT CHECK (color IS NULL OR color IN ('normal', 'blue', 'red', 'yellow')),
  affected_folder_count INTEGER NOT NULL CHECK (affected_folder_count BETWEEN 0 AND 100001),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (
      result_kind = 'mobile_created' AND mutation_kind = 'create'
      AND legacy_folder_id IS NOT NULL AND name IS NOT NULL AND color IS NOT NULL
      AND affected_folder_count = 1
    )
    OR (
      result_kind = 'rpc_void'
      AND legacy_folder_id IS NULL AND name IS NULL AND color IS NULL
    )
  )
);

CREATE TABLE legacy_folder_crud_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_folder_crud_operations_v1(operation_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  mutation_kind TEXT NOT NULL CHECK (mutation_kind IN ('create', 'update', 'delete')),
  scope_kind TEXT NOT NULL CHECK (scope_kind IN ('personal', 'organization', 'space')),
  scope_id TEXT,
  invalidation_json TEXT NOT NULL CHECK (
    json_valid(invalidation_json) AND length(invalidation_json) BETWEEN 2 AND 8192
  ),
  affected_folder_count INTEGER NOT NULL CHECK (affected_folder_count BETWEEN 0 AND 100001),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_folder_crud_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_folder_crud_operations_v1(operation_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  source_operation_id TEXT NOT NULL,
  principal_subject_digest TEXT NOT NULL CHECK (
    length(principal_subject_digest) = 64
    AND principal_subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  mutation_subject_digest TEXT NOT NULL CHECK (
    length(mutation_subject_digest) = 64
    AND mutation_subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome = 'allow'),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

-- Every snapshot and postcondition is rechecked inside the same D1 batch.
-- Typed triggers make authority, target/cycle, and mutation failures stable
-- instead of exposing provider error text.
CREATE TABLE legacy_folder_crud_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'authority', 'scope', 'target', 'parent', 'cycle', 'mutation',
    'receipt', 'effect', 'audit', 'operation_complete', 'durable_receipt'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_folder_crud_authority_assertion_v1
BEFORE INSERT ON legacy_folder_crud_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count AND NEW.assertion_kind = 'authority'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_authority_v1');
END;

CREATE TRIGGER legacy_folder_crud_target_assertion_v1
BEFORE INSERT ON legacy_folder_crud_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN ('scope', 'target')
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_target_v1');
END;

CREATE TRIGGER legacy_folder_crud_parent_assertion_v1
BEFORE INSERT ON legacy_folder_crud_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count AND NEW.assertion_kind = 'parent'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_parent_v1');
END;

CREATE TRIGGER legacy_folder_crud_cycle_assertion_v1
BEFORE INSERT ON legacy_folder_crud_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count AND NEW.assertion_kind = 'cycle'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_cycle_v1');
END;

CREATE TRIGGER legacy_folder_crud_mutation_assertion_v1
BEFORE INSERT ON legacy_folder_crud_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'mutation', 'receipt', 'effect', 'audit', 'operation_complete', 'durable_receipt'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_mutation_v1');
END;

-- Durable evidence is append-only.
CREATE TRIGGER legacy_folder_crud_receipt_update_v1
BEFORE UPDATE ON legacy_folder_crud_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_receipt_delete_v1
BEFORE DELETE ON legacy_folder_crud_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_effect_update_v1
BEFORE UPDATE ON legacy_folder_crud_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_effect_delete_v1
BEFORE DELETE ON legacy_folder_crud_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_audit_update_v1
BEFORE UPDATE ON legacy_folder_crud_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_audit_delete_v1
BEFORE DELETE ON legacy_folder_crud_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_delete_target_update_v1
BEFORE UPDATE ON legacy_folder_crud_delete_targets_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
CREATE TRIGGER legacy_folder_crud_delete_target_delete_v1
BEFORE DELETE ON legacy_folder_crud_delete_targets_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_folder_crud_evidence_immutable_v1');
END;
