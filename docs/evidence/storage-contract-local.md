# Storage contract v1 local evidence

Date: 2026-07-16

Scope: provider-free portions of issues 02 and 18 only. This record covers the typed object-key,
derivative-manifest, object-store/upload-broker port, deterministic fake, and application
capability-preflight slice described in
[`docs/architecture/storage-contract-v1.md`](../architecture/storage-contract-v1.md).

## Commands and results

```text
cargo fmt --package frame-domain --package frame-ports --package frame-application
```

Result: clean formatting; `git diff --check` passed for the owned files.

```text
cargo test -p frame-domain -p frame-ports -p frame-application
```

Result: 125 tests passed, 0 failed (50 application unit tests, 36 domain unit tests, 23 ports unit
tests, and 16 storage contract integration tests), followed by clean doc tests for all three crates.

```text
cargo clippy -p frame-domain -p frame-ports -p frame-application --all-targets -- -D warnings
```

Result: passed with warnings denied.

```text
cargo check -p frame-domain -p frame-ports -p frame-application --target wasm32-unknown-unknown
```

Result: passed for the Worker/browser-compatible target.

## Behaviors exercised

- canonical key round trips and rejection of user basenames, Unicode, traversal, extra segments,
  non-canonical revisions, and uppercase extensions;
- collision separation across tenant, video, source revision, normalized profile, bound output
  descriptor, and profile version, with SHA-256 known-answer and framed-boundary vectors;
- immutable manifest scope/provenance checks, rejection of manifest-shaped output, and rejection of
  unknown wire fields and executors;
- put, head, get, range, copy, scoped pagination, provider-version fencing, conditional delete, and
  idempotent delete;
- barrier-backed atomic create contention in which exactly one different payload can claim an
  immutable key;
- cross-tenant failures hidden as not found before capabilities or injected faults are consumed;
- real byte-to-SHA-256 integrity validation with no partial object after failure;
- explicit capability rejection, actual-I/O-only fault injection, absent-object version-fence
  semantics, and one-shot throttling/error-taxonomy behavior;
- broker plan, completion, exact replay, changed-replay rejection, pending/completed abort semantics,
  cross-tenant/unknown abort indistinguishability, and a barrier-backed complete/abort race;
- HTTPS/direct-authorization validation, all-control-character and case-insensitive duplicate-header
  rejection, same-origin path validation, and secret/raw-byte-safe formatting;
- external-adapter construction and accessor probes for every private request/result field;
- application preflight proving unsupported store/upload modes and missing broker SHA-256 guarantees
  make zero broker calls, cross-tenant puts fail before capability/size inspection or adapter calls,
  plus hostile-adapter postcondition checks for write receipts, HEAD identity, and every upload-plan
  binding field.

## Evidence this record does not provide

No Cloudflare account, R2 bucket, Worker binding, Media Transformations binding, signed URL, real
provider etag/version, hosted network, durability, performance, lifecycle, residency, cost, or
security-owner approval was exercised. No S3-compatible, MinIO, Google Drive, self-hosted, or
user-owned bucket adapter was exercised or approved. The required protected/provider evidence is
listed in the architecture document and remains open; this local record must not be used to close
issues 02 or 18.

Separately, local issue 18 coverage is still incomplete for avatar storage, screenshots/other legacy
roles, typed metadata/tags, and a real legacy-key sample plus mapping. Those local gaps are listed
separately from protected R2 evidence in the architecture document.
