# Audio, camera, and synchronization contracts

Frame's microphone, system-audio, and camera core is provider-free. The media
crate defines the identity, capability, graph, timing, ownership, recovery, and
privacy rules that every native adapter must satisfy. It does not treat a Rust
fake, a GStreamer element name, or a deterministic clock simulation as proof
that physical hardware works.

```text
OS permission and device APIs
        │
        ▼
session-bound NativeAvBridge
  ├─ exact capability + label-free device catalog
  ├─ one-shot operation tickets
  └─ revisioned control events and owner/generation/epoch-stamped raw buffers
        │
        ▼
exact settings resolution ──> explicit appsrc graph specification
        │
        ├─ session-owned per-epoch calibration / latency / drift servo
        ├─ corrected sequence-gated ingress and one-shot payload transfer
        ├─ audio gain, mute, mix, clip, and meter policy
        └─ privacy-safe throttled UI and diagnostic events
```

## Native adapter boundary

`NativeAvBridge` owns every platform object. No Core Audio, AVFoundation,
WASAPI, Media Foundation, PipeWire, PulseAudio, V4L2, portal, or GStreamer type
appears in a public contract. An adapter first accepts a non-cloneable session
claim. The bound wrapper then mints exactly one non-cloneable session-owner
ticket; a copied binding cannot construct a second state machine. All later
calls carry a private owner ticket. Native operations carry a
private, non-cloneable operation ticket containing the exact owner, operation,
source class, opaque device identity, device-instance generation, and stream
epoch.

The intended platform adapter families are:

| Platform | Microphone | System audio | Camera | Permission/lifecycle owner |
| --- | --- | --- | --- | --- |
| macOS | Core Audio | ScreenCaptureKit/Core Audio as capability allows | AVFoundation | TCC plus application lifecycle notifications |
| Windows | WASAPI capture | WASAPI loopback | Media Foundation | Windows privacy settings plus power/device notifications |
| Linux | PipeWire or PulseAudio adapter | PipeWire portal/session adapter where available | PipeWire or V4L2 adapter | desktop portal plus device/power notifications |

These are adapter selections, not execution evidence. An adapter may advertise
only capabilities it can preserve exactly. In particular, system audio is not
inferred from microphone support.

## Device identity and settings

Device IDs are opaque 128-bit, host-local stable identities produced by the
native adapter. Debug output always redacts them. A nonzero device-instance
generation changes whenever the native backing object or effective profile is
recreated. Descriptors contain only source class, default flag, permission
state, privacy-safe route class, timestamp kind, and an explicit bounded list
of complete formats. There is no label, serial number, OS handle, process ID,
or vendor/product string.

Settings version 2 has three explicit states per source:

- `Disabled` never selects a device;
- `Pinned` resolves only the same opaque ID and exact format; and
- `FollowDefault` records whether future default changes were explicitly
  authorized and, otherwise, the one confirmed ID.

The version-1 migration never matches a label and never turns an old default
choice into permission to capture a new default. Rename-only changes are
irrelevant because labels are not persisted. A missing pinned device stays
unavailable. Replug, profile, or capability changes produce a new generation
and require reconfiguration. A changed default without authorization produces
`DefaultConfirmationRequired`; it is never silently selected. An absent or
failed optional device leaves a valid graph with no optional source, allowing
the independently owned screen recording to continue.

The storage boundary is `AvSettingsStorage` plus the bounded
`AvSettingsCodec`. Its label-free version-2 DTO is at most 4 KiB, encodes
opaque IDs as exact 32-character hexadecimal values, and rejects malformed
IDs, missing or extra fields, wrong source/format pairs, and unknown versions.
A restart decodes and revalidates the complete DTO before resolution. Storage
never turns a missing pinned ID or an unconfirmed changed default into a
different selection.

The macOS desktop crate also provides a descriptor-rooted
`DurableAvSettingsStore`. One adapter-owned writer serializes compare-and-swap
mutations into two fixed revision slots. A mutation writes and `fsync`s a
private `0600` staging file, publishes it with a no-replace descriptor-relative
rename, and `fsync`s the directory before returning the new revision. Reads are
hard-capped at 4 KiB and reject symlinks, insecure modes, unknown fields,
conflicting revisions, truncation, and oversized data. The installation secret
used to derive host-local native IDs is a separate, zeroized, exact 32-byte
`0600` value. This is crash-safe storage for one process-owned writer, not a
cross-process lock.

## Exact pipeline graph

The graph is a data specification rather than a parsed pipeline string. Every
selected device and exact input/output caps tuple is retained in the operation
ticket and revalidated against a fresh complete catalog immediately before
native dispatch.

All native bridge paths use `appsrc` with these mandatory properties:

| Property | Contract value |
| --- | --- |
| live source | `is-live=true` |
| clock stamping | `do-timestamp=false`; the corrected explicit PTS is authoritative |
| flow control | `block=false`; a capture callback never waits for downstream |
| time unit | nanoseconds |
| lifetime | native buffer lease and moved byte/opaque body retained until downstream release |

Each native lease reports retained size exactly once at buffer construction;
the immutable snapshot is the only value used for queue accounting. The lease
moves either a byte body or a provider-owned opaque handle into
`AvAppSrcInput` exactly once. The local appsrc adapter owns that input until
the downstream buffer is consumed or dropped, and either path releases the
lease exactly once.

`NativeAvAppSrc` is the concrete CPU-byte implementation of that edge. It
rejects opaque payloads, source/format mismatches, and non-Playing graphs before
ownership transfer and returns the exact untouched input. Once
`gst_app::AppSrc::push_buffer` is called, downstream failure consumes authority;
the adapter never pretends the input can be recovered. The complete input is
the GStreamer buffer's backing owner, so its native lease remains live until
GStreamer releases the buffer.

Selected GStreamer topology is exact:

- microphone and system audio: `appsrc ! queue ! audioconvert ! audioresample
  ! capsfilter ! volume ! level`, followed by distinct request pads on one
  shared `audiomixer`;
- mixed audio: interleaved `F32LE`, 48 kHz, stereo before the encoder/muxer
  boundary owned by the pipeline core; and
- camera: `appsrc ! queue ! videoconvert ! videoscale ! capsfilter`, followed
  by one camera `tee` with an always-present recording branch and an optional
  preview branch. The negotiated camera tuple is retained exactly. Disabling
  preview never removes the recording branch.

Queues have nonzero, hard maxima for buffers, retained bytes, and age. Audio
defaults to 128 buffers/8 MiB/two seconds; camera defaults to eight
buffers/128 MiB/500 ms. The selected policy drops oldest or newest without
blocking. Every accepted, rejected, expired, drained, or stale buffer releases
its native lease exactly once. A format change is never accepted into the old
queue. The negotiated per-source buffer, byte, and age budget is authoritative
for the complete ingress path and is partitioned exactly across the session
queue, GStreamer `appsrc`'s internal queue, and the explicit downstream
`queue`; the three live stages cannot silently multiply that budget. Both
GStreamer stages expose bounded overload observations: `appsrc` reports
conservative pre-push saturation, while the explicit queue reports an exact
overrun signal. Newly observed pressure or overrun produces one stable,
privacy-safe `IngressOverload` diagnostic and marks the next accepted source
buffer discontinuous. Preview-owned appsinks use bounded dropping queues.
Recording-owned mixed-audio, isolated microphone/system-audio original, and
camera branches use non-leaky output queues and appsinks that do not drop;
their output is copied into bounded typed poll reports with per-branch
monotonic sequences, explicit timestamps, and an aggregate byte ceiling. The
isolated audio tee is upstream of per-source gain/mute, so later Studio edits
do not destroy the captured original. A sample that does not fit the caller's
current byte budget is retained for a later larger pull. Runtime attachment
rejects a fixed recording byte budget smaller than the largest active branch
bound, preventing an EOS drain that can never make progress. Exact queue
bounds/leak mode, sink mode, and nonblocking EOS properties are revalidated at
attach.
Some GStreamer aggregator versions append an empty or incompletely timestamped
`GAP` sentinel while reaching EOS. The output boundary consumes only those
incomplete `GAP` sentinels, counts each against the bounded pull budget, and
continues to reject incomplete non-`GAP` media.
Stop queues EOS through each `appsrc` element rather than calling the direct
end-of-stream shortcut. GstBaseSrc therefore serializes stream-start, the exact
caps, a TIME segment, and EOS even when a source produced no buffers. Terminal
EOS is accepted only when those sticky events still match the negotiated
source contract. A recording graph will not confirm `Null` after terminal EOS
until every recording appsink reports EOS with no queued sample and the
runtime-owned pending sample is empty.

`NativeAvRuntime` binds a recording-state session, exact negotiated graph, and
native bridge. It installs one authenticated startup calibration per source
epoch, moves the graph to `Playing`, polls bounded native events and bus
messages, pushes at most the configured number of buffers, returns at most the
configured recording-output samples and bytes, and coalesces only privacy-safe
status/timing events. Attach and poll contract failures attempt native terminal
reconciliation and confirm GStreamer `Null`; successful recording EOS is
reported only after output drain and `Null`. A successful Stop carries a
bounded CPU-byte callback tail that the adapter retains for identical
lost-acknowledgement reconciliation. The session authenticates that tail
against the exact owner/stamp, next per-source sequence, format, negotiated
session-stage count/byte budget including already-queued buffers, and cloned
timebase before committing any cursor. Existing session-queue buffers and the
authenticated tail are then pushed through their owned appsrc edges before
graph EOS. Cancel never publishes a tail. An adapter that cannot reproduce a
confirmed tail cannot reconcile Stop as successful.
The runtime owns a validated bounded EOS deadline, rejects a regressing caller
clock, and rotates its first-polled source so a one-buffer poll budget cannot
starve later sources. Explicit `quiesce` makes one terminal reconciliation/stop
attempt and independently confirms GStreamer `Null`; it may
retry the same sticky terminal authority after an unconfirmed result. Dropping
a runtime still in `Playing` or `EosRequested` invokes that same one-attempt
path. Adapter and graph unwinds are contained separately so a hostile native
panic cannot skip the graph's bounded five-second `Null` confirmation. Drop
does not wait for EOS and does not claim a completed artifact. Native calls are
bounded by their operation-ticket timeout contract, but safe Rust cannot
preempt an adapter that blocks after violating that contract; production
adapters still need their platform watchdog/process-isolation policy. The
desktop owner derives coarse meters from isolated typed audio outputs and routes
the runtime's mixed audio into the final WebM plus isolated audio/camera outputs
into Studio originals; no raw media crosses IPC.

## Current macOS system-audio primitive

`frame-macos-av-capture` supplies a target-gated, safe ScreenCaptureKit source
for one privacy-safe system/application mix. It excludes Frame's own process
audio, admits only exact 48 kHz stereo F32LE, accepts bounded interleaved or
planar callbacks, and derives its sole opaque device ID from the installation
secret without persisting a label or native identifier. Each callback is at
most 100 ms/38,400 bytes. The nonblocking capacity-16 prequeue therefore holds
at most 1.6 seconds/614,400 bytes; overflow drops newest and marks the next
delivered chunk discontinuous.

Shareable-content, start, and stop wrappers run behind five-second caller
deadlines with a crate-wide one-operation lease. A late result retains that
lease through owner/result destruction. Teardown uses two asynchronous
capacity-one queue fences and a delegate proof, each with a one-second caller
deadline, then requires callback-sender disconnection before returning the
bounded tail. Timeout is sticky and never claims reuse or a complete tail.

`MacOsNativeAvBridge` now wraps the source as the selected production
provider-neutral adapter. It derives a separate secret-bound adapter identity,
advertises one virtual device, collects five callback-arrival/source-PTS
samples behind a 750 ms deadline for every stream epoch, transfers subsequent
owned PCM through `NativeAvAppSrc`, probes permission at most once per second,
and consumes the process-lifetime power cursor. On Stop it converts both
already-buffered and callback-fence tail chunks into replayable terminal
buffers before clearing native state; terminal reconciliation returns the
identical tail without releasing ScreenCaptureKit twice. Calibration callbacks
are not replayed after the timebase anchor. Pause and sleep confirm native
stop; resume starts a new epoch and cannot reuse the prior calibration.

Because this ScreenCaptureKit mix is one virtual source, it has no physical
default-route or hotplug choice; catalog revision advances on permission
changes. The single-source adapter remains available as a primitive, while the
desktop product uses the combined adapter below when any optional input is
requested.

`MacOsDeviceAvBridge` is the production combined optional-input adapter. It
places that virtual system-audio source and the GStreamer microphone/camera
providers in one catalog, starts every enabled source from one host-monotonic
origin, and rebases ScreenCaptureKit callback arrival to that origin before
the ordinary per-source calibration. One session therefore owns
microphone/system mixing, isolated audio originals, camera record/preview, a
single permission/hotplug/power event stream, and one terminal reconciliation
tail. The desktop recording owner consumes it through the wrapper below.

`MacOsOptionalInputRecording` is the product-facing owner around that combined
bridge. It requests only selected permissions, resolves each confirmed current
default without authorizing silent default changes, falls back when an optional
source is denied or absent, builds the recording-mode graph, and exposes
bounded poll/control/terminal operations. The macOS desktop starts
ScreenCaptureKit with the same host-monotonic origin, calibrates five screen
callback arrivals into a `SourceTimebase`, and feeds corrected screen plus mixed
audio into the final recording graph. In Studio mode it also feeds screen,
isolated microphone/system-audio originals, and camera into the journaled
multi-track owner.

Pause stops the native optional inputs while the desktop drops screen frames
from the declared timeline. Resume validates a fresh catalog, rotates every
optional source epoch, recalibrates it, and rebases the screen timebase without
replacing either output graph. Wake invokes that same fresh-epoch recovery.
Permission revocation and catalog/profile loss disable the affected source and
serialize EOS on its graph branch when another optional source remains; the
screen recording continues. When all requested optional sources are unavailable
the wrapper retains a terminally accountable no-op input session and the
desktop records screen-only.

## Master clock and declared timeline

The pipeline uses one host monotonic master timebase. The session owns one
`SourceTimebase` for each exact active source stamp, including its stream
epoch. Native buffers contain raw source PTS, duration, arrival, latency,
discontinuity, and an epoch-local sequence; they cannot contain a caller-made
corrected PTS. Ingress accepts only buffers corrected by that active
timebase, with the next sequence and non-rollback corrected PTS:

1. Three to 31 startup samples measure `master arrival - reported latency -
   source PTS`. The median is the startup offset; spread, sample count, and
   latency provenance determine low/medium/high confidence.
2. Startup fails when the measured mapping exceeds the 80 ms charter budget.
3. Source and master elapsed time estimate drift in parts per million, with
   calibration spread treated as bounded estimator uncertainty. The policy is
   invalid unless its per-second correction capacity is at least its maximum
   admitted drift. Correction cannot make output PTS move backward.
4. Each change in the applied correction is rate-limited by the exact elapsed
   master interval; a later estimator update cannot spend a whole-session
   correction budget in one jump.
5. Every ordinary corrected frame enforces the 50 ms long-run offset budget.
   Crossing it yields `SynchronizationBudgetExceeded` and requires a declared
   discontinuity. A latency-confidence change or source/master step mismatch
   likewise requires discontinuity; neither is silently smoothed as drift.
6. Pause removes captured media from the declared timeline. Resume, sleep/wake,
   a native timestamp reset, or a declared discontinuity rebases at the prior
   output end, advances the stream epoch, marks the first buffer discontinuous,
   and never rolls PTS backward.

The default policy encodes the charter ceilings of at most 80 ms at startup,
50 ms throughout the run, and ±5,000 ppm admitted drift with at least 5 ms/s
correction capacity. Deterministic tests exercise seven rates through the
exact ±5,000 ppm bounds with bounded jitter for a simulated hour. Those tests
prove arithmetic and state invariants only; physical one-hour plots remain a
protected gate.

## Audio and preview behavior

The mixer accepts only finite, exactly shaped, interleaved blocks at the
declared rate and channel count. A missing mic or system block contributes
silence, preserving continuity. Gain is 0–4x. Gain and mute changes use a
bounded frame-count ramp, avoiding a step discontinuity. The output uses an
explicit hard or soft clipping policy and reports whether the combined signal
clipped. PTS and duration come from one rational sample-position accumulator,
not a per-block integer-duration floor, so arbitrary partitions at 44.1, 48,
and 96 kHz have no cumulative rounding drift. Meters leave the media process
only as coarse RMS/peak buckets and a boolean clip flag.

Camera preview enable/disable is an exact graph reconfiguration. Enabling
preview with no active camera resolves to disabled rather than claiming a
preview. Raw audio samples, video frames, device labels, opaque identities, and
hardware identifiers are absent from `AvUiEvent`.

The production desktop boundary has a separate versioned projection,
`DesktopInputTelemetry`. `RecorderPoll` remains an owner-scoped, payload-free
health trigger; it cannot choose a source, meter value, interval, or recording
token. The native runtime reads only a bounded 0..=10,000 meter summary from
the active recording authority and emits at most one telemetry event per
100 ms. Polls inside that interval update backend truth but are coalesced; the
next eligible event contains the latest summary. Starting a new recording
resets its sample sequence, while stop, cancel, device loss, restart, and native
failure clear pending telemetry.

The event contains its own version and nonzero sample sequence, the two bounded
audio levels, and only `disabled`, `unavailable`, or `active` camera-preview
state. Its strict decoder rejects unknown fields, so raw PCM/video, filesystem
paths, device labels, and native identifiers cannot be smuggled through an
additive property. The Leptos consumer accepts at most one event per response
from the recorder owner, requires the current runtime version and strictly
increasing envelope sequence, revalidates the telemetry payload, and retains
the last displayed meter when the server suppresses an early poll. It does not
read the newer unthrottled snapshot value as a substitute event. Tauri also
emits the same typed envelope on `frame-desktop://event-v1`; the command
response is the deterministic consumer path used by the current single-window
Leptos shell.

## Lifecycle and race rules

| Event | Required result |
| --- | --- |
| prompt required | owner-bound permission operation; denial returns to a usable screen-only state |
| denial/revocation | source queue drains, source is disabled, native reconfiguration is requested, screen recording continues |
| unplug/replug | old generation is disabled; the new generation is never auto-bound |
| default switch | followed defaults require recorded authorization or explicit confirmation |
| Bluetooth/profile/caps change | old exact format is disabled; renegotiation is required |
| overload | bounded drop policy and stable diagnostic; capture thread never blocks |
| sleep | queues drain; an established stream suspends, while a dispatched ambiguous stream requires teardown |
| wake | fresh capabilities/catalog and a new resume epoch are required before buffers are accepted |
| stop/cancel | stable terminal identity; reconcile the native postcondition before any retry; terminal state requires confirmed teardown |

An action is consumed once and is checked against the session's one pending
operation immediately before any native effect. A superseded action fails
stale. Events are accepted only through a private-field owner envelope minted
by the bound bridge. Buffers must match owner, source class, device ID,
generation, and stream epoch exactly. Epoch allocation is monotonic per source
class for the entire session, including failures and disable/re-add cycles; a
retired epoch is never reused.

Permission and catalog events additionally carry the owner, catalog revision,
and a strictly increasing control sequence. Replayed, reordered, wrong-owner,
or revision-rollback events fail closed. Any accepted permission, hotplug,
default, wireless-profile, or capability event invalidates an affected pending
start, reconfigure, or resume. If native dispatch already occurred, both its
new and predecessor stamps are retained for teardown and its held
acknowledgement can no longer install the obsolete graph.

Stop and cancel deliberately bypass catalog/permission revalidation. Their
ticket contains both acknowledged active stamps and stamps from a pending
start/reconfiguration that may already have succeeded natively. Therefore the
adapter can quiesce an ambiguous native success even when its acknowledgement
has not yet been applied locally. A delayed acknowledgement becomes stale.
Start, reconfigure, and resume retries also carry every ambiguously successful
predecessor stamp and may acknowledge only after those predecessors are
quiescent. Timeout or backend failure leaves those stamps retained for the next
reconfigure or teardown. A stop retry cannot reopen capture, and
`Stopped`/`Cancelled` is reached only after confirmed teardown. Stop/cancel is
assigned a session-scoped stable terminal ID and a validated native timeout of
at most 30 seconds. Before the first attempt and every retry, the adapter
reconciles whether that exact terminal ID was already applied without releasing
again. An applied-but-lost acknowledgement therefore returns the same terminal
result with one native release; a delayed earlier acknowledgement is stale.
The retry carries the same possibly-live stamps and has no route back to start.

## Privacy and evidence boundary

Operational diagnostics contain only contract version, optional source class,
privacy-safe route class, a coarse audio/camera capability bucket, a coarse
timing bucket, and a stable enum code. UI events contain only source class,
meter bucket, timing bucket,
calibration confidence, lifecycle, and preview state. Native media buffers and
mixed local samples redact their payload and exact timing in debug output.

Local conformance covers hostile fake adapters, exact graph negotiation,
ownership races, deterministic clock simulations, mixing, queue bounds, and
privacy structure. It does not close the platform, hardware, performance, or
human-observation gates listed in
[`av-capture-sync-local.md`](../evidence/av-capture-sync-local.md).
