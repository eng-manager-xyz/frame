---
title: "ADR: Split the edge control plane from native GStreamer media workers"
labels:
  - "phase:p0"
  - "area:architecture"
  - "area:gstreamer"
  - "area:d1"
  - "type:adr"
  - "risk:high"
depends_on: [01, 02]
size: epic
---

# 03 · ADR: Split the edge control plane from native GStreamer media workers

## Outcome

A proven deployment topology keeps D1/object bindings in Rust/Wasm and GStreamer in native processes while defining the distributed contract between them.

## Current Cap reference

Cap combines native desktop Rust, web/server TypeScript, and a separate media server. Cloudflare Worker bindings are Wasm-oriented; GStreamer, OS capture APIs, hardware codecs, and GLib require a native desktop or container runtime.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#01](./01-p0-migration-charter-parity-slos.md), [#02](./02-p0-resolve-r3-storage-target.md)

## Scope

Prove Worker, native media-worker, Leptos, and Tauri boundaries. Specify versioned job payloads, authentication, idempotency keys, leases, retries, dead letters, progress callbacks, cancellation, capacity, region selection, and data residency.

### Out of scope

Completing all API handlers or production pipelines is outside this ADR; issue 07 builds the protocol and issues 22–29 build media behavior.

## Deliverables

- [ ] An accepted replacement for proposed ADR 0001 and a deployment/data-flow diagram.
- [ ] A spike that publishes one versioned job, executes native GStreamer work, and reports a terminal result.
- [ ] Job, progress, error, cancellation, and callback schemas with compatibility rules.
- [ ] Threat and failure-mode analysis for duplicate delivery, lost callbacks, expired leases, unavailable storage, and partial D1 updates.
- [ ] A decision for Cloudflare Queues or an alternative bridge, including pull-consumer and dead-letter behavior.

## Acceptance criteria

- [ ] The Worker crate builds for wasm32 without GStreamer, Tokio runtime, or native-only transitive dependencies.
- [ ] The media worker builds and runs outside Workers without direct production D1 credentials.
- [ ] Replaying the same job and callback is safe and produces one logical result.
- [ ] An injected worker crash results in a retry or dead-letter record with observable state.
- [ ] Cancellation, progress, authentication, regional routing, and secret rotation are specified end to end.

## Required test evidence

- A trace from request through queue/job, GStreamer smoke work, object write, and D1 status callback.
- Failure-injection results for duplicate, timeout, crash, and callback loss.
- Build artifacts for wasm32 and at least one native target.

## Risks and open questions

- A distributed split adds consistency and operational complexity.
- Using D1's administrative REST API as an application data plane would create security and rate-limit problems.

## Rollout and rollback

The spike runs in an isolated environment behind a feature flag. Removing it deletes only synthetic jobs and objects; no production data is touched.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
