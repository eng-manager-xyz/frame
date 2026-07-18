PRAGMA foreign_keys = ON;

-- Expand-only compatibility authority for Cap's six retained comment and
-- reaction mutations. The native comments table cannot represent Cap's
-- untrimmed whitespace, empty-string roots, orphan parents, cross-video
-- parents, or caller-controlled notification cleanup, so those rows remain in
-- a deliberately isolated compatibility table until the cutover contract is
-- retired.

CREATE TABLE legacy_collaboration_user_aliases_v1 (
  legacy_user_id TEXT PRIMARY KEY NOT NULL CHECK (
    length(legacy_user_id) = 15
    AND legacy_user_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  mapped_user_id TEXT NOT NULL UNIQUE REFERENCES users(id) ON DELETE RESTRICT,
  image_url TEXT CHECK (image_url IS NULL OR length(image_url) <= 262144),
  provenance TEXT NOT NULL CHECK (
    provenance IN ('cap_backfill', 'membership_backfill', 'native_generated')
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  refreshed_at_ms INTEGER NOT NULL CHECK (
    refreshed_at_ms BETWEEN created_at_ms AND 9007199254740991
  )
);

CREATE TABLE legacy_collaboration_video_aliases_v1 (
  legacy_video_id TEXT PRIMARY KEY NOT NULL CHECK (
    length(legacy_video_id) = 15
    AND legacy_video_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  mapped_video_id TEXT NOT NULL UNIQUE REFERENCES videos(id) ON DELETE RESTRICT,
  provenance TEXT NOT NULL CHECK (provenance IN ('cap_backfill', 'native_generated')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TABLE legacy_collaboration_comments_v1 (
  legacy_comment_id TEXT PRIMARY KEY NOT NULL CHECK (
    length(legacy_comment_id) = 15
    AND legacy_comment_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  mapped_comment_id TEXT NOT NULL UNIQUE CHECK (length(mapped_comment_id) = 36),
  legacy_video_id TEXT NOT NULL CHECK (length(legacy_video_id) BETWEEN 1 AND 262144),
  mapped_video_id TEXT REFERENCES videos(id) ON DELETE RESTRICT,
  author_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  legacy_author_id TEXT NOT NULL CHECK (
    length(legacy_author_id) = 15
    AND legacy_author_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  comment_kind TEXT NOT NULL CHECK (comment_kind IN ('text', 'emoji')),
  content TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 262144),
  source_timestamp REAL CHECK (
    source_timestamp IS NULL
    OR source_timestamp BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  legacy_parent_comment_id TEXT CHECK (
    legacy_parent_comment_id IS NULL OR length(legacy_parent_comment_id) <= 262144
  ),
  notification_kind TEXT NOT NULL CHECK (notification_kind IN ('comment', 'reply', 'reaction')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (
    updated_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  source_action TEXT NOT NULL CHECK (source_action IN (
    'legacy.collaboration.mobile_create_comment',
    'legacy.collaboration.mobile_create_reaction',
    'legacy.collaboration.web_new_comment_action',
    'legacy.collaboration.cap_backfill'
  )),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36)
);
CREATE INDEX legacy_collaboration_comments_video_time_v1
  ON legacy_collaboration_comments_v1(legacy_video_id, created_at_ms, legacy_comment_id);
CREATE INDEX legacy_collaboration_comments_parent_author_v1
  ON legacy_collaboration_comments_v1(legacy_parent_comment_id, author_user_id, legacy_comment_id);
CREATE INDEX legacy_collaboration_comments_author_v1
  ON legacy_collaboration_comments_v1(author_user_id, legacy_comment_id);

CREATE TRIGGER legacy_collaboration_comments_no_update_v1
BEFORE UPDATE ON legacy_collaboration_comments_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_comment_immutable_v1');
END;

CREATE TABLE legacy_collaboration_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN (
    'legacy.collaboration.mobile_create_comment',
    'legacy.collaboration.mobile_create_reaction',
    'legacy.collaboration.mobile_delete_comment',
    'legacy.collaboration.web_delete_comment_route',
    'legacy.collaboration.web_delete_comment_action',
    'legacy.collaboration.web_new_comment_action'
  )),
  idempotency_key_digest TEXT NOT NULL CHECK (
    length(idempotency_key_digest) = 64
    AND idempotency_key_digest NOT GLOB '*[^0-9a-f]*'
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'complete')),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  ),
  UNIQUE (organization_id, actor_id, action, idempotency_key_digest)
);
CREATE INDEX legacy_collaboration_operations_actor_time_v1
  ON legacy_collaboration_operations_v1(actor_id, created_at_ms DESC);

CREATE TRIGGER legacy_collaboration_operations_transition_v1
BEFORE UPDATE ON legacy_collaboration_operations_v1
WHEN NOT (
  OLD.state = 'claimed' AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.action = NEW.action
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_operation_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_operations_no_delete_v1
BEFORE DELETE ON legacy_collaboration_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_operation_immutable_v1');
END;

-- The exact target set is copied before physical deletion. Web route deletion
-- includes only direct replies by the same author; mobile and the server action
-- contain exactly the authored target.
CREATE TABLE legacy_collaboration_delete_targets_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_collaboration_operations_v1(operation_id) ON DELETE RESTRICT,
  legacy_comment_id TEXT NOT NULL CHECK (length(legacy_comment_id) = 15),
  target_role TEXT NOT NULL CHECK (target_role IN ('target', 'authored_direct_reply')),
  ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 100000),
  PRIMARY KEY (operation_id, legacy_comment_id),
  UNIQUE (operation_id, ordinal)
);

-- Notification rows selected by the caller-supplied parentId are staged before
-- deletion. There is intentionally no FK to notifications because the source
-- transaction physically removes those rows.
CREATE TABLE legacy_collaboration_notification_targets_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_collaboration_operations_v1(operation_id) ON DELETE RESTRICT,
  notification_id TEXT NOT NULL CHECK (length(notification_id) >= 1),
  notification_type TEXT NOT NULL CHECK (notification_type IN ('comment', 'reply')),
  PRIMARY KEY (operation_id, notification_id)
);

CREATE TABLE legacy_collaboration_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_collaboration_operations_v1(operation_id) ON DELETE RESTRICT,
  result_kind TEXT NOT NULL CHECK (result_kind IN ('created', 'deleted')),
  legacy_comment_id TEXT NOT NULL CHECK (length(legacy_comment_id) BETWEEN 0 AND 262144),
  legacy_video_id TEXT CHECK (
    legacy_video_id IS NULL OR length(legacy_video_id) BETWEEN 1 AND 262144
  ),
  legacy_author_id TEXT CHECK (legacy_author_id IS NULL OR length(legacy_author_id) = 15),
  author_name TEXT,
  author_image TEXT CHECK (author_image IS NULL OR length(author_image) <= 262144),
  comment_kind TEXT CHECK (comment_kind IS NULL OR comment_kind IN ('text', 'emoji')),
  content TEXT CHECK (content IS NULL OR length(content) BETWEEN 1 AND 262144),
  source_timestamp REAL CHECK (
    source_timestamp IS NULL
    OR source_timestamp BETWEEN -1.7976931348623157e308 AND 1.7976931348623157e308
  ),
  legacy_parent_comment_id TEXT CHECK (
    legacy_parent_comment_id IS NULL OR length(legacy_parent_comment_id) <= 262144
  ),
  created_comment_at_ms INTEGER CHECK (
    created_comment_at_ms IS NULL
    OR created_comment_at_ms BETWEEN 0 AND 9007199254740991
  ),
  updated_comment_at_ms INTEGER CHECK (
    updated_comment_at_ms IS NULL
    OR updated_comment_at_ms BETWEEN 0 AND 9007199254740991
  ),
  notification_kind TEXT CHECK (
    notification_kind IS NULL OR notification_kind IN ('comment', 'reply', 'reaction')
  ),
  deleted_comment_count INTEGER NOT NULL CHECK (deleted_comment_count BETWEEN 0 AND 100000),
  deleted_notification_count INTEGER NOT NULL CHECK (
    deleted_notification_count BETWEEN 0 AND 100000
  ),
  notification_selector TEXT CHECK (
    notification_selector IS NULL
    OR notification_selector IN ('reply_by_comment_id', 'root_comment_and_replies_by_parent_id')
  ),
  revalidation_path TEXT NOT NULL CHECK (length(revalidation_path) <= 262144),
  recorded_at_ms INTEGER NOT NULL CHECK (recorded_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (
      result_kind = 'created'
      AND legacy_comment_id <> ''
      AND legacy_video_id IS NOT NULL AND legacy_author_id IS NOT NULL
      AND comment_kind IS NOT NULL AND content IS NOT NULL
      AND created_comment_at_ms IS NOT NULL AND updated_comment_at_ms IS NOT NULL
      AND notification_kind IS NOT NULL
      AND deleted_comment_count = 0 AND deleted_notification_count = 0
      AND notification_selector IS NULL
    )
    OR (
      result_kind = 'deleted'
      AND legacy_video_id IS NULL AND legacy_author_id IS NULL
      AND author_name IS NULL AND author_image IS NULL
      AND comment_kind IS NULL AND content IS NULL AND source_timestamp IS NULL
      AND legacy_parent_comment_id IS NULL
      AND created_comment_at_ms IS NULL AND updated_comment_at_ms IS NULL
      AND notification_kind IS NULL
      AND deleted_comment_count BETWEEN 1 AND 100000
    )
  )
);

CREATE TABLE legacy_collaboration_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_collaboration_operations_v1(operation_id) ON DELETE RESTRICT,
  notification_timing TEXT NOT NULL CHECK (notification_timing IN (
    'after_insert_best_effort', 'none', 'same_delete_transaction'
  )),
  notification_failure_rolls_back_core INTEGER NOT NULL CHECK (
    notification_failure_rolls_back_core IN (0, 1)
  ),
  revalidation_path TEXT NOT NULL CHECK (length(revalidation_path) <= 262144),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (notification_timing = 'after_insert_best_effort'
      AND notification_failure_rolls_back_core = 0)
    OR (notification_timing = 'none' AND notification_failure_rolls_back_core = 0)
    OR (notification_timing = 'same_delete_transaction'
      AND notification_failure_rolls_back_core = 1)
  )
);

CREATE TABLE legacy_collaboration_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_collaboration_operations_v1(operation_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome = 'allow'),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

-- This row is written only after the core create transaction has committed.
-- Any provider/handoff failure is swallowed, exactly like Cap, and therefore
-- cannot roll back the comment or its durable receipt.
CREATE TABLE legacy_collaboration_notification_attempts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_collaboration_operations_v1(operation_id) ON DELETE RESTRICT,
  legacy_comment_id TEXT NOT NULL CHECK (length(legacy_comment_id) = 15),
  notification_kind TEXT NOT NULL CHECK (notification_kind IN ('comment', 'reply', 'reaction')),
  outcome TEXT NOT NULL CHECK (outcome = 'handoff_queued'),
  attempted_at_ms INTEGER NOT NULL CHECK (attempted_at_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER legacy_collaboration_notification_attempt_complete_only_v1
BEFORE INSERT ON legacy_collaboration_notification_attempts_v1
WHEN NOT EXISTS (
  SELECT 1 FROM legacy_collaboration_operations_v1 operation
  WHERE operation.operation_id = NEW.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_notification_timing_v1');
END;

CREATE TABLE legacy_collaboration_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'tenant_authority', 'video_authority', 'user_alias', 'authored_target',
    'delete_bound', 'notification_bound',
    'comment_inserted', 'comments_deleted', 'notifications_deleted',
    'receipt_inserted', 'effect_inserted', 'audit_inserted',
    'operation_complete', 'durable_receipt'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_collaboration_authority_assertion_v1
BEFORE INSERT ON legacy_collaboration_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN ('tenant_authority', 'video_authority')
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_authority_v1');
END;
CREATE TRIGGER legacy_collaboration_target_assertion_v1
BEFORE INSERT ON legacy_collaboration_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind = 'authored_target'
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_target_v1');
END;
CREATE TRIGGER legacy_collaboration_corrupt_assertion_v1
BEFORE INSERT ON legacy_collaboration_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'user_alias', 'delete_bound', 'notification_bound',
    'comment_inserted', 'comments_deleted',
    'notifications_deleted', 'receipt_inserted', 'effect_inserted',
    'audit_inserted', 'operation_complete', 'durable_receipt'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_corrupt_v1');
END;

CREATE TRIGGER legacy_collaboration_receipts_no_update_v1
BEFORE UPDATE ON legacy_collaboration_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_receipts_no_delete_v1
BEFORE DELETE ON legacy_collaboration_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_effects_no_update_v1
BEFORE UPDATE ON legacy_collaboration_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_effects_no_delete_v1
BEFORE DELETE ON legacy_collaboration_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_audit_no_update_v1
BEFORE UPDATE ON legacy_collaboration_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_audit_no_delete_v1
BEFORE DELETE ON legacy_collaboration_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_delete_targets_complete_no_update_v1
BEFORE UPDATE ON legacy_collaboration_delete_targets_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_collaboration_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_delete_targets_complete_no_delete_v1
BEFORE DELETE ON legacy_collaboration_delete_targets_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_collaboration_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_notification_targets_complete_no_update_v1
BEFORE UPDATE ON legacy_collaboration_notification_targets_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_collaboration_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_collaboration_notification_targets_complete_no_delete_v1
BEFORE DELETE ON legacy_collaboration_notification_targets_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_collaboration_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_collaboration_receipt_immutable_v1');
END;
