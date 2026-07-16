---
title: "Build hybrid Cloudflare Media Transformations and native GStreamer jobs"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:cloudflare-media"
  - "area:api"
  - "area:ops"
  - "type:migration"
  - "risk:high"
depends_on: [07, 18, 19, 22, 23, 25]
size: epic
---

# 28 · Build hybrid Cloudflare Media Transformations and native GStreamer jobs

## Outcome

Server-side jobs use Cloudflare Media Transformations for supported private-R2 derivatives and native Rust/GStreamer for complex, long-form, unsupported, or fallback work, with one durable contract and output-parity gate.

## Current Cap reference

Cap's media server and web workflows use Bun/Hono, Mediabunny/node-av, FFmpeg processes, and provider callbacks for probing, preview, thumbnail, transcription, AI, multipart, and progress work. Cloudflare Media is new target architecture, not current Cap behavior.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#07](./07-p1-control-plane-media-job-protocol.md), [#18](./18-p3-object-storage-adapter-key-contract.md), [#19](./19-p3-multipart-upload-download.md), [#22](./22-p4-gstreamer-runtime-packaging.md), [#23](./23-p4-gstreamer-pipeline-core.md), [#25](./25-p4-audio-camera-sync.md)

## Scope

Inventory every retained job and define a versioned capability matrix. Prefer Cloudflare Media for supported short optimized H.264/AAC MP4 clips, JPEG/PNG frames, JPEG spritesheets, and AAC/M4A audio extraction from private R2. Keep GStreamer for probe, capture-related processing, arbitrary codecs/containers, long or large inputs, full-length outputs, remux/repair, waveform generation, composition, exact rendering, advanced normalization, and fallback.

Implement input/limit preflight, deterministic routing, immutable R2 outputs, cancellation semantics, resource limits, leases where applicable, output manifests, provider/native cost accounting, callbacks, and reconciliation. Current vendor limits and recommendations must be configuration with remote contract tests rather than scattered constants.

### Out of scope

Rebuilding third-party AI models is not required; caption/transcription and AI cleanup remain separate providers behind adapters. Desktop Studio export is issue 27. The `[stream]` managed video-library binding is not selected by this issue.

## Deliverables

- [ ] Versioned job catalog with input/output roles, normalized transform profile, executor capabilities/limits, progress/cancel support, timeout, retryability, fallback, and idempotency.
- [ ] A provider-neutral `MediaTransformer`/derivative-executor port with isolated Cloudflare `wasm-bindgen` interop and an offline fake.
- [x] Cloudflare Media implementations for approved bounded derivatives and GStreamer/Rust implementations for the remaining catalog, with documented exceptions.
- [ ] Canonical Media-compatible distribution-master profile without sacrificing editable/source originals.
- [ ] Sandbox/input validation, SSRF prevention, local scratch policy, memory/CPU/GPU/disk limits, cancellation, and cleanup.
- [ ] Durable progress/result/error handling and R2 output-manifest reconciliation across both executor types.
- [ ] Capacity, quota, cost, scheduling, autoscaling, data-residency, provider-outage, kill-switch, and dead-letter runbooks.

## Acceptance criteria

- [x] Every retained Cap media job has a parity fixture and declared implementation, executor, limits, and fallback/disposition.
- [ ] The router preflights documented size, duration, resolution, format, and output limits and never sends known-unsupported work to Cloudflare Media.
- [ ] Malformed, oversized, adversarial, decompression-bomb, timeout, and unsupported-codec inputs cannot escape resource or sandbox limits.
- [ ] Retrying either backend HEADs/reuses or atomically publishes a deterministic immutable R2 result and never exposes partial objects.
- [ ] Progress is monotonic where supported or explicitly indeterminate; cancellation prevents publication even when a managed invocation cannot be cancelled in flight.
- [ ] Quota, timeout, provider outage, output incompatibility, or beta regression follows the per-job fallback/kill-switch policy without a duplicate logical result.
- [ ] Outputs pass metadata, playback, perceptual, caption, and waveform tolerances from issue 04.
- [ ] Transform requests and logs never expose private media URLs, credentials, bodies, or tenant-sensitive keys.

## Required test evidence

- Cross-executor golden job matrix and output probes, including a licensed synthetic H.264/AAC MP4 fixture for the remote Media lane.
- Exact-limit and just-over-limit routing tests for every managed capability.
- Fuzz, adversarial, SSRF, provider-fault, cancellation, idempotency, and fallback reports.
- Provider-operation cost, native compute cost, latency, throughput, cancellation, fallback, and cleanup measurements.

## Risks and open questions

- Media parsers and codecs process untrusted content and are high-risk.
- Some legacy FFmpeg behavior may lack direct GStreamer parity and needs an explicit exception.
- The Media binding is public beta, remote-only in development, has no automatic output caching, and lacks a first-class `workers-rs` 0.8.5 wrapper.
- Provider formats, limits, pricing, and output characteristics can change; perceptual/metadata parity cannot rely on byte equality.
- Media Transformations extracts audio but does not provide transcription, waveform analysis, or AI cleanup.

## Rollout and rollback

Shadow selected jobs and compare Cloudflare Media, GStreamer, and legacy outputs before serving them. Route each job/profile independently to native or legacy fallback, retain a managed-media kill switch, and keep deterministic R2 artifacts for reconciliation.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.

## Local completion note (2026-07-16)

The four approved Cloudflare modes (`video`, `frame`, `spritesheet`, and
`audio`) have bounded private-R2 binding implementations and exact/just-over
contract tests. The native worker now has a machine-checked entry for all 14
native profiles: four executable local graphs and ten stable graph recipes with
typed codec, sampling, loudness, demux, multi-source, or timeline exceptions.
The exception state fails closed and is not a production-output claim.

Parity matrix schema 2 maps all 16 retained jobs to a concrete SHA-pinned CC0
fixture artifact, primary/fallback executor and implementation, limit profile,
fallback disposition, evidence state, and exception. Remote Cloudflare output,
protected codec graphs, external providers, capacity/cost, and perceptual review
remain open gates elsewhere in this issue; these two completed checkboxes do not
waive them.
