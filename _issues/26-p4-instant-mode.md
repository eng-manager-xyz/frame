---
title: "Rebuild Instant Mode segmentation, live upload, reconnect, and idempotent finalize"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:storage"
  - "area:desktop"
  - "type:migration"
  - "risk:high"
depends_on: [18, 19, 23, 24, 25]
size: epic
---

# 26 · Rebuild Instant Mode segmentation, live upload, reconnect, and idempotent finalize

## Outcome

Instant recordings become shareable during or immediately after capture and recover cleanly from network, process, or device interruptions.

## Current Cap reference

Cap Instant Mode fragments/segments recordings, spools chunks, uploads while recording, resumes, finalizes multipart state, heals tracks, and handles stale desktop segments. This is a distributed state machine, not only an encoder pipeline.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#18](./18-p3-object-storage-adapter-key-contract.md), [#19](./19-p3-multipart-upload-download.md), [#23](./23-p4-gstreamer-pipeline-core.md), [#24](./24-p4-screen-capture.md), [#25](./25-p4-audio-camera-sync.md)

## Scope

Design segmented/fMP4 or approved output, split points/keyframes, local journal/spool, bounded disk usage, live multipart/part upload, retry/backoff, offline continuation, reconnect, crash restart, cancellation, finalize, stale-session repair, share-page readiness, and an approved Media-compatible distribution master when capability limits permit.

### Out of scope

Studio editing/export is issue 27; generic upload authorization is issue 19.

## Deliverables

- [ ] Versioned Instant recording, segment, spool, upload, and finalize state machines.
- [ ] GStreamer segmentation pipeline with deterministic manifest and recoverable segment metadata.
- [ ] Encrypted/private local spool with quota, cleanup, crash restart, and user-visible recovery.
- [ ] Idempotent server finalize and reconciliation of D1 job, multipart state, object manifest, and playable result.
- [ ] Post-finalize derivative policy that can choose Cloudflare Media or native GStreamer without changing the capture/upload state machine.
- [ ] Progress and error contract consumed by desktop and share UI.

## Acceptance criteria

- [ ] Recording continues locally through network loss within disk limits and resumes upload without duplicating verified segments.
- [ ] Process crash at every state boundary can resume or produce a documented recoverable artifact.
- [ ] Duplicate/out-of-order segment, complete, and callback requests cannot publish corrupt or multiple final recordings.
- [ ] Playback begins within the charter target and remains valid after finalization/manifest replacement.
- [ ] Cancel/delete aborts uploads, cleans spool according to policy, and cannot later resurrect the video.

## Required test evidence

- State-transition fault matrix including kill/restart, offline, throttling, expiry, and server error.
- Segment/manifests probe and playback results.
- Disk, upload, CPU, memory, and time-to-share baselines.

## Risks and open questions

- Container choice affects browser compatibility and crash recovery.
- Local spool can leak sensitive media or exhaust disk.

## Rollout and rollback

Internal/test tenants first, with legacy Instant Mode selectable per recording and an upload/finalize kill switch.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
