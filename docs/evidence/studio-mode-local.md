# Studio Mode local evidence

Issue: `_issues/27-p4-studio-mode.md`

This record separates executable provider-neutral evidence from protected
native, historical, hardware, UX, and release evidence.

## Executed locally

The external contract suite exercises:

- canonical project/edit/journal round trips, deterministic bytes, checksum
  corruption, newer schemas, malformed framing, and trailing-byte rejection;
- the production filesystem `.cap` adapter against a schema-shaped directory
  fixture: typed JSON decoding, exact decimal-time conversion, streaming source
  hashes, pinned flattened single-segment and multiple-segment forms, known
  default fields, unsupported-effect and newer-version reporting, normalized
  paths, symlink/traversal/missing-file rejection, and an asset-bound copy plan;
- four isolated screen/camera/microphone/system-audio graph branches and
  rejection of flattened/duplicate tracks, plus a filesystem recording-session
  adapter that seals four independent pre-encoded temporary originals and
  reopens a crash state containing both partial and already-sealed tracks;
- a real GStreamer execution adapter that records four independently playable
  synthetic screen/camera/microphone/system-audio originals on one pipeline
  clock, validates nontrivial immutable outputs, decodes a bounded RGB preview
  frame at a requested position, and tears down every graph to `Null`;
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

The native execution test also decodes and re-encodes a playable WebM export,
decodes that output again, and, behind the explicit codec-decision gate, emits
an MP4 distribution master that a second GStreamer demux/parser graph consumes
to EOS. These are executable media artifacts rather than checksum-only bundles.

Focused command:

```text
cargo test -p frame-media --test studio_mode_contract
43 passed; 0 failed

FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0=/exact/pinned/gstreamer/plugin/root \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  native_execution::tests::studio_tracks_preview_and_webm_export_are_real_and_playable
pass
```

Full media command (using the same exact plugin root embedded at build time):

```text
FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0=/opt/homebrew/Cellar/gstreamer/1.28.0_2/lib/gstreamer-1.0 \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
261 passed; 0 failed (75 unit + 54 A/V + 5 conformance + 26 Instant +
4 media-service + 54 screen + 43 Studio)
```

Static gates:

```text
cargo clippy -p frame-media --all-targets -- -D warnings
pass

RUSTDOCFLAGS=-Dwarnings cargo doc -p frame-media --no-deps
pass

rustfmt --edition 2024 --check crates/media/src/studio.rs crates/media/tests/studio_mode_contract.rs
pass

python3 scripts/ci/check-secrets.py
self-test: 5 fixtures; tracked scan: 392 files; pass
```

The exact Studio-owned files also passed whitespace and secret-marker scans.
Production modules contain no intentional panic/todo boundary and use no
unsafe media bridge. The repository-wide format, lint, and test commands are
rerun at the aggregate gate after concurrent issue lanes merge.

## Synthetic fixtures only

`fixtures/studio/cap-schema-supported/` is a locally authored directory-schema
fixture. Its JSON and descriptor payloads prove the production parser, copy
plan, normalized path, segment, fingerprint, and read-only behavior, but it is
not a historical Cap project and its media-named files are not encoded samples.
The contract suite uses deterministic fake native ports alongside production
filesystem durability adapters. Its reference renderer still writes a
canonical checksum-bound bundle, while the separate native execution adapter
now supplies real track, preview, WebM, and gated MP4 evidence. Timeline
goldens remain mathematical and are not claimed as perceptual-diff evidence.
The JSON keys and non-fragmented `.mp4`/`.ogg` paths were checked against
`crates/project/src/meta.rs`, `crates/project/src/configuration.rs`, and
`crates/recording/src/studio_recording.rs` at the pinned revision.

## Protected evidence not collected

The following remains required before issue closure and must not be inferred
from local contract tests:

- a privacy-reviewed, provenance-pinned real legacy Cap project corpus at the
  referenced Cap revision, including supported and unsupported effects;
- encoded preview/export/reference frame, perceptual, color, and audio diffs
  within approved tolerances;
- physical-device native capture plus content-level playback/seek/mux/export
  comparison on every supported release OS;
- H.264, HEVC, AAC, VP9, Opus, FFV1, and FLAC availability/licensing results on
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

Absence of any protected record blocks promotion. No provider, hardware, user,
or release claim is made by this local evidence file.
