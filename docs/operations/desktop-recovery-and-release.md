# Desktop recovery and release runbook

## Build modes and current boundary

The portable Tauri shell and the native macOS target-capture slice are different
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
```

The native slice records one selected display, non-Frame window, or bounded
single-display region as VP8/WebM and embeds the cursor. Display/region capture
excludes Frame's whole application; Frame windows are absent from the window
catalog. Screen-only recording uses the normalized capture ingress/pump. The
separate direct A/V worker can optionally mux exact 48 kHz stereo system audio
as Opus while excluding Frame's own process audio. The slice supports stop,
cancel, and artifact-bound Editable WebM publication. It does not support
microphone, camera, pause/resume, multitrack or edit-aware Studio export, MP4,
persisted recording recovery, native tray/hotkey/overlay lifecycle, or updater
installation.

The smoke confirms only the production-CSP WebView-to-Rust bootstrap and
coherent adapter truth. A successful smoke is not capture, playback, recovery,
accessibility, signing, notarization, clean-machine, or distribution evidence.
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

This section is the required release behavior, not evidence that the current
`macos-native` target-capture slice implements it.
`macos-native` has no durable journal or recovery-store composition. A
stop/cancel/worker failure is handled
as a terminal backend outcome, and a process crash must not be advertised as
recoverable until the journal integration and protected crash matrix exist.

When the UI disappears or the process restarts, native journal state remains authoritative. The UI
must not claim that recording stopped or continued until a backend event says so.

1. Reconstruct the main window and request a fresh backend snapshot.
2. If the journal reports active capture without a live adapter, move to `recoverable`, hide the
   overlay, zero visual meters, and announce recovery availability.
3. Scan recovery roots read-only. Inspect integrity and format before offering open or discard.
4. Open a preserved copy. Discard only after an explicit user command in the Recovery scope.
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

The checked-in protected workflow and `scripts/ci/desktop-real-hardware.py`
accept externally produced evidence only for the exact partial
`macos_display_webm_v1` capability: a preauthorized ScreenCapture TCC state,
display catalog/selection, display capture, Frame-window exclusion, playable
stopped/exported WebM, and cancel cleanup. The repository does not provide the
external `frame-hardware-driver`.
The workflow and validator are not evidence that a physical run occurred.
Submitted evidence must state
`full_product_gate: not_claimed`; the validator deliberately has no full-product
mode.

The protected runner must be a persistent, logged-in macOS account with an
unlocked Apple Development or Developer ID private key and an existing
ScreenCapture grant for that certificate-backed `xyz.engmanager.frame`
designated requirement. The workflow serializes all candidates, accepts only a
full commit already contained in `origin/main`, builds and verifies the `.app`,
and passes that bundle—not its inner Mach-O—to the external driver. The driver
must launch through LaunchServices, fail without prompting when preflight is
not already granted, and bind its evidence to the source SHA, workflow run,
Apple team, designated requirement, and signed executable digest. Denial →
approval → relaunch remains attended manual evidence because an unattended job
cannot approve a macOS privacy prompt.

The future full product gate must additionally prove physical window/region
selection; multi-monitor scale/rotation placement; microphone, system audio, and camera;
device loss/hotplug; sleep/wake; Instant and Studio; pause/resume; tray/hotkey/
overlay ownership; crash/restart recovery; updater relaunch; keyboard-only
operation; and a named screen-reader journey. A valid protected partial result
cannot satisfy or substitute for that matrix.

## Rollback

The following is normative full-release behavior once signed channel
selection, updater, native journal, upload, and protected matrix integrations
exist.
The current `macos-native` target-capture slice cannot execute this rollback procedure.

1. Stop rollout for the affected OS/mode without changing the other matrix cells.
2. Select the previous signed desktop channel and keep all new project/recovery copies intact.
3. Disable update promotion; do not downgrade project files in place.
4. Reconcile active native journals and uploads before terminating adapters.
5. Attach failure evidence, binary digest, and the matrix cell to the incident. Re-enter rollout only
   after both deterministic regression coverage and the affected real-hardware cell pass.
