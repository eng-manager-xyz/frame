---
title: "Implement cross-platform screen, window, and region capture with cursor metadata"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:desktop"
  - "area:capture"
  - "type:migration"
  - "risk:high"
depends_on: [22, 23]
size: epic
---

# 24 · Implement cross-platform screen, window, and region capture with cursor metadata

## Outcome

Frame captures the selected display, window, or region with Cap-level permissions, fidelity, cursor behavior, and recoverability on each supported OS.

## Current Cap reference

Cap already has native ScreenCaptureKit, Direct3D/Windows capture, target enumeration, cursor capture, window exclusion, and Linux paths. Replacing proven native capture merely to use stock GStreamer elements risks regressions.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#22](./22-p4-gstreamer-runtime-packaging.md), [#23](./23-p4-gstreamer-pipeline-core.md)

## Scope

Define a CaptureSource trait; preserve/adapt approved native sources and feed GStreamer through bounded appsrc where needed. Implement target enumeration, permissions, display/window/region selection, high-DPI coordinates, multi-monitor, cursor image/position/click metadata, exclusion, hotplug, target loss, and protected-content behavior.

### Out of scope

Audio and camera are issue 25; compositing/editor semantics beyond cursor metadata are issues 27/33.

## Deliverables

- [ ] Per-OS source adapters and a normalized frame/cursor/target contract.
- [ ] GStreamer appsrc bridge with explicit pixel format, color space, frame duration, timestamps, and buffer lifetime.
- [ ] Permission preflight/request/settings guidance and non-destructive denial handling.
- [ ] Device/target change events, reconnection policy, and window-exclusion behavior.
- [ ] Target selection and capture parity tests against issue 04 fixtures.

## Acceptance criteria

- [ ] Display, window, and region recordings match selected geometry across scale factors, rotations, and multi-monitor layouts.
- [ ] Cursor visibility, image changes, position, clicks, and exclusion follow the approved mode contract without leaking outside the selected region.
- [ ] Permission denial/revocation, display unplug, window close/minimize, sleep/wake, and protected content produce defined state and recovery.
- [ ] Frames enter GStreamer without unnecessary full-frame copies where the platform permits and remain within memory/latency budgets.
- [ ] No Frame UI/window is captured when exclusion is promised.

## Required test evidence

- OS/architecture/device matrix with recorded samples and probe results.
- Permission and target-loss failure tests.
- Copy/latency/CPU/GPU measurements compared with Cap baseline.

## Risks and open questions

- OS APIs and permissions change independently of GStreamer.
- A stock source element may lack target selection, cursor, or system fidelity.

## Rollout and rollback

Default to the existing/legacy source per OS while shadowing GStreamer output on internal builds; switch one target/mode/platform at a time.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
