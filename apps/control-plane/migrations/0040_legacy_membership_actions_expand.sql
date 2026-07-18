PRAGMA foreign_keys = ON;

-- Exact, tenant-scoped replay journal for the six source-pinned Cap
-- membership mutations. Raw idempotency keys never enter D1.
CREATE TABLE legacy_membership_action_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN (
    'legacy.membership.remove_organization_invite',
    'legacy.membership.add_space_member',
    'legacy.membership.add_space_members',
    'legacy.membership.batch_remove_space_members',
    'legacy.membership.remove_space_member',
    'legacy.membership.set_space_members'
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
    completed_at_ms IS NULL OR completed_at_ms BETWEEN 0 AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  ),
  UNIQUE (organization_id, actor_id, action, idempotency_key_digest)
);
CREATE INDEX legacy_membership_action_operations_actor_time_v1
  ON legacy_membership_action_operations_v1(actor_id, created_at_ms DESC);

CREATE TRIGGER legacy_membership_action_operations_transition_v1
BEFORE UPDATE ON legacy_membership_action_operations_v1
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
  SELECT RAISE(ABORT, 'frame_legacy_membership_operation_immutable_v1');
END;

-- Frame keys memberships by (space,user), while Cap exposed an independent
-- NanoID row key and returned legacy user NanoIDs from bulk add. This alias is
-- therefore required compatibility state, not a display-only projection.
CREATE TABLE legacy_space_member_aliases_v1 (
  mapped_member_id TEXT PRIMARY KEY NOT NULL CHECK (length(mapped_member_id) = 36),
  legacy_member_id TEXT NOT NULL CHECK (length(legacy_member_id) = 15),
  legacy_user_id TEXT NOT NULL CHECK (length(legacy_user_id) = 15),
  space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  removed_at_ms INTEGER CHECK (
    removed_at_ms IS NULL OR removed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  UNIQUE (legacy_member_id),
  CHECK (legacy_member_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'),
  CHECK (legacy_user_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*')
);
CREATE UNIQUE INDEX legacy_space_member_aliases_active_pair_v1
  ON legacy_space_member_aliases_v1(space_id, user_id)
  WHERE removed_at_ms IS NULL;
CREATE INDEX legacy_space_member_aliases_space_active_v1
  ON legacy_space_member_aliases_v1(space_id, user_id, legacy_user_id)
  WHERE removed_at_ms IS NULL;

CREATE TRIGGER legacy_space_member_aliases_transition_v1
BEFORE UPDATE ON legacy_space_member_aliases_v1
WHEN NOT (
  OLD.mapped_member_id = NEW.mapped_member_id
  AND OLD.legacy_member_id = NEW.legacy_member_id
  AND OLD.legacy_user_id = NEW.legacy_user_id
  AND OLD.space_id = NEW.space_id
  AND OLD.user_id = NEW.user_id
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.removed_at_ms IS NULL
  AND NEW.removed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_alias_immutable_v1');
END;
CREATE TRIGGER legacy_space_member_aliases_delete_v1
BEFORE DELETE ON legacy_space_member_aliases_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_alias_immutable_v1');
END;

CREATE TRIGGER legacy_membership_action_operations_delete_v1
BEFORE DELETE ON legacy_membership_action_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_operation_immutable_v1');
END;

-- Final desired members are staged inside the same D1 batch and retained as
-- typed replay evidence. Only exact legacy admin/member roles are representable.
CREATE TABLE legacy_membership_action_final_members_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  legacy_user_id TEXT NOT NULL CHECK (length(legacy_user_id) = 15),
  legacy_member_id TEXT NOT NULL CHECK (length(legacy_member_id) = 15),
  mapped_member_id TEXT NOT NULL CHECK (length(mapped_member_id) = 36),
  role TEXT NOT NULL CHECK (role IN ('manager', 'viewer')),
  ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 500),
  PRIMARY KEY (operation_id, user_id),
  UNIQUE (operation_id, ordinal),
  UNIQUE (operation_id, mapped_member_id),
  CHECK (legacy_user_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'),
  CHECK (legacy_member_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*')
);

-- Bulk replacement snapshots at most 100001 rows. The 100001st row trips a
-- fail-closed assertion before deletion, proving the application bound of 100000.
CREATE TABLE legacy_membership_action_previous_members_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  legacy_user_id TEXT NOT NULL CHECK (length(legacy_user_id) = 15),
  legacy_member_id TEXT NOT NULL CHECK (length(legacy_member_id) = 15),
  mapped_member_id TEXT NOT NULL CHECK (length(mapped_member_id) = 36),
  role TEXT NOT NULL CHECK (role IN ('manager', 'contributor', 'viewer')),
  state TEXT NOT NULL CHECK (state IN ('active', 'suspended', 'removed')),
  revision INTEGER NOT NULL CHECK (revision BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, user_id),
  UNIQUE (operation_id, mapped_member_id),
  CHECK (legacy_user_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'),
  CHECK (legacy_member_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*')
);

-- A compatibility-specific generation makes every affected subject explicit.
-- It is independent of login-session rotation but is committed with revocation
-- of every outstanding browser mutation grant for the same subject set.
CREATE TABLE legacy_membership_authority_generations_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  generation INTEGER NOT NULL CHECK (generation BETWEEN 0 AND 9007199254740991),
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 9007199254740991),
  last_operation_id TEXT NOT NULL CHECK (length(last_operation_id) = 36),
  PRIMARY KEY (organization_id, user_id)
);

CREATE TABLE legacy_membership_action_authority_subjects_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  generation_before INTEGER NOT NULL
    CHECK (generation_before BETWEEN 0 AND 9007199254740990),
  generation_after INTEGER NOT NULL
    CHECK (generation_after = generation_before + 1),
  PRIMARY KEY (operation_id, user_id)
);

-- IDs are copied before revocation and intentionally are not foreign keys to
-- the one-use grant table, whose rows are deleted by the same transaction.
CREATE TABLE legacy_membership_action_revoked_grants_v1 (
  operation_id TEXT NOT NULL
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  mutation_grant_id TEXT NOT NULL CHECK (length(mutation_grant_id) = 36),
  session_id TEXT NOT NULL CHECK (length(session_id) = 36),
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  PRIMARY KEY (operation_id, mutation_grant_id)
);

CREATE TABLE legacy_membership_action_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  result_kind TEXT NOT NULL CHECK (result_kind IN (
    'organization_invite_removed', 'space_member_added', 'space_members_added',
    'space_members_removed', 'space_member_removed', 'space_members_set'
  )),
  invite_id TEXT CHECK (invite_id IS NULL OR length(invite_id) = 36),
  space_id TEXT REFERENCES spaces(id) ON DELETE RESTRICT,
  creator_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
  actor_authority TEXT NOT NULL CHECK (actor_authority IN (
    'organization_owner', 'organization_admin', 'active_organization_member',
    'space_creator', 'space_manager'
  )),
  matching_before INTEGER NOT NULL CHECK (matching_before BETWEEN 0 AND 100000),
  deleted_rows INTEGER NOT NULL CHECK (deleted_rows BETWEEN 0 AND 100000),
  inserted_rows INTEGER NOT NULL CHECK (inserted_rows BETWEEN 0 AND 501),
  matching_after INTEGER NOT NULL CHECK (matching_after BETWEEN 0 AND 100500),
  result_count INTEGER CHECK (result_count IS NULL OR result_count BETWEEN 0 AND 501),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (
      result_kind = 'organization_invite_removed'
      AND invite_id IS NOT NULL AND space_id IS NULL AND creator_id IS NULL
      AND actor_authority IN ('organization_owner', 'organization_admin')
      AND matching_before = 1 AND deleted_rows = 1
      AND inserted_rows = 0 AND matching_after = 0 AND result_count IS NULL
    )
    OR (
      result_kind = 'space_member_added'
      AND invite_id IS NULL AND space_id IS NOT NULL AND creator_id IS NOT NULL
      AND matching_before = 0 AND deleted_rows = 0
      AND inserted_rows = 1 AND matching_after = 1 AND result_count IS NULL
    )
    OR (
      result_kind = 'space_members_added'
      AND invite_id IS NULL AND space_id IS NOT NULL AND creator_id IS NOT NULL
      AND deleted_rows = 0 AND inserted_rows BETWEEN 0 AND 500
      AND matching_after = matching_before + inserted_rows
      AND result_count = inserted_rows
    )
    OR (
      result_kind = 'space_members_removed'
      AND invite_id IS NULL AND inserted_rows = 0 AND matching_after = 0
      AND deleted_rows = matching_before AND matching_before BETWEEN 0 AND 500
      AND result_count BETWEEN 0 AND 500
      AND (
        (matching_before = 0 AND result_count = 0
          AND space_id IS NULL AND creator_id IS NULL)
        OR (matching_before >= 1 AND result_count >= 1
          AND space_id IS NOT NULL AND creator_id IS NOT NULL)
      )
    )
    OR (
      result_kind = 'space_member_removed'
      AND invite_id IS NULL AND space_id IS NOT NULL AND creator_id IS NOT NULL
      AND matching_before = 1 AND deleted_rows = 1
      AND inserted_rows = 0 AND matching_after = 0 AND result_count IS NULL
    )
    OR (
      result_kind = 'space_members_set'
      AND invite_id IS NULL AND space_id IS NOT NULL AND creator_id IS NOT NULL
      AND deleted_rows = matching_before
      AND inserted_rows = matching_after
      AND result_count = matching_after
    )
  )
);

CREATE TABLE legacy_membership_action_effects_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  space_id TEXT REFERENCES spaces(id) ON DELETE RESTRICT,
  invalidates_organization_invites INTEGER NOT NULL
    CHECK (invalidates_organization_invites IN (0, 1)),
  invalidates_space_page INTEGER NOT NULL CHECK (invalidates_space_page IN (0, 1)),
  invalidates_space_members INTEGER NOT NULL CHECK (invalidates_space_members IN (0, 1)),
  bumps_authority_generation INTEGER NOT NULL
    CHECK (bumps_authority_generation IN (0, 1)),
  authority_subject_count INTEGER NOT NULL
    CHECK (authority_subject_count BETWEEN 0 AND 100501),
  revalidation_path TEXT NOT NULL CHECK (length(revalidation_path) BETWEEN 0 AND 255),
  created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (
      invalidates_organization_invites = 1 AND space_id IS NULL
      AND invalidates_space_page = 0 AND invalidates_space_members = 0
      AND bumps_authority_generation = 0 AND authority_subject_count = 0
      AND revalidation_path = '/dashboard/settings/organization'
    )
    OR (
      invalidates_organization_invites = 0 AND space_id IS NOT NULL
      AND invalidates_space_page = 1 AND invalidates_space_members = 1
      AND (
        (bumps_authority_generation = 1 AND authority_subject_count >= 1)
        OR (bumps_authority_generation = 0 AND authority_subject_count = 0)
      )
      AND revalidation_path = '/dashboard/spaces/' || space_id
    )
    OR (
      invalidates_organization_invites = 0 AND space_id IS NULL
      AND invalidates_space_page = 0 AND invalidates_space_members = 0
      AND bumps_authority_generation = 0 AND authority_subject_count = 0
      AND revalidation_path = ''
    )
  )
);

CREATE TABLE legacy_membership_action_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL UNIQUE
    REFERENCES legacy_membership_action_operations_v1(operation_id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
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

-- Every accepted, replayed, conflicting, or rejected attempt consumes its own
-- browser proof. Rejection rows need not reference a durable operation.
CREATE TABLE legacy_membership_action_proof_consumptions_v1 (
  mutation_grant_id TEXT PRIMARY KEY NOT NULL CHECK (length(mutation_grant_id) = 36),
  session_id TEXT NOT NULL CHECK (length(session_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  related_operation_id TEXT CHECK (
    related_operation_id IS NULL OR length(related_operation_id) = 36
  ),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  action TEXT NOT NULL,
  request_digest TEXT NOT NULL CHECK (
    length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (
    outcome IN ('applied', 'replay', 'conflict', 'in_flight', 'rejected')
  ),
  consumed_at_ms INTEGER NOT NULL CHECK (consumed_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX legacy_membership_action_proofs_operation_v1
  ON legacy_membership_action_proof_consumptions_v1(related_operation_id, consumed_at_ms);

-- Checked assertions turn any stale snapshot, partial mutation, or incomplete
-- durable receipt into an abort of the entire D1 batch.
CREATE TABLE legacy_membership_action_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
    'browser_grant', 'grant_consumed', 'action_authority',
    'invite_target', 'target_graph', 'creator_graph', 'add_absent',
    'previous_bound', 'aliases_complete', 'member_targets', 'creator_protected',
    'bulk_add_duplicate', 'mutation_rows', 'members_deleted', 'members_inserted',
    'aliases_removed', 'aliases_inserted', 'alias_postcondition',
    'mutation_postcondition', 'out_of_scope', 'authority_generation',
    'authority_generation_postcondition', 'grant_revoked',
    'grant_revocation_postcondition', 'receipt_inserted', 'effect_inserted',
    'audit_inserted', 'proof_journaled', 'operation_complete', 'durable_receipt'
  )),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 9007199254740991),
  actual_count INTEGER NOT NULL CHECK (actual_count BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (operation_id, assertion_kind),
  CHECK (expected_count = actual_count)
);

CREATE TRIGGER legacy_membership_action_authority_assertion_v1
BEFORE INSERT ON legacy_membership_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN ('browser_grant', 'grant_consumed', 'action_authority')
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_authority_v1');
END;

CREATE TRIGGER legacy_membership_action_target_assertion_v1
BEFORE INSERT ON legacy_membership_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'invite_target', 'target_graph', 'creator_graph', 'member_targets',
    'creator_protected', 'aliases_complete'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_target_v1');
END;

CREATE TRIGGER legacy_membership_action_conflict_assertion_v1
BEFORE INSERT ON legacy_membership_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'add_absent', 'bulk_add_duplicate', 'members_deleted', 'members_inserted'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_conflict_v1');
END;

CREATE TRIGGER legacy_membership_action_corrupt_assertion_v1
BEFORE INSERT ON legacy_membership_action_assertions_v1
WHEN NEW.expected_count <> NEW.actual_count
  AND NEW.assertion_kind IN (
    'previous_bound', 'mutation_postcondition', 'out_of_scope',
    'mutation_rows', 'authority_generation', 'authority_generation_postcondition',
    'aliases_removed', 'aliases_inserted', 'alias_postcondition',
    'grant_revoked', 'grant_revocation_postcondition', 'receipt_inserted',
    'effect_inserted', 'audit_inserted', 'proof_journaled',
    'operation_complete', 'durable_receipt'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_corrupt_v1');
END;

CREATE TRIGGER legacy_membership_action_receipts_immutable_v1
BEFORE UPDATE ON legacy_membership_action_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_receipts_delete_v1
BEFORE DELETE ON legacy_membership_action_receipts_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;

-- Staging rows may change while their operation is claimed (the creator role
-- is deliberately forced with an upsert), but every replay child is immutable
-- once the parent journal entry becomes complete.
CREATE TRIGGER legacy_membership_action_final_members_complete_update_v1
BEFORE UPDATE ON legacy_membership_action_final_members_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_final_members_complete_delete_v1
BEFORE DELETE ON legacy_membership_action_final_members_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;

CREATE TRIGGER legacy_membership_action_previous_members_complete_update_v1
BEFORE UPDATE ON legacy_membership_action_previous_members_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_previous_members_complete_delete_v1
BEFORE DELETE ON legacy_membership_action_previous_members_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;

CREATE TRIGGER legacy_membership_action_authority_subjects_complete_update_v1
BEFORE UPDATE ON legacy_membership_action_authority_subjects_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_authority_subjects_complete_delete_v1
BEFORE DELETE ON legacy_membership_action_authority_subjects_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;

CREATE TRIGGER legacy_membership_action_revoked_grants_complete_update_v1
BEFORE UPDATE ON legacy_membership_action_revoked_grants_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_revoked_grants_complete_delete_v1
BEFORE DELETE ON legacy_membership_action_revoked_grants_v1
WHEN EXISTS (
  SELECT 1 FROM legacy_membership_action_operations_v1 operation
  WHERE operation.operation_id = OLD.operation_id AND operation.state = 'complete'
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;

CREATE TRIGGER legacy_membership_action_effects_update_v1
BEFORE UPDATE ON legacy_membership_action_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_effects_delete_v1
BEFORE DELETE ON legacy_membership_action_effects_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_audit_update_v1
BEFORE UPDATE ON legacy_membership_action_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_audit_delete_v1
BEFORE DELETE ON legacy_membership_action_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_receipt_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_proof_update_v1
BEFORE UPDATE ON legacy_membership_action_proof_consumptions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_proof_immutable_v1');
END;
CREATE TRIGGER legacy_membership_action_proof_delete_v1
BEFORE DELETE ON legacy_membership_action_proof_consumptions_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_membership_proof_immutable_v1');
END;
