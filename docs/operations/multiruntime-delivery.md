# GitHub, Render, and Cloudflare delivery authority

## One authority per target

| Target | Sole release authority | Forbidden duplicate |
| --- | --- | --- |
| Worker and expand-only D1 migrations | Explicit `production-gate.yml` dispatch from `main`, protected `production` GitHub environment | Automatic push release, Cloudflare Workers Builds Git integration, dashboard deploy, another workflow |
| Render web | Render Git integration with `autoDeployTrigger: checksPass` and the committed build filter | deploy hook, Render API/CLI workflow, manual build from a different SHA |
| Frame Cloudflare account/R2 | protected manual `cloudflare-account.yml` plan/apply against isolated remote state | shared-zone resources or an unreviewed local state |
| Shared `engmanager.xyz` zone | portfolio repository's designated zone workflow/state | Frame workflow, Wrangler zone mutation, dashboard-only rules |
| Portfolio service | portfolio repository CI and its Render service | Frame credentials or workflow reuse |

Before enabling production, an owner records screenshots/exports proving
Workers Builds is disabled, no Render deploy hook exists, environment reviewers
and branch protection are active, and the shared zone has one state owner.
Provider settings are protected evidence; repository policy cannot fabricate
them.

## Per-SHA release graph

Every pull request and every main SHA creates `Production gate` without path
filters. Both events run the same secret-free preflight and immutable release
build: policy, format, lint, test, contract, migration, fixture, dependency,
supply-chain, native/wasm, browser, media, compiled Worker binding, and release-
package checks. The Worker is built before bounded D1/R2 readiness probes, so a
cold compiler cannot consume a service-readiness timeout. Branch protection
must require the final `production-gate` context before merge.

Pull requests and main pushes validate exactly that shared build path. The
required `production-gate` sentinel depends only on impact evaluation,
preflight, and immutable build, and resolves those same three results for both
events. Protected provider evidence cannot turn the required main build check
red. A checksummed handoff binds the Git SHA, API major, D1 migration level, web
assets/binary, Worker bundle, cargo metadata, and SBOM.

Provider release is a distinct manual lane. It runs only for an explicit
`workflow_dispatch` from `refs/heads/main`, waits for the same run's successful
`production-gate`, and consumes that run's verified handoff. Pull requests,
automatic main pushes, and non-main dispatches cannot enter the protected
environment. The protected job requires the complete parity corpus before any
provider access or mutation. A Worker-impacting SHA then applies only
expand-first migrations, deploys the prebuilt Worker, and smokes the canonical
API; an irrelevant SHA performs the same compatibility smoke and records why
no Worker deploy occurred. The independent `provider-release-gate` uses
`always()` and fails the manual run unless both the build gate and protected job
succeed. Production concurrency is serialized and is never cancelled
mid-migration.

Render sees that same commit's checks. A web/shared-impacting SHA triggers one
checks-pass build; a correctly filtered SHA triggers none. The secret-free
scheduled/post-gate canonical smoke waits for the paired Render release and
raises a failed check if it never arrives. There is no second Render trigger.

## Compatibility and failure behavior

The provider and Render lanes are independently triggered, so either compatible
half can finish first. Worker/D1 releases must support the current and
immediately preceding web/client contract, and a web release may not require a
new Worker until its explicit provider dispatch succeeds. D1 changes are
expand/migrate/contract; rollback deploys compatible code and never reverses a
destructive migration. A failure in either lane leaves the other compatible
release live and fails the corresponding release or canonical-smoke check.

Build retries create a new, attributable run against the same immutable SHA;
provider retries require another explicit dispatch from `main`. Never hide a
first failure with `continue-on-error`, a neutral conclusion, or a renamed
check. The release record joins Worker version, Render deploy/commit, contract
major, migration level, portfolio consumer SHA when present, outcome, and
previous compatible SHAs.

## Compromised credential response

1. Disable the exact release authority and revoke the environment-scoped token.
2. Audit token capabilities, GitHub environment access, workflow changes,
   Cloudflare deployments/logs, D1 migrations, and account/zone state without
   printing secret material.
3. Rotate the token with the documented least-privilege scope, update only the
   protected environment, and invalidate any exposed backend/state credential.
4. Rebuild from a reviewed commit; do not reuse an artifact or runner workspace
   touched after suspected compromise. Verify release checksums and provenance.
5. Restore the preceding compatible Worker/web pair if integrity is uncertain,
   run canonical privacy/security smoke, and attach a redacted incident record.

## Companion portfolio CI specification

The portfolio repository owns a separate unprivileged pull-request workflow.
It installs its pinned nightly toolchain, checks the committed root lockfile,
runs format/Clippy/all workspace and router/golden tests, compiles the exact
40-character `frame-client` Git revision, and consumes current plus last-
released v1 fixtures. An additive fixture must pass; an incompatible major must
fail with `incompatible_version`. Pull requests receive no Frame, Cloudflare,
Render, zone-state, or portfolio deployment secret.

The consumer workflow also proves request handlers never perform Frame I/O,
Frame failure preserves the static project link and last-known-good snapshot,
and only approved public DTO fields enter HTML. Its actual workflow and locked
SHA are cross-repository protected evidence.
