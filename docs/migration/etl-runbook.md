# Metadata ETL rehearsal runbook

`scripts/migration/etl.py` implements the credential-free reference pipeline for
issue 16. It transforms source NDJSON into versioned, checksummed chunks, quarantines only
table/ordinal/reason metadata, imports each chunk in one transaction, writes a
durable checkpoint in the same commit, and performs disk-backed reconciliation
of primary keys, field hashes, row counts, relationships, aggregates, and policy
semantics without printing row values.

`scripts/migration/mysql_snapshot.py` emits this exact bundle contract from one
read-only repeatable-read MySQL session. It captures a conservative GTID boundary
before the consistent snapshot, requires MySQL `8.0.26` or newer, forces
verified-identity TLS 1.2/1.3, and accepts secrets
only through an owner-private MySQL defaults file. Provider credentials, the CA,
the raw GTID boundary, and incremental binlog capture remain protected artifacts.
The exporter ignores global, user, login-path, and `MYSQL_*` overrides and rejects
option-file includes, TLS overrides, or command-capable options. The reviewed
MySQL executable must be root/operator owned, executable, and not group/other
writable; record and approve its version and SHA-256 in protected evidence.

## Rehearsal sequence

Create a draft JSON plan with a unique run ID, pinned source schema, target
migration, code revision, dependency-ordered source tables, columns, primary
keys, and explicit transforms. Do not invent the `mysql_snapshot` hashes. Derive
them through the same locked metadata transaction used by export; the command
emits digests and compatibility facts only, never source rows or credentials:

At the pinned Cap revision, database identifiers created by
`packages/database/helpers.ts` are 15-character NanoIDs, while Frame contracts
require non-nil UUIDs. Mark every occurrence of such an identifier—including
every primary and foreign key—with the option-free `cap_nanoid_uuid_v1`
transform. It deterministically produces UUIDv8 from the first 128 bits of
`SHA-256("frame-cap-nanoid-to-uuid-v1\\0" || ascii_nanoid)`. The same source ID
therefore maps identically across all tables and runs. Do not apply the transform
to provider IDs or user text, and do not publish source-ID/mapped-ID pairs in
evidence. A duplicate mapped logical key is quarantined and blocks clean
reconciliation rather than silently merging rows.

The normative identifier inventory is
`fixtures/etl/v1/cap-business-plan-contract.json`. The executable plan is
`fixtures/etl/v1/cap-business-plan.json`: 21 streams cover exactly the 20 pinned
source tables because Notifications intentionally produces both a notification
and an outbox row. It includes one-source-to-many ID projections, joined tenant
derivation, versioned documents, digests/checksums, and an independent ledger
import order. Owner-scoped Cap roots require the protected, one-row-per-root
`frame_business_tenant_scope_v1` mapping named by `scope_contract`; missing or
duplicated scope fails export rather than selecting an arbitrary organization.
The checked-in MySQL binding uses zero digests as an unmistakable local
placeholder. Before a protected run, copy the plan, replace every binding digest,
run id, and snapshot window from the approved fingerprint, and run
`python3 -I scripts/migration/test_cap_business_plan_contract.py`. The test
cross-checks the exact 20 pinned source tables, every internal/external
relationship, every executable target column, safe joins/window projection,
option-free transforms, and the explicitly Frame-derived `usage_ledger`;
omitting an identifier blocks authorization.

```sh
python3 scripts/migration/mysql_snapshot.py \
  --fingerprint-source \
  --plan /protected/plan-draft.json \
  --defaults-file /protected/mysql-client.cnf \
  --scratch-directory /protected/scratch \
  --timeout-seconds 300
```

Review the database, server UUID/version, table, column, index, and constraint
digests in that output, copy its exact `mysql_snapshot` object into a new
immutable plan, and obtain the required approval. Fingerprinting fails closed on
an older server, non-GTID/ROW/FULL configuration, non-InnoDB/missing table,
metadata truncation warning, schema mismatch, or unsafe credential/client path.
Then export:

```sh
python3 scripts/migration/mysql_snapshot.py \
  --plan /protected/plan.json \
  --defaults-file /protected/mysql-client.cnf \
  --bundle /protected/run-bundle \
  --chunk-rows 1000 \
  --timeout-seconds 14400

# Credential-free NDJSON reference alternative:
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

python3 scripts/migration/mysql_cdc.py \
  --plan /protected/plan.json \
  --snapshot-boundary /protected/run-bundle/snapshot-boundary.protected.json \
  --defaults-file /protected/mysql-client.cnf \
  --driver /approved/frame-mysql-cdc-driver \
  --state /protected/cdc-state \
  --report /protected/cdc-report.json

python3 scripts/migration/d1_target.py \
  --bundle /protected/run-bundle \
  --endpoint https://approved-operator-worker.example \
  --authorization-file /protected/d1-operator-token \
  --report /protected/d1-reconciliation.json
```

Before accepting any plan that uses the mapping, run its known-answer and ETL
integration contracts:

```sh
python3 -I scripts/migration/test_cap_id_map.py
python3 -I scripts/migration/test_etl_cap_id_map.py
```

Use either the MySQL snapshot command or the SQLite reference `export` command to
create a bundle, never both for the same bundle path. The MySQL command already
performs the transform/export step. Preserve its
`snapshot-boundary.protected.json`; begin the approved incremental reader at the
recorded pre-snapshot GTID. Duplicate events between that conservative boundary
and the snapshot are expected and must converge through the same idempotent target
ledger used by `scripts/migration/cutover.py`. This conservative boundary alone
does not prove there is no gap.

`mysql_cdc.py` invokes an approved replication-protocol driver without putting
credentials or GTIDs on its command line. It requires canonical transaction
frames with FULL before/after row images, binds the same server UUID and start
GTID, rejects retention/filter or sequence gaps, journals each transaction before
advancing its protected checkpoint, reconstructs resume state from the immutable
journal, and requires a post-boundary caught-up heartbeat. The local fake proves
this client protocol, not a production driver or server retention policy.

`d1_target.py` talks only to the reviewed migration API, over loopback to a
Worker started by Wrangler or over HTTPS, with redirects disabled so the
operator credential stays on the reviewed origin. The Worker must acquire an
exclusive D1 generation fence, atomically apply each page with its checkpoint,
and reserve a snapshot id. The first snapshot-page request seals that id and
ends the apply phase; every table page must then come from that immutable
snapshot. The adapter verifies request bounds, page
digests, idempotent acknowledgements, cursor completeness, target migration,
run/manifest binding, generation stability, primary keys, field hashes, and the
snapshot's foreign-key and semantic result before it permits the fence to
finish. Direct administrative
D1 REST calls are not an application data plane.

The deferred Worker wiring is deliberately narrow and operator-only:

- `POST /v1/migrations/etl/fences/begin` validates and echoes the
  run/manifest/migration binding and returns one exclusive generation plus a
  reserved snapshot id;
- `POST /v1/migrations/etl/chunks/apply` atomically verifies the fence, inserts or
  byte-compares one page, and records its page digest checkpoint;
- the first `POST /v1/migrations/etl/snapshots/page` atomically seals the
  reserved snapshot and the apply phase; it then keyset-paginates the requested
  columns from that exact snapshot and never advances the generation;
- `POST /v1/migrations/etl/snapshots/verify` returns foreign-key and semantic
  violation counts bound to the same snapshot; and
- `POST /v1/migrations/etl/fences/finish` records the clean report digest before
  releasing the fence.

These routes still need delegation from the main Worker router after its current
owner completes route integration. Until then, the checked-in client/fake is an
executable contract, not a claim that a remote endpoint exists.

The bundle directory and every artifact are owner-only. Store them on an
approved encrypted volume, never in the repository or CI artifacts. A reject
exit code blocks promotion until the disposition is approved. The importer is
insert-only on primary-key conflict and verifies the existing field hash, so a
rerun cannot silently overwrite authority or duplicate a logical record.
The reconciliation spill database contains source and target row values. It is
created mode `0600` beneath the bundle's owner-private parent, uses secure delete,
and is removed on a normal exit. Budget peak encrypted scratch space for roughly
the combined canonical source and target metadata plus SQLite indexes. After a
host crash, preserve the incident state first, then securely remove orphaned
`.frame-etl-reconcile-*` or `.frame-mysql-*` directories under that protected
parent according to the approved media-destruction procedure.

## Abort, resume, and restore

An abort stops after the current SQLite transaction. Rerun the same command to
skip durable checkpoints and resume. If a commit succeeded before a checkpoint
write, rerun safely observes the same primary key and field hash. A different
row at that key fails closed. Never delete a source snapshot or checkpoint to
force progress.

## Protected no-gap and production gates

Before each production rehearsal, independently record and approve all of these
facts; the local fake-client test proves none of them:

- Binlog retention covers the fingerprint, snapshot, import, and catch-up window.
  The reviewed CDC driver attaches to the same server UUID digest, starts at the exact
  protected pre-snapshot GTID, reaches the current executed set, and observes a
  post-boundary heartbeat written through the normal application writer.
- No source write can use `sql_log_bin=0`; binlog database/table filters cannot
  omit an in-scope write. Freeze DDL, global binlog changes, topology/failover,
  and privileged writer-session overrides for `binlog_format`,
  `binlog_row_image`, and `binlog_row_value_options` until catch-up completes.
- The approved MySQL client/version/digest and server `>=8.0.26` syntax are tested
  once against a disposable pinned MySQL instance, including option-file order,
  login-path isolation, TLS identity verification, transaction syntax, metadata
  locks, and a controlled concurrent write around the captured GTID boundary.
- The remote target implements the checked-in migration API's exclusive
  generation fence and immutable snapshot pagination contract. The local fake
  proves client rejection behavior only; record a named Wrangler/Worker/D1 run
  showing that no concurrent target write spans the snapshot.
- Restore the target backup into a new isolated database, time the restore, apply
  the full bundle twice, inject termination between chunks, and require clean
  reconciliation after both runs. Preserve target-only-tenant and empty-source
  negative controls.

Record the bundle manifest/core digest, source fingerprint and boundary, target
migration/boundary, client/server digests, start/end time, throughput, peak memory
and scratch disk, provider limits/cost, reject dispositions, binlog/heartbeat
evidence, restore/rollback timing, and approvers in protected release evidence.
Production authorization requires two full-scale runs, incremental catch-up,
timed restore/cutover/rollback, and human sign-off. CI does not substitute for
those gates.
