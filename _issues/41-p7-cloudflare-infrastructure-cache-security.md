---
title: "Codify Cloudflare DNS, cache, WAF, rate limits, and R2 browser policy"
labels:
  - "phase:p7"
  - "area:cloudflare"
  - "area:r2"
  - "area:security"
  - "area:ops"
  - "type:infrastructure"
  - "risk:critical"
depends_on: [19, 21, 32, 39, 40]
size: epic
---

# 41 · Codify Cloudflare DNS, cache, WAF, rate limits, and R2 browser policy

## Outcome

Frame's Cloudflare edge and browser-to-R2 behavior are reproducible,
least-privilege, and isolated from the existing portfolio/shop rules; private
or authenticated responses can never become shared-cache hits.

## Current reference

The portfolio manages zone records and cache rules with shell scripts. Its
bootstrap assumes one Render hostname for apex, `www`, and shop, and currently
contains a whole-shop-host cache rule. Its purge script can fall back to a
zone-wide purge. Reusing those defaults for Frame would risk routing Frame to
the wrong service, caching auth/API/share responses, or flushing unrelated
portfolio content.

Frame configures D1, a private R2 bucket, and Media Transformations in Wrangler
but has no production DNS, cache, WAF, rate-limit, R2 CORS, or infrastructure-
state policy. Issues 19/21/32 define storage, privacy, and player semantics that
the edge must preserve.

## Dependencies

[#19](./19-p3-multipart-upload-download.md),
[#21](./21-p3-storage-security-lifecycle.md),
[#32](./32-p5-leptos-share-player.md),
[#39](./39-p7-cloudflare-render-same-origin-routing.md), and
[#40](./40-p7-github-actions-multiruntime-delivery.md)

## Scope

Complete the single infrastructure-as-code authority established by issue 39
for the shared `engmanager.xyz` zone, then define the separate Frame account/
bucket resource boundary. Migrate every existing zone phase entrypoint before
adding Frame cache, WAF, or rate-limit rules. Define remote state,
least-privilege tokens, drift detection, resource naming, cache classification,
direct private-R2 uploads, scoped purge, security rollout, and retirement of
competing portfolio script writes.

Use the Cloudflare Terraform provider unless an ADR demonstrates equivalent
plan, state, import, drift, review, serialization, and rollback guarantees.
Cloudflare has one zone entrypoint ruleset per phase; a host-scoped expression
is not an independently ownable ruleset. The initial owner is the portfolio's
zone-infrastructure directory because that repository already manages the zone,
but its scripts must be imported/retired rather than racing Terraform. Keep
Worker source/bindings/routes owned by Wrangler; do not manage the same Worker
script in Terraform.

Use two disjoint states/directories with no overlapping resource IDs:

```text
engmanager.xyz/infra/cloudflare-zone/
├── versions.tf
├── providers.tf
├── variables.tf
├── dns.tf             # all declared zone records, including exact frame CNAME
├── cache.tf           # one phase entrypoint: portfolio/shop + Frame rules
├── security.tf        # one entrypoint per WAF/rate-limit phase
├── outputs.tf
└── README.md          # import, ordering, apply, drift, rollback

frame/infra/cloudflare-account/
├── versions.tf
├── providers.tf
├── variables.tf
├── r2-cors.tf         # Frame private-bucket browser policy only
├── outputs.tf
└── README.md
```

Import every existing zone phase entrypoint and model its portfolio/shop rules
before the first authoritative apply. Produce a no-op/semantic-equivalence
plan, then disable the corresponding PUT behavior in
`scripts/cloudflare-bootstrap.sh`. Never create a second
`cloudflare_ruleset` for the same zone and phase.

### Resource ownership

- The portfolio zone state owns the exact `frame` CNAME plus the entire
  entrypoint ruleset for every zone phase it manages. Frame expressions are
  named/host-scoped entries inside those authoritative ordered rulesets.
- The Frame account state owns only disjoint Frame bucket/account resources
  such as R2 CORS; it cannot manage zone phase entrypoints or the CNAME.
- Wrangler owns the Frame Worker code, D1/R2/Media bindings, environments, and
  the broad `/api*` Worker Route.
- The portfolio's apex, `www`, shop/store, and existing cache semantics are
  imported into the zone state and regression-tested before the legacy script
  stops mutating the phase entrypoint.
- Zone-wide TLS mode is asserted as Full (strict) and changed by one named
  owner; Frame must not create a second competing setting resource.

State is remote, encrypted, access-controlled, backed up, and locked. Provider
and module versions plus the dependency lockfile are committed. Pull requests
run format/validate and a redacted plan only in trusted contexts; production
apply is manual, protected, serialized, and auditable.

### Cache classification

**Always bypass:** `/api`, auth/session/account, upload/finalize, health,
WebSocket/SSE, non-GET/HEAD methods, private/password/deleted shares, signed
URLs, personalized dashboard HTML, responses with authorization/session
cookies, and any response classified `private` or `no-store`.

**Eligible only after review:** anonymous landing HTML and explicitly public
share/player HTML with cache keys that cannot mix auth/privacy/host variants,
bounded TTLs, deletion/privacy purge SLO, and no `Set-Cookie`.

**Long-lived:** content-addressed/fingerprinted static assets with
`public, max-age=31536000, immutable`. Mutable paths never receive that policy.

Tests inspect `CF-Cache-Status`; dynamic/private requests must be
`DYNAMIC`/`BYPASS`, never `HIT`. Purges use exact Frame URLs or namespaced cache
tags and never the portfolio's zone-wide purge fallback.

Keep Cloudflare Origin Cache Control enabled where supported and preserve the
origin's `private`, `no-store`, `no-cache`, `Set-Cookie`, and authorization
safety behavior. Cache Rules and Cache Response Rules must not set an Edge TTL
or eligibility override that converts those responses into shared-cache
objects. Test the effective policy, not only the intended expression order.

### Direct R2 browser exchange

Raw recordings remain private. The Worker authorizes an upload intent and
returns a short-lived, method/content-type/random-staging-key-scoped presigned
PUT; browsers upload directly to R2 and finalize through the API. Presigned
URLs are bearer credentials and can be reused until expiry, so one-use or
pre-storage byte-limit behavior is not claimed. Authenticated idempotent
finalize validates tenant intent, declared and observed length, checksum,
content type, quota, and state before promoting/referencing an immutable
canonical key. Invalid/unfinalized staging objects are quarantined or deleted
by a bounded lifecycle/reconciler. R2 CORS admits
only approved Frame production/staging/local origins, exact methods/headers,
and the minimum exposed response headers. Portfolio origin access is absent
unless a concrete browser flow is approved.

If a hard maximum must be enforced before bytes land in R2, select and prove a
stateful upload broker or a multipart design with enforceable part/total
limits; presigned PUT alone cannot supply that guarantee.

Large media bodies never transit Render. Public R2 custom domains and public
buckets are disabled by default; presigned S3 URLs do not work on a custom
domain. Authorized Worker delivery or short-lived approved endpoints retain
privacy, range, and revocation semantics.

### Edge security

Apply managed WAF/rate-limit rules first in log/challenge mode, then block only
after false-positive review. Scope auth, OTP, upload-intent/finalize, comment,
view/reaction, and job mutation budgets independently by trusted signals.
Never use a user-controlled identifier as the sole limiter. Preserve WebSocket
and SSE upgrades where retained and verify idle/reconnect behavior.

Cloudflare Access may protect staging/admin/diagnostic routes, not the public
Frame application or unauthenticated share player. If Access protects an API,
explicitly test CORS preflights.

### Out of scope

- Applying a zone phase before all existing portfolio/shop rules in that phase
  have been imported and proven semantically equivalent.
- Making the R2 bucket public to simplify playback.
- Caching authenticated HTML because it appears visually static.
- Committing API tokens, state, account IDs considered sensitive, signed URLs,
  or real media to plans/artifacts.

## Deliverables

- [ ] Infrastructure ownership ADR/inventory, two disjoint remote-state
  designs, pinned provider lockfiles, imports, naming/tags, and drift policy.
- [ ] Issue-39 CNAME/proxy state represented by the authoritative zone owner,
  plus exact-host CAA/AAAA/TLS checks and no manual/IaC duplicate.
- [ ] Import and semantic-equivalence evidence for every existing portfolio/
  shop rule in each managed zone phase, followed by retirement of competing
  script PUTs.
- [ ] Route/method/privacy/cache matrix implemented as origin headers plus
  Cloudflare Cache Rules with automated assertions.
- [ ] Frame-scoped WAF/rate-limit rules, staged enforcement, exception expiry,
  and abuse dashboards.
- [ ] Private R2 CORS and presigned upload/finalize configuration with browser
  contract tests.
- [ ] URL/cache-tag purge tool and privacy-change/deletion purge runbook that
  cannot purge the entire zone accidentally.
- [ ] Least-privilege token scope, protected plan/apply workflow, drift alert,
  backup/restore, and emergency rollback documentation.

## Acceptance criteria

- [ ] The first post-import zone plan is no-op or has only reviewed
  representation changes; apex, `www`, shop/store, portfolio cache semantics,
  and unrelated Workers remain behaviorally unchanged.
- [ ] Exactly one state/resource owns each zone phase entrypoint, and running
  the retired portfolio bootstrap cannot overwrite the authoritative ruleset.
- [ ] DNS can move from DNS-only to proxied and back without replacing the
  Render domain or losing the last-known-good state.
- [ ] Full (strict), certificate renewal eligibility, HTTP-to-HTTPS, exact
  `AAAA` absence, and CAA compatibility pass without a wildcard record.
- [ ] Auth/API/private/upload/health/mutation tests never return a cache HIT or
  leak `Set-Cookie`, authorization, private metadata, signed URLs, or object
  keys across users.
- [ ] Origin Cache Control and application `private`/`no-store` remain
  effective; no Edge TTL/Cache Response override makes a seeded private or
  `Set-Cookie` response eligible.
- [ ] Fingerprinted assets become Cloudflare HITs with immutable headers, and
  changing a fingerprint cannot serve the previous bytes.
- [ ] Public-to-private, password, or deletion changes purge every approved
  cache variant inside issue 21's SLO without a zone-wide purge.
- [ ] Direct PUT is limited to a random authorized staging key/method/type/
  expiry. Reuse before expiry cannot overwrite a finalized canonical object or
  change tenant state; wrong-origin/header, listing, canonical overwrite, and
  cross-tenant attempts fail.
- [ ] Finalize rejects/quarantines wrong size/checksum/type/quota/state,
  succeeds idempotently once for valid content, and a lifecycle/reconciler
  bounds cost from abandoned or oversized staging objects.
- [ ] WAF/rate limits block seeded abuse after observation without breaking
  normal auth, upload, range playback, WebSocket/SSE, or portfolio navigation.
- [ ] Untrusted PRs cannot read provider/state credentials or run production
  plan/apply, and drift is detected without automatic destructive correction.
- [ ] A state restore and provider-rule rollback are rehearsed and do not
  delete or expose media.

## Required test evidence

- Redacted infrastructure plans before/after staged proxy activation.
- Cache matrix with repeated `CF-Cache-Status`, cookie, auth, range, privacy,
  deletion, and host variants.
- Browser direct-upload/CORS abuse matrix and no-Render-body trace.
- WAF/rate-limit observation and enforcement report.
- Drift, remote-state restore, scoped purge, and rollback rehearsal.

Local direct-upload implementation and adversarial SQLite evidence are tracked
in [`docs/evidence/direct-upload-local.md`](../docs/evidence/direct-upload-local.md).
That report deliberately leaves hosted R2 SigV4, checksum, CORS denial, and
provider lifecycle behavior unchecked until protected credentials and a
non-production bucket are available.

## Risks and open questions

- A Terraform ruleset is authoritative for its whole phase, so incomplete
  import can delete portfolio/shop rules; semantic-equivalence and one owner
  are hard prerequisites.
- Rule ordering can turn an intended bypass into a later cache-eligible match.
- Cloudflare plan capabilities vary; record required plan/cost before relying
  on a managed rule or advanced rate-limit expression.
- Provider beta/API changes, especially Media Transformations, require version
  watch and fallback from issues 29/34.

## Rollout and rollback

Import/declare staging resources first. In production, import each complete
zone phase, prove a no-op/semantic-equivalent plan, disable the legacy script
write, and only then add Frame rules. Start cache/security in bypass/observe
mode, then immutable caching, then WAF blocking. Roll back the smallest entry
inside the one authoritative phase; never restore a legacy whole-ruleset PUT
that discards newer entries. Preserve both states and audit evidence
throughout.

Before closing, attach ownership/state review, redacted plans, cache/upload/WAF
evidence, cost decision, and rollback report.
