# Desktop recorder/editor/accessibility local evidence

## Local deterministic evidence

This evidence covers the locally reproducible portion of issue 33. It does not claim native capture,
real provider upload, signed updater, platform permission, or assistive-technology parity.

Validated implementation:

- versioned Rust request/response/event contracts with bounded JSON decoding;
- independent Main, Recorder, Recovery, Editor, Export, Settings, and Overlay scopes;
- replay/gap/duplicate-operation, cross-window, malformed-payload, and path-root rejection;
- backend-confirmed recorder/device/recovery/editor/export/upload/settings/lifecycle/update snapshots;
- explicit release `Unavailable` adapter and debug-only deterministic fake selection;
- explicit release `NotConfigured` Instant provider, strict main-window opaque-handle finalize
  command, native-only secret/request registry, and zero-network disabled state;
- versioned shared Instant progress/error events with determinate/indeterminate accessible progress,
  stable announcements, retry gating, and terminal handle removal;
- fake record/pause/resume/stop, recovery, trim/save, export, verified upload, device-loss,
  crash/restart, settings/preset, and update/relaunch journeys;
- accessible Leptos recorder, recovery, numeric timeline, export, upload, settings, and bounded error
  surfaces; and
- read-only legacy settings/project inspection with preserved originals and the legacy desktop
  selector retained.

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

Local code can satisfy the typed surface, deterministic state/race behavior, fake device/pipeline
harness, static capability/CSP review, semantic accessibility structure, legacy non-mutation, and
rollout/rollback design. The production shell intentionally refuses capture, OS lifecycle, export,
upload, updater, and Instant publication success until real adapters and a native authenticated
journal owner are selected. The registered Instant command therefore proves a fail-closed boundary
and accessible state model, not hosted publication.

## Protected evidence still required

Issue 33 cannot receive production/epic signoff from local evidence alone. Required protected work:

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

The manual protected workflow and evidence validator are checked in, but no hardware result is
fabricated or marked passed.
