# Media conformance operations

This runbook produces Issue-29 evidence without confusing local contracts with
provider or hardware results.

## Fast offline gate

From a clean checkout with the pinned Rust/GStreamer toolchain:

```sh
cargo test --locked -p frame-media conformance::tests --lib
cargo test --locked -p frame-media --test conformance_contract
cargo clippy --locked -p frame-media --test conformance_contract -- -D warnings
python3 -I scripts/ci/check-media-conformance.py \
  --evidence target/evidence/media-conformance-offline.json \
  --dashboard target/evidence/media-conformance-dashboard.json
```

The weekly `Media conformance schedule` workflow runs the same immutable matrix
and seeded mutation corpus without secrets. A retry does not replace the first
failure; record it as a flake and retain both run links.

`--require-protected` intentionally fails against the checked-in plan while a
protected record is absent. Do not change `not_collected` to `passed` merely to
make that command green.

## Native dedicated-runner plan

Use named, patched, dedicated machines for current macOS/arm64 and
Windows/x86-64. Linux/x86-64 is a preview gate and remains separately labeled.
Record the immutable Git SHA, OS build, architecture, desktop/session type,
GStreamer and plugin inventory, Frame version, capture API, device model and
driver/firmware, hardware codec, software fallback codec, power mode, thermal
state, profile version, and fixture hashes.

For each supported native platform:

1. Exercise display, window, region, camera, microphone, and system-audio
   sources that the platform declares.
2. Exercise permission denied, permission revoked during capture, device loss,
   process crash, disk full, cancellation, unsupported codec, and hardware
   encoder failure.
3. Run the hardware path and the identical approved profile in software. Probe
   both outputs and compare metadata, playback, frames, waveform, A/V sync, and
   compatibility without requiring byte identity.
4. Repeat start/stop while collecting start, peak, and end RSS, handles, threads,
   disk, CPU, GPU, temperature, output latency, drift, and cost-equivalent
   resource units.
5. Complete at least 3,600 continuous seconds. A one-hour wall clock that spent
   time suspended, asleep, or disconnected is invalid; record active elapsed
   time and sample cadence.
6. Confirm no final artifact exists after cancellation or failed verification,
   and that crash recovery either adopts an exactly verified artifact or
   performs a fenced retry.

Attach representative synthetic artifacts and their SHA-256 values. Never
attach screen contents, device names containing user identity, filesystem
paths, tokens, or private object keys.

## Remote managed lane

The remote lane runs only from the protected `media-conformance` environment
after a named owner enters a numeric maximum cost and reviews security. Use the
CC0 synthetic H.264/AAC fixture from `fixtures/media-jobs/v1`; verify its bytes
and digest before upload.

Before invoking the binding:

1. Choose an isolated account/environment and a namespace of the form
   `conformance/<git-sha>/<random-run-id>/`. The cleanup principal must have no
   permission outside that namespace.
2. Set concurrency to one, a hard per-job timeout, a maximum invocation count,
   and the approved numeric cost cap. Abort before invocation if any value is
   absent.
3. Record Worker revision, binding contract revision, provider/account alias,
   profile version and digest, input digest/bytes/duration/probe, and kill-switch
   state. Aliases must not contain account IDs or credentials.
4. Exercise exact and just-over input byte/duration, output dimension/duration,
   format, quota, and timeout boundaries. A preflight rejection must record zero
   provider invocations.
5. Inject provider error, outage, quota, timeout, cancellation, and managed
   output drift. Verify the declared stable failure or one fenced native
   fallback.
6. Probe accepted outputs and compare metadata, playback, perceptual frames,
   waveform, A/V sync, and caption timing against the approved baseline.
7. Query the scoped durable records and object HEADs. Exactly one logical final
   key may exist; the manifest, checksum, source/profile digests, executor,
   attempt, usage, and cost must agree. Replaying the request must not repeat a
   billable or publication effect.
8. Delete only the exact test keys, then perform two bounded negative HEADs for
   staging and final objects. Reconcile D1 to a terminal state and retain the
   redacted cleanup receipt.

Provider dashboard exports are supporting evidence, not the sole authority.
The signed run record and object/database reconciliation must agree with them.

## Cross-executor lane

Run the same source digest and normalized profile digest through Cloudflare
Media and native GStreamer. Retain separate provenance and cost records, then
run the pair comparator. A passing result needs playable outputs, declared
metadata/codec compatibility, approved dimensions and timing, perceptual and
waveform scores, caption timing where applicable, deterministic logical keys,
and one publication effect. Byte equality is neither required nor sufficient.

## External-provider adapter lane

Use a separate protected sandbox account for transcription and AI-cleanup
profiles. Pin the adapter and model revisions, region, retention/training
settings, synthetic spoken-audio/caption fixtures, request idempotency key,
timeout, concurrency one, and a numeric cost cap. Exercise provider error,
outage, quota, timeout, cancellation, replay, and unsafe/unsupported input.
Compare caption content and timing to the approved synthetic baseline, and
require human product review for cleanup semantics. Record exact usage/cost and
prove that replay does not repeat a billable or publication effect. This lane
cannot borrow a Cloudflare Media result or an offline fake result.

## Scheduled fuzz and triage

The unprivileged weekly workflow executes the fixed seed corpus and 1,582
deterministic byte mutations. For a crash, abort, timeout, sanitizer finding, or
unexpected acceptance:

1. preserve the first failing run and exact Git SHA;
2. minimize without introducing private bytes;
3. attach the seed ID, mutation index/mask, backtrace, runtime/tool versions,
   and expected gate;
4. open a release-blocking finding with an owner; and
5. add the minimized case to a new fixture version after review.

Do not silently drop a seed because it becomes slow or flaky.

## Dashboard and trend review

Open `target/evidence/media-conformance-dashboard.json`. Each row must include
executor, baseline, budget, result, trend, flake, cost/usage, and evidence link.
Reviewers must reject:

- a protected row reported from local synthetic evidence;
- a result without matching source/profile/runtime provenance;
- a cost or usage result with a missing numeric protected-run cap;
- a trend that discards the first failure or hides retries;
- a hardware soak shorter than 3,600 active seconds; or
- a result whose artifact digest cannot be independently reproduced.

## Waiver, rollout, and rollback

A waiver requires owner, user impact, expiry, cost/security review, and
rollback. It is scoped to one row and one release and cannot waive privacy,
cross-tenant access, corruption, duplicate billing, or acknowledged-write loss.

If a managed or hardware row regresses, disable that profile and route through
the last approved native/legacy path. Preserve durable state and evidence;
never clean unrelated keys, relax the budget, or overwrite the first failing
record.
