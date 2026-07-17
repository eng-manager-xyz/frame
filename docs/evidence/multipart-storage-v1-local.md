# Multipart storage v1 local evidence

Date: 2026-07-16

Scope: local portions of issue 19 only. This record covers the versioned multipart domain,
provider and restart-journal ports, deterministic adversarial fakes, application orchestration,
the control-plane R2/D1 call path, authorization abuse tests, and the private download response
contract described in
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

No Cloudflare account, hosted R2 bucket, deployed Worker, hosted provider multipart session, signed
PUT, temporary credential, hosted D1 database or scheduler, custom domain, browser, desktop app,
media player, large recording, capacity run, production log sink, or security-owner approval was
exercised. The deterministic fake and Wrangler-local provider cannot establish hosted R2
durability, latency, Worker memory safety, browser playback, production CORS, or operational
readiness. Upload parts remain bounded buffers at this local port. Those protected/provider gates
remain explicitly open in the architecture document; this record must not be used to close issue
19.

## Control-plane wiring audit

The local repository now contains the complete server call path that was previously missing:

- a capability-gated multipart upload intent returns exact 16 MiB geometry for brokered recordings
  up to the bounded 2 GB native-probe ceiling;
- authenticated create, list/resume, part PUT, complete, and abort routes construct
  `R2MultipartObjectStoreV1` with the real `RECORDINGS` and `DB` bindings while keeping the opaque R2
  upload handle server-side;
- each part route enforces canonical part numbering, exact content length, identity encoding,
  `application/octet-stream`, and a full SHA-256 before provider dispatch;
- completion streams and hashes the full R2 object, stores an immutable D1 verification receipt,
  and then accepts only an exact `verified_native_probe` row. Client-supplied codec, duration,
  dimensions, and frame-rate claims never enter this boundary;
- the scheduled path replays a `completing` session, idempotently bootstraps the native probe job
  from the immutable verification receipt, and reconciles stale `open` sessions. Completion replay
  uses a durable attempt journal with 15-minute leases, bounded retry/backoff, permanent-failure
  quarantine, expired-final-lease terminalization, and fair due ordering; an older integrity failure
  therefore cannot starve the next completing row. Cleanup first
  checks for a completed object: a present object moves back to `completing`; an abort success or
  authoritative not-found expires the session; every other provider failure retains a durable
  retryable abort record with bounded backoff; and
- completion returns a server-derived immutable object version that the versioned Instant finalize
  request binds.

The offline D1 check is reproducible with:

```text
python3 -I scripts/ci/r2-completion-reconciliation-sqlite-conformance.py
python3 -I scripts/ci/instant-finalize-sqlite-conformance.py
```

Result: passed. The completion proof quarantines a permanently invalid oldest row and selects the
later row, enforces retry backoff and the 12-attempt ceiling, terminalizes a Worker crash after the
final lease, proves a post-0031 N-1 open-to-completing write is scheduler-visible before the 0033
contract fence, terminalizes exact N-1 completions with and without a matching concurrent active
claim, validates the same pre-existing-row backfill while quarantining a mismatched receipt, and
proves lost-ack recovery cannot roll the journal clock backward. The mixed-version SQLite proof
also verifies that 0028 leaves N-1 SQL writable while new provider mutations stay fenced, that
0033 refuses without mutating an open legacy session, and that only a fully drained database
persists the immutable assertion and enables claim-aware writes. The Instant proof
rejects invalid multipart geometry and a forged verification receipt, proves the
post-stream receipt cannot be mutated, retains retryable provider-abort failures without expiring
the session, rejects premature terminal cleanup, proves fair bounded finalize scanning and asserted
dead-letter transitions, rejects a cross-tenant Instant request, and reaches one exact
upload/object/job/playable publication postcondition. The focused Rust client and control-plane unit
suites, migration application, route-classifier tests, and warnings-denied Clippy checks also
passed.

The provider-backed local contract is reproducible with:

```text
python3 -I scripts/ci/r2-storage-conformance.py \
  --evidence target/evidence/r2-storage-conformance.json
```

Result: passed through the compiled Rust/Wasm Worker with Wrangler 4.111.0, all checked-in D1
migrations, an isolated local R2 bucket, and no Cloudflare credentials. Two complete runs exercise
multipart create and concurrent exact create replay; 5 MiB and tail part upload; exact part replay;
persisted part listing; an atomic race between conflicting bytes for the same part, with exactly one
claim allowed to reach a durable receipt; contiguous completion; full-object SHA-256 verification;
concurrent exact completion replay; private HEAD and a range crossing the part boundary; abort and
repeated abort;
an actual concurrent complete/abort race with exactly one durable provider-state winner;
stale-session cleanup; and cross-tenant rejection. The run also discovered and now guards the real
R2 upload-ID length case: the server-only handle is a fixed-size domain-separated digest while the
original provider ID remains only in D1.

This is Wrangler-local provider execution evidence, not hosted Cloudflare execution evidence. It
does not exercise lost acknowledgements, hosted etag/version behavior, provider quotas or failures,
concurrent hosted requests, or Worker body/time/memory limits. Those protected gates remain open,
and this record still must not be used as a production-readiness claim for issue 19.
