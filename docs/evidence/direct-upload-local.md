# Browser-direct R2 upload: local evidence

Status: local contract implemented; hosted Cloudflare R2 gate remains
protected.

## Implemented boundary

`POST /api/v1/uploads/intents` defaults to `transfer_mode: "brokered"`. A
caller may explicitly request `transfer_mode: "direct"` only with a lowercase
SHA-256 and a supported source type. The command still requires bearer auth,
same-origin fetch semantics, `x-frame-tenant-id`, bounded JSON content length,
an idempotency key, active tenant membership, current D1 mutation authority,
and an active R2 integration.

For a direct intent the Worker creates a new UUID, hashes the tenant into an
opaque scope, and signs exactly one private key of the form
`uploads/<64-hex-scope>/staging/<upload-uuid>.<type-extension>`. SigV4 binds
PUT, key, five-minute expiry, exact content length/type, `If-None-Match: *`,
the R2 SHA-256 checksum header, and matching Frame SHA-256 metadata. The public
response contains the capability and `/api/v1/uploads/<id>/finalize`; it never
contains the canonical tenant object key or signing credential. Direct rows
are rejected by the brokered `/content` route.

Finalize is an authenticated, tenant-scoped, origin-checked, idempotent JSON
command. It HEAD-verifies the exact staging key, byte length, content type,
content encoding, provider SHA-256, and Frame checksum metadata. It then
authorizes immutable storage, reserves quota, advances the ordered upload
state, streams staging to the canonical private key with no-overwrite
semantics, HEAD-verifies the canonical object, and atomically records upload,
manifest, governed-object, video, idempotency, and outbox state. Retries can
only accept the same checksum and exact canonical object. Expired staging is
deleted by a scheduled one-shot cleanup receipt; unfinished state becomes
aborted. A capability reused before expiry cannot overwrite the staging
object while it exists and can never overwrite or change the canonical
object/complete tenant state.

## Local verification

```text
cargo check -p frame-control-plane --tests
python3 scripts/ci/direct-upload-sqlite-conformance.py
```

The SQLite suite loads migrations 0001–0023 and proves direct/brokered field
separation, missing/unsafe field rejection, unique and immutable staging keys,
ordered initiated → uploading → finalizing → complete state, and one-shot
expiry selection. Rust unit tests prove signature changes for key, type,
checksum, exact byte size, and timestamp; reject unsafe paths, sizes, types,
checksums, and TTLs; and ensure debug output redacts the signed URL and header
values.

## Protected evidence still required

- apply the reviewed CORS/lifecycle plan to a non-production private bucket;
- run valid PUT + HEAD against hosted R2 and confirm provider checksum/custom
  metadata behavior;
- run wrong origin, method, content length/type/checksum/header, canonical-key,
  listing, overwrite, cross-tenant, expiry, replay, quota, and abandoned-object
  controls;
- prove no media body crosses Render and no signing secret reaches Render,
  browser logs, D1 diagnostics, CI artifacts, or Terraform plans;
- attach provider request IDs and redacted plan/probe output before checking
  issue 41's protected acceptance criteria.
