# Organization repository local conformance

Issue 14 has two deliberately separate local evidence boundaries.

## Current offline semantic evidence

Run the network-free SQLite suite with:

```sh
python3 scripts/ci/organization-sqlite-semantic-conformance.py
```

The suite applies all ordered migrations to Python `sqlite3`, compiles every
checked-in organization query, and executes the repaired transactional SQL. It
covers:

- dirty pre-`0010` upgrade with multiple owners, nested folders, and a parent
  cycle, followed by reviewed normalization and contract-index eligibility;
- invite acceptance bound to the authenticated user's versioned identifier
  digest, including denial for another actor holding the token;
- server-derived semantic request fingerprints, exact replay matching, and
  same-key/different-payload mismatch;
- cross-tenant and invalid-authority denial with zero target-tenant or audit-row
  mutation;
- contributor manage/move authority limited to folders they created, with
  ownership retained after update;
- database-clock tombstone deadlines, in-window recovery, and expired recovery
  rollback through the retention trigger;
- support assertion, graph reads, assertion cleanup, and allow audit in one
  snapshot batch, plus support reassertion before repair-plan insertion; and
- tenant-authorized cursor pagination, redacted admin-only invite listing,
  space/folder management, space-role sharing with session invalidation, and
  allowed-domain capacity fencing.

The machine-readable result is
`target/evidence/organization-sqlite-semantic-conformance.json`. It records the
migration/query digests, scenario results, and its exact exclusions. This is
executable evidence for SQLite migration and query semantics only. It does not
claim compiled Rust/Wasm execution, Wrangler/D1 provider parity, remote
contention, production rollout, or security signoff.

## Stale pre-repair Wrangler artifact

The compiled Worker harness remains available for a future authorized rerun:

```sh
python3 scripts/ci/organization-d1-conformance.py
```

That harness creates an isolated temporary D1 database, applies the migration
chain, loads synthetic opaque fixtures, and reaches the repository only through
an exact loopback-only, per-run-token-gated Worker route. Its intended coverage
includes concurrent invite acceptance/replay, ownership and downgrade races,
folder closure moves, tombstone/recovery, support graph inspection, final D1
state, and redacted telemetry.

However, `target/evidence/organization-d1-conformance.json` was captured before
the current repairs. It is explicitly marked `stale_pre_repair`, its historical
result is `historical_pass_pre_repair`, and `validates_current_repairs` is
`false`. Its old migration/query digests and scenario claims must not be used as
evidence for this revision. A fresh pinned-Wrangler pass must replace it before
compiled Worker/D1 conformance can be claimed.

Neither local suite establishes legacy Cap fixture parity or shadow evaluation,
remote D1 contention/replication, a customer-approved retention window,
browser/public API penetration, OAuth or email-provider behavior,
production-scale graph repair, or production owner/security approval. The
current public handlers are not evidence that all organization traffic uses the
central repository; each missing protected record remains a rollout blocker.
