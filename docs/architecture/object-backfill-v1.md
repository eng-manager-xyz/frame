# Object backfill and reconciliation protocol v1

This document specifies the provider-neutral, locally testable contract for issue 20. It builds
on the canonical immutable object contract in
[`storage-contract-v1.md`](storage-contract-v1.md) and the multipart finalization contract in
[`multipart-storage-v1.md`](multipart-storage-v1.md). It does **not** implement or claim evidence
for a real S3, MinIO, BYO, Google Drive, or Cloudflare R2 adapter; those protected production
records remain outside this local contract.

## Immutable migration authority

`ObjectBackfillManifestV1` is the immutable authority for a migration. Its versioned, length-framed
SHA-256 digest binds all of the following:

- protocol and manifest schema versions, manifest ID, and creation time;
- exact tool and code versions;
- source and target provider classes, regions, non-secret provider locators, and authority
  fingerprints;
- the complete concurrency, attempt, object, logical-byte, cost, bandwidth, rate, retry, circuit,
  lease, and chunk execution policy;
- entry ID, tenant, video, object role, and credential-free source reference;
- exact canonical `ScopedObjectKey` and source revision at the destination;
- logical byte size, required full-object SHA-256, content type, media-probe mode/profile version,
  and an optional opaque source-provider checksum.

Entry and destination-key IDs must be unique. Source references may repeat so reconciliation can
classify legacy duplicates instead of silently normalizing them. Deserialized manifests reject
unknown fields and must pass `validate_integrity`; changing any bound field without recomputing the
digest is rejected. The immutable manifest port accepts an identical replay and rejects replacement
under the same manifest ID.

Provider checksums are diagnostic only. `BackfillProviderChecksumV1` cannot substitute for the
required `ChecksumSha256`; in particular, a multipart ETag is never interpreted as a content hash.

## Credential boundary

Credentials are not manifest data. `BackfillCredentialRefV1` is an opaque runtime secret-store
reference that deliberately implements neither `Serialize` nor `Clone`. Its `Debug` and `Display`
forms are redacted, and raw access is available only through the provider-adapter accessor.
`BackfillRuntimeBindingsV1` binds one source and one target credential reference to the exact
manifest authorities. A provider, region, locator, or fingerprint mismatch fails before inventory
or object I/O.

This boundary proves absence from the versioned manifest and generic debug output. It does not
prove that a production secret store is encrypted, correctly scoped, rotated, or audited; those are
protected deployment requirements.

## Durable journal, leases, and receipts

Mutable state lives in `ObjectBackfillJournalV1`, never in the manifest. The journal is bound to the
manifest ID and digest and contains:

- one ordered entry record with status, attempt count, last failure class, and receipt;
- a monotonically increasing compare-and-swap revision;
- the last durable transition time, which makes clock rollback a contract error;
- expiring worker leases with monotonically increasing fencing tokens;
- aggregate admitted object, logical-byte, and cost budgets;
- consecutive provider failures, an outage-circuit deadline, and one fenced half-open probe;
- run control (`running`, `paused`, `aborted`, or `completed`); and
- a source-retention gate.

The normal entry lifecycle is:

```text
pending -> leased -> succeeded
              \-> retry_scheduled -> leased
              \-> quarantined -> owner-approved retry
                              \-> owner-approved reference/exclusion -> terminal
```

Claims, renewal, success, failure, pause, resume, abort, disposition, and source-release transitions
produce the exact next revision. `BackfillJournalPortV1::compare_and_swap` accepts only
`expected_revision + 1`. Concurrency admission counts every live lease in the manifest, including
leases for other entries and tenants, while excluding expired leases. An expired claim is first
checked for its exact operation-bound target: a committed target is checkpointed without a new
attempt or budget charge; an absent or inexact target is durably normalized before any new claim.
This maintenance scan is manifest-global and runs before tenant selection. It prioritizes an
expired half-open lease, so an abandoned probe for tenant A cannot retain the circuit token and
wedge tenant B. A stale worker cannot write a terminal receipt or publish an object.

A success receipt binds the operation and entry IDs, created/reused result, provider destination
version, logical bytes, strong SHA-256, media-probe profile, and commit time. Journal-load validation
checks those fields against the immutable entry. If a D1-like adapter commits a CAS and loses the
acknowledgement, the coordinator reloads the journal and accepts only the exact proposed state.

## Streaming copy contract

The application core never accepts or returns a whole-object `Vec<u8>`. Provider reads and
conditional writes are boxed, single-consumer asynchronous streams. Each chunk is non-empty,
bounded by both a protocol ceiling and the manifest execution policy, and owned by one write call.
The coordinator keeps only one chunk plus constant-size hash/probe state, so the provider naturally
applies backpressure.

For a new destination the coordinator performs these checks in order:

1. exact target HEAD; an existing object must pass a full streamed post-read or the entry fails;
2. exact source metadata, including authority, tenant, video, role, reference, size, content type,
   optional strong metadata, and opaque provider checksum when declared;
3. conditional immutable destination create bound to the entry and operation IDs;
4. for every source chunk: a fresh injected time, durable lease heartbeat, cancellation, size
   bound, cumulative-size bound, multidimensional bandwidth admission, incremental SHA-256,
   streaming media-probe observation, one backpressured write, and another heartbeat;
5. exact EOF, full logical size, required SHA-256, and required playable probe;
6. a final heartbeat and cancellation check, followed by destination commit whose adapter invokes
   a live durable commit fence immediately before publication; and
7. an independent HEAD plus full streamed target read, SHA-256, size, metadata, operation
   provenance, version, and media probe, with lease heartbeats throughout, before the journal
   receipt is written.

The commit fence reloads the exact journal at a fresh time and requires a running manifest, the
same entry, operation, worker and fencing token, and a non-expired live lease. Abort or lease
normalization therefore wins before publication even when a writer is already waiting in commit.
Early EOF, extra bytes, checksum drift, unplayable media, empty/oversized adapter chunks, midstream
errors, and cancellation fail closed. In-flight read, write, and probe sessions receive idempotent
release/cancel calls on success and failure; ownership/`Drop` remains the final cleanup backstop.

Provider object commit and journal success cannot be atomic. These unknown outcomes are safe:

- destination absent after an error: retry after policy backoff;
- destination committed but commit acknowledgement lost: exact target post-read recovers it;
- destination committed but the process died before journal success: the next run requires the
  exact persisted operation provenance, streams the target, records a `created` receipt, and does
  not consume another attempt, concurrency slot, byte budget, cost budget, or create call;
- journal CAS committed but its acknowledgement was lost: exact journal reload recovers it;
- a stale worker finishes after lease reclaim: its receipt is fenced, while the immutable target
  remains reusable by the current worker.

## Controls and deterministic retry

The immutable manifest binds one `BackfillExecutionPolicyV1`; callers cannot replace it on a
process or reconciliation request. It provides explicit ceilings for concurrency, attempts,
admitted objects, logical bytes, cost units, bandwidth, objects per minute, chunk size, lease
lifetime, exponential backoff, circuit threshold, and circuit cooldown. Tenant plus both source and
target provider/region dimensions are passed to object and byte admission. Source egress and target
write costs are independently estimated, summed, and durably charged once at claim. Exhausted
budgets do not begin another transfer.

Retries are journaled as a deterministic `not_before` timestamp. Transient throttle, expired
authorization, outage, and cancellation classes are retryable. Integrity, ownership, missing-data,
capability, and destination-conflict classes require an owner disposition. Consecutive provider
failures open a durable circuit; a non-provider outcome resets the consecutive sequence. After the
cooldown exactly one half-open probe may hold the circuit token. An owner-approved retry is scoped
to the exact manifest/entry/disposition and permits exactly one bounded extra attempt, never an
unbounded reset. A live half-open token always names exactly one leased entry; after that lease
expires, global normalization clears the token before tenant-scoped admission resumes. The core
does not sleep while holding a lease.

All state-machine and admission times come from `BackfillClockPortV1`. Callers cannot assert a
timestamp. Each long-running boundary obtains fresh time, and any time earlier than the last
durable transition fails with an explicit clock-rollback error.

## Capability preflight

Every initialize, process, and reconciliation call rechecks provider capabilities. The source must
support streaming reads, complete snapshot inventory, and independent inventory. The target must
additionally support streaming conditional create, immutable versions, exact post-write HEAD,
cancelable staging writes, live commit fencing, and complete snapshot inventory. A missing
declaration fails before object mutation. A real adapter must prove that its advertised capability
matches provider behavior; declaring `true` is not evidence.

## Independent reconciliation

Reconciliation performs two complete source passes and two complete target passes for every
manifest tenant, then requires the paired indexes to be identical. Each page is bound to a stable
snapshot digest and exact zero-based page index. Repeated/backward cursors, an empty page with a
continuation, snapshot mutation, excessive pages/objects, and rows with the wrong tenant,
authority, or side fail the entire provider inventory closed. Lookup is indexed rather than one
provider scan per manifest entry. Every returned row that participates in migrated inventory,
including an orphan, is independently HEAD/open-read verified against its inventory metadata and
streamed bytes. A row at the exact location of a trusted terminal exclusion is instead omitted from
migrated observations and retained in disposition evidence; requiring corrupt excluded bytes to
verify would make the approved exclusion impossible. Reused locations that remain expected for a
different manifest entry are not suppressed. Reconciliation compares:

- object count and logical bytes;
- per-role counts;
- full SHA-256 verification count; and
- required media-probe verification count.

It classifies missing source/target, duplicate source/target, orphan source/target, ownership
mismatch, corrupt source/target, unplayable source/target, metadata divergence, checkpoint
divergence, and provider unavailability. A `created` receipt also requires exact target operation
provenance and destination version. An exact target paired with a non-success journal, or a success
journal paired with a missing, corrupt, or provenance-drifted target, is checkpoint divergence.

Every discrepancy contains a deterministic object fingerprint derived from the authority, side,
location, version, and occurrence without exposing raw object identifiers. Equal discrepancy kinds
remain independently actionable in the dry-run repair plan. Expected totals are derived from the
complete immutable manifest and its explicit, verified terminal dispositions:

- a normal entry is expected and stream-verified on both source and target;
- `reference_approved` remains expected and stream-verified on the source, expects no target, and
  therefore produces neither `missing_target` nor checkpoint divergence merely because the target
  is absent; an unexpected target is still an orphan; and
- `exclude_approved` is removed from migrated source/target expected and observed totals only when
  the journal contains the exact trusted approval. Its object, logical-byte, and per-role counts
  remain visible in separate auditable exclusion totals. Missing or corrupt excluded data may
  reconcile clean, but an unapproved, forged, or stale disposition cannot.

Each report embeds the entry, failure, terminal disposition, and redacted approval attestation for
every reference/exclusion, plus separate referenced and excluded count/byte/role totals. Approval
records bind the authenticated subject, exact scope, issue/expiry interval, and the time the trusted
adapter verified the capability. Verification must have occurred within that interval and no later
than the journal transition or report. Reordering disposition input produces the same normalized
report and digest. The report digest binds these records and validates its manifest binding,
derived expectations, normalized ordering/deduplication, clean bit, generation time, and maximum
age before it can authorize source release. `dry_run_repair_plan` is a pure mapping that has no
mutation port.

Source release is a second, independent gate. The coordinator reruns reconciliation at approval
time and changes only the journal retention state when all of these are true:

- the journal is completed;
- the report is clean and bound to the exact manifest digest;
- the report's terminal disposition evidence exactly equals the completed journal's evidence; and
- the trusted approval adapter verifies an opaque, unforgeable owner capability whose subject,
  manifest/report scope, issue time, and expiry are recorded in the journal.

The gate does not delete source data. A provider-specific, audited retention job remains required.

## Provider-free evidence and protected work

The deterministic adapters intentionally buffer small fixture bytes inside the adapter, inject
faults, and emulate conditional commit. They prove the application boundary does not whole-buffer
objects; they do not prove a provider SDK or network runtime streams with the same memory bound.

The following records remain protected and uncollected:

- real Cap-managed S3, MinIO, BYO/custom S3, Google Drive, and Cloudflare R2 inventory/copy behavior;
- production provider credentials, secret-store encryption/scope/rotation, and access audit;
- customer consent, residency, permission, egress, and expectation decisions for BYO and Drive;
- real provider checksum/version semantics, pagination consistency, conditional create, throttling,
  expiry, outage, cancellation, and cleanup behavior;
- D1-backed journal migrations, scheduled workers, monitoring, alerts, and operator authorization;
- production-scale object/byte throughput, error rate, memory, provider cost, and egress budgets;
- owner-approved quarantine dispositions and user-impact lists;
- playback/probe comparison using real media for every role/provider class; and
- observation-window completion, source deletion, rollback rehearsal, and final owner sign-off.

Until those records exist, this protocol is a production-shaped local contract only. It must not be
cited as provider compatibility, production-scale rehearsal, credential-security, customer consent,
or migration-completion evidence.
