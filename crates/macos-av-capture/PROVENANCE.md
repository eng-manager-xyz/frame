# Provenance

## Production implementation

- `screencapturekit = 8.0.0` supplies the safe Rust API used for content
  discovery, stream configuration, audio-buffer access, callbacks, and stream
  lifecycle.
- `apple-cf = 0.9.3` supplies the owned serial dispatch queue and Core Media
  format/timing access used through the published safe wrappers.
- `apple-metal = 0.6.0` constrains `screencapturekit`'s broad compatibility
  range to its newest pre-sampler release. Later releases compile the macOS
  26-only `MTLSamplerDescriptor.lodBias` property on SDKs where it is absent;
  this adapter does not use that sampler surface.
- `core-graphics = 0.25.0` supplies Screen Recording permission preflight and
  request APIs.
- `gstreamer = 0.25.3` and `gstreamer-app = 0.25.2` supply the safe device
  monitor, retained device elements, bounded appsinks, timestamps, and owned
  sample mapping used by the microphone/camera bridge. Only factories declared
  in `gstreamer-runtime.json` and loaded from the build-time trusted plugin root
  are accepted.
- The native-call deadline and delegate/context teardown proof follow the
  bounded ownership pattern established by `frame-macos-screen-capture` in
  Issue 24. No source code is shared through a private or unstable API.
- `frame-platform-lifecycle` supplies the safe process-lifetime sleep/wake
  cursor used by the normalized bridge. It exposes no Apple observer object or
  callback identity.

The crate contains `#![forbid(unsafe_code)]`; all Apple FFI remains inside the
published dependencies.

## Conceptual reference

The system-audio configuration and macOS permission model were compared with:

- repository: <https://github.com/eng-manager-xyz/Screen>
- revision: `0582fc9bcd81ac49f27b45f38eb703fb909b0fe3`
- file: `crates/media/src/sck_audio.rs`

That implementation was used only to confirm the product-level concept:
ScreenCaptureKit system audio, 48 kHz stereo, current-process exclusion, and
the Screen Recording TCC category. Its Objective-C FFI, unsafe blocks,
unbounded channel behavior, labels/process metadata, and timeout-only teardown
were not copied.

The microphone/camera source selection and TCC declarations were compared
with:

- repository: <https://github.com/eng-manager-xyz/Screen>
- revision: `0582fc9bcd81ac49f27b45f38eb703fb909b0fe3`
- files: `crates/media/src/gstreamer_audio.rs`,
  `crates/media/src/gstreamer_video.rs`, `crates/media/src/microphone.rs`,
  `crates/media/src/camera.rs`, and `crates/app/Info.plist`
- repository: <https://github.com/CapSoftware/Cap>
- revision: `6ba69561ac86b8efdb17616d6727f9638015546b`
- files: `apps/desktop/src-tauri/src/permissions.rs`,
  `crates/recording/src/sources/microphone.rs`, and
  `crates/recording/src/sources/camera.rs`

Those sources established the platform choices (`osxaudiosrc`,
`avfvideosrc`, AV media privacy classes, and separate recording inputs). Frame
uses its own GStreamer Rust graph, HMAC identity, bounds, lifecycle, and
provider-neutral bridge; no Screen or AGPL Cap implementation was copied.

## Deliberate integration boundary

This crate implements `frame_media::NativeAvBridge` once for the selected
ScreenCaptureKit system-audio source and once for selected macOS
microphone/camera devices. Both bridges own calibration, appsrc leases,
permission revisions, sleep/wake transitions, pause/resume epochs, and stable
terminal reconciliation. The device bridge additionally owns GStreamer
hotplug/default events and fair two-source polling. Its portable fake-source
suite drives microphone and camera bytes through the real Frame appsrc graph.

This is not a claim that the desktop release records microphone/camera tracks
or that the normalized runtime has a lossless recording tail. The desktop's
current screen/system-audio WebM path remains direct until the Studio journal
and mux owner can authenticate and consume every terminal callback.
