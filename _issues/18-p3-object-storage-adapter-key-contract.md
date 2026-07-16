---
title: "Define versioned object keys and implement an R2/S3 storage adapter"
labels:
  - "phase:p3"
  - "area:storage"
  - "area:rust"
  - "type:migration"
  - "risk:high"
depends_on: [02, 06, 07]
size: epic
---

# 18 · Define versioned object keys and implement an R2/S3 storage adapter

## Outcome

Media bytes use a stable, tenant-safe object contract that works with the approved provider set and can evolve without destructive overwrites.

## Current Cap reference

Cap stores recordings, segments, thumbnails, screenshots, avatars, and generated media through S3-compatible/custom/Google Drive paths. Frame has only an in-memory ObjectStore and provisional R2 binding.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#02](./02-p0-resolve-r3-storage-target.md), [#06](./06-p1-shared-domain-api-contracts.md), [#07](./07-p1-control-plane-media-job-protocol.md)

## Scope

Define object roles, immutable versioned key layout, metadata/tags, content type, cache policy, checksums, capability negotiation, put/head/get/range/copy/delete/list, conditional operations, and R2/S3 adapters chosen by ADR 02.

### Out of scope

End-user multipart flows are issue 19; production object backfill is issue 20; lifecycle/security policy is issue 21.

## Deliverables

- [ ] A versioned object-key and manifest specification with tenant, video, role, revision, and safe filename rules.
- [ ] Provider-neutral storage and upload-broker ports with an explicit capability model.
- [ ] Approved provider adapters plus deterministic contract tests against local and hosted test buckets.
- [ ] Error taxonomy for not found, precondition, throttling, auth, quota, timeout, integrity, and provider outage.
- [ ] Compatibility mapping from every current Cap object role/key to the new layout.

## Acceptance criteria

- [ ] Keys cannot escape tenant/video namespaces, collide across revisions, or expose sensitive user-provided names.
- [ ] Adapters pass identical put/head/get/range/copy/delete/list and conditional-operation contract tests where capabilities claim support.
- [ ] Successful writes record byte count, content type, checksum, provider version/etag semantics, and correlation metadata.
- [ ] Retries never overwrite a different immutable object and deletes are idempotent.
- [ ] BYO/self-hosting parity follows ADR 02 and unsupported capabilities fail before an upload starts.

## Required test evidence

- Cross-provider contract-test report.
- Key collision, Unicode, length, traversal, and tenant-isolation property tests.
- Mapping sample covering each legacy object role.

## Risks and open questions

- Provider etags are not universally content hashes, especially multipart.
- Mutable keys plus CDN caching can serve stale or cross-version media.

## Rollout and rollback

Write only to a namespaced v1 prefix while legacy reads remain authoritative. Adapter selection and dual-read fallbacks remain feature-flagged.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
