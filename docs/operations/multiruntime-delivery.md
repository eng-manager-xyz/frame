# GitHub, Render, and Cloudflare delivery authority

## One authority per target

| Target | Sole release authority | Forbidden duplicate |
| --- | --- | --- |
| Worker and expand-only D1 migrations | `production-gate.yml`, protected `production` GitHub environment | Cloudflare Workers Builds Git integration, dashboard deploy, another workflow |
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

Pull requests validate only the shared build path; the provider job is
structurally unreachable and the final sentinel accepts that one explicit
`skipped` result. Main and manual events require provider success. A checksummed
handoff binds the Git SHA, API major, D1 migration level, web assets/binary,
Worker bundle, cargo metadata, and SBOM.

The protected job first requires the protected parity corpus, then verifies the
handoff. A Worker-impacting SHA applies only expand-first migrations, deploys
the prebuilt Worker, and smokes the canonical API. An irrelevant SHA performs
the same compatibility smoke and records why no Worker deploy occurred. The
final `production-gate` job uses `always()` and converts every dependency
result into success or failure; skipped or neutral provider work cannot satisfy
a main/manual release. Production concurrency is serialized and is never
cancelled mid-migration.

Render sees that same commit's checks. A web/shared-impacting SHA triggers one
checks-pass build; a correctly filtered SHA triggers none. The secret-free
scheduled/post-gate canonical smoke waits for the paired Render release and
raises a failed check if it never arrives. There is no second Render trigger.

## Compatibility and failure behavior

The Worker/D1 half releases before the web half and must support the current
and immediately preceding web/client contract. D1 changes are expand/migrate/
contract; rollback deploys compatible code and never reverses a destructive
migration. Worker failure leaves Render blocked at the prior release. Render
failure leaves the compatible Worker live and fails canonical smoke.

Retries create a new, attributable run against the same immutable SHA. Never
hide a first failure with `continue-on-error`, a neutral conclusion, or a
renamed check. The release record joins Worker version, Render deploy/commit,
contract major, migration level, portfolio consumer SHA when present, outcome,
and previous compatible SHAs.

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
