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
   its tail is bounded to three frames. `stop` remains idempotent for callers
   that deliberately do not need the tail.

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
- `stop_and_drain_frames` first performs ScreenCaptureKit's blocking clean stop
  and then drains at most the three samples already in the callback queue. Its
  returned tail can temporarily coexist with the cached frame: four owned BGRA
  frames total, about 506 MiB at the 7680x4320 dimension ceiling (and about
  32 MiB in the current 1920x1080 production profile).
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
