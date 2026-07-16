# Media service operating runbook

This runbook covers the hybrid Cloudflare Media, native GStreamer, and declared
external-provider job contract. It must be used together with the R2 lifecycle,
native runtime, job protocol, and release/cutover runbooks.

## Release gates

Do not enable an executor/profile until all rows for that exact combination are
green:

| Gate | Managed derivative | Native job | External provider |
| --- | --- | --- | --- |
| catalog/profile revision deployed | required | required | required |
| source and output roles authorized | required | required | required |
| exact-limit and adversarial preflight | required | required | required |
| real adapter contract test | remote Cloudflare account | executed trusted graph | sandbox provider account |
| immutable R2 publish/reconcile | required | required | required |
| cancellation and lost-ack recovery | required | required | required |
| quota/cost ceiling configured | required | required | required |
| data-residency/product approval | required | required | required |
| output probe and issue-04 tolerance | required | required | required |
| codec/model/license approval | output codec | runtime codecs | provider/model terms |
| latency/throughput capacity evidence | required | required | required |

The checked-in local fixture is procedurally generated and CC0-1.0. Customer
media must never enter CI evidence. Real remote and native outputs belong in a
restricted evidence store and are represented in the repository only by a
redacted manifest of digests, probe summaries, dates, account/environment IDs,
and approver identities.

## Configuration and kill switches

Use a revisioned capability record rather than editing scattered constants.
Record the catalog version, managed-contract revision, enabled job/profile
pairs, native worker pool, external adapter revision, tenant/residency scope,
per-tenant concurrency and cost quotas, and rollout percentage.

The managed-media kill switch is profile-granular. Turning it off routes a
hybrid job to native only when native preflight succeeds; it never broadens
native limits. Native and external adapters require independent kill switches.
If no approved executor remains, keep the job queued/dead-lettered with a
bounded public failure code. Never temporarily publish a partial object or make
a private original public.

`segment_mux_v1` must remain disabled in native dispatch until the control plane
can persist and emit 2--64 ordered, independently checksummed source descriptors.
A single-source approximation is not a degraded mode: it is a protocol error.

Treat a Cloudflare beta-behavior change, codec/output drift, unexplained cost
increase, checksum/manifest mismatch, lost cancellation, residency mismatch,
or security signal as an immediate profile kill-switch condition.

## Scheduling and capacity

Admission order is: tenant authorization, input role, trusted probe, catalog
lookup, parser envelope, quota/cost reservation, route, then durable claim.
Queues are separated by executor and resource class. Heavy native composition,
distribution masters, remux, and mux work must not starve lightweight probes or
cancel/cleanup operations.

Enforce:

- per-tenant and global queued/running limits;
- per-job native memory, scratch, decoded-byte, frame, track, CPU/GPU, output,
  and decompression-ratio limits;
- provider operations/output seconds and native CPU/GPU/scratch-byte-seconds;
- bounded claim leases, attempts, execution deadlines, and dead-letter age;
- reserved cleanup capacity that autoscaling cannot consume with new work.

Scale native workers on eligible queue depth, oldest eligible age, weighted
resource demand, and observed completion rate. Scale down only after work and
cleanup leases drain. A worker lost after dispatch enters recovery; it is not
immediately replaced against the same final key. Managed and provider queues
scale by concurrency admission, not by assuming unlimited remote capacity.

## Normal operation

The production Worker creates the media job and `media_job_execution_v1` row in
one D1 batch. It uses `waitUntil` only as a latency optimization. The cron in
`apps/control-plane/wrangler.toml` scans once per minute for queued work and for
expired `leased`, `transforming`, `staged`, or `publishing` claims. Operators
must alert if the cron has not run for two intervals or if the oldest eligible
claim exceeds its profile deadline.

For every attempt, expect this ordered audit trail:

1. authorized request and revisioned route reason;
2. quota/cost reservation;
3. durable claim with attempt and lease epoch;
4. staged-object identity allocation;
5. bounded progress or declared indeterminate state;
6. output probes and exact staging metadata;
7. atomic final publication and manifest commit;
8. final cost charge and quota release.

Logs and metrics use job kind, executor, profile revision, route/failure enums,
duration buckets, operation counts, and byte buckets. They must not include
tenant/video/object keys, URLs, headers, credentials, bodies, captions, source
names, checksums, or raw provider/GStreamer diagnostic strings.

## Provider outage or regression

1. Stop admitting the affected managed profile with its kill switch.
2. Leave already claimed attempts fenced. Request cancellation where supported,
   suppress publication, and wait for exact cleanup/recovery evidence.
3. Route only catalog-approved failure classes to native fallback. Invalid
   input, security violations, and cancellation do not fall back.
4. Quarantine output incompatibility. Preserve private output for restricted
   investigation only if retention policy and incident authorization allow it.
5. Compare the current remote contract/probe to the last approved evidence.
6. Re-enable in shadow mode, then small tenant cohorts, after the exact remote
   contract and tolerance suite passes.

If the managed binding is unavailable and native capacity cannot safely absorb
the load, queue or dead-letter. Do not weaken size, duration, codec, memory,
scratch, deadline, or cost limits.

## Cancellation and lost acknowledgement

Persist cancellation before calling an executor. A late managed response may
be consumed only into the attempt staging key; it cannot publish. Delete the
exact staging/final attempt artifacts and confirm absence after the executor is
fenced. Only then acknowledge cancellation and release quota/reservations.

For a lost execute or publish acknowledgement:

- HEAD the deterministic final and attempt staging keys;
- verify length, checksum, content type, source/profile digests, and manifest;
- adopt only an exact journal-verified commit;
- quarantine any mismatch;
- never treat one negative HEAD as proof an executor can no longer publish;
- close recovery only after lease expiry plus durable executor-fence and
  staging/final-absence proof.

The managed adapter records its deterministic staging key before invocation.
After a crash it HEADs and verifies the exact checksum/length/content type and
source/profile metadata; it does not blindly invoke a second paid transform.
Cancellation may race a provider that cannot stop in flight, so cleanup verifies
ownership, removes both staging and final objects, and confirms both absences
twice before the D1 job becomes `cancelled`.

Native upload follows the same authority rule through
`media_native_output_staging_v1`. The reservation must exist before accepting
the body; the object key includes attempt and declared checksum; only an exact
active lease may promote it; and recovery may mark it cleaned only after two
absence checks or an exact committed final manifest. Any object/journal mismatch
is an immutable conflict, not a retry candidate.

A `ready` manifest with a missing final object is a storage-integrity incident,
not a retry signal.

## Dead letters

Dead-letter when attempts are exhausted, no executor is eligible, cost or
residency policy forbids work, cleanup cannot be proven, an output conflicts,
or a committed artifact is missing. The record must retain catalog/profile
revision, bounded failure code, executor/attempt/fence metadata, redacted
artifact fingerprint, accumulated cost, next operator action, and retention
deadline.

Operator actions are limited to:

- retry after a versioned capability or policy change;
- approve an already verified exact committed output;
- complete fenced cleanup and retry;
- quarantine/escalate a conflict or missing artifact; or
- permanently fail and release only after cleanup proof.

Never edit a job into `ready`, reset an attempt counter, reuse an operation ID
for a different source/profile, or manually overwrite a deterministic final
key.

## Data residency and privacy

Before admission, map tenant policy to the source R2 jurisdiction, D1 journal,
managed execution region/terms, native worker placement/scratch encryption, and
external-provider region/retention. If the complete route cannot meet policy,
it is ineligible even when technically supported. Native scratch is private,
per-attempt, size bounded, cleaned on success/failure/cancel, and included in
recovery proof. Provider retention/training flags are fail-closed configuration.

Security response for suspected media leakage is: disable the adapter/profile,
revoke affected credentials, preserve bounded audit metadata without copying
media into logs, identify scoped deterministic objects, invoke the storage
privacy/deletion workflow, and require security approval before re-enable.

## Cost and performance evidence

For each executor/profile/input class, retain this minimum redacted row:

| Field | Required value |
| --- | --- |
| catalog/profile/adapter revisions | exact immutable revisions |
| environment and date | non-secret account/region label and UTC window |
| fixture digest and probe | licensed synthetic fixture only |
| input/output bytes and duration | exact values |
| provider operations/output seconds | exact counters |
| native CPU/GPU/scratch byte-seconds | exact counters |
| estimated and billed cost | currency/rate revision and amount |
| p50/p95/p99 latency | route, execute, probe, publish separately |
| throughput/concurrency | sustained and burst with error rate |
| cancellation latency | request through fenced cleanup |
| fallback and cleanup result | exact bounded outcome |
| output tolerance result | metadata/playback/perceptual/caption/waveform |

Alert on cost-budget exhaustion, charge-without-terminal-state, queue age,
lease expiry, recovery-required age, dead-letter growth, cleanup failures,
manifest/object drift, fallback rate, provider contract drift, cancellation
latency, and output-tolerance regression.

## Rollout and rollback

Roll out one job/profile and tenant cohort at a time: offline fake, native/remote
contract, shadow execution, probe comparison, 1%, 10%, 50%, then 100%. Shadow
outputs stay private and are lifecycle-expired. A cohort advances only with
approved cost, latency, cancellation, cleanup, and output-tolerance evidence.

Rollback disables the affected revision, drains/fences claimed attempts,
reconciles all deterministic staging/final keys, and selects only the cataloged
fallback. Retain immutable compatible artifacts and manifests; never rewrite
them under a new profile revision. Re-enable the previous approved adapter and
profile only if its dependencies and security posture remain valid.

## Local verification

```sh
cargo test -p frame-media jobs::service::tests --lib
cargo test -p frame-media --test media_service_contract
cargo test -p frame-control-plane --lib
cargo check -p frame-control-plane --target wasm32-unknown-unknown
cargo clippy -p frame-control-plane --all-targets -- -D warnings
python3 -I scripts/ci/check-migrations.py
python3 -I scripts/ci/check-media-service.py
```

The commands above are hardware-free and network-free. They do not satisfy the
remote Cloudflare, native codec graph, provider account, platform performance,
codec/license, residency, or human product-quality gates.
