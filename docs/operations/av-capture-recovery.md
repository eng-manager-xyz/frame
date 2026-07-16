# Audio and camera capture recovery runbook

This runbook applies after the native adapter reports a privacy-safe stable
code. Never add device labels, serials, raw handles, exact clock samples, audio,
or video to logs or support artifacts.

## First response

1. Record the application build, OS family/version, enabled source classes,
   privacy-safe route class, stable code, timing bucket, and whether screen-only
   capture continued.
2. Confirm the session is `Recording`, `Suspended`, `TeardownRequired`, or a
   terminal state. Do not infer state from whether a UI preview is visible.
3. If an optional source was disabled, preserve the screen recording. Do not
   restart the whole recording solely to recover mic, system audio, or camera.
4. If teardown is required, retry the existing session-scoped stop. Never start
   another native operation on that session until teardown is acknowledged.
5. Treat a stale/replayed control-event stamp or malformed persisted settings
   as an adapter/storage incident. Do not bypass revision or codec validation.

## Permission denial or revocation

- Surface the OS-specific permission instructions outside the media process.
- Keep the source disabled and drain its queue.
- Continue screen-only capture when the screen source remains healthy.
- After permission is granted, enumerate a fresh complete catalog and require
  an explicit reconfiguration. Do not reuse the old generation or stream
  epoch.
- System-audio permission is independent from microphone permission.

## Unplug, replug, default switch, and Bluetooth profile change

- Treat an unplug or effective-profile recreation as a generation change.
- Reject delayed buffers from the old generation.
- A pinned selection never falls back to another device.
- A followed default switches only when future changes were explicitly
  authorized. Otherwise prompt for confirmation of the new opaque candidate.
- Bluetooth telephony/wideband transitions are capability changes. Enumerate
  exact formats again; never push the new format into an old caps/queue path.
- Replugging the same stable ID does not resume automatically. Reconfigure and
  advance the stream epoch.
- Persisted settings are a bounded versioned DTO containing only opaque IDs and
  exact formats. Reject unknown versions, malformed IDs, and extra fields;
  never recover by matching a display label or current default.

## Overload and backpressure

- Confirm appsrc remains nonblocking and the configured drop policy is active.
- Confirm retained-byte accounting uses the immutable constructor snapshot.
  A native lease size callback must never be consulted again by queue or
  appsrc code.
- Confirm the native body moved once into the local appsrc adapter and stayed
  retained until downstream consume/drop. Either path must release once.
- Record only `IngressOverload`, source class, route class, and coarse timing
  bucket. Do not record queue contents or exact device timing.
- If overload persists, disable camera preview first, then lower an explicitly
  supported camera format through a full reconfiguration. Do not invent caps.
- If the native callback blocks or retained bytes exceed the negotiated bound,
  disable the adapter and fall back to screen-only capture.

## Clock discontinuity, pause/resume, and sleep/wake

- A timestamp rollback or source/master step beyond the policy threshold must
  enter the discontinuity path. Do not let the drift estimator smooth it.
- Reject sequence gaps/replays, uncorrected native buffers, and buffers for an
  uncalibrated or retired stream epoch. Calibration is keyed to the exact new
  source stamp and sequence restarts only after that calibration succeeds.
- A latency-confidence change requires a declared discontinuity. An ordinary
  frame beyond the 50 ms long-run budget returns
  `SynchronizationBudgetExceeded`; do not raise the budget or silently clamp.
- Pause drains queued optional media. Resume advances every active source epoch,
  rebases at the prior declared output end, and marks the first frame
  discontinuous.
- Sleep invalidates unexecuted work and moves an established stream to
  `Suspended`. A dispatched start/reconfigure/resume with an unapplied result
  moves to `TeardownRequired`; quiesce it before restart. After wake, enumerate
  fresh capabilities/catalog and obtain fresh latency/calibration samples
  before resuming.
- Escalate when startup offset exceeds 80 ms or the measured hour-long offset
  exceeds 50 ms. Preserve only privacy-safe derived plots, never raw media or
  hardware identifiers.

## Stop and cancellation

- Stop/cancel does not depend on permission or enumeration.
- The teardown request includes active streams, retired-but-unconfirmed streams,
  and a pending operation's possibly-live streams.
- A timeout or backend failure means `TeardownRequired`, not stopped. Retry the
  same session-scoped teardown with its stable terminal ID and bounded native
  timeout (never zero and never over 30 seconds).
- If the local caller loses a successful acknowledgement, use the explicit
  teardown retry. The adapter must first reconcile the exact terminal
  postcondition. If it was already applied, return the same terminal result
  without a second native release; do not execute teardown again.
- A delayed acknowledgement from a superseded attempt is stale even when it
  reports success.
- Do not delete temporary output or release session ownership until the adapter
  confirms all listed native resources are quiescent.
- Repeated teardown failure is an adapter incident; prevent new capture in that
  adapter process and preserve the privacy-safe stable code sequence.

## Rollout and rollback

Enable source combinations independently: microphone, system audio, camera
recording, then camera preview. A failing optional-source cohort rolls back by
disabling that source capability and retaining the proven screen path. Rollback
must not weaken exact caps, permission, ownership, queue, timestamp, or
teardown checks.

Physical-device promotion requires every protected item in the evidence record
to be collected for the target OS/route matrix. Local deterministic tests alone
are not a release signal.
