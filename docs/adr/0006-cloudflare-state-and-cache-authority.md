# ADR 0006: Separate shared-zone and Frame account authority

- Status: accepted
- Date: 2026-07-16

## Context

Cloudflare rulesets have one phase entrypoint per zone. Independently managing
two host-scoped rulesets for the same phase can replace unrelated portfolio,
shop, Stripe, or Frame rules. Frame also needs account-scoped R2 CORS and
lifecycle resources that do not belong in the shared zone state.

## Decision

The portfolio repository's `infra/cloudflare-zone` state is the sole owner of
the `engmanager.xyz` CNAME/TLS settings and complete cache, WAF, and rate-limit
phase entrypoints. It must import every existing rule, prove a semantic no-op,
and retire competing bootstrap writes before adding Frame entries. Frame keeps
only the machine-readable `infra/cloudflare-zone/frame-contract.json` handoff,
including its `/api*` and `/media-server*` Worker Route set; it cannot apply
shared-zone mutations.

Frame's `infra/cloudflare-account` is a disjoint state that owns the private
recordings bucket, exact-origin CORS, and abandoned multipart lifecycle only.
Wrangler owns Worker code, bindings, D1, Media, and the reviewed `/api*` and
`/media-server*` routes; it does not own shared cache/WAF phase rules. Each
state uses a pinned provider and lockfile, encrypted locked/versioned remote backend, least-privilege token,
protected manual apply, drift detection, backup, and restore rehearsal.

Origin response headers are the first cache authority. All API, auth, health,
mutation, private, cookie, authorization, signed, range, and upgrade traffic
is bypassed. Public HTML remains bypassed initially. Only exact fingerprinted
assets with the one-year immutable origin header are eligible. Purge operations
accept only exact canonical Frame URLs or `frame:` tags; a whole-zone purge is
not expressible by the tool.

Frame-scoped WAF and independent auth/OTP/upload/comment/view/job rate budgets
start in observe mode. Enforcement requires a false-positive review and an
expiring exception owner. User-controlled identifiers are never the sole key.

## Consequences

The design prevents competing phase ownership and contains the blast radius of
Frame rollback, at the cost of a cross-repository contract and protected import
sequence. Provider plans, semantic-equivalence evidence, cache HIT traces, and
state restore remain required before promotion; static configuration does not
stand in for those observations.
