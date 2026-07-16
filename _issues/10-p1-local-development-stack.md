---
title: "Create a reproducible local stack for D1, object storage, GStreamer, and seed data"
labels:
  - "phase:p1"
  - "area:developer-experience"
  - "area:ops"
  - "type:foundation"
  - "risk:medium"
depends_on: [03, 05, 09]
size: epic
---

# 10 · Create a reproducible local stack for D1, object storage, GStreamer, and seed data

## Outcome

A clean machine can boot a representative Frame environment with one documented command and no production credentials.

## Current Cap reference

Cap has local Docker MySQL/MinIO infrastructure. Frame has native binaries and a Wrangler configuration but no integrated workerd/Miniflare D1/object environment, queue substitute, seeded users/videos, or service orchestration.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#03](./03-p0-runtime-topology.md), [#05](./05-p1-workspace-boundaries-policy.md), [#09](./09-p1-ci-quality-gates.md)

## Scope

Automate tool checks, local Worker bindings, D1 migrations, object storage, job transport, native media worker, Leptos app, seed/reset, observability, and representative failure toggles.

### Out of scope

Perfect emulation of Cloudflare production or real hardware capture is not expected locally.

## Deliverables

- [ ] A doctor command that validates Rust targets, Wrangler/worker-build, GStreamer version/plugins, browser tooling, ports, and optional Tauri dependencies.
- [ ] One-command start, stop, reset, migrate, seed, and log-tail workflows.
- [ ] Deterministic seed tenants, videos, jobs, comments, permissions, and object manifests.
- [ ] Local secrets templates with safe defaults and explicit production-secret refusal.
- [ ] A troubleshooting guide for each supported development OS.

## Acceptance criteria

- [ ] A clean checkout reaches healthy web, Worker, D1, object storage, and media worker services using only documented steps.
- [ ] Reset removes local data and reapplying every migration from empty succeeds.
- [ ] The synthetic walking slice completes locally and leaves a playable object plus consistent D1 rows.
- [ ] Port collisions, missing GStreamer plugins, stale migrations, and absent bindings produce actionable errors.
- [ ] No checked-in or generated default can address a production database or bucket.

## Required test evidence

- Clean-machine transcript for Linux, macOS, and Windows or documented supported subset.
- Local end-to-end test output.
- Doctor output for both passing and intentionally missing prerequisites.

## Risks and open questions

- Local emulators differ from D1/R2 production semantics.
- Cross-platform scripts can become a second build system.

## Rollout and rollback

Development-only. Each service remains independently runnable if orchestration fails; reset is confined to namespaced local state.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
