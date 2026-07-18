# Frame subdomain launch local evidence

Status: the complete provider-free Issue-44 contract is executable; production
launch evidence remains `not_collected` and the launch recommendation remains
`NO_GO_PROTECTED_EVIDENCE_REQUIRED`.

## Reproduce

Run from the repository root without provider credentials:

```sh
python3 -I scripts/ci/check-launch-observability.py
python3 -I scripts/ci/launch-game-day.py --self-test \
  --evidence target/evidence/launch-game-local.json
python3 -I scripts/ci/launch-go-no-go.py --self-test \
  --output target/evidence/launch-go-no-go-self-test.json
python3 -I scripts/ci/release-join-conformance.py --self-test
python3 -I scripts/ci/check-operational-hardening.py
python3 -I scripts/ci/check-cutover-decommission.py
python3 -I scripts/ci/check-same-origin-routing.py
python3 -I scripts/ci/check-cloudflare-edge-policy.py
python3 -I scripts/ci/check-browser-security.py
python3 -I scripts/ci/cross-repo-preview-e2e.py --self-test --timeout 20
```

The quality, production-preflight, and operational-hardening workflows call the
Issue-44 definition. The operational workflow retains only the two bounded
local JSON reports. Neither report contains provider responses, customer data,
contact details, credentials, URLs, or a production timestamp.

## What the local game proves

The deterministic logical-clock game validates the exact ordered steps for
eight journeys: portfolio link/return, DNS/TLS/SSR landing, API negotiation,
auth/session/CSRF/private boundary, generated direct-R2 upload/finalize/process/
cleanup, public `HEAD`/full/range/`416`/caption playback, cache/privacy behavior,
and portfolio availability across seven Frame failure classes.

It injects twelve distinct symptom boundaries—portfolio, DNS/TLS, edge route/
cache, Render, Worker, D1, R2, managed Media, native Media, public playback,
auth/privacy, and paired-release drift. Every modeled alert arrives inside its
30- or 60-second target and resolves to an owner and the launch runbook. These
are logical timings and assertion routing, not a page, dashboard export, or
provider clock measurement.

The game treats private HIT, stale deletion, and cookie variance as release
blockers. It validates current and N-1 contract-major compatibility and proves
an incompatible consumer yields `NO_GO`. Startup, SSR requests, concurrent
requests, upload intents, queues, and playback bytes each retain at least 30%
modeled headroom. No checked-in cost number is presented as spend approval.

All ten rollback definitions complete inside their local target and assert that
durable data is preserved and unrelated resources are unchanged. The game does
not change a portfolio, DNS, Cloudflare, Render, Worker, D1, R2, Media, native
worker, credential, route, or data-authority setting.

The mutation suite proves the assertions are live by rejecting a missing
journey, late alert, private HIT counted as success, incompatible release
promotion, insufficient headroom, data-deleting rollback, checked-in protected
evidence overclaim, and forbidden telemetry field.

## What the go/no-go self-test proves

The launch evaluator accepts only an exact 1 MiB-bounded redacted snapshot and
emits no snapshot values other than a safe ID, digest, scenario class, aggregate
pass/fail groups, and check codes. It recognizes one in-memory complete
*shape-validation* example, while keeping `authorizes_launch: false`. That
example is not provider evidence, not signed, and is not stored as a launch
record.

Thirteen controls then prove deterministic `NO_GO` for stale input, open P7
dependency, high defect, failed API SLO, late privacy alert, contract-major
drift, absent numeric cost approval, data-losing Worker rollback, a privacy
finding, portfolio coupling, missing protected evidence, incomplete sequence,
and an unexpected/forbidden field. Unknown fields fail before evaluation.

## Protected evidence still required

`fixtures/launch-observability/v1/protected-evidence.json` is authoritative and
all ten records remain `not_collected`:

1. signed P7 dependency closure and zero critical/high defects;
2. Cloudflare/Worker/Render/client dashboard exports, seeded alert delivery,
   on-call resolution, and acknowledgement;
3. provider-topology landing/API/auth/direct-upload/process/playback/cache
   synthetic history using approved generated media and exact cleanup;
4. a protected production response and provider comparison for the implemented
   safe `/health/release` join across Git SHA, contract major, Worker release,
   Render deploy, migration level, and current/N-1 portfolio consumers;
5. production-shaped capacity plus named numeric cost/quota approvals;
6. private-HIT/stale-delete/cookie-variance drill and telemetry/support audit;
7. DNS-only, certificate, Full (strict), broad route/normalization, and unrelated
   zone non-regression evidence;
8. portfolio availability/latency baseline during each Frame failure;
9. timestamped full rollback game day for every layer; and
10. signed staged launch, observation, remaining-risk, default-hostname,
    optional-feature, and legacy-path decisions.

Each record names the role that collects it, the exact bounded verifier command,
its required contents, and the acceptance claims it blocks. Run those commands
only from the approved protected environment and retain raw evidence there.
Local fixture digests and logical times never substitute for screenshots,
provider histories, bills, certificates, pages, signatures, or human decisions.
