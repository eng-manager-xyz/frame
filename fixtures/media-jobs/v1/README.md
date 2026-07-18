# Media service fixtures v1

`catalog.json` is the exact retained Cap media-job inventory at
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`. It decomposes Cap's composite
process/edit routes into independently fenced Frame jobs and records the numbered owner for
multipart and progress/callback surfaces that are not derivative executions.

`fixture-registry.json` is the closed input-fixture registry. It assigns every
retained job to exactly one CC0 fixture artifact and pins that artifact by
SHA-256. The segment set, edit timeline, external transcription input, and
caption document are concrete JSON artifacts rather than unresolved fixture
labels. The registry reuses the immutable synthetic MP4 by digest instead of
copying its bytes.

`parity-matrix.json` schema 2 contains exactly one sanitized parity case for
each of the 16 catalog jobs. Each row declares its concrete fixture, primary
executor and implementation, fallback executor and implementation, numeric
limit-profile authority, fallback disposition, evidence state, and typed
exception. The CI checker cross-links rows to the retained Cap catalog, fixture
registry, four bounded Cloudflare implementations, and 14-profile native graph
catalog. Entries whose real native/provider outputs require protected
infrastructure remain explicit gates; the matrix does not turn them into local
passes.

`synthetic-h264-aac.mp4` is a two-second procedural H.264 High/yuv420p + AAC-LC MP4. Its visual
and audio sources are FFmpeg's generated `testsrc2` and `sine` filters; it contains no captured,
third-party, personal, or production media. `synthetic-h264-aac.json` binds its CC0-1.0
provenance, generator command, SHA-256, byte length, and objective probe.

The local lane checks the immutable bytes and objective metadata. The Cloudflare Media binding
is remote-only and may consume a provider operation, so its output and the cross-executor golden
comparison remain an explicit protected gate until a named test account, private R2 namespace,
cost approval, and cleanup owner are supplied. A local pass must never rewrite that state as a
remote pass.
