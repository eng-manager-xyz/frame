# Cutover and decommission v1 fixtures

These fixtures freeze the Issue-35 ramp, cohort, go/no-go, per-profile routing,
reconciliation, retention, and decommission contracts. They contain synthetic
policy values and role names only. They contain no tenant identifiers, customer
data, credentials, provider exports, production observations, signatures, or
real authority changes.

`dashboard-scenarios.json` includes a synthetic all-pass branch solely to prove
the evaluator's positive logic. Every report is advisory and non-mutating.
`protected-evidence.json` is deliberately all `not_collected` and cannot be
satisfied by changing a repository file.

Run:

```sh
python3 -I scripts/ci/check-cutover-decommission.py
python3 -I scripts/ci/cutover_go_no_go.py --self-test
```
