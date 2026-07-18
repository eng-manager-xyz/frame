# Local media-conformance evidence

This record covers Issue 29's provider-free test definition and executable
comparators only. It does not claim Cloudflare Media output, physical capture,
OS permission behavior, hardware codecs, a one-hour platform soak, approved Cap
goldens, or cross-executor product parity.

On 2026-07-16, the focused local lane passed:

- 11 `conformance` unit tests;
- 5 fixture-driven integration tests;
- 46 matrix cases spanning every declared platform/source/mode/codec/container/
  resolution/executor value, every managed boundary, and every fault;
- exact/just-over production-router checks for bytes, input duration, output
  dimensions, and output duration;
- all nine seeded regression-to-gate mappings;
- metadata, playback, perceptual, waveform, caption, A/V sync, progress,
  latency, resource, cost, routing, fault, and idempotency comparators;
- a 13-seed fuzz corpus and 1,582 deterministic substitutions/truncations; and
- strict fixture hashes plus a canonical release-dashboard generator.

Reproduce with:

```sh
cargo test --locked -p frame-media conformance::tests --lib
cargo test --locked -p frame-media --test conformance_contract
cargo clippy --locked -p frame-media --test conformance_contract -- -D warnings
python3 -I scripts/ci/check-media-conformance.py \
  --evidence target/evidence/media-conformance-offline.json \
  --dashboard target/evidence/media-conformance-dashboard.json
```

The local evidence reports `remote_or_hardware_execution_claimed: false` and
seven protected records pending. The dashboard reports offline rows as
`definition_validated`, not as provider or hardware passes. Its
`promotion_ready` value is false.

The following remain release-blocking protected evidence:

- current macOS/arm64 capture, permission, hardware/software codec, lifecycle,
  and at least one one-hour soak;
- current Windows/x86-64 equivalents;
- Linux/x86-64 preview capture/packaging and one-hour soak;
- an isolated, cost-approved Cloudflare Media boundary/fault/usage run;
- real native-versus-managed artifact comparison for overlapping profiles; and
- protected transcription/AI adapter quality, fault, idempotency, usage, and
  cost evidence; and
- the scheduled fuzz/crash-triage artifact from CI rather than this local run.

The fixture's perceptual/waveform/caption thresholds are detector-sensitivity
values only. Approved product baselines and remote cost caps remain required;
missing data does not relax either boundary.
