---
title: "Integrate Frame into the EngManager portfolio without availability coupling"
labels:
  - "phase:p7"
  - "area:portfolio"
  - "area:leptos"
  - "area:test"
  - "type:integration"
  - "risk:medium"
depends_on: []
size: epic
---

# 37 · Integrate Frame into the EngManager portfolio without availability coupling

## Outcome

Visitors can discover and enter Frame from `engmanager.xyz`; optional public
Frame status/project data degrades safely, and no Frame outage can delay or
break the portfolio.

## Current reference

At `matthewharwood/engmanager.xyz@1de52bc8f25793dea3697e67765d53785c05cdfa`,
the project/home content lives in `website/src/pages/homepage.rs`, shared
article/search navigation lives in `website/src/components/nav/mod.rs`, and
the custom navigation router correctly leaves cross-origin URLs to normal
browser navigation. The service worker also ignores cross-origin requests.

The portfolio has no Frame configuration or dependency. It does have a
timeout-bound Discord background poller with a watch-channel last-good
snapshot. HTML can be cached at Cloudflare for an hour with a stale window, so
personal or short-lived Frame data cannot be inserted into normal cached
pages. See [the portfolio reference](../docs/upstream-engmanager.md).

## Dependencies

None for the static top-level link and project/CTA implementation. Any child
task that enables live public Frame data depends on
[#36](./36-p7-frame-client-public-contract.md). Production exposure of the
link remains gated by [#44](./44-p7-subdomain-launch-observability-rollback.md),
so source work does not imply an early launch.

## Scope

Implement the first-party portfolio integration in the portfolio repository:
canonical configuration, a homepage project/CTA and/or nav link, pinned
`frame-client` consumption for approved public data, safe background refresh,
cache/SEO/accessibility behavior, test coverage, and a cross-repository release
record.

Stage the work so a static top-level link ships before any live Frame data.
The canonical production target is `https://frame.engmanager.xyz/`.

### Out of scope

- Routing `frame.engmanager.xyz` to the existing portfolio Render service.
- Sharing cookies or silently authenticating a portfolio visitor into Frame.
- Fetching Frame from a portfolio request handler.
- Embedding the recorder, camera, microphone, or display capture.
- Indexing private recordings or copying Frame's full search index into
  Tantivy.
- Adding Frame subdomain URLs to the apex sitemap; Frame owns its sitemap.

## Deliverables

- [ ] One canonical `FRAME_ORIGIN` configuration path with production default,
  validation, and explicit local/test override; remove duplicate string
  literals from new consumers.
- [ ] Accessible homepage project card/CTA and an intentional navigation
  placement pointing to the canonical Frame origin.
- [ ] Exact pinned `frame-client` Git revision if live public data is enabled,
  plus a corrected `.gitignore` and committed root workspace `Cargo.lock`.
- [ ] Optional `FrameClient` background task modeled on the Discord client:
  shared Reqwest client, short deadline, bounded body, slow polling with
  jitter, last-good snapshot, cancellation, and no handler-path I/O.
- [ ] A three-state presentation for available, stale/unavailable, and not-yet-
  configured data. The link remains usable in every state.
- [ ] Portfolio cache, CSP, security-header, canonical, robots, and sitemap
  decisions with tests.
- [ ] Deployment note that identifies the paired Frame and portfolio commits
  without requiring an atomic cross-repository release.

## Acceptance criteria

- [ ] With Frame healthy, the project link resolves through Cloudflare to the
  Render-hosted Frame UI and is keyboard/screen-reader discoverable.
- [ ] With DNS failure, timeout, invalid JSON, incompatible version, `429`,
  `500`, or a stale snapshot, portfolio startup and every existing route still
  render within their current timeout and cache budgets.
- [ ] No handler performs a synchronous Frame fetch, and the background loop
  has a deadline, jitter/backoff, cancellation, and bounded memory.
- [ ] Only issue-36-approved public fields can enter HTML. Cookies, auth state,
  private titles, signed URLs, object keys, owner/tenant IDs, and raw response
  bodies never enter HTML, logs, search, RUM, or cache tags.
- [ ] The portfolio's cross-origin navigation router and service worker do not
  intercept Frame navigation or cache Frame responses.
- [ ] Existing portfolio/shop host routing is unchanged; requests for the
  Frame host never reach the portfolio service in production.
- [ ] The apex sitemap contains only apex URLs, and Frame pages use Frame's
  canonical/robots/sitemap policies without duplicate indexing.
- [ ] Cache tests prove status changes cannot leak user-specific data or make
  a stale failure banner persist beyond the approved public-data TTL.
- [ ] Removing the optional live-data dependency leaves the static Frame link
  and the rest of the portfolio fully functional.

## Required test evidence

- Portfolio unit/router/golden snapshots for link, healthy, stale, and failed
  states.
- Browser test for keyboard navigation, cross-origin handoff, back navigation,
  service-worker behavior, and reduced motion.
- Fault-injected Frame timeout/error/version matrix with portfolio latency
  measurements.
- Production headers, canonical, sitemap, and Cloudflare cache trace.

## Risks and open questions

- A global nav can become crowded; homepage/project placement should follow
  the portfolio's existing information hierarchy and mobile tests.
- Polling a public health endpoint solely for decoration adds operational
  coupling; omit it unless the product value is clear.
- The portfolio currently accepts both `shop` and `store` names inconsistently.
  Frame work must not widen or silently rewrite that unrelated drift.

## Rollout and rollback

Ship the static link behind a local content/config change, then enable optional
public status data after the Frame contract and cache review pass. Roll back by
disabling live data first and reverting the link/card second; neither action
changes Frame DNS, auth, or stored media.

Before closing, attach the portfolio PR, pinned Frame SHA, browser/cache
evidence, and paired deployment record.
