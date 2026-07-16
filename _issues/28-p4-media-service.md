---
title: "Migrate media service: probe, transcode, thumbnails, waveforms, captions, and AI audio"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:api"
  - "area:ops"
  - "type:migration"
  - "risk:high"
depends_on: [07, 18, 19, 22, 23, 25]
size: epic
---

# 28 · Migrate media service: probe, transcode, thumbnails, waveforms, captions, and AI audio

## Outcome

Server-side media jobs run in native Rust/GStreamer workers with bounded resources, safe inputs, durable progress, and output parity.

## Current Cap reference

Cap's media server and web workflows use Bun/Hono, Mediabunny/node-av, FFmpeg processes, and provider callbacks for probing, preview, thumbnail, transcription, AI, multipart, and progress work.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#07](./07-p1-control-plane-media-job-protocol.md), [#18](./18-p3-object-storage-adapter-key-contract.md), [#19](./19-p3-multipart-upload-download.md), [#22](./22-p4-gstreamer-runtime-packaging.md), [#23](./23-p4-gstreamer-pipeline-core.md), [#25](./25-p4-audio-camera-sync.md)

## Scope

Inventory jobs and implement probe, remux/transcode, thumbnails/contact sheets, preview, waveform/audio extraction, normalization, GIF if retained, repair, captions/transcription provider boundary, AI audio cleanup if retained, cancellation, resource limits, work leases, output manifests, and callbacks.

### Out of scope

Rebuilding third-party AI models is not required; providers remain behind adapters. Desktop Studio export is issue 27.

## Deliverables

- [ ] Versioned job catalog with input/output roles, resource class, timeout, retryability, and idempotency.
- [ ] GStreamer/Rust implementations or documented exceptions where another audited tool remains necessary.
- [ ] Sandbox/input validation, local scratch policy, memory/CPU/GPU/disk limits, cancellation, and cleanup.
- [ ] Durable progress/result/error callbacks and output-manifest reconciliation.
- [ ] Capacity, scheduling, autoscaling, data-residency, and dead-letter runbooks.

## Acceptance criteria

- [ ] Every retained Cap media job has a parity fixture and declared implementation/disposition.
- [ ] Malformed, oversized, adversarial, decompression-bomb, timeout, and unsupported-codec inputs cannot escape resource/sandbox limits.
- [ ] Retrying a job reuses or replaces deterministic outputs safely and never publishes partial objects.
- [ ] Progress is monotonic or explicitly reset by attempt; cancellation releases capacity and cleans scratch/output.
- [ ] Outputs pass metadata, playback, perceptual, caption, and waveform tolerances from issue 04.

## Required test evidence

- Golden job matrix and output probes.
- Fuzz/adversarial/fault test report.
- Capacity, cost, throughput, cancellation, and cleanup measurements.

## Risks and open questions

- Media parsers/codecs process untrusted content and are high-risk.
- Some FFmpeg behavior may lack direct GStreamer parity and needs an explicit exception.

## Rollout and rollback

Shadow selected jobs and compare outputs before serving them. Route each job type independently back to the legacy service on regression.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
