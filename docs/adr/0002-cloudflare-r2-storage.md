# ADR 0002: Use Cloudflare R2 as canonical object storage

- Status: accepted
- Date: 2026-07-15

## Context

Frame needs durable storage for source recordings, segments, thumbnails, previews, exports, and manifests. The Cloudflare control plane can access R2 through a first-class Worker binding, and Cloudflare Media Transformations can consume private R2 bodies without making source media public. Cap also has S3-compatible, MinIO, user-owned bucket, and Google Drive modes whose product disposition must remain explicit during migration.

## Decision

Use Cloudflare R2 as Frame's canonical hosted object store through the `RECORDINGS` binding. Store media under immutable, tenant-scoped, versioned keys and persist checksums and output manifests in D1. Domain and application code depend on provider-neutral `ObjectStore` and upload-broker ports so local fakes, contract tests, and any separately approved self-hosted or bring-your-own-storage adapters do not leak provider behavior into the domain.

Provider selection is settled. Issue 02 records the compatibility matrix for legacy S3-compatible, MinIO, user-owned bucket, and Google Drive modes; issue 18 implements the R2 adapter and key contract.

## Consequences

R2 binding semantics, multipart limits, conditional operations, range access, lifecycle, residency, access control, and cost become production design inputs. Provider etags must not be assumed to be content hashes. Compatibility adapters may expose fewer capabilities and must fail before an upload begins when a required operation is unsupported. Reversing this decision requires a new ADR and an object migration plan, not a domain-model rewrite.

## References

- [R2 Worker binding API](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/)
- [R2 presigned URLs](https://developers.cloudflare.com/r2/api/s3/presigned-urls/)
