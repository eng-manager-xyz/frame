# Studio Mode v1 contracts

Status: provider-neutral contracts and durable filesystem reference adapters are
implemented; native codec/platform adapters and protected parity evidence remain
separate gates.

## Durable documents

Studio persists v1 project, asset, and edit documents plus v2 operation
journals. Journal v2 adds the immutable renderer fence and structured terminal
render receipt. Each uses a canonical bounded binary envelope:

```
FRST | kind:u8 | schema:u16 | payload_len:u32 | canonical payload | sha256:32
```

All integers are big-endian. Collections and strings are length framed, maps are
ordered, IDs are fixed-width, unknown enum tags fail closed, and trailing bytes
are rejected. The decoder limits the complete document to 32 MiB before parsing
and separately limits assets, edit operations, receipts, source-name bytes, and
VFR samples. A successful decode verifies the SHA-256 envelope and then reruns
the complete model invariants. Encoding the same value therefore produces the
same bytes and digest.

This is a purpose-built persistence codec, not Serde. Serde JSON is used only by
the read-only legacy Cap adapter. Schema 0 is never silently decoded as schema
1, and a v1 journal is not guessed as v2. The v1 project decoder returns
`LegacyImportRequired` for schema 0;
callers must use the separate read-only `.cap` directory adapter. Newer v1
schemas are reported without partial interpretation.

Project, asset, worker, operation, and export identities are caller-minted
128-bit CSPRNG values. Zero values are reserved. Debug output redacts identities,
checksums, and source names. Asset records retain their original checksum, byte
length, start, duration, track role, and exact native ticks-per-second timebase.
Original assets can only be `DurableOriginal`; a `Temporary` asset is legal only
inside the recording/recovery boundary.

## Legacy Cap compatibility

Compatibility is pinned to
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`. The core accepts a
normalized, bounded snapshot of that revision's `.cap` directory layout:
`recording-meta.json`, `project-config.json`, flattened legacy single-segment or
sequential multiple-segment metadata, and referenced display, camera,
microphone, and system-audio files below `content/`. Relative paths, nonempty
file metadata, contiguous segment timing, unique paths, the pinned revision
digest, and the complete imported-source identity digest are validated before
import.

`LegacyCapProjectPort` exposes only source fingerprinting and snapshot reads. It
has no mutation callback. `FilesystemLegacyCapProjectPort` is the production
directory adapter: it parses typed `recording-meta.json` and
`project-config.json` views, rejects traversal and symlink components, requires
regular nonempty referenced files, and streams every file through SHA-256. The
source tree is fingerprinted before inspection, against the normalized
snapshot, and after the outcome. Any change aborts the import. The caller must
supply new opaque v1 project/asset IDs; the original paths and bytes remain
untouched. A successful import includes a validated `LegacyCopyPlan` whose
entries bind each source relative path/checksum/length to a new asset ID and
portable destination. Supported edit operations are copied into the v1
manifest. Zoom, scenes, masks, text, captions, keyboard overlays, imported
audio, clips, annotations, chroma key, drop shadow, spring keyframes, external
fonts, and unknown fields produce an actionable compatibility report and no v1
project. A newer snapshot version similarly returns a report rather than being
guessed or decoded as the pinned schema.

The checked-in `cap-schema-supported/` directory is a locally authored,
schema-faithful fixture. Its media files are descriptor payloads, not encoded
reference media. It is not a privacy-reviewed historical Cap corpus and does
not establish product parity.

## Recording graph and original assets

The recording graph is a structured adapter contract, not a string pipeline.
It requires exactly four isolated branches:

| Track | Native source family | Recording codec | Temporary container |
| --- | --- | --- | --- |
| screen | screen bridge | VP9 | Matroska |
| camera | camera bridge | VP9 | Matroska |
| microphone | microphone bridge | Opus or FLAC | Matroska |
| system audio | system-audio bridge | Opus or FLAC | Matroska |

Every branch has a unique opaque asset ID, unique temporary name, exact
timebase, common monotonic clock identity, and a queue bounded by buffers, bytes,
and time. Queue declarations must be nonzero and may not exceed 512 buffers,
256 MiB, or five seconds; adapter-provided larger values fail closed. A flattened
recording cannot satisfy the graph validator.

`FilesystemStudioRecordingSession` is a non-test sink for this graph. Native
bridges feed bounded pre-encoded chunks to four distinct files; finish syncs and
seals each track independently into the temporary namespace. It does not
substitute a flattened master. `FilesystemStudioOriginalStore` verifies bytes
while staging and committing, uses per-asset locks, same-filesystem rename,
canonical sidecars, file and directory sync, and never overwrites an existing
original. If power fails after the media rename but before the sidecar write, a
retry verifies the orphaned original and completes the sidecar without
requiring the already-consumed temporary path.

Dropping an unfinished filesystem recording session syncs and preserves its
partials; it never treats an unwind as authorization to destroy captured media.
`FilesystemStudioRecordingSession::recover` requires the exact original graph,
rehashes every retained file under the per-track byte ceiling, reopens partials
for append, and treats already-sealed temporary tracks as immutable. This also
covers a crash in the middle of the four-track seal, where some tracks are
partials and others are already in the temporary namespace.

Temporary media becomes an original only through a non-cloneable commit ticket
bound to project, operation, asset metadata, checksum, and journal fence. A lost
commit acknowledgement is reconciled by probing the exact original identity.
Any byte length, checksum, timing, source name, or track mismatch fails. Editing,
preview, and export reference originals; none receives a mutation port for them.

Edit persistence likewise uses a non-cloneable save ticket containing the full
next manifest, expected project revision, operation ID, and journal fence. The
core advances the project and edit revision together, retains byte-for-byte
identical original asset records, and reconciles a lost acknowledgement by
probing the exact next manifest. A save is rejected before persistence when its
trim and delete ranges remove the complete renderable screen timeline.
`FilesystemStudioProjectStore` also persists a checksummed per-project maximum
fence under the same project lock. Once a newer owner claims that fence, an old
store instance is rejected even if its in-memory ticket and instance fence
still agree.

## Journal, fencing, and crash safety

The journal is compare-and-swap state with a monotonically increasing revision,
ownership fence, operation receipts, pending asset commit, pending edit save,
and pending render identity. Together, the journal snapshot and pending render
bind the project/export/operation IDs, immutable renderer fence, exact source-set digest, exact
edit-plan digest, full render-spec digest, profile, and output name.
Receipts bind an opaque operation ID to command and outcome digests. Reusing an
operation ID for a different command is rejected. Lost CAS acknowledgement is
accepted only after a read proves the expected receipt and postcondition.
Ownership transfer increments the journal-owner fence, making callbacks from
the old worker stale without rewriting the renderer fence required for exact
cleanup. `FilesystemStudioJournalStore` serializes CAS with a per-project
create-new lock and persists the canonical snapshot through a same-directory
temporary file, file sync, atomic rename, and directory sync.

Transitions preserve the exact pending asset and render value across their
permitted boundary ranges. A recoverable failure carries every pending asset,
edit, or render identity forward instead of erasing or rewriting it. Temp
durability/request/commit must name the same asset, and render prepared/running/
finalizing/terminal must name the same render spec. The edit-save port separately
probes the exact next manifest, and the prepared edit identity remains durable
through `EditSaveCommitted`. Exiting `FailedRecoverably` may resolve an exact
temporary asset, edit, or render identity through its matching committed or
cancelled boundary; a generic resume cannot discard a pending identity. Any
identity drift is corruption, not a recovery choice.

Recording, temporary durability, atomic asset commit, edit save, render start,
render finalization, and cancellation are explicit journal boundaries. Every
boundary maps to one recovery directive; there is no generic “continue” branch.
See [Studio recovery](../operations/studio-mode-recovery.md).

## One deterministic timeline

`StudioTimelineCompiler` is the sole edit interpreter. Preview and export each
receive the same immutable `CanonicalEditPlan` and plan digest. The plan digest
also binds the complete saved edit spec and the complete timeline topology
(duration, coverage, and VFR points), so an equal duration or edit revision is
not treated as an equal source. Time is expressed as rational ticks/timebase or
reduced rational seconds. Comparisons, speed maps, seek, VFR mapping, CFR
simulation, and audio-block simulation use integer math; no floating-point
conversion is allowed.

The compiler:

- validates the outer trim against the source, requires every other range to
  remain within it, and requires each split point to be strictly inside it;
- rejects overlap within duration-changing, layout, camera, cursor, background,
  and per-audio-track effect categories;
- partitions the source at every edit boundary;
- removes deleted spans and applies exact rational speed to retained spans;
- attaches deterministic layout, camera rectangle, cursor, background, gain,
  and mute state to every compiled span;
- rejects any screen gap, inserts explicit silence for audio gaps, and hides the
  camera only for its exact uncovered ranges;
- validates strictly increasing bounded VFR presentation timestamps; and
- detects checked arithmetic overflow and empty output.

A seek returns an exact source-span origin plus rational offset. Frame and audio
golden simulations are explicitly bounded, so a malformed “long project” cannot
allocate without limit.

## Preview and export graphs

Preview owns bounded decoded-video and audio queues and seeks through the
canonical plan. Export builds a structured graph from source demux, timeline
mapper, compositor, mixer, color conversion, selected video/audio encoders,
container muxer, and atomic file sink. Both graphs receive a `StudioSourceSet`
whose digest covers project/revision, complete saved edits, complete timeline
topology, and every immutable original asset descriptor. Asset ranges must equal
timeline coverage one-for-one and their maximum end must equal the declared
duration; a convenient monolithic screen asset is neither assumed nor
synthesized. `FilesystemStudioProjectStore` executes fenced project creation and
edit CAS. `FilesystemStudioRenderer` is a durable local reference executor: it
streams verified immutable originals into a canonical checksum-bound export
bundle and persists inflight/terminal sidecars so probe and cleanup survive
restart. This exercises recording storage, preview-plan, render dispatch,
export, receipt, and recovery without fake ports. The bundle is not claimed to
be a playable MP4/WebM/Matroska or a substitute for the native
GStreamer/Skia/hardware-codec adapter and protected frame/audio evidence.

Approved profiles are exact:

| Profile | Container/video/audio | Geometry/timing/color | Purpose |
| --- | --- | --- | --- |
| Distribution master | MP4 / H.264 / AAC-LC | 1920×1080, 30 fps, BT.709 limited, 48 kHz stereo | approved hosted-media input contract |
| Native HQ WebM | WebM / VP9 / Opus | 2560×1440, 60 fps, BT.709 full, 48 kHz stereo | high-quality native sharing |
| Native HQ HEVC | MP4 / HEVC / AAC-LC | 3840×2160, 60 fps, Display-P3, 48 kHz stereo | licensed native output |
| Native archive | Matroska / FFV1 / FLAC | 3840×2160, 60 fps, BT.709 full, 48 kHz stereo | lossless archival output |

Dispatch re-reads renderer capabilities. Container, codec, maximum resolution,
maximum frame rate, cancellation, postcondition probing, exact cleanup, bounded
queues, and H.264/HEVC/AAC licenses must all match. Hardware is preferred when
available. A hardware failure may restart the identical plan in software only
after the exact partial has been deleted. The retry must preserve the source
set, edit plan, profile, output target, and render-spec digest while using new
export/operation IDs; otherwise the export fails safely and the project remains
intact.

## Renderer protocol

The render ticket is non-cloneable and binds project, export, operation, fence,
portable lowercase output name, canonical graph, and bounded deadline. Its
render-spec digest covers the render protocol, source-set digest, canonical plan
digest, profile, and output name. Encoder backend is intentionally excluded so
an authorized hardware-to-software retry can preserve the identical requested
render. Starts may lose their acknowledgement, so the coordinator probes for
the exact fence and render-spec digest. A matching running or committed
postcondition reconciles; a matching partial is deleted before returning an
ambiguity error. Stale project/export/fence/render-spec or non-contiguous event
sequences fail closed. A ticket cannot reach `renderer.start` directly: a
`RenderJournalAuthorization` must be minted by consuming a created or reopened
`DurableStudioJournal`, then bound into a one-use `AuthorizedRenderDispatch`.

Output names reject path syntax, uppercase/nonportable characters, trailing
dots, and Windows reserved stems. A project/output pair is reserved for the
life of its unreleased session. A distinct export cannot collide with that
target; an authorized software fallback transfers the reservation only after
the failed partial is confirmed absent. Reservation is installed before native
start dispatch. A lost acknowledgement followed by `Absent` is still
ownership-uncertain because publication may be delayed; it remains quarantined
until an exact fenced cancel, exact cleanup, and a second absence probe all
succeed. Probe errors, mismatched postconditions, and unconfirmed cleanup
retain a quarantined session that cannot be released or replaced. A terminal
session becomes releasable only after an exact committed receipt or confirmed
partial absence.

On process restart, the shell opens every Studio journal before allowing new
render dispatch, consumes each durable pending render into a non-forgeable
authorization, and supplies all recovered authorizations to
`StudioRenderCoordinator::new`. Recovered
running/partial outputs remain reserved until exact cancellation, cleanup, and
absence probing succeed. `PendingRender` carries the structured terminal
receipt (project/export/operation identity, immutable renderer fence, source,
plan and render-spec digests, profile/output name, checksum, and byte length).
`adopt_recovered_commit(export_id)` accepts no caller checksum or length; it can
use only an exact renderer probe and the journal-bound receipt. If an exact
renderer commit exists while the recovered journal is still preterminal, the
coordinator first advances the journal through the required render boundaries
and CAS-persists the full receipt; only the resulting durable receipt may be
adopted. Thus an in-memory coordinator restart cannot silently forget durable
output ownership or bless caller-supplied output values.

Progress is monotonic in both ordered phase and basis points. Callers can read a
snapshot containing phase, basis points, sequence, terminal state, and a bounded
public failure code, and can drain an explicitly bounded FIFO of accepted
events. A commit requires 10000 basis points, nonzero output, and a matching
committed postcondition before the structured receipt is CAS-persisted in
journal v2 and mirrored into the coordinator. The reservation is not releasable
before that durable write. Cancelled and failed renders delete the exact
fence/render-spec/output partial and probe for absence. Renderer events and all
control payload chunks are bounded. Small one-shot control assets additionally
verify declared length, chunk size, early EOF/extra bytes, and SHA-256; media
originals remain adapter-owned streams.
