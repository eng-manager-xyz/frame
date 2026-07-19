# Studio Mode local evidence

Issue: `_issues/27-p4-studio-mode.md`

This record separates executable provider-neutral evidence from protected
native, historical, hardware, UX, and release evidence.
It does not classify the complete Studio product path as locally implemented.

## Closure ledger boundary

Issue 27 checkboxes 2, 3, 4, 5, 8, 9, 10, and 11 are repository-local gaps.
Checkboxes 1 and 6 are locally satisfied by the versioned project format and
the production filesystem legacy importer. Checkbox 7 alone remains
`protected_pending`, because the non-mutating importer/reporting path exists
but still needs a reviewed representative legacy-project corpus.

The remaining tests exercise contracts, synthetic GStreamer sources,
filesystem components, and a reference renderer, not a production Studio
service. The `macos-native` release composition now pumps one selected full
display plus optional exact 48 kHz stereo system audio into a separate
VP8/Opus WebM recorder, but it does not supply microphone or camera tracks and
is not connected to the Studio multitrack graph, journal, project, edit plan,
or recovery stores. Its artifact-backed Editable WebM copy/publication
is not an edit-aware Studio export. A new narrow native adapter does execute
the shared canonical plan for a clock-aligned screen original plus optional
microphone/system-audio originals: preview maps an
edited output point before real decode, and export applies trim/delete/rational
speed plus audio gaps/gain/mute to a playable VP8/Opus WebM. It is not wired to
the desktop or `StudioRenderCoordinator`, does not assemble arbitrary
asset-offset ranges, and fails closed for camera, cursor, background,
camera-only, and side-by-side composition. The reference renderer still writes
a checksum-bound bundle rather than dispatching that native adapter. Therefore
these paths cannot yet satisfy the complete Studio native recording/recovery,
edit-aware compositor, Cloudflare distribution-master, decoded-golden,
long-project, or hardware-fallback/cancellation checkboxes.

## Executed locally

The external contract suite exercises:

- canonical project/edit/journal round trips, deterministic bytes, checksum
  corruption, newer schemas, malformed framing, and trailing-byte rejection;
- the production filesystem `.cap` adapter against a schema-shaped directory
  fixture: typed JSON decoding, exact decimal-time conversion, streaming source
  hashes, pinned flattened single-segment and multiple-segment forms, known
  default fields, unsupported-effect and newer-version reporting, normalized
  paths, symlink/traversal/missing-file rejection, and an asset-bound copy plan;
- one required screen graph branch plus independently optional camera,
  microphone, and system-audio branches, with rejection of missing-screen,
  flattened, or duplicate tracks; the filesystem recording-session adapter
  seals only enabled VP8/Opus WebM originals and reopens a crash state containing
  both partial and already-sealed tracks;
- a native-execution test helper that uses GStreamer to record four
  independently playable synthetic VP8/Opus WebM
  screen/camera/microphone/system-audio originals on one pipeline clock,
  validates nontrivial immutable outputs,
  decodes a bounded RGB preview frame at a requested position, and tears down
  every graph to `Null`;
- one bounded `StudioEditExecutor` used by both native preview lookup and
  export batching. It partitions optional-track gaps without reinterpreting the
  saved edit spec, maps edited output time to exact source time, preserves the
  plan digest and composition/audio state, rejects overflow and excess windows,
  and does not advance its export cursor after cancellation;
- a synthetic native edit execution that combines the isolated screen and
  system-audio originals, applies trim/delete/2× speed and per-window audio gain
  through accurate, shared-sequence GStreamer segments, waits for every aligned
  source branch rather than the first aggregate message, bounds any closing
  screen-frame hold to one second, emits a playable VP8/Opus WebM whose measured
  duration is within 100 ms of the exact plan, decodes the result, and removes
  pre-cancelled or failed outputs;
- a separately composed macOS display-only desktop source and recording graph
  whose source-level checks cover permission preflight, opaque display
  selection, bounded frame ingress, stop/cancel, and artifact-bound Editable
  WebM export without claiming a physical capture run or Studio integration;
- journal CAS, ownership fencing, lost acknowledgement reconciliation,
  idempotent replay, stale writers, exact pending asset/render continuity,
  asset/edit/render carry-forward and exact resolution from recoverable failure,
  rejection of identity-dropping recovery exits, and every declared power-loss
  boundary, using both fake stores and the production filesystem journal store;
- journal-minted render authorization, rejection of dispatch without a durable
  `RenderPrepared` reservation, on-disk coordinator reservation reconstruction,
  and delayed renderer publication after an initial `Absent` probe while the
  output remains quarantined until exact fenced cancel/cleanup proof;
- structured terminal render receipts bound to project, export, operation,
  fence, sources, edit plan, render specification, profile, output, checksum,
  and byte count; recovery adoption takes no caller-supplied checksum or size;
- temporary-to-original commit reconciliation after a lost acknowledgement and
  after power loss between the original-media rename and sidecar persistence;
- atomic edit-save reconciliation with unchanged original asset records and a
  durable maximum-fence marker that rejects a superseded store instance;
- rejection of edit saves whose trim/delete combination leaves no renderable
  output;
- trim containment, split, delete, VFR, rational speed, exact seek, layout,
  camera, cursor, background, gain/mute, audio silence, camera gaps, overlap
  rejection, required screen gaps, frame timestamps, audio timestamps, and
  bounded long simulation;
- byte-for-byte canonical preview/export edit-plan equality plus exact binding
  to saved edits, source topology, coverage ranges, and original descriptors;
- exact profile/capability/license preflight and hardware/software disposition;
- bounded one-shot control payloads, length/checksum checks, and cancellation;
- full render-spec replay identity, portable output-name rejection, output
  reservation/release, lost renderer-start acknowledgement, probe and cleanup
  uncertainty quarantine, committed-postcondition mismatch quarantine, durable
  reservation reconstruction after coordinator restart, source/edit/profile/
  output replay mutation rejection, stale callbacks, monotonic observable
  progress, bounded event draining and failure codes, cancellation during all
  six render phases, exact cleanup, hardware fallback mutation rejection and
  exact software restart, and redacted debug output;
- a restartable local filesystem reference path that imports the `.cap` copy
  plan, streams immutable originals, persists a project, compiles the shared
  preview/export plan, performs an exact seek, writes a canonical render bundle,
  persists the terminal receipt, and re-probes that receipt after reopening the
  renderer and journal; and
- hard buffer/byte/time ceilings on every media queue.

The native execution tests also retain the basic single-source WebM path and,
behind the explicit codec-decision gate, emit an MP4 that a second GStreamer
demux/parser graph consumes to EOS. The edit-aware WebM path proves canonical
temporal/audio execution for aligned synthetic originals, bounded cancellation,
and a playable output duration. It does not prove camera/cursor/background
composition, Cloudflare profile geometry/color compliance, arbitrary persisted
asset assembly, perceptual parity, or coordinator/desktop integration.

The production-mode desktop composition can be built and its adapter-truth
bootstrap smoked on macOS with:

```text
python3 scripts/ci/build-desktop-ui.py
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
python3 scripts/ci/desktop-shell-smoke.py --expected-adapter native_macos_display
```

This smoke does not request capture permission, record a frame, create a
Studio project, execute an edit plan, or inspect an exported artifact.

The [local macOS display-recording runbook](../operations/macos-display-recording-local.md)
can now exercise a real five-second display-video recording and byte-identical
Editable WebM export. That makes the narrow recorder/export adapter functional;
it still does not create a Studio project, journal or recover multiple tracks,
interpret edits, or render a distribution master, so issue 27 remains open.

Focused command:

```text
cargo test -p frame-media --test studio_mode_contract

FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0=/exact/pinned/gstreamer/plugin/root \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  studio_ -- --nocapture
```

These commands exercise the provider-neutral Studio contract and synthetic
native-execution helper. Their results must not be reused as evidence that the
new macOS display source was exercised.

Full media command (using the audited plugin root discovered for this build):

```text
FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
```

Record fresh output from this aggregate command for the revision under review;
historical pass counts predate the native display-only composition and are not
native capture evidence.

Static gate commands:

```text
cargo clippy --locked -p frame-media --all-targets -- -D warnings

RUSTDOCFLAGS=-Dwarnings cargo doc -p frame-media --no-deps

rustfmt --edition 2024 --check crates/media/src/studio.rs \
  crates/media/src/studio_edit_executor.rs \
  crates/media/src/studio_native_execution.rs \
  crates/media/tests/studio_mode_contract.rs

python3 scripts/ci/check-secrets.py
```

Record fresh command output and the reviewed commit with any evidence bundle.
Production modules contain no intentional panic/todo boundary and use no unsafe
media bridge. The repository-wide format, lint, and test commands are rerun at
the aggregate gate after concurrent issue lanes merge.

## Synthetic and component evidence only

`fixtures/studio/cap-schema-supported/` is a locally authored directory-schema
fixture. Its JSON and descriptor payloads prove the production parser, copy
plan, normalized path, segment, fingerprint, and read-only behavior, but it is
not a historical Cap project and its media-named files are not encoded samples.
The contract suite uses deterministic fake native ports alongside production
filesystem durability components. Its reference renderer writes a canonical
checksum-bound bundle, while the separate native execution helpers supply
synthetic tracks, edited preview mapping, aligned A/V WebM, single-source WebM,
and gated MP4 evidence. The display-only desktop recorder is connected to the
release shell but not to those multitrack helpers or the Studio coordinator.
Most timeline goldens remain mathematical; the decoded edited artifact adds a
duration/playability check, not a perceptual frame or reference-audio diff.
The JSON keys and non-fragmented `.mp4`/`.ogg` paths were checked against
`crates/project/src/meta.rs`, `crates/project/src/configuration.rs`, and
`crates/recording/src/studio_recording.rs` at the pinned revision.

## Protected and subsequently required evidence

Only the first item below currently supports a `protected_pending`
classification (checkbox 7). The remaining items are subsequent hardware,
quality, UX, and approval gates; they remain invalid for closure until the
corresponding repository-local production paths exist:

- a privacy-reviewed, provenance-pinned real legacy Cap project corpus at the
  referenced Cap revision, including supported and unsupported effects;
- encoded preview/export/reference frame, perceptual, color, and audio diffs
  within approved tolerances;
- physical-device native capture plus content-level playback/seek/mux/export
  comparison on every supported release OS;
- H.264, HEVC, AAC, VP8, Opus, FFV1, and FLAC availability/licensing results on
  every supported release OS and hardware family;
- hardware encoder failure and software fallback on representative machines;
- physical power-loss testing at every recording, asset-commit, edit-save, and
  render boundary;
- long wall-clock projects with measured peak memory, seek latency, preview
  latency, export speed, thermal behavior, and disk pressure;
- clean-install/editor workflow, keyboard/accessibility, screen-reader, reduced
  motion, localization, and destructive-action UX review; and
- migration owner, rollback-window owner, product/security/release signoff, and
  release-candidate evidence links.

Absence of a required protected record blocks promotion, but attaching one
cannot close checkboxes 2–5 or 8–11 while their local integrations remain
absent. No provider, hardware, user, or release claim is made by this local
evidence file.
