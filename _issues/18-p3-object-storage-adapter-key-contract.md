---
title: "Define versioned object keys and implement the Cloudflare R2 storage adapter"
labels:
  - "phase:p3"
  - "area:storage"
  - "area:rust"
  - "type:migration"
  - "risk:high"
depends_on: [02, 06, 07]
size: epic
---

# 18 · Define versioned object keys and implement the Cloudflare R2 storage adapter

## Outcome

Media bytes use a stable, tenant-safe Cloudflare R2 contract that can evolve without destructive overwrites and can safely cache Cloudflare Media or GStreamer outputs.

## Current Cap reference

Cap stores recordings, segments, thumbnails, screenshots, avatars, and generated media through S3-compatible/custom/Google Drive paths. Frame has an in-memory `ObjectStore` and a confirmed R2 binding but no production adapter.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#02](./02-p0-establish-r2-storage-target.md), [#06](./06-p1-shared-domain-api-contracts.md), [#07](./07-p1-control-plane-media-job-protocol.md)

## Scope

Define object roles, immutable versioned key layout, metadata/tags, content type, cache policy, checksums, capability negotiation, put/head/get/range/copy/delete/list, conditional operations, and the R2 adapter chosen by ADR 02. Derivative keys must incorporate the immutable source version, normalized transform profile, and profile version so managed/native retries can reuse results.

### Out of scope

End-user multipart flows are issue 19; production object backfill is issue 20; lifecycle/security policy is issue 21.

## Deliverables

- [ ] A versioned object-key and manifest specification with tenant, video, role, revision, and safe filename rules.
- [ ] Provider-neutral storage and upload-broker ports with an explicit capability model.
- [ ] A production R2 Worker adapter plus deterministic contract tests against local and hosted test buckets; additional adapters only if approved by issue 02.
- [ ] A derivative manifest recording source version/checksum, executor, transform profile/version, output key/checksum/content type, attempt, and creation time without leaking credentials.
- [ ] Error taxonomy for not found, precondition, throttling, auth, quota, timeout, integrity, and provider outage.
- [ ] Compatibility mapping from every current Cap object role/key to the new layout.

## Acceptance criteria

- [ ] Keys cannot escape tenant/video namespaces, collide across revisions, or expose sensitive user-provided names.
- [ ] The R2 adapter passes put/head/get/range/copy/delete/list and conditional-operation contract tests; compatibility adapters pass the same tests wherever they claim support.
- [ ] Successful writes record byte count, content type, checksum, provider version/etag semantics, and correlation metadata.
- [ ] Retries never overwrite a different immutable object and deletes are idempotent.
- [ ] Equivalent Cloudflare Media and GStreamer requests resolve to stable, collision-resistant output keys and can HEAD/reuse completed results.
- [ ] BYO/self-hosting parity follows ADR 02 and unsupported capabilities fail before an upload starts.

## Required test evidence

- Local and hosted R2 contract-test report plus any approved compatibility-adapter report.
- Key collision, Unicode, length, traversal, and tenant-isolation property tests.
- Mapping sample covering each legacy object role.

## Risks and open questions

- Provider etags are not universally content hashes, especially multipart.
- Mutable keys plus CDN caching can serve stale or cross-version media.
- Omitting executor/profile provenance makes managed-output drift and replay cost impossible to diagnose.

## Rollout and rollback

Write only to a namespaced v1 prefix while legacy reads remain authoritative. Adapter selection and dual-read fallbacks remain feature-flagged.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
