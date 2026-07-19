# Audio/camera synchronization local evidence

Status: provider-free contracts, a concrete bounded GStreamer appsrc runtime,
descriptor-rooted macOS settings, and a target-gated ScreenCaptureKit
system-audio primitive. This record does not claim a production `NativeAvBridge`,
desktop integration, recorded audio mux, physical permission success,
Bluetooth recovery, wall-clock soak, or performance completion.

## Closure ledger boundary

Issue 25 checkboxes 1, 4, 5, 6, 7, and 8 are repository-local gaps.
Checkboxes 2, 3, 9, and 10 remain locally satisfied by the executable graph,
clock/timestamp logic, optional-device negotiation, and privacy-safe diagnostic
model. No issue-25 checkbox is currently `protected_pending`; the hardware
portions of checkboxes 1 and 6–8 become meaningful only after the local bridge
and integration gaps close.

`NativeAvBridge` still has no production implementation in this repository.
`NativeAvAppSrc` is a real CPU-byte adapter and `NativeAvRuntime` executes a
real bounded graph against hostile bridges, but no production device source
pumps owned buffers through that bridge and the coalesced events have no
desktop IPC caller. `DurableAvSettingsStore` provides the macOS adapter's
strict storage semantics without pretending to be the older unversioned
`AvSettingsStorage` trait. `MacOsSystemAudioSource` is a real target-gated
source primitive, but it intentionally stops short of the bridge's
calibration/hotplug/default/sleep-wake contract and is not muxed with screen
capture. Consequently these local results are not evidence that a release
recording contains audio or implements checkboxes 1 or 4–8.

## Contract surface exercised locally

- label-free opaque device identity, instance generation, exact formats,
  permission state, route class, and timestamp provenance;
- safe bounded versioned settings codec/storage boundary and migration with
  pinned/default confirmation rules, plus descriptor-rooted two-slot revision
  CAS, private modes, file/directory `fsync`, symlink rejection, and a zeroized
  installation secret on macOS;
- exact provider-neutral appsrc topology plus an executed GStreamer graph
  builder with one real `audiomixer`/`audioconvert`/`audioresample` path,
  per-source gain and level elements, negotiated caps, and camera conversion
  with bounded record/preview branches;
- concrete CPU-byte `NativeAvAppSrc` transfer semantics, one exact ingress
  budget partitioned across the session/appsrc/downstream queues, observable
  appsrc pressure and exact downstream queue overruns with next-buffer
  discontinuity, fair bounded runtime polling, source calibration,
  non-draining appsinks that cannot stall EOS, deadline-bounded EOS-to-`Null`
  completion, serialized empty-source TIME-segment/EOS ordering, and
  fail-closed attach/poll teardown;
- safe macOS ScreenCaptureKit system-audio format/permission/start/stop
  primitives with current-process exclusion, a 1.6-second callback prequeue,
  stable secret-bound IDs, five-second native-call deadlines, one-second queue
  fence/delegate deadlines, and a confirmed bounded callback tail;
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

The native runtime suite constructs the negotiated graph, verifies the exact
three-stage ingress partition, pushes owned CPU buffers through a real
GStreamer appsink, proves pre-transfer rejection versus post-transfer failure,
observes bounded appsrc/queue overload and next-buffer discontinuity, rotates
hostile one-buffer polls fairly, reconciles a lost Stop acknowledgement without
double release, and confirms deadline-bounded EOS/`Null` teardown. It does not
push a production device buffer, consume the mixed-media sinks as a recording,
emit production meters, or connect a UI event consumer.

The EOS regression lane executes 500 empty-source stops and 500
first-buffer-immediate stops, including a one-buffer appsrc budget. Empty stop
leaves every appsrc at zero queued buffers and every owned appsink with zero
samples; normal stop preserves exactly one unchanged 10 ms audio sample. The
required media job sets `G_DEBUG=fatal-criticals`, and workflow policy plus
mutation tests prevent that guard or the required media steps from moving to a
different job.

The sanitized full `frame-media` run is the authoritative aggregate count.
`frame-macos-av-capture` separately tests portable shape/identity bounds and
macOS lifecycle/fence behavior, while the desktop suite tests durable settings
recovery and privacy. Strict all-target Clippy and rustdoc warnings-as-errors
apply to the changed crates.

## Reproduction commands

Run from the repository root:

```bash
cargo test -p frame-media --test av_capture_contract
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  --test av_runtime_contract
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
cargo test --locked -p frame-macos-av-capture
cargo test --locked -p frame-desktop-core --features macos-native av_settings::tests
cargo clippy -p frame-media --all-targets -- -D warnings
cargo clippy -p frame-macos-av-capture --all-targets --no-deps -- -D warnings
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

Until the production bridge, shared-clock screen/audio composition, lossless
tail/mux proof, UI event connection, and recovery integration exist, this slice
is suitable for native adapter development and local conformance only. Later
hardware records cannot repair the absent release code and do not authorize
production promotion.
