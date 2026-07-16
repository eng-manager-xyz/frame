# D1 authentication repository conformance

Issue 13's provider-free D1 adapter evidence is generated through the compiled
Rust/Wasm Worker and pinned Wrangler `4.111.0` local D1 runtime. It is not
Python SQLite substitution evidence. Run it from a clean checkout with:

```sh
python3 scripts/ci/auth-d1-conformance.py
```

The suite creates an isolated temporary D1 database, applies all nine ordered
migrations, loads bounded opaque fixtures, and invokes the repository only
through an exact, loopback-only, per-run-token-gated Worker route. The token is
never written to the repository or the evidence artifact. CLI database access
is limited to migrations, controlled fixtures, and independent final-state
inspection.

The local report proves:

- session found/not-found, expiry, prior revocation, replay-family revocation,
  exact token-rotation replay reconstruction without family lockout,
  single-session logout, logout-all, and session-version invalidation;
- API-client rotation without introducing a CSRF digest, plus atomic mutation
  grant consumption and credential-generation fencing;
- API-key fallback-digest rotation persisted to the active key version, scope
  enforcement, and fail-closed corrupt-row decoding;
- verification success and replay rejection, stable issuance grants, concurrent
  identity-provisioning double-spend with exactly one winner, and privacy-safe
  decision audits;
- concurrent one-time-code and magic-link issuance against the same four
  existing abuse buckets, with both accepted after fresh-plan CAS retry, exact
  receipt replay after materialization, and exactly two challenge/outbox pairs
  before and after replay;
- concurrent distinct-bucket issuance from 4,095 live abuse buckets, producing
  exactly one accepted delivery and one semantic rate-limit decision at the
  4,096 cap with no adapter `Conflict` or `Unavailable` result;
- two distinct verification attempts and two distinct API-key authentications
  contending on the same rate-limit buckets, with both operations completing
  after fresh-plan optimistic-concurrency retries;
- suspended users or memberships, removed memberships, downgraded membership
  roles, and tombstoned organizations denying session, key, grant, and
  account-link paths before a grant, key, or identifier mutation can commit;
- verification delivery materialization, lease/retry, stale acknowledgement,
  final acknowledgement, two-dispatcher single-owner claiming, active
  attempt-12 lease preservation, and idempotent exhaustion tombstones;
- exact operation receipts reconstructing session, verification, API-key, and
  delivery results after the first successful response is deliberately
  discarded by the test caller;
- an injected audit-trigger failure whose message contains the private CAS
  token as extra provider text remaining `Unavailable` while rolling back the
  associated capability and session write atomically;
- all 108 checked-in auth queries compiling against the migration chain with
  external values bound through positional parameters; and
- actual Worker telemetry restricted to the fixed fields `event`, `operation`,
  `outcome`, `duration_ms`, and `rows`. The suite requires the
  exercised low-cardinality operation set and rejects unknown operation names
  or outcome shapes.

The machine-readable artifact is
`target/evidence/auth-d1-conformance.json`. The final passing local run recorded 63
telemetry events, migration digest
`bdc065c9e0bdca354ae19239792a7b1025aa1afea0ffb00f7a093f75a3090b52`, and
query digest
`50b89962b89bf527a288f28f809d8226400a1297ed6af3208fd3399afccc7f8f`.
SQL text, bindings, row values, token/API-key/OTP digests, provider errors,
identifiers, and temporary paths are excluded from telemetry and the report.

CAS conflicts depend on Wrangler `4.111.0`'s known multiline `D1Error`
envelope. The adapter accepts only the repository token as the entire trigger
message together with the exact trigger constraint class and extended code;
both stale row assertions and the rate-bucket cardinality precondition use
that owned marker.
Token prefixes/suffixes, check/unique classes, provider text, and unknown
envelope drift fail closed to `Unavailable`.

The suite statically gates that repository mutation batches await their D1
promise without the adapter's read deadline, but it does not inject a provider
delay and therefore does not claim provider-induced delayed-commit timing. Its
ambiguous-commit check discards a successful result and repeats the exact
operation; it proves deterministic receipt reconstruction, not a transport
that loses a post-commit response.

This evidence deliberately does not claim remote D1 replication or production
contention, provider email delivery, OAuth exchange, or legacy-session cutover.
`begin_oauth`, `preflight_oauth_exchange`, and `finalize_oauth_exchange` remain
explicitly unsupported until protected provider evidence exists. Those gates,
along with cross-client transport and forced-reauthentication rehearsal, remain
promotion blockers rather than locally fabricated passes.
