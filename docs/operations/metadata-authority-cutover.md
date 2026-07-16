# Metadata authority cutover and rollback

This runbook defines the issue-17 authority boundary. It is executable against
the credential-free local rehearsal, but it does not authorize a production
cutover. Production controls must run through the authenticated control plane,
use protected evidence and service bindings, and retain independent approval.

## Authority model

Every state is keyed by `(tenant, domain)` and contains exactly one writer:

| Phase | Writer | Asynchronous mirror | Forward transition gate |
|---|---|---|---|
| `legacy_authoritative` | legacy | off | approved shadow-read readiness |
| `shadow_read` | legacy | off | latest-window query coverage and clean reconciliation |
| `dual_write` | legacy | D1 | drained replay, clean shadow window, legacy fence, rollback rehearsal |
| `d1_authoritative` | D1 | legacy | observation window, clean reconciliation, drained reverse replay |
| `rolled_back` | legacy | D1 | new clean window before another canary |
| `finalized` | D1 | off | terminal; issue 35 owns production finalization |

`dual_write` is deliberately named for migration familiarity, but it never means
two authoritative synchronous writers. The legacy writer commits once and its
durable capture is replayed asynchronously to D1. During the reversible D1 canary,
D1 commits once and its durable capture is replayed asynchronously to legacy.
The state schema and typed contracts cannot represent a `dual_write` authority.

Migration `0006_etl_cutover_expand.sql` remains the released singleton,
fail-closed compatibility gate used by older Workers. Migration
`0012_scoped_cutover_controls.sql` adds the tenant/domain authority, immutable
audit, encrypted capture envelope, digest-only shadow observation, SLO, signal,
required-query, and maintenance-window tables. Captured event envelopes,
observations, required-query definitions, signal events/rollups, SLO policy, and
maintenance windows are immutable. Do not repurpose or contract the singleton
until all deployed readers have moved to the scoped contract.

The D1 runtime reads one scoped row, one SLO row, and the released singleton in
one statement. Current-phase health is derived from append-only events carrying
the exact stable `phase_epoch` and inside `max(phase_started_at_ms, now -
shadow_window_ms)`, so clean observations or signals from an earlier phase
cannot be reused. Pause/resume advances the authority epoch without changing
the phase epoch; an actual phase transition advances both. Every required query
class is bound to its approved normalization digest; an unlisted or differently
normalized observation is rejected before storage.

Every D1 application mutation must call
`D1CutoverAuthorityRepository::execute_fenced_batch`. The method inserts the
exact `(tenant, domain, writer, epoch, occurred_at)` assertion, runs the caller's
mutation statements, and removes the assertion in one D1 batch. A stale epoch,
wrong tenant/domain, wrong writer, or backdated mutation aborts the entire
transaction. A separately fetched status or `AuthorityFence` is not sufficient
authorization by itself.

Authenticated control handlers pass only already-approved commands to
`transition` or `replay_control`. Each method recomputes the evidence and audit
digests, asserts the current scope/epoch/audit head and live database health,
appends the audit record, updates authority, verifies the resulting state, and
removes both assertion rows in one D1 batch. A drained-replay or maintenance
window failure therefore cannot leave a successful audit or a partially moved
authority row. `finalized` is deliberately rejected here because issue 35 owns
that irreversible production boundary.

The Worker exposes the isolated runtime through six exact, administrator-only
routes. Every request requires explicit bearer authentication, `frame:admin`,
an active owner/admin membership, an exact `x-frame-tenant-id` match, the
primary production host, and a bounded JSON body where applicable:

- `GET /api/v1/operations/cutover/{tenant}/{domain}`;
- `POST /api/v1/operations/cutover/{tenant}/{domain}/transition`;
- `POST /api/v1/operations/cutover/{tenant}/{domain}/replay/pause`;
- `POST /api/v1/operations/cutover/{tenant}/{domain}/replay/resume`;
- `POST /api/v1/operations/cutover/{tenant}/{domain}/signals`; and
- `POST /api/v1/operations/cutover/{tenant}/{domain}/shadow-observations`.

The server derives the operator digest and observation timestamp; callers never
supply operator identity or raw comparison values. The current Worker mutation
paths—including request commands, idempotency expiry, storage governance,
native output recovery, and asynchronous managed-media transitions—execute
through the same scoped writer assertion. This is checked-in route and adapter
wiring, not evidence that a named production deployment or its legacy adapters
have been exercised.

Shadow ingestion accepts only lower-case SHA-256 digests, a bounded query-class
code, a fixed classification, the exact tenant/domain, and the current phase
epoch. It never accepts raw result rows. Signal ingestion also requires the
current phase epoch. These parameters must come from the status snapshot used
for the comparison or signal-producing operation; a separately guessed epoch
is rejected. Retrying the exact same shadow digest is idempotent, while reusing
that digest for different result digests or a different classification aborts.

## Before any transition

1. Choose exactly one tenant and approved domain. Its domain and one-way tenant
   digest must be allowlisted by the immutable plan. Confirm its current phase,
   epoch, writer, mirror state, replay state, and audit head.
2. Confirm the planned time is inside an approved, non-overlapping maintenance
   window. An emergency `d1_authoritative` to `rolled_back` transition is the
   only transition permitted outside that window.
3. Verify the operator is authorized for the control scope. The local credential
   file is a non-symlinked, owner-owned `0600` regular file; production uses the
   control-plane identity and approval record, not the public fixture phrase.
4. Run every charter-critical shadow query named by the immutable plan. Each
   query must meet `minimum_shadow_observations` after entry into the current
   phase and inside the latest `shadow_window_ms`; an old clean observation
   from the prior phase cannot promote a new window. Pause/resume controls do
   not reset the phase boundary or erase an operational signal.
5. Require zero unexplained shadow mismatches, a clean reconciliation digest,
   replay lag at or below `max_pending_lag_ms`, and dead-letter/contention counts
   at or below their explicit limits.
6. Verify the full audit hash chain. A stored audit-head value without
   recomputation is not sufficient evidence.

The local configuration in `fixtures/etl/v1/plan.json` is synthetic. Production
limits and query classes require review against measured workload and the
migration charter.

## Local command sequence

All examples use an owner-private encrypted working directory. Reports contain
only digests, bounded labels, counts, phases, epochs, and alert states; the state
database and captured events still contain protected values.

```sh
python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  init --tenant tenant-a --domain metadata --at-ms 1735689700000

python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  transition --tenant tenant-a --domain metadata --to shadow_read \
  --expected-epoch 0 --operator-file /protected/operator.credential \
  --evidence /protected/evidence-shadow.json --at-ms 1735689700001

python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  shadow --domain metadata --observation /protected/shadow-observation.json \
  --at-ms 1735689700002 --report /protected/shadow-report.json

python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  transition --tenant tenant-a --domain metadata --to dual_write \
  --expected-epoch 1 --operator-file /protected/operator.credential \
  --evidence /protected/evidence-dual.json --at-ms 1735689700003

python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  capture --domain metadata --events /protected/events.ndjson \
  --at-ms 1735689700004

python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  replay --tenant tenant-a --domain metadata --target /protected/target.sqlite \
  --at-ms 1735689700005 --max-events 1000

python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  status --tenant tenant-a --domain metadata --now-ms 1735689700006 \
  --report /protected/cutover-status.json
```

Use `control --action pause|resume --expected-epoch N` to stop or resume replay.
Pause is serialized with each target commit and source acknowledgement: it cannot
return while an event is between those boundaries. Resume is a forward control
and therefore requires a maintenance window. Both actions advance the epoch and
append an audit entry.

Before a writer mutation, validate the tenant, domain, writer, and expected epoch
at the same transactional boundary as the mutation. The local
`verify-fence --writer legacy|d1 --expected-epoch N` report proves the stored
state and audit chain at one instant; it is not a reusable authorization token.
Application adapters must reject a stale fence after any transition or replay
control changes the epoch.

Run `verify-audit` before and after every rehearsal:

```sh
python3 scripts/migration/cutover.py \
  --config /protected/plan.json --state /protected/cutover.sqlite \
  verify-audit --tenant tenant-a --domain metadata \
  --report /protected/cutover-audit-verification.json
```

The repository-local D1 authority proof is:

```sh
python3 scripts/ci/cutover-authority-conformance.py
cargo test -p frame-control-plane --test cutover_authority_v1
```

It verifies singleton visibility, tenant/domain isolation, exact writer/epoch
fencing, current-phase observation freshness, append-only operational signals,
rollup consistency, atomic audited transitions, replay-drain gating, and
pause/resume maintenance-window enforcement without contacting Cloudflare or
MySQL.

## Replay incidents and conflict rules

- A target failure before commit increments `replay_write_failure`, leaves the
  event pending, and stops the batch. Restore target health and replay in exact
  sequence order.
- A target commit followed by a lost source acknowledgement leaves a target
  ledger entry. The next replay verifies event digest, tenant, domain, and
  sequence, then acknowledges the source event without applying it twice.
- A malformed or constraint-invalid payload is dead-lettered with a bounded
  reason code and stops later sequence processing. Never skip it silently.
- Reusing an event ID with different content or a sequence with a different
  event fails closed. Resolve the source conflict and preserve both protected
  artifacts for incident review.
- Event identity is `(tenant, domain, event_id)`, so an identifier in one tenant
  cannot reserve another tenant's namespace. New events start at sequence one
  and remain contiguous per tenant/domain; gaps fail at capture instead of
  silently stalling replay.
- Every event carries `source_authority`. Capture compares it with the current
  writer and exact epoch before publication. `dual_write` and `rolled_back`
  accept only legacy-writer events; a reversible D1 canary accepts only D1-writer
  events. Reusing an old event is idempotent but cannot change its digest.
- Both deterministic upserts and tenant-scoped deletes are replayed through the
  same target ledger. Before target I/O, replay recomputes the payload digest and
  binds tenant, domain, event ID, sequence, epoch, and source authority to the
  durable envelope.
- A stale authority epoch increments `authority_contention` and performs no
  transition. Reload state; never retry by guessing the next epoch.

The local test replays a synthetic D1-canary command into an isolated legacy
projection before rollback. The production reverse adapter must encode the same
logical operation for MySQL, preserve its source-acknowledgement/target-ledger
boundary, and reconcile the real legacy schema; the SQLite projection is not proof
that this provider-specific adapter works.

The status report exposes alerts for replay lag, replay write failure, lost
acknowledgement, dead letters, authority contention, current-phase query
coverage, shadow mismatch, and rollback readiness. Signals are append-only
events, so the window count is exact rather than a cumulative counter inferred
from only its latest timestamp. Forward promotion fails while those current-
phase SLOs are unhealthy. Alert payloads must not add emails, row values, source
keys, captured payloads, credentials, or provider errors.

## Timed rollback rehearsal

Start the timer before the D1 fence. In order:

1. pause new canary expansion and page the authority owner;
2. drain or explicitly disposition every in-flight operation;
3. fence D1 at the current epoch inside the write transaction;
4. replay every D1-acknowledged canary change back to legacy in order;
5. reconcile legacy against the canary ledger and require zero unexplained
   differences;
6. submit bounded rollback evidence with `d1_fenced`, `legacy_caught_up`, and
   `rollback_rehearsed`, then transition to `rolled_back` with the current epoch;
7. verify the legacy fence and audit chain, resume the legacy writer, and stop
   the timer only after synthetic reads and writes succeed.

The repository test records a local synthetic tenant/domain cutover and rollback
duration. It is not production-scale. Promotion remains blocked until two
representative protected rehearsals preserve canary writes and complete within
the charter's 15-minute rollback window, with alert delivery, provider limits,
and independent human approval attached to the immutable release record.

## Evidence retention

Attach only redacted reports: transition/audit digests, status counts and alert
states, reconciliation digests, query coverage counts, elapsed time, release
revision, and approver digests. Keep the state database, event payloads, raw
shadow results, operator material, provider traces, and source/target snapshots
on approved encrypted storage. A missing protected artifact blocks promotion;
local success must never be relabeled as production evidence.
