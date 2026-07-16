# Organization, RBAC, space, and folder authority

The organization authority core is split across `frame-domain`, `frame-ports`,
`frame-application`, and the control-plane D1 adapter. Callers use typed tenant,
organization, member, invite, space, folder, revision, operation, and digest
values. The organization UUID is also the tenant UUID; construction rejects a
scope whose two identities differ.

This is a production-shaped local authority boundary, not a production cutover.
The existing public handlers have not all been moved to this repository, legacy
Cap parity fixtures and shadow decisions have not been approved, and remote D1
contention/replication evidence is not present. Those are rollout blockers.

## Central policy matrix

`AuthorizationPolicy::evaluate_organization` is pure and has no database or
network dependency. Every decision binds actor tenant, resource tenant,
organization, principal state, organization state, membership state and role,
object kind/state, optional space role, ownership, invite grant, and explicit
support authority. Unknown and cross-tenant resources both return
`organization_access_denied`.

| Action group | Owner | Admin | Member | Viewer | Exceptional authority |
|---|---|---|---|---|---|
| Organization create | deny as an existing member | deny as an existing member | deny as an existing member | deny as an existing member | active non-member principal plus repository absence assertion |
| Organization read | allow | allow | allow | allow | active membership required |
| Organization/settings update | allow | allow | deny | deny | active organization only |
| Billing and seats | allow | billing denied; seats allowed | deny | deny | billing collection remains out of scope |
| Invite issue/revoke | allow | allow | deny | deny | hashed invite data only |
| Invite accept | deny | deny | deny | deny | only a non-member with a repository-validated, identity-bound invite grant may accept |
| Member read | allow | allow | allow | allow | active membership required |
| Member role/remove | allow | allow | deny | deny | owner role cannot be assigned by this command |
| Ownership transfer | allow | deny | deny | deny | target must be an active non-owner member |
| Allowed domain | allow | allow | deny | deny | maximum 256 domains per organization |
| Space read/create | allow | allow | allow | read only | organization and space fences both apply |
| Space manage/role | allow | allow | manager only | deny | target principal generation is invalidated |
| Folder read | allow | allow | allow | allow | active object required |
| Folder create | allow | allow | manager or contributor | deny | space authority and tree fences apply |
| Folder move | allow | allow | manager; contributor only for owned folder | deny | cycle, cross-space, parent, subtree, and depth checks are transactional |
| Tombstone/recover | allow | deny | deny | deny | recovery is allowed only while tombstoned and inside retention |
| Graph audit/repair plan | deny without support | deny without support | deny | deny | explicit unexpired support authority only; plans are dry-run |

`CreateSpace` includes the requested visibility under the approved member
create authority; the creator is atomically added as that space's manager.
Likewise, a contributor creates and owns a folder and may manage or publish
that owned folder. Commercial plan eligibility for public collection links is
a separate service-layer parity gate and is not fabricated in this authority
slice.

The exhaustive domain test evaluates 26 actions across four organization roles,
four space-role states, and both ownership states: 832 table cases. Additional
tests cover wrong object types, inactive principals and memberships, tombstones,
invite grants, tenant mismatch, and support-only graph operations.

## D1 write boundary

Migration `0010_organization_authority_expand.sql` adds revision and authority
fences, staged active-owner enforcement, operation receipts, immutable audit
and tombstone histories, explicit support grants, folder closure rows, retention
assertions, and dry-run repair plans. Runtime SQL is checked in under
`apps/control-plane/queries/organization`; all external values are bound.

The migration is deliberately expand-phase safe for legacy data. Migration
`0003` did not prohibit multiple active owners, so `0010` must not create the
partial unique owner index: doing so would prevent the audit and repair schema
from becoming available on a dirty tenant. Instead, insert/update transition
guards prevent a clean or new organization from acquiring a second owner and
prevent a dirty organization from acquiring another one. The
`organization_owner_integrity_v1` view exposes active-owner and pointer-owner
counts. Repository postconditions still require exactly one owner for every
accepted new create/transfer. A later contract migration may add the partial
unique index only after every organization has been reviewed clean.

Legacy closure backfill retains distance-zero self edges and, because distance
is part of the closure key, also retains nonzero self paths produced by a parent
cycle. This makes cycles visible to the checked-in graph audit. Folder depth is
recomputed only for active nodes reached from an active root through the same
organization and space, up to depth 32. Cycles, missing/deleted parents, and
cross-scope parents are not guessed or silently normalized.

Every organization mutation batch performs these steps:

1. Assert the operation/idempotency tuple is absent. Receipt lookup is joined
   to the original allow audit and current actor identity/session, so a
   different actor sees existing and absent keys identically.
2. Assert the authenticated user identity revision and session version.
3. Assert organization status, revision, authority version, membership state,
   membership revision/authority version, and required role class.
4. For space/folder operations, assert space revision and the actor's space
   membership revision and role.
5. Perform the mutation and assert its exact postcondition.
6. Store a server-derived, length-framed semantic fingerprint, operation
   receipt, and immutable allow audit in the same batch.

Only the migration-owned exact trigger envelopes are interpreted as a stale
authority or retention conflict. Unknown D1/provider text is
`organization_unavailable`; raw SQL, bindings, provider messages, subjects,
emails, and digests are not returned or logged. A retry by the same authenticated
actor with the exact operation ID, idempotency key, action, subject, and
canonical semantic payload reconstructs the receipt. The caller never supplies
the fingerprint. Any semantic mismatch is a conflict.

## Race and lifecycle invariants

- Invite acceptance validates the invite ID, organization, hashed token,
  invitation key version plus the authenticated user's registered identifier
  digest, pending state, expiry, user state, identity/session fence, and
  absence of any existing membership in the committing batch. Concurrent accepts
  have one winner; an exact winner replay returns its receipt.
- Ownership transfer demotes the old owner, updates the owner pointer, promotes
  the target, invalidates both principals, revokes mutation grants, and asserts
  exactly one active pointer-matching owner in one batch. A concurrent target
  removal cannot leave zero or two owners. A legacy organization already
  containing multiple owners remains write-disabled until an explicit reviewed
  owner choice is normalized; migration guards only prevent the condition from
  getting worse.
- Member and space-role downgrades increment the target's session generation
  and revoke outstanding grants. A write with the old generation either commits
  before the downgrade or fails stale after it; it cannot commit afterward.
- Folder closure contains a distance-zero self edge and bounded ancestor edges.
  Moves reject descendants as parents, cross-organization/space parents,
  deleted nodes, stale tree revisions, and depth above 32. Closure replacement,
  subtree depth changes, and postconditions share the mutation batch.
- Tombstoning increments both data and authority revisions and appends immutable
  history. Recovery reasserts the exact tombstone timestamp and current time at
  the write boundary; expiry returns `organization_retention_locked`.

## Read and repair consistency

Authorization snapshots join the organization, active user identity,
membership, and active/default selection under one tenant predicate. A missing
organization, missing membership, wrong actor, and wrong tenant are intentionally
indistinguishable.

Graph audit is read-only and support-gated. Wrangler local D1 limits compound
query terms, so membership, selection, and folder findings use three bounded
queries inside the same D1 batch as the support assertion, assertion cleanup,
and allow audit. The adapter deterministically merges by finding kind and
subject, retains each observed revision, applies one global result bound, and
reports truncation. Repair-plan generation obtains the findings snapshot, then
reasserts support in the separate plan-insert batch; it stores only a one-way
fingerprint of the support authority. A plan contains revision-fenced steps
with `dry_run = 1` and this slice has no automatic apply path. State may change
after the snapshot, so any future reviewed apply must re-read and revalidate
every subject and revision.

## Legacy normalization stages

Organization authority writes remain disabled while `0010` is applied and dirty
graphs are reviewed. Promotion has three distinct stages:

1. **Expand and inspect.** Apply `0010`, query
   `organization_owner_integrity_v1`, and run the support-gated graph audit.
   Migration success is not evidence that a tenant is clean.
2. **Explicit normalization.** Pause every writer for the affected organization,
   take a protected backup, record the security/data-owner decision identifying
   the intended owner and every folder reparent, then run the guarded procedure
   in the operations runbook. Bump affected identity session versions, revoke
   mutation grants, and re-run all owner/folder checks before writes resume.
3. **Contract enforcement.** Only after the integrity view is `(1, 1)` for every
   organization and graph audits are clean may a separately reviewed contract
   migration add `organization_members_one_active_owner_idx`. Do not add that
   index to the expand migration or enable two authority writers during cleanup.

The native migration test upgrades a pre-`0010` database containing two active
owners, a valid nested tree, and a parent cycle. It proves the upgrade succeeds,
nested depths become `0/1/2`, cycle findings retain nonzero self paths, a third
owner is rejected, explicit owner/folder normalization becomes clean, and the
future contract index can then be created.

## Rollout and rollback boundary

Promotion requires approved Cap parity fixtures, shadow-decision telemetry,
tenant-by-tenant authority flags, public/API adapter penetration tests,
customer-approved retention windows, remote D1 contention and restore drills,
and security/production-owner signoff. Rollback keeps the source authority and
replays idempotent accepted writes; it must not enable two organization writers
for one tenant. Changing `0010` or any query changes the evidence digests. The
checked-in Wrangler artifact is explicitly stale and pre-repair; the current
network-free SQLite semantic suite does not replace compiled Worker/D1
conformance. A fresh pinned-Wrangler run is required before the PR can claim
that boundary.

Local proof and its exclusions are recorded in
[`organization-d1-local.md`](../evidence/organization-d1-local.md). Operational
inspection and repair are defined in
[`organization-graph-repair.md`](../operations/organization-graph-repair.md).
