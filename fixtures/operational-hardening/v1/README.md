# Operational hardening v1 fixtures

These machine-checked fixtures freeze the Issue-34 service/SLO, release,
telemetry, security, recovery, capacity, residency, managed-media, and game-day
contracts. They contain synthetic policy data only. They do not contain
provider exports, personal contacts, credentials, media, captions, customer
identifiers, or production observations.

`protected-evidence.json` is deliberately all `not_collected`. Changing a row
to a passing state requires an immutable external evidence digest, an
authorized actor, and the protected procedure named by the corresponding
runbook; a pull-request edit is not evidence.

Run the network-free gate with:

```sh
python3 -I scripts/ci/check-operational-hardening.py
python3 -I scripts/ci/release-provenance.py --self-test
python3 -I scripts/ci/support-bundle.py --self-test
python3 -I scripts/ci/restore-dr-rehearsal.py
python3 -I scripts/ci/operational-game.py
```
