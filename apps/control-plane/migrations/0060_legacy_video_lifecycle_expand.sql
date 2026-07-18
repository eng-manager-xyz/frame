PRAGMA foreign_keys = ON;

-- Effect RPC supplies a request identifier for protocol correlation, not a
-- business idempotency key.  Persist every mutation before its first external
-- R2 effect so an interrupted delete/copy can be resumed without widening the
-- authorized video prefix.  Reusing an RPC request id with different bytes is
-- rejected; a genuinely new request retains Cap's non-idempotent semantics.
CREATE TABLE legacy_video_lifecycle_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  source_operation_id TEXT NOT NULL CHECK (source_operation_id IN (
    'cap-v1-ac0d7aa564f2991c',
    'cap-v1-e32af2138aa62c8d',
    'cap-v1-1e909cc023a9c4a7',
    'cap-v1-e6a882aeeffaa4f6',
    'cap-v1-7b4e8210491e549d'
  )),
  action TEXT NOT NULL CHECK (action IN (
    'delete_route', 'organisation_update', 'video_delete',
    'video_duplicate', 'video_instant_create'
  )),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  mapped_video_id TEXT REFERENCES videos(id) ON DELETE RESTRICT,
  legacy_video_id TEXT CHECK (
    legacy_video_id IS NULL OR (
      length(legacy_video_id) = 15
      AND legacy_video_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  request_key_digest TEXT NOT NULL CHECK (
    length(request_key_digest) = 64 AND request_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  destination_mapped_video_id TEXT CHECK (
    destination_mapped_video_id IS NULL OR length(destination_mapped_video_id) = 36
  ),
  destination_legacy_video_id TEXT CHECK (
    destination_legacy_video_id IS NULL OR (
      length(destination_legacy_video_id) = 15
      AND destination_legacy_video_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
    )
  ),
  source_prefix TEXT CHECK (source_prefix IS NULL OR (
    length(source_prefix) BETWEEN 3 AND 512 AND substr(source_prefix, -1, 1) = '/'
  )),
  destination_prefix TEXT CHECK (destination_prefix IS NULL OR (
    length(destination_prefix) BETWEEN 3 AND 512 AND substr(destination_prefix, -1, 1) = '/'
  )),
  result_json TEXT CHECK (
    result_json IS NULL OR (json_valid(result_json) AND length(result_json) <= 1048576)
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'storage_pending', 'complete', 'failed')),
  failure_code TEXT CHECK (failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 64),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state IN ('claimed', 'storage_pending') AND completed_at_ms IS NULL AND failure_code IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL AND failure_code IS NULL)
    OR (state = 'failed' AND completed_at_ms IS NOT NULL AND failure_code IS NOT NULL)
  ),
  UNIQUE (source_operation_id, actor_id, request_key_digest)
);
CREATE INDEX legacy_video_lifecycle_video_state_v1
  ON legacy_video_lifecycle_operations_v1(mapped_video_id, state, created_at_ms);

CREATE TRIGGER legacy_video_lifecycle_operation_transition_v1
BEFORE UPDATE ON legacy_video_lifecycle_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state IN ('storage_pending', 'complete', 'failed')
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.action = NEW.action AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id IS NEW.mapped_video_id
  AND OLD.legacy_video_id IS NEW.legacy_video_id
  AND OLD.request_key_digest = NEW.request_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.destination_mapped_video_id IS NEW.destination_mapped_video_id
  AND OLD.destination_legacy_video_id IS NEW.destination_legacy_video_id
  AND OLD.source_prefix IS NEW.source_prefix
  AND OLD.destination_prefix IS NEW.destination_prefix
  AND OLD.created_at_ms = NEW.created_at_ms
  AND (OLD.result_json IS NULL OR OLD.result_json = NEW.result_json)
) AND NOT (
  OLD.state = 'storage_pending' AND NEW.state IN ('complete', 'failed')
  AND OLD.operation_id = NEW.operation_id
  AND OLD.source_operation_id = NEW.source_operation_id
  AND OLD.action = NEW.action AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.mapped_video_id IS NEW.mapped_video_id
  AND OLD.legacy_video_id IS NEW.legacy_video_id
  AND OLD.request_key_digest = NEW.request_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.destination_mapped_video_id IS NEW.destination_mapped_video_id
  AND OLD.destination_legacy_video_id IS NEW.destination_legacy_video_id
  AND OLD.source_prefix IS NEW.source_prefix
  AND OLD.destination_prefix IS NEW.destination_prefix
  AND OLD.created_at_ms = NEW.created_at_ms
  AND (OLD.result_json IS NULL OR OLD.result_json = NEW.result_json)
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_lifecycle_operation_immutable_v1');
END;

CREATE TRIGGER legacy_video_lifecycle_operation_no_delete_v1
BEFORE DELETE ON legacy_video_lifecycle_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_lifecycle_operation_immutable_v1');
END;

-- A receipt is inserted only after R2 acknowledged the destination object.
-- Listing can therefore restart from page one after a crash and skip objects
-- already proven copied, without buffering a whole recording in memory.
CREATE TABLE legacy_video_lifecycle_copy_receipts_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_video_lifecycle_operations_v1(operation_id) ON DELETE RESTRICT,
  source_key TEXT NOT NULL CHECK (length(source_key) BETWEEN 3 AND 2048),
  destination_key TEXT NOT NULL UNIQUE CHECK (length(destination_key) BETWEEN 3 AND 2048),
  source_version TEXT NOT NULL CHECK (length(source_version) BETWEEN 1 AND 255),
  source_bytes INTEGER NOT NULL CHECK (source_bytes BETWEEN 0 AND 9007199254740991),
  copied_at_ms INTEGER NOT NULL CHECK (copied_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, source_key)
);

CREATE TRIGGER legacy_video_lifecycle_copy_receipt_no_update_v1
BEFORE UPDATE ON legacy_video_lifecycle_copy_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_lifecycle_copy_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_video_lifecycle_copy_receipt_no_delete_v1
BEFORE DELETE ON legacy_video_lifecycle_copy_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_lifecycle_copy_receipt_immutable_v1');
END;

CREATE TABLE legacy_video_lifecycle_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'authority', 'mutation', 'postcondition', 'storage_cleanup', 'copy_closure'
  )),
  expected_count INTEGER NOT NULL,
  actual_count INTEGER NOT NULL,
  PRIMARY KEY (operation_id, assertion_kind)
);

CREATE TRIGGER legacy_video_lifecycle_assertion_guard_v1
BEFORE INSERT ON legacy_video_lifecycle_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_video_lifecycle_assertion_v1');
END;
