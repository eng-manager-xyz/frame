# ADR 0005: Leptos rendering and Tauri desktop shell

Status: accepted

## Context

Frame needs useful HTML and metadata before JavaScript on the public web, while
the desktop product runs inside a native WebView and must obtain all privileged
state from an allowlisted Rust command boundary. Tauri serves static frontend
assets and does not provide a server runtime inside the desktop application.

## Decision

- `frame-web` uses Leptos SSR behind Axum plus progressive Leptos hydration
  islands. Native dependencies select only Leptos' `ssr` feature; the separate
  `frame-web-hydrate` Wasm binary selects only `hydrate`.
- Web hydration is deliberately island-scoped. Axum remains authoritative for
  route, metadata, privacy, session, role, loading, and error state. The browser
  hydrates only shared, data-free components whose exact initial markup was
  rendered by the server. Full-body hydration would require the browser to
  reconstruct server-authorized private state and would risk a mismatch or a
  flash/leak, so it is prohibited until a typed same-origin bootstrap exists.
- Trunk produces a clean, locked bundle. A post-build manifest names every
  asset by its full SHA-256; Axum verifies the manifest, filename, digest,
  type, and size before injecting any script tag. The verified assets receive
  immutable caching. A missing or tampered bundle removes both injection
  points and leaves the useful no-JavaScript page intact.
- `frame-desktop-ui` uses Leptos CSR compiled to Wasm by Trunk. Tauri 2 embeds
  the fingerprinted `ui/dist` output; no HTTP server ships in the desktop app.
- The desktop and web targets select their Leptos features in their own
  manifests. The workspace dependency intentionally selects neither rendering
  mode, preventing accidental server code in the desktop Wasm graph.
- The desktop frontend deserializes the native bootstrap result into the same
  `ShellCapabilities` Rust type returned by the Tauri command. A protocol and
  backend-truth check must pass before the UI enables native work.
- Tauri's global bridge is enabled because that is the supported Tauri/Leptos
  integration. Each of the four registered commands re-checks the exact `main`
  window label. The build manifest and capability file allow only those
  commands and grant no filesystem, shell, process, network, or dialog
  permission, and the CSP rejects remote scripts.
- Desktop runtime builds currently support macOS and Windows. Linux remains
  disabled until the time-bounded GLib advisory record in
  `docs/security/dependency-policy.md` can be removed.
- A portable `tauri-app` build deliberately selects `Unavailable` in release
  mode. On macOS, adding `macos-native` composes the narrow
  `NativeMacOsDisplay` adapter; construction includes trusted GStreamer
  recorder preflight and native-source initialization and falls back to
  `Unavailable` if either cannot be established. ScreenCaptureKit permission
  preflight/request occurs only when the user prepares capture. The debug-only
  deterministic fake remains separately gated by `FRAME_DESKTOP_FAKE_PIPELINE=1`.
- `NativeMacOsDisplay` is not the complete recorder described by issues 24,
  27, and 33. It enumerates opaque displays, requests screen-recording
  permission, records one full display with embedded cursor and Frame-owned
  window exclusion, optionally includes exact 48 kHz stereo system audio, and
  seals and safely publishes an Editable WebM artifact. Window/region capture,
  microphone, camera, pause/resume, multitrack Studio, edit-aware export,
  recovery, MP4 distribution, updater, and native lifecycle integrations remain
  unavailable.

## Commands and evidence

From the repository root:

```sh
cargo test --locked -p frame-desktop-core
cargo test --locked -p frame-desktop-core --features tauri-app --bin frame-desktop
cargo clippy --locked -p frame-desktop-ui --no-default-features --features csr --target wasm32-unknown-unknown -- -D warnings
python3 scripts/ci/build-desktop-ui.py
python3 scripts/ci/check-desktop-bundle.py
# Portable macOS/Windows shell: release adapter remains Unavailable.
cargo build --locked --release -p frame-desktop-core --features tauri-app,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter unavailable

# Native macOS display-only shell, run on a macOS build machine with the
# audited GStreamer installation discovered by pkg-config.
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display

cargo test --locked -p frame-web
cargo clippy --locked -p frame-web --no-default-features --features hydrate --target wasm32-unknown-unknown --bin frame-web-hydrate -- -D warnings
python3 -I scripts/ci/build-web-hydration.py
python3 -I scripts/ci/check-web-hydration-bundle.py
python3 -I scripts/ci/web-hydration-smoke.py --origin http://127.0.0.1:3000
```

`quality-gates.yml` repeats the portable command boundary, Wasm bundle, and
release build on both supported operating systems and retains the exact binary
and frontend bundle. That cross-platform shell lane does not enable
`macos-native`. The deterministic core tests remain the fake-backend evidence
for replay, stale revisions, device loss, recovery, paths, and accessibility
state. Source checks and a production-CSP smoke for a `macos-native` binary do
not replace protected capture hardware, assistive-technology, signing,
notarization, or clean-machine evidence.

The release handoff copies `web-dist` next to `frame-web`, and Render copies the
same verified directory next to `target/release/frame-web`. Production and
preview trust only that executable-adjacent package; only local development may
use `FRAME_WEB_ASSET_DIR` or fall back to `apps/web/dist` in the checkout. This
prevents a writable working directory or deployment environment override from
becoming a same-origin script authority and makes the executable independent of
its cwd. Nonlocal readiness is degraded until the complete package verifies.
Public HTML is `no-store` in this phase because it names one exact hashed asset
closure and a release retains only that closure. Public-document caching can be
restored only with atomic CDN purge/versioning or retention beyond every HTML
cache TTL; immutable content-addressed assets themselves remain cacheable for a
year.

## Consequences

The public web can evolve independently of the desktop WebView transport, and
the desktop cannot claim a backend transition from local UI state alone. A
future Linux target, capture adapter, or broader command surface requires a
new reviewed dependency/security decision and explicit capability changes.
