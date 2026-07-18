# Local metadata ETL and cutover evidence

This document records **credential-free local evidence**, not production acceptance.
It exercises the contracts needed by issues 16 and 17 without connecting to MySQL,
Cloudflare D1, provider dashboards, or production data.

## Reproduce

From the repository root, run:

```sh
python3 -I scripts/migration/test_local.py
python3 -I scripts/migration/test_mysql_snapshot.py
python3 -I scripts/migration/test_mysql_cdc.py
python3 -I scripts/migration/test_d1_target.py
python3 -I scripts/migration/test_cap_id_map.py
python3 -I scripts/migration/test_etl_cap_id_map.py
python3 -I scripts/migration/test_cap_business_plan_contract.py
python3 scripts/ci/check-migrations.py
cargo test -p frame-domain -p frame-ports -p frame-application
```

The test uses only the Python standard library and an isolated temporary directory.
Its final JSON report has `production_evidence: false` and proves all of the following
locally:

- byte-identical manifests and chunks from two exports of the same source window;
- plan binding to source schema, target migration, code SHA-256, snapshot window,
  source checksum, plan checksum, and per-chunk checksums;
- deterministic boolean, JSON, timestamp, safe-integer, fixed-scale decimal,
  collation, enum, nullable, and pinned-Cap NanoID-to-UUIDv8 transforms;
- shared known answers proving that every primary/foreign-key occurrence of one
  Cap NanoID receives the identical Frame UUID, while wrong alphabets, lengths,
  types, and option-bearing mapping plans fail closed;
- a credential-free pinned-Cap business-plan contract covering 56 NanoID
  primary/foreign-key occurrences across the exact 20 source tables, checked
  against real D1 target columns and the Frame-derived usage ledger;
- an executable Cap plan with 21 streams over those exact 20 sources, including
  one source to multiple targets, repeated source identifiers, joined tenant
  scopes, versioned documents, digests/checksums, and ledger-specific import order;
- per-tenant/table/chunk transactional checkpoints, an injected interruption,
  resume, and an idempotent third import;
- a dry run that leaves both application rows and checkpoint schema unchanged;
- quarantine records containing only table, ordinal, tenant digest, and a bounded
  reason code;
- row count, primary-key presence, field hash, foreign-key, aggregate, and policy
  semantic reconciliation from a bounded disk-backed index, including a seeded
  field/aggregate divergence and an empty-source/target-only-tenant negative case;
- chunk, manifest-core, public-proof, and protected-boundary tamper rejection;
- approved shadow normalization plus a seeded semantic mismatch, with no result
  values in the report;
- durable ordered change capture, duplicate capture, a retryable injected target
  outage, poison dead-lettering, and target-ledger recovery when commit succeeds
  before source acknowledgement;
- a replay/pause race that proves pause waits for the in-flight target commit and
  acknowledgement boundary before advancing the authority epoch;
- plan-allowlisted tenant digests and domains, per-scope legacy and D1 writer
  fences, stale compare-and-swap rejection, monotonic epochs, bounded maintenance
  windows, and a timed synthetic rollback;
- preservation of a synthetic D1-canary write by ordered replay into an isolated
  legacy projection before the writer returns to legacy;
- current-phase-only shadow promotion, exact signal-window accounting,
  source-writer/epoch binding, tenant-scoped event IDs, contiguous sequences,
  pre-I/O envelope tamper rejection, and ordered tenant-scoped delete replay;
- required-query coverage in the latest configured shadow window, plus explicit
  mismatch, lag, write-failure, contention, dead-letter, and rollback-readiness
  alert contracts;
- owner-private/non-symlinked operator controls, bounded JSON/event inputs, and a
  hash-chained immutable audit trail whose verifier rejects a seeded stored-row
  tamper rather than trusting the saved audit head.
- a fake fenced D1 migration API proving atomic page acknowledgements, safe rerun,
  complete paginated snapshot enumeration, immutable generation/snapshot binding,
  mismatch detection, and refusal to finish a dirty report;
- a fake CDC subprocess proving same-server/start-GTID and retention fences,
  FULL allowlisted row images, contiguous transaction sequences, immutable
  journal recovery, resume, and a terminal post-boundary caught-up heartbeat.

The scoped D1 migration check additionally proves that invalid phase/writer/mirror
combinations, unaudited state updates, invalid transitions, mutable audit rows, and
overlapping maintenance windows fail at the schema boundary. Rust domain,
application, and port tests prove that scopes are isolated, `dual_write` retains
one legacy writer plus a mirror, replay controls advance the epoch, and stale or
wrong-writer fences fail closed. The operational sequence and report boundaries
are in [Metadata authority cutover and rollback](../operations/metadata-authority-cutover.md).

The MySQL snapshot proof additionally exercises a deterministic fake executable
through the production subprocess boundary. It proves the Python-side generation,
stream parsing, publication, and failure contracts; it does **not** prove that a
real MySQL server accepts or implements the generated SQL. Locally it verifies that
the exporter:

- passes credentials only by an owner-private, non-symlinked MySQL defaults file;
- copies validated credentials into private scratch, forces TCP,
  verified-identity TLS 1.2/1.3, UTC, read-only repeatable-read, and one
  consistent-snapshot transaction in a single client session;
- orders the generated conservative GTID read before the snapshot. The snapshot
  fake proves marker ordering and digest binding; the separate CDC fake proves
  client-side same-server/retention/heartbeat enforcement. A real protected
  source execution is still required for a no-gap production result;
- generates and parses a value-free source fingerprint using the same locked
  metadata recipe, with database/server/schema hashes, MySQL `>=8.0.26`, GTID,
  ROW/FULL, InnoDB, and metadata-truncation preconditions;
- maps only validated identifiers into generated SQL and orders every table by
  source tenant plus primary key;
- streams bounded row envelopes, uses LF-only record framing, bounds chunks/plans/
  manifests/counts, rejects malformed and out-of-window fake output, discards
  protected stderr, and never renders client details;
- produces byte-identical bundles and boundary proofs across identical snapshots;
- recovers after a failed staged publication without overwriting a final bundle,
  using a kernel-released lock and atomic no-replace rename;
- imports and reconciles the resulting bundle with canonical boolean, decimal,
  JSON, timestamp, collation, enum, and Cap NanoID-to-UUID transforms.

The D1 target fake exercises the real HTTP-adapter logic without opening a
network connection. It bounds requests, rejects redirects, binds the fence to the
run and manifest, splits and durably acknowledges pages, treats a repeated
acknowledgement as idempotent, and keeps import order independent from the logical
primary key. The first reconciliation request seals the apply phase. The fake
then enumerates more than one immutable snapshot page, spills comparison keys and
hashes to an owner-private SQLite index, rejects generation drift, and will not
finish the target fence after a seeded field mismatch. The accepted endpoint
forms are loopback HTTP for `wrangler dev` and HTTPS for an approved operator
Worker; no named D1 database was contacted.

The CDC fake crosses the production subprocess boundary with an owner-private
MySQL defaults file and protected request file. It proves start-GTID and server
digest binding, retention/filter/ROW/FULL preconditions, allowlisted complete row
images, contiguous transaction framing, immutable per-transaction journal files,
recovery-based resume, redacted failures, and a terminal post-boundary heartbeat.
It deliberately reports `production_evidence: false`: the fake does not prove a
real replication connection, binlog retention, or application heartbeat.

The credential-free ETL/cutover fixtures live in `fixtures/etl/v1`; the MySQL fake
and its fault cases are generated by `test_mysql_snapshot.py`. The operator phrase
in the fixture is deliberately public test data; it is not a production credential
or an example of production secret management.

## ETL operator contract

`scripts/migration/etl.py` accepts source NDJSON envelopes and writes a new immutable
bundle. A protected repeatable-read MySQL exporter must emit the same envelope and
must pin its snapshot boundary in the plan. The local exporter never overwrites an
existing bundle directory.

`scripts/migration/mysql_snapshot.py` implements that exporter boundary. Its MySQL
option file must be a regular file owned by the invoking account with no group or
other permissions. The file supplies the approved host, port, database, user,
password, and CA configuration. Command-line options force `VERIFY_IDENTITY` and
TLS 1.2/1.3;
secrets are not accepted as flags or emitted in failure text. The exporter uses
that file exclusively, disables login-path and environment overrides, rejects
includes, repeated options, and command-capable client settings, and permits only
the reviewed connection/TLS option set. The server must be MySQL `8.0.26` or newer.
The client executable must be root/operator owned and not group/other writable;
its approved version and digest remain protected release evidence.

Generate the plan's source binding first from an owner-private encrypted scratch
directory. Review and copy the returned `mysql_snapshot` object into an immutable
approved plan; output contains hashes and compatibility facts only:

```sh
python3 scripts/migration/mysql_snapshot.py \
  --fingerprint-source \
  --plan /protected/plan-draft.json \
  --defaults-file /protected/mysql-client.cnf \
  --scratch-directory /protected/scratch \
  --timeout-seconds 300
```

```sh
python3 scripts/migration/mysql_snapshot.py \
  --plan /protected/plan.json \
  --defaults-file /protected/mysql-client.cnf \
  --bundle /protected/run-bundle \
  --chunk-rows 1000 \
  --timeout-seconds 14400
```

The protected bundle contains `snapshot-boundary.protected.json`, including the raw
pre-snapshot GTID set needed by the approved binlog/change-data-capture reader. The
shareable `snapshot-proof.json` contains only its SHA-256 digest and bounded counts.
Neither artifact authorizes cutover by itself.

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
Reconciliation opens one read-only/query-only SQLite target transaction and spills
its bounded comparison index beneath the bundle's owner-private parent. That spill
contains values and therefore requires the same encrypted-volume controls. Normal
exit removes it; crash-orphan cleanup follows the runbook's incident procedure.

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
evidence fields, an approved maintenance window, current required-query coverage,
and live agreement with pending/dead-letter/shadow counts. D1
authority requires clean reconciliation, a legacy write fence, zero pending and poison
events, zero shadow mismatches, and a rehearsed rollback. Rollback requires a D1 fence
and proof that legacy is caught up.

## Evidence still required before production authorization

Local fixtures do not satisfy the production acceptance criteria. Protected release
evidence must still include:

- execution of the reviewed repeatable-read exporter against the protected source,
  plus a disposable pinned MySQL `>=8.0.26` parser/transaction/TLS test and the
  reviewed CDC driver connected through the checked-in adapter from its preserved GTID;
- proof that binlog retention spans catch-up, CDC resumes on the same server UUID
  at the exact boundary and observes a post-boundary heartbeat, no in-scope writer
  uses `sql_log_bin=0` or filtered/session-downgraded binlogging, and DDL/topology/
  binlog configuration remained frozen through catch-up;
- at least two production-scale rehearsal manifests and clean reconciliation reports;
- measured MySQL/D1 duration, throughput, throttling, provider limits, and cost against
  the approved window and budget;
- timed backup restore plus timed cutover and rollback rehearsals;
- production-scale shadow-read coverage for charter-critical queries and lag/divergence
  dashboards with alert delivery evidence;
- approved quarantine dispositions, incident exercise evidence, and human sign-off;
- approved MySQL/CDC binary and version digests plus execution of the checked-in
  generation-fenced API against a named Wrangler/Worker/D1 target;
- protected operator authentication and authorization integrated with the production
  control plane rather than the public local fixture.

Until those artifacts exist, this implementation is a locally verified migration
slice and fail-closed control contract, not a claim that production metadata has been
migrated or that issues 16 and 17 are fully accepted.
