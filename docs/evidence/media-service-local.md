# Media service v1 local evidence

This is a bounded local evidence record for issue 28. It covers only the
provider-neutral catalog/router/journal contract, production D1/R2 adapter code,
native and wasm compilation of the scheduled Cloudflare binding consumer,
offline executor, procedurally generated synthetic fixtures, and four audited
native GStreamer/Rust graphs. The remaining native rows have machine-checked
graph recipes and typed exceptions. It does not claim a Cloudflare account invocation,
protected H.264/AAC or composition/remux graphs, external AI/transcription
provider, production latency/cost, platform capacity, codec/license approval,
data-residency approval, or human output review.

On 2026-07-16, the following passed:

- 13 focused Rust unit tests covering the exact 16-job catalog, all managed
  modes at and just beyond every limit, supported/unsupported formats,
  decompression bombs, deterministic 10,000-case adversarial preflight,
  routing/fallback policy, input-role and tenant/key isolation, redacted debug,
  deterministic artifact identity, leases/fences, monotonic progress, cost and
  output verification, immutable publish/reuse, cancellation suppression,
  ambiguous lost-ack recovery, and exact fenced cleanup;
- 4 integration contract tests proving the pinned Cap inventory matches the
  executable catalog, non-transform surfaces have explicit owners, the
  2026-06-10 Cloudflare binding contract and 2026-04-21 product limits are one
  revisioned object, and the licensed
  synthetic MP4 fixture is immutable and probeable;
- the schema-2 16-row parity matrix contains exactly one sanitized fixture and
  declared primary/fallback executor and implementation, limit profile,
  fallback disposition, typed exception, and honest evidence boundary for
  every retained job;
- the five-entry fixture registry owns all 16 jobs exactly once and verifies
  the licensed media manifest, dense segment set, edit timeline, transcription
  adapter input, and caption document by SHA-256 with no customer media;
- the fixture SHA-256
  `7e63a1e00ad2f12b52d0b68fd017a615501ab529f666839e1b6c491df4943dd0`,
  size 292,382 bytes, ISO BMFF markers, AVC/H.264 video, AAC audio, 640x360 at
  30 fps, 48 kHz mono audio, and two-second duration matched the checked-in
  provenance/probe manifest;
- the offline executor HEAD/reuse, attempt staging, atomic publication,
  cancellation cleanup, and privacy-safe debug boundary passed without network
  access.
- 35 sanitized pinned-plugin native-worker tests executed real synthetic
  VP8/Opus WebM through `thumbnail_v1`, `probe_v1`, `audio_presence_v1`, and
  `waveform_v1`, including exact PNG/JSON validation, plural streaming,
  cancellation, lease loss, cleanup, redaction, closed codec policy, and the
  exact typed implementation/exception entry for all 14 native profiles;
- the runtime doctor and checked-in manifest agreed on all 38 required/optional
  factory names, capabilities, platform scopes, availability, and trusted
  plugin provenance;
- 5 focused control-plane tests passed for all four bounded Cloudflare Media
  implementations, exact/just-over limits, signatures, private keys, and
  cancellation identity; native and `wasm32-unknown-unknown` compilation cover
  the private-R2 binding adapter,
  trusted-probe router, D1 attempt/lease recovery, managed-to-native fallback,
  attempt-scoped native staging, exact cancellation cleanup, manifest
  publication, and tenant/domain cutover-fence paths;
- the ordered expand-first D1 migrations applied with no foreign-key
  violations, including the 16-profile policy catalog, trusted source probes,
  fenced execution journal, immutable output manifests, event chain, scoped
  cutover controls, API-workflow replay authority, and immutable dense
  media-job input authority;
- the provider-free media-input SQLite suite rejects sparse, mutable,
  cross-tenant, stale-governance, duplicate segment, and incomplete source
  sets; permits repeated composition occurrences by ordinal; and proves claim
  versus authority mutation is serialized by a real write lock.

Migration `0027_media_job_inputs.sql` now persists and transports the declared
1--64-source protocol with current manifest/governance revalidation. Despite
that transport, `segment_mux_v1` is rejected by native and hybrid-remote
admission before persistence because its executable graph is not audited.
Every other non-executable native profile fails terminally as
`unsupported_media` through its typed exception; setting the codec approval
environment value alone enables no graph.

The fixture is generated from FFmpeg `testsrc2` and `sine` filters and is
declared CC0-1.0 in `fixtures/media-jobs/v1/synthetic-h264-aac.json`. It contains
no Cap or customer bytes. `fixtures/media-jobs/v1/README.md` deliberately leaves
the remote result gate open; a real output must be kept in a restricted evidence
store and represented here only by an approved redacted digest/probe record.

Reproduce with:

```sh
cargo test -p frame-media jobs::service::tests --lib
cargo test -p frame-media --test media_service_contract
cargo test -p frame-control-plane --lib
cargo check -p frame-control-plane --target wasm32-unknown-unknown
cargo clippy -p frame-control-plane --all-targets -- -D warnings
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test -p frame-media-worker --all-targets
python3 -I scripts/ci/check-migrations.py
python3 -I scripts/ci/check-media-service.py
python3 -I scripts/ci/media-job-inputs-sqlite-conformance.py
```

Expected focused result is 35 native-worker tests, 5 Cloudflare adapter tests,
and successful migration and static catalog/fixture verification. Protected
provider/native output lanes remain open regardless of these local passes.
