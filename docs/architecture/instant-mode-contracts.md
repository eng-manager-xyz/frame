# Instant Mode contracts

Instant Mode is a provider-neutral distributed state machine. The media crate
owns immutable segment identity, bounded payload ownership, local-spool safety,
journal fencing, multipart scheduling, and publication reconciliation. Native
GStreamer, filesystem/keychain, R2/multipart, D1/job, desktop, and share-page
adapters remain outside this module; the control-plane crate now supplies the
R2/D1 server-finalize adapter without importing the native media dependency.

```text
exact native A/V timeline
          │
          ▼
audited fMP4 split graph ──> immutable segment descriptor + SHA-256
          │                              │
          ▼                              ▼
authenticated private spool ──> fenced CAS journal ──> multipart part
                                           │                 │
                                           └──── manifest ◀───┘
                                                    │
                                                    ▼
                                  sealed server-finalize request
                                                    │
                                                    ▼
                              one playable publication + derivatives
```

## Identity, versions, and diagnostics

Protocol version 2 separates the protocol, manifest, journal, upload,
finalize, and progress versions. Session, operation, worker, upload, object,
job, and publication IDs are nonzero opaque 128-bit values supplied by a CSPRNG
boundary. They expose equality and ordering but no string or byte serialization
API. Debug output redacts every opaque ID and SHA-256 digest. The runtime spool
key handle is non-cloneable and has neither a serialization nor key-byte API.

The public journal port accepts typed snapshots. `InstantJournalCodec` supplies
a canonical, bounded binary representation with a magic/version prefix and a
trailing SHA-256 integrity checksum. Decode rejects truncation, trailing data,
unknown tags or versions, impossible states, and oversized journals before a
snapshot can re-enter the CAS boundary. This checksum detects accidental
corruption; storage authenticity remains an adapter/D1 obligation. Adapters
must not derive a general-purpose debug/JSON representation of runtime key
handles, opaque provider identities, checksums, or operation receipts. Native
journal state is projected into the shared, versioned
`frame-client::InstantUiProgressV1` contract before it reaches a consumer. The
desktop and public share surfaces both consume its coarse phase, optional
basis-point progress, retry flag, and closed error-code vocabulary. Native
counts, byte totals, identities, hashes, credentials, and receipts are not
fields in the shared projection and cannot cross either UI boundary.

## Segment and manifest identity

An immutable segment descriptor binds:

- session and zero-based index;
- exact start and duration in the corrected session timeline;
- a mandatory video keyframe at the fragment boundary;
- the fragmented-MP4 baseline;
- a bounded, unique set of track numbers and roles;
- exact H.264/AVC video or AAC-LC audio codec, timescale, sample count, first
  presentation time, and duration for every track;
- exact byte length and full plaintext SHA-256; and
- a domain-separated SHA-256 over all preceding fields.

Every track begins and ends at the same segment boundary. A screen-video track
is mandatory. A reused index with different metadata or bytes is a conflict.
The manifest sorts by index, then requires indexes `0..n`, start zero, and
exactly contiguous times. Gaps, overlaps, missing segments, cross-session
segments, overflow, or duplicate indexes fail closed. Its digest excludes
journal revision and insertion order, making the same immutable media produce
the same manifest while still binding every segment identity.

SHA-256 is computed incrementally by the core for segment streaming. The
provider does not choose or attest local media identity.

## Exact pipeline baseline

`InstantPipelineSpec` is data, not a parsed GStreamer string and not execution
evidence. Negotiation succeeds only when an adapter explicitly proves all of:

- `splitmuxsink` and fragmented `mp4mux` support;
- H.264/AVC encoding and force-key-unit support;
- exact running-time split support;
- AAC-LC and aligned audio fragments when audio is enabled; and
- eligibility of the resulting fMP4 as the Media-compatible distribution
  master.

The exact video path is appsrc, bounded queue, conversion, exact caps, H.264
encoder/parser, fragmented MP4 muxer, and splitmux sink. The optional audio path
is appsrc, bounded queue, conversion/resampling, exact 48 kHz stereo caps,
AAC-LC encoder/parser, and the same fragment boundary. Segment duration is
250 ms–30 s; allowed split slip is no more than one quarter of that duration.
Missing keyframe, split, track-alignment, codec, or distribution capability
rejects the graph rather than silently choosing another master.

## Bounded one-owner payloads

`InstantSegmentPayload` is a single-consumer pull body. The validated wrapper:

- requires the provider's declared length to equal immutable metadata;
- requests at most 1 MiB per pull and rejects empty or oversized chunks;
- rejects overflow, early EOF, extra bytes, and a midstream adapter error;
- incrementally recomputes full plaintext SHA-256; and
- calls `cancel` when an incomplete body is cancelled or dropped.

Upload tickets are non-cloneable and consumed by dispatch. Payload bytes are
never cloned by the coordinator. A filesystem or native adapter owns any
zero-copy implementation and must release its resource on consume, error,
cancel, or drop.

## Private authenticated spool

Instant Mode starts only when the spool adapter advertises authenticated
encryption, atomic replacement, and private permissions. The only accepted
capabilities are AES-256-GCM or XChaCha20-Poly1305; this enum is an adapter
claim, not proof that an OS keystore works. `PrivateButUnencrypted` and
`Unavailable` fail closed.

The core obtains one runtime-only key handle, enforces retained/reservation and
per-segment quotas, and streams plaintext into a reservation lease. The adapter
must write an authenticated temporary object and make it durable with one
atomic temp-to-final commit. Dropping an incomplete lease aborts and removes
the temporary object. A commit receipt must bind the segment index, identity,
length, ciphertext-integrity digest, and durable bit before the journal can
record it.

On Windows, temporary and lock files receive their protected current-user and
SYSTEM DACL in the same `CreateFileW(CREATE_NEW)` call that creates them.
Existing file and directory ACL repair opens the final object with
`FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS`, rejects a reparse
attribute from that handle, and calls `SetSecurityInfo` on the pinned handle.
Final publication pins and validates both the source file and destination
directory, then uses `SetFileInformationByHandle(FileRenameInfo)` with a simple
relative leaf and `ReplaceIfExists=false`; the exact source handle is flushed
before and after the rename. A pre-existing destination therefore loses the
atomic collision race instead of being replaced.

That boundary closes the former final-component metadata-check/name-use race;
it is not a claim that Win32 path traversal is safe under a hostile writable
ancestor. The configured spool root must be a caller-controlled local path
whose ancestors are not concurrently mutable by an untrusted or same-account
process. Root/session `create_dir_all`, later enumeration and later file opens
remain path based, and Windows exposes no documented unprivileged directory
fsync. Same-account path mutation and power-loss durability therefore remain
protected platform evidence rather than locally proven guarantees.

Recovery lists only committed entries with their full immutable descriptor and
commit receipt. Duplicate indexes, uncommitted entries, receipt mismatch,
invalid lengths, or aggregate quota overflow mark the spool corrupt. Opening a
recovered entry passes through the same length/SHA validator, detecting
truncation or modification. Missing key material never falls back to plaintext
or a new key. A durable file committed just before a journal crash becomes an
exact `JournalOrphanCommit` action; a physical eviction committed just before
its journal CAS becomes `JournalVerifiedEviction`. A missing unverified file is
corruption, never fabricated recovery.

Eviction requires an exact verified multipart part receipt. A stricter policy
can require the finalized object-manifest digest as well. Metadata is retained
in the journal after bytes are evicted. Cancel/delete seals the journal
tombstone first, then wipes the entire session spool; wipe failures remain a
retryable cleanup obligation.

## Durable journal and fencing

The journal snapshot begins at revision one with a nonzero fence. Every command
requires the exact current revision and fence and produces one next revision.
An ownership transfer is itself a journaled command and must increase the
fence. A claim records worker, fence, upload/probe work kind, attempt, and lease
deadline before a ticket can be minted. Dispatch re-reads the current snapshot;
a replaced worker, fence, attempt, work kind, lease, upload generation, or
segment cannot reach the provider.

Each command stores a bounded operation receipt containing operation ID, kind,
committed revision, domain-separated command digest, and outcome digest in the
same CAS write. Replaying the same operation returns that receipt. Reusing its
ID for another command is a conflict. If the adapter loses a successful CAS
acknowledgement, `DurableInstantJournal` reloads and accepts only the exact
stored receipt; absence is a stale write, never assumed success.

States are `Created`, online/offline capture, `Finalizing`, `Ready`, sealed
`Tombstoned`, and `RecoverableFailure`. The typed snapshot is the complete
restart artifact at every state boundary. No process-memory transition is
authoritative until its CAS succeeds.

## Multipart scheduling and reconnect

One segment maps to part number `index + 1` on the issue-19 multipart boundary.
Binding, part, and completion receipts bind the opaque session ID. Binding and
renewal receipts also include opaque upload ID, monotonic generation, expiry,
exact part-size limits, and maximum part count. Lost create/renew
acknowledgements are reconciled with upload inspection; a renewal must preserve
the session, upload ID, and limits while increasing generation and expiry.

The scheduler has nonzero bounded concurrency, attempt count, lease, initial
and maximum exponential backoff, and capped provider `Retry-After`. Online
eligible parts may upload out of order. Offline capture continues spooling and
does not claim network work. Offline, throttle, and outage results become
durable deferrals. Expiry requires a generation renewal and requeues every
unverified part. Expired worker leases stop counting against concurrency and
can be reclaimed with a higher attempt.

A successful part receipt must match upload ID/generation, part number, segment
identity, bytes, and SHA. Duplicate exact receipts are stable; different
receipts conflict. After a lost PUT acknowledgement the coordinator performs a
part inspection. If inspection is unavailable, the journal records
`ProbeRequired`; subsequent claims remain probe-only. No second PUT occurs
until inspection definitively returns `NotFound`. This distinction prevents an
unavailable HEAD from being interpreted as absence.

Multipart completion binds the deterministic manifest, ordered exact part
receipt digest, immutable object/version, and total bytes. Lost complete
acknowledgements are inspected before any retry.

## Finalize, publication, and callbacks

Stopping capture first requires every segment to have an exact verified part,
then freezes the deterministic manifest and expected D1/job generation. After
multipart completion is journaled, `SealFinalizeRequest` writes one exact
finalize request into the journal. Its digest binds session, committed journal
revision, fence, manifest, multipart/object identity, deterministic job ID, and
job generation. Later cleanup or ownership revisions do not alter that sealed
request.

The server finalize port reconciles lost acknowledgements by inspecting the
same request. Its internal D1 postcondition binds the playable master without
placing that storage identity on the wire. The first-party receipt is accepted
only when publication identity, request digest, job and generation, upload,
object version, and distribution eligibility match exactly. `Ready` stores one
publication identity. An exact duplicate complete or callback is stable;
another publication is a conflict.

That full request is the native `frame-media` journal contract, not a claim
about fields the control plane can independently authenticate. The v1 HTTP
wire DTO is intentionally narrower: tenant, session, retry operation, upload,
video, ordered-part digest, server-derived object-version digest, job and
generation, and the canonical request digest. It does not transmit journal
revision/fence, native manifest digest, or native object ID because those values
have no independent D1 authority and would only be echoed client assertions.
The DTO lives in `frame-authenticated-client`, not the anonymous public
`frame-client`, and its receipt never exposes an R2 or governed-object key.
The desktop HTTP transport maps the exact transportable identities from a
validated sealed native request and retains the omitted authority inside the
native journal. Its wire-only path is available to native desktop code without
importing `frame-media`; the journal projection bridge stays behind the
optional `instant-finalize` feature so a normal Tauri/Leptos build has no
GStreamer or GLib dependency.

The production Tauri surface registers a main-window-only, bounded,
deny-unknown-fields finalize command. WebView IPC contains only a native-minted
256-bit opaque handle and exact monotonic sequence. A native registry owns the
bearer credential and validated request, rechecks request digest and job
generation after the await, never holds its mutex across network I/O, seals and
drops credentials on terminal outcomes, and bounds retained contexts. Runtime
preflight rejects stale handles before network access; a committed result can
be reconciled across command-future cancellation without a second HTTP call.
Events and responses contain only the shared public-safe progress projection.

The release composition explicitly installs `NotConfigured`: it constructs no
provider, origin, HTTP client, credential, or request context, returns
`unavailable`, and leaves the accessible retry control disabled. A future
native authenticated capture/journal owner may register the already-tested
authority; neither bearer credentials nor native requests can be supplied by
the WebView. This closes the repository-local boundary and UI integration but
does not claim a real production desktop publication journey. The HTTP receipt
binds the retained upload and server-derived object version;
publication is allowed only after the immutable R2 verification receipt, exact
trusted probe, and exact D1 object/video/job postconditions agree.

Cancel/delete first writes `Tombstoned`, which rejects every late publish or
callback. Cleanup then attempts multipart abort, finalize-job cancellation, and
spool wipe independently. Lost abort/cancel acknowledgements are inspected.
Failures produce a retry-required report and cannot unseal the tombstone.

Managed Media versus native GStreamer derivative selection occurs only after
the finalized playable master exists. It consumes the same immutable master
and cannot change capture, upload, object, or publication state.

## Evidence boundary

The local suite proves state, arithmetic, ownership, redaction, hostile-fake
invariants, and the offline D1 server-finalize relational postcondition. It does
not prove that a native GStreamer graph produces playable
fMP4, an OS keystore encrypts files, atomic filesystem replacement survives
power loss, R2 implements the modeled multipart semantics, D1 commits the
modeled CAS, a browser begins playback within target, or production CPU,
memory, disk, and network budgets. Those remain protected promotion evidence.
