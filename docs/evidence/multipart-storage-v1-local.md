# Multipart storage v1 local evidence

Date: 2026-07-16

Scope: provider-free portions of issue 19 only. This record covers the versioned multipart domain,
provider and restart-journal ports, deterministic adversarial fakes, application orchestration,
authorization abuse tests, and private download response contract described in
[`docs/architecture/multipart-storage-v1.md`](../architecture/multipart-storage-v1.md).

## Commands and results

```text
cargo fmt --package frame-domain --package frame-ports --package frame-application -- --check
```

Result: passed.

```text
cargo test -p frame-domain -p frame-ports -p frame-application --locked
```

Result: 149 tests passed, 0 failed: 66 application tests (53 unit and 13 multipart integration), 41
domain unit tests, and 42 ports tests (23 unit, 3 external multipart adapter, and 16 storage
contract). All three doc-test suites also passed.

```text
cargo clippy -p frame-domain -p frame-ports -p frame-application --all-targets --locked -- -D warnings
```

Result: passed with warnings denied.

```text
cargo check -p frame-domain -p frame-ports -p frame-application --target wasm32-unknown-unknown --locked
```

Result: passed for all three crates.

The final owned-file diff check, whitespace scan, and isolated-index repository secret scan also
passed after this evidence file was updated.

## Behaviors exercised locally

- exact part-count derivation, non-final/final part sizes, total/count/Worker/provider limits, and
  actual per-part plus full-object SHA-256 checks;
- HMAC-SHA-256 RFC 4231 and fixed protocol vectors, length-framed domain/version/secret separation,
  constant-time digest comparison, overlap rotation, active-key issuance, individual revocation,
  and retired-key rejection;
- altered, expired, cross-tenant, wrong-key, wrong-upload, and wrong-operation grant opacity before
  provider faults are consumed;
- idempotent create, one-create-claim-per-grant enforcement, sparse provider-verified restart
  resume, exact part replay, changed replay rejection, complete replay, finalize replay, and abort
  replay;
- structural client/system replay namespaces, client text resembling old reconciliation keys,
  terminal operation/fingerprint collisions, and semantic replay rebound to fresh correlations;
- injected crash windows after provider create/part/complete and before journal/finalize state,
  followed by safe retry or reconciliation without re-uploading verified bytes;
- create-response/activation crash recovery by provider lookup, transient lookup fail-closed
  behavior, expiry abort, and a zero-live-provider-session assertion;
- stale `creating` and `uploading` cleanup, plus provider-completed preservation;
- a real two-task complete/abort race with a linearizable terminal result;
- barrier-forced concurrent first finalization and concurrent reconciliation finalization with
  different correlations/timestamps, one shared durable timestamp, unpoisoned replay namespaces,
  and exactly one provider completion;
- hostile provider and journal success responses that swap or corrupt
  create/list/part/complete/abort/download bindings, all rejected as integrity failures, followed
  by safe correct-response recovery;
- external-crate implementations of both provider and journal traits and construction/accessor
  coverage for private request/response fields plus the boxed download-body pull trait;
- HEAD, full GET, range GET, `If-Match`, `If-None-Match`, ETag, last-modified, content length/range,
  content type/disposition, cache policy, exact HTTPS CORS allowlisting, durable-finalization gating,
  and exact provider metadata matching;
- streamed download rejection for empty/oversized chunks, early EOF, extra bytes, midstream errors,
  and terminal full-object checksum drift, plus explicit cancellation and `Drop` release;
- redaction checks proving grant secrets, HMAC keys, provider handles, idempotency values, canonical
  object paths, and raw media bytes do not enter generic debug output.

## Evidence this record does not provide

No Cloudflare account, R2 bucket, Worker binding, real provider multipart session, signed PUT, temporary
credential, D1 journal, hosted reconciler, custom domain, browser, desktop app, media player, large
recording, network/runtime streaming body, capacity run, production log sink, or security-owner
approval was exercised. The deterministic fake pulls chunks from small in-memory vectors and cannot
establish R2 compatibility, durability, latency, Worker memory safety, browser playback, CORS
correctness in production, or operational readiness. Upload parts remain bounded buffers at this
local port. Those protected/provider gates remain explicitly open in the architecture document;
this record must not be used to close issue 19.

## Repository wiring audit

The later control-plane work adds two distinct R2 implementations, but only one currently has a
production call path:

- `r2_direct_upload.rs` is constructed by upload intent, signs a private staging PUT, and is consumed
  by the authenticated direct-finalize path. That path verifies R2 size, SHA-256, content type, and
  metadata before an immutable publication and D1 mutation.
- `r2_multipart.rs` implements the `MultipartObjectStoreV1` provider operations and has D1 tables for
  sessions, parts, and completion receipts. It is exported by the Worker crate, but no runtime code
  constructs `R2MultipartObjectStoreV1`, no API route invokes it, and no scheduled task invokes its
  stale-session cleanup.

The missing constructor is not a cosmetic wiring omission. Multipart completion requires a
`TrustedR2MediaProbeV1`; the repository has no implementation of that trait. The existing native
probe is an asynchronous media-job result produced only after a source object manifest exists, so it
cannot be injected into the current synchronous multipart-complete contract without defining a
durable provider-completed/reconcile boundary. The current upload-intent path therefore returns
`multipart_required` for objects above the single-PUT ceiling rather than creating a multipart
session.

Only deterministic helper tests exist beside the R2 multipart adapter. There is no Wrangler-local
or hosted test that constructs the adapter and exercises create/list/part/resume/complete/abort,
lost acknowledgement, expiry cleanup, or checksum failure through the Worker binding. Consequently
the checked-in adapter and migrations are useful implementation groundwork, but they do not satisfy
issue 19's provider-adapter deliverable or acceptance evidence yet.
