# Native executor v1

## Contract

The private claim and child plan are both versioned. A new claim carries API schema 1, native
plan schema 1, the exact media job and service catalog versions, a catalog-derived execution
origin, dense zero-based source and output ordinals, and the complete sandbox envelope. The child
revalidates the serialized plan, canonical scratch location, every source size and SHA-256, output
type and limit, catalog versions, origin, cardinality, and deny-network policy before constructing
a graph.

Catalog v1 has exactly one logical output for every retained native profile. The plural output
shape is deliberate but a v1 claim or completion with zero or multiple outputs is rejected.
`segment_mux_v1` requires 2–64 sources; `composition_v1` accepts 1–64; every other profile requires
exactly one. The control plane currently has only a primary-source record, so segment mux remains
unclaimable until a source-input table and API can supply all dense descriptors.

| Profile | Origin | Sources | Catalog output | Limit | Graph implementation / typed exception |
| --- | --- | ---: | --- | --- | --- |
| `optimized_clip_v1` | managed fallback | 1 | `video/mp4` | light | `optimized_clip_h264_aac_mp4_v1`; codec approval exception |
| `thumbnail_v1` | managed fallback | 1 | `image/jpeg`, `image/png` | light | `thumbnail_png_v1` executable; JPEG variant exception |
| `spritesheet_v1` | managed fallback | 1 | `image/jpeg` | light | `sampled_contact_sheet_jpeg_v1`; sampling-contract exception |
| `audio_extract_v1` | managed fallback | 1 | `audio/mp4` | light | `audio_extract_aac_m4a_v1`; codec approval exception |
| `probe_v1` | native only | 1 | `application/json` | light | `probe_manifest_v1` executable |
| `audio_presence_v1` | native only | 1 | `application/json` | light | `audio_presence_manifest_v1` executable |
| `distribution_master_v1` | native only | 1 | `video/mp4` | heavy | `distribution_master_h264_aac_mp4_v1`; codec approval exception |
| `animated_preview_v1` | native only | 1 | `image/gif`, `video/mp4` | light | `sampled_animated_preview_gif_v1`; sampling-contract exception |
| `audio_normalize_v1` | native only | 1 | `audio/mpeg`, `audio/mp4`, `audio/wav` | light | `two_pass_audio_normalize_v1`; loudness-algorithm exception |
| `remux_repair_v1` | native only | 1 | `video/mp4` | heavy | `allowlisted_remux_repair_mp4_v1`; demux-allowlist exception |
| `segment_mux_v1` | native only | 2–64 | `video/mp4` | heavy | `ordered_segment_mux_mp4_v1`; multi-source transport exception |
| `waveform_v1` | native only | 1 | `application/json` | light | `bounded_waveform_manifest_v1` executable |
| `composition_v1` | native only | 1–64 | `video/mp4` | heavy | `timeline_composition_h264_aac_mp4_v1`; timeline transport exception |
| `normalize_v1` | native only | 1 | `video/mp4` | heavy | `normalized_h264_aac_mp4_v1`; codec approval exception |

`NativeProfile::implementation` is the executable authority for this table. It
declares the engine, graph identity, output variants, pinned factories, state,
and typed exception for all 14 profiles. The worker currently advertises only
four profiles as executable. A documented exception is terminal and
non-retryable; it does not silently fall through to a different codec,
container, or result shape. The four hybrid rows are labeled
`managed_fallback` because the control plane selected the native lane after
managed routing; the worker never makes that routing decision itself.

## Probe manifest

`probe_v1` emits compact canonical JSON with this exact field order and no optional or unknown
fields:

```text
schema_version, profile, container, video_codec, audio_codec, duration_ms,
width, height, frame_rate_numerator, frame_rate_denominator, track_count
```

The graph observes original encoded caps during decodebin autoplugging and decoded raw caps for
dimensions and frame rate. Container labels must agree with the verified source content type.
Unknown container, video codec, audio codec, dimensions, duration, frame rate, track count,
integer conversion, or provenance fails closed. Audio absence is the closed label `none`. The
worker and control plane both reserialize the manifest byte-for-byte and derive decoded-byte and
frame-count upper bounds with checked arithmetic. The worker rejects observed duration, geometry,
frame rate, frame count, decoded bytes, or tracks outside the claimed catalog envelope before
upload; the control plane persists the immutable output SHA-256 as the probe identity.

`audio_presence_v1` is likewise canonical compact JSON in the field order `schema_version`,
`profile`, `has_audio`, `track_count`. `waveform_v1` uses `schema_version`, `profile`,
`duration_ms`, `sample_rate`, `channels`, `waveform_milli`; it contains 1–4,096 integer points in
the closed range 0–1,000. The parent re-parses the exact profile schema, rejects unknown fields or
noncanonical encodings, and independently checks the metadata envelope before hashing an output.

## Codec and plugin policy

Native H.264/AAC output requires all three gates:

1. the exact service setting `FRAME_NATIVE_H264_AAC_APPROVED=approved-v1`;
2. trusted, declared encoder/mux factories from the pinned GStreamer runtime; and
3. an implemented, reviewed profile graph.

The current worker intentionally fails at gate 3 for every H.264/AAC output. The setting is an
approval input, not a capability switch. No health response, test record, or runbook may claim
H.264/AAC production output until plugin distribution, patent/license review, product approval,
and graph evidence are all attached.

## Isolation, cancellation, and cleanup

Only local `filesrc` inputs and declared output sinks are authored; claims cannot provide URLs or
arbitrary GStreamer descriptions. Dynamic decode elements must come from the pinned plugin root.
The child receives a cleared environment containing only the trusted plugin root and minimal
process paths. Unix CPU and file limits are applied before exec; the parent independently polls
RSS, scratch bytes, cancellation, child status, and the catalog wall deadline. Any monitor error
kills and reaps the child. Child exit codes form a closed privacy-safe failure protocol.
Analysis graphs also rederive observed duration, geometry, frame, track, and decoded-byte ceilings;
the thumbnail graph seeks and scales under the same process, memory, scratch, output, CPU, and wall
limits. Scratch directories receive mode `0700` atomically and their mode and canonical temp
location are checked again in the child. The child runs from that directory with core dumps
disabled, a cleared environment, and a restrictive umask.

The authored graphs contain no network source or sink, but this process sandbox is not an OS
network namespace. Production deployment must still enforce outbound deny policy at the workload
or host boundary. This is an explicit rollout gate, not evidence of syscall-level network
isolation.

Heartbeats begin before source transfer and continue through output upload and completion.
Cancellation is checked while streaming each input, while the graph runs, and while the output
request is in flight. Progress callbacks are monotonic (`1000` after verified download and `9000`
after verified output); terminal completion is owned by the control plane. Attempt cleanup is
RAII-backed and suppresses publication on every cancellation or fence loss.

## Local verification

Run with `GST_PLUGIN_SYSTEM_PATH_1_0` set to the exact value returned by
`pkg-config --variable=pluginsdir gstreamer-1.0`:

```sh
scripts/ci/gstreamer-sanitized-exec cargo test -p frame-media-worker --all-targets
cargo clippy -p frame-media-worker --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p frame-media-worker --no-deps
```

The local GStreamer tests use only procedurally generated VP8/Opus WebM media and assert the exact
probe manifest, bounded waveform, PNG signature, dense plan validation, codec-policy rejection,
monotonic protocol, cancellation cleanup, and plural streaming walking slice. They are not
evidence for H.264/AAC output, remote Cloudflare execution, production capacity, host egress
isolation, or human perceptual parity.
