---
title: "Create cross-executor media conformance, fault, performance, and soak suites"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:cloudflare-media"
  - "area:test"
  - "area:performance"
  - "type:test"
  - "risk:high"
depends_on: [04, 22, 23, 24, 25, 26, 27, 28]
size: epic
---

# 29 · Create cross-executor media conformance, fault, performance, and soak suites

## Outcome

Objective evidence—not anecdote—decides whether the hybrid Cloudflare Media/GStreamer plane is safe to replace Cap paths.

## Current Cap reference

Individual Frame smoke tests pass, while Cap contains a larger recording test system. The migration needs a unified platform/media gate that covers quality, timing, routing, provider limits, recovery, cost, resources, and long-running behavior.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#04](./04-p0-parity-fixtures-baselines.md), [#22](./22-p4-gstreamer-runtime-packaging.md), [#23](./23-p4-gstreamer-pipeline-core.md), [#24](./24-p4-screen-capture.md), [#25](./25-p4-audio-camera-sync.md), [#26](./26-p4-instant-mode.md), [#27](./27-p4-studio-mode.md), [#28](./28-p4-media-service.md)

## Scope

Build fast synthetic tests, cross-executor golden probes and diffs, perceptual metrics, Media capability-limit boundaries, A/V sync/drift, device/permission faults, network/storage/provider/job failures, crash/recovery, codec/hardware/provider fallback, cost/latency/load, repeated lifecycle, long-duration soak, fuzzing, and leak detection.

### Out of scope

Feature implementation belongs to issues 22–28; this issue can block release but must not silently alter approved parity budgets or vendor capability policy.

## Deliverables

- [ ] Media test matrix derived from charter platforms, sources, modes, codecs, containers, resolutions, executors, capability limits, and failure scenarios.
- [ ] Deterministic harness and artifact manifest with tool/runtime/hardware/provider/profile provenance.
- [ ] Offline Media fake tests and an isolated, budgeted remote binding lane using immutable synthetic fixtures.
- [ ] Objective metadata, frame/audio, perceptual, sync, performance, cost, resource, routing, and recovery comparators.
- [ ] Dedicated-runner/manual plan for hardware and permission cases plus scheduled remote, soak, and fuzz lanes.
- [ ] Release dashboard showing executor, baseline, budget, result, trend, flake, cost/usage, and evidence link.

## Acceptance criteria

- [ ] Golden recordings pass on every declared OS/device class within issue 01 budgets.
- [ ] Cloudflare Media and GStreamer outputs for overlapping profiles pass declared metadata/playback/perceptual tolerances without requiring byte identity.
- [ ] Exact and just-over Media size, duration, resolution, format, quota, and timeout boundaries select the expected executor or fail before invocation.
- [ ] Injected device loss, process crash, disk full, network loss, provider error, cancellation, and unsupported codec produce the specified recovery or failure.
- [ ] Provider outage, quota, or output drift triggers the declared fallback or kill switch, preserves one logical R2 result, and leaves reconcilable D1 state.
- [ ] Hardware paths and software fallback both pass quality and compatibility tests.
- [ ] Repeated start/stop and long-duration runs remain within memory, handle, thread, disk, drift, CPU/GPU, temperature, latency, and cost budgets.
- [ ] Seeded quality, sync, recovery, resource, routing, provider-limit, managed-output-drift, fallback, and repeat-cost regressions each fail the correct gate.

## Required test evidence

- Full cross-executor matrix report, remote Media usage/cost record, and representative artifacts.
- At least one completed long-duration soak per supported native platform.
- Fuzz corpus/crash triage, provider-fault traces, deterministic-key/idempotency evidence, and leak/resource trends.

## Risks and open questions

- Hosted CI cannot reproduce all hardware/permission behavior.
- Normal local CI cannot simulate the Media binding, while remote tests introduce beta drift, credentials, latency, quota, and cost.
- Perceptual scores alone can miss timing, cursor, edit, routing, or recovery regressions.

## Rollout and rollback

Run advisory during implementation, then make the fast/offline suite required and the native plus remote matrices release gates. Waivers need an owner, user impact, expiry, cost/security review, and rollback.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
