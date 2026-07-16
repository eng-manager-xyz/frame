---
title: "ADR: Define the Worker, Cloudflare Media, and native GStreamer topology"
labels:
  - "phase:p0"
  - "area:architecture"
  - "area:gstreamer"
  - "area:cloudflare-media"
  - "area:d1"
  - "type:adr"
  - "risk:high"
depends_on: [01, 02]
size: epic
---

# 03 · ADR: Define the Worker, Cloudflare Media, and native GStreamer topology

## Outcome

A proven deployment topology keeps D1, R2, and Media Transformations at the Worker boundary, keeps GStreamer in native processes, and routes each media job by declared capabilities and limits.

## Current Cap reference

Cap combines native desktop Rust, web/server TypeScript, and a separate media server. The target adds Cloudflare Media Transformations over private R2 for bounded derivatives. GStreamer, OS capture APIs, hardware codecs, and GLib still require a native desktop or container runtime.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#01](./01-p0-migration-charter-parity-slos.md), [#02](./02-p0-establish-r2-storage-target.md)

## Scope

Prove Worker, Cloudflare Media, native media-worker, Leptos, and Tauri boundaries. Define a capability matrix and deterministic routing for Media Transformations versus GStreamer. Specify versioned transform profiles, authentication, immutable R2 keys, idempotency, leases, retries, dead letters, progress capability, cancellation, fallback, quotas, cost, region selection, and data residency. Keep the separate `[stream]` managed video-library binding disabled unless separately approved.

### Out of scope

Completing all API handlers or production pipelines is outside this ADR; issue 07 builds the protocol and issues 22–29 build media behavior.

## Deliverables

- [ ] Accepted ADRs 0001 and 0003 plus deployment, data-flow, and executor-selection diagrams.
- [ ] An R2 source → `MEDIA` frame/preview → immutable R2 output spike in a remote Cloudflare environment.
- [ ] A separate versioned job that executes native GStreamer work and reports a terminal result.
- [ ] Job, progress, error, cancellation, and callback schemas with compatibility rules.
- [ ] Threat and failure-mode analysis for duplicate delivery, lost callbacks, expired leases, provider quota/outage, unavailable storage, unsupported inputs, and partial D1 updates.
- [ ] A decision for Cloudflare Queues or an alternative bridge, including pull-consumer and dead-letter behavior.
- [ ] A maintainable `workers-rs` interop plan: isolated `wasm-bindgen` adapter or a minimal service-bound Worker fallback.

## Acceptance criteria

- [ ] The Worker crate builds for wasm32 without GStreamer, Tokio runtime, or native-only transitive dependencies.
- [ ] The media worker builds and runs outside Workers without direct production D1 credentials.
- [ ] The dispatcher rejects over-limit or unsupported Media inputs before invocation and selects native fallback according to policy.
- [ ] Supported managed derivatives persist deterministic outputs in R2; repeated delivery reuses the same logical artifact and does not transform on every playback.
- [ ] Replaying the same job and callback is safe and produces one logical result.
- [ ] An injected worker crash results in a retry or dead-letter record with observable state.
- [ ] Cancellation and progress account for executors that cannot report in-flight progress or cancel; authentication, fallback, regional routing, and secret rotation are specified end to end.

## Required test evidence

- Traces for both R2 → Media Transformations → R2 and queue → GStreamer → R2 → D1.
- Failure-injection results for duplicate, timeout, crash, provider error/quota, unsupported input, fallback, and callback loss.
- Build artifacts for wasm32 and at least one native target.

## Risks and open questions

- A distributed split adds consistency and operational complexity.
- The Media binding is public beta, remote-only in development, has bounded formats/sizes/durations, and has no first-class `workers-rs` 0.8.5 wrapper.
- Using D1's administrative REST API as an application data plane would create security and rate-limit problems.

## Rollout and rollback

The spike runs in an isolated environment behind a feature flag. Removing it deletes only synthetic jobs and objects; no production data is touched.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
