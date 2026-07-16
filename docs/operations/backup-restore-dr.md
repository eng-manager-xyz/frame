# Backup, restore, and disaster recovery

The recovery contract covers D1 state, exact object manifests, non-secret
configuration, signing-key metadata/custody receipts, and desktop projects.
The charter targets are durable-state RPO <= 5 minutes and service RTO <= 60
minutes. Backup and restore principals are separate, encryption and immutable
retention are mandatory, and restores never target production.

Run the bounded local rehearsal with:

```sh
python3 -I scripts/ci/restore-dr-rehearsal.py \
  --evidence target/evidence/operational-restore-local.json
```

It exports a synthetic SQLite database, binds every file by SHA-256, restores
into a new temporary root, and checks integrity, foreign keys, row counts, auth
session links, object/video/share relations, exact object size/digest, bounded
range read, MP4 container structure, configuration without secret values,
key-catalog metadata only, and read-only desktop project opening. Corrupt and
missing-object manifests must fail. This is not D1/R2 or encrypted-backup
evidence.

## D1 export and restore

Capture a consistent export with replay checkpoint and last acknowledged-write
time. Encrypt before leaving the isolated runner; record ciphertext digest,
key ID, retention class, region, and immutable receipt. Restore to a new D1
database, apply only compatible forward migrations, run integrity/referential,
auth, application-query, replay, and idempotency checks, then record measured
RPO/RTO. Never import over the active database.

## Object manifest and durability

Use application manifests, not provider listings. Copy exact versioned objects
into an isolated prefix, verifying role, size, strong checksum, provenance,
and relationship before exposure. Probe range and playback with synthetic
content. Reconcile absent, extra, corrupt, staging, and final objects; do not
publish or delete while unexplained drift remains.

## Configuration, keys, and projects

Back up versioned non-secret configuration plus provider secret references,
never secret values. The signing catalog contains IDs, public-key digests,
rotation state, and dual-control recovery receipts; private material stays in
its custody authority. Desktop project manifests bind schema and segment
digests and open read-only before editing or export is enabled.

## Regional disaster

Cross-residency failover is disabled unless the target and customer/security
policy are approved. Restore data and objects, replay acknowledged writes,
reconcile to zero unexplained differences, validate auth/playback, then move
traffic. Fail back through the same fenced process. Record data location,
transfer, RPO/RTO, cost, traffic control, cleanup, and approvals.

Provider exports, encryption custody, signing-key recovery, timed
production-shaped restore, and regional traffic games remain protected.
