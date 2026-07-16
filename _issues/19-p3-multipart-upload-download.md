---
title: "Implement authorized multipart upload, resume, finalize, range download, and signed access"
labels:
  - "phase:p3"
  - "area:storage"
  - "area:api"
  - "area:security"
  - "type:migration"
  - "risk:high"
depends_on: [13, 18]
size: epic
---

# 19 · Implement authorized multipart upload, resume, finalize, range download, and signed access

## Outcome

Large recordings upload and play reliably through least-privilege, resumable protocols that match the selected object provider.

## Current Cap reference

Cap uses several presigned POST and multipart paths. Cloudflare R2 presigned URLs support GET, HEAD, PUT, and DELETE but not S3 POST form uploads, so a mechanical endpoint translation would fail. Large desktop recordings also need resume across process/network loss.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#13](./13-p2-auth-sessions-identity.md), [#18](./18-p3-object-storage-adapter-key-contract.md)

## Scope

Implement upload intent, direct/brokered selection, signed PUT or temporary credentials, multipart create/list/part/complete/abort, local upload journal, checksums, expiry, quota, idempotent finalize, authorized HEAD/GET/range, CORS, and client retry guidance.

### Out of scope

Bulk migration of existing objects is issue 20. Retention and legal-hold policy is issue 21.

## Deliverables

- [ ] Versioned upload protocol and threat model for browser, desktop, mobile, extension, and service clients.
- [ ] Provider adapter for single PUT and multipart upload with resume, abort, stale-session cleanup, and checksum verification.
- [ ] Short-lived, tenant/object/operation-scoped authorization with key rotation and revocation behavior.
- [ ] Byte-range and conditional download service suitable for media players and private/custom-domain content.
- [ ] Compatibility plan that replaces Cap's presigned POST flows without breaking supported clients.

## Acceptance criteria

- [ ] A multi-part recording resumes after client restart without reuploading verified parts and completes to one immutable object.
- [ ] Duplicate create, part, complete, finalize, and abort requests are safe and return stable outcomes.
- [ ] Part sizing, count, total size, URL expiry, Worker request, and provider limits are validated before transfer.
- [ ] An expired, altered, cross-tenant, wrong-method, or wrong-key authorization is rejected without disclosing object existence.
- [ ] Range, HEAD, cache validators, content type/disposition, CORS, and private access pass player and security tests.

## Required test evidence

- Interrupted upload/resume tests at multiple failure points.
- Authorization abuse and cross-tenant test report.
- Browser/desktop playback traces covering ranges and cache validators.

## Risks and open questions

- Never embed long-lived object credentials in clients.
- D1 finalize and object-store complete cannot be atomic; reconciliation is mandatory.

## Rollout and rollback

Enable for test tenants and new v1 keys. Keep legacy upload endpoints available during a measured client compatibility window; abort incomplete test uploads on rollback.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
