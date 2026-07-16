---
title: "Build the Rust GStreamer pipeline core: caps, clocks, state, errors, cancellation, and metrics"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:rust"
  - "type:migration"
  - "risk:high"
depends_on: [06, 07, 22]
size: epic
---

# 23 · Build the Rust GStreamer pipeline core: caps, clocks, state, errors, cancellation, and metrics

## Outcome

All media features build on one deterministic, observable pipeline owner with typed state and bounded resource behavior.

## Current Cap reference

Cap has mature native recording/output pipelines and recovery code, largely organized around platform capture and FFmpeg/native encoders. Frame currently has only a synchronous synthetic GStreamer pipeline.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#06](./06-p1-shared-domain-api-contracts.md), [#07](./07-p1-control-plane-media-job-protocol.md), [#22](./22-p4-gstreamer-runtime-packaging.md)

## Scope

Create pipeline ownership around GLib/GStreamer contexts, element construction, caps negotiation, clocks/segments, bus handling, state machine, bounded appsrc/appsink queues, backpressure/drop policy, EOS/finalization, cancellation, timeouts, diagnostics, and test hooks.

### Out of scope

Real capture sources, product modes, and server transforms are issues 24–28.

## Deliverables

- [ ] Typed Idle → Preparing → Running ↔ Paused → Finalizing → Completed/Failed/Cancelled state machine.
- [ ] Safe pipeline builder and element registry abstraction with explicit caps and capability errors.
- [ ] Dedicated bus/context owner and async command/event boundary that never sends raw frames through UI IPC.
- [ ] Clock, timestamp, queue, buffer-pool, backpressure, drop, memory, and cancellation policies.
- [ ] Structured metrics and redacted diagnostic bundle including topology and negotiated caps.

## Acceptance criteria

- [ ] Every command/state transition is deterministic, tested, and returns a terminal result exactly once.
- [ ] Caps negotiation and missing-element failures identify the incompatible pad/factory without exposing paths or media.
- [ ] Slow sinks and producers remain within configured memory bounds and apply the declared drop/backpressure policy.
- [ ] Cancellation and timeout drive the graph to Null, release devices/files, and leave recoverable output where specified.
- [ ] Bus warnings/errors, latency, queue depth, dropped buffers, state duration, and finalization are observable with correlation IDs.

## Required test evidence

- Synthetic video plus audio pipeline tests.
- Fault injection for negotiation, plugin error, blocked sink, EOS loss, cancellation, and timeout.
- Memory/thread/resource-leak report across repeated start/stop cycles.

## Risks and open questions

- GLib/GStreamer thread affinity and callbacks can conflict with async runtimes.
- Incorrect timestamps or queue policy causes subtle A/V drift and memory growth.

## Rollout and rollback

Introduce as a parallel backend used by synthetic and internal recordings. Keep legacy pipeline selection until parity gate 29 passes.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
