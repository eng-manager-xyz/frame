# ADR 0001: Split edge and native media runtimes

- Status: accepted
- Date: 2026-07-15

## Context

Cloudflare D1, R2, and Media Transformations bindings are exposed inside a Worker/Wasm runtime. GStreamer, hardware codecs, screen capture, and desktop permission APIs require a native process. Combining them into one deployment unit would either make the Worker impossible to build or force the control plane onto a native server and give up the requested Cloudflare integrations.

## Decision

Use a Rust/Wasm Worker for authenticated APIs, D1, R2 upload authorization, supported Cloudflare Media transformations, and native job publication. Use separately deployable native Rust processes for desktop capture and advanced or fallback GStreamer work. Connect every executor through versioned, authenticated, idempotent job and result contracts. Keep Leptos UI code independent of all transports.

## Consequences

The system is operationally more explicit and can scale the control and media planes independently. A capability-aware dispatcher must choose Cloudflare Media or native GStreamer without changing domain contracts. The split also introduces distributed failure modes: retries, duplicate delivery, cancellation, progress reporting, object/database reconciliation, provider fallback, and regional data handling must be designed rather than hidden.

Issue 03 owns the validation evidence and implementation plan for this decision.
