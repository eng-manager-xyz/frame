---
title: "Establish Cloudflare R2 as canonical storage and decide legacy/BYO compatibility"
labels:
  - "phase:p0"
  - "area:architecture"
  - "area:storage"
  - "type:adr"
  - "risk:high"
depends_on: [01]
size: epic
---

# 02 · Establish Cloudflare R2 as canonical storage and decide legacy/BYO compatibility

## Outcome

Cloudflare R2 is the approved hosted object store, with an explicit compatibility disposition for every Cap storage mode and no provider ambiguity left for implementation.

## Current Cap reference

Cap supports AWS and other S3-compatible stores, Cloudflare R2 through S3 compatibility, MinIO for self-hosting, custom user buckets, and Google Drive. Frame already configures the first-class `RECORDINGS` R2 Worker binding; this is the target migration architecture rather than behavior present in the pinned Cap snapshot.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#01](./01-p0-migration-charter-parity-slos.md)

## Scope

Approve R2 as canonical hosted storage and decide whether each S3-compatible, MinIO, Google Drive, self-hosted, and user-owned bucket mode is preserved, adapted, deferred, or retired. Cover direct and brokered uploads, multipart behavior, signed access, ranges, custom domains, lifecycle, data residency, bring-your-own storage, self-hosting, pricing, egress, and Cloudflare Media access to private R2 inputs.

### Out of scope

Implementing the R2 adapter, migrating objects, and building upload UI belong to issues 18–21. Selecting the `[stream]` managed video-library binding is a separate product decision; the configured Media Transformations binding does not imply it.

## Deliverables

- [ ] Approval record for accepted ADR 0002 and the `RECORDINGS` R2 binding.
- [ ] A capability, compliance, residency, and cost model for representative recording sizes and retention periods.
- [ ] Provider-neutral `ObjectStore` and upload-broker boundaries with explicit capability negotiation.
- [ ] A preserve/change/defer/retire decision for S3-compatible, MinIO, Google Drive, self-hosted, and user-owned storage.
- [ ] Naming, key, bucket, environment, and ownership conventions for R2 source and derivative objects.

## Acceptance criteria

- [ ] The ADR links authoritative Cloudflare R2 documentation and names the production binding and buckets.
- [ ] Every existing Cap storage mode has an approved disposition, migration impact, owner, and rollback implication.
- [ ] Security owners approve credentials, signing, tenant isolation, Media Transformations access, lifecycle, and data residency.
- [ ] Issue 18 can implement R2 without reopening provider selection; optional adapters cannot weaken the R2 contract.
- [ ] README, Wrangler configuration, architecture docs, and the backlog use consistent R2 terminology.

## Required test evidence

- A private R2 Worker binding spike covering put, head, get/range, conditional behavior, and delete.
- A private R2 input-to-Media Transformations feasibility spike owned jointly with issue 03.
- Compatibility and cost reports plus product acceptance of any legacy-storage parity loss.

## Risks and open questions

- R2-only coupling can break self-hosting or bring-your-own-storage expectations unless the charter explicitly changes them.
- Provider etags, multipart checksums, signing, conditional operations, and lifecycle behavior are not interchangeable.

## Rollout and rollback

Use namespaced non-production R2 buckets until the storage contract and security review close. Domain types stay provider-neutral; reversing the hosted storage decision requires a new ADR and migration plan.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
