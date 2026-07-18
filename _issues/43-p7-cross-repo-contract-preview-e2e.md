---
title: "Add cross-repository contract CI, previews, and portfolio-to-Frame E2E"
labels:
  - "phase:p7"
  - "area:test"
  - "area:ci"
  - "area:portfolio"
  - "area:render"
  - "area:cloudflare"
  - "type:integration"
  - "risk:high"
depends_on: [36, 37, 38, 39, 40, 41, 42]
size: epic
---

# 43 · Add cross-repository contract CI, previews, and portfolio-to-Frame E2E

## Outcome

Frame and `engmanager.xyz` can release independently because shared fixtures,
consumer builds, paired previews, and real-browser tests detect contract,
routing, cache, auth, media, and accessibility regressions before production.

## Current reference

Frame CI has native/wasm/GStreamer scaffolding but no browser or cross-repo
lane. The pinned portfolio has roughly seventy Rust tests and golden/router
coverage but no GitHub Actions, browser E2E, Blueprint, or Frame fixture
consumer. The repositories use different Rust toolchains and must be built in
separate jobs/caches.

Render can create manual expiring preview services, but `sync: false` secrets
are not copied automatically and production endpoints must not be used. A
Cloudflare Worker preview/staging environment needs separate D1/R2 resources
and routes or a safe non-production public endpoint.

## Dependencies

[#36](./36-p7-frame-client-public-contract.md) through
[#42](./42-p7-browser-auth-embed-boundaries.md)

## Scope

Create provider-neutral contract fixtures, producer/consumer compatibility
jobs, hermetic two-origin local tests, isolated staging/preview wiring,
production-like browser and HTTP suites, artifact/privacy policy, flake/cost
budgets, and cross-repository release signaling without requiring an atomic
merge or broad bot credential.

### Contract strategy

- Frame owns canonical versioned fixtures/schema in
  `fixtures/frame-api/v1` and tests its producers against them.
- The portfolio pins a Frame Git SHA and tests its client/rendering against the
  same fixtures; fixture copies, if unavoidable, include source SHA and a
  drift check.
- Additive compatible Frame changes must pass the last released portfolio
  consumer. Breaking major changes require parallel-version support and a
  portfolio migration before old removal.
- A scheduled advisory lane tests the portfolio default branch against Frame
  main without silently updating its pinned dependency.

### Local and preview topology

Hermetic tests run distinct portfolio and Frame origins (for example mapped
localhost hostnames or isolated ports) and a Worker-compatible API fake with
the exact version/error/cache headers. Provider smoke tests run separately.

Manual Render previews are short-lived, `noindex`, and point only to a staging
Worker/D1/R2 namespace. They receive no production mutation token. Pairing a
portfolio preview with a Frame preview must be explicit and recorded; an
untrusted pull request cannot cause either repository to deploy billable or
privileged infrastructure.

### Required journeys

1. Portfolio homepage/nav link opens canonical Frame landing and browser Back
   returns correctly.
2. Frame landing, health, login boundary, dashboard denial/entry, and logout.
3. Public share/player with Range seeking, captions, unavailable/deleted/
   processing/error states, and canonical metadata.
4. Optional public portfolio status in healthy, stale, incompatible, timeout,
   and malformed-response states.
5. Optional code/PKCE return flow and public-player embed only when enabled.
6. Direct R2 upload-intent, CORS PUT, finalize, processing state, derivative,
   and playback without media bytes crossing Render.
7. Cloudflare routing and cache: non-API to Render, API to Worker, dynamic
   bypass, immutable asset HIT, privacy-change purge.
8. Render/Worker outage, slow response, deploy, reconnect, retry, and rollback.

### Out of scope

- Copying real private recordings or production databases into previews/CI.
- Giving a cross-repo bot write/admin access merely to keep fixtures in sync.
- Making remote provider/beta tests part of every untrusted PR.
- Retrying flakes until green without preserving first-failure evidence.

## Deliverables

- [ ] Contract producer/consumer matrix, fixture ownership/version policy, and
  last-released-consumer job.
- [ ] Portfolio GitHub Actions workflow using its pinned nightly, committed
  lockfile, Rust/router/golden tests, and `frame-client` fixture tests.
- [ ] Hermetic local two-origin harness with API/provider fakes and one command
  for the critical browser journeys.
- [ ] Manual expiring Render preview plus staging Worker/D1/R2 configuration,
  secret isolation, pairing instructions, and cleanup monitor.
- [ ] Browser/device/accessibility/security/cache suite for the required
  journeys with bounded synthetic media fixtures.
- [ ] Scheduled protected provider canary for Worker route, R2/Media, Render,
  and canonical-domain behavior.
- [ ] Artifact retention/redaction, timeout, retry, flake quarantine, cost, and
  failure ownership policy.

## Acceptance criteria

- [ ] A compatible additive API change passes current and last-released
  portfolio consumers; a seeded breaking field/type/status/path change fails
  before deploy.
- [ ] Frame and portfolio compile under their own pinned toolchains and
  lockfiles with separate caches; neither workflow mutates the other's repo.
- [ ] One local command runs the top-level link, public contract, routing,
  failure-degradation, and cache-header journeys without provider credentials.
- [ ] A trusted manual preview uses only staging Worker/D1/R2 resources, is
  `noindex`, receives no production secrets, expires automatically, and leaves
  no DNS/route/storage resource after cleanup.
- [ ] Untrusted PRs cannot create previews, call billable Media
  Transformations, read state/tokens, or access production data.
- [ ] Real-browser tests pass the supported mobile/desktop, keyboard,
  screen-reader, reduced-motion, CSP/CORS, Range, and optional iframe/message
  matrix.
- [ ] Cache tests repeatedly prove API/auth/private responses are never HIT and
  fingerprinted assets do become HIT; privacy changes purge inside the SLO.
- [ ] Fault injection proves Frame DNS/API/Render/Media failure never prevents
  the portfolio from starting or rendering its normal pages.
- [ ] CI artifacts contain only synthetic approved media and redacted traces;
  no cookie, token, signed URL, internal key, personal data, or private Frame
  response is retained.
- [ ] Flakes have an owner/deadline and first-failure artifacts; retries cannot
  silently convert a release-blocking failure to green.

## Required test evidence

- Seeded compatible/breaking producer-consumer runs.
- Local harness logs and browser reports for all required journeys.
- Preview creation, isolation, noindex, expiration, and cleanup evidence.
- Provider canary route/cache/upload/media traces with bounded cost.
- Outage/degradation and privacy artifact audit.

## Risks and open questions

- Git-SHA consumption can hide drift until the portfolio intentionally bumps;
  the scheduled advisory lane makes drift visible without changing authority.
- Preview custom domains and Cloudflare Routes add cost/cleanup complexity;
  start with Render preview URLs plus staging API where sufficient.
- Browser media tests can be slow and flaky; keep deterministic small fixtures
  and separate fast hermetic from protected provider lanes.

## Rollout and rollback

Land fixtures and hermetic consumers first, make them required, then add the
portfolio workflow, then manual previews, then protected provider canaries.
Disable a flaky provider lane only with an incident, owner, deadline, and
continued hermetic coverage. Roll back preview routing without touching
production DNS.

Before closing, attach workflow runs, consumer matrix, browser reports,
preview cleanup, artifact audit, cost baseline, and seeded-failure evidence.
