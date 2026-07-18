# Same-origin routing, TLS, and rollback

This runbook owns the Issue 39 boundary for `frame.engmanager.xyz`. It does not
authorize a DNS, Render, or Cloudflare mutation by itself. Production changes
must use the protected owners recorded in
`fixtures/same-origin-routing/v1/ownership-inventory.json`.

## Fixed ownership

| Layer | Production value | Sole owner |
|---|---|---|
| Render service and domain declaration | `frame-web`, `frame.engmanager.xyz` | Frame `render.yaml` through controlled Render Git sync |
| Worker script and route declaration | `frame-control-plane`, `frame.engmanager.xyz/api*` plus query-safe `frame.engmanager.xyz/media-server*` compatibility fence | Frame protected Wrangler deploy |
| DNS record | exact `CNAME frame -> <frame-web>.onrender.com` | `eng-manager-xyz/engmanager.xyz/infra/cloudflare-zone` |
| Edge certificate, Full (strict), shared rulesets | exact Frame host entries in whole-phase state | `eng-manager-xyz/engmanager.xyz/infra/cloudflare-zone` |
| Render origin certificate | Render-managed certificate for the exact host | Render |

The Frame repository is a read-only consumer of shared-zone state. It must not
create a second DNS or ruleset state, add a wildcard, or hand a manually
created record back to infrastructure-as-code later. Staging is a separate
protected service and route at `frame-staging.engmanager.xyz`; the repository
inventory deliberately marks it pending rather than treating a Render preview
URL as staging infrastructure.

## Route and transport policy

Cloudflare's route pattern evaluates the complete URL, including the query.
The trailing wildcard is therefore mandatory:

```toml
[[routes]]
pattern = "frame.engmanager.xyz/api*"
zone_name = "engmanager.xyz"

[[routes]]
pattern = "frame.engmanager.xyz/media-server*"
zone_name = "engmanager.xyz"
```

The broad API pattern also catches `/apix` and `/apiary`. The narrow
compatibility prefix catches query strings on `/media-server`, as Cloudflare
cannot express an exact path plus arbitrary query parameters without a
trailing wildcard; it therefore also catches suffix lookalikes. The control
plane parses the request target before application dispatch and owns only the
exact raw pathname `/api`, a pathname beginning `/api/`, exact
`GET /media-server`, or these 16 source-pinned, method-bound child shapes:

- `POST /media-server/audio/check`, `POST /media-server/audio/convert`, and
  `POST /media-server/audio/extract`;
- `GET /media-server/audio/status` and `GET /media-server/health`;
- `POST /media-server/video/cleanup`, `POST /media-server/video/convert`,
  `POST /media-server/video/edit`, `POST /media-server/video/force-cleanup`,
  `POST /media-server/video/mux-segments`, `POST /media-server/video/probe`,
  `POST /media-server/video/process`, and
  `POST /media-server/video/thumbnail`;
- `GET /media-server/video/status`;
- `POST /media-server/video/process/:jobId/cancel` and
  `GET /media-server/video/process/:jobId/status`.

The matrix exercises the dynamic shapes as
`POST /media-server/video/process/job-42/cancel` and
`GET /media-server/video/process/job-42/status`. The exact root preserves its
pinned metadata body with a query. All 16 children are Worker-owned but remain
`fail_closed_unavailable` behind `hardware_execution` and
`provider_execution`; an unauthenticated or unconfigured trace must close with
`401`, `403`, or `503`, never success or redirect. This ownership update is not
provider promotion.

`/media-server/`, `/media-server/health/`, unknown children,
`/media-server/video/process-extra`, an empty `:jobId`, and
`/media-server/Health` receive the reviewed `404 not_api_route` response with
`Cache-Control: no-store`, as do API lookalikes. Exact `/Media-server` remains
case-sensitive, misses the Worker Route, and continues to Render. These paths
are never forwarded by an application gateway. Unknown or malformed paths
inside the API boundary also fail closed. Every other non-API path misses both
Worker Routes and continues to the Render CNAME.

The initial design has no edge gateway. The Worker dispatches the original
`Request`, so its method, query, body stream, and safe headers are not rebuilt.
Fixed-length uploads use the incoming stream, and R2 playback returns the R2
body stream with verified single-range `206`/`416` semantics. API routes do
not support protocol upgrade and must never return `101`. Resource `Location`
headers are not redirects; canonical redirects and user-selected redirect
hosts are forbidden.

Production accepts HTTPS and the exact public Host only. `workers.dev` is
disabled. The Worker derives one opaque request ID from a syntactically valid
Cloudflare Ray value or an internal fallback; it never trusts client
`X-Request-ID`, `Forwarded`, `X-Forwarded-*`, `CF-Connecting-IP`, or an
internal routing header as authority. API/auth/health/error responses are
explicitly `no-store` and must report Cloudflare `DYNAMIC` or `BYPASS`.

### URL normalization gate

Cloudflare can normalize encoded characters and dot/slash spellings before a
Worker observes a URL, while `raw.http.request.uri.path` remains available to
the authoritative zone ruleset. Before enabling the route, trace every raw and
normalized case in the checked-in owner matrix against the zone's actual URL
normalization settings. If an encoded prefix such as `/%61pi` becomes `/api`
before application classification, launch is blocked until the zone owner adds
a host-scoped raw-path rejection in the existing whole-phase ruleset (or an
ADR approves a gateway that owns normalization). Never weaken the Worker
boundary and never change the shared zone's normalization mode solely for
Frame. Baseline normalization of repeated slashes must also be recorded rather
than inferred from a local URL parser.

## Staged DNS and certificate sequence

Record the exact release, Render service ID, prior DNS record set, zone plan,
operator, and rollback owners before step 1. Change one layer at a time.

1. Deploy `frame-web` on its default Render hostname. Verify `/health/live`
   and `/health/ready`, bounded shutdown, and the expected release. Keep the
   default Render hostname enabled for diagnosis during the observation window.
2. Apply the validated Blueprint custom domain. In the authoritative portfolio
   zone plan, assert that `frame` has no CNAME/A/AAAA conflict, no wildcard is
   being introduced, and unrelated apex, `www`, shop/store, and Stripe entries
   are semantic no-ops. Remove only an inventoried conflicting exact `AAAA`.
3. Create the exact CNAME as DNS-only. Do not proxy it yet. Wait for Render
   domain verification and its public origin certificate.
4. Check CAA at the hostname and inherited zone levels. Existing CAA policy
   must continue to permit Render's currently documented issuers (including
   Let's Encrypt and Google Trust Services when applicable). Do not install a
   Cloudflare Origin CA certificate on Render.
5. While DNS-only, prove HTTPS, hostname verification, HTTP-to-HTTPS behavior,
   the full origin certificate chain, and renewal eligibility. A certificate
   warning or hostname mismatch blocks proxy activation.
6. Enable the exact Cloudflare proxy entry. Require Full (strict), then prove
   both the edge certificate and the Render origin certificate. Confirm the
   response is Frame, not the portfolio service.
7. Deploy the broad API and narrow compatibility Worker Routes together.
   Exercise every path, host, query,
   method, body, range, streaming, redirect, upgrade, and error class from
   `route-owner-matrix.json`. Cloudflare and application request IDs must show
   one Worker execution for API requests and no Worker execution for ordinary
   Render paths.
8. Observe renewal, cache, WAF, error, and latency signals through a complete
   release window. Only then disable the default Render hostname and prove
   Cloudflare-to-Render traffic remains healthy while direct
   `*.onrender.com` access no longer provides a cache/WAF bypass.

## Conformance and monitoring

Run locally before any protected operation:

```bash
python3 -I scripts/ci/check-same-origin-routing.py \
  --evidence target/evidence/same-origin-routing-local.json
python3 -I scripts/ci/same-origin-live-conformance.py --self-test \
  --evidence target/evidence/same-origin-live-runner-self-test.json
cargo test --locked -p frame-control-plane --test same_origin_routing_v1
ruby scripts/ci/check-render-blueprint.rb
python3 -I scripts/ci/check-cloudflare-zone-contract.py
```

The protected route trace must cover `/api`, `/api?query`, `/api/`, versioned
and unknown API paths, repeated slashes, literal and encoded dot segments,
semicolons, encoded slashes/backslashes, `/apix`, `/apiary`, uppercase and
encoded-prefix lookalikes, exact `/media-server` with and without a query,
all 16 protected children with their source-pinned methods (including concrete
`job-42` cancel/status paths), their `401`/`403`/`503` protected closure, and
root/child trailing slashes, unknown children, empty dynamic IDs, prefix
lookalikes, and uppercase rejection/fallthrough cases; it must also cover
unexpected Host, HTTP, explicit ports, duplicate
query keys, chunked and fixed-length bodies, GET/HEAD/POST/PUT/DELETE/OPTIONS,
single and multiple ranges, a streamed playback body, a non-followed error and
redirect probe, and an upgrade probe that does not return `101`.

Run the same read-only driver against a protected public environment. A known
public one-byte-or-larger share is required so the full lane proves both a
streamed `206` and a multiple-range `416`:

```bash
python3 -I scripts/ci/same-origin-live-conformance.py \
  --origin https://frame-staging.engmanager.xyz \
  --public-share-id "$PUBLIC_SHARE_ID" --require-full \
  --evidence target/evidence/same-origin-staging-route-trace.json
```

The driver allowlists only the two canonical origins, does not mutate provider
state, bounds response bodies, and writes status/header facts rather than
cookies, tokens, IPs, or response bodies.

Monitor by layer:

- DNS/TLS: resolution, edge/origin certificate expiry, CAA eligibility, and
  Full (strict) failures;
- Render: liveness, readiness, release ID, 404 rate, and default-host bypass;
- Worker: route invocation count, `421`, `400 invalid_api_path`, lookalike 404,
  API latency/errors, and request-ID uniqueness;
- cache: API/auth/health must never be `HIT`; only issue 41's reviewed hashed
  asset policy may become immutable `HIT`;
- non-regression: apex, `www`, shop/store, portfolio static paths, Stripe
  webhook behavior, and existing cache status are compared before and after.

Store status, certificate issuer/expiry, DNS answers, coarse cache status,
request IDs, and release IDs only. Redact cookies, authorization, IP addresses,
signed URLs, object keys, provider IDs not needed for correlation, and bodies.

## Layered rollback

Roll back one authoritative layer and record its start/end time and validation.

### Worker Route rollback

Remove only `frame.engmanager.xyz/api*` and
`frame.engmanager.xyz/media-server*` through the protected Wrangler owner in
one deployment so no declared Worker surface is stranded at an old script.
Verify `/`, health, assets, share pages, and unknown non-API paths still reach
Render. API requests will then reach Render and must produce its reviewed
non-cacheable 404; do not add a temporary wildcard or portfolio fallback.

### CNAME rollback

If edge TLS or routing is at fault and the Render certificate is still valid,
switch only the Frame CNAME to DNS-only. Prove direct HTTPS and Render health.
This bypasses Cloudflare controls, so keep the interval bounded and recorded.
Restore the exact prior Frame record only if DNS itself must be reverted.

### Render rollback

Roll back to the named compatible Render deploy. Re-enable the default Render
hostname temporarily only for a bounded diagnostic window, then repeat the
bypass test before disabling it again. Do not point Frame at the portfolio
service.

### Edge rules rollback

Disable only the exact Frame-host rule that regressed, using the imported
whole-phase state. Never replace a ruleset, purge the zone, relax Full
(strict), or modify apex/shop/Stripe behavior as a Frame rollback.

The route removal, DNS-only transition, and default-host toggle are separate
rehearsals. A failed layer must not trigger simultaneous changes to all three.

## Protected closeout record

Issue 39 cannot close from credential-free evidence alone. Attach the
authoritative DNS history and no-op plan, Render verification, CAA and both TLS
chains, Full (strict) trace, exhaustive raw/normalized owner matrix, cache
results, single-hop request-ID trace, default-host denial, unrelated-host
comparison, and timestamped Worker Route rollback and CNAME rollback
rehearsals. Any missing item remains an explicit launch blocker.

References: [Cloudflare Worker Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/),
[Cloudflare URL normalization](https://developers.cloudflare.com/rules/normalization/),
[raw request path field](https://developers.cloudflare.com/ruleset-engine/rules-language/fields/reference/raw.http.request.uri.path/),
and [Render with Cloudflare DNS](https://render.com/docs/configure-cloudflare-dns).
