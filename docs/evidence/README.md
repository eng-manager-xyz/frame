# Verification evidence

Repository checks produce reproducible evidence; provider and human gates are
attached to the immutable release record rather than fabricated in source.

## Local and CI evidence

The following are required from a clean checkout:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p frame-control-plane --target wasm32-unknown-unknown
cargo check -p frame-domain --target wasm32-unknown-unknown
cargo check -p frame-ports --target wasm32-unknown-unknown
cargo tree -p frame-web
cargo tree -p frame-client
```

CI additionally performs D1 migration, Worker bundle, production-mode web,
GStreamer probe/artifact, contract-fixture, forbidden-dependency, supply-chain,
secret, and hermetic journey checks. Test output must retain the first failure;
retries may establish flakiness but cannot replace a failing release result.

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
