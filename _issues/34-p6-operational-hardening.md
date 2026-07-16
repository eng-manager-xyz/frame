---
title: "Harden packaging, deployment, observability, privacy/security, backups, DR, and incident runbooks"
labels:
  - "phase:p6"
  - "area:ops"
  - "area:security"
  - "area:release"
  - "type:release"
  - "risk:high"
depends_on: [09, 10, 17, 21, 29, 30, 31, 32, 33]
size: epic
---

# 34 · Harden packaging, deployment, observability, privacy/security, backups, DR, and incident runbooks

## Outcome

Frame is operable under normal load and failure, with secure releases, actionable telemetry, restorable data, and rehearsed incidents.

## Current Cap reference

The scaffold and migration issues create multiple runtimes and data stores. Without unified release, observability, capacity, backup, security, and response work, distributed failures would be difficult to diagnose or reverse safely.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#09](./09-p1-ci-quality-gates.md), [#10](./10-p1-local-development-stack.md), [#17](./17-p2-shadow-dual-write-cutover.md), [#21](./21-p3-storage-security-lifecycle.md), [#29](./29-p4-media-conformance-performance.md), [#30](./30-p5-rust-api-workflow-parity.md), [#31](./31-p5-leptos-auth-dashboard.md), [#32](./32-p5-leptos-share-player.md), [#33](./33-p5-leptos-desktop-editor-a11y.md)

## Scope

Production build/signing/SBOM/provenance, environments/config/secrets, Worker/D1/R2/Media/native/desktop deploys, release channels, SLO dashboards/alerts, tracing correlation, privacy budgets, audit, capacity/cost/load, backups/restores, DR, incident response, on-call, vulnerability handling, self-hosting, and support diagnostics.

### Out of scope

Final authority cutover and legacy destruction are issue 35.

## Deliverables

- [ ] Reproducible signed release pipelines and promotion policy for every deployable artifact.
- [ ] Service catalog, ownership, SLO/error-budget dashboards, alerts, logs/metrics/traces, and privacy-safe diagnostic bundles.
- [ ] Threat model closure, penetration test, secret/key rotation, SBOM/license/vulnerability gates, and incident contacts.
- [ ] D1 export/backup, object durability/manifest, configuration, signing-key, and project recovery plans with tested restores.
- [ ] Capacity, cost, data-residency, DR, on-call, incident, rollback, self-hosting, and customer-support runbooks.
- [ ] Cloudflare Media beta/change watch, usage/cost budget, quota/outage/output-drift alerts, remote-test controls, provider kill switch, and GStreamer fallback capacity plan.

## Acceptance criteria

- [ ] A release can be reproduced, verified, signed, promoted, rolled back, and traced to source/dependencies.
- [ ] Operators detect and localize seeded Worker, D1, queue/job, object, media-worker, desktop-update, and client failures within approved targets.
- [ ] A Media outage/quota/breaking-output game proves per-profile disablement, native/legacy fallback, deterministic R2 reconciliation, and recovery without duplicate billing or artifacts.
- [ ] Backups restore into an isolated environment and pass referential, object, auth, and playback verification within RPO/RTO.
- [ ] No telemetry or support bundle contains media, tokens, signed URLs, raw email, captions, or unapproved personal data.
- [ ] Load/cost/capacity and regional failure rehearsals remain within charter budgets or produce approved scaling actions.

## Required test evidence

- Release provenance and rollback demonstration.
- Penetration/security review and secret-rotation rehearsal.
- Backup/restore plus disaster-game report and SLO dashboard screenshots.

## Risks and open questions

- A restore that has never been tested is not a backup strategy.
- Correlation IDs can become sensitive if they encode tenant or object identifiers.

## Rollout and rollback

Harden pre-production first, then require release gates. Operational controls must be independently reversible and cannot weaken security for convenience.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
