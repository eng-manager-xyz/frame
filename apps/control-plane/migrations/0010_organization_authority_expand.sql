PRAGMA foreign_keys = ON;

-- Expand-only organization authority metadata. Existing collaboration tables
-- remain the compatibility read source during the shadow period.
ALTER TABLE users ADD COLUMN default_organization_id TEXT;
ALTER TABLE users ADD COLUMN active_organization_id TEXT;
ALTER TABLE users ADD COLUMN organization_preference_revision INTEGER NOT NULL DEFAULT 0
  CHECK (organization_preference_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE users ADD COLUMN organization_last_operation_id TEXT
  CHECK (organization_last_operation_id IS NULL OR length(organization_last_operation_id) = 36);

ALTER TABLE organizations ADD COLUMN authority_version INTEGER NOT NULL DEFAULT 0
  CHECK (authority_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE organizations ADD COLUMN retention_until_ms INTEGER
  CHECK (retention_until_ms IS NULL OR retention_until_ms BETWEEN 0 AND 9007199254740991);
ALTER TABLE organizations ADD COLUMN recovered_at_ms INTEGER
  CHECK (recovered_at_ms IS NULL OR recovered_at_ms BETWEEN 0 AND 9007199254740991);
ALTER TABLE organizations ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE organization_members ADD COLUMN authority_version INTEGER NOT NULL DEFAULT 0
  CHECK (authority_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE organization_members ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE organization_invites ADD COLUMN accepted_by_user_id TEXT REFERENCES users(id) ON DELETE RESTRICT;
ALTER TABLE organization_invites ADD COLUMN invited_email_key_version INTEGER
  CHECK (invited_email_key_version IS NULL OR invited_email_key_version BETWEEN 1 AND 65535);
ALTER TABLE organization_invites ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE organization_allowed_domains ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE organization_allowed_domains ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE spaces ADD COLUMN authority_version INTEGER NOT NULL DEFAULT 0
  CHECK (authority_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE spaces ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE space_members ADD COLUMN state TEXT NOT NULL DEFAULT 'active'
  CHECK (state IN ('active', 'suspended', 'removed'));
ALTER TABLE space_members ADD COLUMN revision INTEGER NOT NULL DEFAULT 0
  CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE space_members ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

ALTER TABLE folders ADD COLUMN depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 32);
ALTER TABLE folders ADD COLUMN tree_revision INTEGER NOT NULL DEFAULT 0
  CHECK (tree_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE folders ADD COLUMN last_operation_id TEXT
  CHECK (last_operation_id IS NULL OR length(last_operation_id) = 36);

-- Do not add the contract-phase unique index yet. The legacy schema permitted
-- more than one active owner, so CREATE UNIQUE INDEX would make this expand
-- migration fail before the audit/repair surface could inspect the tenant.
-- These transition guards preserve the at-most-one invariant for clean/new
-- organizations and prevent a dirty organization from acquiring another
-- active owner. Exact-one remains a repository postcondition; a later contract
-- migration may add the unique index after every dirty tenant is reviewed.
CREATE INDEX organization_members_active_owner_lookup_idx
  ON organization_members(organization_id, user_id)
  WHERE role = 'owner' AND state = 'active';

CREATE TRIGGER organization_members_active_owner_insert_guard_v1
BEFORE INSERT ON organization_members
WHEN NEW.role = 'owner' AND NEW.state = 'active'
  AND EXISTS (
    SELECT 1 FROM organization_members existing
    WHERE existing.organization_id = NEW.organization_id
      AND existing.role = 'owner' AND existing.state = 'active'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_organization_cas_conflict_v1');
END;

CREATE TRIGGER organization_members_active_owner_update_guard_v1
BEFORE UPDATE OF organization_id, role, state ON organization_members
WHEN NEW.role = 'owner' AND NEW.state = 'active'
  AND NOT (
    OLD.organization_id = NEW.organization_id
    AND OLD.role = 'owner' AND OLD.state = 'active'
  )
  AND EXISTS (
    SELECT 1 FROM organization_members existing
    WHERE existing.organization_id = NEW.organization_id
      AND existing.role = 'owner' AND existing.state = 'active'
  )
BEGIN
  SELECT RAISE(ABORT, 'frame_organization_cas_conflict_v1');
END;

CREATE VIEW organization_owner_integrity_v1 AS
SELECT o.id AS organization_id,
       o.owner_id AS pointer_owner_id,
       (
         SELECT COUNT(*) FROM organization_members owners
         WHERE owners.organization_id = o.id
           AND owners.role = 'owner' AND owners.state = 'active'
       ) AS active_owner_count,
       (
         SELECT COUNT(*) FROM organization_members pointer_owner
         WHERE pointer_owner.organization_id = o.id
           AND pointer_owner.user_id = o.owner_id
           AND pointer_owner.role = 'owner' AND pointer_owner.state = 'active'
       ) AS pointer_owner_count
FROM organizations o;
CREATE INDEX organization_members_authority_idx
  ON organization_members(organization_id, user_id, state, role, revision, authority_version);
CREATE INDEX organization_invites_token_state_idx
  ON organization_invites(token_digest, status, expires_at_ms, organization_id);
CREATE INDEX folders_move_fence_idx
  ON folders(organization_id, space_id, parent_id, deleted_at_ms, revision, tree_revision);

-- Closure rows make cycle/cross-space checks part of the write transaction.
CREATE TABLE organization_folder_closure_v1 (
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
  ancestor_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  descendant_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  distance INTEGER NOT NULL CHECK (distance BETWEEN 0 AND 32),
  -- Distance is part of the key so a dirty legacy cycle can retain both the
  -- canonical distance-zero self edge and a nonzero self path for audit.
  PRIMARY KEY (organization_id, space_id, ancestor_id, descendant_id, distance)
);
CREATE INDEX organization_folder_closure_v1_descendant_idx
  ON organization_folder_closure_v1(organization_id, space_id, descendant_id, distance);

INSERT OR IGNORE INTO organization_folder_closure_v1(
  organization_id, space_id, ancestor_id, descendant_id, distance
)
SELECT organization_id, space_id, id, id, 0
FROM folders
WHERE space_id IS NOT NULL;

WITH RECURSIVE legacy_tree(organization_id, space_id, ancestor_id, descendant_id, distance) AS (
  SELECT organization_id, space_id, id, id, 0
  FROM folders
  WHERE space_id IS NOT NULL
  UNION ALL
  SELECT legacy_tree.organization_id,
         legacy_tree.space_id,
         legacy_tree.ancestor_id,
         child.id,
         legacy_tree.distance + 1
  FROM legacy_tree
  JOIN folders child
   ON child.parent_id = legacy_tree.descendant_id
   AND child.organization_id = legacy_tree.organization_id
   AND child.space_id = legacy_tree.space_id
  WHERE legacy_tree.distance < 32
)
INSERT OR IGNORE INTO organization_folder_closure_v1(
  organization_id, space_id, ancestor_id, descendant_id, distance
)
SELECT organization_id, space_id, ancestor_id, descendant_id, distance
FROM legacy_tree;

-- Recompute depth only for active folders reached from a real active root.
-- A parent cycle has no root in this single-parent model, and a missing,
-- deleted, cross-organization, or cross-space parent is deliberately not
-- normalized silently. Those rows retain depth zero and remain audit findings.
WITH RECURSIVE rooted_folders(organization_id, space_id, folder_id, depth) AS (
  SELECT organization_id, space_id, id, 0
  FROM folders
  WHERE space_id IS NOT NULL AND parent_id IS NULL AND deleted_at_ms IS NULL
  UNION ALL
  SELECT child.organization_id, child.space_id, child.id, rooted_folders.depth + 1
  FROM rooted_folders
  JOIN folders child
    ON child.parent_id = rooted_folders.folder_id
   AND child.organization_id = rooted_folders.organization_id
   AND child.space_id = rooted_folders.space_id
   AND child.deleted_at_ms IS NULL
  WHERE rooted_folders.depth < 32
)
UPDATE folders
SET depth = (
  SELECT rooted_folders.depth
  FROM rooted_folders
  WHERE rooted_folders.folder_id = folders.id
)
WHERE id IN (SELECT folder_id FROM rooted_folders);

-- A failed predicate is the only provider error classified as an optimistic
-- conflict. Every write batch inserts one or more assertion rows.
CREATE TABLE organization_repository_assertions_v1 (
  id TEXT PRIMARY KEY NOT NULL,
  satisfied INTEGER NOT NULL CHECK (satisfied = 1)
);

CREATE TRIGGER organization_repository_assertions_v1_conflict
BEFORE INSERT ON organization_repository_assertions_v1
WHEN NEW.satisfied <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_organization_cas_conflict_v1');
END;

CREATE TABLE organization_retention_assertions_v1 (
  id TEXT PRIMARY KEY NOT NULL,
  satisfied INTEGER NOT NULL CHECK (satisfied = 1)
);

CREATE TRIGGER organization_retention_assertions_v1_locked
BEFORE INSERT ON organization_retention_assertions_v1
WHEN NEW.satisfied <> 1
BEGIN
  SELECT RAISE(ABORT, 'frame_organization_retention_locked_v1');
END;

CREATE TABLE organization_repository_operations_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 128),
  operation_kind TEXT NOT NULL CHECK (operation_kind IN (
    'organization_create', 'active_organization_set', 'invite_issue', 'invite_revoke',
    'invite_accept', 'ownership_transfer', 'member_change', 'allowed_domain_upsert',
    'settings_update', 'space_create', 'space_update', 'space_role_change',
    'folder_create', 'folder_update', 'folder_move',
    'organization_tombstone', 'organization_recover'
  )),
  subject_id TEXT NOT NULL CHECK (length(subject_id) BETWEEN 1 AND 255),
  request_fingerprint TEXT NOT NULL CHECK (
    length(request_fingerprint) = 64
    AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  result_code TEXT NOT NULL CHECK (result_code IN (
    'created', 'applied', 'accepted', 'revoked', 'tombstoned', 'recovered', 'unchanged'
  )),
  resulting_revision INTEGER NOT NULL CHECK (resulting_revision BETWEEN 0 AND 9007199254740991),
  authority_version INTEGER NOT NULL CHECK (authority_version BETWEEN 0 AND 9007199254740991),
  committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE (organization_id, idempotency_key)
);
CREATE INDEX organization_repository_operations_v1_subject_idx
  ON organization_repository_operations_v1(organization_id, operation_kind, subject_id);

CREATE TABLE organization_audit_events_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  action TEXT NOT NULL CHECK (length(action) BETWEEN 3 AND 64 AND action NOT GLOB '*[^a-z_]*'),
  subject_kind TEXT NOT NULL CHECK (subject_kind IN (
    'organization', 'membership', 'invite', 'allowed_domain', 'settings', 'seat',
    'space', 'folder', 'tombstone', 'repair_plan'
  )),
  subject_digest TEXT NOT NULL CHECK (
    length(subject_digest) = 64 AND subject_digest NOT GLOB '*[^0-9a-f]*'
  ),
  outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'error')),
  denial_code TEXT CHECK (
    denial_code IS NULL OR denial_code IN (
      'organization_access_denied', 'organization_authority_inactive',
      'organization_state_denied', 'organization_authority_stale',
      'organization_retention_locked'
    )
  ),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  metadata_json TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(metadata_json) AND length(metadata_json) <= 4096),
  CHECK ((outcome = 'deny') = (denial_code IS NOT NULL))
);
CREATE INDEX organization_audit_events_v1_org_time_idx
  ON organization_audit_events_v1(organization_id, occurred_at_ms DESC);
CREATE INDEX organization_audit_events_v1_operation_idx
  ON organization_audit_events_v1(operation_id);

CREATE TRIGGER organization_audit_events_v1_immutable_update
BEFORE UPDATE ON organization_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'organization audit is append-only');
END;

CREATE TRIGGER organization_audit_events_v1_immutable_delete
BEFORE DELETE ON organization_audit_events_v1
BEGIN
  SELECT RAISE(ABORT, 'organization audit is append-only');
END;

CREATE TABLE organization_support_authorities_v1 (
  support_actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  ticket_digest TEXT NOT NULL CHECK (
    length(ticket_digest) = 64 AND ticket_digest NOT GLOB '*[^0-9a-f]*'
  ),
  issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK (revoked_at_ms IS NULL OR revoked_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY (support_actor_id, organization_id, ticket_digest),
  CHECK (expires_at_ms > issued_at_ms)
);

CREATE TABLE organization_repair_plans_v1 (
  id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  generated_by_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  support_authority_fingerprint TEXT NOT NULL CHECK (
    length(support_authority_fingerprint) = 64
    AND support_authority_fingerprint NOT GLOB '*[^0-9a-f]*'
  ),
  findings_json TEXT NOT NULL CHECK (json_valid(findings_json) AND length(findings_json) <= 1048576),
  steps_json TEXT NOT NULL CHECK (json_valid(steps_json) AND length(steps_json) <= 1048576),
  dry_run INTEGER NOT NULL CHECK (dry_run = 1),
  generated_at_ms INTEGER NOT NULL CHECK (generated_at_ms BETWEEN 0 AND 9007199254740991)
);
CREATE INDEX organization_repair_plans_v1_org_time_idx
  ON organization_repair_plans_v1(organization_id, generated_at_ms DESC);

CREATE TABLE organization_tombstone_events_v1 (
  operation_id TEXT PRIMARY KEY NOT NULL CHECK (length(operation_id) = 36),
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  actor_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  event_kind TEXT NOT NULL CHECK (event_kind IN ('tombstoned', 'recovered')),
  occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms BETWEEN 0 AND 9007199254740991),
  retention_until_ms INTEGER CHECK (retention_until_ms IS NULL OR retention_until_ms BETWEEN 0 AND 9007199254740991)
);

CREATE TRIGGER organization_tombstone_events_v1_immutable_update
BEFORE UPDATE ON organization_tombstone_events_v1
BEGIN
  SELECT RAISE(ABORT, 'organization tombstone history is append-only');
END;

CREATE TRIGGER organization_tombstone_events_v1_immutable_delete
BEFORE DELETE ON organization_tombstone_events_v1
BEGIN
  SELECT RAISE(ABORT, 'organization tombstone history is append-only');
END;
