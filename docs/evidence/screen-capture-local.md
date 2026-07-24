# Local screen-capture contract evidence

This record covers the provider-free issue-24 contract plus repository-local
source evidence for the narrow `macos-native` and `windows-native`
display-video compositions. It contains no physical
screen recording, observed OS permission prompt, customer media, device label,
window title, process identity, or platform certification.

## Closure ledger boundary

The semantic closure audit classifies Issue 24 checkboxes 1–3 as locally
satisfied, checkboxes 5–7 and 9–10 as protected pending, and checkboxes 4 and 8
as true local gaps. Local satisfaction is repository evidence, not production
sign-off. The macOS and Windows sources and screen-only desktop compositions
implement display, privacy-filtered window, and single-display-region targets
through the normalized provider-neutral ingress.
The issue acceptance criteria still require lifecycle parity, representative
hardware, performance, protected-content observations, and an issue-04 parity
recording. Linux is a chartered preview rather than an initial release target,
so its absent adapter is not used to weaken the macOS/Windows release claim.

The provider-neutral tests below still compile a dummy `ScreenCaptureSource`
using only the exported API. Their simulated frames, permission events,
geometry, cursor, recovery, copy-budget, and exclusion results remain invalid
as physical or parity evidence. The production macOS and Windows screen-only
workers bind `MacOsNormalizedScreenCaptureSource` and
`WindowsNormalizedScreenCaptureSource`, negotiate the exact catalog and CPU
BGRA appsrc plan, and drain `ScreenCaptureIngress` through one shared
`ScreenRecordingPump` worker. The optional macOS system-audio worker still owns
the earlier direct screen source until Issue 25 connects the provider-neutral
A/V contract. Neither local path may be generalized into complete issue-24
closure.

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
- a timestamp-preserving, non-blocking, CPU-copy-only appsrc ingress plan with
  exact owned allocations; sealed exact-allocation frame and cursor payloads
  checked at construction and queue/cache admission; rejection when a CPU
  payload claims a native-memory frame type; an exact
  session/ingress/stream-bound recording pump that exclusively borrows
  ingress, with the smaller upstream/appsrc iteration limit, pre-pop
  actual-byte/duration downstream-capacity checks, cumulative submission
  accounting, and multi-drain backlog coverage; opaque graceful-Stop
  request/completion proofs that require an empty upstream queue and an
  unchanged exact seal epoch/transition revision plus native Stop
  acknowledgement before artifact finalization; rejected pre-mutation
  Stop/abort correlations that return their exact one-shot proof for the
  correct result; rejection of zero-frame EOS/failed graphs during pump
  construction; compile-fail duplicate-pump/completion-reuse proofs; and
  cancellation, terminal graph failure, suspended Stop retry, and stale
  old-ack tests proving those paths preserve Stop/teardown evidence and cannot
  publish a partial segment;
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

## Native macOS target source evidence

Repository inspection and focused checks establish the following local source
facts, not hardware results:

- `frame-macos-screen-capture` provides an unsafe-free ScreenCaptureKit source
  with permission preflight/request, bounded opaque display/window/region
  enumeration, BGRA/sRGB frames, embedded-or-hidden cursor policy, a bounded
  nonblocking callback queue, explicit stop, unchanged-screen `Idle` frame
  repetition, bounded stop-tail draining, and privacy-safe diagnostics;
- display and region filters exclude the entire current Frame application by
  exact process identity, including later-created windows, and fail closed
  when ownership is missing or ambiguous; the window catalog omits all Frame
  windows and a selected non-Frame window is isolated by its exact binding;
- the `macos-native` desktop feature constructs
  `MacOsNativeDesktopBackend` only after GStreamer recorder preflight, reports
  `NativeMacOsDisplay`, and degrades to `Unavailable` if construction fails;
- the runtime requires granted permission and a fresh opaque display, window,
  or user-defined region selection; screen-only recording uses the normalized
  owner-bound ingress and pump, while optional exact 48 kHz stereo system audio
  still uses the separately bounded direct A/V worker; both feed VP8/Opus WebM,
  and the A/V path excludes Frame's own process audio; and
- stop seals one verified recording artifact, cancel tears down the worker, and
  Editable WebM export is bound to that artifact and a prevalidated destination;
  recorder writes/verifies use a preopened descriptor, publication is a rooted
  identity-checked no-replace rename, and seal/export both enforce the verified
  SHA-256; media, recordings, export, and private staging descriptors are pinned,
  visible directory replacements fail closed, and export rehashes the retained
  descriptor after publication; and
- worker health is polled once per second while Recording, and the first slice
  fails closed at four hours, 2 GB, or a 512 MB filesystem reserve.

Repository-local pump tests additionally establish that the negotiated
CPU/BGRA/sRGB plan configures explicit appsrc colorimetry and timing, payload
ownership transfers into a GStreamer buffer until its final reference is
released, exact frame/cursor allocation is authenticated before retention,
one drain is bounded by the negotiated queue frame count, an over-time frame
remains upstream, sequence loss becomes `DISCONT`, graceful retirement retains
the graph for EOS and
verified finish, and cancellation confirms Null while preserving the exact
native Stop transition. The exclusive pump borrow prevents a competing pop or
second graph. Opaque publication, retry, and abort proofs retain their actions;
an epoch race or terminal graph failure can only abort and reports whether
teardown was confirmed.

The shared source contract now exposes a bounded post-stop event channel, and
the pump requires every retained ScreenCaptureKit tail frame before encoder
EOS. ScreenCaptureKit still cannot truthfully advertise exact protected-content
events from ambiguous `Blank`/`Suspended` statuses. The normalized adapter
instead advertises a content-unavailable failure and negotiates only the
fail-session policy; the production screen-only worker aborts rather than
mislabeling or silently encoding ambiguous content.

The macOS adapter also implements metadata-mode cursor sampling without adding
an input-monitoring package: Core Graphics supplies checked visibility,
desktop-logical position, and primary/secondary button state; AppKit supplies
the current image and hotspot. Image changes are fingerprinted, decoded into
one exact bounded RGBA allocation, and emitted before the first frame that
references the new revision. Window coordinates are converted to frame-local
physical coordinates, display/region coordinates use the canonical geometry
transform, activity outside the selected target becomes a fully hidden sample,
and retained stop-tail frames fail closed to hidden cursor metadata rather than
inventing a stale image or position. The default production recording request
continues to use an embedded cursor until Studio persists the separate cursor
track.

These facts do not prove that ScreenCaptureKit returned a display or frame on a
real machine, that the permission prompt behaved correctly, that Frame windows
were absent from recorded pixels, that a written WebM was viewed or decoded in
this desktop journey, or that performance and lifecycle bounds hold on
supported hardware.

## Native Windows target source evidence

Repository inspection and focused checks establish the following local source
facts, not Windows hardware results:

- `frame-windows-screen-capture` provides an unsafe-free Windows Graphics
  Capture adapter for display, privacy-filtered window, and display-relative
  region targets; its separate `frame-windows-capture-ffi` crate is the only
  pointer-level Win32 boundary and exposes no window title, process name,
  device name, raw handle, or pointer;
- native target identities are converted to session-secret HMAC tokens, exact
  display geometry includes DPI and rotation, and Frame's own process windows
  are omitted from the catalog before a normalized target can be selected;
- WGC frames are copied into exact CPU BGRA/sRGB allocations, timestamped on a
  monotonic capture timeline, and handed off through a three-frame
  nonblocking channel with explicit drop diagnostics; regions are cropped only
  after exact source-dimension validation;
- startup and teardown have caller-provided deadlines, a dedicated worker owns
  the WGC message loop, `WM_QUIT` requests shutdown, and any retained stop tail
  is bounded before it enters the shared `ScreenCaptureSource` contract; and
- the Windows CI lane compiles and lints both target-gated crates on a native
  Windows runner, while the portable desktop dependency gate rejects either
  native crate or `wgc` from the default shell closure;
- the `windows-native` desktop feature asks Tauri to protect Frame's main
  window before constructing `WindowsNativeDesktopBackend`, reports
  `NativeWindowsDisplayWindowRegion`, and degrades to `Unavailable` if window
  protection, GStreamer preflight, private-root setup, or source construction
  fails;
- the backend exposes only fresh opaque display/window/region selections,
  rejects system audio, and feeds the normalized Windows source through the
  same bounded ingress/pump and VP8/WebM recording path as screen-only macOS;
  and
- recording and export roots use private Windows DACLs, no-reparse-point opens,
  verified artifact hashes, and atomic publication before a completed export
  is reported.

The adapter advertises bounded cursor metadata but deliberately does not
advertise topology recovery. The existing audited Win32 FFI boundary samples
cursor visibility, physical desktop position, primary/secondary button state,
image changes, and hotspot without exposing an `HCURSOR` or pointer identity.
Only changed images cross into the safe adapter as exact bounded BGRA
allocations; display, DWM-window, and region geometry are converted to
frame-local coordinates before the provider-neutral privacy clipping and
revision rules run. Image updates precede referencing frames, and stop-tail
metadata fails closed to hidden for the same reason as macOS. The default
production recording request remains embedded-cursor until Studio owns a
separate cursor track.

Its explicit protected-content policy relies on the operating
system's public-capture redaction contract rather than inventing a detection
event, and the composition refuses to start unless Frame's own window has been
placed behind Tauri's content-protection boundary. Those are source and
composition contracts, not physical proof that every protected surface or
Frame pixel was absent on a supported Windows build. System audio, microphone,
camera, pause/resume, durable recovery, and Studio export remain unavailable.

Reproduce the focused tests and lint gate with:

```sh
export GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)"

cargo test --locked -p frame-media --test screen_capture_contract
cargo test --locked -p frame-media --doc
cargo clippy --locked -p frame-media --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p frame-media --no-deps
rustfmt --edition 2024 --check \
  crates/media/src/capture.rs \
  crates/media/src/screen_capture.rs \
  crates/media/tests/screen_capture_contract.rs
scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets

# macOS source and desktop composition checks.
cargo test --locked -p frame-macos-screen-capture --all-targets
cargo test --locked -p frame-media --test screen_recording_contract
cargo test --locked -p frame-desktop-core \
  --features tauri-app,macos-native --all-targets

# Production-mode macOS composition smoke; this does not start capture.
python3 scripts/ci/build-desktop-ui.py
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display

# Run these target-gated checks on a native Windows host. DOCS_RS skips the
# GStreamer link probe for cargo check; it does not constitute capture evidence.
export DOCS_RS=1
cargo check --locked \
  -p frame-windows-capture-ffi -p frame-windows-screen-capture --all-targets
cargo clippy --locked \
  -p frame-windows-capture-ffi -p frame-windows-screen-capture \
  --all-targets --no-deps -- -D warnings
cargo check --locked -p frame-desktop-core \
  --features windows-native,custom-protocol --all-targets
cargo clippy --locked -p frame-desktop-core \
  --features windows-native,custom-protocol --all-targets --no-deps -- \
  -D warnings

# Windows production composition smoke; this does not start capture.
cargo build --locked --release -p frame-desktop-core \
  --features windows-native,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py \
  --expected-adapter native_windows_display_window_region
```

The complete media test suite remains the integration gate because this module
reuses the existing frame timing, cancellation, runtime capability, and video
format contracts.

The provider-neutral path currently rejects CoreVideo, D3D11, and DMA-BUF
requests even when a platform capability profile describes them. Native-memory
zero-copy requires a future safe, bounded lease/accounting contract; the
present evidence proves only exact CPU allocation ownership after any adapter
copy.

For a physical local recording and artifact probe, follow
[`docs/operations/macos-display-recording-local.md`](../operations/macos-display-recording-local.md).
That run validates one selected macOS target through the local production
composition, not the complete issue-24 or Studio contracts.

## Evidence not present

| Gate | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Native source and release composition | display/window/region screen-only source wired; physical run pending | display/window/region screen-only source wired; physical run pending | preview; outside the initial release matrix |
| Permission preflight, prompt, denial, settings, revocation | preflight source wired; observed flow pending | WGC availability preflight implemented; observed flow, settings, and revocation pending | pending |
| Physical display/window/region samples | pending | pending | pending |
| Multi-monitor negative origins, mixed/fractional DPI, rotations | pending | pending | pending |
| Cursor image/position/click parity and clipping | metadata adapter wired; physical parity pending | metadata adapter wired; physical parity pending | preview |
| Frame UI/window exclusion recording | pending | pending | pending |
| Unplug, close/minimize, hotplug, sleep/wake, protected content | pending | pending | pending |
| Native-memory zero-copy lifetime and latency/CPU/GPU/memory measurements | pending | pending | pending |
| Cap-baseline and issue-04 fixture parity | pending | pending | pending |

No pending row in this table may be inferred from a unit test or an
enum-to-source mapping. Before the OS/architecture/device matrix can produce
valid acceptance evidence, Frame must exercise the wired macOS and Windows
targets, implement the missing lifecycle/recovery behavior, and complete the performance
and parity work represented by checkboxes 1–10. Recorded samples, probes,
measurements, operational documentation, and rollout evidence remain subsequent
gates rather than substitutes for that code.
