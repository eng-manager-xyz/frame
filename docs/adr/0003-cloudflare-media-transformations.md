# ADR 0003: Use Cloudflare Media Transformations with native GStreamer

- Status: accepted
- Date: 2026-07-15

## Context

Frame needs both native recording/editing and server-side media derivatives.
Cloudflare's Media Transformations product can produce optimized video, still
frames, spritesheets, or extracted audio. Its Workers binding can accept a
private R2 `ReadableStream` without publishing the source. It does not replace
operating-system capture, long-form processing, arbitrary codec/container
support, timeline composition, repair, or local/offline export.

The product documentation dated 2026-04-21 marks Media Transformations and URL
mode generally available. The binding documentation updated 2026-06-10 still
marks the Workers binding public beta. The binding
requires remote Wrangler development, does not automatically cache binding
outputs, and has no first-class wrapper in `workers-rs` 0.8.5. Binding beta
billing and product URL-mode pricing must not be conflated. The separately
configured `[stream]` binding provides managed upload, video-library,
adaptive-playback, and delivery capabilities and is not implied by `[media]`.

## Decision

Configure the Worker with `[media] binding = "MEDIA"`. Route supported short derivatives from private R2 through Cloudflare Media Transformations, persist every successful immutable result back to R2, and update its manifest/state in D1. Route capture, synchronization, Studio/Instant editing and export, long or large inputs, complex composition, unsupported formats, and fallback work to native Rust/GStreamer executors.

Hide both executors behind a versioned media-processing port and an explicit capability/limit matrix. Isolate the unstable JavaScript interop behind a small Rust `wasm-bindgen` adapter; if that spike proves unmaintainable, use a minimal TypeScript Worker behind a service binding while keeping the Rust control plane and native media plane. Do not enable the `[stream]` managed video-library binding without a separate ADR and product decision.

## Consequences

Job routing, deterministic keys, idempotency, R2 persistence, cost accounting, limit checks, observability, and cross-backend conformance become required. Local tests use a fake media port; a remote integration lane exercises the real binding. Production rollout must measure binding maturity, limits, latency, output compatibility, and post-beta billing, and must preserve native or legacy fallback per job type.

## References

- [Media Transformations binding](https://developers.cloudflare.com/stream/transform-videos/bindings/)
- [Media Transformations limits and formats](https://developers.cloudflare.com/stream/transform-videos/)
- [R2 event notifications](https://developers.cloudflare.com/r2/buckets/event-notifications/)
- [`STREAM` managed video-library binding (separate capability)](https://developers.cloudflare.com/stream/manage-video-library/bindings/)
