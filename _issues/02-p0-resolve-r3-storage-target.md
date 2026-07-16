---
title: "ADR: Resolve “R3” and choose the R2/S3 object-storage target"
labels:
  - "phase:p0"
  - "area:architecture"
  - "area:storage"
  - "type:adr"
  - "risk:high"
depends_on: [01]
size: epic
---

# 02 · ADR: Resolve “R3” and choose the R2/S3 object-storage target

## Outcome

The ambiguous R3 requirement becomes an explicit, approved storage decision with no hidden product regression.

## Current Cap reference

Cloudflare documents D1 and R2 bindings, not a matching R3 object-storage product. Cap supports AWS and other S3-compatible stores, Cloudflare R2 through S3 compatibility, MinIO for self-hosting, custom user buckets, and Google Drive.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#01](./01-p0-migration-charter-parity-slos.md)

## Scope

Confirm what the requester meant by R3; compare R2-only, R2 plus S3-compatible adapters, and any corrected target. Cover direct and brokered uploads, multipart behavior, signed access, ranges, custom domains, lifecycle, data residency, BYO storage, self-hosting, pricing, and egress.

### Out of scope

Implementing the selected adapter, migrating objects, and building upload UI belong to issues 18–21.

## Deliverables

- [ ] An accepted replacement for proposed ADR 0002 with the exact product and terminology.
- [ ] A capability and cost comparison against Cap's current storage modes.
- [ ] A provider-neutral ObjectStore and UploadBroker contract boundary.
- [ ] A product decision for S3-compatible, MinIO, Google Drive, and user-owned bucket parity.
- [ ] A naming migration that removes ambiguous R3 references after the decision.

## Acceptance criteria

- [ ] The ADR links authoritative vendor documentation and names the production binding/API.
- [ ] Every existing Cap storage mode is marked preserved, changed, deferred, or intentionally removed with approval.
- [ ] Security owners approve the credential, signing, tenant-isolation, and data-residency model.
- [ ] Issue 18 can implement the contract without reopening provider selection.
- [ ] README, Wrangler configuration, architecture docs, and backlog use consistent final terminology.

## Required test evidence

- A compatibility spike against the chosen provider.
- Cost model for representative recording sizes and retention periods.
- ADR approval record and product acceptance of any parity loss.

## Risks and open questions

- Silently treating R3 as R2 could build the wrong system.
- R2-only coupling can break self-hosting and bring-your-own-storage expectations.

## Rollout and rollback

Keep the existing neutral RECORDINGS binding and adapter port until the ADR is accepted. Reverting the ADR must not require changing domain types.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
