# API and workflow family cutover runbook

Use this runbook one route/workflow family and one client release at a time. It cannot authorize a
billing provider, production secret, retirement, or migration authority change by itself.

## Local preflight

From the repository root:

```sh
python3 scripts/ci/check-api-workflow-parity.py --require-reference
python3 -I scripts/ci/api-workflow-d1-conformance.py
cargo test -p frame-domain --test api_workflow_contract_v1
cargo test -p frame-domain api_workflow --lib
cargo test -p frame-application api_workflow --lib
cargo test -p frame-control-plane api_workflow_runtime --lib
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Without the ignored reference checkout, omit `--require-reference`; the report, source identities,
generated documentation, and Rust fixtures still validate offline. Never regenerate from a
different commit in place. Change the versioned corpus and record the reference decision.

Confirm the target rows have endpoint-level success, validation, authorization/non-disclosure,
idempotency/retry, and failure evidence. `family_contract`, `endpoint_adapter_pending`,
`dependency_pending`, and `protected_evidence_required` do not authorize traffic movement.

## Strangle sequence

1. Select one family, client surface, current/N-1 release pair, and tenant cohort. Record the exact
   Git SHA and report row IDs.
2. Keep the legacy writer authoritative. Shadow only privacy-safe decisions and normalized result
   classes; never duplicate a billing, notification, upload-finalize, provider, or destructive effect.
3. Compare stable response/status classes, non-disclosure behavior, durable side-effect receipts,
   and latency/error budgets. Quarantine unexplained differences.
4. Enable the Frame adapter for synthetic traffic, then an internal cohort, then a bounded tenant
   cohort. `LegacyRoutePolicyV1` must still have a working fallback.
5. Run current and N-1 client E2E suites before redirecting either client flag. A pass by one client
   does not promote another.
6. Observe at least the route family's approved window. Reconcile D1 rows, R2 manifests, outbox
   receipts, and provider ledgers where applicable before expanding.
7. Remove a fallback only in a later change after the deprecation/removal date and approval record.

## Webhook key rotation

1. Create a new secret reference; do not put the value in Git, logs, evidence, or support bundles.
2. Add the new key with a future `not_before` and keep the previous key active through at least one
   replay window plus provider-delivery skew.
3. Send a signed synthetic event. Confirm one D1 replay claim, one business idempotency receipt, and
   a duplicate rejection with no repeated side effect.
4. Switch provider signing, observe both key IDs as privacy-safe counters, then expire the old key.
5. On replay-store errors, return unavailable and perform no business effect. Do not bypass replay
   defense to restore availability.

The callback route must construct `D1WebhookReplayStoreV1` from the request's D1 binding and pass it
to `WebhookVerifierV1`; the memory store is never an acceptable deployed fallback.

Compromise response: disable the affected callback family, revoke the key at the provider, rotate,
inventory event IDs over the exposure window, replay only reconciled missing events, and attach the
incident/security approval. Never print candidate signatures or raw bodies while diagnosing.

## Stuck or indeterminate workflow

- For a running lease, wait for expiry or use the repository's compare-and-set recovery command;
  never edit a fence in place.
- For `waiting_for_provider`, query the provider with the stored idempotency key. Record confirmed
  or rejected. Do not submit another mutation to discover the result.
- For an exhausted retry budget, quarantine the workflow and expose the stable terminal failure.
- For partial output, verify immutable checksums and publication fences. Delete only the attempt's
  staging namespace; never enumerate a tenant or bucket prefix from an untrusted request.
- For poison outbox data, quarantine that message, preserve ordering rules for its aggregate, and
  continue unrelated tenant partitions.

## Rollback

Disable the affected client/family flag and restore the legacy fallback. Stop new Frame mutations,
allow already-fenced operations to finish or reconcile, and replay acknowledged writes from their
durable idempotency receipts. Rollback does not delete D1/R2 data, rewind a migration, publish a
partial object, change provider keys, or convert an indeterminate effect into a failure.

Escalate immediately for any cross-tenant disclosure, duplicated billing/provider effect, replayed
webhook side effect, acknowledged-write loss, or mismatched object checksum. These are release
blockers independent of aggregate availability.
