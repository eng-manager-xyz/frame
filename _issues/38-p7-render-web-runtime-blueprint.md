---
title: "Package the Leptos web service for Render and codify its Blueprint"
labels:
  - "phase:p7"
  - "area:render"
  - "area:leptos"
  - "area:ops"
  - "type:deployment"
  - "risk:high"
depends_on: [08, 09, 30, 34]
size: epic
---

# 38 · Package the Leptos web service for Render and codify its Blueprint

## Outcome

Frame's Leptos/Axum service is a stateless, reproducible Render web service
that binds correctly, reports readiness, drains safely, scales horizontally,
and remains isolated from GStreamer and Cloudflare-only runtime dependencies.

## Current reference

`apps/web` currently reads only `FRAME_ADDR`, defaults to
`127.0.0.1:3000`, exposes one unconditional `/health` response, and does not
install graceful shutdown. That will not honor Render's injected `PORT` unless
manually configured. The repository has no `render.yaml` or Render packaging.

The pinned portfolio shows a working Axum pattern: bind `0.0.0.0:$PORT` in
production and handle `SIGTERM`. Render requires a public service to listen on
`0.0.0.0`, recommends `PORT`, gates deploys with health checks, and sends
`SIGTERM` before its configurable shutdown deadline.

## Dependencies

[#08](./08-p1-leptos-web-desktop-shells.md),
[#09](./09-p1-ci-quality-gates.md),
[#30](./30-p5-rust-api-workflow-parity.md), and
[#34](./34-p6-operational-hardening.md)

## Scope

Implement Render-specific composition-root behavior in `frame-web`, add a
schema-valid root `render.yaml`, choose and prove the build/runtime packaging,
define health and shutdown contracts, configure previews and custom domain,
and document scaling, region, cost, observability, rollback, and ephemeral
filesystem rules.

The default packaging is Render's native Rust runtime with a targeted
`cargo build --locked --release -p frame-web`, because the web crate must not
load GStreamer. If evidence requires Docker, record the reason and retain the
same targeted dependency boundary. A server-side native media executor is a
separate Docker private/background service; it is never added to the web
process merely to simplify deployment.

### Blueprint baseline

The reviewed initial Blueprint should encode, without committing secrets:

- one dedicated `frame-web` web service; issue 39 adds the production custom
  domain in a controlled Blueprint sync after the default-host service passes;
- a paid production plan decision and measured region decision;
- `autoDeployTrigger: checksPass` as the sole Render deploy authority;
- targeted build/start commands or a pinned Dockerfile;
- `healthCheckPath: /health/ready`;
- a bounded `maxShutdownDelaySeconds` backed by a drain test;
- manual, short-lived previews with non-production endpoints;
- a `buildFilter` covering `apps/web`, public/shared crates, lock/toolchain
  files, and Render packaging;
- non-secret environment values and `sync: false` placeholders for secrets;
- the default Render subdomain enabled during bring-up and disabled only after
  custom-domain/proxy verification.

Use this as the concrete starting shape; the plan and region remain explicit
review decisions before the first production resource is created:

```yaml
# yaml-language-server: $schema=https://render.com/schema/render.yaml.json
previews:
  generation: manual
  expireAfterDays: 3

services:
  - type: web
    name: frame-web
    runtime: rust
    plan: starter
    region: oregon
    buildCommand: cargo build --locked --release -p frame-web
    startCommand: ./target/release/frame-web
    healthCheckPath: /health/ready
    maxShutdownDelaySeconds: 60
    autoDeployTrigger: checksPass
    renderSubdomainPolicy: enabled
    buildFilter:
      paths:
        - apps/web/**
        - crates/**
        - Cargo.toml
        - Cargo.lock
        - rust-toolchain.toml
        - render.yaml
    envVars:
      - key: FRAME_DEPLOYMENT
        value: production
        previewValue: preview
      - key: FRAME_PUBLIC_ORIGIN
        value: https://frame.engmanager.xyz
        previewValue: https://frame-preview.invalid
      - key: FRAME_API_ORIGIN
        value: https://frame.engmanager.xyz
        previewValue: https://frame-staging.engmanager.xyz
      - key: RUST_LOG
        value: info
```

If a clean Render Blueprint validation proves that a field's current schema
differs, update the example and cite the pinned schema/CLI version rather than
silently configuring the service only in the dashboard.

For `IS_PULL_REQUEST=true`, the runtime must require
`FRAME_DEPLOYMENT=preview`, ignore the fail-closed public-origin sentinel, and
derive its validated preview origin from Render's `RENDER_EXTERNAL_URL`. It
must never emit the production canonical URL or production-scoped cookies from
a preview. `FRAME_API_ORIGIN` points only to a non-production API; authenticated
preview access additionally needs issue 43's explicit origin pairing and
staging credentials. Every selector gets a `previewValue`, and `sync: false`
secrets are not copied to previews. Render Preview Environments require an
eligible Pro-or-higher workspace plan; cost/entitlement is a readiness gate,
not an assumption.

Do not set a production `branch` in a way that makes preview services build
the production branch. Do not use a persistent disk for source videos,
derivatives, sessions, job state, or manifests.

### Runtime contract

- Precedence: explicit `FRAME_ADDR` for local/test override; otherwise
  `PORT` means `0.0.0.0:$PORT`; otherwise local `127.0.0.1:3000`.
- `/health/live` proves the process/event loop is alive without network calls.
- `/health/ready` proves validated configuration, router/assets, and required
  local initialization. External Cloudflare dependency checks are bounded and
  reported separately so an upstream outage does not restart every web
  instance.
- SSR performs only fixed-destination, bounded anonymous reads for approved
  public metadata. Authenticated/private pages render a generic safe shell and
  hydrate through the browser's same-origin `/api`; Render receives no D1/R2
  administrator or broad service credential. Any future authenticated SSR
  requires the ADR/security gate in ADR 0004.
- Axum graceful shutdown listens for `SIGTERM` and Ctrl-C, stops accepting new
  requests, drains within the Blueprint budget, and records only safe release
  metadata.
- Local filesystem use is bounded, temporary, and disposable. Durable state is
  D1/R2; large upload bodies do not transit Render.

### Out of scope

- Moving D1, R2, or Media Transformations bindings onto Render.
- Running desktop capture or hardware codecs in the public web service.
- Attaching a Render disk as a substitute for R2.
- Choosing a free/sleeping instance for production.
- Deploying before issue 40 defines the release authority and gates.

## Deliverables

- [ ] Typed/tested bind, production/preview public and API origin,
  proxy-trust, health, and shutdown configuration at the web composition root.
- [ ] Separate liveness, readiness, and protected dependency-diagnostic
  handlers with stable response contracts.
- [ ] Schema-valid `render.yaml`, build filtering, preview policy, environment
  inventory, and no secret values.
- [ ] Native-runtime versus Docker evidence, including a forbidden GStreamer /
  Worker dependency check for `frame-web`.
- [ ] Region/plan/capacity/startup/build-time decision with a monthly budget.
- [ ] Render custom-domain, logs/metrics, scaling, zero-downtime, preview,
  rollback, and emergency-restart runbook.
- [ ] Local production-mode and Render preview smoke tests.

## Acceptance criteria

- [ ] `PORT=10000 ./target/release/frame-web` binds only
  `0.0.0.0:10000`; invalid/missing production configuration fails before
  readiness with a redacted error.
- [ ] The release build compiles only `frame-web` and its allowed dependency
  graph; no GStreamer/native media or Worker/Wasm SDK is present in the binary.
- [ ] Blueprint validation passes with the pinned Render CLI/schema, and a
  clean Render build uses the committed lockfile/toolchain.
- [ ] Readiness blocks a broken new release but does not flap because of one
  transient D1/R2/Media/network timeout.
- [ ] A `SIGTERM` test with in-flight HTTP completes or cancels requests inside
  the configured deadline and exits cleanly; no durable work is lost or
  duplicated.
- [ ] Restart, scale-out, and redeploy tests prove no durable state depends on
  the instance filesystem and identical requests can hit different instances.
- [ ] Preview services use staging/non-production APIs and cannot mutate
  production D1/R2, emit the production canonical, or issue production-scoped
  cookies; previews expire and are `noindex`.
- [ ] Public SSR calls only fixed `/api/v1` read endpoints with deadline,
  body/redirect/header limits and a circuit breaker; authenticated/private
  state is absent from HTML and Worker failure yields a generic noindex shell.
- [ ] A failed build/start/readiness deploy leaves the preceding Render release
  serving, and an operator can roll back to a named commit.
- [ ] The default `onrender.com` hostname is disabled only after Cloudflare and
  custom-domain tests pass, with a documented re-enable rollback.

## Required test evidence

- Blueprint validation, clean Render build, startup timing, and dependency
  tree/SBOM.
- Port/host/config table and liveness/readiness fault matrix.
- SIGTERM/in-flight drain and zero-downtime deployment trace.
- Preview isolation plus restart/scale statelessness evidence.

## Risks and open questions

- Native Rust is simpler for the web crate, but an accidental media dependency
  can introduce unavailable system libraries; enforce the boundary in CI.
- Render region is immutable for a service; benchmark before provisioning the
  production service.
- WebSocket/SSE connections still disconnect when an old instance exits;
  clients need reconnect/resume behavior.
- Render health checks may use the verified custom-domain Host header; host
  validation must admit the canonical host without admitting arbitrary hosts.

## Rollout and rollback

Prove local production mode, then a manual preview, then a staging service.
Attach the custom domain only in issue 39. Keep the prior Render release and
default hostname available during the observation window; roll back the
application independently from Cloudflare DNS and Worker routes.

Before closing, attach Blueprint validation, preview URL/evidence, dependency
report, shutdown trace, cost/region decision, and runbooks.
