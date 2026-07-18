# Local screen-capture contract evidence

This record covers the provider-free issue-24 contract plus repository-local
source evidence for the narrow `macos-native` display-video composition. It
contains no physical screen recording, observed OS permission prompt, customer
media, device label, window title, process identity, or platform certification.

## Closure ledger boundary

Issue 24 checkboxes 1–10 remain unclosed in the closure ledger. This evidence
file does not reclassify any checkbox. The new macOS source and desktop
composition materially implement one narrow full-display path, but the issue
acceptance criteria also require window/region behavior, cursor and lifecycle
parity, protected-content semantics, representative hardware, performance,
cross-platform coverage, and an issue-04 parity recording.

The provider-neutral tests below still compile a dummy
`ScreenCaptureSource` using only the exported API. Their simulated frames,
permission events, geometry, cursor, recovery, copy-budget, and exclusion
results remain invalid as physical or parity evidence. The production macOS
desktop does not implement that complete provider-neutral contract: it uses a
smaller ScreenCaptureKit source to feed the owned GStreamer recorder directly.
That deliberate composition must not be generalized into issue-24 closure.

The focused contract gates cover:

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

## Native macOS display-only source evidence

Repository inspection and focused checks establish the following local source
facts, not hardware results:

- `frame-macos-screen-capture` provides an unsafe-free ScreenCaptureKit source
  with permission preflight/request, bounded opaque display enumeration,
  BGRA/sRGB frames, embedded-or-hidden cursor policy, a bounded nonblocking
  callback queue, explicit stop, unchanged-screen `Idle` frame repetition,
  bounded stop-tail draining, and privacy-safe diagnostics;
- its display filter excludes the entire current Frame application by exact
  process identity, including later-created windows, and fails closed when
  ownership is missing or ambiguous;
- the `macos-native` desktop feature constructs
  `MacOsNativeDesktopBackend` only after GStreamer recorder preflight, reports
  `NativeMacOsDisplay`, and degrades to `Unavailable` if construction fails;
- the runtime requires granted permission and a fresh opaque display selection,
  rejects audio, camera, window, region, pause, and MP4 paths, then feeds the
  selected full display into the bounded VP8/WebM `ScreenRecording` graph; and
- stop seals a single-source artifact, cancel tears down the worker, and
  Editable WebM export is bound to that artifact and a prevalidated destination;
  recorder writes/verifies use a preopened descriptor, publication is a rooted
  identity-checked no-replace rename, and seal/export both enforce the verified
  SHA-256; media, recordings, export, and private staging descriptors are pinned,
  visible directory replacements fail closed, and export rehashes the retained
  descriptor after publication; and
- worker health is polled once per second while Recording, and the first slice
  fails closed at four hours, 2 GB, or a 512 MB filesystem reserve.

These facts do not prove that ScreenCaptureKit returned a display or frame on a
real machine, that the permission prompt behaved correctly, that Frame windows
were absent from recorded pixels, that a written WebM was viewed or decoded in
this desktop journey, or that performance and lifecycle bounds hold on
supported hardware.

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

# macOS source and desktop composition checks.
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  cargo test --locked -p frame-macos-screen-capture --all-targets
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  cargo test --locked -p frame-media --test screen_recording_contract
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  cargo test --locked -p frame-desktop-core \
  --features tauri-app,macos-native --all-targets

# Production-mode macOS composition smoke; this does not start capture.
python3 scripts/ci/build-desktop-ui.py
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display
```

The complete media test suite remains the integration gate because this module
reuses the existing frame timing, cancellation, runtime capability, and video
format contracts.

For a physical local recording and artifact probe, follow
[`docs/operations/macos-display-recording-local.md`](../operations/macos-display-recording-local.md).
That run validates the narrow display-video adapter, not the complete issue-24
or Studio contracts.

## Evidence not present

| Gate | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Native source and release composition | source wired; physical run pending | pending | pending |
| Permission preflight, prompt, denial, settings, revocation | preflight source wired; observed flow pending | pending | pending |
| Physical display/window/region samples | pending | pending | pending |
| Multi-monitor negative origins, mixed/fractional DPI, rotations | pending | pending | pending |
| Cursor image/position/click parity and clipping | pending | pending | pending |
| Frame UI/window exclusion recording | pending | pending | pending |
| Unplug, close/minimize, hotplug, sleep/wake, protected content | pending | pending | pending |
| Zero-/bounded-copy buffer lifetime and latency/CPU/GPU/memory measurements | pending | pending | pending |
| Cap-baseline and issue-04 fixture parity | pending | pending | pending |

No pending row in this table may be inferred from a unit test or an
enum-to-source mapping. Before the OS/architecture/device matrix can produce
valid acceptance evidence, Frame must exercise the wired macOS path and still
implement the missing window/region, lifecycle, protected-content, cursor,
cross-platform, performance, and parity behavior represented by checkboxes
1–10. Recorded samples, probes, measurements, operational documentation, and
rollout evidence remain subsequent gates rather than substitutes for that code.
