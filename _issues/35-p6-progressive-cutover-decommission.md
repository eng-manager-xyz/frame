---
title: "Execute progressive cutover, validate parity, rehearse rollback, and decommission legacy services"
labels:
  - "phase:p6"
  - "area:ops"
  - "area:program"
  - "area:release"
  - "type:release"
  - "risk:critical"
depends_on: [04, 16, 17, 20, 34]
size: epic
---

# 35 · Execute progressive cutover, validate parity, rehearse rollback, and decommission legacy services

## Outcome

Production authority moves to Frame with verified user/data/media outcomes, a timed rollback path, and controlled legacy retirement.

## Current Cap reference

Until this issue, Cap remains authoritative for at least some traffic, data, clients, media, or workflows. A migration is incomplete while fallback paths, duplicate infrastructure, and ambiguous data ownership remain unmanaged.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#04](./04-p0-parity-fixtures-baselines.md), [#16](./16-p2-mysql-d1-etl-reconciliation.md), [#17](./17-p2-shadow-dual-write-cutover.md), [#20](./20-p3-object-backfill-reconciliation.md), [#34](./34-p6-operational-hardening.md)

## Scope

Define go/no-go checklist, freezes/catch-up, tenant/cohort canaries, client/version readiness, route/job/storage/data authority flags, support communications, SLO/mismatch monitoring, rollback drills, final reconciliation, observation window, source retention, decommission, credential revocation, cost cleanup, and postmortem.

### Out of scope

No new feature work is accepted into the cutover unless it closes an approved blocker.

## Deliverables

- [ ] Detailed runbook with owners, timestamps, communication, authority state, checkpoints, stop conditions, and command/evidence links.
- [ ] Canary cohorts covering internal, representative tenants, storage modes, platforms, media modes, and high-risk workflows.
- [ ] Automated go/no-go dashboard combining SLOs, parity, reconciliation, backlog, client adoption, capacity, and rollback readiness.
- [ ] Final MySQL/D1 and object reconciliation plus immutable migration evidence.
- [ ] Legacy decommission/retention plan covering services, routes, queues, databases, buckets, secrets, DNS, jobs, clients, dashboards, runbooks, and billing.

## Acceptance criteria

- [ ] Every P0–P5 phase gate is signed and no unowned critical/high cutover blocker remains.
- [ ] Canary SLOs, parity comparisons, support volume, data reconciliation, and media quality remain within charter budgets for the approved observation period.
- [ ] A production-scale rollback is successfully rehearsed, timed, and preserves canary writes before the irreversible gate.
- [ ] Final row/relationship/aggregate/object count/byte/checksum and sampled semantic reconciliation has zero unexplained differences.
- [ ] Legacy authority is removed only after rollback expiry approval; credentials and scheduled work are revoked, retained data is access-controlled, and post-cutover monitoring remains active.

## Required test evidence

- Signed go/no-go records for each ramp stage.
- Timed rollback rehearsal and final reconciliation manifests.
- Decommission checklist, cost delta, customer/support report, and migration retrospective.

## Risks and open questions

- Pressure to decommission early can destroy rollback evidence or source data.
- Old clients, BYO storage, long-running uploads, and scheduled jobs can continue writing after an incomplete fence.

## Rollout and rollback

Ramp by explicitly defined cohort percentages/tenants with automatic and manual stop conditions. Before the irreversible gate, rollback restores legacy authority and replays captured writes. Afterward, recovery follows issue 34 DR rather than an undocumented fallback.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
