# Render web-service operations

This runbook owns the `frame-web` Render process only. Cloudflare DNS and the
same-origin Worker route are governed by ADR 0004 and issue 39; the native media
executor is a separate service. Never install GStreamer, attach a disk, or give
this process D1, R2, Media, session-signing, or administrator credentials.

## Committed service decision

| Decision | Initial value | Gate and reason |
|---|---|---|
| Packaging | Native Rust | The build is targeted to `frame-web`; CI rejects GStreamer, Worker, media, and control-plane dependencies. Docker adds no required system capability. |
| Region | Oregon | This is the committed pre-provisioning candidate. Because a Render service region is immutable, production creation is blocked until an Oregon preview/staging trace meets the latency row below. Recreate before launch if it does not. |
| Compute | Two Starter instances | Paid, always-on, 512 MiB/0.5 CPU instances provide node diversity without autoscaling. A single instance is not the production topology. |
| Workspace | Pro or higher | Required before the manual preview-environment gate is enabled. Entitlement must be inspected in Render; it is not inferred from YAML. |
| Preview | Manual, one isolated environment at a time, three-day inactivity expiry | A preview uses Render's URL as its canonical origin and the non-production API selector. `sync: false` values are absent. |
| Shutdown | Render 60 seconds; application 55 seconds | Five seconds remain for platform enforcement. This web tier performs no durable mutation or background work. |
| Monthly ceiling | USD 50 before usage overages | The owner records the current two-Starter compute price plus the Pro workspace price before provisioning. If the fixed subtotal reaches the ceiling, stop; do not silently reduce replicas or use Free. |

The price and entitlement inspection is deliberately a provider gate because
rates and workspace state change. The release record stores the dated price
page/export, fixed monthly subtotal, included pipeline minutes/bandwidth, and
an approver. No checked-in document is treated as a bill.

### Promotion budgets

- Clean locked native build, including the pinned hydration bundler: p95 at or
  below 15 minutes across three Render builds.
- Process start to `/health/ready`: p95 at or below 30 seconds across ten
  restarts; local smoke must be below five seconds.
- Oregon browser-to-public-page TTFB: p95 at or below 250 ms from the primary
  audience probe and below 500 ms from the secondary probe.
- Anonymous SSR Worker read: 1.5-second attempt deadline, 64 KiB body ceiling,
  two attempts, three failed calls before a ten-second circuit opens.
- Sustained capacity: both instances below 70% CPU and 75% memory at the
  reviewed peak request rate, with readiness remaining successful.

Failure of a budget blocks production. Capacity is increased by changing the
explicit Blueprint decision and rerunning cost/load evidence; a persistent disk
or media dependency is never a scaling shortcut.

## Configuration contract

Bind precedence is exact: `FRAME_ADDR` (local/test override), then
`0.0.0.0:$PORT`, then `127.0.0.1:3000`. A non-zero `PORT` is required by the
Render environment. Nonlocal startup additionally requires:

| Variable | Production | Preview |
|---|---|---|
| `FRAME_DEPLOYMENT` | `production` | `preview` |
| `IS_PULL_REQUEST` | absent/false | `true` |
| `FRAME_PUBLIC_ORIGIN` | `https://frame.engmanager.xyz` | ignored fail-closed sentinel |
| `RENDER_EXTERNAL_URL` | admitted default Render host | authoritative preview canonical origin |
| `FRAME_API_ORIGIN` | same as public origin | exact non-production staging origin |
| `FRAME_PROXY_TRUST` | `render` | `render` |
| `RENDER_GIT_COMMIT` / `FRAME_RELEASE_ID` | bounded safe release label | bounded safe release label |
| `FRAME_DIAGNOSTIC_TOKEN` | optional `sync: false`, at least 24 ASCII bytes | absent unless separately provisioned |
| `FRAME_WORKER_RELEASE` | protected safe Worker deployment ID | absent |
| `FRAME_RENDER_DEPLOY` | protected safe Render deployment ID | absent |
| `FRAME_MIGRATION_LEVEL` | exact ordered migration filename | absent |
| `FRAME_PORTFOLIO_CONSUMER` | protected safe portfolio consumer ID | absent |

The four release-join values are all-or-nothing. When they are present,
`RENDER_GIT_COMMIT`/`FRAME_RELEASE_ID` must be a full lowercase Git SHA and all
identifiers pass strict length/character validation. A partial or unsafe join
fails startup. They are promotion metadata, not credentials; `sync: false`
keeps them absent from previews and requires the protected release owner to
update all four together.

Render-edge mode accepts only an exact `X-Forwarded-Proto: https` when that
header is present. RFC `Forwarded` is rejected. Host authority comes only from
the validated `Host` allowlist; forwarded host/client-IP values are never used
for authorization, redirects, canonical URLs, rate identity, or logs.

`FRAME_RUNTIME_TEST_MODE=true` creates one bounded local-only drain route. The
configuration parser rejects it in preview and production.

## Health and dependency matrix

All health responses are non-cacheable. Contracts are small JSON objects with
safe service, status, deployment, release, and boolean/component values.

| Fault | `/health/live` | `/health/ready` | Protected `/health/dependencies` | Protected `/health/release` |
|---|---:|---:|---:|---:|
| Event loop alive | 200 | depends below | depends below | depends below |
| Invalid startup config | process exits before bind | unavailable | unavailable | unavailable |
| Router/config initialized, verified assets present | 200 | 200 | independent | independent |
| Production/preview hydration bundle missing/tampered | 200 | 503 | independent | independent |
| Anonymous SSR client cannot be constructed | 200 | 503 | unavailable | independent |
| One Worker/D1/R2/Media/network timeout | 200 | 200 | 503/degraded | independent expected metadata |
| Release join absent | 200 | unaffected | unaffected | 503/incomplete |
| Complete safe release join | 200 | unaffected | unaffected | 200/joined |
| Missing/wrong diagnostic bearer | 200 | unaffected | 404 indistinguishable | 404 indistinguishable |

Readiness never calls Cloudflare. That prevents a transient upstream incident
from restarting every Render instance. The diagnostic performs only the fixed
anonymous `/api/v1/health` read and exposes no origin, binding, account, body,
or credential detail.

The release diagnostic is also no-store and token-hidden. It exposes exactly
service/status, source Git SHA, contract major, Worker release, Render deploy,
migration level, and portfolio consumer. It never reads a request-supplied
origin or emits provider/account data. Its values are expected promotion
metadata; Issue 44's protected verifier still compares them with provider and
consumer evidence before launch.

## Stateless and SSR boundary

`frame-web` loads one immutable, hash-verified hydration directory next to its
executable. It writes no working-directory state. The filesystem is disposable:
no video, derivative, session, job, manifest, or upload body may be stored
there. Browser upload capabilities go directly to the Worker/R2 boundary.

Authenticated/private pages server-render a generic `noindex` shell and load
through same-origin browser API calls. Public SSR has only two fixed reads:
`/api/v1/health` for protected diagnostics and
`/api/v1/public/shares/{bounded-id}` for authorized public metadata. The client
disables redirects and ambient proxies, sends no cookie/authorization header,
limits response type/body/deadline/retries, validates the public DTO, and uses
a circuit breaker. Failure renders the same generic non-cacheable unavailable
shell; no upstream body is logged or embedded.

## Deploy, observe, and roll back

1. Validate YAML plus repository invariants, then validate against Render CLI
   2.21.0 (minimum supported validation is 2.7.1):
   `render blueprints validate render.yaml`. Attach the version and output.
2. Confirm the Blueprint plan: exactly one native Rust web service, two paid
   instances, no disk, no branch override, `checksPass` authority, manual
   previews, three-day expiry, and the exact environment inventory.
3. Run a clean locked build and retain build duration, dependency tree, SBOM,
   binary digest, startup/readiness trace, and logs containing only safe release
   metadata.
4. Create one manual preview. Confirm its canonical is
   `RENDER_EXTERNAL_URL`, its API is staging, it is `noindex`, it has no
   production cookie/credential/data, and it expires or is deleted.
5. Create or update staging in Oregon. Run ten restarts, two-instance request
   distribution, SIGTERM in-flight drain, load, and regional latency probes.
6. Promote only after issue 40's release authority passes. Render waits for all
   new instances to pass readiness before moving traffic; a failed build/start/
   readiness must leave the named preceding release serving.
7. Observe error rate, p95/p99 latency, CPU, memory, restart count, readiness,
   dependency diagnostic, and request correlation IDs. Alert on consecutive
   readiness failures, restart loops, circuit-open duration, or budget breach.

Application logs include the validated release and, when available, bounded
`CF-Ray`/`Rndr-Id` correlation values. They never include cookies, authorization,
query strings, public identifiers, upstream bodies, CSP samples, or client IPs.

To roll back, name the last known-good commit and use Render's rollback action;
do not rebuild an unpinned ref. Verify liveness/readiness and the release label,
then separately assess Worker compatibility. Native-runtime rollback rebuilds
on the current platform, so retain the immutable release artifact/SBOM and use
the compatibility gate. D1 expand-only migrations are not reversed.

For an emergency restart, first capture the release/event/request IDs and
current health responses, then restart the affected service through the Render
dashboard or pinned CLI. Restarting all replicas is not a repair for a Worker
outage because dependency health is deliberately separate. If the new release
is implicated, rollback is preferred to repeated restarts.

## Domain and default-host sequence

Keep `renderSubdomainPolicy: enabled` through bring-up. Issue 39 owns DNS-only
certificate verification, proxied Full (strict), Worker `/api*` routing, cache
policy, and custom-domain Host tests. Disable the `onrender.com` hostname only
after that protected record passes. If custom-domain routing or Cloudflare
policy regresses, re-enable it as the reviewed rollback before changing DNS;
never expose it as an undocumented long-term policy bypass.

Provider records and commands must use synthetic data, bounded spend/time,
redacted artifacts, explicit cleanup, and production concurrency one.
