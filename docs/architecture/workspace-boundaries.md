# Workspace and runtime boundaries

Frame uses one Rust workspace but not one runtime. Dependency direction is a
security and deployability contract.

```text
frame-client ───────────────────────────────┐
domain ──> ports ──> application/adapters  │
  │             ├─> control-plane (Wasm)   │
  │             ├─> web (native SSR)       │
  │             └─> media (native only)    │
  └────────────────────────────────────────┘
```

## Allowed responsibilities

| Package | Runtime | May depend on | Must not depend on |
|---|---|---|---|
| `frame-domain` | native + wasm32 | Serde, validation primitives | HTTP, databases, Worker, Axum, Leptos, GStreamer, OS APIs |
| `frame-ports` | native + wasm32 | domain, async traits | concrete D1/R2/provider/GStreamer values in public signatures |
| `frame-client` | native + wasm32 core | Serde, URL parsing; optional native HTTP transport | domain internals, object keys, Axum, Leptos, Worker, GStreamer |
| `frame-control-plane` | Cloudflare Worker/Wasm | domain/contracts/ports, Worker bindings | GStreamer, GLib, filesystem persistence, native capture, broad service credentials |
| `frame-web` | native Render process | public client/contracts, Axum, Leptos SSR | Worker SDK, D1/R2 bindings, GStreamer, capture, durable local state |
| `frame-media` / worker | native | domain/contracts/ports, GStreamer | production D1 credentials, Worker bindings, browser session secrets |
| desktop shell | native + browser WebView | versioned IPC and media application services | broad filesystem/shell APIs or unversioned command payloads |

Adapters convert runtime-specific values at the boundary. JavaScript streams,
Cloudflare binding handles, SQL rows, GStreamer elements, device handles, and
signed URLs never enter shared DTOs.

## Toolchain and lock policy

- `rust-toolchain.toml` pins the CI/developer toolchain. `rust-version` is the
  minimum supported compiler and may trail the pinned toolchain only while CI
  checks both intentionally.
- `Cargo.lock` is committed and `--locked` is required for production builds.
- Features may add a transport or adapter but may not invert dependency
  direction. Core contract builds remain default-feature-light and wasm-safe.
- Workspace `unsafe_code = "forbid"` applies by default. A platform FFI crate
  that cannot meet it must be isolated, documented, audited, and granted a
  narrow package-level exception rather than weakening the workspace.

## Enforced checks

The CI boundary job checks native and wasm contract builds and inspects
dependency trees for forbidden packages. The Render release build targets
only `frame-web`; the Worker build targets only `frame-control-plane`; native
media tests own GStreamer installation. A failure in any owning lane blocks
the production sentinel.

Dependency, license, vulnerability, and provenance exceptions require an
owner, reason, affected version, compensating control, and expiry. Exceptions
are data reviewed in the pull request, not an ignored tool exit code.
