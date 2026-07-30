# Desktop recorder/editor/accessibility local evidence

## Closure ledger boundary

Issue 33 checkboxes 1–5, 7, and 10 are locally satisfied by the typed Tauri
surface, backend-owned state model, accessible Leptos product, production
window lifecycle, deterministic browser/fake harnesses, IPC security boundary,
and non-destructive Cap scan/import path. Checkboxes 6 and 8 are
`protected_pending`: the rapid-command/error/device-loss/restart and complete
keyboard paths execute locally, while representative signed native hardware
and named VoiceOver/Narrator results remain uncollected. Checkbox 9 is the one
remaining repository-local gap because the signed macOS display lane is not a
full-product macOS/Windows lifecycle, exclusion, and multi-monitor driver.

The portable release shell selects `DesktopAdapterKind::Unavailable`. A macOS
release built with `macos-native` instead requests `NativeMacOs`, reports
`NativeMacOsDisplay` after successful backend construction, and falls back to
`Unavailable` if trusted GStreamer preflight or native source construction
fails. The deterministic
adapter remains debug-only and fake-gated. This native slice covers permission
preparation, opaque display/window selection, bounded single-display region
definition, target-video record/pause/resume/stop/cancel, and artifact-backed
Editable WebM export. Its Studio composition additionally records selected
microphone, system-audio, camera, cursor, and screen tracks, exposes a
descriptor-rooted bounded catalog of canonical project/journal pairs, and
connects authenticated preview/edit/save to coordinator-owned asynchronous
export with progress, cancellation, and hardware/software fallback. Completed
projects enter the editor through a fresh opaque token after native
reauthentication, and interrupted entries are reported as recovery-required.
Only a proven-empty attempt can be archived, and that archive deletes no media.
Named assistive-technology journeys and representative physical hardware
execution remain protected.

The media crate now has a production bounded isolated-track Studio encoder and
durable partial-recovery boundary. The macOS desktop adapter now composes its
native selected screen and optional direct system-audio source with that
recorder, commits immutable originals, and retains an authenticated canonical
project as Rust-only authority. The native recovery scan publishes only opaque
tokens and coarse status; completed projects open with their real revision and
duration after descriptor-rooted reauthentication. Incomplete recording and
edit-save journals recover through exact graph/original/project identities,
while the “archive” control is restricted to a proven-empty attempt and
preserves every graph/media file. The combined optional-input bridge,
source-set-bound preview, camera/cursor/background compositor, and durable
render coordinator now close the repository-local Studio integration portion
without claiming physical device, codec, or provider evidence.

The checked-in hardware workflow now invokes the exact certificate-signed
`Frame.app` executable with a protected, token-gated in-process driver. The
driver refuses to request ScreenCapture permission, records and discovers a
short display-only WebM, exports it through the native adapter, and proves
cancel cleanup. Its source, validator, and workflow shape are not a physical
real-hardware result; only a successful protected run can supply that evidence.

## Local deterministic evidence

This evidence covers the locally reproducible portable contract and fake
portion of issue 33. It does not claim a physical native capture, real provider
upload, signed updater, observed platform permission flow, or
assistive-technology parity.

The executable release-UI journey builds the Leptos production assets and a
bounded Rust host, opens them in headless Chrome with the production CSP, and
uses only DevTools keyboard events to activate controls. It traverses Studio,
device and target configuration, permission preparation, two recordings,
pause/resume/stop, recovery, edit/save, export/upload, reduced motion, hotkeys,
and update/relaunch. Between the two recordings, a gated host-only control
injects backend device loss and process restart. The stale UI action is
consumed as a backend-confirmed error, never success; the resulting modal is
checked for labelling, description, autofocus, forward/reverse focus trap,
Escape dismissal, and safe main-landmark restoration. Every accepted action
crosses the newline-delimited JSON bridge into the real `DesktopRuntime`; the
browser cannot synthesize success locally. The journey also checks duplicate
IDs, accessible names, label references, landmarks, live regions, progress,
numeric timeline controls, and backend status. This closes the deterministic
browser keyboard/semantic and state-consistency gaps, but deliberately sets
both protected hardware and assistive-technology claims to false.

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
  surfaces; only the narrow native target controls described below are
  connected to a release backend; and
- a production, path-redacted Cap settings/project scan; generation-fenced
  macOS copy into immutable Frame originals; coarse Windows compatibility
  reporting; and the retained previous-desktop rollback action.

## Native macOS target source evidence

Static source checks and focused Rust tests establish a bounded native path:

- `macos-native` is an explicit opt-in feature; the portable Tauri shell does
  not accidentally acquire capture or GStreamer authority;
- the Tauri composition derives shell capability truth from the runtime
  snapshot, invokes `dispatch_native_json` only for `NativeMacOs`, and degrades
  failed backend construction to `Unavailable`;
- the backend performs GStreamer recorder preflight and ScreenCaptureKit
  permission preflight/request before accepting a recording;
- target catalogs expose opaque display/window tokens and coarse geometry
  rather than native IDs, application names, or window titles; a region is
  accepted only inside a freshly selected display and receives a new opaque
  topology-bound token;
- native start accepts a selected display/window/region with embedded cursor;
  display/region capture excludes Frame's whole application and Frame windows
  are absent from the window catalog; screen-only recording uses the normalized
  capture ingress/pump, while the optional direct A/V worker includes exact
  48 kHz stereo system audio and excludes Frame's own process audio;
- the bounded one-second recorder poll carries only a coarse system-audio level
  (0..=10,000) from a worker-owned atomic; no PCM, device label, or native
  identifier crosses the WebView boundary; and
- stop/cancel and artifact-bound Editable WebM publication require confirmed
  backend outcomes before the runtime announces success.

This is source and deterministic boundary evidence. The browser journey is a
real production-asset keyboard and semantic traversal, but it is not a
physical screen-capture, named screen-reader, output-playback, hardware
recovery, performance, signing, notarization, clean-install, or distribution
result.

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

# Execute the production Leptos UI against the real Rust dispatch boundary.
cargo build --locked --release -p frame-desktop-core \
  --bin frame-desktop-e2e-host
python3 -I scripts/ci/desktop-browser-journey.py \
  --evidence target/evidence/desktop-browser-journey-local.json

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

Local code now satisfies the typed surface, backend-owned state model,
accessible product surface, deterministic fake/browser harness, native
hotkey/tray/window lifecycle, and IPC security classifications. The macOS
composition records the selected target and optional normalized microphone,
system-audio, camera, and cursor tracks into authenticated Studio projects;
preview/edit/save and the durable render coordinator use the canonical plan.
The production Instant finalize service and hosted upload remain explicitly
unconfigured until protected provider authority exists, so the accessible
Instant/upload UI and state machine do not claim a live publication. Windows
continues to expose the narrower target-video adapter and rejects unsupported
audio/Studio operations truthfully.

The legacy migration acceptance criterion is locally complete. A main-window
scan reads Cap's bounded settings and known recording roots without mutation,
publishes only ordinal compatibility results, and rejects stale tokens. On
macOS, supported projects are copied into immutable Frame originals, committed
under a source- and manifest-bound journal receipt, and surfaced through a
fresh authenticated Studio catalog. Unsupported/newer projects remain
report-only, Windows does not expose an import button, and the previous signed
desktop remains selectable. The checked-in fixture is synthetic; approval
against a privacy-reviewed historical corpus remains protected Studio
evidence.

## Protected evidence still required

The following evidence is still required for checkboxes 6 and 8 and for the
broader desktop release. Checkbox 9 cannot become `protected_pending` until the
full-product macOS/Windows driver and validator are implemented:

- macOS and Windows permission prompts using real screen, microphone, system-audio, and camera APIs;
- real Instant/Studio pipelines from issues 24–27 and API/provider journeys from issue 30;
- device hotplug/loss, Bluetooth, disk pressure, network loss, sleep/wake, crash/kill/restart at state
  boundaries, and real project recovery;
- target picker, Frame-window exclusion, tray, global hotkeys, overlay and multi-monitor placement
  across scale/rotation/topology matrices;
- physical signed updater check/install/relaunch on distributed macOS and
  Windows builds (the previous-channel selector, strict downgrade comparator,
  R2 retention, and deterministic state transitions are repository-local);
- real filesystem no-follow/reparse-point handle verification under platform roots;
- complete keyboard walkthrough plus named VoiceOver and Narrator reports; and
- product, accessibility, privacy, security, desktop, media, and release-owner approvals, followed by
  parity gate 29 before removing the legacy selector.

The checked-in partial macOS workflow, in-process signed-app driver, runner,
and validator execute only the verified executable named by the signed
bundle's `Info.plist`; evidence is bound to its certificate-backed designated
requirement and executable digest. They prove the narrow display capability
only. No hardware result is fabricated or marked passed, and source inspection
or a validator-only artifact is not valid completion evidence.
