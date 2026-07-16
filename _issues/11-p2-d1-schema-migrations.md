---
title: "Port the MySQL/Drizzle schema, constraints, and indexes to D1/SQLite migrations"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:data"
  - "type:migration"
  - "risk:high"
depends_on: [06, 10]
size: epic
---

# 11 · Port the MySQL/Drizzle schema, constraints, and indexes to D1/SQLite migrations

## Outcome

A complete, reviewed D1 schema preserves required Cap data semantics within documented D1 constraints.

## Current Cap reference

Cap's packages/database/schema.ts defines roughly 32 MySQL tables and its migration history contains dozens of SQL migrations. It relies on MySQL types, collations, timestamps, JSON, unsigned/big integer behavior, indexes, and update semantics that do not translate mechanically to SQLite/D1.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#06](./06-p1-shared-domain-api-contracts.md), [#10](./10-p1-local-development-stack.md)

## Scope

Map every table, column, constraint, relationship, enum, index, default, generated/on-update behavior, JSON field, and retention rule. Design tenant partitioning/sharding before the D1 size or single-writer model becomes a production limit.

### Out of scope

Application repository methods and bulk production data transfer belong to issues 12 and 16.

## Deliverables

- [ ] A source-to-target schema matrix with semantic transformations and rejected values.
- [ ] Ordered, immutable D1 migrations for all in-scope domains plus rollback/forward-fix policy.
- [ ] Explicit timestamp, boolean, bigint, decimal/money, JSON, collation, case-sensitivity, and foreign-key conventions.
- [ ] Index/query plan tied to known access patterns and D1 limits.
- [ ] Database sizing, tenant partitioning, retention, and growth model.

## Acceptance criteria

- [ ] Every source column and constraint has a target mapping or approved omission.
- [ ] Migrations apply from empty and upgrade from every supported released schema in local D1.
- [ ] Foreign-key checks and integrity probes pass after seeded imports.
- [ ] Values beyond JavaScript's safe integer range and MySQL-specific date/collation cases have explicit tested handling.
- [ ] Expected dataset growth remains within the approved D1 database strategy or a sharding plan is accepted.

## Required test evidence

- Schema diff and migration test report.
- Representative query plans and index-usage evidence.
- Boundary-value fixtures for types and constraints.

## Risks and open questions

- D1 limits and serial query execution can invalidate a one-database design.
- SQLite accepting loose types can hide corrupt imports without explicit checks.

## Rollout and rollback

Migrations first run on disposable preview databases. Production changes are expand/contract or forward-fix; backups and restore evidence precede irreversible changes.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
