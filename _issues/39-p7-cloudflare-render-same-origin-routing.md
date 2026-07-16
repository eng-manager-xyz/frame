---
title: "Route frame.engmanager.xyz between Render and the Cloudflare Worker"
labels:
  - "phase:p7"
  - "area:cloudflare"
  - "area:render"
  - "area:api"
  - "type:deployment"
  - "risk:critical"
depends_on: [07, 21, 30, 38]
size: epic
---

# 39 · Route `frame.engmanager.xyz` between Render and the Cloudflare Worker

## Outcome

`https://frame.engmanager.xyz` is one secure public origin: Cloudflare routes
`/api` and `/api/*` to Frame's Rust/Wasm control plane and forwards every other
path to the dedicated Render Leptos service, with no portfolio-route collision
or origin bypass.

## Current reference

Frame's Worker has `/` and `/health` routes but no production hostname or
`/api/v1` prefix. `frame-web` has no Render configuration. The portfolio zone
already proxies apex, `www`, and shop to its existing Render service, and its
bootstrap script assumes those hosts share one `RENDER_HOSTNAME`. That
assumption must not be reused for Frame.

[ADR 0004](../docs/adr/0004-engmanager-render-cloudflare-topology.md)
accepts a dedicated Render service plus Cloudflare Worker Routes on one
first-level subdomain. A Worker Custom Domain is not appropriate because it
would make the Worker the origin for every path and replace Render.

## Dependencies

[#07](./07-p1-control-plane-media-job-protocol.md),
[#21](./21-p3-storage-security-lifecycle.md),
[#30](./30-p5-rust-api-workflow-parity.md), and
[#38](./38-p7-render-web-runtime-blueprint.md)

## Scope

Provision the Render custom domain and staged Cloudflare CNAME through the
portfolio repository's designated zone-infrastructure state, add a query-safe
broad Worker Route, move
the public API under `/api/v1`, validate the first path segment plus host and
forwarded metadata, prove lookalike/unmatched-path origin behavior, and
document certificate, CAA, default-hostname, monitoring, and rollback behavior.

### Request routing contract

| Request | Owner | Cache default |
|---|---|---|
| `/api` | Worker (redirect/error/version discovery only) | bypass |
| `/api/*` | Worker control plane | bypass |
| `/health/live`, `/health/ready` | Render web process | bypass |
| hashed assets | Render origin through Cloudflare | immutable public |
| landing/dashboard/share/embed HTML | Render origin through Cloudflare | route/privacy specific |
| unknown non-API path | Render 404/fallback | bypass unless explicitly public |

Cloudflare matches the entire URL, including the query string. A route ending
at `/api` would miss `/api?x=1`, so use one broad interception pattern:

```toml
[[routes]]
pattern = "frame.engmanager.xyz/api*"
zone_name = "engmanager.xyz"
```

The Worker must inspect the URL pathname before normal router decoding. It owns
only pathname `/api` or a pathname beginning `/api/`. A lookalike such as
`/apix` is explicitly proxied to the Render origin or receives a reviewed
non-cacheable 404; it can never enter an API handler. The initial route may
invoke `frame-control-plane` directly after its router adopts the prefix. A
separate edge gateway Worker is justified only if it owns a concrete policy
such as version routing or service-binding isolation. If introduced, it must
stream request/response bodies, preserve method/status and safe headers,
normalize one request ID, reject unexpected hosts, and bind to a control-plane
Worker deployed first.

### DNS and certificate sequence

1. Add `domains: [frame.engmanager.xyz]` to issue 38's validated Blueprint and
   apply the controlled Render sync.
2. Through `engmanager.xyz/infra/cloudflare-zone`, the single designated
   zone-infrastructure state, create
   `CNAME frame -> <frame-service>.onrender.com` with proxy disabled. Do not
   create it manually and import it later.
3. Remove only a conflicting `AAAA` at `frame`; do not touch apex/shop records.
4. Verify the domain and wait for Render's public certificate.
5. Prove HTTPS while DNS-only, then enable the Cloudflare proxy.
6. Require Full (strict), verify edge and origin certificates, then add the
   broad Worker Route and strict segment-boundary behavior.
7. After observation, disable the default Render hostname and prove the custom
   host still works.

Do not create a wildcard. Do not install a Cloudflare Origin CA certificate on
Render. If zone CAA records exist, verify that Render's current certificate
authorities remain allowed before changing anything.

### Out of scope

- Serving Frame from the existing portfolio process.
- A separate `api.frame.engmanager.xyz` unless this ADR is superseded with a
  certificate/CORS/migration plan.
- Sending large uploads or media downloads through the Render process.
- Managing DNS/cache/WAF resources from Wrangler. This issue establishes the
  designated zone-infrastructure owner and exact CNAME; issue 41 migrates each
  shared phase entrypoint into that same owner before adding Frame rules.
- Globally changing the zone's apex SSL/DNS behavior solely for Frame.

## Deliverables

- [ ] Production/staging hostname and route inventory with one owner per DNS,
  Worker route, Render domain, certificate, and shared zone phase; the exact
  CNAME is created by that owner with no later manual-to-IaC handoff.
- [ ] `/api/v1` control-plane routes, stable health contract, host validation,
  request IDs, proxy metadata policy, and safe error mapping.
- [ ] Render custom-domain record and staged DNS-only-to-proxied runbook.
- [ ] Wrangler environment configuration for the broad `/api*` route, strict
  first-segment pass-through/404 policy, and `workers.dev`/legacy exposure
  disposition.
- [ ] Route-conformance suite covering path encoding, slashes, query strings,
  methods, bodies, ranges, streaming, redirects, upgrades, and errors.
- [ ] Origin-bypass, CAA/TLS-renewal, cache, and rollback runbooks.

## Acceptance criteria

- [ ] `frame.engmanager.xyz/` and all non-API test paths reach the Frame Render
  service, never the portfolio service or Worker fallback.
- [ ] `/api`, `/api?query`, and `/api/*`, including encoded, repeated-slash,
  semicolon, dot-segment, and trailing-slash variants, reach the intended
  Worker exactly once and never fall through to Render.
- [ ] `/apix`, `/apiary`, and encoded segment-lookalikes never enter an API
  handler; they follow the documented Render pass-through or no-store 404 path.
- [ ] Worker routing preserves safe methods, query strings, streaming bodies,
  response status, range semantics, content type, and request ID without
  forwarding spoofable Cloudflare/internal headers from a client.
- [ ] Unexpected Host values and direct default-host attempts fail according
  to policy; canonical redirects cannot be used for open redirect or host
  header poisoning.
- [ ] DNS-only certificate issuance, proxied Full (strict), HTTP-to-HTTPS, and
  renewal eligibility pass; no conflicting `AAAA` or wildcard affects other
  subdomains.
- [ ] API/auth/health responses are never a Cloudflare cache HIT, while an
  immutable hashed asset becomes a HIT under issue 41's rules.
- [ ] Disabling the Worker Route leaves non-API Render traffic intact, and
  toggling the CNAME back to DNS-only is a tested edge rollback.
- [ ] Disabling the Render default hostname prevents a simple WAF/cache bypass
  without breaking Cloudflare-to-Render traffic.
- [ ] Existing `engmanager.xyz`, `www`, shop/store, Stripe webhook, and
  portfolio cache behavior are unchanged.

## Required test evidence

- DNS history, Render verification, edge/origin certificate, and Full (strict)
  traces.
- Exhaustive route-owner matrix and negative host/path cases.
- Cloudflare/Render request IDs proving a single intended hop.
- Worker-route removal, DNS-only, and Render-hostname rollback rehearsal.

## Risks and open questions

- A pattern without a trailing wildcard misses query strings; a broad wildcard
  also intercepts `/apix`, so raw first-segment enforcement is mandatory.
- URL normalization differences between Cloudflare, Axum, and the Worker can
  produce auth or cache bypasses; test raw and decoded forms.
- Cloudflare proxy activation before Render certificate issuance can block
  validation or hide the failing origin.
- Disabling `onrender.com` too early removes a useful diagnostic path.

## Rollout and rollback

Bring up Render on its default hostname, attach the custom domain DNS-only,
validate TLS, enable proxying, then enable the Worker Route in staging and
production. Each layer has an independent rollback: remove the Worker Route,
switch DNS to DNS-only, re-enable the Render subdomain, or restore the prior
Render/Worker release. Never change multiple rollback layers without recording
which one is authoritative.

Before closing, attach DNS/TLS evidence, route traces, default-origin test,
cache results, and the rehearsed rollback timeline.
