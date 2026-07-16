# Progressive cutover and decommission local evidence

This record covers only the credential-free Issue-35 contract. It proves that
the repository has one machine-checked ramp plan, a complete cohort dimension
matrix, a deterministic non-mutating dashboard evaluator, a per-profile media
disposition, final reconciliation requirements, and a complete legacy
retention/decommission inventory. It does not prove that production moved.

Run from the repository root:

```sh
python3 -I scripts/ci/check-cutover-decommission.py
python3 -I scripts/ci/cutover_go_no_go.py --self-test \
  --output target/evidence/cutover-dashboard-local.json
python3 scripts/ci/cutover-authority-conformance.py
cargo test -p frame-control-plane --test cutover_authority_v1
```

The local checker validates:

- a monotonic `0% -> 1% -> 5% -> 25% -> 50% -> 100% reversible ->
  irreversible` sequence with minimum observation windows and explicit next
  states;
- synthetic, internal, representative hosted, BYO/migration-input, desktop,
  browser, managed/native/fallback/external-media, and high-risk workflow
  dimensions without checked-in tenant identifiers;
- one advisory evaluator combining P0-P5 readiness, unowned blockers, charter
  SLOs, parity, support, metadata/object reconciliation, replay/backlog, current
  and N-1 clients, adoption, capacity, rollback, managed-media cost/limits/error/
  fallback/quality/publication, and immutable evidence digests;
- deterministic seeded `GO` and `NO_GO` results, including missing phase,
  stale input, parity, reconciliation, rollback, capacity, client, media, and
  irreversible-approval failures;
- `authorizes_transition: false` and `production_authority_changed: false` on
  every evaluator result;
- all sixteen profiles in the media catalog, each with an exact primary,
  operational fallback, legacy cutover rollback, revision kill switch, and
  queued/claimed/started/staged/published/canceling/indeterminate disposition;
- final MySQL/D1 row/relationship/hash/aggregate/semantic/checkpoint and object
  count/bytes/role/SHA-256/probe/missing/duplicate/orphan/ownership/provenance
  requirements with zero unexplained differences; and
- services, routes, queues, databases, buckets, secrets, DNS, scheduled jobs,
  clients, dashboards, runbooks, and billing inventory, all explicitly
  `planned_not_executed` and without automated destructive actions.

The Issue-17 conformance and Rust tests independently prove the checked-in
tenant/domain authority SQL, audit chain, current-window health, one-writer
fence, replay gating, control routes, and same-transaction application mutation
assertions. Their local SQLite/fake-D1 results are prerequisites, not evidence
that a production legacy/D1 adapter or protected operator procedure ran.

The dashboard fixture's synthetic all-pass case deliberately recommends `GO`
to exercise the positive branch. Its zero-filled evidence digests and fixed
metrics are labeled `synthetic_local_validation`; they are neither signatures
nor production measurements. The protected-shape case remains `NO_GO` because
external evidence is absent.

Fourteen protected record classes remain `not_collected`: phase signatures,
cohort/consent records, per-stage approvals, canary observation, managed-media
cost/quality, production-scale rollback, final metadata and object
reconciliation, rollback-expiry/retention approval, legacy write/schedule
drain, credential revocation, decommission observation, cost cleanup, and the
customer/support report plus retrospective. No production authority, legacy
resource, credential, DNS, provider contract, source object, or database was
changed or revoked by these checks.

See [the production runbook](../operations/progressive-cutover-decommission.md)
for owners, timestamps, stop conditions, control boundaries, communications,
rollback timing, reconciliation, irreversible approval, and the itemized
retention/decommission sequence.
