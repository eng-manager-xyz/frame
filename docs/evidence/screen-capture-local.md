# Local screen-capture contract evidence

This record covers only the provider-free issue-24 contract and deterministic
state machinery in `frame-media`. It contains no physical screen recording,
native adapter, OS permission prompt, customer media, device label, window
title, process identity, or platform certification.

## Closure ledger boundary

Issue 24 checkboxes 1–10 are repository-local gaps. None is currently
`protected_pending`: representative hardware will eventually be required, but
hardware evidence cannot precede the missing production implementation.

The tests below validate contracts for a future adapter. They do not provide a
production `ScreenCaptureSource`, a release-wired GStreamer appsrc pump fed by
an OS source, OS permission integration, target/lifecycle notifications,
Frame-window exclusion, or an issue-04 parity recording. A standalone owned
appsrc recording component does not change that boundary. The only source
implementation exercised here is the test `DummySource`; therefore its frames,
permission events, geometry, cursor, recovery, copy-budget, and exclusion
results are invalid as evidence for issue-24 completion.

On 2026-07-16, 54 dedicated out-of-module contract tests and the complete
113-test `frame-media` suite passed on the local native toolchain. The focused
suite compiles a dummy `ScreenCaptureSource` using only the exported API; no
test-only constructor or in-module field access is needed. The gates cover:

- redacted, kind-safe opaque source/target identities; caller-supplied 128-bit
  CSPRNG session identities; source-, topology-, and target-epoch-bound
  selections/exclusions; bounded duplicate-free snapshots; exact canonical
  region-transform matching; and forged descriptor rejection;
- checked signed logical geometry, rational DPI, fractional edges, negative
  origins, all four rotations, region containment, and target epochs;
- cursor hidden/embedded/metadata policies, explicit logical versus frame-local
  coordinate spaces, mixed-DPI window rejection, target-local scaling,
  rotation, bounded/redacted one-entry image caching, missing/stale/future and
  nonmonotonic revision rejection, epoch-scoped lease release, click/image
  feature negotiation, and complete clearing outside a selection;
- internally consistent platform capability descriptors, read-only capability
  inspection, exact supported frame-profile tuples without cross-product
  inference, and exact rejection of unsupported cursor, exclusion, recovery,
  protected-content, appsrc, memory, format, size, and rate semantics;
- a timestamp-preserving, non-blocking appsrc ingress plan with owned native
  frame leases;
- a mandatory ingress that alone owns the cursor policy, cursor cache, frame
  queue, active stream, and epoch; exact source/target/session/stream rejection;
  pre-ack and delayed-data rejection; cursor policy enforcement; frame-count,
  retained-byte, and age bounds; cancellation coupled to a terminal session,
  one atomic drain, and one session-scoped exact stop; recovery from cancelled,
  deadline, retryable, and nonretryable terminal-stop races without reopening
  or redraining; terminal exact-stop acknowledgement and delayed-failure replay
  rejection; mixed-ingress/session/source poll and pop rejection before native
  or queue side effects; and replayed nonmonotonic reset rejection;
- monotonic source-control epoch/sequence rejection for stale grant, wake, and
  protected-clear events; prompt/grant flow; `AccessRevoked` fresh-preflight
  latching; start/stop; unrelated/stale hotplug; selected-target loss/restore;
  reconfiguration; and protected-content detection behind permission, sleep,
  and target-loss blockers; and
- one-shot live capability/catalog revalidation and non-cloneable operation
  tickets; stale superseded action rejection before ticket minting; one
  pending start/reconfigure/stop with exact acknowledgement and
  failure correlation; the four reviewed grant/stop/reconfiguration orderings;
  stop execution despite failed post-revocation enumeration; bound enumeration
  and raw-poll failure transitions; retryable preflight/request errors; complete
  selected/exclusion epoch and semantics comparison on unrelated topology;
  fail-closed capability loss and promised-exclusion removal with exact
  ingress retirement, old-frame rejection, and native Stop; pre-negotiation
  bound-source bootstrap whose opaque binding is required by session creation;
  private call tickets on capability, permission, enumeration, and poll trait
  methods so raw/pre-bind dispatch is structurally unavailable; no mutable
  adapter accessor or `DerefMut`, a compile-fail replacement proof, and a
  whole-wrapper swap test proving adapter and binding move together; opaque
  action ownership checked against ingress/session/source before control or
  operation dispatch; foreign control-action rejection with no native or
  session side effects; opaque epoch-transition ownership checked before queue
  or cursor mutation; foreign handoff rejection with queued data preserved;
  private internal session events and no generic public event-application API;
  a compile-fail forged-preflight proof plus real bound preflight execution in
  the external harness; opaque owner-stamped permission results, poll events,
  poll failures, operation acknowledgements, and operation failures; no public
  raw-event ingress; foreign permission/topology/sleep envelopes rejected
  before cancellation or any A session/queue/cache mutation; predecessor-
  aware, all-and-only session Stop quiescence before and after an unacknowledged
  reconfigure dispatch while a second session sharing the backend stays live;
  mandatory source/control/appsrc-flush actions for permission revocation,
  target loss/reconfiguration, sleep, and protected content; fail-closed access
  loss without a preflight action; and privacy-safe diagnostics.

Reproduce the focused tests and lint gate with:

```sh
cargo test --locked -p frame-media --test screen_capture_contract
cargo test --locked -p frame-media --doc
cargo clippy --locked -p frame-media --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p frame-media --no-deps
rustfmt --edition 2024 --check \
  crates/media/src/capture.rs \
  crates/media/src/screen_capture.rs \
  crates/media/tests/screen_capture_contract.rs
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
```

The complete media test suite remains the integration gate because this module
reuses the existing frame timing, cancellation, runtime capability, and video
format contracts.

## Evidence not present

| Gate | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Native ScreenCaptureKit / Graphics Capture / PipeWire or approved X11 adapter | pending | pending | pending |
| Permission preflight, prompt, denial, settings, revocation | pending | pending | pending |
| Physical display/window/region samples | pending | pending | pending |
| Multi-monitor negative origins, mixed/fractional DPI, rotations | pending | pending | pending |
| Cursor image/position/click parity and clipping | pending | pending | pending |
| Frame UI/window exclusion recording | pending | pending | pending |
| Unplug, close/minimize, hotplug, sleep/wake, protected content | pending | pending | pending |
| Zero-/bounded-copy buffer lifetime and latency/CPU/GPU/memory measurements | pending | pending | pending |
| Cap-baseline and issue-04 fixture parity | pending | pending | pending |

No row in this table may be inferred from a unit test or an enum-to-source
mapping. Before the OS/architecture/device matrix can produce valid acceptance
evidence, Frame must implement and wire the production source, appsrc,
permission, lifecycle, exclusion, and parity paths represented by checkboxes
1–10. Recorded samples, probes, measurements, operational documentation, and
rollout evidence remain subsequent gates rather than substitutes for that code.
