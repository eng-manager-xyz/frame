# Desktop product contract v1

Issue 33 owns the Leptos/Tauri recorder and editor boundary, while issues 24–27
own capture and media behavior. The portable contract composes bounded macOS
and Windows display/window/region video adapters without allowing a WebView to
acquire filesystem, shell, device, tray, updater, or arbitrary Tauri authority.
macOS also connects optional normalized A/V inputs, Studio recovery,
preview/edit, and coordinated export. Physical device/provider/release evidence
remains separate from these repository-local implementations.

## Backend truth

The native `DesktopRuntime` is the only authority for recorder, device, recovery, editor, export,
upload, settings, lifecycle, and updater state. The Leptos client sends a versioned
`RequestEnvelope`, waits for `DesktopDispatch`, then replaces its render snapshot. A click never
optimistically changes a success state. Recorder start, pause, resume, stop, editor revision changes,
verified upload parts, export progress, and recoverability all originate in checked backend events.

There are four Tauri commands:

- `bootstrap_main` reports the shell and protocol compatibility marker;
- `bootstrap_desktop` returns opaque logical-window scopes and the first redacted snapshot;
- `dispatch_main` decodes one bounded JSON envelope and emits `frame-desktop://event-v1` events; and
- `finalize_instant` accepts only a strict opaque-handle/sequence envelope and returns the exact
  shared Instant progress projection. The release provider is explicitly `NotConfigured`, so no
  finalize network request can start until a native authenticated journal owner binds authority.

All product operations are variants of the Rust `IpcCommand` enum. Unknown commands, malformed or
oversized JSON, unsupported protocol versions, duplicate request IDs, replayed/gapped sequences,
stale settings/editor/update revisions, and invalid payloads fail closed.

## Composition and adapter truth

Adapter truth is a build- and startup-time property, not a UI inference:

- a release build with `tauri-app,custom-protocol` is the portable shell and
  selects `DesktopAdapterKind::Unavailable` on macOS and Windows;
- a macOS release build that also enables `macos-native` requests
  `DesktopAdapterKind::NativeMacOs`, runs the trusted GStreamer bootstrap, and
  constructs `MacOsNativeDesktopBackend`; failed construction degrades to
  `Unavailable` before the runtime is exposed to the WebView;
- a Windows release build that enables `windows-native` requests
  `DesktopAdapterKind::NativeWindows`, protects Frame's main WebView from
  public capture before constructing `WindowsNativeDesktopBackend`, and
  degrades to `Unavailable` if either operation fails; and
- only a debug build with `FRAME_DESKTOP_FAKE_PIPELINE=1` selects
  `DeterministicFake`.

`bootstrap_main` derives `RecorderAdapterState` from the runtime snapshot, so
the WebView observes `Unavailable`, `DeterministicFake`,
`NativeMacOsDisplay`, or `NativeWindowsDisplayWindowRegion` consistently with
`bootstrap_desktop`. Native commands use
`dispatch_native_json` only while that same runtime snapshot names
`NativeMacOs` or `NativeWindows`; the portable shell continues through the
non-native dispatcher.

The native macOS implementation performs GStreamer factory preflight before
backend construction, uses ScreenCaptureKit permission preflight/request, and
enumerates bounded privacy-safe display and non-Frame-window summaries. The
user may select a display/window or define a single-display region through the
bounded numeric picker. Screen-only recording binds that exact catalog target
to the normalized `ScreenCaptureSource`/ingress/pump path and records BGRA/sRGB
video with an embedded cursor. Display/region filters exclude the whole Frame
application; the window catalog and filter never target a Frame window. The
optional exact 48 kHz stereo system-audio path remains the direct Issue 25 A/V
worker and excludes Frame's own process audio. ScreenCaptureKit `Idle`
callbacks repeat the last valid Complete frame at the nominal cadence,
including the bounded stop tail, so unchanged time remains in the media
timeline.
The recorder writes and verifies through a preopened descriptor, publishes the
sealed inode with a rooted no-replace rename, retains its SHA-256, and copies
exports through rooted descriptors while checking that digest. Media,
recordings, Studio, export, and private export-staging directories stay pinned for the
backend lifetime; their visible identities are revalidated around publication
so a rename or real-directory replacement fails closed instead of producing a
false path. Export keeps its staging descriptor through the cross-root rename
and rehashes the published inode. A bounded health poll reconciles
terminal worker failures without leaving the UI in Recording. The first slice
is capped at four hours, 2 GB, and a 512 MB filesystem reserve. The Instant
path rejects microphone, camera, pause/resume, and MP4. Its export is
artifact-backed screen-plus-optional-system-audio WebM, not the canonical
Studio edit plan. The flattened Instant artifact is not a Studio project.
When the backend-bound request selects Studio mode, the same worker also feeds
the corrected screen timeline and any included system audio into independent
bounded VP8/Opus branches. Those branches seal recoverable temporary originals
under the pinned private Studio root, advance the durable journal, commit
immutable originals, and create a canonical project. The editor opens that
project only through a reminted descriptor-authenticated handle, compiles
bounded edits in Rust, and saves through an edit-save journal transaction. A
clean aligned screen-plus-optional-audio project can now invoke the native
edit-aware adapter for Editable WebM, archive Matroska, or the explicitly
approved H.264/AAC distribution MP4 profile. Export reauthenticates every
original sidecar, opens and hashes each original through the pinned Studio
directory, and retains those exact descriptors while the seekable decoder reads
them through `/dev/fd`. It renders to a preopened private staging inode, then
publishes and rehashes that inode through the pinned export directory. Native
camera layouts, cursor masks, and cursor metadata are baked into each aligned
BGRA buffer from its output timestamp; the first video buffer is anchored to
the exact edit-window boundary, and per-window audio gain is installed behind
the decoder segment-start barrier. Queued media therefore cannot observe a
later edit window. Multi-asset segmented-source assembly remains fail-closed.
Distribution and archive export dispatch through the durable
`StudioRenderCoordinator`: a bounded worker publishes phase/basis-point
progress through the payload-free `ExportPoll` command, and cancellation does
not complete until the worker stops, its exact fenced staging identity is
deleted, and absence is proven. The final inode is published only after the
coordinator has accepted 10000 basis points; its checksum/length receipt is
CAS-persisted before authority is released. Hardware encoder selection,
identical-plan software fallback, and native post-restart render adoption
remain fail-closed.

The native Windows implementation uses Windows Graphics Capture permission
availability and bounded privacy-safe display/non-Frame-window enumeration.
The user may select a display/window or define a single-display region. It
records CPU BGRA/sRGB frames through the same normalized
`ScreenCaptureSource`/ingress/pump path and VP8/WebM graph. Construction fails
closed unless Tauri successfully protects Frame's own WebView; the WGC source
uses the operating system's public-capture redaction policy for protected
content. Recording and export use private DACL roots, reparse-point-safe opens,
verified hashes, and atomic publication. The current Windows slice rejects all
audio, microphone, camera, pause/resume, MP4, recovery, and Studio operations.

## Window ownership

Tauri creates three physical WebViews: `main`, `overlay`, and
`target-picker`. The main WebView multiplexes the product's logical scopes;
the two auxiliary WebViews can use only their matching logical role. Bootstrap
creates independent logical scopes, each with a different opaque window and
session token, monotonic request sequence, command allowlist, and path policy.

| Logical owner | Allowed authority |
| --- | --- |
| Main | open known surfaces, enumerate/scan, lifecycle, update |
| Recorder | capture configuration/target/device, recording, recorder upload |
| Recovery | scan, inspect, open, or explicitly discard recovery copies |
| Editor | open/revision-fenced edit/save/export/upload |
| Export | export start/cancel only |
| Settings | revision-fenced settings and approved presets |
| Overlay | pause/resume/stop/cancel and overlay lifecycle only |
| Target picker | enumerate/select bounded capture targets and picker lifecycle only |

A command copied to another logical scope is rejected before state mutation. Backend events carry an
explicit owner. Closing or hiding UI surfaces never rewrites recorder/editor truth.

## Filesystem and capability boundary

Project reads/deletes, media reads, and export writes accept only normalized absolute paths beneath
the native roots assigned to that logical owner and only approved extensions. Validation rejects
parent/current components and requires adapters to open using no-follow/reparse-point-safe semantics
and re-check the resolved handle. The deterministic fake never opens these paths.

`capabilities/main.json` grants only the four commands above to window `main`. The product commands
have isolated allow and deny permissions, including the generated `finalize_instant` permission.
The WebView CSP has no remote scripts,
general `unsafe-eval`, object embedding, base URI, ancestor framing, or broad network origin.

## Recorder and project state

The portable contract represents Instant/Studio mode, bounded countdown, display/window/region targets,
Frame-window exclusion, permission state, typed device counts and selection, microphone/system-audio
meters, camera activity, pause/resume/stop/cancel, recovery copies, revision-fenced trim/save,
monotonic export progress, verified multipart upload progress/pause/resume, settings/presets, and
update/relaunch state. Instant publication status uses the shared versioned
phase/progress/retry/error DTO. Active work has determinate or indeterminate
progress, terminal states remove the opaque WebView handle, and stable error
copy is announced without exposing credentials or recording identity.

Representation is not implementation. `NativeMacOsDisplay` and
`NativeWindowsDisplayWindowRegion` currently enable
permission preparation, display/window refresh and selection, bounded region
definition and selection, target-video start, stop, cancel, and Editable WebM
export. macOS alone optionally supports exact 48 kHz stereo system audio; the
Windows composition rejects audio. The macOS Studio path also commits verified
immutable originals, a durable journal, and a canonical revision-one project.
A descriptor-rooted bounded catalog maps those artifacts to fresh opaque
WebView tokens; the editor can open only a completed project after the journal
and project are re-read and authenticated. Interrupted recording/edit-save
entries can be inspected and reconciled through a new journal fence; only a
proven-empty attempt can be archived, without deleting graph or media files.
The native editor persists compiled trim/delete/split/speed/audio-gain drafts.
A clean aligned project can publish verified edit-aware distribution,
editable, native-master, and archive profiles from the Leptos editor. The
durable render coordinator exposes bounded progress through `ExportPoll`,
confirms cancellation only after worker termination and exact partial cleanup,
and records the terminal inode/hash/length receipt before releasing authority.
The combined optional-input bridge and canonical compositor connect
microphone, system audio, camera, cursor, background, and layout state to
recording, preview, and export.

The Leptos product uses native buttons, fieldsets, labels, meters, progress elements, headings,
landmarks, a polite atomic status region, an assertive modal error surface, visible focus, a skip
link, forced-color support, and reduced-motion rules. The essential timeline trim has labeled
numeric controls, so drag gestures are not required. The portable accessibility model separately
checks deterministic tab order, keyboard shortcuts, accessible names, value text, modal focus trap,
focus restoration, and Escape dismissal.

CI additionally builds the release Leptos assets and drives the rendered
product with Chrome DevTools keyboard events only. A loopback bridge forwards
each action to a bounded, fake-backed `DesktopRuntime` host, so the journey
executes the real versioned Rust dispatch boundary while checking the rendered
accessibility tree and DOM semantics. This is executable browser evidence, not
a VoiceOver, Narrator, or physical-device result.

## Hotkey, tray, overlay, and update lifecycle

Lifecycle transitions are typed and backend-owned. The fake adapter proves ownership and state
reconstruction without pretending to call OS APIs. The portable release shell
selects `Unavailable`; the `macos-native` and `windows-native` release
compositions select their narrow target adapters when backend construction
succeeds; and only a debug build with
`FRAME_DESKTOP_FAKE_PIPELINE=1` can select the deterministic fake. Production
shell code registers OS-specific global shortcuts, installs the tray menu,
binds three physical windows to their logical owners, hides known windows on
close, reopens/focuses them through typed actions, content-protects all Frame
windows, and positions overlay/target-picker surfaces relative to the main
window's current monitor. Pure tests cover every shortcut/tray/close mapping,
unknown labels, negative monitor origins, oversized windows, and coordinate
overflow.

The entire current Frame application is excluded inside the ScreenCaptureKit
display filter, including windows created after capture starts, but that
source-level invariant is not physical exclusion-recording
evidence by itself.

The protected product hardware mode closes the repository-local harness gap
without promoting physical results. It is compiled into both native signed
applications and requires a protected-runner marker, a per-run CSPRNG token,
and exact source/run/signature metadata. The normal Tauri setup still creates
the backend, windows, hotkeys, and tray. A bounded driver then exercises the
same handler functions, OS close requests, and physical monitor moves. It
places per-run-random solid colors in always-on-top Frame windows on every
accepted monitor, records through the native excluded-window path, and decodes
one bounded RGB frame; matching sentinel pixels fail closed. The evidence
contains only strict case booleans and monitor/scale/rotation counts.

The independent validator binds macOS cells to the certificate-backed bundle,
Apple team, designated requirement, and executable digest; Windows cells bind
to a valid Authenticode certificate thumbprint/certificate digest and
executable digest. It accepts only single-standard, dual mixed-scale, or
rotated topology cells. Actual macOS and Windows execution of all cells
remains protected evidence for the broader window/lifecycle acceptance gate.

## Fake adapter

The fake adapter supplies two displays, two microphones, one system-audio source, one camera, opaque
targets, one read-only recovery fixture, a 90-second revision-fenced project, export progress, and a
four-part verified upload. It exercises device loss, crash/restart, close/reopen state, and the
check/install/relaunch update lifecycle. Its journey paths are present only in fake bootstrap and are
never rendered or logged.

## Legacy safety and rollback

Legacy settings and project headers are inspected into explicit compatible, migratable,
needs-review, unsupported, or invalid reports. Inspection never mutates input, every proposed project
plan preserves the original, and unknown settings/effects require review rather than silent loss.
The state model retains a legacy-selector flag and rollback contract. The
native shell wires an explicit previous-channel check and install through a
dedicated control-plane route backed by the retained R2 `previous.json`
pointer. The control plane returns only a version strictly older than the running
desktop, and Tauri verifies the retained artifact signature before installing
it. Browser code never receives an updater URL, signature, or filesystem path.
Rollback changes the signed desktop release selector and does not rewrite
projects produced by either desktop. The previous desktop remains selectable
until parity gate 29 is approved.

## Production-mode build and smoke

From the repository root, build and smoke the portable shell separately from
the native OS composition:

```sh
# Cross-platform portable shell; recorder truth is Unavailable.
python3 scripts/ci/build-desktop-ui.py
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter unavailable

# macOS only; recorder truth is NativeMacOsDisplay if backend construction succeeds.
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display

# Windows only; recorder truth is NativeWindowsDisplayWindowRegion if
# WebView protection and backend construction succeed.
cargo build --locked --release -p frame-desktop-core \
  --features windows-native,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py \
  --expected-adapter native_windows_display_window_region
```

The smoke proves the production-CSP WebView reaches the allowlisted Rust
bootstrap and reports coherent adapter truth. It does not grant screen-recording
permission, enumerate a physical display, record a frame, inspect the output,
exercise recovery, or provide accessibility or distribution evidence.

The separate deterministic browser journey exercises the production Leptos
assets and Rust state machine without native hardware:

```sh
cargo build --locked --release -p frame-desktop-core \
  --bin frame-desktop-e2e-host
python3 -I scripts/ci/desktop-browser-journey.py \
  --evidence target/evidence/desktop-browser-journey-local.json
```

It proves keyboard activation and rendered semantic invariants across the
portable fake journey. It makes no assistive-technology or hardware claim.

The separate [local macOS recording runbook](../operations/macos-display-recording-local.md)
builds the `.app` and exercises the narrow real display-video path. A successful
run proves that slice only; it does not close the complete issue-24, issue-27,
or issue-33 product contracts.
