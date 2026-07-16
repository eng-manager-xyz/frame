# Storage contract v1 local evidence

Date: 2026-07-16

Scope: locally achievable portions of issues 02 and 18 only. This record covers the typed
object-key and legacy-mapping contracts, derivative manifest, object-store/upload-broker port,
deterministic fake, application capability preflight, production R2 Worker-binding adapter, and
credential-free Wrangler-local R2 conformance slice described in
[`docs/architecture/storage-contract-v1.md`](../architecture/storage-contract-v1.md).

## Commands and results

```text
cargo fmt --package frame-domain --package frame-ports --package frame-application
```

Result: clean formatting; `git diff --check` passed for the owned files.

```text
cargo test -p frame-domain -p frame-ports -p frame-application
```

Result: 263 tests passed, 0 failed (114 application, 104 domain, and 45 ports unit/integration
tests), followed by clean doc tests for all three crates.

```text
cargo clippy -p frame-domain -p frame-ports -p frame-application --all-targets -- -D warnings
```

Result: passed with warnings denied.

```text
cargo check -p frame-domain -p frame-ports -p frame-application --target wasm32-unknown-unknown
```

Result: passed for the Worker/browser-compatible target.

Issue 18 legacy-key extension:

```text
cargo test -p frame-domain storage::tests:: --lib
```

Result: 14 storage tests passed, 0 failed. This includes the avatar, screenshot/caption,
eight-role legacy-mapping, and cross-runtime mapping-digest known-answer suites.

```text
python3 -I scripts/ci/check-workflow-policy.py
python3 -I scripts/ci/test-workflow-policy.py
```

Result: the workflow policy passed for all owned workflows, and all seven unsafe workflow
mutations—including removal of the required local R2 lane—were rejected.

```text
python3 -I scripts/ci/r2-storage-conformance.py \
  --wrangler-bin <cached-wrangler-4.111.0-js> \
  --evidence target/evidence/r2-storage-conformance.json
```

Result: passed through the compiled Rust/Wasm Worker with Wrangler 4.111.0 and no Cloudflare
credentials. The report records all seven adapter operations; immutable create, exact replay,
provider-version fencing, and opaque cross-tenant `not_found`; two complete contract runs;
idempotent cleanup; and exact route/method/IPv4-loopback guards. The generated report is a CI
artifact rather than a committed provider result, and its `not_claimed` list excludes hosted R2,
provider network/quota behavior, production access, durability, latency, residency, lifecycle, and
cost.

```text
cargo test -p frame-control-plane r2_storage::tests --lib
cargo test -p frame-control-plane \
  repository_conformance_route_is_exact_and_requires_ipv4_loopback --lib
cargo test -p frame-control-plane \
  production_hides_reserved_repository_route_before_route_specific_processing --lib
cargo clippy -p frame-control-plane --lib -- -D warnings
cargo check -p frame-control-plane --target wasm32-unknown-unknown
```

Result: 4 adapter helper/fence tests and both route-isolation tests passed; control-plane Clippy
passed with warnings denied; and the Worker target compiled successfully.

## Behaviors exercised

- canonical key round trips and rejection of user basenames, Unicode, traversal, extra segments,
  non-canonical revisions, and uppercase extensions;
- immutable user-scoped avatar round trips and collision separation across tenant, user, revision,
  and extension, with video/user namespace separation and redacted debug output;
- closed screenshot-source and caption-derivative roles, generated filenames, role/filename
  disagreement rejection, and profile-bound caption identity;
- representative legacy locators for recording, segment, thumbnail, screenshot, avatar, generated
  media, caption, and manifest across the declared provider inventory;
- strict legacy locator/metadata wire validation, role-to-target enforcement, redacted legacy keys,
  deterministic metadata-bound mapping digests, unknown-field rejection, and forged-digest
  rejection;
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
- compiled Worker/R2-binding put, exact replay/conflict, full typed metadata, head/get/range,
  same-scope conditional copy, cursor pagination, version-fenced/idempotent delete, and complete
  cross-tenant `not_found` coverage before provider access;
- production hiding plus exact local POST path, method, IPv4 loopback authority, and path-lookalike
  rejection, with a credential-refusing Wrangler-local runner and required source-bound CI report.

## Evidence this record does not provide

No Cloudflare account, hosted R2 bucket, Media Transformations binding, signed URL, real hosted
provider etag/version, provider network, durability, performance, lifecycle, residency, cost, or
security-owner approval was exercised. No S3-compatible, MinIO, Google Drive, self-hosted, or
user-owned bucket adapter was exercised or approved. The checked-in legacy samples prove the
mapping grammar, not completeness against private production inventory. The required
protected/provider evidence is listed in the architecture document and remains open; this local
record must not be used as a hosted migration or production-cutover claim.
