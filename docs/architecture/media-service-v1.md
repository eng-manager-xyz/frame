# Media service v1 contract

Status: executable provider-neutral catalog/router, D1 lease and publication
authority, isolated Cloudflare Workers binding adapter, offline adapter, four
native GStreamer graphs, immutable dense multi-source D1 authority, and a
licensed synthetic fixture implemented. Ten native profiles (including the
segment-mux graph), external-provider adapters, remote-account Cloudflare
execution, and protected cross-executor evidence remain release gates until
their respective adapters and environments are exercised.

## Catalog and retained Cap surface

The machine-readable authority is `media_service_catalog()` in
`crates/media/src/jobs/service.rs`. Catalog schema v1 is pinned to
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b` and contains exactly
one declaration for each retained job:

| Job | Input -> output | Primary | Fallback |
| --- | --- | --- | --- |
| optimized clip | source original -> preview MP4 | Cloudflare Media | native GStreamer |
| frame | source original -> JPEG/PNG thumbnail | Cloudflare Media | native GStreamer |
| spritesheet | source original -> JPEG spritesheet | Cloudflare Media | native GStreamer |
| audio extract | source original -> AAC/M4A | Cloudflare Media | native GStreamer |
| probe | source original -> probe manifest | native GStreamer | none |
| audio presence | source original -> probe manifest | native GStreamer | none |
| distribution master | source original -> H.264/AAC MP4 | native GStreamer | none |
| animated preview | distribution master -> GIF/MP4 | native GStreamer | none |
| audio normalize | extracted audio -> normalized audio | native GStreamer | none |
| remux/repair | source original -> repaired MP4 | native GStreamer | none |
| segment mux | recording segments -> muxed MP4 | native GStreamer | none |
| waveform | extracted audio -> waveform JSON | native GStreamer | none |
| composition | edit timeline -> composed MP4 | native GStreamer | none |
| normalize | source original -> normalized MP4 | native GStreamer | none |
| transcription | extracted audio -> WebVTT | declared external adapter | none |
| AI cleanup | captions -> bounded JSON metadata | declared external adapter | none |

The catalog also owns input/output roles, normalized profile ID, progress and
cancellation behavior, timeout, maximum attempts, retryable failure classes,
idempotency rule, accepted output media types, sandbox/resource/cost envelope,
network policy, and the exact pinned Cap source references. Upload, download,
job status/cancellation/webhook delivery remain owned by issues 07 and 19 and
are called out in `fixtures/media-jobs/v1/catalog.json` rather than duplicated
as media transformations.

`fixtures/media-jobs/v1/parity-matrix.json` supplies one stable parity case for
every catalog row. Each case names its sanitized input fixture, implementation,
limit profile, fallback disposition, and current evidence boundary. A protected
case is still an explicit fixture and release gate; it is not represented as a
passing native/provider output.

## Trusted input and preflight

Executors never receive public or signed source URLs. `PrivateMediaInput`
contains an opaque tenant/video-scoped R2 key, exact source version and
SHA-256, verified probe metadata, and conservative decoded byte/frame/track
upper bounds. Validation requires canonical UUIDs, a key beneath
`tenants/{tenant}/videos/{video}/`, no URL syntax, query/fragment/path traversal,
and either a verified upload manifest or verified native probe in the neutral
port. The production managed Worker is deliberately stricter: it admits a
Cloudflare transform only from `media_source_probes_v1` with
`verified_native_probe` trust whose key, checksum, byte length, content type,
tenant, video, and source version still match the authoritative manifest. Debug
output redacts tenant, video, key, and checksums.

Preflight occurs before selecting an executor. The shared parser envelope caps
source bytes, duration, dimensions, decoded bytes, frames, tracks,
decompression ratio, output bytes, memory, scratch, CPU/GPU time, and cost.
Native and external jobs have explicit network policies. Unknown or caller-only
metadata fails closed.

The revisioned managed contract `cloudflare-media-binding-2026-06-10` is the
single authority for the binding surface and the 2026-04-21 published product
limits:

- input size is strictly less than 100,000,000 bytes and duration is at most
  ten minutes;
- input must be MP4 with H.264 video and AAC, MP3, or no audio;
- start is bounded to ten minutes, must be inside the source, and is routed to
  the binding only when representable by the adapter's whole-second contract;
- timed outputs are one through sixty whole seconds;
- requested dimensions are 10 through 2,000 pixels;
- every binding result is capped at 32,000,000 bytes so the adapter can compute
  the required R2 SHA-256 before its conditional immutable PUT, and spritesheet
  image count is separately operator-bounded;
- the unpublished input-resolution ceiling is an explicit Frame operator cap
  that must be exercised by the remote contract lane.

Exact-limit and just-over-limit tests cover every managed mode. Known
unsupported inputs route to bounded native work and never reach the binding.
Disabling the managed kill switch makes the same deterministic decision.

## Routing and fallback

Only optimized clips, frames, spritesheets, and audio extraction are hybrid.
Cloudflare is preferred only after both managed preflight and native fallback
preflight pass. A fallback may follow quota, timeout, provider outage, output
incompatibility, or beta regression. Invalid input, security violations, and
cancellation never fall back. Native-only and external-provider jobs cannot be
silently reclassified.

Fallback preserves the logical job, source digest, profile digest, and final
artifact identity while advancing the durable attempt fence. An output is
therefore reusable across executors without allowing two logical results.
Routing reasons are stable enum values suitable for bounded metrics; they do
not contain tenant data.

## Immutable objects and publication

`MediaArtifactIdentity` derives the final and manifest keys from catalog
version, tenant/video scope, job kind, source version and digest, normalized
profile digest, and output format. Attempt-specific bytes are written only to
`{final}.attempt-{n}.partial`. The executor port can HEAD, execute into staging,
publish, and cancel/clean the exact fenced attempt.

The durable state machine is:

```text
queued -> running -> publishing -> ready
           |            |
           +-> retryable/failed/recovery-required
           +-> cancel-requested -> cancelled
```

`media_job_execution_v1` claims bind attempt, lease epoch, expiry, and a hashed
lease token. A one-minute scheduled Worker reconciles queued and expired
managed claims, reuses an exact staged/final object after a lost acknowledgement,
and resumes publication without changing the logical artifact identity. Stale
callbacks cannot advance state. Monotonic jobs reject progress regression;
indeterminate jobs cannot invent numeric progress. Cost accumulation is checked
for overflow and against the job envelope.

Native output upload first reserves an immutable row in
`media_native_output_staging_v1`, then streams to the exact
`{final}.attempt-{n}.{sha256}.partial` key. Completion verifies that R2 object,
rechecks the active attempt/lease and cancellation fence, conditionally copies
it to the deterministic final key, and commits the staging transition, manifests,
job state, and outbox in one D1 batch. Scheduled recovery removes only a staging
object whose exact publication authority is committed, or an abandoned/cancelled
attempt after its fence settles; conflicts fail closed.

Publication requires a staged checksum, length, content type, source/profile
digests, executor/attempt identity, and all declared metadata, playback,
perceptual, caption, and waveform verification flags. The final manifest must
match that staged record exactly. A committed manifest is the only `ready`
postcondition. Debug output for staged objects, heads, manifests, and requests
redacts all scoped keys and digests.

## Ambiguity, cancellation, and reconciliation

A negative HEAD is never enough to close an ambiguous in-flight attempt: an
acknowledged-late managed or native executor could still publish. Recovery
retains the reservation until a proof bound to attempt and lease epoch confirms
the executor is fenced and both staging and final objects are absent after
lease expiry. Conflicting objects are quarantined.

Cancellation records intent durably before adapter cancellation. Managed work
may finish in flight, but publication remains suppressed. The job becomes
`cancelled` only after exact staging cleanup is confirmed. A `ready` job whose
committed object disappears raises a missing-artifact incident rather than
rerunning and hiding loss.

## Distribution master

`distribution_master_v1` is a native-only, immutable derivative. Its target is
MP4 with H.264 video, AAC-LC audio when audio exists, progressive playback,
BT.709 limited-range video, square pixels, bounded geometry, and normalized
timestamps. The source/editable original remains a separate immutable object
and is never replaced or transcoded in place. H.264/AAC encoder availability,
redistribution, and patent/product approval remain the codec gates defined by
issues 22, 27, and 29; a profile declaration is not legal approval.

## Adapter boundary and evidence

`MediaDerivativeExecutorPort` is provider neutral. The offline implementation
uses the same private reference, deterministic staging/final identity,
idempotent HEAD/reuse, and cleanup contract without network access. Production
adapters must preserve that boundary:

- Cloudflare interop is isolated from domain code and receives an R2
  `ReadableStream`, never a customer-facing URL;
- native workers receive a fenced job and use only declared GStreamer graphs
  under the trusted-runtime policy;
- transcription/AI adapters are separately declared network capabilities and
  may not inherit object-store or general internet access;
- D1 owns journal transitions and the manifest authority; R2 owns immutable
  bytes. `waitUntil` starts an admitted managed job, while the scheduled handler
  is the durable recovery path rather than a correctness dependency on the
  original request lifetime.

Local evidence proves the catalog, limit matrix, fault paths, deterministic
identity, D1 schema invariants, native and wasm compilation of the binding and
scheduled recovery consumer, monotonic progress, cancellation suppression,
ambiguous-execution fencing, licensed synthetic MP4 integrity, and offline
adapter semantics. The schema-2 parity matrix cross-links every retained job to
a concrete SHA-pinned CC0 fixture, executor, named implementation, numeric
limit-profile authority, fallback/disposition, evidence state, and typed
exception.

The native worker has a closed 14-profile implementation catalog. It locally
executes thumbnail PNG, probe, audio-presence, and waveform graphs; every other
profile has a stable graph identity, pinned factory set, and typed exception
instead of an anonymous unsupported branch. Migration 0027 now binds 1--64
ordered immutable source occurrences, revalidates current manifest/governance
authority at claim and delivery time, and supports repeated composition inputs
by ordinal. Segment mux remains deliberately undispatchable because its
GStreamer graph is still a typed, unaudited exception; completing source
transport does not fabricate executable output. Local evidence does not prove
actual Cloudflare outputs, protected native graphs, provider accounts, platform
performance, codec licensing, or product/perceptual review. Those are explicit
protected gates in the runbook.
