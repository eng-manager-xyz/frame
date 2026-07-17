# Frame subdomain launch, observation, and rollback

This is the Issue-44 operator runbook for `frame.engmanager.xyz`. The
machine-readable authority is
`fixtures/launch-observability/v1/launch-policy.json`; the checked-in protected
ledger is deliberately all `not_collected`. Repository checks can prove the
definition and fail-closed evaluator, but they cannot approve spend, deliver a
page, change DNS, launch a portfolio link, or sign a go/no-go.

## Authority and launch gate

The repository owner makes the final launch decision. The release commander
sequences gates; the incident commander alone calls a rollback. Portfolio,
Cloudflare edge, Render, Worker, D1/R2, media, security/privacy, cost, and
support each have a separate role in the policy. The protected contact registry
resolves those roles to people and tests acknowledgement without checking
personal contact data into this repository. An operator never self-approves an
irreversible gate or credential revocation.

Every dependency—issues 34, 35, and 37 through 43—needs an immutable evidence
digest for the exact release. Missing, stale, failed, or unsigned evidence is
`NO_GO`. So is any open critical/high defect, privacy or cache correctness
failure, incompatible current/N-1 consumer, unapproved numeric cost or quota,
lost rollback readiness, or absent decision-maker acknowledgement. The
evaluator is advisory and always emits `authorizes_launch: false`:

```sh
python3 -I scripts/ci/launch-go-no-go.py \
  --snapshot /protected/frame-launch/snapshot.json \
  --output /protected/frame-launch/decision.json
```

The snapshot is capped at 1 MiB and has an exact allowlisted shape: safe release
identity, dependency digests, defect counts, SLO aggregates, dashboard/alert
status, synthetic aggregates, capacity/cost approvals, rollback aggregates,
privacy counts, portfolio baseline, staged-launch decisions, and protected
evidence digests. Unknown keys fail. It contains no contact details, tenant or
object identifiers, customer content, URLs, request bodies, secrets, or
provider messages. A `GO` recommendation still requires independent signatures
in the protected approval system before a separate authorized control changes
anything.

## Service levels and telemetry

The launch policy owns fourteen separate SLOs. Portfolio link, edge, Render
landing, Worker API, upload/finalize, and playback success use a 99.9% objective
and ten-basis-point error budget. Edge p95 is at most 750 ms, API p95 500 ms,
Render deploy-to-ready p95 five minutes, generated 60-second 1080p processing
to share p95 30 seconds, and native oldest-ready-job p95 30 seconds. Auth,
cache/privacy, and release compatibility have zero error budget.

Capacity measures startup, SSR and concurrent requests, upload intents, queued
jobs, and playback bytes with at least 30% headroom. Cost/quota inventory is
separate for Render plan/instances/build minutes, Worker requests/CPU, D1
rows/storage, R2 operations/storage/egress, managed transformations, and native
compute/scratch. Each has an owner and unit catalog; every checked-in numeric
cap remains null and quota status `not_collected` until the protected cost
approver supplies the signed launch values. Missing approval is `NO_GO`, not a
zero-cost assumption.

Cloudflare, Worker, Render, and client signals join only through a random UUIDv4
correlation ID and safe release/boundary/operation/result labels. The server-side
mapping is access controlled and retained under the approved incident policy.
The dashboard never uses media, captions, a title, email, user/tenant/object
identifier, cookie, token, credential, signed URL, body, provider message, or
filesystem path. Operational aggregates retain for 30 days unless a separately
approved incident hold applies.

Five dashboards separate portfolio/edge, Render/Worker, D1/R2/Media/playback,
auth/cache/release, and capacity/cost. Alerts are symptom based, target the
failing boundary within 60 seconds (30 seconds for privacy), identify an owner
role, and link back to this runbook. Provider dashboard clock skew, retention,
and sampling must be recorded in the protected export; a request ID alone does
not prove two events are the same.

## Release identity

The immutable build manifest supplies source Git SHA, contract major, and D1
migration level. Promotion evidence adds the actual Worker release, Render
deploy, and portfolio consumer. The token-hidden, no-store `/health/release`
endpoint joins those six values from all-or-nothing bounded configuration and
returns 503 when the join is absent. The launch snapshot accepts only bounded
identifier characters, a full lowercase Git SHA, contract major 1, and the
ordered migration filename. Unknown fields or an unverified production endpoint
record stop launch.

Read and verify the live join only from the protected environment. The
diagnostic token is accepted only from a regular, non-symlink, owner-only file;
the verifier disables ambient HTTP proxies and redirects, caps the response,
requires no-store/no-cookie and exact fields, and writes only safe release
metadata:

```sh
python3 -I scripts/ci/release-join-conformance.py \
  --origin https://frame.engmanager.xyz \
  --token-file /protected/frame-launch/diagnostic-token \
  --expected-source-git-sha "$RELEASE_SHA" \
  --expected-worker-release "$WORKER_RELEASE" \
  --expected-render-deploy "$RENDER_DEPLOY" \
  --expected-migration-level "$MIGRATION_LEVEL" \
  --expected-portfolio-consumer "$PORTFOLIO_CONSUMER" \
  --evidence /protected/frame-launch/release-join.json
```

The current portfolio consumer and last released consumer both build against
the candidate contract. The Worker and web remain N/N-1 compatible for the
whole observation window. A Render deploy reaching the expected source SHA
while the Worker or portfolio remains on an incompatible release is drift, not
eventual success. Page `release_contract`, retain the first mismatch, and stop
promotion until the paired record is exact.

The repository release bundle intentionally leaves `portfolio_consumer_sha`
null because an independently owned repository must supply it. Do not replace
that null with an invented revision. Protected promotion joins the separately
verified records and attaches their digests to the snapshot.

## Staged launch

Run these gates in order. A failed gate pauses the sequence; never compensate by
changing unrelated portfolio or zone resources.

1. Resolve all owner roles, on-call/support paths, SLO/error budgets, region and
   plan, numeric spend/quota limits, capacity scaling actions, and rollback
   decision authority.
2. Prove current/N-1 consumers, staging Worker/D1/R2/Media, Render preview,
   hermetic suites, browser boundary, cache policy, and provider canaries.
3. Deploy the compatible production Worker and Render service on its default
   hostname; verify immutable subjects and the release join.
4. Attach only the exact Frame DNS record in DNS-only mode and wait for the
   Render certificate. Do not enable the proxy during certificate issuance.
5. Enable Cloudflare proxy with Full (strict), then the broad `frame.../api*`
   route. Verify raw first-segment validation and bypass-first cache/security.
6. Run internal synthetics. Observe WAF/rate rules before enforcing and retain
   false-positive disposition.
7. Add the ordinary top-level portfolio link on a limited flag, verify the
   portfolio baseline, then move it to normal placement.
8. Release optional status, handoff, CORS, or embed independently. None is a
   base-launch prerequisite.
9. Hold the signed observation window. Close critical/high defects and make
   explicit post-launch decisions before changing the default Render hostname
   or any legacy path.

The protected record binds start/end timestamps, release and prior compatible
release, every gate digest, decision roles, defects, SLO/error-budget state,
synthetic history, cost/quota, and rollback pointers. Never edit a prior record;
supersede it with a new digest.

## Portfolio

The portfolio owns only an ordinary accessible absolute link. Its request
handler must not call Frame. Optional anonymous status stays in a cancellable,
deadline- and body-bounded background task with last-good state. Frame cookies,
auth headers, and response bodies never cross into portfolio telemetry or HTML.

Before and during the launch window, measure portfolio availability and p95
latency with Frame healthy, then separately with Frame DNS, Render, Worker, D1,
R2, managed Media, and native processing unavailable. Availability remains at
least 99.9%; outage p95 may regress by no more than 5% from the bound baseline.
Capture the independently owned portfolio build and measurements in protected
evidence. The local two-origin harness validates semantics only.

Symptom: the link/card is absent or navigation fails. Page the portfolio owner,
confirm the independently deployed portfolio is otherwise healthy, disable any
optional status poll first, and then remove only the flagged link/card if the
base launch is unhealthy. Verify portfolio rendering makes zero Frame request
and that no apex/shop configuration changed. Data effect is none.

## DNS, TLS, and edge

For DNS/certificate work, follow `same-origin-routing.md`: capture the exact
`frame` record and unrelated apex, `www`, shop/store, webhook, and portfolio
records; apply a semantic no-op plan; attach DNS-only; wait for the Render
certificate; prove HTTPS at the origin; then enable proxy with Full (strict).
CAA eligibility, certificate renewal, raw/normalized hostile paths, and the
exact broad Worker route are protected evidence.

On DNS/TLS failure, freeze later gates and retain the first trace. Classify DNS
resolution, edge certificate, origin certificate, strict-mode validation, or
route ownership. Return the exact Frame record to DNS-only only after origin
certificate verification. If necessary, restore/remove that record alone. Do
not alter apex, `www`, shop/store, portfolio, wildcard, or zone-wide settings.

On cache/WAF/rate failure, switch the one Frame rule from enforce to observe or
disable it. A private `HIT`, stale deleted share, or authenticated cookie variant
is a release-blocking privacy incident even if it returned 200. Disable the
exact rule and purge only bounded `https://frame.engmanager.xyz` URLs or
`frame:` tags through `scripts/ops/frame_cache_purge.py`; whole-zone purge is
forbidden. Re-prove API/auth/private bypass and fingerprinted immutable
MISS-to-HIT before resuming.

## Render

Classify startup, SSR, assets, readiness, deploy, shutdown/drain, custom domain,
or external Worker dependency separately. Render readiness covers local router,
configuration, hydration assets, and public SSR composition; it does not make
the Worker a process-readiness dependency. Protected dependency diagnostics use
the hidden token route and never expose provider origins or credentials.

On a failed deploy or web regression, retain the previous healthy deploy, select
the named previous compatible Render deploy, and re-enable the default hostname
for bounded diagnosis if it had been disabled. Verify readiness, SSR, assets,
canonical host, preview `noindex`, API origin, and the release join. Render is
stateless; rollback must not write or delete D1/R2. A local restart/drain smoke
does not satisfy Render rollback timing.

## Worker API

Classify Worker code, broad-route ownership, raw path rejection, host/scheme,
rate limiting, authentication, D1, R2, or Media before taking action. Verify the
safe API health contract, dependency diagnostics, request-ID continuity, and
shared-cache bypass. Never retry a mutation merely because its response was
lost; query by durable idempotency and reconcile first.

On Worker regression, deploy the named previous schema-compatible Worker. If
route ownership itself is implicated, remove only `frame.engmanager.xyz/api*`
and `frame.engmanager.xyz/media-server*` together and prove all other non-API
Render traffic still works. D1 migrations remain expand-only and
are not rolled back. Restore the route only after the raw/normalized matrix,
current/N-1 contract, auth/privacy, and direct-upload synthetics pass.

## D1, R2, and processing

For D1 failure, stop authority changes and new promotion. Distinguish contention,
migration mismatch, quota, and outage from safe aggregate dependency signals.
Use a forward compatible fix; never destructively rewind a migration. Reconcile
acknowledged writes, auth/session links, job state, and object manifests before
resuming.

For R2 upload/finalize failure, stop issuing new grants for the affected scope.
Preserve committed multipart state and query exact finalize/idempotency records.
Do not proxy bytes through Worker, infer success from a provider list, guess a
prefix, or delete source media. Resume after intent, direct PUT, finalize,
checksum/size, revoke, and range probes pass.

For a provider outage or output drift, disable only the exact managed profile
revision. Fence the current attempt, reconcile its terminal status, and select
one approved native/legacy GStreamer fallback. Preserve source and committed
outputs; publish one deterministic result and bill one logical effect. Do not
repeat an indeterminate provider call. Re-enable only after the contract probe,
cost/quota review, native headroom, and staged/final reconciliation pass.

For a stuck job, alert on oldest ready age, expired lease, recovery age, dead
letters, and charge-without-terminal-state. Stop admission, fence stale leases,
and disposition queued, claimed, started, staged, published, canceled, and
indeterminate work using the progressive-cutover rules. A started external
effect is queried, never resubmitted to discover its outcome. An unowned
indeterminate effect blocks launch.

## Public playback

The public synthetic verifies descriptor privacy, `HEAD`, full GET, bounded and
suffix ranges, deterministic `416`, captions, and immediate revocation after a
privacy/deletion transition. Public, unlisted, private, missing, processing,
failed, and deleted outcomes must follow the versioned contract; unavailable
states are indistinguishable where required.

On descriptor, range, caption, or playback failure, preserve the request/release
class only, not the share ID, title, caption text, URL, or response body. Compare
Worker route, R2 manifest/checksum, exact generation fence, cache disposition,
and Render player. Disable publication or the affected profile revision on any
manifest/privacy drift. Availability never overrides a privacy mismatch.

## Privacy, cache, and credentials

Any cross-tenant/private exposure, private cache hit, stale deleted object,
cookie variance, session/CSRF bypass, leaked URL/title/caption/email, or
credential marker is severity one and automatic `NO_GO`. Freeze promotion,
disable the narrow optional/browser/cache surface, preserve redacted evidence,
and follow `service-reliability-and-incidents.md`. Do not broaden logging to
debug it.

Scan Cloudflare/Worker/Render/client exports, alert payloads, synthetics,
support bundles, and the launch record against the telemetry denylist. The
support bundle generator rejects unknown fields. Zero findings are required;
redacting a forbidden field after collection does not prove collection was
lawful.

On suspected credential exposure, inventory the one credential class and its
capabilities, issue a scoped replacement, deploy it, prove health, revoke the
old key ID, and probe denial. Record receipts and capability changes without
secret values. Rotate Cloudflare deploy, R2 signing, session hash, webhook HMAC,
desktop update, and backup recovery credentials through their own overlap
policies; never rotate all classes simultaneously without incident authority.

## Support and incident response

Open a protected incident, assign commander, operations, security, support,
communications, and scribe. Freeze the launch and preserve the first failure.
Classify one or more explicit boundaries; do not report only “Frame down.”
Apply the narrow kill switch/rollback, reconcile data and provider effects,
confirm SLO recovery, close temporary access, and retain a redacted timeline.

Support messages include public symptom, affected surface, safe workaround,
next update time, and status location. They contain no customer list, private
title, resource ID, object locator, credential, raw provider error, or approval
identity. Support aggregates record severity/count and owner only. An open
severity-one/two launch case or an unowned customer impact is `NO_GO`.

## Timed rollback game day

Exercise every row below in the protected topology and bind monotonic start/end
times, exact scope, prior/current release, decision authority, verification,
data effect, and unrelated-resource probes. Local logical-clock games prove the
assertions, not the provider timing.

| Layer | Target | Required verification |
|---|---:|---|
| optional portfolio status | 5 min | static portfolio/link usable; no handler dependency |
| portfolio link/card | 5 min | portfolio healthy without Frame request |
| optional embed/handoff/CORS | 5 min | base link/UI/API/playback unchanged |
| cache/WAF/rate | 5 min | exact rule/tag/URL only; private bypass; zone unchanged |
| Worker/route | 5 min | previous compatible Worker or exact route removal; non-API Render works |
| Render | 5 min | named deploy, readiness/SSR/assets, default host re-enabled if needed |
| proxy/DNS | 5 min | exact Frame record only; apex/www/shop/portfolio unchanged |
| D1 forward fix | 15 min | expand schema and acknowledged writes preserved; zero unexplained differences |
| Media fallback | 5 min | one fenced fallback/publication/billable effect; source preserved |
| credential rotation | 15 min | replacement healthy, old key denied, capability inventory reconciled |

No rollback deletes D1/R2/source media, rewinds a destructive migration, purges
the zone, repeats billing, changes unrelated records, or removes the diagnostic
default hostname before its re-enable path is tested.

## Observation and post-launch review

Hold at least the signed launch observation window with current/N-1 compatibility,
all SLO/error budgets, synthetic history, alert delivery, capacity/quota/cost,
privacy scans, and rollback pointers fresh. Capture defects and decisions as new
immutable records. A homepage 200 is never sufficient.

The post-launch review assigns an owner and deadline to every remaining risk and
makes an explicit `keep_enabled`, `disable`, `defer`, `retain`, or `remove`
decision for the default Render hostname, optional portfolio status, auth
handoff, browser CORS, public embed, and legacy paths. The default hostname
stays available until its protected disable/re-enable rehearsal passes. Optional
features stay disabled unless their individual browser/security matrices pass.
Core Cap authority/decommission remains governed by Issue 35 and is not reopened
or destructively reversed by this launch.
