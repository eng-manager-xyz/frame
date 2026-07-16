# Local metadata ETL and cutover evidence

This document records **credential-free local evidence**, not production acceptance.
It exercises the contracts needed by issues 16 and 17 without connecting to MySQL,
Cloudflare D1, provider dashboards, or production data.

## Reproduce

From the repository root, run:

```sh
python3 -I scripts/migration/test_local.py
```

The test uses only the Python standard library and an isolated temporary directory.
Its final JSON report has `production_evidence: false` and proves all of the following
locally:

- byte-identical manifests and chunks from two exports of the same source window;
- plan binding to source schema, target migration, code SHA-256, snapshot window,
  source checksum, plan checksum, and per-chunk checksums;
- deterministic boolean, JSON, timestamp, safe-integer, fixed-scale decimal,
  collation, enum, and nullable transforms;
- per-tenant/table/chunk transactional checkpoints, an injected interruption,
  resume, and an idempotent third import;
- a dry run that leaves both application rows and checkpoint schema unchanged;
- quarantine records containing only table, ordinal, tenant digest, and a bounded
  reason code;
- row count, primary-key presence, field hash, foreign-key, aggregate, and policy
  semantic reconciliation, including a seeded field/aggregate divergence;
- manifest/chunk tamper rejection;
- approved shadow normalization plus a seeded semantic mismatch, with no result
  values in the report;
- durable ordered change capture, duplicate capture, replay pause/resume, poison
  dead-lettering, and recovery when the target commit succeeds before acknowledgement;
- fenced D1 authority, a rollback to legacy authority, monotonic epochs, and a
  hash-chained immutable audit trail.

The fixture plan and fault corpus live in `fixtures/etl/v1`. The operator phrase in
the fixture is deliberately public test data; it is not a production credential or
an example of production secret management.

## ETL operator contract

`scripts/migration/etl.py` accepts source NDJSON envelopes and writes a new immutable
bundle. A protected repeatable-read MySQL exporter must emit the same envelope and
must pin its snapshot boundary in the plan. The local exporter never overwrites an
existing bundle directory.

```sh
python3 scripts/migration/etl.py export \
  --source /protected/source.ndjson \
  --plan /protected/plan.json \
  --bundle /protected/run-bundle \
  --chunk-rows 1000

python3 scripts/migration/etl.py import \
  --target /isolated/target.sqlite \
  --bundle /protected/run-bundle \
  --dry-run

python3 scripts/migration/etl.py import \
  --target /isolated/target.sqlite \
  --bundle /protected/run-bundle \
  --max-rows-per-second 500

python3 scripts/migration/etl.py reconcile \
  --target /isolated/target.sqlite \
  --bundle /protected/run-bundle \
  --report /protected/reconciliation.json
```

Bundle directories are mode `0700`; files are mode `0600`. Bundles and the cutover
state database contain transformed or captured row values, so they still belong on
an approved encrypted volume and must never be attached to a public issue or CI log.
Reconciliation, status, shadow, and quarantine evidence is value-free, but should
also be retained as protected release evidence.

A non-empty quarantine blocks a clean reconciliation. Operators must not edit a
bundle or delete a checkpoint to force progress. Approve a documented disposition,
fix the deterministic transform or source record, and create a new run ID and bundle.

## Abort, restore, resume, and incident sequence

1. Restore the approved target backup into a new isolated database and record elapsed
   time, artifact digest, and operator/approver digests outside this repository.
2. Export once from the pinned repeatable-read source window. Preserve the bundle and
   `manifest.sha256`; do not regenerate it during resume.
3. Run dry-run import and reconciliation before any target-authority transition.
4. During import, abort by terminating the process. A chunk and its checkpoint commit
   in the same target transaction, so rerunning the exact bundle safely resumes.
5. If a target key contains different data, stop. Preserve the target, bundle, state
   database, and safe error code for incident review; never overwrite the conflicting
   authority silently.
6. Run reconciliation. Any unexplained row, relationship, field, aggregate, policy,
   or quarantine mismatch blocks cutover.
7. For change replay, use the audited pause control before investigation. A replayed
   event is written with a target ledger row in one transaction. If acknowledgement
   is interrupted, the next replay recovers from that ledger instead of duplicating
   the logical change.
8. Restore/rollback into a separate database first, reconcile it, then use an audited
   fenced authority transition. At every phase the state contains exactly one writer;
   `dual_write` means legacy authority plus an asynchronous D1 mirror, not atomic
   cross-database writes.

Cutover transitions require an expected epoch, an authorized operator digest, bounded
evidence fields, and live agreement with pending/dead-letter/shadow counts. D1
authority requires clean reconciliation, a legacy write fence, zero pending and poison
events, zero shadow mismatches, and a rehearsed rollback. Rollback requires a D1 fence
and proof that legacy is caught up.

## Evidence still required before production authorization

Local fixtures do not satisfy the production acceptance criteria. Protected release
evidence must still include:

- a reviewed repeatable-read MySQL exporter and incremental/binlog catch-up adapter;
- at least two production-scale rehearsal manifests and clean reconciliation reports;
- measured MySQL/D1 duration, throughput, throttling, provider limits, and cost against
  the approved window and budget;
- timed backup restore plus timed cutover and rollback rehearsals;
- production-scale shadow-read coverage for charter-critical queries and lag/divergence
  dashboards with alert delivery evidence;
- approved quarantine dispositions, incident exercise evidence, and human sign-off;
- protected operator authentication and authorization integrated with the production
  control plane rather than the public local fixture.

Until those artifacts exist, this implementation is a locally verified migration
slice and fail-closed control contract, not a claim that production metadata has been
migrated or that issues 16 and 17 are fully accepted.
