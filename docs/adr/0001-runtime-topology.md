# ADR 0001: Split edge and native media runtimes

- Status: accepted
- Date: 2026-07-15

## Context

Cloudflare D1, R2, and Media Transformations bindings are exposed inside a Worker/Wasm runtime. GStreamer, hardware codecs, screen capture, and desktop permission APIs require a native process. Combining them into one deployment unit would either make the Worker impossible to build or force the control plane onto a native server and give up the requested Cloudflare integrations.

## Decision

Use a Rust/Wasm Worker for authenticated APIs, D1, R2 upload authorization, supported Cloudflare Media transformations, and native job publication. Use separately deployable native Rust processes for desktop capture and advanced or fallback GStreamer work. Connect every executor through versioned, authenticated, idempotent job and result contracts. Keep Leptos UI code independent of all transports.

For the initial bridge, use D1-backed job records plus bounded scheduled scans
and scoped HTTP claim/lease/heartbeat/result calls. Native workers pull through
the control-plane API and never receive D1 credentials. Exhausted attempts and
unrecoverable results enter the D1 dead-letter inventory. Cloudflare Queues is
therefore not an implicit second authority in v1; adopting it later requires a
compatibility decision that preserves the same idempotency, lease fence,
dead-letter, cancellation, and reconciliation contracts.

The `MEDIA` binding interop remains isolated in the control-plane adapter. If
the reviewed `wasm-bindgen` boundary ceases to compile or safely model the
provider stream, the permitted fallback is a minimal service-bound Worker that
implements the same private request/result contract; domain, application, and
native crates remain unchanged.

```text
client -> Worker/API -> D1 job + private R2 source
                         |                 |
                         | bounded scan    +-> MEDIA -> immutable R2 output
                         v
                 native claim API -> GStreamer -> immutable R2 output
                         |
                         +-> fenced callback -> D1 terminal state/manifest
```

## Consequences

The system is operationally more explicit and can scale the control and media planes independently. A capability-aware dispatcher must choose Cloudflare Media or native GStreamer without changing domain contracts. The split also introduces distributed failure modes: retries, duplicate delivery, cancellation, progress reporting, object/database reconciliation, provider fallback, and regional data handling must be designed rather than hidden.

Issue 03 owns the validation evidence and implementation plan for this decision.
