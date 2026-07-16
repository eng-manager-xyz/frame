# Metadata ETL rehearsal runbook

`scripts/migration.py` implements the credential-free reference pipeline for
issue 16. It exports a consistent read-only SQLite snapshot into versioned,
checksummed NDJSON chunks, applies deterministic transforms, quarantines only
table/ordinal/reason metadata, imports each chunk in one transaction, writes a
durable checkpoint after commit, and reconciles primary keys, field hashes,
row counts, and foreign keys without printing row values.

The production MySQL reader must emit this exact bundle contract after a
repeatable-read snapshot. It is intentionally not embedded here: provider
credentials, TLS policy, snapshot mechanics, throttling, and incremental binlog
capture belong to the protected migration environment.

## Rehearsal sequence

Create a JSON plan with a unique run ID, pinned source schema, target migration,
code revision, dependency-ordered tables, columns, primary keys, and explicit
transforms. Then run:

```sh
scripts/migration.py export \
  --source /isolated/source.sqlite \
  --plan /isolated/plan.json \
  --bundle /isolated/bundle \
  --chunk-rows 1000
scripts/migration.py import \
  --target /isolated/target.sqlite \
  --bundle /isolated/bundle \
  --dry-run
scripts/migration.py import \
  --target /isolated/target.sqlite \
  --bundle /isolated/bundle
scripts/migration.py reconcile \
  --target /isolated/target.sqlite \
  --bundle /isolated/bundle \
  --report /isolated/reconciliation.json
```

The bundle directory and every artifact are owner-only. Store them on an
approved encrypted volume, never in the repository or CI artifacts. A reject
exit code blocks promotion until the disposition is approved. The importer is
insert-only on primary-key conflict and verifies the existing field hash, so a
rerun cannot silently overwrite authority or duplicate a logical record.

## Abort, resume, and restore

An abort stops after the current SQLite transaction. Rerun the same command to
skip durable checkpoints and resume. If a commit succeeded before a checkpoint
write, rerun safely observes the same primary key and field hash. A different
row at that key fails closed. Never delete a source snapshot or checkpoint to
force progress.

Before a production rehearsal, restore the target backup into a new isolated
database, time the restore, apply the full bundle twice, inject termination
between chunks, and require a clean reconciliation both times. Record bundle
manifest digest, source snapshot boundary, target migration, start/end time,
throughput, reject dispositions, restore timing, and approvers in the protected
release evidence. Production authorization still requires two full-scale runs,
incremental catch-up validation, and rollback rehearsal; CI does not substitute
for those gates.
