---
title: "Rebuild Studio Mode local recording, crash recovery, timeline edits, rendering, and export"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:desktop"
  - "area:editor"
  - "type:migration"
  - "risk:high"
depends_on: [23, 24, 25]
size: epic
---

# 27 · Rebuild Studio Mode local recording, crash recovery, timeline edits, rendering, and export

## Outcome

Studio recordings, projects, edits, previews, recovery, and exports preserve Cap workflows and output quality.

## Current Cap reference

Cap has project/edit schemas, local studio recording, separate screen/camera/audio assets, recovery, playback, rendering, Skia/GPU composition, and FFmpeg/native export paths. A single flattened capture would remove editing capability.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#23](./23-p4-gstreamer-pipeline-core.md), [#24](./24-p4-screen-capture.md), [#25](./25-p4-audio-camera-sync.md)

## Scope

Define project format and migrations; record isolated tracks; journal safely; recover crashes; implement timeline/segments, trim/split/delete, camera/cursor/background/layout transforms, preview, audio, render/export profiles, progress, cancellation, and hardware/software fallback.

### Out of scope

Complete product UI is issue 33; server media derivatives are issue 28.

## Deliverables

- [ ] Versioned project/asset/edit format with forward/backward compatibility and corruption detection.
- [ ] Multi-track GStreamer/native recording pipeline and atomic journal/recovery design.
- [ ] Playback/seek/preview engine with deterministic edit interpretation.
- [ ] Render/export graph for approved containers/codecs/resolutions/frame rates with progress and cancellation.
- [ ] Legacy Cap project import strategy and explicit unsupported-effect handling.

## Acceptance criteria

- [ ] Approved legacy projects open or produce an actionable compatibility report without modifying the source.
- [ ] Crash/power-loss injection leaves a recoverable project at each recording and edit-save boundary.
- [ ] Preview and export interpret the same edit spec within approved frame/audio tolerances.
- [ ] Seek, trim boundaries, variable frame rate, speed, camera/cursor layout, audio gain, and long projects pass goldens.
- [ ] Hardware failure falls back or fails safely without losing the project; cancelled exports clean partial outputs.

## Required test evidence

- Legacy-project compatibility corpus.
- Frame/audio diff between preview/export/reference.
- Crash recovery, long-project, seek, memory, and export benchmark report.

## Risks and open questions

- Editor parity is larger than media encoding and may expose hidden format assumptions.
- Hardware codecs can differ in color, keyframe, and timing behavior.

## Rollout and rollback

Open projects read-only first, then opt-in new recordings, then editing/export. Preserve original assets and legacy editor access through the rollback window.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
