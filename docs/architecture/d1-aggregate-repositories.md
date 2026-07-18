# D1 aggregate repository patterns

The control-plane D1 boundary is `AggregateRepository`. It exposes
authorization-ready video reads, upload aggregates, media-job aggregates,
native-worker job aggregates, organization snapshots, video keyset pages, and
bounded bulk video reads. It does not expose generic table CRUD or accept SQL
fragments from callers. Runtime SQL is checked in under
`apps/control-plane/queries`; every external value is a D1 bound parameter.
Bulk lookup generates only positional placeholder names.

## Query and response bounds

- Page sizes are `1..=100`; the default is 25. A page fetches one lookahead
  row and emits a next cursor only when another page exists.
- Cursors are canonical lowercase, versioned encodings of the exact
  `(created_at_ms, id)` ordering boundary. Invalid versions, lengths,
  timestamps above JavaScript's safe integer, uppercase/non-hex input, and a
  nil identifier fail validation before D1 is called.
- A statement uses at most 100 bound parameters. Bulk video reads reserve one
  parameter for the tenant and split at 99 identifiers. A request accepts at
  most 1,000 validated identifiers and de-duplicates them before querying.
  Results are returned only after every chunk succeeds; a later chunk failure
  discards the accumulated local rows rather than returning a partial page.
- Result rows are decoded with checked field counts/types and then validated
  against bounded public states, timestamps, revisions, identifiers, and
  titles. Malformed persisted data produces a fixed corrupt-result failure;
  it cannot panic through D1's convenience result decoder.
- Organization snapshot upload and media-job counts join each child back to
  its video's primary key and require both rows to carry the same organization.
  A denormalized child therefore contributes to neither tenant's count.

The keyset page index is defined in migration 0008. Local conformance retains
the real D1 query plan and fails if pagination regresses to a video-table scan.

## Atomic writes and read-after-write behavior

The video-title compare-and-swap command submits one transient operation
statement through `D1Database.batch`. Migration 0008 owns explicit SQLite
triggers for that operation. The `BEFORE INSERT` trigger reserves
`(organization_id, idempotency_key)`, checks actor/tenant authority, applies the
optimistic revision update, inserts the outbox event, and stores the exact
response. After every mutation, a `changes() = 1` guard uses `RAISE(ABORT)` so a
missing reservation, stale aggregate, constraint failure, or incomplete
response rolls back the whole statement. The final statement in the same
`BEFORE INSERT` trigger is `RAISE(IGNORE)`: it suppresses the transient envelope
row while retaining the trigger's prior guarded effects. Actual Worker D1
batch metadata reports `changes = 4` for those four retained effects (command
reservation, aggregate update, outbox insert, and stored response), while the
ignored outer insert contributes no envelope row. The repository requires that
exact metadata value while reporting one logical command result, and
conformance independently verifies the aggregate, command, outbox, and
empty-envelope state. An exact replay reads the stored response; a different
digest under the same key is a conflict. The command digest and trigger both
bind the actor fetched for authorization, so callers cannot replace the actor
after the read.

Within one request, a command's follow-up read uses the same Worker D1 binding
after the batch completes. Cross-request transactions do not exist. Work that
also touches R2 or a media executor uses an idempotent state machine plus
manifest/outbox reconciliation; it never presents D1 and an external provider
write as one transaction.

The selected Rust Workers D1 binding does not expose session bookmarks.
Consequently this implementation records bookmark use as unavailable and does
not claim production replication-lag or bookmark evidence. Workflows that
eventually require that behavior must add a capability-specific adapter and a
protected injected-lag/provider test before changing this policy.

## Errors and telemetry

Repository failures map to the fixed codes `repository_invalid_request`,
`repository_conflict`, `repository_timeout`, `repository_unavailable`, and
`repository_corrupt_result`. Public handlers retain their versioned API error
contracts. Raw D1 errors, SQL, bound values, tenant/aggregate identifiers,
bindings, and row data are never copied into the error or log.

Each repository query emits one structured telemetry record containing only
query class, elapsed milliseconds, returned row count, retry count, bookmark
availability, and fixed outcome code. Query classes are an allowlist rather
than caller-controlled strings.

## Verification

Run `scripts/frame d1-conformance`. The harness starts the compiled Worker
against isolated Wrangler local D1 and exercises atomic rollback,
duplicate/conflicting commands, stale revisions, concurrent HTTP contention,
an expired pre-dispatch adapter deadline, tenant isolation, pagination,
parameter chunking, actual JSON telemetry, and query plans. It does not claim
in-flight provider cancellation or a provider network timeout. Its exact
evidence boundary is documented in
[`docs/evidence/d1-repository-local.md`](../evidence/d1-repository-local.md).
