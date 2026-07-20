# Frame Axum project profile

Use this profile for Axum work in this repository. Re-derive it when manifests,
the lockfile, architecture decisions, or server entry points change.

## Contents

- [Snapshot and authority](#snapshot-and-authority)
- [Dependency contract](#dependency-contract)
- [Runtime boundaries](#runtime-boundaries)
- [Current application shape](#current-application-shape)
- [Security and behavior invariants](#security-and-behavior-invariants)
- [Known-modern baseline](#known-modern-baseline)
- [Validation matrix](#validation-matrix)
- [Profile refresh triggers](#profile-refresh-triggers)

## Snapshot and authority

Snapshot date: **2026-07-19**.

Resolve conflicts in this order:

1. Current manifests, `Cargo.lock`, source, CI scripts, and deployment checks
2. Current architecture, operations, and security documentation
3. This dated profile

The repository pin, not this prose, is the compatibility authority. This
snapshot describes `main` at merge commit `4dd90de9ac3dc072631e23752707eabc223de6e9`.

## Dependency contract

| Item | Repository contract at snapshot |
| --- | --- |
| Workspace Axum requirement | `axum = "0.8.9"` |
| Locked Axum | `0.8.9` |
| Locked Axum Core | `0.5.6` |
| Locked Tower | `0.5.3` |
| Locked Tower HTTP | `0.6.11`, transitive through Reqwest only |
| Locked Hyper / Hyper Util | `1.10.1` / `0.1.20` |
| Locked HTTP / HTTP Body | `1.4.2` / `1.1.0` |
| Locked HTTP Body Util | `0.1.4` |
| Locked Tokio | `1.52.3` |
| Rust contract | edition 2024, rust-version 1.96, toolchain 1.96.1 |

Direct Axum consumers are only:

- `frame-web`
- `frame-media-worker`

Both currently inherit Axum's default features: `form`, `http1`, `json`,
`matched-path`, `original-uri`, `query`, `tokio`, `tower-log`, and `tracing`.
The `macros`, `multipart`, `ws`, and `http2` features are not enabled. There is
no `axum-extra` dependency. Do not mistake the lockfile's transitive
`tower-http` for an application middleware dependency.

There is also no direct `tower`, `hyper`, `hyper-util`, `http`, `http-body`, or
`http-body-util` dependency in either Axum application. Take HTTP types through
`axum::http`. A service-level test using `tower::ServiceExt` would require an
explicitly authorized direct dev-dependency; prefer existing live-router/server
tests when they prove the behavior without changing the manifest.

Feature tightening is a possible optimization, not an existing defect. If a
task explicitly changes features, inspect Cargo feature unification and verify
both binaries before removing defaults.

## Runtime boundaries

| Area | Axum role | Boundary |
| --- | --- | --- |
| `frame-web` | Native SSR/page/assets/health/CSP report server | Axum is under the non-Wasm target dependency table and the server module is SSR + native only. |
| Web hydration binary | Browser/Wasm hydration | Must remain Axum-free. |
| `frame-media-worker` | Native health listener and test-only loopback mock | Production job transport remains outbound Reqwest; GStreamer stays native. |
| `frame-control-plane` | Cloudflare Worker control plane | Must remain Axum-free. |
| Shared/domain/application crates | Portable logic | Must not gain an Axum dependency. |

The web process is a dedicated stateless native renderer. `/api*` remains owned
by the Cloudflare Worker/same-origin routing layer. Do not move D1, R2, Worker
credentials, durable application state, or control-plane APIs into Axum as an
incidental server change.

Consult these repository authorities when boundaries are implicated:

- `docs/architecture/workspace-boundaries.md`
- `docs/adr/0001-runtime-topology.md`
- `docs/adr/0004-engmanager-render-cloudflare-topology.md`
- `docs/adr/0005-leptos-rendering-and-tauri-shell.md`
- `docs/operations/render-web-service.md`
- `docs/operations/same-origin-routing.md`
- `docs/operations/gstreamer-runtime.md`
- `docs/security/threat-model.md`
- `scripts/ci/check-workspace-boundaries.sh`

Re-resolve these paths with `rg --files` whenever the profile is refreshed.

## Current application shape

### `frame-web`

- Builds one explicit flat `Router` for page, asset, health, CSP-report, and
  local-only drain routes plus a handler fallback.
- Uses 0.8 brace captures such as `/spaces/{resource_id}` and `/s/{video_id}`.
- Uses cloneable `AppState` with `State<AppState>` and finishes with
  `.with_state(state)`.
- Uses `Path<String>`, typed `Query`, `HeaderMap`, and `Bytes`; the body-consuming
  `Bytes` extractor is last.
- Uses `middleware::from_fn_with_state` for request/response policy and
  `DefaultBodyLimit::max(16 * 1024)`.
- Binds a Tokio `TcpListener`, calls `axum::serve`, and attaches bounded graceful
  shutdown. Render allows 60 seconds; the application drain is bounded to 55.
- Returns `Response` where policy varies and otherwise uses `Json`, `Redirect`,
  `StatusCode`, and response tuples.

Preserve the request-policy layer's forwarding/authority validation, canonical
redirect behavior, and cache/security/CSP/framing headers. Layer order and state
availability are observable behavior.

### `frame-media-worker`

- Serves only `/health/live` and `/health/ready` in production.
- Uses `Json` or `(StatusCode, Json)` responses without router state or layers.
- Uses `axum::serve` and Ctrl-C graceful shutdown coordinated with a bounded
  50-second consumer join.
- Uses a stateful Axum loopback mock only in tests; production jobs call the
  Worker with Reqwest.

## Security and behavior invariants

- Preserve same-origin ownership; do not add CORS middleware by reflex.
- Keep the web renderer stateless and keep privileged APIs in the control plane.
- Keep explicit body limits on untrusted body-consuming routes.
- Keep body consumers last and preserve typed rejection behavior.
- Preserve response-policy middleware coverage, including errors and fallbacks.
- Preserve bounded shutdown below the deployment platform's termination window.
- Preserve health endpoint availability during normal startup and shutdown
  behavior expected by operations.
- Treat route/function names containing `legacy_` as intentional product URL
  compatibility until repository evidence proves otherwise. They are not Axum
  legacy APIs and the audit must not flag them by name.

## Known-modern baseline

At the snapshot, targeted source and compiler checks found no deprecated or
removed Axum use. These current markers are intentional:

- `{name}` and `{*rest}` route syntax
- `State<T>` and `.with_state(state)` for application state
- `DefaultBodyLimit`
- non-generic `middleware::Next`
- `Request<axum::body::Body>` in middleware
- `axum::serve(listener, app)` with graceful shutdown
- bounded `axum::body::to_bytes` in tests

Do not ban `Extension<T>` globally; it remains current for per-request extension
data. Do not ban `into_make_service_with_connect_info`; it remains current when
connection metadata is actually required.

## Validation matrix

Select the smallest matrix that covers the changed boundary, then follow the
owning CI lane exactly.

### Always for Axum production-code changes

```sh
cargo fmt --all -- --check
RUSTFLAGS='-Ddeprecated' cargo check --locked -p frame-web -p frame-media-worker
scripts/ci/check-workspace-boundaries.sh
```

### `frame-web`

```sh
cargo test --locked -p frame-web --features ssr
cargo clippy --locked -p frame-web --all-targets --features ssr -- -D warnings
cargo check --locked -p frame-web --target wasm32-unknown-unknown \
  --no-default-features --features hydrate --bin frame-web-hydrate
cargo clippy --locked -p frame-web --target wasm32-unknown-unknown \
  --no-default-features --features hydrate --lib -- -D warnings
```

For route, middleware, HTML, asset, or server behavior, build the hydration
bundle and release web binary, run the service, and execute the owning
`share-player`, `leptos-authenticated-web`, and production smoke checks.

### `frame-media-worker`

In an environment with the trusted GStreamer runtime, run:

```sh
scripts/ci/gstreamer-sanitized-exec cargo test --locked \
  -p frame-media -p frame-media-worker
scripts/ci/gstreamer-sanitized-exec cargo clippy --locked \
  -p frame-media -p frame-media-worker --all-targets -- -D warnings
```

Probe both health routes when listener or lifecycle behavior changes.

### Dependencies and integration

- Run the sanitized workspace checks and production-gate release smoke when the
  dependency graph or cross-crate boundary changes.
- Require `git diff --exit-code -- Cargo.lock` when no dependency update was
  intended.
- Test requests through the real layer stack when routing, extraction, limits,
  redirects, fallback, or response policy changes; compilation alone is not
  behavioral evidence.

## Profile refresh triggers

Refresh this file when any of these changes:

- Axum or a companion crate resolves to a new version
- Axum features or target-specific dependency tables change
- another crate directly depends on Axum
- a new listener, router, middleware stack, protocol upgrade, or server target appears
- same-origin, renderer, Worker, media, health, or shutdown ownership changes
- CI renames or changes the authoritative validation commands
