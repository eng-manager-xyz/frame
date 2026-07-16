---
title: "Create golden-media, fault-injection, hardware-fallback, performance, and soak suites"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:test"
  - "area:performance"
  - "type:test"
  - "risk:high"
depends_on: [04, 22, 23, 24, 25, 26, 27, 28]
size: epic
---

# 29 · Create golden-media, fault-injection, hardware-fallback, performance, and soak suites

## Outcome

Objective evidence—not anecdote—decides whether the GStreamer media plane is safe to replace Cap paths.

## Current Cap reference

Individual Frame smoke tests pass, while Cap contains a larger recording test system. The migration needs a unified platform/media gate that covers quality, timing, recovery, resources, and long-running behavior.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#04](./04-p0-parity-fixtures-baselines.md), [#22](./22-p4-gstreamer-runtime-packaging.md), [#23](./23-p4-gstreamer-pipeline-core.md), [#24](./24-p4-screen-capture.md), [#25](./25-p4-audio-camera-sync.md), [#26](./26-p4-instant-mode.md), [#27](./27-p4-studio-mode.md), [#28](./28-p4-media-service.md)

## Scope

Build fast synthetic tests, golden probes/diffs, perceptual metrics, A/V sync/drift, device/permission faults, network/storage/job failures, crash/recovery, codec/hardware fallback, load, repeated lifecycle, long-duration soak, fuzzing, and leak detection.

### Out of scope

Feature implementation belongs to issues 22–28; this issue can block release but should not silently alter approved parity budgets.

## Deliverables

- [ ] Media test matrix derived from charter platforms, sources, modes, codecs, containers, resolutions, and failure scenarios.
- [ ] Deterministic harness and artifact manifest with tool/runtime/hardware provenance.
- [ ] Objective metadata, frame/audio, perceptual, sync, performance, resource, and recovery comparators.
- [ ] Dedicated-runner/manual plan for hardware and permission cases plus scheduled soak/fuzz lanes.
- [ ] Release dashboard showing baseline, budget, result, trend, flake, and evidence link.

## Acceptance criteria

- [ ] Golden recordings pass on every declared OS/device class within issue 01 budgets.
- [ ] Injected device loss, process crash, disk full, network loss, provider error, cancellation, and unsupported codec produce the specified recovery/failure.
- [ ] Hardware paths and software fallback both pass quality/compatibility tests.
- [ ] Repeated start/stop and long-duration runs remain within memory, handle, thread, disk, drift, CPU/GPU, and temperature budgets.
- [ ] A seeded quality, sync, recovery, and resource regression each fails the correct gate.

## Required test evidence

- Full matrix report and representative artifacts.
- At least one completed long-duration soak per supported native platform.
- Fuzz corpus/crash triage and leak/resource trend reports.

## Risks and open questions

- Hosted CI cannot reproduce all hardware/permission behavior.
- Perceptual scores alone can miss timing, cursor, edit, or recovery regressions.

## Rollout and rollback

Run advisory during implementation, then make the fast suite required and full matrix a release gate. Waivers need owner, user impact, expiry, and rollback.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
