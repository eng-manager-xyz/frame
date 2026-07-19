# Provenance

## Production implementation

- `screencapturekit = 8.0.0` supplies the safe Rust API used for content
  discovery, stream configuration, audio-buffer access, callbacks, and stream
  lifecycle.
- `apple-cf = 0.9.3` supplies the owned serial dispatch queue and Core Media
  format/timing access used through the published safe wrappers.
- `core-graphics = 0.25.0` supplies Screen Recording permission preflight and
  request APIs.
- The native-call deadline and delegate/context teardown proof follow the
  bounded ownership pattern established by `frame-macos-screen-capture` in
  Issue 24. No source code is shared through a private or unstable API.

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

## Deliberate integration gap

This slice does not implement `frame_media::NativeAvBridge`. The contract is
being extended with startup calibration and also requires hotplug,
default-change, and sleep/wake behavior. Claiming those capabilities before a
session-owned calibration/event implementation exists would be false. The
narrow source exposes raw source PTS, duration, discontinuity, stable device
identity, and exact format so that integration can be completed explicitly in
the next media-runtime slice.
