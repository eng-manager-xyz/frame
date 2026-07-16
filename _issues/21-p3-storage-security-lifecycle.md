---
title: "Enforce tenant isolation, CORS, custom domains, lifecycle, retention, legal hold, and deletion"
labels:
  - "phase:p3"
  - "area:storage"
  - "area:security"
  - "area:ops"
  - "type:security"
  - "risk:high"
depends_on: [14, 18, 19]
size: epic
---

# 21 · Enforce tenant isolation, CORS, custom domains, lifecycle, retention, legal hold, and deletion

## Outcome

Stored media follows one enforceable security and data-governance policy from upload through cache, retention, hold, export, and verified deletion.

## Current Cap reference

Cap supports private/shareable content, custom domains, custom buckets, organization controls, deletion, and multiple generated objects. R2 is strongly consistent at the object layer, but custom-domain caching can retain stale overwrites, deletes, or negative responses.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#14](./14-p2-organizations-rbac-spaces-folders.md), [#18](./18-p3-object-storage-adapter-key-contract.md), [#19](./19-p3-multipart-upload-download.md)

## Scope

Define tenant/object authorization, bucket/public-access policy, CORS, custom-domain ownership, cache keys/purge/versioning, malware/untrusted-media handling, encryption/keys, retention, legal hold, tombstone, cascade, erasure verification, quota, and audit.

### Out of scope

Application privacy UI is issue 32; organization policy storage is issue 14; provider choice is ADR 02.

## Deliverables

- [ ] Storage threat model and object-role access matrix.
- [ ] Policy-as-code or centralized checks for reads, writes, listing, copy, signing, deletion, and custom domains.
- [ ] Immutable-key/cache-purge strategy with stale-delete, overwrite, and cached-404 behavior tested.
- [ ] Lifecycle and deletion workflow covering source, outputs, segments, thumbnails, captions, avatars, manifests, multipart sessions, and backups.
- [ ] Legal-hold, export, restore, erasure-proof, quota, and incident runbooks.

## Acceptance criteria

- [ ] Cross-tenant and unlisted-object enumeration attempts fail at every direct, signed, cached, and custom-domain path.
- [ ] Deleting or changing privacy makes content inaccessible within the approved cache/SLO window, including prior negative/positive cache entries.
- [ ] Legal hold prevents lifecycle and user deletion while preserving auditability; release resumes the policy deterministically.
- [ ] Deletion is idempotent, reconciles all derived objects, and produces a privacy-safe completion record.
- [ ] CORS, content disposition, sniffing protection, CSP, range responses, and untrusted-media processing pass security review.

## Required test evidence

- Storage authorization and cache-behavior penetration tests.
- Timed delete/restore/hold rehearsal.
- Lifecycle inventory demonstrating no orphan role is omitted.

## Risks and open questions

- Public bucket or overly broad signed URLs can bypass application authorization.
- Cache behavior can contradict strongly consistent origin semantics.

## Rollout and rollback

Apply policies to new v1 prefixes first, audit existing access, then migrate per tenant. Rollback disables new signing and restores the prior read path without making private objects public.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
