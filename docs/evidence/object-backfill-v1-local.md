# Object backfill v1 local evidence

Date: 2026-07-16

Scope: provider-free portions of issue 20 only. This record covers the immutable manifest, runtime
credential boundary, durable journal/CAS state machine, streaming provider ports, deterministic
adversarial adapters, application coordinator, independent reconciliation, dry-run repair plan,
and source-retention gate described in
[`object-backfill-v1.md`](../architecture/object-backfill-v1.md).

## Commands and results

```text
cargo test -p frame-domain -p frame-ports -p frame-application --locked
```

Result: 186 tests passed, 0 failed: 82 application tests (43 unit, 13 multipart integration, and 26
object-backfill integration), 59 domain unit tests, and 45 ports tests (26 unit, 3 external multipart
adapter, and 16 storage contract). All three doc-test suites passed.

```text
cargo clippy -p frame-domain -p frame-ports -p frame-application --all-targets --locked -- -D warnings
```

Result: passed with warnings denied.

```text
cargo check -p frame-domain -p frame-ports -p frame-application --target wasm32-unknown-unknown --locked
```

Result: passed for all three crates.

```text
RUSTDOCFLAGS='-D warnings' cargo doc -p frame-domain -p frame-ports -p frame-application --no-deps --locked
```

Result: passed with rustdoc warnings denied.

Direct `rustfmt --check` and `git diff --check` passed for every object-backfill-owned Rust and
documentation file. The parent goal owns the final aggregate `cargo fmt --all -- --check` and secret
scan after all concurrently produced issue files are formatted and staged; this record does not
claim those repository-wide results early.

## Two independent synthetic rehearsal records

The `two_independent_rehearsals_resume_idempotently_and_reconcile_exactly` test created two distinct
immutable manifests, deliberately stopped after the first entry, reconstructed the coordinator over
the same durable ports, completed the remaining entries, replayed completion, and generated a clean
independent reconciliation report for each. This point-in-time run emitted:

| Rehearsal | Provider path | Manifest SHA-256 | Report SHA-256 | Objects | Logical bytes | Result |
| --- | --- | --- | --- | ---: | ---: | --- |
| A | synthetic S3 to R2 | `f7a7cb23d73e74544efbcda2f49af158dce06c1a85c32894cc3aca273680e022` | `9a542b5d6daa5cf0b77b09a3d610f99513ceb99e7736d499d66f5064732509b9` | 3 | 86 | clean |
| B | synthetic MinIO to custom S3-compatible | `60b0b4dab7879d981ea00222e4eb9b9105976a8ddc6af095fef8859a4ef18a4d` | `1431426ba15cc75ca0e576746c480f3ac537fadc70c9b90cb645b0003317aade` | 2 | 58 | clean |

The IDs are UUIDv7 values, so a new run intentionally produces new manifest/report digests. The
table records the actual 2026-07-16 local run, not a stable known-answer vector. These are small
synthetic adapters and are not production-scale or provider evidence.

## Behaviors exercised locally

- immutable, versioned, unknown-field-denying manifest serialization and a length-framed digest
  over source/target authority fingerprints, non-secret locators, tenant/video/role, source
  references, canonical target key/revision, bytes, SHA-256, content type, probe policy, tool, and
  code versions;
- tampered manifest digest, duplicate entry/target, cross-scope target, signed URL, and unsafe
  provider-value rejection;
- credential references that implement no serialization, remain outside manifests, and redact both
  generic debug and display output;
- explicit proof that opaque multipart-style ETags are not accepted as strong SHA-256;
- immutable manifest replay, journal creation, exact CAS revision, lost journal-commit
  acknowledgement recovery, pause/resume/abort, retry scheduling, owner dispositions, and clean
  report plus owner-approval source-retention fencing;
- expiring leases, durable renewal during source copy and target verification, global live
  concurrency across entries/tenants, expired-lease exclusion, greater reclaim fencing tokens,
  stale-worker completion rejection, an expired tenant-A half-open lease reclaimed globally before
  tenant-B selection, exactly one live half-open probe, and two real competing async workers
  producing one claim and one target object;
- one non-empty bounded chunk at a time, incremental source and target SHA-256/size, streaming media
  probe, bandwidth admission, explicit cancellation, staging cleanup, and no whole-object buffer in
  application code;
- interrupted midstream transfer, deterministic backoff/resume, and exactly one eventual target;
- destination commit followed by a lost acknowledgement, exact full target post-read recovery, and
  no duplicate create;
- a persisted claimed operation representing process death after provider commit/before journal
  success, with normal attempts plus entry/byte/cost/concurrency budgets exhausted, followed by
  exact operation-provenance recovery with zero new attempts, charges, or creates, while an
  operation-bound but corrupt target is never checkpointed as recovered;
- a destination writer blocked immediately before publication while abort or stale reclaim wins the
  journal race, with the live commit fence rejecting publication and cleanup counters proving read
  release/write cancel;
- injected truncation, byte corruption, extra bytes, invalid empty-chunk response, over-policy
  chunk, midstream outage, provider authorization expiry, object-rate throttle, byte-rate throttle,
  outage circuit, and cancellation;
- fresh injected-clock use across a transfer lasting multiple lease TTLs, explicit rollback
  rejection, source-plus-target provider/region throttle observations, and summed source-egress plus
  target-write cost accounting;
- consecutive-only provider circuit counting, reset on a non-provider outcome, exactly one
  half-open probe, and exactly one bounded owner-approved extra retry;
- reference approval that retains fresh streamed source verification while deliberately requiring
  no target; exclusion approval for missing and corrupt sources that removes them from migrated
  totals while preserving auditable object/byte/role disposition totals; deterministic disposition
  ordering/digests; exact journal/report disposition matching at source release; and forged/stale
  approval records, invalid/expired opaque capabilities, forged reports, and stale reports rejected
  before disposition or source release;
- capability preflight failure before manifest/journal/provider mutation;
- independent two-pass snapshot source/target inventory over 501 objects, strict tenant isolation,
  and fail-closed snapshot mutation, wrong page index, repeated/backward cursor, empty continuation,
  and cross-tenant row injection;
- classification of missing source/target, duplicate source/target, orphan source/target, ownership
  mismatch, corrupt target, metadata drift, and Created-receipt operation/version checkpoint drift;
- distinct deterministic redacted object fingerprints for equal-kind discrepancies and actionable
  dry-run repair-plan generation with a zero-mutation assertion; and
- streaming playback/probe success for all eight local object roles, while the rehearsal/provider
  matrix collectively exercises synthetic S3, R2, MinIO, custom S3-compatible, and Google Drive
  provider classes.

## Evidence this record does not provide

No real Cap-managed S3 account, MinIO server, customer BYO/custom bucket, Google Drive API,
Cloudflare R2 bucket/Worker binding, D1 journal, provider SDK stream, production credential, customer
permission, owner-approved disposition, real user-impact list, or source deletion was exercised.
No production-scale object/byte count, throughput, peak memory, error rate, egress, provider request,
or cost budget was measured. No real media corpus was probed or played back.

The deterministic provider stores small fixture bytes in adapter memory and can only prove the
application consumes a bounded stream. It cannot establish network backpressure, provider
pagination consistency, conditional-write semantics, durability, residency, permission, latency,
cost, or cleanup behavior. Real S3/MinIO/BYO/Drive/R2 credentials and all owner/security/provider
approvals remain protected and uncollected. This record must not be used to close issue 20 or to
authorize a production migration.
