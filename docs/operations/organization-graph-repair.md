# Organization graph audit and repair runbook

The current organization repair capability is inspection plus a persisted
dry-run plan. It never applies a proposed repair. Do not edit organization,
membership, space, folder-closure, tombstone, assertion, receipt, or audit rows
ad hoc to make an audit appear clean. The only legacy exception is the reviewed,
write-paused normalization procedure below; it must run as one atomic restricted
D1 batch and retain its case approval and immutable audit event.

## Required authority and inputs

Use a dedicated support identity with an unexpired ticket grant bound to the
exact organization and a SHA-256 ticket digest. Confirm the identity revision
and session version immediately before the request. Do not paste a raw ticket,
email, session credential, provider error, or tenant export into logs or an
incident comment.

An invalid, revoked, expired, stale, unknown, or cross-organization support
grant fails as `organization_access_denied`. Repeated unavailable results are
not evidence that a graph is absent or clean.

## Audit procedure

1. Record the incident/change identifier and affected user-facing symptoms in
   the protected case system.
2. Verify the organization is in the approved scope and that no ownership,
   tombstone, backfill, or cutover operation is currently running.
3. Run the authenticated bounded graph audit. If it reports truncation, stop;
   use a larger approved bound or an offline protected investigation.
4. Review every finding's kind, opaque subject, and observed revision. The
   audit can report missing/multiple/mismatched owners, orphan membership or
   active selection, space membership without organization membership, missing
   or cross-space folders, cycles, depth mismatch, and deleted ancestors.
5. Generate the dry-run repair plan. Confirm it persists with `dry_run = 1`
   and that organization/member/folder counts did not change.
6. Attach the redacted finding/step summary and plan ID to the case. Never
   attach SQL output containing personal data or support ticket material.

One audit executes its support assertion, three bounded provider-safe graph
queries, assertion cleanup, and allow audit in a single D1 batch, then merges
the result deterministically. Repair-plan generation takes a fresh such
snapshot and reasserts support in its separate insert batch; only a one-way
support-authority fingerprint is persisted. Findings carry observed revisions.
State may change after the snapshot, but no automatic apply path exists and any
future apply must re-read every revision.

## Legacy expansion and normalization

Apply migration `0010` before attempting cleanup. It intentionally leaves a
legacy multi-owner organization intact so the audit schema can become available.
The insert/update guards prevent another active owner, but they do not choose an
owner or make the tenant clean. Keep the tenant's organization-authority flag
off and pause legacy plus new writers throughout this procedure.

First capture the protected evidence:

```sql
SELECT organization_id, pointer_owner_id, active_owner_count, pointer_owner_count
FROM organization_owner_integrity_v1
WHERE active_owner_count <> 1 OR pointer_owner_count <> 1
ORDER BY organization_id;

SELECT user_id, role, state, revision, authority_version
FROM organization_members
WHERE organization_id = :organization_id
  AND role = 'owner' AND state = 'active'
ORDER BY user_id;
```

Do not infer the intended owner from the current pointer, timestamps, activity,
seat state, or lexical order. Security and the data owner must record one
explicit `:selected_owner_id`, a 36-character `:operation_id`, reviewer identity,
case/ticket digest, timestamp, and protected backup. The selected user must
already be exactly one active owner row. Execute the following statements as one
bound D1 batch; the assertion trigger rolls the whole batch back if a pre- or
postcondition is false:

```sql
INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT :operation_id || ':selected_owner',
       CASE WHEN (
         SELECT COUNT(*) FROM organization_members
         WHERE organization_id = :organization_id
           AND user_id = :selected_owner_id
           AND role = 'owner' AND state = 'active'
       ) = 1 THEN 1 ELSE 0 END;

UPDATE organization_members
SET role = 'admin',
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE organization_id = :organization_id
  AND role = 'owner' AND state = 'active'
  AND user_id <> :selected_owner_id;

UPDATE organizations
SET owner_id = :selected_owner_id,
    revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE id = :organization_id;

UPDATE auth_identities_v2
SET session_version = session_version + 1,
    revision = revision + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE user_id = :selected_owner_id
   OR user_id IN (
     SELECT user_id FROM organization_members
     WHERE organization_id = :organization_id
       AND last_operation_id = :operation_id
   );

DELETE FROM auth_session_mutation_grants_v2
WHERE user_id IN (
  SELECT user_id FROM auth_identities_v2
  WHERE last_operation_id = :operation_id
);

INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT :operation_id || ':owner_post',
       CASE WHEN (
         SELECT active_owner_count = 1 AND pointer_owner_count = 1
           AND pointer_owner_id = :selected_owner_id
         FROM organization_owner_integrity_v1
         WHERE organization_id = :organization_id
       ) THEN 1 ELSE 0 END;

INSERT INTO organization_audit_events_v1(
  id, operation_id, organization_id, actor_id, action, subject_kind,
  subject_digest, outcome, denial_code, occurred_at_ms, metadata_json
) VALUES (
  :audit_id, :operation_id, :organization_id, :reviewed_by_user_id,
  'repair_plan', 'repair_plan', :organization_digest, 'allow', NULL,
  :occurred_at_ms, '{"mode":"reviewed_owner_normalization"}'
);

DELETE FROM organization_repository_assertions_v1
WHERE id LIKE :operation_id || ':%';
```

### Reviewed folder-cycle break

Migration backfill preserves a nonzero self path for every detected legacy
cycle. For each `folder_cycle` finding, reviewers must explicitly identify the
folder whose parent edge will be broken. Reparent that folder to root first;
more elaborate placement can use the normal repository only after the graph is
clean. Never select the break node automatically.

After all approved `parent_id = NULL` changes have been included, execute the
following closure rebuild and depth normalization in the same atomic batch. The
tenant must remain write-paused. Bind one organization and space per batch.

```sql
UPDATE folders
SET parent_id = NULL,
    revision = revision + 1,
    tree_revision = tree_revision + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE id = :reviewed_cycle_break_folder_id
  AND organization_id = :organization_id
  AND space_id = :space_id
  AND deleted_at_ms IS NULL;

DELETE FROM organization_folder_closure_v1
WHERE organization_id = :organization_id AND space_id = :space_id;

INSERT INTO organization_folder_closure_v1(
  organization_id, space_id, ancestor_id, descendant_id, distance
)
SELECT organization_id, space_id, id, id, 0
FROM folders
WHERE organization_id = :organization_id AND space_id = :space_id;

WITH RECURSIVE reviewed_tree(
  organization_id, space_id, ancestor_id, descendant_id, distance
) AS (
  SELECT organization_id, space_id, id, id, 0
  FROM folders
  WHERE organization_id = :organization_id AND space_id = :space_id
  UNION ALL
  SELECT reviewed_tree.organization_id,
         reviewed_tree.space_id,
         reviewed_tree.ancestor_id,
         child.id,
         reviewed_tree.distance + 1
  FROM reviewed_tree
  JOIN folders child
    ON child.parent_id = reviewed_tree.descendant_id
   AND child.organization_id = reviewed_tree.organization_id
   AND child.space_id = reviewed_tree.space_id
  WHERE reviewed_tree.distance < 32
)
INSERT OR IGNORE INTO organization_folder_closure_v1(
  organization_id, space_id, ancestor_id, descendant_id, distance
)
SELECT organization_id, space_id, ancestor_id, descendant_id, distance
FROM reviewed_tree;

UPDATE folders
SET depth = 0,
    tree_revision = tree_revision + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE organization_id = :organization_id AND space_id = :space_id
  AND deleted_at_ms IS NULL;

WITH RECURSIVE rooted(organization_id, space_id, folder_id, depth) AS (
  SELECT organization_id, space_id, id, 0
  FROM folders
  WHERE organization_id = :organization_id AND space_id = :space_id
    AND parent_id IS NULL AND deleted_at_ms IS NULL
  UNION ALL
  SELECT child.organization_id, child.space_id, child.id, rooted.depth + 1
  FROM rooted
  JOIN folders child
    ON child.parent_id = rooted.folder_id
   AND child.organization_id = rooted.organization_id
   AND child.space_id = rooted.space_id
   AND child.deleted_at_ms IS NULL
  WHERE rooted.depth < 32
)
UPDATE folders
SET depth = (SELECT depth FROM rooted WHERE folder_id = folders.id)
WHERE id IN (SELECT folder_id FROM rooted);

INSERT INTO organization_repository_assertions_v1(id, satisfied)
SELECT :operation_id || ':folder_post',
       CASE WHEN NOT EXISTS (
         SELECT 1 FROM organization_folder_closure_v1
         WHERE organization_id = :organization_id AND space_id = :space_id
           AND ancestor_id = descendant_id AND distance <> 0
       ) AND NOT EXISTS (
         SELECT 1 FROM folders f
         WHERE f.organization_id = :organization_id AND f.space_id = :space_id
           AND f.deleted_at_ms IS NULL
           AND f.depth <> (
             SELECT COUNT(*) FROM organization_folder_closure_v1 c
             WHERE c.organization_id = f.organization_id
               AND c.space_id = f.space_id
               AND c.descendant_id = f.id AND c.ancestor_id <> f.id
           )
       ) AND NOT EXISTS (
         SELECT 1
         FROM folders f
         LEFT JOIN folders parent ON parent.id = f.parent_id
         WHERE f.organization_id = :organization_id AND f.space_id = :space_id
           AND f.deleted_at_ms IS NULL AND f.parent_id IS NOT NULL
           AND (
             parent.id IS NULL OR parent.organization_id <> f.organization_id
             OR parent.space_id <> f.space_id OR parent.deleted_at_ms IS NOT NULL
           )
       ) THEN 1 ELSE 0 END;

UPDATE organizations
SET revision = revision + 1,
    authority_version = authority_version + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE id = :organization_id;

UPDATE auth_identities_v2
SET session_version = session_version + 1,
    revision = revision + 1,
    updated_at_ms = :occurred_at_ms,
    last_operation_id = :operation_id
WHERE user_id IN (
  SELECT user_id FROM organization_members
  WHERE organization_id = :organization_id AND state = 'active'
);

DELETE FROM auth_session_mutation_grants_v2
WHERE user_id IN (
  SELECT user_id FROM auth_identities_v2
  WHERE last_operation_id = :operation_id
);

INSERT INTO organization_audit_events_v1(
  id, operation_id, organization_id, actor_id, action, subject_kind,
  subject_digest, outcome, denial_code, occurred_at_ms, metadata_json
) VALUES (
  :audit_id, :operation_id, :organization_id, :reviewed_by_user_id,
  'repair_plan', 'repair_plan', :organization_digest, 'allow', NULL,
  :occurred_at_ms, '{"mode":"reviewed_folder_normalization"}'
);

DELETE FROM organization_repository_assertions_v1
WHERE id LIKE :operation_id || ':%';
```

If more than one cycle break is required, include every reviewed folder update
before rebuilding closure; do not run the template partially.
Re-run the graph audit and owner-integrity view after the batch. If any finding,
unexpected row count, foreign-key violation, or stale assertion remains, keep
authority disabled and restore or escalate rather than improvising another edit.

The partial unique owner index belongs to a later contract migration. Create it
only after the owner-integrity view reports `(1, 1)` for every organization and
all protected cleanup records are approved. The runtime coordination is:
expand with writes off, audit, normalize one tenant at a time, invalidate
authority, re-audit, then enable that tenant; never run legacy and new writers
concurrently.

## Review and escalation

- A missing, multiple, or pointer-mismatched owner is a security incident. Pause
  organization authority changes and assign security plus data owners.
- An orphan membership or active selection requires identity and membership
  history review; do not infer the intended owner or role from activity.
- A folder cycle, cross-space edge, or deleted ancestor requires both the
  organization owner and data owner because a reparent can change visibility.
- Tombstoned organizations remain under the approved retention policy. A repair
  plan cannot bypass retention or recover an expired organization.

Any future apply tool must require a separately approved capability, re-read
every organization/subject/revision, use the same repository authority and
audit boundary, apply bounded reversible steps, and stop on the first stale
finding. This repository intentionally provides no such apply method today.

## Rollback and closure

Because generation is dry-run only, rollback is deletion of no data: preserve
the immutable plan and audit records and revoke the temporary support grant.
If an independently approved repair is later implemented, take a protected D1
backup, retain source/legacy authority, use idempotent operations, and run a new
audit before and after each bounded cohort. Close the incident only after the
graph is clean, user impact is reconciled, support authority is revoked, and
security/data owners sign the record.

Local tests do not authorize production repair. Remote D1 backup/restore,
representative graph scale, customer retention approval, public/API behavior,
and production security-owner rehearsal remain blockers.
