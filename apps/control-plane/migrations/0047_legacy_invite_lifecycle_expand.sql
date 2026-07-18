PRAGMA foreign_keys = ON;

-- Cap fields used by invite seat allocation do not exist in the retained
-- Frame user row. Nullable subscription state and the non-negative quota are
-- kept losslessly for compatibility reads and writes.
ALTER TABLE users ADD COLUMN legacy_invite_quota INTEGER NOT NULL DEFAULT 1
  CHECK (legacy_invite_quota BETWEEN 0 AND 2147483647);
ALTER TABLE users ADD COLUMN legacy_third_party_stripe_subscription_id TEXT
  CHECK (
    legacy_third_party_stripe_subscription_id IS NULL
    OR length(legacy_third_party_stripe_subscription_id) <= 255
  );

-- Frame stores an invite's email as a digest and maps source NanoIDs to UUIDs.
-- Exact accept/decline requires the source email, source role, and source ID.
-- The row deliberately survives deletion of organization_invites so a resolved
-- decision remains auditable and the source identifier is never lost.
CREATE TABLE legacy_invite_lifecycle_invite_aliases_v1 (
  mapped_invite_id TEXT PRIMARY KEY NOT NULL CHECK (length(mapped_invite_id) = 36),
  legacy_invite_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_invite_id) = 15
    AND legacy_invite_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  invited_email TEXT NOT NULL COLLATE NOCASE CHECK (length(invited_email) BETWEEN 1 AND 255),
  legacy_role TEXT NOT NULL CHECK (length(legacy_role) BETWEEN 1 AND 255),
  decision TEXT NOT NULL DEFAULT 'pending'
    CHECK (decision IN ('pending', 'accepted', 'declined')),
  recorded_at_ms INTEGER NOT NULL
    CHECK (recorded_at_ms BETWEEN 0 AND 9007199254740991),
  resolved_at_ms INTEGER CHECK (
    resolved_at_ms IS NULL OR resolved_at_ms BETWEEN recorded_at_ms AND 9007199254740991
  ),
  last_operation_id TEXT CHECK (
    last_operation_id IS NULL OR length(last_operation_id) = 36
  ),
  CHECK (
    (decision = 'pending' AND resolved_at_ms IS NULL AND last_operation_id IS NULL)
    OR (decision IN ('accepted', 'declined')
      AND resolved_at_ms IS NOT NULL AND last_operation_id IS NOT NULL)
  )
);
CREATE INDEX legacy_invite_lifecycle_pending_email_v1
  ON legacy_invite_lifecycle_invite_aliases_v1(organization_id, invited_email, legacy_invite_id)
  WHERE decision = 'pending';

CREATE TRIGGER legacy_invite_lifecycle_invite_alias_transition_v1
BEFORE UPDATE ON legacy_invite_lifecycle_invite_aliases_v1
WHEN NOT (
  OLD.mapped_invite_id = NEW.mapped_invite_id
  AND OLD.legacy_invite_id = NEW.legacy_invite_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.invited_email COLLATE BINARY = NEW.invited_email COLLATE BINARY
  AND OLD.legacy_role = NEW.legacy_role
  AND OLD.recorded_at_ms = NEW.recorded_at_ms
  AND OLD.decision = 'pending'
  AND NEW.decision IN ('accepted', 'declined')
  AND OLD.resolved_at_ms IS NULL AND NEW.resolved_at_ms IS NOT NULL
  AND OLD.last_operation_id IS NULL AND NEW.last_operation_id IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_invite_alias_immutable_v1');
END;
CREATE TRIGGER legacy_invite_lifecycle_invite_alias_delete_v1
BEFORE DELETE ON legacy_invite_lifecycle_invite_aliases_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_invite_alias_immutable_v1');
END;

-- Cap exposes an independent membership NanoID while Frame keys the retained
-- row by (organization,user). Removal marks the alias instead of deleting it.
CREATE TABLE legacy_invite_lifecycle_member_aliases_v1 (
  mapped_member_id TEXT PRIMARY KEY NOT NULL CHECK (length(mapped_member_id) = 36),
  legacy_member_id TEXT NOT NULL UNIQUE CHECK (
    length(legacy_member_id) = 15
    AND legacy_member_id NOT GLOB '*[^0123456789abcdefghjkmnpqrstvwxyz]*'
  ),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  removed_at_ms INTEGER CHECK (
    removed_at_ms IS NULL OR removed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  created_operation_id TEXT CHECK (
    created_operation_id IS NULL OR length(created_operation_id) = 36
  ),
  removed_operation_id TEXT CHECK (
    removed_operation_id IS NULL OR length(removed_operation_id) = 36
  ),
  CHECK (
    (removed_at_ms IS NULL AND removed_operation_id IS NULL)
    OR (removed_at_ms IS NOT NULL AND removed_operation_id IS NOT NULL)
  )
);
CREATE UNIQUE INDEX legacy_invite_lifecycle_member_active_pair_v1
  ON legacy_invite_lifecycle_member_aliases_v1(organization_id, user_id)
  WHERE removed_at_ms IS NULL;
CREATE TRIGGER legacy_invite_lifecycle_member_alias_transition_v1
BEFORE UPDATE ON legacy_invite_lifecycle_member_aliases_v1
WHEN NOT (
  OLD.mapped_member_id = NEW.mapped_member_id
  AND OLD.legacy_member_id = NEW.legacy_member_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.user_id = NEW.user_id
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.created_operation_id IS NEW.created_operation_id
  AND OLD.removed_at_ms IS NULL AND NEW.removed_at_ms IS NOT NULL
  AND OLD.removed_operation_id IS NULL AND NEW.removed_operation_id IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_invite_member_alias_immutable_v1');
END;
CREATE TRIGGER legacy_invite_lifecycle_member_alias_delete_v1
BEFORE DELETE ON legacy_invite_lifecycle_member_aliases_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_invite_member_alias_immutable_v1');
END;

CREATE TABLE legacy_invite_lifecycle_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  legacy_invite_id TEXT NOT NULL CHECK (length(legacy_invite_id) BETWEEN 1 AND 255),
  action TEXT NOT NULL CHECK (action IN ('accept', 'decline')),
  state TEXT NOT NULL CHECK (state IN ('claimed', 'complete')),
  created_at_ms INTEGER NOT NULL
    CHECK (created_at_ms BETWEEN 0 AND 9007199254740991),
  completed_at_ms INTEGER CHECK (
    completed_at_ms IS NULL OR completed_at_ms BETWEEN created_at_ms AND 9007199254740991
  ),
  CHECK (
    (state = 'claimed' AND completed_at_ms IS NULL)
    OR (state = 'complete' AND completed_at_ms IS NOT NULL)
  )
);
CREATE TRIGGER legacy_invite_lifecycle_operation_transition_v1
BEFORE UPDATE ON legacy_invite_lifecycle_operations_v1
WHEN NOT (
  OLD.operation_id = NEW.operation_id
  AND OLD.actor_id = NEW.actor_id
  AND OLD.organization_id = NEW.organization_id
  AND OLD.legacy_invite_id = NEW.legacy_invite_id
  AND OLD.action = NEW.action
  AND OLD.created_at_ms = NEW.created_at_ms
  AND OLD.state = 'claimed' AND NEW.state = 'complete'
  AND OLD.completed_at_ms IS NULL AND NEW.completed_at_ms IS NOT NULL
)
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_invite_operation_immutable_v1');
END;
CREATE TRIGGER legacy_invite_lifecycle_operation_delete_v1
BEFORE DELETE ON legacy_invite_lifecycle_operations_v1
BEGIN
  SELECT RAISE(ABORT, 'frame_legacy_invite_operation_immutable_v1');
END;

CREATE TABLE legacy_invite_lifecycle_receipts_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_invite_lifecycle_operations_v1(operation_id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN ('accept', 'decline')),
  membership_existed INTEGER NOT NULL CHECK (membership_existed IN (0, 1)),
  membership_created INTEGER NOT NULL CHECK (membership_created IN (0, 1)),
  membership_removed INTEGER NOT NULL CHECK (membership_removed IN (0, 1)),
  pro_seat_assigned INTEGER NOT NULL CHECK (pro_seat_assigned IN (0, 1)),
  inherited_subscription_cleared INTEGER NOT NULL
    CHECK (inherited_subscription_cleared IN (0, 1)),
  fallback_organization_id TEXT REFERENCES organizations(id) ON DELETE RESTRICT,
  completed_at_ms INTEGER NOT NULL
    CHECK (completed_at_ms BETWEEN 0 AND 9007199254740991),
  CHECK (
    (action = 'accept'
      AND membership_created = 1 - membership_existed
      AND membership_removed = 0
      AND inherited_subscription_cleared = 0
      AND fallback_organization_id IS NULL)
    OR (action = 'decline'
      AND membership_created = 0
      AND membership_removed = membership_existed
      AND pro_seat_assigned = 0)
  )
);

CREATE TABLE legacy_invite_lifecycle_audit_events_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL
    REFERENCES legacy_invite_lifecycle_operations_v1(operation_id) ON DELETE RESTRICT,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  action TEXT NOT NULL CHECK (action IN ('accept', 'decline')),
  occurred_at_ms INTEGER NOT NULL
    CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991)
);

-- Computed assertions inserted by the checked-in D1 batch fail the entire
-- transaction whenever a precondition or postcondition is not exactly one.
CREATE TABLE legacy_invite_lifecycle_assertions_v1 (
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  assertion_kind TEXT NOT NULL CHECK (length(assertion_kind) BETWEEN 1 AND 80),
  expected_count INTEGER NOT NULL CHECK (expected_count BETWEEN 0 AND 1000000),
  actual_count INTEGER NOT NULL CHECK (actual_count = expected_count),
  PRIMARY KEY (operation_id, assertion_kind)
);
