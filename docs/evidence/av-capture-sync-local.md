# Audio/camera synchronization local evidence

Status: repository-local integration complete, protected hardware evidence
pending. The production macOS desktop owns one shared host-monotonic screen/A/V
timeline, a bounded GStreamer appsrc runtime for microphone, system audio, and
camera, mixed audio into the final WebM, isolated Studio originals, live
controls, epoch-rotating pause/resume and wake recovery, source-loss fallback,
and server-throttled telemetry consumed by Leptos. This record does not claim
physical permission success, audible/visual continuity, Bluetooth recovery,
wall-clock soak, cross-OS parity, or performance completion.

## Closure ledger boundary

Issue 25 checkbox 1 is locally satisfied by the normalized contracts plus the
production macOS system-audio and microphone/camera native-to-appsrc bridges.
Checkbox 4 is locally satisfied by the validated V2 codec, migration rules, and the
revisioned descriptor-rooted `DurableAvSettingsStore` implementation of the
provider-neutral `AvSettingsStorage` boundary. Checkbox 5 is locally satisfied
by the versioned desktop input-telemetry event, native meter adapter, runtime
throttle/coalescer, strict WebView decoder, and Leptos meter/preview-state
consumer. Checkboxes 2, 3, 9, and 10 remain locally satisfied by the executable
graph, clock/timestamp logic, optional-device negotiation, and privacy-safe
diagnostic model. The repository-local portions of checkboxes 6, 7, and 8 are
now satisfied by the product recording owner described below. Those three
checkboxes are `protected_pending` because their remaining claims require
representative physical devices, recorded media, and the declared OS matrix.
Issue 25 has no remaining repository-local gap, but it is not production
complete.

`MacOsNativeAvBridge` now pumps the production ScreenCaptureKit source through
the real owned-byte `NativeAvAppSrc` runtime. It supplies callback-derived
per-epoch calibration, permission revisions, process sleep/wake, pause/resume,
and stable terminal reconciliation. Its one virtual system-mix device has no
physical hotplug/default-route choice. `DurableAvSettingsStore` now implements
the provider-neutral `AvSettingsStorage` interface on top of its revisioned,
descriptor-rooted CAS.

`MacOsDeviceAvBridge` now composes the ScreenCaptureKit system/application mix
with the audited macOS GStreamer microphone/camera providers under one adapter
identity and session clock. It retains a selected `GstDevice` without exposing
its label or provider identifier, and derives only a secret-bound opaque
public ID. `osxaudiosrc` normalizes into exact 48 kHz stereo F32LE and
`avfvideosrc`
normalizes into exact 1280×720/30 BGRA. Three-buffer leaky source queues and
appsinks bound retained data; samples require exact shape, explicit PTS and
duration, trusted plugin provenance, and an owned copy before entering the
existing Frame appsrc graph. Device-monitor events rotate the catalog for
hotplug/default changes. All three optional source classes share one
master-arrival origin, calibrate per epoch, poll fairly, and confirm
multi-source teardown.

The macOS desktop now selects `MacOsDeviceAvBridge` whenever microphone, system
audio, or camera is requested. `MacOsOptionalInputRecording` owns permission
fallback, exact default-device resolution, the negotiated recording-mode
GStreamer graph, and terminal reconciliation. ScreenCaptureKit starts with the
same `Instant` used by the optional-input bridge. Five callback-arrival samples
calibrate the screen `SourceTimebase`; each optional source independently
calibrates to that same host-monotonic origin. The worker sends corrected screen
frames and mixed audio to the final WebM and sends corrected screen, isolated
microphone/system originals, and camera samples to the journal-fenced Studio
recording. The older direct source remains only for the screen-only path.

Live microphone/system controls change the owned mixer branch without replacing
the graph or isolated original. Camera track inclusion gates new Studio camera
samples; pausing stops every optional native input, drops screen frames from the
declared timeline, and resume rotates/recalibrates source epochs in the same
output graph. Wake performs that same fresh-catalog epoch recovery
automatically. Permission revocation, unplug, default change, wireless-profile
change, and format change retire the affected graph branch when another source
remains. If no optional source is available, the product owner retains a
screen-only recording instead of failing start. Privacy-safe diagnostic
class/capability/timing codes are logged without labels or native identifiers.

The recorder health command is a bounded trigger at 100 ms, but
the native runtime is the authority for whether a UI event may be emitted. It
coalesces repeated observations, emits at most once per 100 ms, and sends only
0..=10,000 microphone/system levels plus `disabled`/`unavailable`/`active`
camera-preview state. The Leptos consumer validates runtime/event versions,
strictly increasing event-envelope sequence, event ownership, payload bounds,
and at most one telemetry event per response; when an event is suppressed it
retains the last displayed meter rather than reading an unthrottled snapshot.
Unknown telemetry fields fail deserialization, and raw PCM/video, paths, labels,
and device identities have no field in the event.

The normalized A/V runtime has a recording mode that returns bounded,
typed mixed-audio, isolated microphone/system-audio original, and camera
samples and holds EOS completion until every recording sink is drained. The
isolated audio branches tee before gain/mute. Stop now authenticates every
retained session buffer and the
complete replayable native callback tail, pushes them before graph EOS, and
replays the identical tail after a lost acknowledgement without a second
native release. The desktop optional-input worker consumes all four output
kinds, preserves their timestamps and per-branch sequences, and drains the
runtime to confirmed EOS/`Null` before finishing either the flattened artifact
or Studio originals. The shared-clock normalizer rejects pre-origin samples,
enforces the startup and long-run sync policies, rebases screen and optional
inputs across pause/resume, and does not publish an optimistic artifact after a
terminal failure.
GStreamer-version-specific incomplete terminal `GAP` sentinels are consumed
without being exposed as media, every consumed sentinel counts against the
same bounded pull budget, and incomplete non-`GAP` samples still fail closed.
Consequently no issue-25 checkbox remains a repository-local gap. The physical
observations required by checkboxes 6–8 remain protected pending.

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
  discontinuity, fair bounded runtime polling, source calibration, distinct
  dropping preview and non-leaky/non-dropping recording output branches,
  bounded typed mixed-audio/isolated-audio/camera output pulls, retained
  over-budget samples,
  an attach-time progress budget, replayable bounded CPU-byte terminal tails,
  transactional stamp/sequence/format/timebase authentication, queued-plus-tail
  appsrc delivery before EOS, an EOS output-drain barrier,
  deadline-bounded EOS-to-`Null` completion, serialized empty-source
  TIME-segment/EOS ordering, and
  fail-closed attach/poll teardown plus one-attempt abandonment cleanup;
- safe macOS ScreenCaptureKit system-audio format/permission/start/stop
  primitives with current-process exclusion, a 1.6-second callback prequeue,
  stable secret-bound IDs, five-second native-call deadlines, one-second queue
  fence/delegate deadlines, and a confirmed bounded callback tail;
- a production macOS system-audio `NativeAvBridge` with a separate
  secret-bound adapter ID, five-sample/750 ms per-epoch calibration, owned PCM
  appsrc transfer, one-second permission probes, process sleep/wake,
  pause/resume epoch rotation, and stable terminal reconciliation;
- a production combined macOS optional-input `NativeAvBridge` with one
  ScreenCaptureKit system-audio virtual device, HMAC-redacted GStreamer
  microphone/camera catalogs, real selected-device elements, audited
  `osxaudiosrc`/`avfvideosrc` factories, bounded three-buffer appsinks, exact
  PCM/BGRA shape validation, a shared arrival clock, fair source polling,
  hotplug/default-change catalog events, per-epoch calibration, and confirmed
  multi-source teardown;
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
- privacy-safe throttled media-runtime events and the production desktop
  `InputTelemetry` event/Leptos consumer, plus diagnostic records.

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
three-stage ingress partition, pushes owned CPU buffers through real
GStreamer appsinks, proves pre-transfer rejection versus post-transfer
failure, observes bounded appsrc/queue overload and next-buffer discontinuity,
rotates hostile one-buffer polls fairly, reconciles a lost Stop acknowledgement
without double release, replays and authenticates the identical callback tail,
observes that tail on appsrc before EOS, rejects a tail beyond its negotiated
source budget, proves queued buffers and the tail share that same stage budget,
and confirms deadline-bounded EOS/`Null` teardown. The recording-mode cases
prove that recording sinks cannot silently switch to drop, one over-budget
output remains recoverable, poll output stays inside sample/byte limits with
monotonic per-branch sequences, an undersized fixed
budget is rejected at attach, and `Null` is refused until queued EOS output is
drained. Live-control cases prove that microphone/system gain and mute preserve
one graph and isolated originals, explicit pause/resume rotates and recalibrates
source epochs, sleep/wake performs the same recovery automatically, and
permission loss retires only the failed branch before a clean terminal drain.
Running and EOS-requested abandonment tests prove that Drop attempts
native quiescence and confirms the graph `Null` without a second release. A
hostile adapter-panic test
proves the unwind is contained, the graph is still confirmed `Null`, and native
authority remains explicitly unconfirmed; explicit `quiesce` then reconciles
the same terminal ID on retry. This is a one-attempt destructor safeguard, not
a hard preemption boundary: an adapter that ignores its operation-ticket
timeout can still block its caller and needs a platform watchdog or process
isolation. The suite does not push a physical production device buffer. The
desktop suite exercises the shared screen clock, typed mixed/isolated/camera
output routing, Studio optional-track ownership, backend-confirmed
pause/resume/live controls, server-side telemetry coalescing, strict
raw-media-free IPC, and Leptos release compilation. The macOS
bridge suites push fake-native system-audio, microphone, and camera buffers
through the exact bridge cores and real GStreamer appsrc runtime; physical
ScreenCaptureKit, `osxaudiosrc`, and `avfvideosrc` execution remains protected.

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
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked \
  -p frame-macos-av-capture
cargo test --locked -p frame-desktop-core --features macos-native av_settings::tests
cargo test --locked -p frame-desktop-core input_telemetry
cargo test --locked -p frame-desktop-core native_input_events
cargo clippy --locked -p frame-desktop-core --features macos-native --all-targets -- -D warnings
python3 scripts/ci/build-desktop-ui.py
python3 scripts/ci/check-desktop-bundle.py
cargo clippy -p frame-media --all-targets -- -D warnings
cargo clippy -p frame-macos-av-capture --all-targets --no-deps -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p frame-media --no-deps
cargo fmt --all -- --check
git diff --check
```

The final command results for this change are recorded in the commit/CI output;
this document intentionally does not copy machine-specific paths or logs.

## Hardware evidence not yet valid

The following is protected evidence after the repository-local integration
closed. It cannot be inferred from local tests:

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

The repository-local product path now owns shared-clock screen/audio
composition, desktop mux routing, mixed-source controls, and defined recovery.
Only representative hardware evidence can close the protected rows above; local
tests cannot manufacture that evidence and do not authorize production
promotion.
