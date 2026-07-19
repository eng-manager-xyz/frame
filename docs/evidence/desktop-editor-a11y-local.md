# Desktop recorder/editor/accessibility local evidence

## Closure ledger boundary

Issue 33 checkboxes 3, 4, 5, 6, 8, 9, and 10 are repository-local gaps.
Checkboxes 1, 2, and 7 remain locally satisfied by the typed Tauri surface,
backend-owned state model, and IPC security boundary. No issue-33 checkbox is
currently `protected_pending`: real hardware and assistive-technology runs will
eventually be required, but they cannot validate the still-incomplete recorder,
editor, recovery, lifecycle, and updater journeys.

The portable release shell selects `DesktopAdapterKind::Unavailable`. A macOS
release built with `macos-native` instead requests `NativeMacOs`, reports
`NativeMacOsDisplay` after successful backend construction, and falls back to
`Unavailable` if trusted GStreamer preflight or native source construction
fails. The deterministic
adapter remains debug-only and fake-gated. This new native slice covers only
permission preparation, opaque full-display selection, display-video
record/stop/cancel, and artifact-backed Editable WebM export. It does not make
the fake recovery, lifecycle, updater, multitrack Studio, or accessibility
journeys production behavior.

The checked-in hardware workflow invokes `frame-hardware-driver`, but this
repository does not provide that driver. Its validator and workflow shape are
not real-hardware-suite evidence.

## Local deterministic evidence

This evidence covers the locally reproducible portable contract and fake
portion of issue 33. It does not claim a physical native capture, real provider
upload, signed updater, observed platform permission flow, or
assistive-technology parity.

Validated contract, state-model, and fake implementation:

- versioned Rust request/response/event contracts with bounded JSON decoding;
- independent Main, Recorder, Recovery, Editor, Export, Settings, and Overlay scopes;
- replay/gap/duplicate-operation, cross-window, malformed-payload, and path-root rejection;
- backend-confirmed recorder/device/recovery/editor/export/upload/settings/lifecycle/update snapshots
  within the deterministic fake state machine;
- explicit portable-release `Unavailable` adapter and debug-only deterministic fake selection;
- explicit release `NotConfigured` Instant provider, strict main-window opaque-handle finalize
  command, native-only secret/request registry, and zero-network disabled state;
- versioned shared Instant progress/error events with determinate/indeterminate accessible progress,
  stable announcements, retry gating, and terminal handle removal;
- fake record/pause/resume/stop, recovery, trim/save, export, verified upload, device-loss,
  crash/restart, settings/preset, and update/relaunch journeys;
- semantic Leptos recorder, recovery, numeric timeline, export, upload, settings, and bounded error
  surfaces; only the narrow native display controls described below are
  connected to a release backend; and
- read-only legacy settings/project inspection models and a retained-selector flag, without a
  production migration adapter or usable legacy-desktop selection action.

## Native macOS display-only source evidence

Static source checks and focused Rust tests establish a bounded native path:

- `macos-native` is an explicit opt-in feature; the portable Tauri shell does
  not accidentally acquire capture or GStreamer authority;
- the Tauri composition derives shell capability truth from the runtime
  snapshot, invokes `dispatch_native_json` only for `NativeMacOs`, and degrades
  failed backend construction to `Unavailable`;
- the backend performs GStreamer recorder preflight and ScreenCaptureKit
  permission preflight/request before accepting a recording;
- display catalogs expose opaque tokens and coarse geometry rather than native
  display IDs or titles;
- native start accepts only a selected full display with Frame-owned window
  exclusion and embedded cursor, can optionally include exact 48 kHz stereo
  system audio while excluding Frame's own process audio, and keeps microphone
  and camera inputs disabled; and
- stop/cancel and artifact-bound Editable WebM publication require confirmed
  backend outcomes before the runtime announces success.

This is source and deterministic boundary evidence. It is not a physical
screen-capture, output-playback, recovery, accessibility, performance, signing,
notarization, clean-install, or distribution result.

Commands run from the repository root:

```sh
cargo test --locked -p frame-desktop-core
cargo test --locked -p frame-desktop-core --features tauri-app --bin frame-desktop
cargo clippy --locked -p frame-desktop-core --all-targets -- -D warnings
cargo clippy --locked -p frame-desktop-core --features tauri-app --bin frame-desktop -- -D warnings
cargo clippy --locked -p frame-desktop-core --features instant-finalize --all-targets -- -D warnings
cargo clippy --locked -p frame-desktop-ui --no-default-features --features csr --target wasm32-unknown-unknown -- -D warnings

# macOS native source and composition tests require the exact build-time plugin root.
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  cargo test --locked -p frame-desktop-core \
  --features tauri-app,macos-native --all-targets

python3 scripts/ci/build-desktop-ui.py
python3 scripts/ci/check-desktop-bundle.py --evidence target/evidence/desktop-bundle-local.json
python3 scripts/ci/check-desktop-product.py --evidence target/evidence/desktop-product-local.json

# Production-mode macOS adapter-truth smoke; it does not start capture.
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display
```

The fake integration test is `apps/desktop/tests/fake_desktop_journey.rs`; security/race/fault tests
also live beside the IPC, workflow, accessibility, surface, and runtime implementations. Evidence
JSON contains only booleans, file digests, platform labels, and public state—not device names,
project paths, session tokens, or user data.

## Local result boundary

Local code still satisfies the typed surface, backend-owned state model, and IPC
security classifications without closing broader product-integration
checkboxes. In addition, the macOS composition now has a real but narrow
display-video and optional-system-audio WebM path. It continues to refuse
microphone, camera, window, region, pause, MP4, upload, updater, Instant
publication, recovery, and edit-aware Studio behavior.
Optional macOS system audio is the only native audio source currently supported;
microphone capture remains unavailable. The registered Instant command
therefore proves a fail-closed boundary and state model, not a working
publication journey; the native WebM path proves no editor or recovery journey.

## Protected evidence still required

The following evidence will still be required after the repository-local gaps
close. It cannot currently convert checkboxes 3–6 or 8–10 to
`protected_pending`:

- macOS and Windows permission prompts using real screen, microphone, system-audio, and camera APIs;
- real Instant/Studio pipelines from issues 24–27 and API/provider journeys from issue 30;
- device hotplug/loss, Bluetooth, disk pressure, network loss, sleep/wake, crash/kill/restart at state
  boundaries, and real project recovery;
- target picker, Frame-window exclusion, tray, global hotkeys, overlay and multi-monitor placement
  across scale/rotation/topology matrices;
- signed updater check/install/relaunch and previous-channel rollback;
- real filesystem no-follow/reparse-point handle verification under platform roots;
- complete keyboard walkthrough plus named VoiceOver and Narrator reports; and
- product, accessibility, privacy, security, desktop, media, and release-owner approvals, followed by
  parity gate 29 before removing the legacy selector.

The manual workflow and evidence validator are checked in, but the workflow's external driver and
the release services it would need are absent. No hardware result is fabricated or marked passed,
and a validator-only artifact is not valid completion evidence.
