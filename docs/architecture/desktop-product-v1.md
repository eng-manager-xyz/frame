# Desktop product contract v1

Issue 33 owns the Leptos/Tauri recorder and editor boundary, not the native capture or media
implementations from issues 24–27. This contract connects those future adapters without allowing the
WebView to acquire filesystem, shell, device, tray, updater, or arbitrary Tauri authority.

## Backend truth

The native `DesktopRuntime` is the only authority for recorder, device, recovery, editor, export,
upload, settings, lifecycle, and updater state. The Leptos client sends a versioned
`RequestEnvelope`, waits for `DesktopDispatch`, then replaces its render snapshot. A click never
optimistically changes a success state. Recorder start, pause, resume, stop, editor revision changes,
verified upload parts, export progress, and recoverability all originate in checked backend events.

There are three Tauri commands:

- `bootstrap_main` reports the shell and protocol compatibility marker;
- `bootstrap_desktop` returns opaque logical-window scopes and the first redacted snapshot;
- `dispatch_main` decodes one bounded JSON envelope and emits `frame-desktop://event-v1` events.

All product operations are variants of the Rust `IpcCommand` enum. Unknown commands, malformed or
oversized JSON, unsupported protocol versions, duplicate request IDs, replayed/gapped sequences,
stale settings/editor/update revisions, and invalid payloads fail closed.

## Window ownership

The single physical WebView is deliberately narrow. Bootstrap creates independent logical scopes;
each has a different opaque window and session token, monotonic request sequence, command allowlist,
and path policy.

| Logical owner | Allowed authority |
| --- | --- |
| Main | open known surfaces, enumerate/scan, lifecycle, update |
| Recorder | capture configuration/target/device, recording, recorder upload |
| Recovery | scan, inspect, open, or explicitly discard recovery copies |
| Editor | open/revision-fenced edit/save/export/upload |
| Export | export start/cancel only |
| Settings | revision-fenced settings and approved presets |
| Overlay | pause/resume/stop/cancel and overlay lifecycle only |

A command copied to another logical scope is rejected before state mutation. Backend events carry an
explicit owner. Closing or hiding UI surfaces never rewrites recorder/editor truth.

## Filesystem and capability boundary

Project reads/deletes, media reads, and export writes accept only normalized absolute paths beneath
the native roots assigned to that logical owner and only approved extensions. Validation rejects
parent/current components and requires adapters to open using no-follow/reparse-point-safe semantics
and re-check the resolved handle. The deterministic fake never opens these paths.

`capabilities/main.json` grants only the three commands above to window `main`. The two product
commands each have an explicit allow and deny permission file. The WebView CSP has no remote scripts,
general `unsafe-eval`, object embedding, base URI, ancestor framing, or broad network origin.

## Recorder and project state

The contract represents Instant/Studio mode, bounded countdown, display/window/region targets,
Frame-window exclusion, permission state, typed device counts and selection, microphone/system-audio
meters, camera activity, pause/resume/stop/cancel, recovery copies, revision-fenced trim/save,
monotonic export progress, verified multipart upload progress/pause/resume, settings/presets, and
update/relaunch state.

The Leptos product uses native buttons, fieldsets, labels, meters, progress elements, headings,
landmarks, a polite atomic status region, an assertive modal error surface, visible focus, a skip
link, forced-color support, and reduced-motion rules. The essential timeline trim has labeled
numeric controls, so drag gestures are not required. The portable accessibility model separately
checks deterministic tab order, keyboard shortcuts, accessible names, value text, modal focus trap,
focus restoration, and Escape dismissal.

## Hotkey, tray, overlay, and update lifecycle

Lifecycle transitions are typed and backend-owned. The fake adapter proves ownership and state
reconstruction without pretending to call OS APIs. A release binary selects `Unavailable`; only a
debug build with `FRAME_DESKTOP_FAKE_PIPELINE=1` can select the deterministic fake. Native global
hotkey registration, tray actions, overlay placement, target-picker placement, real updater install,
and OS window exclusion remain blocked until the platform adapters and the protected hardware matrix
pass.

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
The previous signed desktop remains selectable until parity gate 29 is approved. Rollback changes
the release selector; it does not rewrite projects produced by either desktop.
