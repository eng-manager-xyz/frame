---
title: "Migrate organizations, memberships, invites, RBAC, spaces, and folders to D1"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:security"
  - "area:api"
  - "type:migration"
  - "risk:high"
depends_on: [12, 13]
size: epic
---

# 14 · Migrate organizations, memberships, invites, RBAC, spaces, and folders to D1

## Outcome

Tenant boundaries and collaboration structures behave consistently under D1 and a centralized Rust authorization policy.

## Current Cap reference

Cap models organizations, members, invites, spaces, space members/videos, folders, domains, ownership, roles, seats, onboarding, and policy settings across its schema and backend services.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#12](./12-p2-d1-repositories-query-conformance.md), [#13](./13-p2-auth-sessions-identity.md)

## Scope

Port organization lifecycle, active/default org, ownership transfer, member roles, invites, seat flags, allowed domains, spaces, folders, move/share operations, tombstones, and policy evaluation. Centralize object-level authorization.

### Out of scope

Video/comment payloads are issue 15; billing collection is not rebuilt here beyond fields required for authorization.

## Deliverables

- [ ] A policy matrix covering actor, tenant, role, object, action, ownership, and exceptional states.
- [ ] Rust domain services and D1 repositories for organizations, invites, memberships, spaces, and folders.
- [ ] Race-safe invite acceptance, ownership transfer, removal, tombstone, and resource-move workflows.
- [ ] Tenant-scoped query helpers and negative authorization tests.
- [ ] Audit, support, and repair tooling for orphaned or inconsistent membership graphs.

## Acceptance criteria

- [ ] Cross-tenant IDs cannot disclose existence or mutate data through any repository or API path.
- [ ] Owner/admin/member and space-role behavior matches the approved parity fixtures.
- [ ] Concurrent invite acceptance, member removal, ownership transfer, and folder moves preserve invariants.
- [ ] Deleting or tombstoning a tenant follows the approved retention and recovery policy.
- [ ] Authorization decisions are testable without network/database access and report stable denial reasons.

## Required test evidence

- Policy-table tests including every role/action combination.
- Concurrency and idempotency test results.
- Tenant-boundary penetration-test report.

## Risks and open questions

- Distributed policy checks can drift if not centralized.
- Ownership and deletion changes can orphan videos or grant stale access.

## Rollout and rollback

Shadow-evaluate Rust policy beside legacy decisions, alert on mismatches, then enable writes per tenant with a reversible authority flag.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
