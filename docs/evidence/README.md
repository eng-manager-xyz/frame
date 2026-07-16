# Verification evidence

Repository checks produce reproducible evidence; provider and human gates are
attached to the immutable release record rather than fabricated in source.

The [checkbox closure inventory](../../fixtures/closure/v1/README.md) assigns
all 553 issue deliverable/acceptance checkboxes to locally satisfied,
protected-pending, or true local-gap status. Its checker pins the complete
requirement text and fails on missing evidence or classification drift:

```sh
python3 -I scripts/ci/check-issue-closure.py
```

This audit is a ledger, not a waiver: protected records remain pending and a
local gap remains open even when a surrounding design document or fake passes.

## Local and CI evidence

The [API and workflow parity v1 record](api-workflow-parity-local.md) freezes the
pinned route/action/RPC/workflow inventory and the local admission, webhook,
compatibility, retry, fault, and state-machine load evidence. It explicitly
does not promote endpoint adapters, protected providers, or retirements.

The [Leptos authenticated-web local record](leptos-authenticated-web-local.md)
binds 17 private routes to owner/admin/member and denial states, form/query
contracts, legacy redirects, and loopback SSR/privacy/performance checks. It
explicitly leaves production adapters, cross-browser visual diffs, named
screen-reader checks, billing/provider journeys, and route cutover pending.

The [share/player v1 local record](share-player-local.md) binds the privacy
matrix, descriptor scope, range planner, accessible SSR/hydration controls,
embed protocol, comments, transcripts, analytics consent, legacy links, custom
domains, and cache revisions. Its browser/device rows remain explicitly
protected pending real media, providers, assistive technology, and production
adapters.

The following are required from a clean checkout:

```sh
scripts/frame check
scripts/frame test
cargo check -p frame-control-plane --target wasm32-unknown-unknown
cargo check -p frame-domain --target wasm32-unknown-unknown
cargo check -p frame-ports --target wasm32-unknown-unknown
cargo tree -p frame-web
cargo tree -p frame-client
```

CI additionally performs D1 migration, Worker bundle, production-mode web,
GStreamer probe/artifact, contract-fixture, forbidden-dependency, supply-chain,
secret, hermetic journey, and credential-free two-origin cross-repository
preview checks. See [Cross-repository local preview evidence](cross-repo-preview-local.md)
and [D1 aggregate repository conformance](d1-repository-local.md) for the exact
local proofs and their deliberately narrow evidence boundaries. Test output
must retain the first failure; retries may establish flakiness but cannot
replace a failing release result.

Organization policy, race, tenant-boundary, retention, and dry-run graph-repair
proof is bounded in
[organization repository local conformance](organization-d1-local.md). The
current repaired SQL semantics have network-free SQLite evidence; the older
compiled Wrangler artifact is explicitly stale until rerun.

The [local media-conformance record](media-conformance-local.md) freezes the
Issue-29 matrix, comparator, boundary, fault, idempotency, resource, dashboard,
and seeded-fuzz contracts. It explicitly leaves provider, cross-executor,
hardware, permission, and one-hour platform evidence protected.

Storage authorization, signed-grant, custom-domain, cache-generation,
manifest-lifecycle, hold, restore, export, and erasure contracts are bounded in
[storage governance local evidence](storage-governance-local.md). Provider
cache timing, bucket/encryption inspection, malware, and real erasure remain
protected rather than inferred from those local tests.

## Protected evidence

These records require trusted provider or representative hardware access and
are never synthesized by a local unit test:

- R2 conditional/range/multipart/CORS and R2-to-Media-to-R2 traces;
- Render Blueprint validation, build/start/readiness, preview isolation,
  SIGTERM drain, scale/restart, deploy and rollback records;
- DNS-only certificate, proxied Full (strict), Worker-route, cache HIT/bypass,
  CAA/renewal, WAF/rate, and default-origin tests;
- macOS/Windows/Linux clean install, capture/permissions/device/hardware-codec,
  A/V drift, power-loss recovery, signing and updater evidence;
- production-shaped MySQL/D1 and object migration rehearsals, restore,
  reconciliation, canary observation, and timed authority rollback;
- browser/device/accessibility/security matrices and manual screen-reader
  walkthroughs;
- capacity, cost/quota, privacy audit, alert screenshots/exports, incident game
  day, and final go/no-go approvals.

Protected jobs use synthetic data, isolated resources, scoped credentials,
bounded cost/time, redacted artifacts, explicit cleanup, and production
concurrency one. An absent record blocks the corresponding promotion; it is
not converted into a checked box by documentation alone.
