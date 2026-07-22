# Windows capture dependency provenance

Frame's Windows adapter is an original integration over published safe Rust
APIs. No Cap source text is copied into this crate.

- Reference reviewed for behavior only: `CapSoftware/Cap` at
  `6ba69561ac86b8efdb17616d6727f9638015546b`, including the MIT-licensed
  `crates/scap-direct3d` and `crates/scap-targets` paths recorded in
  `docs/upstream-cap.md`.
- `wgc` `1.0.6`, MIT OR Apache-2.0, wraps Windows Graphics Capture and
  performs an owned D2D Map/copy/Unmap for CPU pixels.
- `frame-windows-capture-ffi` is Frame's separately audited narrow Win32
  boundary for label-free monitor/window enumeration, geometry, scale,
  rotation, capture-item construction, and bounded worker stop.

The wrapper remains CPU/BGRA-only. It does not claim D3D11 zero-copy, cursor
metadata, physical-device parity, or lifecycle evidence.
