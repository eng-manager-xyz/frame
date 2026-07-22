# frame-windows-screen-capture

Safe, target-gated Windows Graphics Capture adapter primitives for Frame.

The crate keeps WinRT/Direct3D handles and display/window identifiers behind
an opaque catalog. Captured CPU BGRA frames cross a bounded nonblocking channel
and implement Frame's provider-neutral `ScreenCaptureSource` contract before
they can reach GStreamer's `appsrc` pump.

Repository-local tests prove value, ownership, timing, and queue invariants.
They are not physical Windows capture evidence. Run the protected Windows
hardware workflow before claiming release or parity support.
