# Operational hardening local evidence

Scope: deterministic repository controls only. These commands require no
provider credentials, external service, customer data, or production access:

```sh
python3 -I scripts/ci/check-operational-hardening.py
python3 -I scripts/ci/release-provenance.py --self-test
python3 -I scripts/ci/support-bundle.py --self-test
python3 -I scripts/ci/restore-dr-rehearsal.py \
  --evidence target/evidence/operational-restore-local.json
python3 -I scripts/ci/operational-game.py \
  --evidence target/evidence/operational-game-local.json
```

The provenance self-test uses a temporary Ed25519 key and proves exact subject,
source, dependency, canonical statement, trusted-allowlist, and tamper-failure
semantics. It is not a production signature. The support-bundle test proves
the strict event allowlist and rejection of dynamically generated media,
token, signed-URL, raw-email, caption, path, and identifier-shaped inputs.

The restore rehearsal creates a synthetic database/file backup in a temporary
directory and passes manifest, corruption/missing-object, database integrity,
referential, auth, object checksum/range, local MP4 structure, configuration,
signing-catalog, and read-only project checks within the charter's local RPO/RTO
bound. It is not an encrypted D1/R2 or signing-custody restore.

The operational game exercises seven failure boundaries, twelve managed-media
fault/profile pairs, sustained/burst/exhausted capacity, and fail-closed region
selection. It proves deterministic local state transitions, not provider
outage injection, alert delivery, billed usage, or native production capacity.

The authoritative `protected-evidence.json` remains entirely
`not_collected`. Trusted release signatures/promotion/rollback, dashboards and
pages, independent penetration, provider secret rotation, D1/R2 restore,
signing-key recovery, managed Media game/change watch, production load/cost,
regional DR, contact acknowledgement, and signed desktop evidence require
external authorized records and block their corresponding acceptance claims.
