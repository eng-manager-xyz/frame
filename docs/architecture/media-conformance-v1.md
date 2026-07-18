# Media conformance v1

Issue 29 supplies a release gate, not another media executor. The gate compares
the behavior implemented by issues 22–28 against the migration charter while
keeping provider, native-hardware, and local synthetic evidence visibly
separate.

## Authority and evidence classes

The migration charter is authoritative for release platforms and objectives.
The media-service catalog is authoritative for executor routing, provider
limits, fallback classes, cancellation semantics, and per-profile sandboxes.
`fixtures/media-conformance/v1/matrix.json` joins those two authorities into a
versioned test definition.

Evidence has five non-interchangeable classes:

1. `offline` uses immutable synthetic metadata, fake traces, and executable
   Rust contracts. It is safe for untrusted pull requests.
2. `native_hardware` requires a representative dedicated runner, physical
   sources or OS permission surfaces, codec provenance, and at least a one-hour
   soak.
3. `remote_managed` requires an approved Cloudflare account, an isolated
   synthetic namespace, a numeric spend cap, and exact cleanup.
4. `external_provider` requires the declared sandbox provider account, adapter
   and model revision, retention/residency approval, and a numeric spend cap.
5. `cross_executor` requires both a real managed result and a native result for
   the same immutable source/profile identity.

An offline row can validate routing or comparator sensitivity. It cannot be
promoted into a provider, platform, codec, permission, quality, or soak claim.
The provenance validator enforces the same boundary in Rust: hardware lanes
need a hardware class, managed lanes need a provider revision, and a
cross-executor artifact needs both.

## Matrix construction

The v1 matrix declares and covers:

- current macOS/arm64 and Windows/x86-64 release classes plus the explicit
  Linux/x86-64 preview class;
- display, window, region, camera, microphone, system-audio, generated-file,
  edit-timeline, and recording-segment sources;
- instant, Studio, managed derivative, native derivative, and hybrid fallback
  modes;
- VP8, H.264, H.265, VP9, AV1, audio-only, and unsupported video inputs;
- Opus, AAC, MP3, PCM, absent, and unsupported audio inputs;
- WebM, MP4, QuickTime, Matroska, Wave, and unsupported containers;
- native GStreamer, Cloudflare Media, external-provider, and control-plane
  executor boundaries;
- exact and just-over managed byte, duration, dimension, quota, and timeout
  cases; and
- the complete device, permission, process, disk, network, provider,
  cancellation, codec, hardware-fallback, drift, and timeout fault inventory.

The checker requires every declared dimension value to appear in at least one
case. It also freezes all nine seeded regression-to-gate mappings. Adding a
dimension without a case or weakening a protected case to local scope fails the
gate.

## Deterministic harness

The immutable manifest hashes the matrix, offline corpus, fuzz corpus, and
protected-lane plan. The Rust integration suite consumes those files at compile
time and exercises the public conformance and media-service contracts. The
Python checker independently validates exact JSON shapes, coverage, source
markers, privacy declarations, and hashes, then emits canonical JSON using
sorted keys and compact separators.

The generated local artifact records the fixture manifest hash, matrix case
count, protected-pending count, executable test command, and evidence class.
It deliberately has no generated timestamp or host claim, so repeated local
runs produce byte-identical JSON for the same source revision.

## Comparator model

The media-pair comparator gates playability, container and codec policy,
dimensions, duration, frame rate, A/V offset, full-reference perceptual score,
waveform correlation, and caption timing. Cross-executor parity does not
require byte identity because two conforming encoders can emit different bytes.

The resource comparator covers absolute and end-growth memory, handles,
threads, disk, CPU, GPU, temperature, output latency, A/V drift, and accumulated
cost. The latency comparator uses deterministic nearest-rank p50, p95, p99, and
maximum calculations. Progress traces reject time regression, mixed
determinate/indeterminate modes, range violations, progress regression, and an
incomplete determinate terminal sample.

The logical-result comparator binds every attempt to one object key and one
checksum, permits only one publication mutation and one billable effect, and
allows subsequent effect-free replays. The routing comparator proves the exact
executor invocation count, including zero calls for preflight rejection. The
fault comparator rejects missing, duplicate, unexpected, or disposition-drifted
scenarios.

Detector sensitivity is not a substitute for an approved product baseline.
The perceptual, waveform, and caption values in the offline corpus are labeled
`local_synthetic_gate_only_not_release_baseline`. Charter values retain their
exact authority, and the remote cost maximum remains null until the named
owner approves a numeric cap for a protected run.

## Provider-limit and fallback proof

The offline suite invokes the production `MediaCapabilityRouter` with 99,999,999
and 100,000,000 byte inputs, 600,000 and 600,001 millisecond inputs, 10/9 and
2,000/2,001 output dimensions, and 1,000/999 and 60,000/60,001 millisecond
outputs. Exact accepted values select managed execution. Rejected values select
native before a managed invocation when the native envelope can safely accept
them.

Quota, timeout, provider outage, output incompatibility, and beta-regression
classes permit the declared native fallback. Invalid input, security
violations, and cancellation cannot be reclassified into a fallback. Live
evidence must additionally prove the durable D1/R2 behavior; the local router
test does not claim that provider execution or object reconciliation occurred.

## Fuzz and crash triage

The fuzz surface is a 512-byte, NUL-free envelope parser for labels, private
object keys, SHA-256 values, and progress. The integration test runs every
checked-in seed plus deterministic byte substitutions and truncations. A panic,
abort, timeout, or sanitizer finding is release-blocking and must retain the
source revision, seed, minimized input, backtrace, runtime, and triage owner.
Private or customer bytes are forbidden from the corpus.

## Release dashboard

`check-media-conformance.py --dashboard` emits one row per offline case and one
row per protected record. Every row contains executor, baseline, budget,
result, trend, flake, cost/usage, and evidence link. Offline rows say
`definition_validated`; they do not imply that hardware or provider output
passed. Protected rows stay `not_collected`, with no evidence link, until their
trusted lanes complete.

Promotion is false while any required protected record is absent. A waiver is
not a pass: it needs an owner, user impact, expiry, cost/security review, and
rollback, and remains visible in the immutable release record.

## Rollout and rollback

The offline matrix and mutation suite are required and provider-free. Native,
remote, and cross-executor lanes remain release gates. Rollback disables a
managed profile or returns it to its approved native/legacy executor; it never
loosens comparison budgets, publishes a partial object, repeats a billable
effect, or deletes outside the test namespace.
