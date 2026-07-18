# frame-macos-screen-capture

Safe, bounded full-display capture for Frame's macOS production path. The
crate uses the published `screencapturekit = 8.0.0` and
`core-graphics = 0.25.0` crates; it contains no direct FFI or unsafe block.

## Lifecycle

1. Construct one `MacOsScreenCaptureSource` per recording session with a
   validated `ScreenSourceInstanceId` and 32 fresh CSPRNG bytes.
2. Call `preflight_permission`. Only call `request_permission` in response to
   an explicit user action when the preflight result is `PromptRequired`.
3. Call `enumerate_displays`. The returned `ScreenTargetSnapshot` contains no
   display labels or raw macOS handles. Target tokens are HMAC-derived and
   valid only for the source session that produced them.
4. Create a `MacOsCaptureConfig` from a catalog binding, a CPU BGRA/sRGB
   `VideoFrameSpec`, and either a hidden or frame-embedded cursor mode.
5. Call `start`. The adapter resolves exactly one `SCRunningApplication` for
   the current process PID and excludes that whole application from the
   display filter. This also excludes Frame windows created after capture
   starts. A missing or ambiguous PID match fails closed; names, bundle
   identifiers, and raw window identifiers are never read or reported.
6. Drain `poll_frame` regularly. Recording finalizers call
   `stop_and_drain_frames` and ingest every returned frame before encoder EOS;
   its tail is bounded to three frames. `MacOsCaptureStopError` distinguishes
   an unconfirmed native stop, unconfirmed callback quiescence after native
   stop, a capture fault observed before confirmed teardown, and a
   tail-processing failure after confirmed teardown. An unconfirmed stop is
   sticky: polling, enumeration, start, and repeated stop cannot reinterpret
   it as reusable authority. A failed start is also sticky if its delegate
   teardown cannot be proved within the bounded deadline. Never-started and
   completely stopped sources remain idempotent. Compatibility `stop` maps a
   structured failure back to its original `MacOsCaptureError` for callers
   that deliberately do not need the tail.

The pinned ScreenCaptureKit crate implements shareable-content discovery,
stream start, and stream stop with synchronous wrappers whose internal native
completion wait has no deadline. This adapter transfers each of those calls,
including `SCStream` ownership for start and stop, to a named helper and waits
at most five seconds for the result. A process-wide lease permits only one
in-flight native wait (and therefore at most one stream stranded in such a
wait). If the deadline expires, the source becomes sticky/non-reusable, the
result receiver is dropped, and the helper handle is detached rather than
joined—even when the source itself is dropped. The lease remains held until a
late native return. A late successful start then fails to return its stream
through the dropped receiver, so the helper drops that stream. Once a native
call returns, its lease can be released while the helper finishes its bounded
result send/return; a subsequent short-lived helper can overlap only with that
final non-native window, not with another stuck native wait or owner.

`MacOsScreenCaptureSource` is `Send`: composition may construct it and move it
onto one serial worker thread. Control and polling methods require `&mut self`,
so callers cannot concurrently drive its lifecycle.

`MacOsCaptureFrame` owns tightly packed BGRA bytes. Its sequence, normalized
PTS, duration, and discontinuity bit can be passed to
`frame_media::BgraScreenFrame`; use a 1920x1080-or-smaller configuration for
the current `ScreenRecordingSpec` production graph.

The crate build script embeds the Xcode Swift runtime search path into this
package's test/example executables. A downstream application binary must also
embed `/usr/lib/swift` and the active Xcode toolchain's `usr/lib/swift/macosx`
as linker rpaths; Cargo does not propagate a library dependency's link args to
the final application target. The desktop composition task owns that final
binary setting.

## Callback and memory bounds

- ScreenCaptureKit's native queue depth is exactly three.
- The Rust callback queue is a `sync_channel(3)` and uses `try_send`; native
  callbacks never wait for the consumer or encoder.
- A full queue drops the incoming sample and increments
  `dropped_callback_frames`.
- Core Video locks, row-stride validation, and owned copies happen in
  `poll_frame`, outside the callback.
- The adapter accepts at most 7680x4320 and 256 MiB per owned BGRA frame.
- The latest Complete frame is retained so an Idle callback can emit one
  nominal-duration duplicate without touching an absent pixel buffer. At most
  one cached frame and one delivered frame are owned during ordinary polling;
  no unbounded idle-frame backlog exists.
- Every stream owns the serial queue passed to ScreenCaptureKit. After native
  stop returns successfully within its five-second helper deadline, teardown
  fences that queue but deliberately keeps the registered Rust handler
  installed while dropping `SCStream`. It waits at most one second for the
  custom delegate's drop signal. That signal is emitted only when the
  ref-counted stream context, both Swift bridge callback owners, and every
  in-flight callback are gone. A final queue fence plus a disconnected bounded
  channel independently proves the callback tail is complete before the
  adapter drains its at most three samples. This ordering retains a sample
  queued between the first fence and stream release instead of silently losing
  it through early handler removal. A missing handler ID fails the artifact
  after teardown; a still-connected sender leaves teardown unconfirmed.
- Delegate callbacks only set a per-stream atomic flag. The serial worker
  records `unexpected_native_stops` when it observes that flag, preventing a
  failed start's late callback from mutating a later session baseline.
- The returned tail can temporarily coexist with the cached frame: four owned
  BGRA frames total, about 506 MiB at the 7680x4320 dimension ceiling (and
  about 32 MiB in the current 1920x1080 production profile).
- Native sample durations outside 1 ms through 1 s fall back to the negotiated
  nominal duration and increment `duration_fallbacks`.
- Backward/duplicate PTS, Core Media epoch changes, native state gaps, and
  gaps over two seconds are surfaced through the discontinuity bit.

## Deliberate contract gap

This crate intentionally does **not** implement
`frame_media::ScreenCaptureSource`. That contract requires the adapter to
advertise and emit an exact protected-content signal. ScreenCaptureKit 8 only
reports ambiguous `Blank` and `Suspended` frame states; neither proves that
DRM/protected content caused the state.

`FRAME_MEDIA_CONTRACT_STATUS` makes the blocker compile-visible. Blank,
suspended, and Started samples are skipped, counted, and cause the next
delivered frame to carry a discontinuity. Idle samples repeat the last valid
Complete frame at the negotiated nominal duration, preserving static and
trailing time without interpreting Idle as new pixel content. None of these
states is mislabeled as protected content.

See [PROVENANCE.md](PROVENANCE.md) for the mechanical-port reference and
license record.
