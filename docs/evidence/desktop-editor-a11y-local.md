# Desktop recorder/editor/accessibility local evidence

## Closure ledger boundary

Issue 33 checkboxes 3, 4, 5, 6, 8, and 9 remain broader product-integration
gaps. Checkboxes 1, 2, 7, and 10 are locally satisfied by the typed Tauri
surface, backend-owned state model, IPC security boundary, and production
non-destructive Cap scan/import path. Real hardware and assistive-technology
runs remain protected evidence and cannot be replaced by repository-local
tests.

The portable release shell selects `DesktopAdapterKind::Unavailable`. A macOS
release built with `macos-native` instead requests `NativeMacOs`, reports
`NativeMacOsDisplay` after successful backend construction, and falls back to
`Unavailable` if trusted GStreamer preflight or native source construction
fails. The deterministic
adapter remains debug-only and fake-gated. This native slice covers permission
preparation, opaque display/window selection, bounded single-display region
definition, target-video record/stop/cancel, and artifact-backed Editable WebM
export. Its Studio composition additionally exposes a descriptor-rooted,
bounded catalog of canonical project/journal pairs. Completed projects can
enter the editor through a fresh opaque token after native reauthentication,
and interrupted entries are reported as recovery-required. Recording and
edit-save boundaries can be inspected and reconciled through a new ownership
fence; only a proven-empty attempt can be archived, and that archive deletes no
media. The editor now applies and durably saves bounded canonical mutations,
and a clean aligned project can invoke the verified native distribution export
adapter. Lifecycle, updater, combined microphone/camera Studio, asynchronous
export cancellation, and named assistive-technology journeys remain
non-production behavior.

The media crate now has a production bounded isolated-track Studio encoder and
durable partial-recovery boundary. The macOS desktop adapter now composes its
native selected screen and optional direct system-audio source with that
recorder, commits immutable originals, and retains an authenticated canonical
project as Rust-only authority. The native recovery scan publishes only opaque
tokens and coarse status; completed projects open with their real revision and
duration after descriptor-rooted reauthentication. Incomplete recording and
edit-save journals recover through exact graph/original/project identities,
while the “archive” control is restricted to a proven-empty attempt and
preserves every graph/media file. It does not yet compose the normalized
combined microphone/camera source or provide coordinator-owned asynchronous
preview/export. The implemented editor mutation/save and synchronous
screen/audio distribution-export subset therefore does not close an issue-33
product journey.

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
device and target configuration, permission preparation, recording,
pause/resume/stop, recovery, edit/save, export/upload, reduced motion, hotkeys,
and update/relaunch. Every accepted action crosses the newline-delimited JSON
bridge into the real `DesktopRuntime`; the browser cannot synthesize success
locally. The journey also checks duplicate IDs, accessible names, label
references, landmarks, live regions, progress, numeric timeline controls,
backend status, and modal semantics. This closes a deterministic browser
keyboard/semantic gap, but it deliberately sets both protected hardware and
assistive-technology claims to false.

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

Local code still satisfies the typed surface, backend-owned state model, and IPC
security classifications without closing broader product-integration
checkboxes. In addition, the macOS composition now has a real but narrow
target-video and optional-system-audio WebM path plus a native Studio catalog
with authenticated edit/save/export authority. It can open a completed
Studio project and truthfully report
interrupted entries. It reconciles recording and edit-save boundaries and can
archive a proven-empty attempt without deleting media. It now accepts
revision-fenced editor mutations/save and a clean aligned Studio distribution
MP4, but continues to refuse microphone/camera Studio composition, pause,
asynchronous export cancellation, upload, updater, Instant publication, and
complete preview/export parity.
Optional macOS system audio is the only native audio source currently supported;
microphone capture remains unavailable. The registered Instant command
therefore proves a fail-closed boundary and state model, not a working
publication journey. The native Studio recovery path proves journal-fenced
recording/edit-save reconciliation, not a complete editor, accessibility, or
full-device recovery journey.

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

The following evidence will still be required after the repository-local gaps
close. It cannot currently convert checkboxes 3–6, 8, or 9 to
`protected_pending`:

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

The protected workflow, in-process signed-app driver, runner, and evidence
validator are checked in. The runner executes only the verified executable
named by the signed bundle's `Info.plist`, and evidence is bound to its
certificate-backed designated requirement and executable digest. No hardware
result is fabricated or marked passed, and source inspection or a
validator-only artifact is not valid completion evidence.
