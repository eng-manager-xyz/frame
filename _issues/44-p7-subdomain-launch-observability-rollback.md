---
title: "Launch the Frame subdomain with SLOs, observability, and rehearsed rollback"
labels:
  - "phase:p7"
  - "area:ops"
  - "area:portfolio"
  - "area:render"
  - "area:cloudflare"
  - "type:release"
  - "risk:critical"
depends_on: [34, 35, 37, 38, 39, 40, 41, 42, 43]
size: epic
---

# 44 · Launch the Frame subdomain with SLOs, observability, and rehearsed rollback

## Outcome

`frame.engmanager.xyz` launches as an observable, supportable portfolio
service with approved SLOs, capacity and cost budgets, privacy-safe telemetry,
layer-specific alerts, a staged go/no-go, and a timed rollback that preserves
portfolio availability and Frame data.

## Current reference

Issue 34 owns Frame-wide production hardening and issue 35 owns the core Cap
migration cutover. Issues 36–43 add a new external distribution surface across
two repositories, Render, Cloudflare DNS/Worker/D1/R2/Media, and optional
native GStreamer executors. Each layer can fail or roll back independently;
one generic health check or one provider dashboard cannot establish service
health.

The portfolio already sits behind the same Cloudflare zone. Frame launch must
not alter apex/shop routing, purge the whole zone, share credentials/cookies,
or make portfolio rendering depend on Frame.

## Dependencies

[#34](./34-p6-operational-hardening.md),
[#35](./35-p6-progressive-cutover-decommission.md), and
[#37](./37-p7-engmanager-portfolio-integration.md) through
[#43](./43-p7-cross-repo-contract-preview-e2e.md)

## Scope

Define service ownership and SLOs, correlate privacy-safe telemetry across
Cloudflare and Render, instrument synthetic journeys, establish capacity/cost
budgets and alerts, write support/incident/runbooks, perform staged DNS/proxy/
route/portfolio launch, observe canaries, rehearse every rollback layer, and
record final go/no-go evidence.

### Service indicators

At minimum, measure separately:

- portfolio-to-Frame link success without counting portfolio rendering as a
  Frame dependency;
- edge DNS/TLS/HTTP availability and latency;
- non-API Render SSR/assets/readiness/startup/deploy health;
- `/api/v1` Worker availability/latency/error/rate-limit and D1/R2/Media
  dependency state;
- direct upload-intent, R2 PUT/finalize, processing, and share playback success;
- cache correctness (private bypass and immutable HIT), not only hit ratio;
- auth/session/CSRF and public/private player outcomes;
- native GStreamer queue age/capacity/fallback where enabled;
- release freshness and paired Worker/Render/contract/portfolio versions.

No metric, log, trace, alert, support bundle, or synthetic artifact contains
raw media, captions, private titles, email, tenant/object identifiers, cookies,
tokens, signed URLs, or full request/response bodies. Correlation IDs are random
and mapped server-side under retention/access controls.

### Launch sequence

1. Approve owners, on-call/support path, SLO/error budgets, region/plan,
   provider spend/quota, and rollback decision authority.
2. Prove contract consumers, staging Worker/D1/R2/Media, Render preview, and
   hermetic/provider suites.
3. Deploy the production Worker/API compatibly and the Render service on its
   default hostname; verify release manifest.
4. Attach `frame.engmanager.xyz` DNS-only and wait for Render certificate.
5. Enable Cloudflare proxy with Full (strict), then the broad `/api*` Worker
   Route with strict segment validation and bypass-first cache/security rules.
6. Run internal synthetic/canary traffic; enforce WAF/rate limits only after
   observation.
7. Add the portfolio link to a limited/flagged surface, then normal placement.
8. Enable optional status, auth handoff, or player embed separately; none is a
   launch prerequisite.
9. Observe the approved window, close launch defects, then consider disabling
   the default Render hostname.

### Rollback layers

Document triggers, authority, expected time, data effect, and verification for:

- portfolio live status off, then portfolio link/card removal;
- optional embed/handoff/browser API kill switches;
- WAF/rate/cache rule disable or exact scoped purge;
- Worker version rollback and `/api` Route removal;
- Render instant rollback and default-hostname re-enable;
- Cloudflare proxy back to DNS-only and DNS record restoration/removal;
- D1 forward-fix/compatibility behavior without destructive rollback;
- Media Transformations kill switch and native/legacy GStreamer fallback;
- credential rotation after suspected exposure.

Removing the Worker Route must leave Render/non-API traffic working; removing
Frame DNS must not change portfolio/shop records. No rollback deletes D1/R2
data or source media.

### Out of scope

- Declaring success from a homepage `200` while API/upload/privacy paths fail.
- Using real customer media for synthetic monitoring.
- Hiding a breached SLO by increasing cache TTL, retrying mutations, or
  disabling privacy/security checks.
- Reopening issue 35's completed core Cap authority/decommission decision.
  P7 rollback preserves migrated D1/R2 data and changes only the new portfolio,
  Render, Cloudflare-route, and optional browser-integration layers.

## Deliverables

- [ ] Service catalog, owner/on-call/support/escalation map, SLOs/error budgets,
  capacity and provider cost/quota budgets, and launch decision authority.
- [ ] Correlated Cloudflare/Worker/Render/client dashboards and actionable
  symptom-based alerts with runbook links.
- [ ] Synthetic landing, API, auth boundary, direct upload/finalize/process,
  public playback/Range, cache/privacy, and portfolio-degradation monitors.
- [ ] Release/version endpoint or safe headers joining Git SHA, contract major,
  Worker release, Render deploy, migration level, and portfolio consumer.
- [ ] DNS/TLS, deploy, cache/WAF, provider outage, stuck job, privacy incident,
  cache leak, credential rotation, and support runbooks.
- [ ] Staged launch checklist, go/no-go record, observation report, and a timed
  game day for every rollback layer.
- [ ] Post-launch review with remaining risk owners and explicit decision on
  default Render hostname, optional integration features, and legacy paths.

## Acceptance criteria

- [ ] Approved SLOs cover landing, API, upload/finalize/process, and public
  playback; dashboards distinguish Render, Worker, D1/R2/Media, native worker,
  DNS/TLS, and cache failures.
- [ ] Seeded failures at each layer alert within the target and lead an
  operator to the failing boundary without exposing sensitive data.
- [ ] Portfolio pages remain available and within baseline latency when Frame
  DNS, Render, Worker, D1/R2/Media, or native processing is unavailable.
- [ ] Capacity/load tests meet startup, SSR, concurrent request/upload-intent,
  queue, playback, and cost budgets with headroom or approved scaling actions.
- [ ] Synthetic upload uses only approved generated media, verifies direct R2
  transit and deterministic derivative/playback, and cleans up under retention
  policy.
- [ ] Cache/privacy monitors catch a seeded private HIT, stale deletion, or
  cookie variance as release-blocking incidents rather than availability
  successes.
- [ ] DNS-only, proxied Full (strict), Worker route, Render rollback, portfolio
  removal, cache purge, Media fallback, and secret rotation are timed and
  rehearsed without changing unrelated zone resources or losing data.
- [ ] N/N-1 Worker/web compatibility holds through the observation window;
  release metadata identifies drift and prevents an incompatible promotion.
- [ ] No launch artifact/telemetry/support bundle contains forbidden media,
  personal data, internal object keys, credentials, or signed URLs.
- [ ] Go/no-go is signed only after all P7 dependencies close, critical/high
  defects close, and rollback decision makers confirm the irreversible gates.

## Required test evidence

- Dashboard/alert screenshots or exports with seeded boundary failures.
- Load/capacity/cost/quota report and synthetic journey history.
- Cache/privacy incident drill and telemetry privacy audit.
- Timestamped launch and full rollback game-day record.
- Portfolio baseline comparison during Frame outage.

## Risks and open questions

- A single hostname hides two compute origins; request IDs and route-owner
  labels are essential for diagnosis.
- Provider dashboards use different clocks, sampling, and retention; define
  correlation and incident evidence expectations.
- Disabling the Render default hostname reduces bypass but removes a diagnostic
  endpoint; retain a tested re-enable procedure.
- Cloudflare Media is beta and must retain the issue-34 cost/change watch and
  GStreamer fallback.

## Rollout and rollback

Follow the launch sequence without skipping DNS-only certificate validation or
the bypass-first cache phase. Pause when a gate fails; do not compensate by
changing unrelated portfolio or zone resources. Keep each rollback reversible
through the observation window and delay optional embed/handoff/status features
until the base link, UI, API, upload, and playback SLOs hold.

Before closing, attach the approved SLOs, service catalog, dashboards, launch
record, observation report, privacy audit, and timed rollback evidence.
