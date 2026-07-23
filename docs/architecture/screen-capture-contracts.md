# Screen-capture contract and recovery boundary

Frame's screen-capture core is provider-free. `frame-media` defines what a
native adapter must prove before capture starts, but it does not treat a type
mapping as evidence that ScreenCaptureKit, Windows Graphics Capture, PipeWire,
or X11 works on physical hardware. Platform FFI and permission UI remain in
separate native adapters behind `ScreenCaptureSource`.

The current Windows slice follows that boundary with an unsafe-free normalized
adapter over a separately audited, target-gated Win32 crate. It implements
bounded CPU BGRA display/window/region capture but intentionally remains out of
the desktop release composition until protected-content semantics, lifecycle
recovery, and physical Windows evidence can satisfy this contract.

```text
native adapter
  ├─ stamped permission preflight / source-bound target snapshot
  ├─ exact frame-profile capability descriptor
  └─ stamped poll events (frame, cursor, target, permission, sleep, protection)
          │
          ▼
catalog membership + exact negotiation ──> session one-shot operation action
          │                                  │
          ▼                                  └─ live revalidation + private ticket
mandatory session ingress (stream/epoch/cursor policy)
          │
          ├─ bounded frame lease queue
          └─ bounded cursor-image cache
          │
          ▼
non-blocking appsrc plan ──> supervised GStreamer graph
```

## Target identity and geometry

`ScreenTargetId` is a 128-bit host-local opaque identity with the target kind
included in equality. `ScreenSourceInstanceId` changes whenever a native
adapter instance restarts. A `ScreenTargetBinding` combines both identities
with the current topology generation and a nonzero target epoch. Selected and
excluded targets therefore cannot be replayed across a source restart or a
new catalog generation. A display ID cannot be reinterpreted as a window or
region ID. Debug output always redacts opaque bytes and adapter generations;
the contract has no window title, application name, process ID, device label,
or raw platform handle field.
Target-descriptor debug output also redacts geometry; operational diagnostics
never expose desktop layout or window position.

Logical rectangles use signed origins and half-open edges so displays left or
above the desktop origin are representable. `DisplayGeometryTransform`
validates:

- a reduced rational DPI scale between 1/16x and 16x;
- exact physical display dimensions after 0°, 90°, 180°, or 270° rotation;
- checked logical and physical coordinate arithmetic;
- target containment for display-local regions; and
- deterministic floor/ceil pixel coverage for fractional-scale rectangle
  edges.

The transform is used for validation and normalized metadata, not as proof of
a platform crop implementation. Native region selection and its no-overcapture
recording remain a protected per-platform gate.

Window geometry is retained in global logical coordinates because a window
may span displays with different scale factors. Its negotiated output frame is
the authoritative physical canvas. Display and region targets retain their
display transform, including rotation. A target epoch advances whenever its
geometry or native backing changes. Topology events carry source instance,
generation, and sequence stamps; cross-source, older-generation, and replayed
events are rejected without partially updating session state.

Target snapshots are capped at 256 label-free descriptors and reject duplicate
identities or bindings from another source/generation. Region targets carry a
binding to the containing display; the embedded transform must exactly equal
that display descriptor's canonical transform, and the region must be inside
its logical bounds. Negotiation checks the selected descriptor and every
exclusion against the complete current snapshot. A descriptor with a real
binding but forged geometry is rejected. `ScreenCaptureSource::start` and
`reconfigure` accept only a non-cloneable, private-field operation ticket made
by the library-owned action executor after it re-enumerates and exactly
revalidates the complete live capability and catalog snapshots. The ticket
contains the full negotiated contract and exact source, target, catalog,
session-owner, operation, stream, predecessor-stream, and capture-epoch
binding. Before permission control, enumeration, start, reconfigure, poll, or
stop reaches a native method, the caller first wraps the raw adapter in
`BoundScreenCaptureSource` using its CSPRNG session identity. The wrapper
presents a private, non-cloneable `ScreenSourceSessionTicket` exactly once and
retains the resulting opaque `ScreenSourceSessionBinding`. Capability discovery
and target enumeration therefore happen after binding and before negotiation;
`ScreenCaptureSession::new` accepts that binding rather than a raw session ID.
Every non-operation trait method requires a private `ScreenSourceCallTicket`
minted only by the wrapper, while start/reconfigure/stop require their private
operation ticket. The only ungated source methods expose pure ownership
metadata and may not call a platform API. A raw or wrong-bound adapter cannot
dispatch permission, enumeration, polling, or capture work. Multiple bound
source objects may share one multiplexing backend, but each remains bound to
its own session. Successful preflight/request results are immediately sealed
in private owner envelopes. `poll_owned_event` seals every native event or
failure in a public opaque, private-field owner envelope before returning it.
Raw `ScreenSourceEvent` values are adapter output only and no public ingress
method accepts one. `ScreenSessionEvent` is private, and the former generic
public `apply_session_event` method no longer exists, so a caller cannot inject
a granted preflight, permission/topology event, sleep/wake, or
protected-content transition directly.
`stop` uses the same exact operation/stream ticket but deliberately bypasses
enumeration after checking the exact bound owner: permission revocation,
portal loss, cancellation, or a deadline must not make mandatory teardown
depend on target access. Stop is scoped to the opaque session binding. The
ticket stream records the operation being invalidated and its predecessor
records the acknowledged stream that may still be live. An adapter must
quiesce all and only native resources for that binding and must not disturb
another session sharing the backend. This covers both an unexecuted
reconfigure (the predecessor is live) and a dispatched reconfigure whose
acknowledgement has not reached the session (the new stream may be live).

## Ownership-boundary audit

The following table is the exhaustive ownership inventory for public or
publicly reachable native-dispatch and mutable session/ingress surfaces. Pure
validated value constructors and read-only diagnostics are omitted.

| Boundary | Reachable effect | Ownership proof checked before the effect |
| --- | --- | --- |
| `BoundScreenCaptureSource::new` | claims an adapter | constructs one opaque binding from the adapter's pure source identity and caller's CSPRNG session ID; the adapter must accept and report that exact binding before the wrapper is returned |
| bound `capabilities` / `enumerate_targets` | capability/catalog adapter call during bootstrap or live validation | wrapper mints a private call ticket for its retained binding; a raw adapter cannot mint the ticket |
| bound `poll_owned_event` | native poll result leaves the adapter wrapper | wrapper mints an opaque private-field event or failure envelope carrying its retained binding; callers cannot relabel a raw event |
| bound adapter inspection | shared adapter observation only | `adapter`/`Deref` return `&S`; there is no `adapter_mut` or `DerefMut`, so safe code cannot replace the adapter beneath a retained binding; swapping whole wrappers moves adapter and binding together |
| `ScreenSessionAction::execute_source` | enumerate and start/reconfigure/stop | action owner must equal the session binding and bound-source binding before the one-shot request is consumed or any adapter call; request and pending operation must then match exactly |
| `ScreenCaptureIngress::new` | creates queue/cache owner | copies the session's exact opaque binding, source, target, and epoch into the ingress |
| `apply_intent` / `cancel_session` | applies request-permission/start/stop/cancel user intent | only the narrow freely constructible `ScreenSessionIntent` enum is accepted; common ingress/session binding and epoch checks precede mutation |
| `execute_control_action` | preflight or permission prompt, then session mutation | action, ingress, session, and bound source must carry the same opaque binding before native dispatch; the returned private permission-result envelope is checked again before session mutation |
| `complete_operation` / `apply_operation_failure` | applies a library operation result | acknowledgement/failure is minted with the action's opaque owner; ingress checks that owner before session or queue/cache mutation |
| `apply_source_event` | queue/cache/session mutation from a native event | accepts only an opaque envelope minted by `poll_owned_event`; exact envelope/ingress/session owner checks precede cancellation and event handling, then stream/control/topology semantics are validated |
| `poll_source` | native poll and normalized event handling | common ingress/session/source check runs before cancellation and native poll; event/failure envelopes are owner-checked before the private handler performs mutation |
| `apply_epoch_transition` | drains queue/cache and changes epoch/target | transition carries the opaque session binding; exact owner, source, epoch, and target checks all precede the first queue/cache mutation |
| `try_pop` | cancellation or frame removal | exact ingress/session binding and epoch check precedes cancellation, queue clock observation, and removal |
| `ScreenRecordingPump::new` | claims frame removal and the appsrc graph for one segment | requires a pristine running graph (not merely zero submitted frames), then retains an exclusive mutable borrow of the exact ingress until finish/abort; safe code cannot create a second pump or call competing pop/event mutators while that claim exists |
| pump graceful-stop/retry/completion methods | retires native capture and may publish one artifact | the pump alone invokes the borrowed ingress; the request retains the exact post-request seal epoch and ingress-transition revision, retries preserve/rebind their one-shot Stop action, rejected pre-mutation correlation returns that exact proof, and finish consumes an opaque exact-Stop completion by value |
| frame queue / cursor cache | push, pop, activate, reset, drain | types and mutators are private; they are reachable only through ingress paths after the checks above |
| source session/call/operation tickets | adapter claim or native dispatch | fields and constructors are private; only the bound wrapper/action executor can mint them, and every ticket carries or borrows the exact binding |
| internal session events / session actions / epoch transitions | state-machine input or deferred native/ingress work | internal event enum is private and created only by the typed paths above; action/transition fields are private and minted by the owning session; foreign-session replay fails before side effects |

## Cursor and exclusion contract

Cursor behavior is exact, never best-effort:

| Requested mode | Output |
| --- | --- |
| hidden | neither cursor pixels nor metadata |
| embedded | cursor pixels are supplied by the native frame; no duplicate metadata |
| metadata | target-local frame coordinates with separately negotiated image-revision and click fields |

A metadata observation outside the selected target becomes a fully hidden
sample. Position, image revision, and click state are all cleared so desktop
activity outside a region cannot leak. Raw and normalized cursor debug output
redacts coordinates, image revisions, and clicks.

Cursor coordinate spaces are also negotiated. Display/region adapters may
provide global logical coordinates when the validated display transform is
available, or native frame-local physical coordinates. Window adapters must
provide frame-local coordinates: proportional conversion from desktop logical
coordinates would be incorrect for a window spanning mixed-DPI displays and is
rejected.

The production macOS adapter supplies global logical coordinates for display
and region targets and converts an enumerated window's point-space bounds to
frame-local coordinates. The production Windows adapter treats WGC/DWM and
`GetCursorInfo` values as one physical desktop coordinate space and converts
display, region, and window selections to frame-local coordinates before
normalization. Both adapters keep the native cursor hidden in metadata mode,
emit a changed bounded image before a referencing frame, suppress all fields
outside the selected target, and hide cursor metadata on a retained stop tail
when a new image update can no longer be admitted.

Cursor image changes use a separately owned, release-on-drop update with a
nonzero revision, BGRA/RGBA format, bounded dimensions/bytes, and a validated
hotspot. Frame and cursor-image payload types are sealed to exact-capacity
vectors and boxed slices. Their constructors authenticate the declared bytes
against the complete allocation, and the queue/cache repeats that check when
the lease changes authority; spare capacity and caller-defined sidecars are
therefore not hidden from retained-memory bounds. A sealed CPU allocation
cannot claim a CoreVideo, D3D11, or DMA-BUF frame type. Frame cursor metadata
references that revision without copying image pixels into every frame. The
mandatory ingress owns the negotiated
`CursorPolicy` and the one-entry cache together; adapters cannot write either
the cache or frame queue directly. Hidden and embedded modes reject cursor
metadata and cursor-image events. Metadata mode rejects unnegotiated revision
or click fields, and revision-disabled mode rejects cursor images. The cache is
capture-, target-, and stream-scoped. Updates must be strictly
revision-monotonic; a visible cursor cannot omit a negotiated revision or
reference a missing, future, or stale image. Replacement and epoch reset
release the prior owned CPU allocation and report the exact image/byte drain.

Window exclusion is an explicit bounded list of source/catalog-bound window
bindings.
Negotiation fails if the source does not advertise exclusion or its supported
limit is lower; Frame never silently drops an excluded window. This is a
contractual precondition only. The promise that no Frame window appears in a
real recording remains blocked until per-platform recorded evidence exists.

## Capability negotiation and appsrc ingress

Every source publishes contract version, platform and source-instance binding,
topology/control epochs, target classes, cursor modes and metadata features,
permission preflight, topology/recovery, protected-content signaling, window
exclusion, bounded-appsrc support, and explicit supported frame-profile
tuples. Each tuple couples pixel format, color space, memory type, maximum
dimensions, and maximum rate; negotiation never invents a cross-product of
independently advertised values. Native-memory capability descriptions remain
platform checked: CoreVideo is macOS-only, D3D11 is Windows-only, and DMA-BUF
is Linux-only. The provider-neutral ingress intentionally negotiates only
`FrameMemory::Cpu` in this slice because its sealed payload contract can prove
the complete retained CPU allocation. CoreVideo, D3D11, and DMA-BUF zero-copy
payload ownership/accounting remain an explicit implementation gap.

Negotiation rejects unsupported target, cursor, image/click metadata,
exclusion, recovery, protection, frame format, color space, memory, size, or
rate. It also rejects a source bound to another host OS. The resulting
`ScreenAppSrcPlan` requires the audited `AppSourceBridge` runtime capability,
uses explicit platform PTS/duration (`do_timestamp=false`), never asks appsrc
to block the driver (`block=false`), and retains an exact owned CPU allocation
until downstream releases the corresponding buffer. An adapter must therefore
perform any required native-to-CPU copy before ingress; this contract does not
claim native-memory zero-copy.
The negotiated plan retains the complete capability and target snapshots.
`ScreenRecordingPump` validates every plan flag against the real
`ScreenRecording` graph and exclusively borrows one exact capture ingress for
the pump lifetime, binding the graph to that session, ingress epoch, and active
stream. Safe code cannot create a second pump or pop the queue through another
owner while that borrow is live. It declares BGRA plus sRGB caps, drains no
more than the smaller of the upstream and appsrc frame bounds per call, expires
and peeks the next frame before removal, checks live appsrc frame/actual-byte/
actual-duration capacity, and moves each owned CPU payload into a GStreamer
buffer without another full-frame copy after adapter conversion. It preserves
source timing, cumulative submission accounting, and marks sequence-loss
boundaries as discontinuities. Only a drained ingress can mint the opaque
graceful-Stop request. The request records the exact post-request seal epoch
and transition revision. Repeated exact Stop failures rebind either its
publication proof or an opaque abort-only proof without discarding the new
action. A mismatched
acknowledgement/failure, exhausted ingress-transition revision, or exhausted
retry operation id is rejected before mutation and returns the same one-shot
proof so the exact result can still be applied. Any later epoch handoff,
suspension, permission, target, or protected-content transition permanently
invalidates publication. A matching old acknowledgement may still settle
native teardown but produces only an abort completion. Artifact finish
consumes the unchanged-lineage exact-Stop completion, so it cannot be
replayed.
Cancellation, suspension, fault transitions, and any terminal appsrc failure
atomically retire ingress/session leases, preserve the exact native Stop
transition and teardown status, and confirm Null without publishing a partial
artifact. A frame that would exceed current appsrc time capacity remains in the
upstream queue; it is never popped merely to terminalize the graph.

After a successful native Stop, a source may expose a finite callback tail
through `poll_stopped_event`. `ScreenRecordingPump` accepts no more than 16
tail frames, verifies that each still belongs to the stopped stream, applies
the normal ingress allocation/timing checks, submits them directly to the
owned graph, and requires an empty tail before accepting the exact Stop
acknowledgement and encoder EOS. Sources without a tail use the empty default.
An excessive, foreign, malformed, or failing tail aborts the segment; it can
never be silently discarded or published as a complete artifact.

Executing a start/reconfigure action first verifies the action owner, session,
bound source, and one pending operation. It then re-enumerates the complete
catalog, reads the resulting live capabilities, and compares both exactly
before constructing an operation ticket or dispatching start/reconfiguration.
An unexecuted start superseded by a stop therefore cannot mint a delayed
ticket. A portal restart, capability change, unrelated catalog change,
forged/cross-source binding, cross-kind dispatch, or replayed action fails
before adapter start or reconfiguration.
Enumeration and native-operation errors are rebound to that exact pending
operation and stream. Feeding the bound failure through ingress produces its
defined stop/flush transition; retryable stop failures reissue only a new exact
stop operation, even for a terminal cancelled session, without reopening
capture or draining twice.

Every frame and cursor image carries an exact `ScreenStreamStamp`: the source
instance, full target binding, collision-resistant session ID, stream
sequence, and capture epoch. Production callers must create each 128-bit
session ID with a cryptographically secure random-number generator; the
provider-free media crate intentionally does not own an OS RNG. The ingress
accepts data only for its one acknowledged active stream and caps frame count,
retained native bytes, and age. A
producer selects drop-newest or drop-oldest; blocking is not available. The
queue rejects sequence replay, backward timestamps without an explicit
discontinuity, output-spec changes without renegotiation, and a backward
monotonic queue clock. Cancellation returns an exact frame/byte drain instead
of discarding that information. Each authentic epoch transition carries the
same opaque session/source binding and atomically drains both frame queue and
cursor cache, releases all leases, clears the active stream, and resets clock,
sequence, and timestamp history. Replayed, foreign-owner, or nonmonotonic
transitions fail before either owner is mutated, so a new source epoch may
restart frame sequence at one. Each native adapter must separately prove that
its internal callback-to-poll handoff is also bounded.

Topology events carry a complete capability/catalog snapshot. For an event
whose named target is unrelated, the session still compares the old and new
selected target and every promised exclusion, including geometry, target
epoch, and capture-relevant capabilities. Only a canonical generation/control
sequence rebind may leave an acknowledged active stream running. A disguised
selected/exclusion epoch or semantic change flushes and reconfigures. Any
pending start/reconfigure is superseded by stop and later reissued against the
new snapshot, so an old unexecuted action cannot be stranded. If an authentic
snapshot removes a promised exclusion or can no longer satisfy the negotiated
target, cursor, frame-profile, exclusion, recovery, protection, or bounded
ingress contract, the session does not return a validation error while leaving
the old stream active. It enters terminal `ContractInvalidated`, advances the
capture epoch, atomically drains ingress, rejects late old-stream data, and
issues mandatory Stop.

## Permission and recovery state machine

Permission preflight distinguishes prompt-required, granted, denied,
restricted, and revoked. Settings guidance is an enum, not a platform URL or
native error string. A prompt occurs only after an explicit request action.
Denial is non-destructive and never starts a source.

Permission, sleep/wake, and protected-content observations carry a source
instance, control epoch, and strictly increasing sequence. Stale grant, wake,
or protection-clear events are rejected transactionally. Wake, permission
revocation, and `AccessRevoked` target loss latch a fresh-preflight requirement;
an asynchronous grant cannot clear it. Only a newer stamped granted preflight
can release that latch.
Library-owned control execution leaves a preflight or permission-request action
retryable when the adapter call fails, without partially mutating the session.
Every control-bearing action carries its session's opaque source binding;
control execution compares that action owner with the ingress, session, and
bound source before preflight or prompt dispatch, then checks the private
owner-stamped native result before session mutation. A session-B action
presented to session A is rejected without adapter, session, queue, or cache
effects. Preflight completion cannot be freely constructed or applied; the
out-of-module harness reaches it only by executing the bound native control
action.
The normalized poll entry point binds raw poll failures to the exact live
operation; a raw cancellation takes the same terminal cancellation path as
ingress cancellation. Poll and queue-pop entry points first apply one common
ingress/session/source-owner check. Mixing ingress A with session or bound
source B fails before cancellation, native polling, queue clock observation,
or frame removal. Direct normalized-event ingestion accepts only an opaque
owner envelope minted by the bound poll path. Permission, topology, and sleep
envelopes minted by source B are rejected by ingress A before cancellation or
any session, queue, or cache mutation, even when both adapters share the same
source-instance identity.

The deterministic session state machine covers:

- preflight, prompt, ready, start, capture, reconfiguration, stop, and terminal
  phases;
- permission revocation with an immediate stop/flush action and explicit
  restart only after a fresh granted preflight;
- independent permission, target-availability, sleep, and protected-content
  blockers, so clearing any one cannot bypass another;
- unrelated hotplug events without disturbing the selected target;
- fail-closed target loss or identity-bound, attempt-limited recovery of the
  same target;
- target/capture epoch advancement before reconfiguration or restart;
- sleep/wake with a fresh permission preflight before restart;
- protected content latched even while another nonterminal blocker is active,
  with either suspend-until-clear or terminal failure; and
- idempotent cancellation plus bounded, low-cardinality diagnostics.

Cancellation is a session transition, not a queue utility. The required
ingress API moves the session to `Cancelled`, advances the epoch, atomically
drains queue and cursor cache once, and returns the one exact native stop
action. Repeating cancellation is a no-op. Any exact native stop failure,
including a nonretryable cancellation or deadline race, reissues only Stop and
cannot change the terminal phase or create a second drain. The exact matching
Stop acknowledgement is accepted in terminal state, clears the pending
operation without reopening capture, and makes delayed failure replay stale.

Start, reconfigure, and stop use one explicit pending-operation slot. Each
command has an operation ID and exact stream stamp; only the acknowledgement
returned by executing that one-shot action can complete it. A benign newer
grant preserves `Starting` or `Reconfiguring` without duplicating a command.
Target, permission, sleep, or protection invalidation replaces an unsafe
pending start/reconfigure with one stop. A target reconfiguration observed
while a stop is pending preserves that exact stop and cannot reopen capture.
Frames and cursor images remain blocked until a matching start/reconfigure
acknowledgement activates their exact stream. Bound source failures use the
same operation and stream identity, so delayed and cross-session failures are
rejected even when local numeric counters happen to match.

Platform calls receive a cooperative cancellation/deadline budget capped at
30 seconds. Stop derives its own non-cancelled five-second teardown budget, so
the session-lifetime cancellation token that triggered teardown cannot cancel
the teardown itself. Exact Stop failures remain recoverable through a fresh
bounded Stop operation. These budgets bound compliant adapters but cannot
preempt a native API that ignores the contract. Hard process isolation or a
platform watchdog is still required before claiming a wall-clock kill
guarantee.

Every permission revocation, target loss, sleep, first protected-content
detection, and target reconfiguration advances the capture epoch. The emitted
`ScreenSessionAction` independently carries a source command, permission
command, and mandatory epoch flush. `ScreenCaptureIngress` applies that flush
to both bounded owners before returning the transition; the appsrc owner must
also flush downstream GStreamer state before accepting the new epoch. Old
queued or late native frames then fail exact stream validation.

Diagnostics include only public schema/source/target-kind enums, lifecycle and
permission states, the local capture epoch, saturating internal event counters,
and low-cardinality event or failure codes. Adapter target epochs, topology and
control sequences, target IDs, native errors, titles, application identity,
cursor activity, frame payloads, and desktop coordinates are excluded or
redacted.

## Protected completion gates

This architecture does not close issue 24. The following evidence must be
recorded on approved physical or clean virtual hosts before enabling an
adapter by default:

- signed/bundled adapter implementation and clean-machine capture on supported
  macOS, Windows, and Linux versions/architectures;
- real permission preflight, prompt, denial, settings, revocation, and
  non-destructive recovery;
- display, window, and region samples across negative-origin multi-monitor,
  mixed DPI, fractional scale, and every supported rotation;
- cursor hidden/embedded/metadata image changes, positions, clicks, and
  selection clipping;
- recorded proof that promised Frame-window exclusion never captures Frame UI;
- display unplug, window close/minimize, hotplug/reconfiguration, sleep/wake,
  and protected-content behavior;
- per-memory-path copy count plus latency/CPU/GPU/memory comparison against the
  approved Cap baseline and issue 04 fixtures; and
- end-to-end GStreamer appsrc buffer-lifetime and timestamp evidence under
  backpressure and cancellation.

Until those gates exist, rollout remains adapter-by-adapter and target-mode by
target-mode behind the legacy source. Rollback selects the existing source;
the normalized contracts and stored media format do not require migration.
