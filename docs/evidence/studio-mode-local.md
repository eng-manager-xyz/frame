# Studio Mode local evidence

Issue: `_issues/27-p4-studio-mode.md`

This record separates executable provider-neutral evidence from protected
native, historical, hardware, UX, and release evidence.
It does not classify the complete Studio product path as locally implemented.

## Closure ledger boundary

Issue 27 checkboxes 10 and 11 are repository-local gaps. Checkboxes 1–6,
8, and 9 are locally satisfied by the versioned project format, production
isolated-track recorder, source-set-bound native preview engine, exact
aligned-source export profiles, production filesystem legacy importer, and
journal-fenced native recording/edit-save recovery. Checkbox 9 is backed by
the release editor's one-shot native preview command and a decoded RGB
preview/export golden that executes trim, delete, rational speed, and
audio-gain windows through the same canonical plan.
Checkbox 7 alone remains `protected_pending`, because the
non-mutating importer/reporting path exists but still needs a reviewed
representative legacy-project corpus.

The remaining tests exercise contracts, synthetic GStreamer sources,
filesystem components, and a reference renderer, not a complete Studio
service. The media crate now owns a production `NativeStudioRecording` graph:
one common GStreamer clock, one bounded non-leaky appsrc per enabled
screen/camera/microphone/system-audio source, independent VP8/Opus streamable
WebM branches, encoded appsink chunks written directly into the durable
recording session, aggregate EOS, and confirmed `Null`. The native test
executes all four branches, decodes the isolated video assets, commits all four
temporary originals, and proves that a no-EOS streamable partial can be
rehashed, sealed, and decoded after recovery.

The `macos-native` release composition pumps one selected
display/window/region plus optional exact 48 kHz stereo system audio into the
existing flattened VP8/Opus WebM recorder. A backend-bound `RecorderMode`
selects an additional Studio path: the same corrected screen and included
system-audio samples feed independent bounded `NativeStudioRecording` branches
whose encoded chunks seal recoverable temporary originals under the pinned
private Studio root. Stop waits for both graphs; cancellation and failure drive
both graphs to `Null`, and an unconfirmed Studio teardown poisons the native
backend instead of claiming success. The release path now creates a fenced
filesystem journal before native capture, records graph/capture and per-asset
power-loss boundaries, atomically commits each verified temporary track into
the immutable originals namespace, and creates a canonical revision-one
Editing project with an empty edit plan before recording-stop is acknowledged.
The project file is re-opened through the pinned projects root, authenticated
by size and SHA-256, and retained as Rust-only artifact authority; its path is
not serialized into the WebView. On a later scan, the native backend enumerates
the pinned root through its directory descriptor, applies explicit document
and catalog bounds, pairs canonical journals and projects, and returns only
fresh opaque tokens plus coarse status. A completed `OpenEditor` boundary can
be opened only after both documents are re-read through the pinned root and
authenticated; its real revision and duration drive the editor snapshot.
Interrupted capture is surfaced as `RecoveryRequired` and cannot be opened as
a completed project. Recovery inspection reauthenticates the exact journal
through the pinned projects directory and reports only a coarse action through
the opaque generation-fenced handle. The native recovery worker takes a new
journal fence, reopens the one checksummed recording graph, reconciles already
committed originals, probes retained WebM duration, commits remaining
temporaries through every durable asset boundary, and creates the canonical
revision-one project. An `EditSavePrepared` boundary either commits the exact
next manifest or reconciles its lost acknowledgement while retaining the
original asset records byte-for-byte. `Created` and
`RecordingGraphPrepared` attempts can be archived only after proving that the
session is empty; the journal is descriptor-moved to a private archive and no
media, graph, or completed project is deleted. Captured attempts cannot enter
that archive action.

The production desktop editor now installs one Rust-owned draft only after
reauthenticating a ready project. Apply requests carry a bounded mutation and
the current editor revision, not paths or durable identities. Rust converts
millisecond inputs to rational time, compiles the complete candidate edit
specification with `StudioTimelineCompiler`, and advances the editor revision
only after compilation succeeds. Save reauthenticates the exact discovered
manifest and journal, takes a new ownership fence, persists
`EditSavePrepared`, compares and swaps the complete next manifest, persists
`EditSaveCommitted`, proves that every original asset record is unchanged, and
remints the project catalog before reporting success. A failure after the
authority is consumed discards the active editor so the caller must rediscover
or recover rather than optimistically retrying stale state. Tests cover invalid
draft atomicity, a real filesystem journal/project save, immutable-original
preservation, opaque-token redaction, and backend-confirmed desktop
apply/save transitions.

The combined optional-input bridge feeds selected microphone, camera, and
system-audio samples into isolated Studio tracks. The artifact-backed Editable
WebM copy/publication remains the flattened recorder output and is not an
edit-aware Studio export. The narrow native edit adapter
executes the shared canonical plan for one clock-aligned screen original plus
optional microphone/system-audio originals: preview maps an edited output
point before real decode, and export applies trim/delete/rational speed plus
audio gaps/gain/mute. The adapter executes and postcondition-probes all four
approved software profiles, including the H.264/AAC hosted-media distribution
master, while preserving input originals. It emits bounded monotonic progress
and cleans its private staging artifact when cancellation occurs before
publication.

The macOS desktop now connects a clean authenticated Studio project to that
adapter. Rust revision-fences the request, re-reads the project through the
pinned project directory, verifies every immutable original and sidecar,
opens and hashes each original through the pinned Studio directory, and retains
those exact descriptors while the seekable decoder reads `/dev/fd` rather than
the project pathname. It recompiles the canonical plan, renders into a
preopened private export-staging inode, and publishes only that hashed regular
file to the scoped destination. The Leptos editor exposes the distribution MP4
action only for a clean project; the runtime accepts terminal success only when
revision, profile, nonzero byte count, and lowercase SHA-256 match the request.
A native integration test generates real isolated screen/system-audio sources,
commits them, replaces the visible Studio root after preparation, and still
produces the artifact from the retained original descriptors.

The release editor now submits a bounded revision/position request, and the
native backend reauthenticates the durable base manifest before constructing
the exact draft manifest that the next save would commit. Multiple optimistic
editor mutations advance only the editor revision; the draft preview and save
share one next durable project revision. `NativeStudioPreviewEngine` decodes
the requested screen frame and exact audio decisions, and only the bounded
one-shot preview event crosses into the WebView; raw RGB bytes never persist in
the runtime snapshot. The Leptos editor paints that frame into a canvas and
announces the mapped source position plus microphone/system-audio decisions.

The macOS export call now dispatches its graph through
`StudioRenderCoordinator`. The render reservation is CAS-persisted before a
bounded worker starts; payload-free polling carries monotonic progress, and
terminal publication is accepted only after exact inode/hash/length probing and
a durable receipt. Cancellation waits for the worker, conditionally deletes
the exact private inode, proves absence, and persists `RenderCancelled`.
Distribution master prefers the trusted hardware-only VideoToolbox H.264
factory when present. A deterministic native integration test forces that
hardware attempt to fail, proves exact partial cleanup, reopens and rehashes
the originals, starts the identical graph in software under new
operation/export and staging identities, and verifies a committed playable
artifact with an empty staging directory. Runtime tests also cover monotonic
completion, backend-confirmed cancellation, and quarantine when cleanup is
unconfirmed.

The adapter still does not assemble arbitrary asset-offset ranges and fails
closed for camera, cursor, background, camera-only, and side-by-side
composition. Native staging-identity reconstruction after process restart is
also not connected. Therefore the remaining paths cannot yet satisfy complete
desktop Studio composition, complete decoded preview/export/reference goldens,
or long-project effects. Representative physical-hardware fallback remains
protected evidence rather than a repository-local claim.

Separately, `NativeStudioPreviewEngine` opens the complete immutable
`StudioPreviewGraphSpec`, verifies every original against its durable sidecar
and content hash, resolves exact asset-local positions across segmented
originals, and decodes real bounded screen and active-camera RGB frames. Its
paused/playing/ended transport provides exact seek and deterministic CFR/VFR
frame stepping, while generation and sequence fences identify stale samples.
Each sample also binds the source-set and edit-plan digests and carries exact
microphone/system-audio source, gap, gain, and mute decisions. The local test
uses two sequential screen assets, camera, and system audio, proves selection
of the second screen asset at an edited seek, advances playback, rejects a
cancelled seek without state mutation, and rejects an original path replaced
after the engine opened.

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
- the production native isolated-track recorder validates exact graph/session
  identity, raw payload shape, finite audio, monotonic shared-clock timestamps,
  and per-source buffer/byte/time ceilings; streams encoded chunks into the
  durable session; executes all four isolated WebM branches; decodes both video
  tracks; commits every temporary asset; and recovers and decodes a streamable
  video partial stopped before EOS;
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
- the production `NativeStudioPreviewEngine` verifies the complete durable
  source set, selects segmented asset-local positions, decodes real screen and
  active-camera frames, emits exact audio source/gain/mute decisions, advances
  a fenced pause/play/seek transport, and rejects cancelled seeks or replaced
  original identities without publishing stale state;
- a synthetic native edit execution that combines the isolated screen and
  system-audio originals, applies trim/delete/2× speed and per-window audio gain
  through accurate, shared-sequence GStreamer segments, waits for every aligned
  source branch rather than the first aggregate message, bounds any closing
  screen-frame hold to one second, emits a playable VP8/Opus WebM whose measured
  duration is within 100 ms of the exact plan, decodes the result, compares the
  decoded export frame to the shared-plan preview within an explicit mean/peak
  RGB tolerance, emits a bounded monotonic progress trace, and removes
  pre-cancelled, mid-render cancelled, or failed outputs;
- real approved-profile exports that decode and verify the exact H.264/AAC MP4
  hosted-media master, VP8/Opus WebM native master, HEVC/AAC MP4 native master,
  and FFV1/FLAC Matroska archive geometry, frame rate, colorimetry, 48 kHz stereo
  audio, container/codec markers, and complete output hash; licensed profiles
  require their explicit approval environment value, and an approved profile
  without an aligned audio source fails closed;
- a separately composed macOS display/window/region desktop source and
  recording graph whose source-level checks cover permission preflight, opaque
  target selection, bounded normalized frame ingress and stop tail,
  stop/cancel, and artifact-bound Editable WebM export without claiming a
  physical capture run or Studio integration;
- journal CAS, ownership fencing, lost acknowledgement reconciliation,
  idempotent replay, stale writers, exact pending asset/render continuity,
  asset/edit/render carry-forward and exact resolution from recoverable failure,
  rejection of identity-dropping recovery exits, and every declared power-loss
  boundary, using both fake stores and the production filesystem journal store;
- production macOS recovery at every recording and edit-save crash boundary:
  empty Created/GraphPrepared attempts are archived without deleting their
  graph, CaptureStarted and every temporary/commit boundary converge on the
  same immutable original and revision-one project, and edit saves reconcile
  both precommit and lost-acknowledgement states without changing originals;
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

The native execution tests also retain the basic single-source WebM path and
the older video-only MP4 compatibility helper. The edit-aware path proves
canonical temporal/audio execution for aligned synthetic originals, bounded
progress and cancellation, a playable output duration, and exact decoded
postconditions for every approved profile. It does not prove
camera/cursor/background composition, arbitrary persisted asset assembly,
perceptual parity, or coordinator/desktop integration.

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
Studio mode additionally commits screen and optional system-audio originals
through a durable journal and creates an authenticated canonical Studio
project. The desktop now discovers that project without exposing its path and
opens a descriptor-reauthenticated completed project in the editor. It
inspects interrupted entries through opaque handles, recovers recording and
edit-save boundaries into a reminted ready-project handle, and archives only a
proven-empty attempt without deleting media. The editor now initiates bounded
trim/delete/split/speed/audio-gain drafts and persists them through the durable
edit-save journal transaction. A clean aligned screen/audio project can now
asynchronously render and identity-publish a verified distribution master
through the durable coordinator, including hardware-first H.264 with an
identical-plan software fallback. It does not yet composite the recorded
camera/cursor/background tracks into native preview/export or prove the
remaining long-project goldens and representative physical-hardware matrix, so
issue 27 remains open.

Focused command:

```text
cargo test -p frame-media --test studio_mode_contract
cargo test -p frame-media --test studio_native_recording

GST_PLUGIN_SYSTEM_PATH_1_0=/exact/pinned/gstreamer/plugin/root \
  cargo test --locked -p frame-desktop-core \
  --features tauri-app,macos-native --all-targets

GST_PLUGIN_SYSTEM_PATH_1_0=/exact/pinned/gstreamer/plugin/root \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked \
  -p frame-desktop-core --features macos-native,custom-protocol \
  production_coordinator_commits_and_cancels_exact_preopened_outputs

FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
FRAME_NATIVE_HEVC_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0=/exact/pinned/gstreamer/plugin/root \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  studio_ -- --nocapture
```

These commands exercise the provider-neutral Studio contract, production
isolated-track recorder, synthetic native-execution helper, and forced
hardware-failure/native-software-retry coordinator path. Their results must not
be reused as evidence that the macOS display source or a representative
physical hardware-failure matrix was exercised.

Full media command (using the audited plugin root discovered for this build):

```text
FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
FRAME_NATIVE_HEVC_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
```

Record fresh output from this aggregate command for the revision under review;
historical pass counts predate the current native target composition and are not
native capture evidence.

Static gate commands:

```text
cargo clippy --locked -p frame-media --all-targets -- -D warnings

RUSTDOCFLAGS=-Dwarnings cargo doc -p frame-media --no-deps

rustfmt --edition 2024 --check crates/media/src/studio.rs \
  crates/media/src/studio_edit_executor.rs \
  crates/media/src/studio_native_execution.rs \
  crates/media/src/studio_recording_native.rs \
  crates/media/tests/studio_mode_contract.rs \
  crates/media/tests/studio_native_recording.rs

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
synthetic tracks, edited preview mapping, aligned A/V execution for all four
approved profiles, single-source WebM, and gated licensed-codec evidence. The
production isolated-track recorder has both synthetic all-track coverage and a
release desktop bridge for screen plus optional microphone, camera, and system
audio. Physical capture remains protected evidence, and the desktop bridge is
not yet connected to the Studio coordinator.
Most timeline goldens remain mathematical; the decoded edited artifact now
adds a bounded RGB preview/export frame diff, but not a perceptual reference or
reference-audio diff.
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
cannot close checkboxes 8–11 while their local integrations remain absent. No
provider, hardware, user, or release claim is made by this local evidence file.
