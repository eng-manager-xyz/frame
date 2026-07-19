# Frame repository Leptos profile

This reference is intentionally repository-specific. It records the accepted
architecture observed on 2026-07-19. Re-read the manifests, lockfile, ADRs, and
current code before every task; update this profile when those authorities change.

## Contents

- [Dependency contract](#dependency-contract)
- [Web architecture](#web-architecture)
- [Desktop architecture](#desktop-architecture)
- [Do not introduce implicitly](#do-not-introduce-implicitly)
- [Current patterns that are legitimate](#current-patterns-that-are-legitimate)
- [Validation commands](#validation-commands)
- [Profile maintenance](#profile-maintenance)

## Dependency contract

The workspace pins:

```toml
leptos = { version = "0.8.20", default-features = false }
```

Rendering features are selected in target crates, not at workspace level:

- `apps/web`, native target: `leptos` with `ssr`.
- `apps/web`, `wasm32` target: `leptos` with `hydrate`.
- `apps/desktop/ui`: optional `leptos` with `csr` behind the crate's `csr`
  feature.

Do not enable a rendering mode on the workspace dependency. Keep native and Wasm
dependency tables separate. The lockfile currently resolves `server_fn` only as
a transitive Leptos dependency; that does not authorize application server
functions.

## Web architecture

Accepted ADR 0005 makes Axum authoritative for:

- HTTP routes and metadata;
- privacy, session, role, loading, and error state;
- server-rendered public HTML;
- verified, content-addressed hydration assets.

Leptos supplies synchronous server-rendered view fragments and deliberately
scoped progressive hydration/mounting. The separate `frame-web-hydrate` Wasm
binary does not hydrate the full document. Full-body hydration is prohibited
until the repository adopts a typed same-origin bootstrap for server-authorized
state.

Preserve the individual boundary decision in `apps/web/src/bin/hydrate.rs`:
some roots use `hydrate_from` because matching HTML is server-rendered; other
roots use `mount_to` because they are client-mounted. Do not normalize all roots
to one mounting function without an architecture change and end-to-end evidence.

Axum owns the server boundary directly. There is currently no direct application
dependency on `leptos_router`, `leptos_axum`, `leptos_actix`, or `server_fn`, and
no generated Leptos route list. A Leptos full-stack template is therefore not an
appropriate model for this crate.

## Desktop architecture

`frame-desktop-ui` is a CSR Wasm application mounted with
`leptos::mount::mount_to_body`. Tauri serves static assets and exposes a narrow
allowlisted Rust command boundary; there is no Leptos server runtime in the
desktop application.

Keep native/Tauri authority separate from local reactive state. A UI signal
cannot claim a privileged backend transition until the typed command boundary
confirms it. Apply ADR 0005 and the repository security/dependency policy before
adding a browser, network, filesystem, or Tauri capability.

## Do not introduce implicitly

Do not add any of the following as part of a routine component or API migration:

- full-body web hydration;
- a Leptos router;
- Leptos server functions;
- `leptos_axum` route ownership;
- islands-router;
- a shared workspace rendering feature;
- desktop HTTP/SSR runtime;
- broader Tauri commands or browser capabilities.

Each is a separate architectural/dependency decision governed by
`pragmatic-tiger`, ADRs, and explicit task scope.

## Current patterns that are legitimate

- `view.to_html()` in `apps/web/src/pages.rs` and hydration-focused tests is the
  current synchronous rendering approach. Do not replace it simply because a
  full-stack example streams HTML. Reassess if a view gains `Resource`,
  `<Suspense>`, or another async SSR boundary.
- `RwSignal`, `Effect::new`, and `<Show>` are current APIs, not legacy matches.
- `hydrate_from(...).forget()` and `mount_to(...).forget()` intentionally keep
  long-lived roots mounted. Preserve boundary and lifecycle intent.
- Direct Axum handlers are the accepted server integration.
- Hydratable views must remain data-free at their initial boundary and emit exact,
  valid, deterministic matching markup.

## Validation commands

Run the smallest relevant subset from the repository root. Add the legacy audit
for the changed path before these commands.

### Web SSR and shared views

```sh
cargo test --locked -p frame-web
```

### Web hydration Wasm

```sh
cargo clippy --locked -p frame-web --no-default-features --features hydrate --target wasm32-unknown-unknown --bin frame-web-hydrate -- -D warnings
python3 -I scripts/ci/build-web-hydration.py
python3 -I scripts/ci/check-web-hydration-bundle.py
```

When hydration behavior or injected assets change, also run the configured app
and:

```sh
python3 -I scripts/ci/web-hydration-smoke.py --origin http://127.0.0.1:3000
```

### Desktop CSR Wasm

```sh
cargo clippy --locked -p frame-desktop-ui --no-default-features --features csr --target wasm32-unknown-unknown -- -D warnings
python3 scripts/ci/build-desktop-ui.py
python3 scripts/ci/check-desktop-bundle.py
```

Do not run commands blindly. A shared component change may require web SSR, web
hydration, and desktop CSR; a desktop-only view does not require the web bundle.
Follow `rust-modern` for the repository's broader formatting/lint/test protocol.

## Profile maintenance

Re-check this file when any of these change:

- root or target `Cargo.toml` Leptos versions/features;
- `Cargo.lock` companion versions;
- ADR 0005 status or replacement ADRs;
- hydration entry points and root IDs;
- Axum route ownership;
- desktop command/capability policy;
- CI command names.

Do not copy Frame-specific crate names, commands, or architectural prohibitions
into a generic Leptos project. Derive a fresh profile there.
