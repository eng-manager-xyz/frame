# Frame subdomain launch fixtures v1

This directory is the machine-readable, provider-free contract for Issue 44.
It does not record a production launch.

- `launch-policy.json` binds role ownership, launch-specific SLOs and error
  budgets, telemetry exclusions, boundary dashboards and alerts, synthetic
  journeys, release/version joining, capacity, portfolio independence, the
  staged sequence, rollback layers, and post-launch decisions.
- `local-game.json` is deterministic logical-clock input. It uses generated
  media only and models all eight journeys, every named failure boundary,
  cache/privacy release blockers, current and N-1 consumers, six capacity
  dimensions, and all ten rollback layers. Its millisecond values are test
  fixtures, not provider measurements.
- `protected-evidence.json` is the launch blocker ledger. Every record remains
  `not_collected`, names an owner role and a bounded command, and states what
  acceptance it blocks. Checked-in local evidence cannot replace it.

Run the complete local definition and mutation controls with:

```sh
python3 -I scripts/ci/check-launch-observability.py
python3 -I scripts/ci/launch-game-day.py --self-test \
  --evidence target/evidence/launch-game-local.json
python3 -I scripts/ci/launch-go-no-go.py --self-test \
  --output target/evidence/launch-go-no-go-self-test.json
python3 -I scripts/ci/release-join-conformance.py --self-test
```

The game evidence contains only fixture digests, bounded aggregate results,
safe boundary names, timings, and booleans. Its recommendation always remains
`NO_GO_PROTECTED_EVIDENCE_REQUIRED`; it never changes DNS, provider settings,
traffic, data authority, portfolio content, credentials, or production state.

The go/no-go self-test creates an in-memory protected-*shape* example to prove
the evaluator can recognize a complete record, then proves stale evidence,
open dependencies/defects, failed SLOs, late alerts, release drift, unapproved
cost, unsafe rollback, privacy findings, portfolio coupling, missing protected
evidence, and an incomplete sequence all yield `NO_GO`. The synthetic shape is
not written as a launch record and never authorizes a launch.
