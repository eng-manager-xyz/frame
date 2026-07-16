---
title: "Scaffold Rust control-plane services and a capability-routed media-job protocol"
labels:
  - "phase:p1"
  - "area:api"
  - "area:rust"
  - "area:ops"
  - "area:cloudflare-media"
  - "type:foundation"
  - "risk:high"
depends_on: [03, 05, 06]
size: epic
---

# 07 · Scaffold Rust control-plane services and a capability-routed media-job protocol

## Outcome

One authenticated walking slice creates a video, uploads it to R2, routes a supported derivative through Cloudflare Media or advanced work through native GStreamer, and exposes a stable shareable result.

## Current Cap reference

The scaffold has a Worker health route, D1/R2/Media bindings, a provider-neutral port boundary, and a GStreamer smoke executable but no end-to-end application protocol.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#03](./03-p0-runtime-topology.md), [#05](./05-p1-workspace-boundaries-policy.md), [#06](./06-p1-shared-domain-api-contracts.md)

## Scope

Implement application services, request validation, auth context, D1 row creation, R2 upload intent, normalized transform profiles, capability/preflight evaluation, deterministic executor selection, managed invocation or native job publication, lease/heartbeat where supported, progress and completion, cancellation, retry/fallback classification, cost attribution, and reconciliation hooks.

### Out of scope

Production-grade auth, complete multipart uploads, and real transcode/capture pipelines remain in issues 13, 19, and 22–28.

## Deliverables

- [ ] Versioned create-video, upload-intent, enqueue, claim/lease, heartbeat, progress, complete/fail, cancel, and status contracts.
- [ ] Versioned media request/result contracts containing source version, transform profile/version, selected executor, deterministic output key, capability decision, and redacted provider error class.
- [ ] D1 state and idempotency records that survive duplicate HTTP requests, job delivery, and callbacks.
- [ ] A native control-plane client that uses scoped credentials rather than direct D1 application access.
- [ ] Dead-letter and stale-lease handling with operator-visible diagnostics.
- [ ] End-to-end R2 → Media thumbnail → R2 and R2 → native GStreamer → R2 walking-slice tests and sequence diagrams.

## Acceptance criteria

- [ ] Repeating any command with the same tenant and idempotency key produces one logical state transition.
- [ ] Capability-matched work uses Cloudflare Media, over-limit/unsupported work uses the declared GStreamer fallback, and routing remains invisible to API/UI callers.
- [ ] A media worker that crashes after object upload but before callback is reconciled without duplicate published output.
- [ ] Repeating a completed managed transform reuses the persisted R2 artifact rather than incurring another invocation.
- [ ] Expired leases can be reclaimed, active leases cannot be stolen, and cancelled jobs cannot later become ready.
- [ ] All state changes emit correlation-safe telemetry with no signed URL or auth-token leakage.
- [ ] The slice passes in the local stack and an isolated Cloudflare/native environment.

## Required test evidence

- End-to-end traces and D1/R2 manifests for both executors.
- Fault tests for duplicate, out-of-order, provider outage/quota, fallback, lost callback, expired lease, and cancellation.
- Contract fixtures and API examples.

## Risks and open questions

- D1 and object storage cannot share a transaction.
- Retries without stable idempotency and object naming can corrupt state or leak objects.
- Managed transforms provide different progress/cancellation semantics and may change limits or billing after beta.

## Rollout and rollback

Gate the walking slice to test tenants. Rollback disables job publication, drains or dead-letters outstanding work, and deletes only namespaced test artifacts.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
