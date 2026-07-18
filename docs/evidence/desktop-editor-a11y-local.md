# Desktop recorder/editor/accessibility local evidence

## Closure ledger boundary

Issue 33 checkboxes 3, 4, 5, 6, 8, 9, and 10 are repository-local gaps.
Checkboxes 1, 2, and 7 remain locally satisfied by the typed Tauri surface,
backend-owned state model, and IPC security boundary. No issue-33 checkbox is
currently `protected_pending`: real hardware and assistive-technology runs will
eventually be required, but they cannot validate product journeys that the
release adapter cannot execute.

The release binary selects only `DesktopAdapterKind::Unavailable`; the
deterministic adapter is debug-only and fake-gated. Recorder preparation,
device selection, recording, export, lifecycle, updater, and fault journeys do
not call production capture, Studio, OS, or updater services. The checked-in
hardware workflow invokes `frame-hardware-driver`, but this repository does not
provide that driver or a release adapter for it to exercise. Its validator and
workflow shape are not real-hardware-suite evidence.

## Local deterministic contract and fake evidence

This evidence covers the locally reproducible portion of issue 33. It does not claim native capture,
real provider upload, signed updater, platform permission, or assistive-technology parity.

Validated contract, state-model, and fake implementation:

- versioned Rust request/response/event contracts with bounded JSON decoding;
- independent Main, Recorder, Recovery, Editor, Export, Settings, and Overlay scopes;
- replay/gap/duplicate-operation, cross-window, malformed-payload, and path-root rejection;
- backend-confirmed recorder/device/recovery/editor/export/upload/settings/lifecycle/update snapshots
  within the deterministic fake state machine;
- explicit release `Unavailable` adapter and debug-only deterministic fake selection;
- explicit release `NotConfigured` Instant provider, strict main-window opaque-handle finalize
  command, native-only secret/request registry, and zero-network disabled state;
- versioned shared Instant progress/error events with determinate/indeterminate accessible progress,
  stable announcements, retry gating, and terminal handle removal;
- fake record/pause/resume/stop, recovery, trim/save, export, verified upload, device-loss,
  crash/restart, settings/preset, and update/relaunch journeys;
- semantic Leptos recorder, recovery, numeric timeline, export, upload, settings, and bounded error
  surfaces that are not connected to a usable release backend; and
- read-only legacy settings/project inspection models and a retained-selector flag, without a
  production migration adapter or usable legacy-desktop selection action.

Commands run from the repository root:

```text
cargo test -p frame-desktop-core
cargo test -p frame-desktop-core --features tauri-app --bin frame-desktop
cargo clippy -p frame-desktop-core --all-targets -- -D warnings
cargo clippy -p frame-desktop-core --features tauri-app --bin frame-desktop -- -D warnings
cargo clippy -p frame-desktop-core --features instant-finalize --all-targets -- -D warnings
cargo clippy -p frame-desktop-ui --no-default-features --features csr --target wasm32-unknown-unknown -- -D warnings
python scripts/ci/build-desktop-ui.py
python scripts/ci/check-desktop-bundle.py --evidence target/evidence/desktop-bundle-local.json
python scripts/ci/check-desktop-product.py --evidence target/evidence/desktop-product-local.json
```

The fake integration test is `apps/desktop/tests/fake_desktop_journey.rs`; security/race/fault tests
also live beside the IPC, workflow, accessibility, surface, and runtime implementations. Evidence
JSON contains only booleans, file digests, platform labels, and public state—not device names,
project paths, session tokens, or user data.

## Local result boundary

Local code currently satisfies only the typed surface, backend-owned state model, and IPC security
classifications. The deterministic state/race behavior, fake device/pipeline harness, semantic
accessibility structure, legacy inspection model, and rollout design remain useful development
evidence, but they do not close their product-integration checkboxes. The production shell refuses
capture, OS lifecycle, export, upload, updater, and Instant publication success because no release
adapter or native authenticated journal owner is selected. The registered Instant command therefore
proves a fail-closed boundary and state model, not a working publication or desktop journey.

## Hardware and accessibility evidence not yet valid

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
