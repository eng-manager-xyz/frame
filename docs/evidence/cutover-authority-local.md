# Cutover authority local evidence

This evidence is credential-free and provider-free. It proves the local
Issue-17 control contract; it does not authorize a production cutover.

Run from the repository root:

```sh
python3 scripts/ci/check-migrations.py
python3 scripts/ci/cutover-authority-conformance.py
python3 -I scripts/migration/test_local.py
cargo test -p frame-control-plane --test cutover_authority_v1
cargo clippy -p frame-control-plane --test cutover_authority_v1 -- -D warnings
cargo check --locked -p frame-control-plane --target wasm32-unknown-unknown
```

The SQLite proof applies the complete ordered migration set and executes the
same checked-in SQL embedded by the Worker adapter. It verifies:

- the released singleton remains present and visible as a compatibility gate;
- each status read returns exactly one tenant/domain phase and epoch;
- shadow coverage counts only approved query classes, approved normalization
  digests, and observations in the latest current-phase window;
- old operational events remain immutable but do not pollute the current SLO
  window, while exact cumulative rollups remain bound to the event log;
- observations and signals carry a stable phase epoch, preventing reuse across
  transitions without erasing a window when replay is paused or resumed;
- exact shadow-observation retries are idempotent, but a digest reused with a
  changed envelope fails closed;
- captured change envelopes cannot be rewritten or deleted and advance only
  from pending to applied or dead letter;
- a writer assertion accepts only the exact tenant, domain, writer, epoch, and
  non-backdated occurrence time; and
- the assertion, application mutation, and cleanup share one transaction;
- an undrained replay blocks D1 authority without appending an audit record or
  changing the scoped state; and
- transition, pause, and resume bind their audit append, state update,
  postcondition, and cleanup atomically, with resume restricted to an approved
  maintenance window;
- the exact scoped status, transition, replay-control, signal, and shadow paths
  are closed router variants rather than wildcard-decoded endpoints; and
- request mutations, idempotency cleanup, storage-governance commands, native
  recovery, and managed-media orchestration all enter D1 through a same-batch
  scoped writer assertion in production mode.

The Rust test validates strict D1 row decoding, phase/writer/mirror invariants,
wire-safe integers and booleans, state/policy freshness, signal health,
singleton compatibility conflict detection, and stale fence rejection. Adapter
errors expose only stable codes. Pure tests also bind approved transition
evidence, reconciliation digests, replay controls, and audit hashes to the exact
scope and epoch.

Protected evidence is still required for a named Worker/D1 deployment, a real
authenticated operator exercise, alert delivery, real legacy/D1 forward and
reverse replay, two production-scale rehearsals, canary-write preservation, and
rollback completion inside the charter window. All local reports remain
`production_evidence: false` in meaning even when every local check passes.
