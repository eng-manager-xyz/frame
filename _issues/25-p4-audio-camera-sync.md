---
title: "Implement microphone, system-audio, camera, permissions, hotplug, and A/V sync"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:desktop"
  - "area:capture"
  - "type:migration"
  - "risk:high"
depends_on: [22, 23, 24]
size: epic
---

# 25 · Implement microphone, system-audio, camera, permissions, hotplug, and A/V sync

## Outcome

Screen, microphone, system audio, and camera remain synchronized and resilient across device and lifecycle changes.

## Current Cap reference

Cap includes CPAL/native audio, camera backends, meters, mixing, sync calibration, and platform-specific capture. GStreamer can mix/convert/encode, but native bridges may still be needed for permissions and device fidelity.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#22](./22-p4-gstreamer-runtime-packaging.md), [#23](./23-p4-gstreamer-pipeline-core.md), [#24](./24-p4-screen-capture.md)

## Scope

Implement enumeration and stable device IDs, permission flows, mic/system capture, camera formats, audio mixing/gain/mute/meters, camera preview, clock selection, latency/calibration, drift correction, pause/resume, hotplug/default changes, Bluetooth changes, sleep/wake, and no-device fallback.

### Out of scope

Visual camera overlay/editor layout is issue 27/33; server transcription/AI is issue 28.

## Deliverables

- [ ] Normalized audio/camera source contracts and native-to-appsrc bridges where selected.
- [ ] GStreamer audio mixer/resampler/converter and camera conversion paths with negotiated formats.
- [ ] Clock and timestamp design with measured startup offset, drift, correction, and discontinuity handling.
- [ ] Device settings persistence and migration rules that survive renamed/missing devices safely.
- [ ] Meters/preview events throttled for UI without sending raw media through IPC.

## Acceptance criteria

- [ ] A/V offset and long-duration drift remain within charter budgets across the declared device/OS matrix.
- [ ] Mute, gain, system/mic mix, camera enable/disable, and pause/resume produce continuous declared timelines.
- [ ] Permission denial/revocation, unplug, default-device switch, Bluetooth profile change, and sleep/wake yield defined recovery or actionable failure.
- [ ] Absent optional devices do not prevent screen-only recording.
- [ ] Diagnostics record device class/capability and timing statistics without user labels, media, or sensitive hardware identifiers.

## Required test evidence

- Long-duration sync plots and media probes.
- Hotplug, denial, drift, overload, and sleep/wake fault matrix.
- CPU/memory/latency comparison to baseline.

## Risks and open questions

- Multiple clocks and OS latency reporting can cause cumulative drift.
- System-audio capture has major platform-specific permission and API constraints.

## Rollout and rollback

Enable source combinations incrementally in internal channels with automatic fallback to proven Cap paths until conformance passes.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
