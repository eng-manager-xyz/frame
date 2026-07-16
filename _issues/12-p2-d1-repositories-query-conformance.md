---
title: "Implement D1 repository adapters, transaction patterns, and query-conformance tests"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:rust"
  - "type:migration"
  - "risk:high"
depends_on: [07, 11]
size: epic
---

# 12 · Implement D1 repository adapters, transaction patterns, and query-conformance tests

## Outcome

Rust services use tested repositories whose results and authorization-relevant semantics match Cap without leaking D1 details into domain code.

## Current Cap reference

Cap uses Drizzle/MySQL across database and web-backend packages. Frame has only in-memory port adapters and one D1 health query.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#07](./07-p1-control-plane-media-job-protocol.md), [#11](./11-p2-d1-schema-migrations.md)

## Scope

Implement D1 adapters for repository contracts, pagination, filtering, optimistic concurrency, batches, sessions/bookmarks where needed, transaction patterns, idempotent writes, error mapping, query limits, and observability.

### Out of scope

Porting each business workflow is divided across issues 13–15; bulk ETL is issue 16.

## Deliverables

- [ ] D1 repository implementations grouped by aggregate rather than table-shaped leakage.
- [ ] Conformance suites that run identical behavior against in-memory/reference fixtures and local D1.
- [ ] Documented batch/transaction and sequential-consistency patterns.
- [ ] Bounded pagination and query builders that respect parameter, statement, and response limits.
- [ ] Metrics for query class, duration, rows, retries, bookmark use, and redacted failures.

## Acceptance criteria

- [ ] Repository conformance tests cover found/not-found, conflict, validation, tenant isolation, pagination boundaries, and retryable failures.
- [ ] Multi-row invariants use a D1-supported atomic pattern or compensating workflow with documented failure behavior.
- [ ] No user input is interpolated into SQL and parameter chunking handles provider limits.
- [ ] Read-after-write workflows use appropriate consistency/session behavior and pass injected replication-lag tests where applicable.
- [ ] Public errors remain stable while internal query details are redacted.

## Required test evidence

- Local D1 conformance report and query plans.
- Fault tests for contention, timeout, partial batch, duplicate command, and stale version.
- Trace samples showing safe database telemetry.

## Risks and open questions

- A generic ORM-shaped layer can hide inefficient or unsupported D1 behavior.
- Cross-request transactions do not exist; business workflows need explicit state machines.

## Rollout and rollback

Ship adapters behind repository selection flags and compare read results before making D1 authoritative.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
