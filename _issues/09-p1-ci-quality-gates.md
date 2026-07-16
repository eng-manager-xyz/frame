---
title: "Build CI gates for Rust, Wasm, Leptos, GStreamer, and desktop targets"
labels:
  - "phase:p1"
  - "area:ci"
  - "area:test"
  - "area:ops"
  - "type:foundation"
  - "risk:medium"
depends_on: [05]
size: epic
---

# 09 · Build CI gates for Rust, Wasm, Leptos, GStreamer, and desktop targets

## Outcome

Every change receives fast, target-correct feedback and release-critical paths run on representative operating systems.

## Current Cap reference

The scaffold has a Linux CI job for format, Clippy, workspace tests, wasm32 check, and a synthetic GStreamer artifact. It does not yet cover macOS, Windows, Tauri, D1 migrations, browser behavior, supply chain, or long media tests.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#05](./05-p1-workspace-boundaries-policy.md)

## Scope

Create layered CI for fast PR checks, OS-specific native builds, Wasm/Worker builds, Leptos browser tests, D1 migration tests, R2/Media adapter tests, GStreamer smoke/goldens, Tauri build smoke, security audits, and scheduled soak tests.

### Out of scope

Running hardware/permission tests on generic hosted runners is not required; dedicated/manual lanes must be documented.

## Deliverables

- [ ] A required-check matrix mapped to runtime and platform ownership.
- [ ] Cached, reproducible jobs for Linux, macOS, Windows, wasm32, browser, Worker, and Tauri build smoke.
- [ ] Local D1 migration-from-empty and upgrade-path validation.
- [ ] Required offline tests for the Media port/fake plus an isolated, opt-in or protected-branch remote binding canary with spend, quota, and artifact limits.
- [ ] Artifact retention for logs, media probes, screenshots, test reports, and SBOMs with privacy controls.
- [ ] Flake policy, quarantine rules, timeout budgets, and dedicated-runner plan.

## Acceptance criteria

- [ ] A compile failure in any supported target blocks merge in the owning lane.
- [ ] CI proves native GStreamer and Worker/Wasm dependency separation.
- [ ] Formatting, warnings-as-errors Clippy, tests, migrations, dependency policy, and secret scanning are required.
- [ ] A synthetic media artifact is probed and compared to expected stream/container metadata.
- [ ] The Wasm lane proves the configured `MEDIA` binding detection and adapter compile without importing native GStreamer; the remote lane proves R2 → Media → R2 with a synthetic H.264/AAC fixture.
- [ ] Fork and untrusted PR jobs receive no Cloudflare credentials and cannot trigger billable remote transforms.
- [ ] Intermittent failures cannot be silently retried into green without recorded evidence.

## Required test evidence

- Required-check settings and successful matrix run.
- Deliberate failing changes demonstrating each gate.
- CI duration, cache-hit, and flake baseline.

## Risks and open questions

- A single monolithic workspace job can hide target-specific incompatibility.
- Untrusted media or secrets in artifacts can create a supply-chain/privacy issue.
- Remote-only beta tests can be flaky or billable; they need separate required/advisory policy and a bounded retry budget.

## Rollout and rollback

Add new lanes as advisory, stabilize them, then mark required. Reverting a broken gate requires an owner, incident note, and restoration deadline.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
