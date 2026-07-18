# Business data v1 local evidence

Date: 2026-07-16 (America/Los_Angeles)

Scope: Issue 15 locally provable behavior. Source reference is `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`. This record does not claim Worker, Wrangler, D1-provider, R2/S3/Google Drive, payment, email, browser, production migration, or human approval evidence.

## Source and contract inventory

- Exactly 20 pinned Cap source tables have an explicit retained or fail-closed disposition in migration 0011; `usage_ledger` is recorded separately as Frame-derived.
- A credential-free Cap plan contract covers 56 NanoID PK/FK identifier occurrences with one deterministic UUIDv8 transform. Its executable companion has 21 streams over the exact 20 source tables, including Notifications-to-Outbox, joined tenant scopes, and a separate ledger import order.
- The pinned `packages/database/schema.ts` checksum is `7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9`; its parity fixture freezes every source column and 39 intentional target-schema drifts.
- 20 data classes have explicit export, deletion, retention, and legal-hold policy.
- Messenger conversations, messages, and support-email rows are excluded, tenant-classified only when unambiguous, quarantined for bounded deletion, and blocked from new writes. The local administrative adapter proves tenant/deadline-guarded raw deletion plus quarantine transition and absence postcondition in one batch.
- Provider-neutral business domain, port, and application modules contain no JavaScript binding type or private media URL field.

## Offline SQLite result

Command:

```text
python3 scripts/ci/business-sqlite-semantic-conformance.py --json
```

Observed result:

```json
{"compiled_queries":84,"data_classes":20,"derived_aggregates":1,"dirty_findings":7,"exported_data_classes":20,"mapped_source_tables":20,"mode":"offline_sqlite","quarantined_messenger_rows":3,"sample_exports":1,"semantic_assertions":87,"statically_mapped_tables":20,"status":"ok"}
```

The suite proved clean and dirty pre-0011 upgrades, foreign-key integrity, query compilation, owner-only complete keyset export with a final revision/count fence, subject-to-tenant binding, no-mutation rollback after an injected failed postcondition, anonymous/private/cross-tenant denials, full-record immutable replay checks, comment and notification list/mark/delete surfaces, folder-aware shares, principal-scoped replay receipts, same-key deferred outbox convergence, canonical outbox/import/upload initialization, redacted failed-import diagnostics, tenant-scoped outbox/usage keys, exact storage/share/derivative postconditions, upload lifecycle advancement, digest-only developer keys, credit-account reads, legal-hold placement/release, actual secret-safe export rows, concrete class deletion, organization-only and app-scoped compensation eligibility, balanced credit/usage compensation entries with immutable originals, and tenant-bound fail-closed Messenger purge.

Additional isolated mapping commands:

```text
python3 -I scripts/migration/test_cap_id_map.py
python3 -I scripts/migration/test_etl_cap_id_map.py
python3 -I scripts/migration/test_cap_business_plan_contract.py
```

They observed four cross-runtime known answers, PK/FK stability, non-string and invalid-shape rejection, option-bearing plan rejection, 20 source tables, 21 executable streams, one Frame-derived aggregate, 56 transformed identifiers, joined tenant scopes, one-source-to-many targets, ordered ledger projection, and 39 complete intentional-drift records.

## Rust result

Non-colliding commands observed at this checkpoint:

```text
cargo check -p frame-domain -p frame-ports -p frame-application -p frame-control-plane
cargo test -p frame-domain business --lib
cargo test -p frame-application --test business_data_v1
cargo test -p frame-control-plane business_repository --lib
cargo clippy -p frame-domain -p frame-ports -p frame-application -p frame-control-plane --lib -- -D warnings
cargo check -p frame-control-plane --target wasm32-unknown-unknown
```

The four business libraries compiled under strict library clippy, all 12 domain business tests passed, all 6 application integration tests passed, all 3 filtered business control-plane tests passed, and the control plane compiled for `wasm32-unknown-unknown`. Repository-wide all-target clippy/format remains an integration gate across other issue-owned modules.

## Checked artifacts

- Migration SHA-256 at this checkpoint: `854ed9b15106f76edfbb49b72c777df4739a3265c5f039f23f5cbb096b35037e`.
- Sorted concatenated 84-query-file digest at this checkpoint: `25c12a73c0d9a791830b15b95adc286c6b8b4f50f541ce2470ad234d1d1b6c3d`.
- Deterministic zero-row export fixture: `business-data-sample-export-v1.json`.

Digests must be recomputed after any repair. They identify local checked-in inputs, not a deployed provider schema.

## Protected gaps

The following remain promotion gates, not local passes:

- current Wrangler execution against a named D1 database, including dirty upgrade, rollback, race, and lost-ack scenarios;
- storage provider reconciliation and delete/absence probes for R2, S3-compatible, MinIO, and Google Drive;
- managed Cloudflare and native GStreamer derivative parity against real objects;
- payment/credit source reconciliation and independent financial approval;
- authorized messenger legacy purge plus privacy/legal approval;
- tenant export/delete exercise under a real legal hold;
- production telemetry, alert, browser/API consumer, and operator sign-off.
