# Desktop recovery and release runbook

## Build modes and current boundary

The portable Tauri shell and the native OS target-capture slices are different
release-mode builds. Build and smoke them separately from the repository root:

```sh
python3 scripts/ci/build-desktop-ui.py

# Portable macOS/Windows shell; recorder adapter truth is Unavailable.
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter unavailable

# macOS only; requests the preflight-backed NativeMacOsDisplay adapter.
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display

# Windows only; requests NativeWindowsDisplayWindowRegion after protecting
# Frame's main WebView from public capture.
cargo build --locked --release -p frame-desktop-core \
  --features windows-native,custom-protocol --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py \
  --expected-adapter native_windows_display_window_region
```

Each native slice records one selected display, non-Frame window, or bounded
single-display region as VP8/WebM and embeds the cursor. Display/region capture
excludes Frame's whole application on macOS; Windows construction requires
Tauri's main-window content protection, and both catalogs omit Frame windows.
Screen-only recording uses the shared normalized capture ingress/pump. The
macOS A/V worker can mux exact 48 kHz stereo system audio as Opus while
excluding Frame's own process audio, and its Studio composition can add
normalized microphone and camera inputs. Windows rejects all audio and Studio
input operations. Both target-video slices support stop, cancel, and
artifact-bound Editable WebM publication; macOS additionally supports
pause/resume. The macOS Studio composition persists isolated originals, a
journal, and a canonical project. Its descriptor-rooted bounded catalog opens
only reauthenticated completed projects and reports interrupted entries as
recovery-required. Recording and edit-save boundaries reconcile through a new
journal fence and reminted opaque handle. Only a proven-empty attempt can be
archived; its journal is moved without deleting media, a graph, or a completed
project. Authenticated preview/edit/save feeds the durable render coordinator,
which supports progress, confirmed cancellation, edit-aware WebM/MP4/archive
profiles, compositor state, and hardware-to-software encoder fallback.

The Rust-owned shell registers platform global shortcuts, tray actions, and
three content-protected physical windows; it hides/reopens known windows and
positions the overlay and target picker relative to the current monitor. It
also supports signed forward and previous-desktop update
check/install/relaunch. Representative physical hotkey/tray interaction,
Frame-window exclusion, multi-monitor placement, updater relaunch, and
distribution evidence remain protected or part of the pending full-product
hardware matrix.

The smoke confirms only the production-CSP WebView-to-Rust bootstrap and
coherent adapter truth. The separate
`scripts/ci/desktop-browser-journey.py` lane executes keyboard-only release UI
actions through the real Rust dispatch boundary and checks rendered semantics.
Neither lane is capture, playback, named screen-reader, signing, notarization,
clean-machine, or distribution evidence.
The current `.app` can use the build-time GStreamer installation only while it
remains beneath the checkout's canonical `target` tree; Issue 22 still blocks a
distributable app-relative runtime.

## Release prerequisites

1. Build the exact commit with the pinned Rust and Trunk versions, record
   whether it is the portable or `macos-native` composition, and retain binary,
   bundle, CSP, and capability digests.
2. Run the portable core tests, fake desktop journey, strict clippy, deterministic bundle checker,
   and desktop product/accessibility checker.
3. Run `.github/workflows/desktop-real-hardware.yml` on the protected macOS
   display runner and retain its complete non-fake JSON trace. This lane is
   deliberately narrower than the full desktop matrix; Windows,
   microphone/camera, system-audio playback, Studio, updater, recovery, and
   accessibility hardware gates remain pending.
4. Name the macOS VoiceOver and Windows Narrator versions used for the keyboard/screen-reader
   journeys. Record OS build, architecture, monitor topology/DPI/rotation, device models, permission
   reset procedure, and binary digest.
5. Keep the legacy desktop selector enabled. Gate expansion per OS and recording mode; do not infer
   parity on one platform from the other.

## Crash and recovery

This section is the required release behavior. The current `macos-native`
Studio path writes a durable journal, classifies exact journal/project pairs
through the pinned projects directory descriptor, and opens only a
reauthenticated completed project. It inspects interrupted entries through
generation-fenced opaque handles. A recovery worker takes a new ownership
fence, reconciles the exact persisted graph and immutable originals, seals
remaining recording tracks, creates the idempotent project, and reconciles
edit-save precommit or lost-acknowledgement states. An archive action is
available only for a proven-empty `Created` or `RecordingGraphPrepared`
attempt, and it moves only the journal. The flattened target-capture path still
has no persisted recording recovery. A process crash must not be advertised as
fully validated until the protected crash/hardware matrix exists.

When the UI disappears or the process restarts, native journal state remains authoritative. The UI
must not claim that recording stopped or continued until a backend event says so.

1. Reconstruct the main window and request a fresh backend snapshot.
2. If the journal reports active capture without a live adapter, move to `recoverable`, hide the
   overlay, zero visual meters, and announce recovery availability.
3. Scan recovery roots read-only. Inspect integrity and the coarse recovery
   action before offering mutation.
4. Recover through the exact journal/graph/original identities and remint the
   project handle. Offer archive only for a backend-proven empty attempt; never
   describe archive as deleting captured media.
5. Never log session IDs, opaque device/target tokens, project paths, tenant data, or backend error
   strings.

Device loss follows the same rule: the backend emits `device_lost`; recording becomes recoverable,
permissions return to not-determined, and the UI offers device refresh/recovery rather than success.

## Error and stale-state response

- `invalid_request`: keep the last confirmed snapshot and explain that the action is unavailable.
- `forbidden`: record the bounded public code, revoke/recreate the affected logical window scope, and
  investigate cross-window misuse.
- `conflict`: fetch a fresh snapshot; never retry an edit, settings save, or updater action against a
  stale revision automatically.
- `unavailable`: keep privileged controls disabled and retain the legacy selector.
- `internal`: preserve project/journal data, offer a bounded retry only where the backend marks it
  retryable, and collect native diagnostics outside the WebView.

## Real-hardware gate

The checked-in protected workflow, `scripts/ci/run-desktop-real-hardware.py`,
and `scripts/ci/desktop-real-hardware.py` produce and validate evidence only
for the exact partial
`macos_display_webm_v1` capability: a preauthorized ScreenCapture TCC state,
display catalog/selection, display capture with Frame's application-exclusion
filter configured, playable stopped/exported WebM, and cancel cleanup. The
repository does not provide the
driver as an external executable. Instead, a token-gated driver is compiled
into the certificate-signed Frame binary and is unreachable unless both the
protected argument and environment marker are exact.
The workflow and validator are not evidence that a physical run occurred.
Submitted evidence must state
`full_product_gate: not_claimed`; the validator deliberately has no full-product
mode.

The protected runner must be a persistent, logged-in macOS account with an
unlocked Apple Development or Developer ID private key and an existing
ScreenCapture grant for that certificate-backed `xyz.engmanager.frame`
designated requirement. The workflow serializes all candidates, accepts only a
full commit already contained in `origin/main`, builds and verifies the `.app`,
then independently verifies the bundle before resolving the executable named
by its `Info.plist`. The runner launches that exact signed executable directly;
the driver fails without prompting when preflight is not already granted and
binds its evidence to the source SHA, workflow run, Apple team, designated
requirement, and signed executable digest. Denial →
approval → relaunch remains attended manual evidence because an unattended job
cannot approve a macOS privacy prompt.

The future full product gate must additionally prove physical window/region
selection; multi-monitor scale/rotation placement; microphone, system audio, and camera;
device loss/hotplug; sleep/wake; Instant and Studio; pause/resume; tray/hotkey/
overlay ownership; crash/restart recovery; updater relaunch; keyboard-only
operation; and a named screen-reader journey. A valid protected partial result
cannot satisfy or substitute for that matrix.

## Rollback

The current signed updater retains the prior promoted manifest at
`previous.json`. Promotion writes that pointer before advancing `latest.json`,
rejects older candidates, and accepts an equal candidate only when the pointer
is byte-for-byte equivalent. The desktop exposes an explicit previous-channel
check; the control plane returns no manifest unless the retained version is
strictly older than the running version, and Tauri verifies its signature
before installation. Install and relaunch remain separate Rust-owned actions,
and browser code receives neither signed URLs nor signatures.

The following is the attended full-release rollback procedure. Native journal,
upload, and protected hardware evidence still determine whether the affected
matrix cell may be promoted again.

1. Stop rollout for the affected OS/mode without changing the other matrix cells.
2. Select the previous signed desktop channel and keep all new projects,
   immutable originals, and archived recovery journals intact.
3. Disable update promotion; do not downgrade project files in place.
4. Reconcile active native journals and uploads before terminating adapters.
5. Attach failure evidence, binary digest, and the matrix cell to the incident. Re-enter rollout only
   after both deterministic regression coverage and the affected real-hardware cell pass.
