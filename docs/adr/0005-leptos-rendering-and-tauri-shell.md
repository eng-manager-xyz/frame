# ADR 0005: Leptos rendering and Tauri desktop shell

Status: accepted

## Context

Frame needs useful HTML and metadata before JavaScript on the public web, while
the desktop product runs inside a native WebView and must obtain all privileged
state from an allowlisted Rust command boundary. Tauri serves static frontend
assets and does not provide a server runtime inside the desktop application.

## Decision

- `frame-web` uses Leptos SSR behind Axum. Its Cargo dependency enables only
  Leptos' `ssr` feature and remains outside browser Wasm.
- `frame-desktop-ui` uses Leptos CSR compiled to Wasm by Trunk. Tauri 2 embeds
  the fingerprinted `ui/dist` output; no HTTP server ships in the desktop app.
- The desktop and web targets select their Leptos features in their own
  manifests. The workspace dependency intentionally selects neither rendering
  mode, preventing accidental server code in the desktop Wasm graph.
- The desktop frontend deserializes the native bootstrap result into the same
  `ShellCapabilities` Rust type returned by the Tauri command. A protocol and
  backend-truth check must pass before the UI enables native work.
- Tauri's global bridge is enabled because that is the supported Tauri/Leptos
  integration. The only registered bootstrap command re-checks the exact
  `main` window label. The build manifest and capability file allow only that
  command and grant no filesystem, shell,
  process, network, or dialog permission, and the CSP rejects remote scripts.
- Desktop runtime builds currently support macOS and Windows. Linux remains
  disabled until the time-bounded GLib advisory record in
  `docs/security/dependency-policy.md` can be removed.
- The shell reports `not_selected` for capture until a real platform adapter is
  integrated. UI controls remain disabled instead of simulating success.

## Commands and evidence

From the repository root:

```sh
cargo test --locked -p frame-desktop-core
cargo test --locked -p frame-desktop-core --features tauri-app --bin frame-desktop
cargo clippy --locked -p frame-desktop-ui --no-default-features --features csr --target wasm32-unknown-unknown -- -D warnings
python3 scripts/ci/build-desktop-ui.py
python3 scripts/ci/check-desktop-bundle.py
cargo build --locked --release -p frame-desktop-core --features tauri-app,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py
```

`quality-gates.yml` repeats the command boundary, Wasm bundle, and native
release build on both supported operating systems and retains the exact binary
and frontend bundle. The deterministic core tests remain the fake-backend
evidence for replay, stale revisions, device loss, recovery, paths, and
accessibility state.

## Consequences

The public web can evolve independently of the desktop WebView transport, and
the desktop cannot claim a backend transition from local UI state alone. A
future Linux target, capture adapter, or broader command surface requires a
new reviewed dependency/security decision and explicit capability changes.
