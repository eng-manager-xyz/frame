# Multipart storage protocol v1

This document specifies the provider-neutral, locally testable core of issue 19. It builds on the
canonical `ScopedObjectKey`, immutable-write, and storage failure contracts in
[`storage-contract-v1.md`](storage-contract-v1.md). The control plane now implements the checked-in
R2/D1 adapter and brokered routes, but this document does **not** prove a real provider operation,
exercise a browser or desktop media player, or close issue 19's protected promotion gates.

## Threat model and client contract

Protocol version 1 supports browser, desktop, mobile, extension, and service clients through the
same brokered operation model. A client can ask the authenticated API to create an upload, persist
the returned upload ID and exact part geometry, put a part, list provider-verified parts after a
restart, complete, and finalize. Clients never receive a storage account credential. They must:

1. persist the upload ID, canonical key, total size, part size/count, expiry, and their local file
   identity before transferring a part;
2. reuse the same idempotency key and bytes for a retried command;
3. checksum every part locally and send the declared SHA-256 with the bytes;
4. list verified parts after restart and skip only exact provider receipts;
5. abandon the old session and request a new create grant after expiry.

The v1 control-plane transport is brokered through authenticated same-origin Worker routes. It
deliberately does not expose a presigned POST,
provider URL, temporary provider credential, or long-lived secret. This replaces Cap's S3 POST-form
assumption with an operation contract implemented over R2 multipart primitives. Whether a later
client path also supports an R2-signed PUT or narrowly scoped temporary credentials remains a
protected provider/security decision and is not claimed here.

Small immutable uploads continue to use `ObjectStoreV1` plus `ImmutableStorageService`; the v1
multipart boundary covers large/resumable objects. Download bodies cross the provider port as a
boxed, single-consumer asynchronous pull stream. Upload-part bodies in this provider-free v1 core
remain one bounded `Vec<u8>` per part and are rejected above the declared broker/Worker request
limit. That is an intentional buffering contract, not a claim that an entire recording is buffered
or that Worker request streaming is proven. A real adapter still needs hosted request/response
streaming and memory evidence. The deterministic adapter chunks or buffers small test vectors only.

## Authorization grants

Each grant is bound to exactly:

- one tenant and canonical object key;
- one operation (`create`, `list_parts`, `put_part`, `complete`, `finalize`, `abort`, `head`, `get`,
  or `range`);
- one upload ID for upload-mutating operations other than create;
- an issued-at time, half-open expiry, and non-zero verification-key version.

The journal stores only `HMAC-SHA-256(key, framed(domain, key_version, secret))`. Domain, key
version, and secret are independently length-delimited. Presented bearer material and HMAC key
material have redacted `Debug`/`Display`; raw secrets are exposed only to the HMAC boundary. Digest
comparison is constant-time. A rotation ring can retain an old key for bounded overlap; new grants
must use the active version. Removing an old key immediately rejects its remaining grants, and an
individual journal grant can also be revoked. Altered, expired, revoked, retired-key, cross-tenant,
wrong-key, wrong-upload, and wrong-operation presentations all return the same `not_found` class
before provider I/O, hiding object and capability existence.

The authorization is a short-lived application grant, not an object-store credential. It cannot be
used outside the exact application operation and key scope. The returned durable grant ID must
equal the requested ID, and a create grant can claim exactly one create idempotency record.

## Transfer geometry and adapter boundary

`MultipartLimitsV1` binds minimum and maximum part size, maximum part count, maximum total size,
maximum broker/Worker request size, and maximum grant lifetime. An immutable upload specification
derives its part count by ceiling division. Every non-final part must have the exact declared part
size; the final part must have the exact remaining size. The application intersects those limits
with the provider's declared minimum/maximum part size, part count, total size, range size, supported
operations, and mandatory SHA-256 capability before creating provider state.

`MultipartObjectStoreV1` is the external adapter boundary. Create-session lookup, create, list,
put-part, complete, abort, HEAD, and GET responses repeat all relevant bindings. An adapter must
look up and validate the upload ID, key, and opaque provider handle before capability rejection or
body hashing for put-part. The application rejects a nominally successful response if it changes an
upload ID, key, part number, byte count, part or full-object checksum, content type, expiry,
provider handle, provider version/etag, range, or correlation ID. Raw part/download bytes are
omitted from all generic debug output.

The deterministic provider fake verifies actual SHA-256 values, exact part geometry, ordered
completion receipts, immutable destination creation, and conditional/range reads. Duplicate create
for the same upload specification, duplicate identical part, and duplicate complete are stable.
Changed replays fail with `precondition_failed`. Abort is stable as `aborted`, `already_aborted`, or
`already_completed`. Complete and abort share one linearizable state lock, so only one can win.
One-shot failures can be injected after tenant/reference checks; an unauthorized probe cannot
consume a fault meant for an authorized request.

## Restart journal and reconciliation

The durable contract is:

```text
creating -> uploading -> provider_completed -> finalized
                 \-> aborted
creating ---------> aborted (stale cleanup)
```

`MultipartJournalV1` atomically claims create idempotency, stores the provider session, records
verified part receipts, records provider completion, records immutable source identity, and records
abort. Client idempotency and reconciliation use structurally distinct replay namespaces: system
keys contain a typed operation plus upload ID and are never parsed from or accepted by client
command DTOs. A client string resembling a reconciliation key therefore cannot poison system work.
Each replay also carries a length-framed command fingerprint. Semantic replay compares every durable
command field except the request correlation ID and rebinds the returned result to the retry
correlation. For finalization, a valid caller timestamp is a first-writer durable result rather than
part of command identity: concurrent first finalizers may propose different timestamps, but every
proposal must be at least provider last-modified and every caller receives the exact committed
timestamp.
Terminal complete/finalize/abort paths still claim or verify their replay key, so operation and
fingerprint collisions cannot bypass validation. The deterministic journal is linearizable and
intentionally separate from the provider fake so process restart can be simulated by constructing a
fresh application service over the same two ports.

Provider object completion and D1 metadata finalization cannot be atomic. The
`provider_completed` state is the explicit reconciliation fence. These crash windows are safe:

- create journaled, provider create failed: look up the upload ID and create only on an authoritative
  miss; a lookup error never falls through to a second create;
- provider create returned but session activation failed: lookup recovers the same opaque handle,
  while expiry aborts that recovered provider session before journaling `aborted`;
- provider part accepted, journal write failed: repeat the same part, receive the stable provider
  receipt, then journal it;
- provider complete succeeded, journal write failed: repeat complete, receive the same immutable
  object identity, then record `provider_completed`;
- `provider_completed` recorded, finalize failed: reconciliation writes the source identity and
  trusted media probe without touching object bytes.

Reconciliation also lists incomplete candidates and aborts expired `creating` or `uploading`
sessions. It never aborts a provider-completed object. The final source record contains the exact
canonical key, provider version and etag, size, SHA-256, content type, provider last-modified,
finalization timestamp, and a server-side trusted media probe (container, audio/video codecs,
dimensions, duration, and frame rate). Finalization must not predate provider last-modified; the
application checks this before mutation and the journal checks it again atomically. These values are
copied from the validated provider/probe response, never from a client finalize claim, and give
issue 28 a bounded preflight input.

## Private download behavior

HEAD, full GET, and ranged GET require distinct operation grants. The provider returns typed
metadata; the application derives the public response headers instead of forwarding arbitrary
provider headers:

- exact `Content-Type` and `Content-Length`;
- `Content-Range` only for a validated partial response, with both the exact range and total size;
- opaque ETag and last-modified validator values;
- `Accept-Ranges: bytes`;
- a closed `inline` or `attachment` disposition with no user filename;
- the configured cache policy;
- an exact allowlisted HTTPS `Access-Control-Allow-Origin` and `Vary: Origin` when an origin is
  present.

Before provider I/O, the application must resolve a durable finalization by canonical key, load its
upload ID, and verify the finalized journal snapshot. Provider size, SHA-256, content type, version,
etag, and last-modified must then exactly equal that durable identity before a body is exposed.
Unfinalized objects are not downloadable.

`If-Match` mismatch returns `precondition_failed`; matching `If-None-Match` returns a bodyless
not-modified result. A range must be non-empty, within the object, equal the returned range, and no
larger than the provider limit. The application stream wrapper rejects empty or over-limit chunks,
cumulative overflow, early EOF, extra bytes, and midstream errors. Full GET incrementally hashes
against the durable finalization checksum. That SHA comparison is a terminal stream result: earlier
chunks may already have been consumed before a final checksum error is reported. The wrapper owns
the provider stream, supports explicit cancellation, and releases it on cancellation, validation
failure, completion, or `Drop`. A disallowed CORS origin is rejected before provider I/O and
therefore cannot disclose existence.

## Compatibility, rollout, and rollback

During migration, supported clients map legacy operations as follows:

| Legacy behavior | Protocol v1 behavior |
| --- | --- |
| presigned POST form | authenticated create plus brokered single PUT (`ObjectStoreV1`) or multipart |
| multipart create | idempotent journal claim plus provider create |
| upload part retry | same upload/part/idempotency key/checksum/bytes |
| restart | list provider-verified parts and rebuild the local journal |
| multipart complete | provider complete, durable `provider_completed`, then finalize |
| signed/private read | short-lived HEAD/GET/range operation grant and same-origin response |

Rollout must be limited to test tenants and v1 canonical keys. Legacy endpoints remain authoritative
during a measured compatibility window. Rollback stops new v1 creates, lets completed objects
finish reconciliation, aborts only incomplete test sessions, and restores legacy client routing; it
must not delete a provider-completed object.

## Protected evidence still pending

The following work is required before issue 19 can close:

- Wrangler-local and hosted evidence for the constructed Cloudflare R2/Worker multipart adapter's
  actual create/list/part/complete/abort/checksum/etag behavior, lost-response recovery, and
  documented provider limits;
- an approved decision and implementation for same-origin brokering versus R2-supported signed PUT
  or temporary credentials, including signing-key rotation and emergency revocation;
- hosted D1 durability/contention evidence for the checked-in session, part, post-stream
  verification, completion, and scheduled reconciliation records, plus any remaining client-journal
  parity work;
- hosted quota, expiry, Worker body/time/memory, retry/backoff, stale-upload cleanup, and concurrent
  load evidence;
- exact production/custom-domain CORS, TLS, cache, content-disposition, and private-access policy;
- browser, desktop, mobile, extension, and service compatibility traces, including player range and
  validator traces after restart;
- security-owner approval of the grant, tenant-isolation, logging, credential, and download model;
- test-tenant rollout, rollback, and legacy endpoint retirement evidence.

Until those records exist, this protocol is a provider-free contract foundation only. It must not
be cited as R2, browser playback, capacity, or production-security evidence.
