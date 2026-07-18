PRAGMA foreign_keys = ON;

-- Expand-only write markers let the notification adapter prove that every
-- scoped row (and no row outside the scope) was changed by one operation.
ALTER TABLE notifications ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
CREATE INDEX notifications_operation_scope_v1
  ON notifications(last_operation_id, organization_id, recipient_user_id);

-- Notification preferences remain actor-global. These fields are deliberately
-- separate from the organization-selection revision added in migration 0010.
ALTER TABLE users ADD COLUMN notification_preferences_revision INTEGER NOT NULL DEFAULT 0
  CHECK (notification_preferences_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE users ADD COLUMN notification_preferences_last_operation_id TEXT
  CHECK (
    notification_preferences_last_operation_id IS NULL
    OR length(notification_preferences_last_operation_id) = 36
  );
CREATE INDEX users_notification_preferences_operation_v1
  ON users(notification_preferences_last_operation_id, id);

-- Mark-as-read is organization-tenant scoped. Preference updates use the actor
-- itself as an actor-global tenant, so retries cannot change meaning merely
-- because the user's selected organization changed between requests.
CREATE TABLE legacy_notification_action_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  tenant_kind TEXT NOT NULL CHECK (tenant_kind IN ('organization', 'actor')),
  tenant_id TEXT NOT NULL CHECK (length(tenant_id) = 36),
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (
    action IN ('legacy.notification.mark_as_read', 'legacy.notification.update_preferences')
  ),
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
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  ),
  CHECK (
    (
      tenant_kind = 'organization'
      AND tenant_id = organization_id
      AND action = 'legacy.notification.mark_as_read'
    )
    OR (
      tenant_kind = 'actor'
      AND tenant_id = actor_id
      AND organization_id IS NULL
      AND action = 'legacy.notification.update_preferences'
    )
  ),
  UNIQUE (tenant_kind, tenant_id, actor_id, action, idempotency_key_digest)
);
CREATE INDEX legacy_notification_action_operations_actor_time_v1
  ON legacy_notification_action_operations_v1(actor_id, created_at_ms DESC);

CREATE TRIGGER legacy_notification_action_operations_transition_v1
BEFORE UPDATE ON legacy_notification_action_operations_v1
WHEN NOT (
  OLD.state = 'claimed'
  AND NEW.state = 'complete'
  AND OLD.operation_id = NEW.operation_id
  AND OLD.tenant_kind = NEW.tenant_kind
  AND OLD.tenant_id = NEW.tenant_id
  AND OLD.organization_id IS NEW.organization_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.action = NEW.action
  AND OLD.idempotency_key_digest = NEW.idempotency_key_digest
  AND OLD.request_digest = NEW.request_digest
  AND OLD.created_at_ms = NEW.created_at_ms
  AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_notification_operation_immutable_v1');
END;

CREATE TRIGGER legacy_notification_action_operations_delete_v1
BEFORE DELETE ON legacy_notification_action_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_notification_operation_immutable_v1');
END;

-- The receipt is column-typed rather than an opaque response blob. It retains
-- only the notification branch and sibling digest; unrelated preferences never
-- enter the replay journal.
CREATE TABLE legacy_notification_action_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_notification_action_operations_v1(operation_id) ON DELETE RESTRICT,
  result_kind TEXT NOT NULL CHECK (result_kind IN ('marked_read', 'preferences_updated')),
  selected_notification_id TEXT
    CHECK (selected_notification_id IS NULL OR length(selected_notification_id) = 36),
  matched_count INTEGER CHECK (
    matched_count IS NULL OR matched_count BETWEEN 0 AND 4294967295
  ),
  read_at_ms INTEGER CHECK (read_at_ms IS NULL OR read_at_ms BETWEEN 0 AND 9007199254740991),
  notifications_json TEXT CHECK (
    notifications_json IS NULL
    OR (
      json_valid(notifications_json)
      AND json_type(notifications_json) = 'object'
      AND length(notifications_json) <= 512
    )
  ),
  preserved_before_sha256 TEXT CHECK (
    preserved_before_sha256 IS NULL
    OR (
      length(preserved_before_sha256) = 64
      AND preserved_before_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  preserved_after_sha256 TEXT CHECK (
    preserved_after_sha256 IS NULL
    OR (
      length(preserved_after_sha256) = 64
      AND preserved_after_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  matching_before INTEGER NOT NULL CHECK (matching_before BETWEEN 0 AND 4294967295),
  updated_rows INTEGER NOT NULL CHECK (updated_rows BETWEEN 0 AND 4294967295),
  matching_after INTEGER NOT NULL CHECK (matching_after BETWEEN 0 AND 4294967295),
  out_of_scope_updated_rows INTEGER NOT NULL
    CHECK (out_of_scope_updated_rows BETWEEN 0 AND 4294967295),
  other_actor_rows_updated INTEGER NOT NULL
    CHECK (other_actor_rows_updated BETWEEN 0 AND 4294967295),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (
      result_kind = 'marked_read'
      AND matched_count IS NOT NULL
      AND read_at_ms IS NOT NULL
      AND notifications_json IS NULL
      AND preserved_before_sha256 IS NULL
      AND preserved_after_sha256 IS NULL
      AND matching_before = matched_count
      AND updated_rows = matched_count
      AND matching_after = matched_count
      AND out_of_scope_updated_rows = 0
      AND other_actor_rows_updated = 0
    )
    OR (
      result_kind = 'preferences_updated'
      AND selected_notification_id IS NULL
      AND matched_count IS NULL
      AND read_at_ms IS NULL
      AND notifications_json IS NOT NULL
      AND preserved_before_sha256 IS NOT NULL
      AND preserved_after_sha256 = preserved_before_sha256
      AND matching_before = 1
      AND updated_rows = 1
      AND matching_after = 1
      AND out_of_scope_updated_rows = 0
      AND other_actor_rows_updated = 0
    )
  )
);

CREATE TABLE legacy_notification_action_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_notification_action_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (
    action IN ('legacy.notification.mark_as_read', 'legacy.notification.update_preferences')
  ),
  value_json TEXT NOT NULL CHECK (
    json_valid(value_json) AND json_type(value_json) = 'object' AND length(value_json) <= 512
  ),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (action = 'legacy.notification.mark_as_read' AND organization_id IS NOT NULL)
    OR (action = 'legacy.notification.update_preferences' AND organization_id IS NULL)
  )
);

CREATE TABLE legacy_notification_action_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL
    REFERENCES legacy_notification_action_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (
    action IN ('legacy.notification.mark_as_read', 'legacy.notification.update_preferences')
  ),
  principal_subject_digest TEXT NOT NULL CHECK (
    length(principal_subject_digest) = 64
    AND principal_subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64 AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome = 'allow'),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (action = 'legacy.notification.mark_as_read' AND organization_id IS NOT NULL)
    OR (action = 'legacy.notification.update_preferences' AND organization_id IS NULL)
  )
);
CREATE UNIQUE INDEX legacy_notification_action_audit_operation_v1
  ON legacy_notification_action_audit_events_v1(operation_id);

-- Every accepted or rejected retry consumes a distinct one-use browser grant.
-- Successful/replayed rows point at the durable operation by value; rejection
-- rows may use an isolated assertion id, so this column is intentionally not a
-- foreign key.
CREATE TABLE legacy_notification_action_proof_consumptions_v1 (
  mutation_grant_id TEXT PRIMARY KEY NOT NULL CHECK (length(mutation_grant_id) = 36),
  session_id TEXT NOT NULL CHECK (length(session_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  related_operation_id TEXT CHECK (
    related_operation_id IS NULL OR length(related_operation_id) = 36
  ),
  tenant_kind TEXT NOT NULL CHECK (tenant_kind IN ('organization', 'actor')),
  tenant_id TEXT NOT NULL CHECK (length(tenant_id) = 36),
  organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (
    action IN ('legacy.notification.mark_as_read', 'legacy.notification.update_preferences')
  ),
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (
    outcome IN ('applied', 'replay', 'conflict', 'in_flight', 'rejected')
  ),
  consumed_at_ms INTEGER NOT NULL CHECK (consumed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (
      tenant_kind = 'organization'
      AND tenant_id = organization_id
      AND action = 'legacy.notification.mark_as_read'
    )
    OR (
      tenant_kind = 'actor'
      AND tenant_id = actor_id
      AND organization_id IS NULL
      AND action = 'legacy.notification.update_preferences'
    )
  )
);
CREATE INDEX legacy_notification_action_proofs_operation_v1
  ON legacy_notification_action_proof_consumptions_v1(related_operation_id, consumed_at_ms);

CREATE TABLE legacy_notification_action_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'browser_grant', 'grant_consumed', 'mark_authority', 'preferences_authority',
    'mark_precondition', 'mark_updated', 'mark_postcondition', 'out_of_scope',
    'preferences_updated', 'preferences_postcondition', 'other_actor',
    'receipt_inserted', 'effect_inserted', 'audit_inserted', 'proof_journaled',
    'operation_complete', 'durable_receipt'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_notification_action_authority_assertion_v1
BEFORE INSERT ON legacy_notification_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'browser_grant', 'grant_consumed', 'mark_authority', 'preferences_authority'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_notification_authority_v1');
END;

CREATE TRIGGER legacy_notification_action_conflict_assertion_v1
BEFORE INSERT ON legacy_notification_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'mark_precondition', 'mark_updated', 'mark_postcondition', 'out_of_scope',
    'preferences_updated', 'preferences_postcondition', 'other_actor'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_notification_conflict_v1');
END;

CREATE TRIGGER legacy_notification_action_corrupt_assertion_v1
BEFORE INSERT ON legacy_notification_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'receipt_inserted', 'effect_inserted', 'audit_inserted', 'proof_journaled',
    'operation_complete', 'durable_receipt'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_notification_corrupt_v1');
END;

-- Durable action records are append-only. The operation transition trigger is
-- the sole exception and permits exactly claimed -> complete.
CREATE TRIGGER legacy_notification_action_receipts_update_v1
BEFORE UPDATE ON legacy_notification_action_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_receipts_delete_v1
BEFORE DELETE ON legacy_notification_action_receipts_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_receipt_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_effects_update_v1
BEFORE UPDATE ON legacy_notification_action_effects_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_effect_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_effects_delete_v1
BEFORE DELETE ON legacy_notification_action_effects_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_effect_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_audit_update_v1
BEFORE UPDATE ON legacy_notification_action_audit_events_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_audit_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_audit_delete_v1
BEFORE DELETE ON legacy_notification_action_audit_events_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_audit_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_proofs_update_v1
BEFORE UPDATE ON legacy_notification_action_proof_consumptions_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_proof_immutable_v1'); END;
CREATE TRIGGER legacy_notification_action_proofs_delete_v1
BEFORE DELETE ON legacy_notification_action_proof_consumptions_v1
BEGIN SELECT RAISE(ABORT, 'frame_legacy_notification_proof_immutable_v1'); END;
