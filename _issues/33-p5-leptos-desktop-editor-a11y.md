---
title: "Integrate the Leptos desktop recorder/editor with Rust commands and accessibility coverage"
labels:
  - "phase:p5"
  - "area:leptos"
  - "area:desktop"
  - "area:accessibility"
  - "type:migration"
  - "risk:high"
depends_on: [08, 24, 25, 26, 27, 30]
size: epic
---

# 33 · Integrate the Leptos desktop recorder/editor with Rust commands and accessibility coverage

## Outcome

The Tauri desktop UI controls capture, Instant/Studio recording, recovery, editing, export, and upload through typed least-privilege Rust IPC.

## Current Cap reference

Cap's desktop combines SolidStart UI, Tauri commands, native Rust capture/media, hotkeys, tray, overlays, permissions, presets, recovery, editor, upload, and updater behavior. Frame has only a minimal shell.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#08](./08-p1-leptos-web-desktop-shells.md), [#24](./24-p4-screen-capture.md), [#25](./25-p4-audio-camera-sync.md), [#26](./26-p4-instant-mode.md), [#27](./27-p4-studio-mode.md), [#30](./30-p5-rust-api-workflow-parity.md)

## Scope

Build target/device selection, permission guidance, countdown/controls, camera/mic meters, Instant/Studio state, pause/resume/stop/cancel, recovery, projects/editor/timeline, export/upload progress, hotkeys, tray, overlays, settings/presets, updates, crash UX, typed events, CSP/capabilities, accessibility, and test automation.

### Out of scope

Native pipeline implementations are issues 24–27; broad API behavior is issue 30.

## Deliverables

- [ ] Versioned typed Tauri command/event surface and explicit capability/permission files.
- [ ] Recorder and editor state models driven by backend truth rather than duplicated UI assumptions.
- [ ] Accessible Leptos recorder, recovery, editor, export, upload, settings, and error flows.
- [ ] Hotkey/tray/window/overlay lifecycle and multi-window ownership model.
- [ ] Desktop E2E harness with fake devices/pipelines plus dedicated real-hardware suite.

## Acceptance criteria

- [ ] UI state remains consistent through rapid commands, backend error, device loss, process restart, window close/reopen, and update/relaunch.
- [ ] IPC rejects unapproved commands, malformed payloads, stale operation IDs, cross-window misuse, and filesystem paths outside allowed scopes.
- [ ] Keyboard-only and screen-reader users can configure, start, pause, stop, recover, edit essential controls, export, and upload.
- [ ] Hotkeys, tray, overlays, target picker, window exclusion, and multi-monitor placement pass each supported OS matrix.
- [ ] Legacy settings/projects are migrated or reported without destructive mutation, and the legacy desktop remains selectable until parity gate 29.

## Required test evidence

- Fake-pipeline deterministic E2E results.
- Real device/permission/platform smoke matrix.
- Tauri security/capability review and desktop accessibility report.

## Risks and open questions

- Event races can make the UI claim recording stopped while native work continues.
- Desktop accessibility and custom timeline widgets require deliberate semantics and alternative controls.

## Rollout and rollback

Ship in nightly/internal channels with a backend/UI selector and preserved project copies. Expand per OS and mode only after conformance.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
