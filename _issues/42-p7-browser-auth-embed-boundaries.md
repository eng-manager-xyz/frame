---
title: "Secure portfolio-to-Frame auth, browser, CSP, and embed boundaries"
labels:
  - "phase:p7"
  - "area:security"
  - "area:auth"
  - "area:portfolio"
  - "area:player"
  - "type:integration"
  - "risk:critical"
depends_on: [13, 21, 31, 32, 36, 37, 39]
size: epic
---

# 42 · Secure portfolio-to-Frame auth, browser, CSP, and embed boundaries

## Outcome

Portfolio navigation, Frame sessions, optional signed-in handoff, public-player
embedding, and browser API calls have explicit trust boundaries; sibling
subdomains cannot share or steal ambient authority, and private media never
leaks through CSP, CORS, cache, redirects, or messages.

## Current reference

The pinned portfolio has no general login/session or CORS middleware. Its
security layer sends `X-Frame-Options: DENY`, a report-only CSP that does not
allow Frame in `frame-src` or media outside self, and a Permissions Policy that
disables camera, microphone, display capture, and autoplay while limiting
fullscreen to self. Those defaults make an embedded recorder intentionally
non-viable.

Frame's current web shell has no auth/session/cookie/CSP/embed behavior. Issues
13, 21, 31, and 32 define the underlying identity, storage, dashboard, and
player semantics. [ADR 0004](../docs/adr/0004-engmanager-render-cloudflare-topology.md)
keeps Frame UI and API same-origin at `frame.engmanager.xyz`.

## Dependencies

[#13](./13-p2-auth-sessions-identity.md),
[#21](./21-p3-storage-security-lifecycle.md),
[#31](./31-p5-leptos-auth-dashboard.md),
[#32](./32-p5-leptos-share-player.md),
[#36](./36-p7-frame-client-public-contract.md),
[#37](./37-p7-engmanager-portfolio-integration.md), and
[#39](./39-p7-cloudflare-render-same-origin-routing.md)

## Scope

Threat-model the sibling-origin relationship and specify/implement canonical
navigation, session cookies, CSRF, redirect/return URLs, optional login handoff,
CORS, CSP, Permissions Policy, referrer policy, public-player iframe policy,
`postMessage`, analytics/telemetry boundaries, local/preview origins, and cache
interactions.

Top-level navigation, host-only sessions, CSRF, redirects, and deny-by-default
headers are base launch requirements. Handoff, direct portfolio browser API,
and player embedding remain independent optional capabilities: this issue can
close with an explicit disabled disposition and tested deny policy for any
capability not selected for launch. Its implementation/acceptance clauses
apply when that capability is enabled.

### Required trust model

- `engmanager.xyz` is a discovery/referral origin, not a Frame identity
  provider, session domain, or privileged API caller by default.
- `frame.engmanager.xyz` owns Frame UI and `/api/*`; routine UI traffic is
  same-origin and does not need CORS.
- Render SSR may call only fixed, anonymous public-read `/api/v1` endpoints
  with bounded transport. Authenticated/private state hydrates in the browser;
  Render does not forward ambient cookies or hold a broad service credential.
  Authenticated SSR is disabled unless a follow-up ADR proves a least-privilege
  session/service-proof design.
- Frame session cookies are host-only, `Secure`, `HttpOnly`, and have a reviewed
  `SameSite`/expiry/rotation policy. Never set `Domain=.engmanager.xyz` merely
  to share login with the portfolio.
- State-changing API calls require CSRF protection and exact Origin/Fetch-
  Metadata validation. Sibling origins are same-site in browser terminology,
  so SameSite cookies alone are not sufficient CSRF protection.
- Return URLs are parsed and matched against exact approved origins/paths;
  userinfo, encoded-host, scheme-relative, port, backslash, and nested URL
  tricks fail closed.

### Initial navigation

The first release is an ordinary top-level link. Public landing/share viewing
requires no shared auth. Login happens on Frame. If an authenticated handoff is
later approved, use authorization code plus PKCE or a one-time, audience-bound,
short-lived signed exchange with state/nonce and replay storage. Do not put a
bearer token, session, email, signed R2 URL, or private return state in query
strings, fragments, analytics, or referrers.

### Optional browser API access

A portfolio browser call to Frame is a separate capability, not implied by
using `frame-client` server-side. It must allow only exact production and
approved preview/local origins, return `Vary: Origin`, bound methods/headers,
handle unauthenticated OPTIONS safely, and forbid wildcard origin with
credentials. Public endpoints still enforce response size, rate, and privacy
policy. Prefer server-side last-good public status polling when browser access
adds no user value.

### Optional public-player embed

Recorder/dashboard/auth routes remain top-level. A public player may become
embeddable only on dedicated routes and only after:

- the portfolio adds exact Frame `frame-src` and media/connect origins;
- Frame's player CSP allows exact `frame-ancestors https://engmanager.xyz`
  (and reviewed `www`/preview origins) while other routes deny framing;
- the iframe `allow` list grants only required playback/fullscreen/PiP
  capabilities, never display capture/camera/microphone for the public player;
- `postMessage` uses an explicit schema/version, exact target/source origin,
  source-window validation, bounded payloads, and no secrets/private metadata;
- accessibility, consent, autoplay, sandbox, fullscreen, captions, keyboard,
  mobile, and privacy states pass issue 32.

The portfolio's `X-Frame-Options: DENY` protects portfolio pages and does not
need removal to embed a Frame response. Frame embed routes should rely on CSP
`frame-ancestors`; conflicting legacy headers must be tested across supported
browsers.

### Out of scope

- Parent-domain cookies, localStorage sharing, token-in-URL SSO, or implicit
  trust of every `*.engmanager.xyz` host.
- Delegating recorder capture permissions to the portfolio iframe.
- Allowing arbitrary customer domains in CORS/CSP without the custom-domain
  ownership and tenant policy from issues 14/21/32.
- Treating a report-only CSP as enforcement completion.

## Deliverables

- [ ] Sibling-origin threat model and data-flow diagram covering navigation,
  auth, API, iframe, upload, media delivery, analytics, previews, and failures.
- [ ] Host-only session/CSRF/Origin/Fetch-Metadata policy with rotation,
  logout/revocation, and sibling-subdomain tests.
- [ ] SSR/API data-flow contract proving public anonymous reads, generic
  authenticated/private shells, fixed upstream, bounded failures, and absence
  of cookies/tokens/private DTOs in server-rendered HTML.
- [ ] Canonical/return URL parser and optional code/PKCE handoff contract with
  state, nonce, audience, TTL, replay prevention, and safe errors.
- [ ] Exact CORS matrix for each approved direct-browser endpoint or an explicit
  decision that no portfolio browser endpoint is enabled.
- [ ] Per-route enforcing CSP, Permissions Policy, referrer, framing, sandbox,
  and iframe `allow` matrices for top-level app versus public player.
- [ ] Versioned `postMessage` contract and parent/player implementations with
  origin/source/schema validation.
- [ ] CSP report collection/triage and staged report-only-to-enforcing rollout.

## Acceptance criteria

- [ ] Portfolio navigation works with no shared cookie or token, and Frame
  login/logout/session rotation does not alter portfolio cookies or cached HTML.
- [ ] A cookie issued by Frame is host-only and cannot be sent to the portfolio
  or another sibling through a `Domain` attribute; seeded fixation/replay/
  logout races fail.
- [ ] Cross-origin state changes fail without valid CSRF plus exact Origin/
  Fetch Metadata even though the attacker is a same-site sibling.
- [ ] Redirect parser tests reject open redirects, Unicode/punycode ambiguity,
  userinfo, encoded delimiters, backslashes, port/scheme changes, and nested
  return URLs.
- [ ] Any login handoff code is single-use, short-lived, PKCE/audience/redirect
  bound, absent from logs/referrers/analytics after exchange, and revocable.
- [ ] CORS preflight and actual response tests cover allowed/disallowed origins,
  credentials, methods, headers, null/file origins, redirects, errors, and
  `Vary: Origin`; no wildcard is combined with credentials.
- [ ] Recorder/dashboard/auth pages cannot be framed; the approved public
  player embeds only in exact portfolio origins and leaks no private metadata
  for password/private/deleted/processing/error states.
- [ ] Malicious `postMessage` origin, source window, version, type, oversized
  payload, replay, and confused-deputy cases are ignored and safely logged.
- [ ] Enforcing CSP/Permissions Policy passes supported browsers without
  widening to generic `https:` or enabling capture capabilities on the player.
- [ ] Auth/API/embed responses remain non-cacheable wherever state or privacy
  varies, and cache tests cannot replay one user's result to another.

## Required test evidence

- Threat-model review and cookie/CSRF/sibling-origin penetration matrix.
- Redirect and handoff fuzz/property tests plus replay/expiry traces.
- Browser CORS/preflight/CSP/Permissions Policy matrix.
- Public/private player embed and hostile `postMessage` harness.
- Cache/referrer/log scan proving no credential or private metadata leakage.

## Risks and open questions

- Same registrable domain does not mean same trust; a compromised sibling can
  exploit overly broad cookies, CSP, or redirect allowlists.
- Browser support differs for CSP framing and Permissions Policy; test rather
  than assume header composition.
- Supporting customer custom-domain embeds requires dynamic policy without
  turning `frame-ancestors` into a wildcard.

## Rollout and rollback

Ship top-level navigation with host-only sessions first. Observe CSP reports
before enforcing exact policies. Add optional handoff, browser API, and player
embed as independent flags in that order, each with a kill switch. Rollback
removes the optional capability while preserving top-level Frame navigation
and normal Frame login.

Before closing, attach the threat model, header/cookie matrices, browser and
penetration evidence, CSP report review, and rollback tests.
