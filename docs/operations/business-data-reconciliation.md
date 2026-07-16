# Business data reconciliation runbook

This runbook covers local validation, D1 rollout, reconciliation, deletion/export, and rollback for Issue 15. Provider and production steps are protected evidence and must not be recorded as passed until executed against the named environment.

## Local gate

Run:

```text
python3 scripts/ci/business-sqlite-semantic-conformance.py
python3 -I scripts/migration/test_cap_id_map.py
python3 -I scripts/migration/test_etl_cap_id_map.py
python3 -I scripts/migration/test_cap_business_plan_contract.py
cargo test -p frame-domain
cargo test -p frame-application --test business_data_v1
cargo test -p frame-control-plane --lib
cargo clippy -p frame-domain -p frame-ports -p frame-application -p frame-control-plane --all-targets -- -D warnings
cargo check -p frame-control-plane --target wasm32-unknown-unknown
cargo doc -p frame-domain -p frame-ports -p frame-application --no-deps
cargo fmt --all -- --check
```

The SQLite suite applies clean and dirty pre-0011 upgrades, compiles every business query, injects a failed postcondition, and exercises privacy, cross-tenant isolation, semantic replay, deferred events, storage scope, derivative manifests, digest-only keys, ledger arithmetic, usage scope, legal holds, export coverage, and messenger rejection. The isolated migration tests freeze the pinned Cap schema/column inventory, all intentional drifts, and the option-free NanoID-to-UUID transform. They do not prove D1-provider behavior.

## Pre-cutover audit

1. Apply 0011 in the migration rehearsal database and run `PRAGMA foreign_key_check` or the D1 equivalent.
2. Query `business_source_integrity_v1`. Any nonzero finding except an explicitly approved messenger quarantine blocks write cutover.
3. Compare `business_source_table_map_v1` with the pinned Cap source inventory. It must contain exactly 20 rows. Verify the separate `business_derived_aggregate_map_v1` row identifies `usage_ledger` as Frame-derived.
4. Backfill canonical document version/checksum in bounded batches. Unknown versions remain read-only and are not rewritten.
5. Reconcile each credit account by replaying transactions in ledger sequence and compare the terminal balance.
6. Join usage entries to organization/app/video/job; any unresolved or cross-tenant reference blocks cutover.
7. Compare every daily snapshot source checksum and charge total.
8. Drain or account for every deferred outbox/import/upload event.
9. Verify messenger quarantine deadlines and the tenant classification. Exercise the administrative purge adapter only after approval: its guarded raw delete, quarantine transition, and absence postcondition must commit together. Do not expose a product read endpoint, and do not purge a row whose organization is null or ambiguous.

## Aggregate rollout

Enable shadow reads in this order: videos/edits, shares/comments, notifications/outbox, storage/imports, developer data, then ledger/snapshots. Compare typed projections and counts by tenant. Enable writes for one internal tenant, wait through the retry/deferred-event window, reconcile, and widen gradually.

Managed and native derivative executors must both write `business_derivative_manifests_v1`. Compare source version, profile version, output role/key/checksum/content type, usage, and cost. A provider completion without a matching storage object is a failed postcondition, not success.

## Duplicate, gap, and lost-ack recovery

- Retry the same idempotency key and canonical fingerprint. The current principal receives the immutable original receipt.
- A changed fingerprint under the same key is an operator-visible conflict; do not generate a new key automatically.
- Lower/equal ordered events are stale/duplicate. A changed fingerprint at the same sequence is quarantined as conflict.
- A future event is durable in `business_event_inbox_v1` with `deferred`. Replay it after preceding events; the disposition advances to `applied` only with the exact fingerprint.
- A failed import must carry one bounded lowercase failure class; nonfailed states must carry none. Never persist provider messages, URLs, or stack traces.
- After an adapter timeout, retrieve the current-principal receipt and aggregate revision before retrying. Never assume the write failed.

## Ledger reconciliation

For each account, start from zero or the approved migration opening balance, then read immutable transactions in sequence. Verify no gap, duplicate idempotency key, duplicate semantic reference, overflow, or negative balance. The final computed balance must equal `developer_credit_accounts.balance_microcredits` and the final sequence must match `ledger_sequence`.

Group usage entries by semantic reference and compare to derivative/upload/storage facts. Sum `microcredits_charged` and compare with usage credit transactions. For daily storage, independently calculate source units, compare `source_checksum`, then compare charge totals. Correct a financial error only with an approved compensating transaction.

## Export and deletion

Exports enumerate all 20 data classes and include source revision, per-class count, actual export rows, row checksums, and one deterministic checksum over counts, rows, and revision. Storage configuration and API-key material are excluded. Only an active organization owner can export. Legal hold and subject-to-tenant binding are checked transactionally before deletion. The D1 executor performs the class-specific tombstone, purge, or cryptographic erasure before marking completion. Object deletion remains a metadata state transition until the provider delete and subsequent absence probe succeed. Credit/usage deletion appends a balanced adjustment entry while retaining the immutable original and audit history.

The deterministic zero-row sample is in `docs/evidence/business-data-sample-export-v1.json`.

## Rollback

Disable business writes first. Preserve operation receipts, event inbox, ledgers, audit events, and deletion/export requests. Return reads to the compatibility source and reconcile any maybe-committed operation by receipt. Do not down-migrate 0011 while receipts, deferred events, holds, or dirty findings exist. A later contract migration may remove compatibility columns only after per-tenant approval.

## Protected evidence

Production promotion still requires: a named D1 database rehearsal and rollback, current Wrangler query execution, provider object reconciliation for each storage backend, managed/native derivative parity, payment/credit reconciliation approval, authorized messenger purge, and privacy/legal sign-off. Record environment identifiers, immutable logs, timestamps, and reviewer approval; never substitute local SQLite output.
