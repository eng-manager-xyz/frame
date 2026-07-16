---
title: "Build resumable MySQL-to-D1 export, transform, import, and reconciliation tooling"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:data"
  - "area:ops"
  - "type:migration"
  - "risk:high"
depends_on: [04, 11, 12, 13, 14, 15]
size: epic
---

# 16 · Build resumable MySQL-to-D1 export, transform, import, and reconciliation tooling

## Outcome

Production metadata can be moved repeatedly and safely with deterministic transforms, complete audit evidence, and bounded downtime.

## Current Cap reference

The source is MySQL/Drizzle with provider-specific values and relationships; the target is D1/SQLite with different type, query, size, and consistency constraints. A one-shot SQL dump cannot prove semantic equivalence.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#04](./04-p0-parity-fixtures-baselines.md), [#11](./11-p2-d1-schema-migrations.md), [#12](./12-p2-d1-repositories-query-conformance.md), [#13](./13-p2-auth-sessions-identity.md), [#14](./14-p2-organizations-rbac-spaces-folders.md), [#15](./15-p2-video-collaboration-business-data.md)

## Scope

Build snapshot/export, chunked transform, staged import, dependency ordering, resumable checkpoints, idempotency, rejects/quarantine, redaction, throttling, incremental catch-up, and row/relationship/semantic reconciliation.

### Out of scope

Object bytes are migrated in issue 20. Choosing the final authority and traffic cutover is issue 17/35.

## Deliverables

- [ ] A versioned ETL manifest tied to source schema, target migration, code SHA, window, and checksums.
- [ ] Deterministic transforms for timestamps, booleans, JSON, bigints, decimals, collations, enums, nulls, and invalid legacy rows.
- [ ] Resumable, rate-limited import with per-tenant/table checkpoints and safe reruns.
- [ ] Reconciliation for counts, primary/foreign keys, field hashes, aggregates, policy semantics, and sampled API behavior.
- [ ] Dry-run, abort, restore, incident, and operator runbooks.

## Acceptance criteria

- [ ] Rerunning any completed or interrupted chunk produces no duplicate logical records or ledger effects.
- [ ] A full production-scale rehearsal finishes within the approved window and resource/cost budget.
- [ ] Reconciliation reports zero unexplained row, relationship, field-hash, aggregate, or semantic mismatches.
- [ ] Rejected records are quarantined with non-sensitive reason codes and an approved disposition.
- [ ] Backup restore and rollback are timed and successfully rehearsed before production authorization.

## Required test evidence

- At least two full-scale rehearsal manifests and reconciliation reports.
- Injected interruption/resume and malformed-record results.
- Restore timing and operator sign-off.

## Risks and open questions

- Source data can change during export and create inconsistent relationships.
- PII in dumps, logs, or reject files creates a security incident.

## Rollout and rollback

Run read-only dry runs first, then isolated preview imports, then a staged snapshot plus catch-up. Abort never mutates source authority.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
