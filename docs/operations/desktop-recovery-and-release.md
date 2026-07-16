# Desktop recovery and release runbook

## Release prerequisites

1. Build the exact commit with the pinned Rust and Trunk versions and retain binary, bundle, CSP, and
   capability digests.
2. Run the portable core tests, fake desktop journey, strict clippy, deterministic bundle checker,
   and desktop product/accessibility checker.
3. Run `.github/workflows/desktop-real-hardware.yml` on both protected runners and retain the complete
   non-fake JSON traces.
4. Name the macOS VoiceOver and Windows Narrator versions used for the keyboard/screen-reader
   journeys. Record OS build, architecture, monitor topology/DPI/rotation, device models, permission
   reset procedure, and binary digest.
5. Keep the legacy desktop selector enabled. Gate expansion per OS and recording mode; do not infer
   parity on one platform from the other.

## Crash and recovery

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

The protected driver must prove permission denial/recovery; display/window/region selection; Frame
window exclusion; at least two multi-monitor scale/rotation layouts; microphone, system audio, and
camera; device loss/hotplug; sleep/wake; Instant and Studio; pause/resume/stop/cancel; tray/hotkey/
overlay ownership; crash/restart recovery; updater relaunch; keyboard-only operation; and a named
screen-reader journey. `scripts/ci/desktop-real-hardware.py` rejects fake-adapter evidence or a
partial matrix.

## Rollback

1. Stop rollout for the affected OS/mode without changing the other matrix cells.
2. Select the previous signed desktop channel and keep all new project/recovery copies intact.
3. Disable update promotion; do not downgrade project files in place.
4. Reconcile active native journals and uploads before terminating adapters.
5. Attach failure evidence, binary digest, and the matrix cell to the incident. Re-enter rollout only
   after both deterministic regression coverage and the affected real-hardware cell pass.
