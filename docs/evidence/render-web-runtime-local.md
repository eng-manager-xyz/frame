# Render web runtime local evidence

This record covers the credential-free portion of issue 38. It proves the
composition root, Blueprint invariants, dependency boundary, real binary
startup, preview selection, stateless scale/restart behavior, and actual
SIGTERM in-flight drain on loopback. It is not Render provider evidence.

## Reproduction

```sh
ruby scripts/ci/check-yaml-syntax.rb
ruby scripts/ci/check-render-blueprint.rb
cargo test --locked -p frame-client --all-features
cargo test --locked -p frame-web
cargo clippy --locked -p frame-web --all-targets -- -D warnings
cargo clippy --locked -p frame-web --no-default-features --features hydrate \
  --target wasm32-unknown-unknown --bin frame-web-hydrate -- -D warnings
scripts/ci/check-workspace-boundaries.sh
python3 -I scripts/ci/build-web-hydration.py \
  --runtime-dir target/release/web-dist
python3 -I scripts/ci/check-web-hydration-bundle.py
cargo build --locked --release -p frame-web
python3 -I scripts/ci/render-web-runtime-smoke.py \
  --binary target/release/frame-web \
  --evidence target/evidence/render-web-runtime-local.json
```

The smoke launches the real release binary from isolated empty working
directories. It checks redacted fail-fast production configuration; the exact
`0.0.0.0:$PORT` production bind; liveness/readiness fields; protected
diagnostics; Render/direct proxy-header policy; preview canonical, staging API,
`noindex`, and cookie isolation; byte-identical responses from two independent
instances; absence of working-directory writes; and an in-flight HTTP request
that completes while a real `SIGTERM` drains and exits cleanly. The JSON stores
only a binary digest, bounded timings, booleans, and configuration classes.

The Rust tests additionally cover address precedence, origin/host parsing,
production/preview separation, weak diagnostic rejection, local-only test mode,
hash-verified assets, readiness fault behavior, bounded public DTO/transport,
preview public-versus-API origin separation, retry limits, redirect/body/content
type rejection, the three-failure/ten-second circuit breaker, and an
all-or-nothing bounded release join hidden behind the diagnostic token. The
join test is configuration-contract evidence, not verification of a Render,
Worker, or portfolio deployment ID.

The 2026-07-16 macOS local run exercised release binary SHA-256
`94bb5e9b3235ad9f7245fc50e5171094ec34f354e90b324ed97a926192ccb570`.
Production and preview reached HTTP liveness in 40.279 ms and 37.525 ms;
the two restart/scale instances reached it in 38.111 ms and 41.604 ms. The
400 ms in-flight handler completed and the process exited 244.815 ms after
`SIGTERM`, with status zero. These are loopback confidence measurements, not
Render startup, regional latency, or capacity results.

The dependency gate rejects `gstreamer`, `glib`, `frame-media`, `worker`,
`worker-sys`, and `frame-control-plane` from `frame-web`. The native web tree
contains the provider-neutral `frame-client` plus its Rustls HTTP transport; it
contains no Worker SDK or native media runtime. Hydration remains a separate
Wasm feature/build and cannot pull the native transport target dependency.
The observed normal dependency tree had 304 unique printed package lines and
zero forbidden matches. `otool -L` showed only macOS Security, CoreFoundation,
libiconv, and libSystem dynamic links—no GStreamer/GLib library.

## Evidence boundary

The following remain protected and must be attached to the immutable release
record with Render credentials and an eligible workspace:

- Render CLI 2.21.0/API schema validation and the workspace-specific plan;
- current price/entitlement approval under the USD 50 fixed monthly ceiling;
- clean Render build duration, startup timing, exact dependency/SBOM artifact,
  and native-runtime tool availability;
- Oregon versus representative-region latency and capacity/load measurements;
- real preview creation, staging-only credentials/data, expiry/deletion, logs,
  metrics, and cleanup;
- multi-node restart/scale distribution, zero-downtime deploy, failed
  build/start/readiness preservation of the preceding release, emergency
  restart, and named-commit rollback;
- custom-domain/TLS/Cloudflare Host behavior and disabling/re-enabling the
  default `onrender.com` hostname after issue 39; and
- production alert exports, observation window, and issue 40 approval.

An absent protected artifact blocks promotion. Local loopback timings and
semantic checks are never renamed as Render, provider, production, or regional
evidence.
