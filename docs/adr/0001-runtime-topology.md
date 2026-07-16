# ADR 0001: Split edge and native media runtimes

- Status: proposed
- Date: 2026-07-15

## Context

Cloudflare D1 and object-storage bindings are exposed inside a Worker/Wasm runtime. GStreamer, hardware codecs, screen capture, and desktop permission APIs require a native process. Combining them into one deployment unit would either make the Worker impossible to build or force the control plane onto a native server and give up the requested D1 integration.

## Proposed decision

Use a Rust/Wasm Worker for authenticated APIs, D1, upload authorization, and job publication. Use separately deployable native Rust processes for desktop capture and server-side GStreamer work. Connect them through versioned, authenticated, idempotent job and callback contracts. Keep Leptos UI code independent of both transports.

## Consequences

The system is operationally more explicit and can scale the control and media planes independently. It also introduces distributed failure modes: retries, duplicate delivery, cancellation, progress reporting, object/database reconciliation, and regional data handling must be designed rather than hidden.

Issue 03 owns validation and approval of this decision.
