---
title: "Build protected GitHub Actions delivery for Worker, Render, and contracts"
labels:
  - "phase:p7"
  - "area:ci"
  - "area:cloudflare"
  - "area:render"
  - "area:security"
  - "type:deployment"
  - "risk:critical"
depends_on: [09, 34, 36, 38, 39]
size: epic
---

# 40 · Build protected GitHub Actions delivery for Worker, Render, and contracts

## Outcome

One commit receives target-correct tests and a controlled production release:
Cloudflare deploys only from protected GitHub Actions, Render deploys exactly
once after checks pass, secrets never reach untrusted code, and every release
has compatibility and rollback evidence.

## Current reference

Frame has one CI workflow for Linux Rust, wasm32, Wrangler dry-run, and a
synthetic GStreamer artifact. It does not validate Render configuration,
exercise `frame-client` consumers, deploy a Worker, apply remote D1 migrations,
or track deployment environments. The portfolio has no GitHub Actions workflow
at the pinned snapshot.

Render supports CI-gated Git-linked deployment through
`autoDeployTrigger: checksPass`; deploy hooks are secret URLs and would create
a second authority if enabled at the same time. Cloudflare recommends Wrangler
deployment from GitHub Actions with a scoped API token.

## Dependencies

[#09](./09-p1-ci-quality-gates.md),
[#34](./34-p6-operational-hardening.md),
[#36](./36-p7-frame-client-public-contract.md),
[#38](./38-p7-render-web-runtime-blueprint.md), and
[#39](./39-p7-cloudflare-render-same-origin-routing.md)

## Scope

Extend CI, add a protected Cloudflare production deployment workflow, gate
Render with checks-pass auto-deploy, define D1/API compatibility ordering,
validate Blueprint and infrastructure plans, add deployment concurrency and
provenance, run domain smoke tests, and specify the companion portfolio
consumer workflow.

### Deployment authority

- **Cloudflare Worker/D1 migrations:** GitHub Actions using a pinned Wrangler
  version/action and the `production` environment. Cloudflare Workers Builds
  Git integration is disabled so it cannot become a second Worker authority.
- **Render web:** Render Git integration with
  `autoDeployTrigger: checksPass`; no deploy hook and no parallel API trigger.
- **Cloudflare DNS/cache/WAF:** protected manual apply from issue 41's single
  designated zone-infrastructure repository/state; never an automatic mutation
  from an untrusted PR or a competing Frame-local zone ruleset.
- **Portfolio:** its own repository checks and Render service.

If exact-SHA Render waiting/post-deploy orchestration becomes mandatory,
replace—not supplement—checks-pass deployment with a protected Render CLI/API
workflow and record the authority migration.

### Required workflow layers

1. Fast PR checks: format, Clippy, unit/contract tests, wasm check, lockfile,
   forbidden dependency graphs, secret/license/vulnerability checks.
2. Target/build checks: Worker dry-run, `frame-web` release build and
   `PORT` smoke, GStreamer lane, `frame-client` feature/wasm matrix, Blueprint
   validation, infrastructure fmt/validate, and route/cache contract tests.
3. An always-present `production-gate` job is created immediately on every
   `main` push SHA with no event/job-level path filter. It evaluates the diff
   inside the same push's immediately-created check set and cannot enter the
   protected provider step until every named CI/build/security dependency
   succeeds. It performs
   the expand-only D1 migration, Worker deploy, and API smoke when Worker/shared
   contracts changed, or explicitly verifies the compatible current Worker
   when they did not. Its final sentinel always ends `success` or `failure`,
   never `skipped`/`neutral`; a delayed `workflow_run` is not the sentinel.
4. Render checks-pass auto-deploy considers all checks detected for the commit,
   not merely branch-protection required checks. Because Render counts
   `success`, `neutral`, and `skipped` as passing, the non-skippable
   `production-gate` is the semantic release sentinel. Render's `buildFilter`
   may correctly suppress commits with no web/shared impact.
5. Independent synthetic verification watches the canonical domain and creates
   an actionable failed deployment/incident if Render never reaches the paired
   release.

The intended repository layout is explicit so deploy authority is reviewable:

```text
.github/workflows/
├── ci.yml                 # unprivileged required checks
├── production-gate.yml    # every main SHA -> evaluate -> Worker/verify -> gate
├── cloudflare-account.yml # disjoint Frame R2/account plan/apply only
└── production-smoke.yml   # scheduled/manual canonical-domain verification
render.yaml                # Render checks-pass authority; no deploy hook

engmanager.xyz/.github/workflows/
└── cloudflare-zone.yml    # one zone-phase owner; trusted plan/manual apply
```

The portfolio receives its own unprivileged `.github/workflows/ci.yml`; it does
not call these privileged workflows or inherit their secrets.

Worker/API changes must remain compatible with the preceding web client; the
new Worker can be live while Render still serves N-1. D1 migrations follow
expand/migrate/contract and never make rollback depend on an immediate
destructive schema change.

### Secrets and permissions

Use least-privilege workflow permissions, pinned third-party action SHAs,
environment-scoped `CLOUDFLARE_API_TOKEN`/account identifiers, and protected
review/branch rules. Fork and untrusted PR jobs receive no provider token,
deploy hook, production endpoint credential, Terraform state secret, signed
URL, or media fixture with personal data. Logs and artifacts follow issue 34's
redaction/retention policy.

### Out of scope

- Deploying on every PR or giving previews production D1/R2 access.
- Running production Media Transformations from untrusted CI.
- Maintaining both Render auto-deploy and a deploy-hook/API path.
- A repository-scoped token capable of changing unrelated
  `engmanager.xyz` zone resources.
- Atomic cross-repository releases; use compatible contracts and paired
  release records instead.

## Deliverables

- [ ] Required-check graph and path filters mapped to native, wasm, Render,
  Worker, client-contract, and infrastructure owners.
- [ ] Extended CI for `frame-client`, production-mode web smoke, Blueprint
  validation, Worker route dry-run, lockfiles, dependency boundaries, and
  supply-chain checks.
- [ ] Same-push, always-present, non-skippable `production-gate` with pinned
  Wrangler/action, change evaluation, production concurrency, migration
  ordering, Worker deploy-or-verify, smoke, and rollback metadata.
- [ ] Render checks-pass/build-filter configuration and evidence that one
  successful web-impacting SHA creates one Render deployment trigger, a
  non-web filtered SHA creates none, and failed/zero-check SHAs create none.
- [ ] Cloudflare Workers Builds Git integration disabled and an authority audit
  proving no second Worker or Render deploy trigger exists.
- [ ] Companion portfolio CI specification: pinned nightly, committed lockfile,
  all Rust/router/golden tests, client fixture compatibility, and no production
  secret on pull requests.
- [ ] Release manifest joining Git SHA, Worker version, D1 migration level,
  Render deploy/commit, contract major, and portfolio consumer SHA.
- [ ] Failure notification, retry, rollback, and compromised-token runbooks.

## Acceptance criteria

- [ ] A failing format, test, contract, Worker dry-run, Blueprint, dependency,
  or security gate prevents both Worker and Render production deployment.
- [ ] Every main SHA immediately receives the non-skippable production gate. A
  relevant successful commit applies only approved expand-first migrations,
  deploys the Worker, and passes `/api/v1/health`; an irrelevant commit records
  why no Worker deploy was needed and verifies the compatible current version.
- [ ] A successful web/shared-impact SHA causes one Render trigger with no
  duplicate hook/API/Workers-Builds authority. A build-filtered non-web SHA
  causes no Render deploy, and the release record explains the suppression.
- [ ] Render never deploys before the production gate completes, and the newly
  deployed or verified Worker remains compatible with the live N-1 web binary.
- [ ] Fork/untrusted PRs cannot access or exfiltrate any Cloudflare, Render,
  infrastructure-state, R2, Media, or portfolio deployment secret.
- [ ] Workflow concurrency prevents two production releases or migrations from
  interleaving; cancellation cannot leave the migration/API state ambiguous.
- [ ] Actions and toolchains are pinned, lockfiles are enforced, generated
  artifacts have provenance/SBOM as required by issue 34, and logs contain no
  media, signed URL, cookie, token, or private DTO.
- [ ] A deliberate Worker failure leaves Render at the prior release; a Render
  failure leaves the compatible Worker live and raises an actionable alert.
- [ ] Rollback restores the previous Worker/web pair without reverting a
  destructive D1 change, and the release manifest identifies the result.
- [ ] Portfolio contract tests pass against current fixtures and a deliberate
  incompatible major version fails before deployment with a useful message.
- [ ] Seeded skipped/neutral/mis-filtered Worker jobs and a delayed
  `workflow_run` design cannot satisfy the production sentinel; changing path
  filters or check names fails the authority test.

## Required test evidence

- Successful and deliberately failed workflow runs for every required gate.
- Secret-boundary proof from fork/untrusted PR simulation.
- N/N-1 staged deployment and D1 forward/backward compatibility test.
- Per-SHA Render trigger/build-filter trace and independent domain smoke result.
- Worker, Render, and portfolio rollback rehearsal tied to a release manifest.

## Risks and open questions

- Render detects all commit checks dynamically and treats skipped/neutral as
  passing; protect the always-present gate name/creation semantics and test
  zero-check, skipped, neutral, and build-filter behavior.
- Deploying a Worker before Render is safe only with N/N-1 API compatibility.
- Environment approvals can leave a checks-pass Render deploy waiting; document
  expected pending behavior and cancellation.
- Remote beta Media tests are billable/flaky and must remain isolated from the
  normal deploy authority.

## Rollout and rollback

Make new CI lanes required before adding credentials. Dry-run the production
workflow against staging, then enable the protected Worker deployment, then
enable Render checks-pass. Retain manual provider rollback during the
observation window. Any move to Render CLI/API delivery removes the old
authority in the same controlled change.

Before closing, attach environment/branch settings, workflow runs, release
manifest, secret review, provider deploy traces, and rollback evidence.
