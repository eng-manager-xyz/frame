# Organization authorization security contract

Organization authorization is deny-by-default. A preflight snapshot can inform
the application but is never mutation authority; D1 reasserts the complete
identity, tenant, role, object, and revision fence in the write batch.

## Trust boundaries

- Organization and tenant identity are the same typed UUID. Caller-supplied
  cross-tenant IDs cannot select an alternate row or reveal whether it exists.
- An active user is insufficient. The identity revision, session version,
  organization status, membership state/role/revision/authority version, and
  applicable space membership must all remain current.
- Invite plaintext is accepted only at the service boundary, hashed before the
  repository call, and represented by redacted, versioned secret types. D1
  binds the invitation's key version and digest to the authenticated user's
  registered identifier; it never trusts an actor-supplied email digest.
- Ownership, member-role, and space-role changes invalidate affected principal
  generations and outstanding grants as part of the authority mutation.
- Graph inspection is not implied by organization ownership. It requires an
  explicit, unexpired, non-revoked support grant bound to actor, organization,
  ticket digest, identity revision, and session version.

## Threat and control matrix

| Threat | Control |
|---|---|
| Tenant/ID enumeration | One tenant-bound predicate; missing, unknown, and cross-tenant results collapse to `organization_access_denied` |
| Stale session or downgraded role | Identity/session plus membership authority fences in the committing batch; target generation bump and grant revocation |
| Invite double spend | Hashed token plus versioned authenticated-identifier eligibility assertion, expiry, invite revision, absence of any existing membership, actor-gated receipt replay, and one transaction |
| Owner orphaning or split ownership | Expand-safe transition guards plus exact one-owner/pointer postcondition in the transfer batch; the partial unique index is deferred until dirty legacy tenants are normalized |
| Folder cycle or tenant escape | Organization/space/parent predicates, closure-based descendant rejection, bounded depth, and tree postconditions |
| Tombstone resurrection | Status/authority fence, exact tombstone timestamp, retention trigger, immutable event history |
| Forged support ticket | Explicit support assertion bound to active principal, current identity/session, active-or-tombstoned organization, ticket digest, expiry, and revocation; invalid grants return access denied without mutating an attacker-selected tenant |
| Ambiguous retry | Stable operation ID/idempotency key plus a server-derived canonical semantic fingerprint; receipt lookup is joined to the original allow-audit actor and current identity/session |
| Provider-message spoofing | Only exact migration-owned D1 trigger envelope and constraint class map to stale/retention; everything else is unavailable |
| Secret or tenant leakage in telemetry | Fixed event/operation/outcome/row-count fields; no SQL, bindings, IDs, emails, digests, or provider text |

## Stable decisions and audit

Public denial reasons are limited to access denied, inactive authority, state
denied, stale authority, and retention locked. Adapter conflict, invalid,
unavailable, and corrupt results are also stable but do not expose D1 details.

Allowed mutations write their operation receipt and allow audit atomically.
Failed optimistic, cross-tenant, and support assertions roll the complete batch
back and emit only fixed-shape telemetry; they do not write a denial row into a
tenant selected by the rejected request. Existing audit rows are immutable and
their metadata is bounded valid JSON. A trusted higher-level audit sink may
record denials only after independently binding its own authority and tenant;
the repository's public `audit_decision` input is telemetry-only.

This local contract does not establish that every existing handler uses the new
repository. Public/API route inventory, legacy Cap parity and shadow decisions,
remote D1 behavior, real support identity integration, and production log/audit
review remain required before authority promotion.
