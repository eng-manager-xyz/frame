---
title: "Add shadow reads, controlled dual writes, D1 cutover flags, and rollback"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:ops"
  - "area:api"
  - "type:migration"
  - "risk:high"
depends_on: [12, 13, 14, 15, 16]
size: epic
---

# 17 · Add shadow reads, controlled dual writes, D1 cutover flags, and rollback

## Outcome

Authority can move from MySQL to D1 gradually, observably, and reversibly without pretending cross-database writes are atomic.

## Current Cap reference

Legacy Cap reads and writes MySQL. During migration, Frame needs evidence that D1 answers match and a plan for changes arriving after snapshots. Naive synchronous dual writes can diverge on partial failure.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#12](./12-p2-d1-repositories-query-conformance.md), [#13](./13-p2-auth-sessions-identity.md), [#14](./14-p2-organizations-rbac-spaces-folders.md), [#15](./15-p2-video-collaboration-business-data.md), [#16](./16-p2-mysql-d1-etl-reconciliation.md)

## Scope

Implement shadow-read comparison, write capture/outbox or approved dual-write pattern, replay, lag and divergence metrics, per-domain/tenant flags, authority fencing, maintenance windows, rollback boundaries, and conflict resolution.

### Out of scope

Final production canary and legacy decommission are issue 35.

## Deliverables

- [ ] An authority state machine per domain/tenant with one writer at each irreversible boundary.
- [ ] Shadow-read comparator that normalizes approved differences and protects sensitive values.
- [ ] Durable change capture/replay with idempotency, ordering rules, poison-message handling, and lag SLOs.
- [ ] Cutover, pause, resume, fence, rollback, and reconciliation controls with audit history.
- [ ] Dashboards and alerts for mismatch, lag, write failure, contention, and rollback readiness.

## Acceptance criteria

- [ ] Injected failure between source and target writes is detected, replayed, and reconciled without silent loss.
- [ ] Cutover flags are access-controlled, audited, scoped, and cannot create two authoritative writers.
- [ ] Shadow comparisons cover charter-critical queries and report only approved normalized differences.
- [ ] Rollback completes within the charter window in a production-scale rehearsal and preserves changes made during the canary.
- [ ] No PII or secrets are emitted in comparison payloads or dashboards.

## Required test evidence

- Failure-injection and replay report.
- Timed tenant/domain cutover and rollback rehearsal.
- Mismatch dashboard with a seeded known divergence.

## Risks and open questions

- True atomic dual writes are unavailable and false confidence is dangerous.
- Long overlap windows multiply operational states and conflict scenarios.

## Rollout and rollback

Start with shadow reads, then asynchronous change capture, then test-tenant authority. Expand only while mismatch and lag SLOs hold.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
