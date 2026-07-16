---
title: "Migrate video, upload, edit, comment, notification, storage, import, billing, and developer metadata"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:api"
  - "area:data"
  - "type:migration"
  - "risk:high"
depends_on: [12, 14]
size: epic
---

# 15 · Migrate video, upload, edit, comment, notification, storage, import, billing, and developer metadata

## Outcome

All non-identity Cap metadata required by the charter has D1-backed Rust behavior and explicit retention/accounting semantics.

## Current Cap reference

Cap's remaining schema includes videos and edits, shared videos, comments, notifications, messenger records, storage integrations/objects, uploads/imports, developer apps/domains/API keys/videos, credit accounts/transactions, and daily storage snapshots.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#12](./12-p2-d1-repositories-query-conformance.md), [#14](./14-p2-organizations-rbac-spaces-folders.md)

## Scope

Port these aggregates in coherent slices: media metadata and edits; sharing/comments/notifications; storage and imports; developer platform; usage/billing ledgers; support/messenger only if retained. Define deletion, immutable ledger, and asynchronous workflow boundaries.

### Out of scope

Binary media objects are issues 18–21; external payment, email, AI, and analytics provider integrations are covered by API parity/operations work.

## Deliverables

- [ ] Domain-by-domain source mapping, repositories, services, APIs, and invariants.
- [ ] Versioned edit and metadata documents with compatibility and size policies.
- [ ] Durable notification/outbox and usage-ledger patterns with deduplication.
- [ ] Storage-object manifests that reconcile D1 metadata with provider objects.
- [ ] Derivative/job metadata records executor, source version, transform profile/version, output role/key/checksum/content type, state, usage/cost units, and redacted failure class.
- [ ] Retention, export, deletion, and legal/compliance handling for every data class.

## Acceptance criteria

- [ ] Every remaining in-scope source table is mapped and covered by a repository conformance suite.
- [ ] Comments/shares respect tenant, privacy, deletion, and anonymous-view policy under adversarial tests.
- [ ] Usage and credit operations are append-only or otherwise auditable, idempotent, and reconcile to source fixtures.
- [ ] Large JSON/edit documents have enforced limits and forward-compatible versioning.
- [ ] Async notifications, imports, and storage events tolerate duplicate and out-of-order delivery.
- [ ] Managed and native results update the same domain lifecycle idempotently without storing private media URLs or binding-specific JavaScript objects.

## Required test evidence

- Aggregate-level parity reports.
- Ledger reconciliation and notification outbox fault tests.
- Data-class retention/deletion matrix and sample export.

## Risks and open questions

- Combining too many domains hides sequencing; track implementation as child tasks under this epic.
- Billing or developer-ledger drift has direct financial impact.

## Rollout and rollback

Enable read parity per aggregate, then writes per tenant. Financial and developer ledgers require independent reconciliation and rollback approval.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
