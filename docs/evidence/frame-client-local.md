# `frame-client` local verification

This evidence is synthetic and credential-free. It proves the Frame-side
public contract and does not claim a portfolio repository change, provider
deployment, production traffic, or a protected GitHub check.

## Implemented boundary

- core DTO, origin, error, and transport-abstraction code compiles without the
  native client feature and for `wasm32-unknown-unknown`;
- the optional native Reqwest/Rustls adapter disables redirects and ambient
  proxy use and enforces operation deadline and streaming body limits;
- the Worker exposes the raw privacy-safe `Health` DTO at `/api/v1/health`,
  while legacy `/health` diagnostics remain outside the public contract;
- public/private/deleted/failed/processing fixtures, an additive v1 fixture,
  malicious input tests, and a draft-2020-12 schema are checked in;
- `InstantUiProgressV1` is a versioned, Serde-backed projection with closed
  phase/error/retry invariants. Public shares admit only coarse upload/finalize
  state, while desktop-only local recovery and storage errors are rejected;
- the Worker derives optional processing state from retained D1 finalize truth
  without inventing percentages, and the Leptos page renders determinate or
  indeterminate progress without requesting media or exposing private fields;
- public media is served only from clean, active, public governed derivatives,
  and public DTOs never contain an object key or signed URL.

## Reproducible commands

```text
python3 -I scripts/ci/check-fixtures.py
python3 -I scripts/ci/check-frame-client-contract.py
cargo test --locked -p frame-client --all-features
cargo clippy --locked -p frame-client --all-targets --all-features -- -D warnings
cargo check --locked -p frame-client --no-default-features
cargo check --locked -p frame-client --no-default-features --target wasm32-unknown-unknown
cargo tree --locked -p frame-client --edges normal
cargo test --locked -p frame-control-plane --lib worker_health_and_share_are_consumable_by_frame_client
```

The dependency tree must contain only the Serde/JSON/URL core when features
are disabled; the optional native tree may additionally contain Reqwest and
its Rustls HTTP stack. The workspace boundary gate rejects Axum, Leptos,
Worker, GStreamer, `frame-media`, `frame-domain`, and `frame-ports` from this
crate.

Last local run: all 26 `frame-client` tests passed; strict all-target/all-feature
Clippy, native core check, wasm core check, nine-fixture/schema validation, and
the six-source/three-core-dependency boundary checker all passed. The focused
Worker serialization tests also passed and proved the public health object has
exactly `api_version`, `capabilities`, `release`, `service`, and `status`, and
that processing output is either a validated coarse D1-backed projection or
the existing indistinguishable unavailable representation.

## Protected completion boundary

The exact-SHA consumer compile and root `Cargo.lock` proof must run in the
pinned `engmanager.xyz` checkout. It remains a protected cross-repository
action until that repository is explicitly placed in scope. The local
two-origin preview harness exercises the degradation and fixture protocol but
does not claim to be that consumer lockfile evidence.
