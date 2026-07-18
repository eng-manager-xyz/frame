# Audio/camera synchronization local evidence

Status: provider-free local contract and native GStreamer-graph evidence. This
record does not claim a physical-device source adapter, OS permission,
Bluetooth, wall-clock soak, or performance completion.

## Closure ledger boundary

Issue 25 checkboxes 1, 4, 5, 6, 7, and 8 are repository-local gaps.
Checkboxes 2, 3, 9, and 10 remain locally satisfied by the executable graph,
clock/timestamp logic, optional-device negotiation, and privacy-safe diagnostic
model. No issue-25 checkbox is currently `protected_pending`; the hardware
portions of checkboxes 1 and 6–8 become meaningful only after the local bridge
and integration gaps close.

`NativeAvBridge`, `AvLocalAppSrcAdapter`, and `AvSettingsStorage` have no
production implementation in this repository. Their exercised implementations
are test doubles. `NativeAvGstreamerGraph` constructs a real graph, but no
production device source pumps owned buffers into it, and the UI event
coalescer has no production desktop IPC caller. Consequently synthetic timing,
mix, permission, hotplug, Bluetooth, sleep/wake, and continuity results are not
valid evidence that a release capture path implements checkboxes 1 or 4–8.

## Contract surface exercised locally

- label-free opaque device identity, instance generation, exact formats,
  permission state, route class, and timestamp provenance;
- safe bounded versioned settings codec/storage boundary and migration with
  pinned/default confirmation rules;
- exact provider-neutral appsrc topology plus an executed GStreamer graph
  builder with one real `audiomixer`/`audioconvert`/`audioresample` path,
  per-source gain and level elements, negotiated caps, and camera conversion
  with bounded record/preview branches;
- one-shot session owner, session-bound native bridge, one-shot operation
  tickets, live catalog
  revalidation, source stamps, stale/replay/cross-session rejection, and
  revisioned control events, ambiguous predecessor/teardown fencing, stable
  terminal reconciliation, and never-reused session epochs;
- bounded nonblocking ingress with immutable byte accounting, raw-to-corrected
  session timebase gating, and one-shot byte/opaque appsrc payload transfer;
- median startup calibration, reported latency confidence, drift estimation,
  correction-capacity validation, continuously enforced long-run budget,
  pause/resume/discontinuity handling, and no rollback;
- mic/system gain and mute ramps, silence continuity, explicit clipping,
  rational sample-position timelines, coarse meters, and preview toggles; and
- privacy-safe throttled UI events and diagnostic records.

## Local hostile scenarios

The external `av_capture_contract` suite covers:

- invalid/duplicate devices, defaults, formats, classes, IDs, generations,
  settings versions, and bridge capabilities;
- renamed-equivalent, missing, changed-default, unplug/replug-generation, and
  wireless profile/capability catalogs;
- permission prompt, denial/revocation, no-device screen-only fallback, and
  absent-camera preview fallback;
- exact per-source graph families/caps/appsrc properties, distinct request pads
  on one shared mixer, and explicit camera tee record/preview branches;
- executable fake-appsrc byte and opaque-handle delivery, exactly-once
  downstream release, payload transfer, byte-length checks, and a hostile lease
  whose reported size changes after the one allowed snapshot;
- superseded and cross-session operations/events, delayed old-epoch buffers,
  native snapshot changes immediately before dispatch, and start-ack/stop
  ambiguity, ambiguous reconfigure retry, sleep during ambiguous start, and
  resume snapshot revalidation, plus permission/catalog event invalidation
  before and after dispatch for hotplug/default/profile/capability reasons;
- monotonic control revision/sequence enforcement and held-ack rejection after
  an accepted control event;
- stop failure/retry/idempotency, bounded adapter timeout, stable terminal ID,
  applied-but-lost postcondition reconciliation, one native release, delayed
  acknowledgement rejection, and confirmed terminal teardown;
- bounded count/bytes/age (including consumer-side expiry while a producer is
  idle), drop-oldest/drop-newest, format mismatch, and exact
  lease release on acceptance, rejection, expiry, drain, and constructor error;
- rejection of uncorrected buffers, missing per-epoch calibration, sequence
  gaps/replays, raw PTS rollback, stale epochs, and extreme timestamp overflow;
- finite/exact audio block validation, mix continuity, gain/mute ramps, hard and
  soft clipping, silence fill, meters, declared discontinuities, and
  partition-independent 60-minute rational timelines at 44.1/48/96 kHz;
- UI throttling/coalescing and structural absence of device/media fields;
- startup confidence and the 80 ms budget; and
- deterministic 60-minute simulations through the exact -5,000 and +5,000 ppm
  bounds with bounded jitter; correction-capacity rejection; jitter just inside
  and outside 50 ms; latency-confidence transitions; saturation; and
  pause/resume/reset discontinuities. Every ordinary accepted offset remains
  within the 50 ms policy ceiling.

The native execution unit test constructs the negotiated microphone,
system-audio, and camera graph, verifies all typed appsrc/appsink handles, moves
the real pipeline to `Ready`, and confirms teardown to `Null` under the pinned
plugin policy. It does not push a production device buffer or connect a UI
event consumer.

The focused contract suite contains 54 tests. The native execution suite
contains four tests shared with Instant and Studio. The sanitized full
`frame-media` run is the authoritative aggregate count. Strict all-target
Clippy and rustdoc warnings-as-errors also apply to `frame-media`.

## Reproduction commands

Run from the repository root:

```bash
cargo test -p frame-media --test av_capture_contract
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  native_execution::tests::native_av_graph_builds_real_mixer_resampler_and_camera_paths
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
cargo clippy -p frame-media --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p frame-media --no-deps
cargo fmt --all -- --check
git diff --check
```

The final command results for this change are recorded in the commit/CI output;
this document intentionally does not copy machine-specific paths or logs.

## Hardware evidence not yet valid

The following will remain protected evidence after the repository-local gaps
close. It must not currently be used to reclassify those gaps as protected or
be inferred from local tests:

- macOS, Windows, and Linux physical microphones and cameras across the declared
  built-in, wired, virtual, and wireless route matrix;
- ScreenCaptureKit/Core Audio, WASAPI loopback, and PipeWire/portal system-audio
  permission prompts, denial, revocation, and recovery;
- physical unplug/replug, default-device changes, Bluetooth wideband/telephony
  changes, native format renegotiation, and sleep/wake on every target OS;
- native appsrc buffer mapping and lifetime, real encoded/muxed media probes,
  audible mute/gain continuity, camera preview observation, and A/V content
  alignment;
- 60-minute wall-clock recordings and privacy-reviewed sync plots;
- CPU, memory, callback latency, queue depth, drop rate, and thermal comparison
  to the approved Cap baseline;
- overload injection on real adapters and confirmation that screen-only capture
  continues; and
- product, media, privacy, accessibility, and release-owner signoff.

Until the production bridges, durable settings adapter, UI event connection,
and recovery integration exist, this slice is suitable for native adapter
development and local conformance only. Later hardware records cannot repair
the absent release code and do not authorize production promotion.
