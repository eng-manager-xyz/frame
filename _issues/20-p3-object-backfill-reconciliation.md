---
title: "Backfill media objects with manifests, checksums, retries, and reconciliation"
labels:
  - "phase:p3"
  - "area:storage"
  - "area:data"
  - "area:ops"
  - "type:migration"
  - "risk:high"
depends_on: [04, 18, 19]
size: epic
---

# 20 · Backfill media objects with manifests, checksums, retries, and reconciliation

## Outcome

Every retained Cap object is copied or deliberately referenced with provable integrity and consistent D1 metadata.

## Current Cap reference

Cap object data may live in Cap-managed S3, R2-compatible providers, MinIO, custom user buckets, or Google Drive, with database keys/URLs and multipart artifacts. Row counts alone cannot prove media preservation.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#04](./04-p0-parity-fixtures-baselines.md), [#18](./18-p3-object-storage-adapter-key-contract.md), [#19](./19-p3-multipart-upload-download.md)

## Scope

Inventory objects and ownership, create manifests, stream copy with bounded concurrency, provider-specific readers, checksum/probe verification, resume, quarantine, missing/orphan detection, retry budgets, cost/egress controls, and source retention.

### Out of scope

Reconsidering canonical R2 requires a superseding ADR; ongoing lifecycle policy is issue 21; MySQL metadata ETL is issue 16.

## Deliverables

- [ ] Immutable migration manifest with source locator, target key, tenant/video/role, bytes, checksum strategy, status, attempts, and tool version.
- [ ] Resumable copy/reference workers with rate, bandwidth, cost, provider, tenant, and region controls.
- [ ] Verification using byte counts, provider checksums where valid, cryptographic hashes where available, and media probes for critical objects.
- [ ] Reconciliation for missing source, missing target, duplicates, orphan target objects, ownership mismatch, and corrupt/unplayable media.
- [ ] Dry-run, pause, resume, abort, source-retention, and customer/support runbooks.

## Acceptance criteria

- [ ] Re-running a completed or interrupted manifest is idempotent and does not duplicate billable objects.
- [ ] Source and target object count, logical bytes, role counts, and verified checksums reconcile with zero unexplained differences.
- [ ] Private/custom-provider credentials remain scoped, encrypted, redacted, and absent from manifests.
- [ ] Corrupt or unavailable objects are quarantined with an owner-approved disposition and user-impact list.
- [ ] A production-scale rehearsal meets throughput, error-rate, and cost budgets from the charter.

## Required test evidence

- Two independent rehearsal manifests and reconciliation reports.
- Injected truncation, corruption, throttling, expiry, and provider-outage results.
- Sample playback/probe comparison across every object role and provider class.

## Risks and open questions

- Multipart etags are often not content hashes.
- Copying BYO or Drive data may violate user expectations, permissions, residency, or cost constraints.

## Rollout and rollback

Start with read-only inventory, then test prefixes, then non-critical tenants. Preserve source objects through the approved observation/rollback window.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
