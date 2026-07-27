# frame-macos-av-capture

Safe, target-gated ScreenCaptureKit system-audio capture for Issue 25. This
crate captures one privacy-safe source: the macOS system/application mix,
excluding Frame's own process audio. It does not capture microphone or camera
input and does not claim that the normalized preview runtime losslessly muxes
audio into a recording artifact.

## Current boundary

`MacOsSystemAudioSource` remains the narrow raw native owner.
`MacOsNativeAvBridge` wraps it in the provider-neutral session contract:

- one secret-bound virtual device and one secret-bound adapter identity;
- exact owner, generation, and per-epoch source stamps;
- five callback-derived startup calibration samples retained only for
  calibration, followed by a fresh bounded ingress sequence;
- owned PCM leases transferred through the real CPU `appsrc` adapter;
- permission revision events, process-lifetime sleep/wake events, pause/resume,
  and stable terminal reconciliation; and
- a one-second permission probe that never exports labels, native IDs, or raw
  media through UI/diagnostic events.

The source has no physical default-route or hotplug choice: it represents the
single ScreenCaptureKit application/system mix. Its catalog therefore changes
only when permission changes. Microphone and camera adapters remain separate
Issue 25 work, and the desktop recording composition still uses its direct
screen/system-audio worker until the Studio journal can authenticate and drain
the normalized runtime's terminal callback tail.

The only admitted format is interleaved stereo F32LE PCM at 48 kHz. Native
ScreenCaptureKit output must report linear PCM, float, little-endian, 32 bits
per channel, 48 kHz, and two channels. Both one-buffer interleaved stereo and
two-buffer planar mono layouts are accepted; planar input is copied into the
same interleaved output. Format changes, non-finite samples, malformed layouts,
and oversized chunks are rejected and force a discontinuity on the next valid
chunk.

## Lifecycle and permission

1. Construct a source with a 32-byte installation secret. Its sole
   `AvDeviceId` is HMAC-derived from a fixed domain; debug output is redacted
   and no display, process, application, or raw macOS identifier leaves the
   crate.
2. Call `preflight_permission`. Call `request_permission` only from an explicit
   user action. System-audio capture uses macOS Screen Recording permission and
   may require an application relaunch after the first grant.
3. Call `start`, poll `poll_chunk`, and finish with
   `stop_and_drain_chunks`. A clean stop returns the complete bounded callback
   tail. `stop` deliberately discards that tail.
4. A completely stopped source may start again. A timeout, unexpected native
   stop, or unconfirmed callback teardown is sticky: future start/poll/stop
   calls fail closed rather than reusing ambiguous native authority.

The normalized bridge collects five samples behind a 750 ms ceiling. Each
sample carries callback arrival relative to the stream start and raw native
PTS relative to the first callback. Calibration callbacks are not replayed
after the timebase anchor; the next callback begins ingress sequence one.
Pause or sleep confirms native stop. Resume starts a new stream epoch and
requires a new calibration batch before PCM can enter `appsrc`.

ScreenCaptureKit 8's shareable-content, start, and stop wrappers use an
unbounded native completion wait. Each call runs on a named helper with a
five-second caller deadline. A crate-wide lease allows at most one stuck native
wait and stranded stream owner. Timeout drops the result receiver and detaches
the helper; source `Drop` never joins it. If a late start returns, the helper
drops the returned stream before releasing the lease.

After a timely native stop, teardown fences the owned serial callback queue,
keeps the Rust handler installed while dropping `SCStream`, waits for the
delegate/context drop proof, fences again, and requires the bounded callback
sender to be disconnected before returning the tail. Each fence is an
asynchronous capacity-one acknowledgement with a one-second caller deadline;
the delegate proof has its own one-second deadline. A timed-out fence retains a
sticky unconfirmed-teardown state and never claims a complete callback tail.
Its late closure owns only an acknowledgement sender and uses `try_send`, so it
cannot access released capture state or block after the receiver is gone.

## Resource bounds

- Every callback admits at most 4,800 stereo frames: 100 ms / 38,400 bytes.
- The callback queue is `sync_channel(16)` and callbacks use `try_send` only.
- The prequeue therefore retains at most 1.6 seconds and 614,400 bytes, staying
  inside the media contract's two-second ingress ceiling.
- Full queues drop the newest chunk, increment `dropped_callback_chunks`, and
  mark the next delivered chunk discontinuous.
- Invalid callbacks, callbacks after receiver teardown, and unexpected native
  stops have separate saturating diagnostics.
- A clean stop tail contains at most 16 chunks.
- Native stop has a five-second caller deadline. Each of the two callback-queue
  fences and the delegate proof has a separate one-second deadline, so no queue
  fence can turn `stop` or source `Drop` into an unbounded wait.
- Normalized startup calibration admits exactly five callbacks behind a
  750 ms deadline; it never waits for an unbounded first sample.

The production application must target macOS 13 or newer, provide
`NSScreenCaptureUsageDescription`, and embed the Swift runtime search paths
described by the existing desktop macOS bundle policy.

See [PROVENANCE.md](PROVENANCE.md) for the conceptual reference and dependency
record.
