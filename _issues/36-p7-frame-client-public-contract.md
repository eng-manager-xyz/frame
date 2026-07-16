---
title: "Define the EngManager integration contract and build frame-client"
labels:
  - "phase:p7"
  - "area:api"
  - "area:portfolio"
  - "area:rust"
  - "type:integration"
  - "risk:high"
depends_on: [06, 30, 32]
size: epic
---

# 36 · Define the EngManager integration contract and build `frame-client`

## Outcome

Frame exposes a small, versioned, privacy-safe public contract that the
EngManager portfolio can consume without depending on Frame's server, Worker,
Leptos, GStreamer, D1, R2, or internal domain implementation.

## Current reference

The pinned portfolio snapshot is one Rust/Axum workspace member and already
uses pinned Git dependencies. It has no Frame integration, general auth
system, or CORS layer. Its reusable HTTP integration pattern uses Reqwest with
a short timeout and a last-known-good background snapshot instead of handler-
path I/O.

Frame currently returns unrelated health shapes from `apps/web` and
`apps/control-plane`, has no public client crate, and has no versioned `/api`
surface. See [the portfolio reference](../docs/upstream-engmanager.md) at
`matthewharwood/engmanager.xyz@1de52bc8f25793dea3697e67765d53785c05cdfa`.

## Dependencies

[#06](./06-p1-shared-domain-api-contracts.md),
[#30](./30-p5-rust-api-workflow-parity.md), and
[#32](./32-p5-leptos-share-player.md)

## Scope

Add `crates/frame-client` with versioned public DTOs, origin validation, URL
construction, stable structured errors, capability negotiation, fixtures, and
an optional timeout-bound HTTP transport. Define `/api/v1/health` and only the
approved anonymous public-share summaries needed by portfolio/project
surfaces. Document compatibility, deprecation, privacy, and release rules.

### Proposed crate boundary

```text
crates/frame-client/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── origin.rs       # FrameOrigin and same-origin /api/v1 URL builders
    ├── dto.rs          # public request/response types only
    ├── error.rs        # stable codes and redacted client errors
    └── client.rs       # optional reqwest transport
fixtures/frame-api/v1/
├── health.ok.json
├── share.public.json
├── share.unavailable.json
└── error.json
```

The core DTO/origin layer must compile for native and
`wasm32-unknown-unknown` without default transport. A `client` feature may add
Reqwest with Rustls for native consumers. The crate must not depend on Axum,
Leptos, `worker`, GStreamer, Cloudflare SDK types, `frame-media`, repositories,
or storage/object-key internals.

### Public contract

- `FrameOrigin`: a validated absolute origin with no credentials, query,
  fragment, or non-root path. Production constructors require HTTPS; an
  explicit test/local constructor may allow loopback HTTP.
- `ApiVersion`: an explicit major contract version and request/response header
  policy. URLs use the canonical origin plus `/api/v1`, never a second API
  hostname.
- `Health`: public service state, contract version, build/release identifier,
  and coarse capabilities. It must not expose binding IDs, database names,
  bucket names, regions, tenant counts, stack traces, or credentials.
- `PublicShareSummary`: only policy-approved public title/description,
  canonical share/player URLs, safe derivative descriptors, duration, and
  availability. It never contains an owner email, tenant ID, internal object
  key, signed R2 URL, session data, comments, transcript, or private metadata.
- `ApiError`: stable machine code, safe message, request ID, and retry hint;
  internal causes stay server-side.
- Capability/version negotiation that lets an older portfolio ignore additive
  fields and degrade when a newer feature is unavailable.

Serde debug output and error chains must redact bearer tokens, cookies, query
credentials, signed URLs, and response bodies. No public DTO may accidentally
derive a debug representation that leaks a secret-bearing URL.

### Out of scope

- Sharing Frame's authenticated dashboard/session types with the portfolio.
- Making the portfolio a Frame API gateway or identity provider.
- Exposing direct D1, R2, Media Transformations, or GStreamer controls.
- Publishing to crates.io before ownership, support, and SemVer policy are
  approved; a pinned Git revision is sufficient for the first consumer.
- Adding a request-time portfolio dependency on Frame availability.

## Deliverables

- [ ] Accepted public-data classification, endpoint inventory, schema/version
  policy, and N/N-1 compatibility rules.
- [ ] `frame-client` crate with core-only and optional `client` feature sets.
- [ ] `/api/v1/health` and approved public-share endpoints implemented by the
  control-plane Worker under `frame.engmanager.xyz/api/*`.
- [ ] Checked-in canonical JSON fixtures and generated or validated schema
  artifacts consumed by both repositories.
- [ ] Timeout, redirect, response-size, content-type, retry, and error-mapping
  policy for the optional transport.
- [ ] Dependency-policy test and documentation showing the crate's one-way
  relationship: portfolio may depend on Frame; Frame never depends on the
  portfolio repository.
- [ ] Upgrade/deprecation guide for bumping the pinned Frame Git revision in
  `engmanager.xyz`.

## Acceptance criteria

- [ ] `cargo test -p frame-client --all-features` passes, and core types check
  for native and wasm32.
- [ ] `cargo tree -p frame-client` proves no Leptos, Axum, Worker, GStreamer,
  D1/R2 adapter, or media-runtime dependency enters the client boundary.
- [ ] Origin parsing rejects credentials, fragments, query strings, path
  confusion, non-HTTPS production origins, Unicode/port ambiguity, and
  protocol-relative input; loopback test origins require an explicit API.
- [ ] Unknown additive fields and capabilities do not break the pinned
  portfolio client; incompatible major versions fail closed with a useful,
  redacted error.
- [ ] Public/private/deleted/processing/failed fixtures prove that private
  titles, thumbnails, existence, signed URLs, object keys, and owner/tenant
  identifiers never cross the public contract.
- [ ] Redirects cannot leave the configured Frame origin unless a call
  explicitly opts into a reviewed public media origin.
- [ ] Every network operation has a deadline and maximum response size;
  retries are limited to idempotent operations and honor cancellation.
- [ ] A deliberate secret fixture is absent from `Debug`, display, tracing,
  and snapshot output.
- [ ] The portfolio can compile against an exact Frame Git SHA and its root
  `Cargo.lock` records that revision reproducibly.

## Required test evidence

- Native/wasm feature matrix and forbidden-dependency report.
- Consumer fixture tests from both Frame and the pinned portfolio checkout.
- Fuzz/property tests for origin and URL construction.
- Public-data/privacy review with malicious and forward-versioned fixtures.

## Risks and open questions

- A generic client crate can become a dumping ground for internal types; only
  reviewed public wire contracts belong here.
- Git dependencies are reproducible only when pinned and locked.
- A health endpoint that exposes provider detail helps attackers more than
  portfolio users; operational diagnostics need a separately protected path.
- Public-share fields must follow issue 32's privacy policy, not convenience.

## Rollout and rollback

Ship types and fixtures before the portfolio consumes them. Add endpoints
behind a contract-version gate, prove N/N-1 behavior, then pin the accepted
Frame commit in the portfolio. Roll back consumers to the preceding SHA while
keeping the corresponding API major available through its deprecation window.

Before closing, attach the schema review, dependency report, fixture evidence,
consumer build, and any API/ADR updates produced by this issue.
