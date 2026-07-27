# Workspace and runtime boundaries

Frame uses one Rust workspace but not one runtime. Dependency direction is a
security and deployability contract.

```text
frame-client (anonymous) ────────────────────────┐
authenticated-client (first party) ─────────────┤
domain ──> ports ──> application/adapters       │
  │             ├─> control-plane (Wasm)        │
  │             ├─> web (native SSR)            │
  │             ├─> desktop (native + WebView)  │
  │             └─> media (native only)         │
  └─────────────────────────────────────────────┘
```

## Allowed responsibilities

| Package | Runtime | May depend on | Must not depend on |
|---|---|---|---|
| `frame-domain` | native + wasm32 | Serde, validation primitives | HTTP, databases, Worker, Axum, Leptos, GStreamer, OS APIs |
| `frame-ports` | native + wasm32 | domain, async traits | concrete D1/R2/provider/GStreamer values in public signatures |
| `frame-client` | native + wasm32 core | Serde, URL parsing; optional native HTTP transport | domain internals, object keys, Axum, Leptos, Worker, GStreamer |
| `frame-authenticated-client` | native + wasm32 | first-party authenticated request/receipt identities, Serde, hashing | credentials, anonymous public DTOs, object keys, provider/runtime types, HTTP clients |
| `frame-control-plane` | Cloudflare Worker/Wasm | domain/contracts/ports, Worker bindings | GStreamer, GLib, filesystem persistence, native capture, broad service credentials |
| `frame-web` | native Render process | public client/contracts, Axum, Leptos SSR | Worker SDK, D1/R2 bindings, GStreamer, capture, durable local state |
| `frame-media` / worker | native | domain/contracts/ports, GStreamer | production D1 credentials, Worker bindings, browser session secrets |
| `frame-windows-secure-spool` | Windows native FFI | narrowly featured `windows-sys`, `zeroize`, path/metadata primitives | media/domain/application contracts, networking, raw pointers or handles in its public API |
| `frame-windows-capture-ffi` | Windows native FFI | narrowly featured `windows`, WGC | media/domain/application contracts, networking, raw pointers or handles in its public API |
| `frame-platform-lifecycle` | macOS + Windows native FFI | narrowly featured AppKit/Foundation and Windows power notifications | media/domain/application contracts, networking, native observer or callback identities in its public API |
| `frame-macos-av-capture` | macOS native adapter | media capture contracts, platform-lifecycle cursor, safe published ScreenCaptureKit wrappers | desktop IPC, networking, raw Apple handles/types in its public API |
| desktop shell | native + browser WebView | versioned IPC and media application services | broad filesystem/shell APIs or unversioned command payloads |

Adapters convert runtime-specific values at the boundary. JavaScript streams,
Cloudflare binding handles, SQL rows, GStreamer elements, device handles, and
signed URLs never enter shared DTOs.

The two client crates are intentionally disjoint. `frame-client` is the
anonymous external/public boundary. `frame-authenticated-client` is a
first-party wire-contract package used by the desktop and control plane; it
contains no transport or credential storage and is not a portfolio contract.

## Toolchain and lock policy

- `rust-toolchain.toml` pins the CI/developer toolchain. `rust-version` is the
  minimum supported compiler and may trail the pinned toolchain only while CI
  checks both intentionally.
- `Cargo.lock` is committed and `--locked` is required for production builds.
- Features may add a transport or adapter but may not invert dependency
  direction. Core contract builds remain default-feature-light and wasm-safe.
- Workspace `unsafe_code = "forbid"` applies by default. Three narrow
  package-level exceptions own audited platform boundaries:
  `frame-windows-secure-spool` owns Win32 Credential Manager, ACL/SID,
  reparse-point, handle-allocation, handle-relative no-replace rename, and
  file-flush calls; `frame-windows-capture-ffi` owns Win32 display/window
  enumeration, cursor sampling, WGC item construction, and message-loop
  control; and `frame-platform-lifecycle` owns process-lifetime macOS
  notification observers and the Windows suspend/resume callback. All expose
  safe APIs without raw pointers, handles, observer identities, or callback
  contexts. No other crate may add an unsafe block or depend directly on
  `windows-sys` for the secure-spool boundary.

## Enforced checks

The CI boundary job checks native and wasm contract builds and inspects
dependency trees for forbidden packages. The Render release build targets
only `frame-web`; the Worker build targets only `frame-control-plane`; native
media tests own GStreamer installation. A failure in any owning lane blocks
the production sentinel.

Dependency, license, vulnerability, and provenance exceptions require an
owner, reason, affected version, compensating control, and expiry. Exceptions
are data reviewed in the pull request, not an ignored tool exit code.
