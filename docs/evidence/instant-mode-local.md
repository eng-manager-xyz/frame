# Instant Mode local evidence

Status: local contract, filesystem-encryption, native GStreamer segmentation,
and offline D1 reconciliation evidence. This record does not claim physical-capture behavior,
live OS-credential-store behavior, hosted R2/D1/job behavior, browser
playback, wall-clock performance, or production completion.

## Implemented contract surface

- versioned, opaque, redacted Instant session/operation/worker/upload/object/job
  and publication identities;
- immutable fMP4 segment identity over index, exact time, keyframe boundary,
  aligned track metadata, byte length, full SHA-256, and domain-separated
  identity digest;
- deterministic gap/overlap-free manifest identity;
- fail-closed provider-neutral splitmux/fMP4/H.264/AAC graph specification and
  an explicitly enabled real GStreamer path that writes, syncs, hashes, and
  recovers deterministic fMP4 segments;
- bounded single-owner streaming bodies with empty/oversize/overflow/early-EOF,
  extra-byte, checksum, cancel, and drop enforcement;
- authenticated encrypted private-spool capability and production filesystem
  adapter with OS-credential-store key handles, private directories, bounded
  chunked encryption, reservation/quota accounting, atomic rename and sync,
  crash listing, corrupt read detection, durability-gated eviction, and
  session wipe;
- monotonic revision/fence CAS journal, exact command/outcome operation
  receipts, ambiguous-commit reload, leased upload/probe claims, and restartable
  state at every durable boundary, plus a bounded canonical binary codec with
  version/tag/state validation and a corruption checksum;
- issue-19-shaped session-bound multipart binding/renewal, ordered part mapping,
  offline queue, bounded concurrency, retry/backoff/throttle/expiry, lost-ack
  probes, exact duplicate receipts, and complete inspection;
- durably sealed server-finalize request binding manifest, multipart object,
  journal/fence, deterministic job, and generation;
- exactly one publication/master, duplicate callback stability, derivative
  selection after publish, terminal tombstone, abort/job-cancel/spool-wipe
  reconciliation, and late-resurrection rejection; and
- stable redacted desktop/share progress plus bounded time-to-share arithmetic.

## Hostile local matrix

The external `instant_mode_contract` suite covers:

| Boundary | Hostile scenarios proved locally |
| --- | --- |
| Identity | SHA-256 known vector; deterministic insertion-independent manifest; cross-session, gap, track misalignment, and non-keyframe rejection |
| Pipeline | exact fMP4 node family; missing force-keyframe and audio-alignment capabilities fail closed; an explicit codec-enabled run produces multiple recoverable fMP4 segments with `ftyp` plus `moov`/`moof` evidence |
| Payload | bounded chunks; empty, truncated, extra, wrong-hash, successful EOF, and incomplete-drop cancellation |
| Spool | production filesystem encryption with an injected keystore; atomic commit/restart/stream; plaintext-absence, tamper, missing-key, quota, disk-full, write-abort, corrupt recovery, exact remote-proof eviction, orphan-commit, and physical-eviction crash-window reconciliation |
| Journal | operation replay/collision; committed-but-lost CAS acknowledgement; close/reopen after each tested transition; concurrent owner/fence rejection; canonical codec round-trip plus version, truncation, trailing-byte, and checksum rejection |
| Upload | offline/reconnect, throttle, outage, provider expiry, renewal generation, bounded concurrency, expired worker lease, exact duplicate and corrupt receipt, and out-of-order verification |
| Lost acknowledgement | create/renew inspection; one PUT followed by repeated probe-only unavailability; part postcondition; multipart complete postcondition; finalize postcondition |
| Finalize | missing-part rejection; exact object/manifest/job generation; pending, duplicate, unavailable, and stale generation; request preserved across later journal/fence revision |
| Publication | exact duplicate callback; second publication conflict; managed/native post-finalize policy; tombstone rejects late callback |
| Cleanup | confirmed upload abort, finalize-job cancel, spool wipe; independent retry report when all three fail |
| Resources/UI | bounded spool/inflight settings, deterministic throughput/time-to-share simulation, progress counts/basis points, public stable code, and debug redaction |

The focused contract suite contains 26 tests. The native execution suite adds
an explicitly enabled real segmentation test, and the filesystem spool module
adds restart/tamper/quota tests. The sanitized full `frame-media` run is the
authoritative aggregate count. Provider transitions still use hostile local
adapters and do not prove Cloudflare execution.

## Control-plane server-finalize integration

The exact server-finalize slice is wired on the Worker side through a versioned Wasm-safe DTO,
authenticated route, retained D1 request/operation/job rows, multipart/probe postconditions, and
scheduled reconciliation. The desktop side below is a tested adapter boundary, not a currently
registered Tauri command:

```text
sealed desktop Instant request (contract fixture)
  -> frame-desktop-core conversion to frame-authenticated-client InstantFinalizeRequestV1
     (canonical digest; retry operation excluded from semantic identity)
  -> POST /api/v1/instant-recordings/{session_id}/finalize
  -> retained instant_finalize_requests/jobs/operations rows
  -> exact r2_multipart_completions_v1 + verified native probe postconditions
  -> one D1 batch: upload + immutable object authorities + ready video + publication/job/operations
  -> 200 published or stable 202 pending, with scheduled reconciliation
```

The server derives the immutable object version from the provider version, never accepts client
media-probe facts, and validates the exact tenant/session/upload/video, ordered parts, object
version, job, generation, and request digest on every replay. The HTTP contract deliberately omits
the native journal revision/fence, manifest digest, and native object ID because D1 has no
independent authority for those values. They remain covered by the `frame-media` journal and are
not copied into the DTO merely to echo client assertions. The wire receipt also omits the internal
playable storage key; D1 keeps that identity behind its relational publication assertion. The
desktop adapter can map the exact
session/upload/ordered-part/object-version/job identities from the sealed native request, adds
tenant/video and a typed retry operation, and validates the bound receipt before reconstructing
the native publication receipt. A missing multipart completion or trusted probe remains `pending`; an
identity mismatch fails closed. The HTTP idempotency key, semantic request, operation, and retained
job are reserved in one authority-fenced batch.

The offline SQLite conformance test proves tenant and immutable-row triggers, revoked-writer and
deleted-video rollback, contingent-row rollback, bounded fair scanning and dead-letter assertions,
retryable multipart-abort retention, and the final relational publication postcondition. This
locally satisfies the server-finalize/D1-reconciliation deliverable. The concrete desktop caller
disables redirects and ambient proxies, bounds deadline and response bytes, sends bearer, tenant,
and deterministic idempotency headers, validates the shared DTO, and is covered by a real loopback
HTTP test. It is not yet connected to `apps/desktop/src/main.rs`, and the repository therefore does
not cite this test as a production desktop publication path. The bounded journal remains inside
`frame-media` because the control plane cannot import its native GStreamer dependency. Hosted D1
contention, R2 execution, callback ordering, and a real command-to-journal desktop journey remain
protected evidence.

## Reproduction commands

Run from the repository root:

```bash
cargo test -p frame-authenticated-client
cargo clippy -p frame-authenticated-client --all-targets -- -D warnings
cargo check -p frame-authenticated-client --target wasm32-unknown-unknown
cargo test -p frame-media --test instant_mode_contract
cargo test -p frame-desktop-core --features instant-finalize
cargo clippy -p frame-desktop-core --features instant-finalize --all-targets -- -D warnings
# On Windows, with DOCS_RS=1 in the environment:
cargo test --locked -p frame-windows-secure-spool
cargo clippy --locked -p frame-windows-secure-spool --all-targets -- -D warnings
cargo check --locked -p frame-media --all-targets
FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  native_execution::tests::approved_instant_profile_writes_recoverable_fmp4_segments
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
cargo clippy -p frame-media --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p frame-media --no-deps
cargo fmt --all -- --check
python3 -I scripts/ci/instant-finalize-sqlite-conformance.py
python3 scripts/ci/check-migrations.py
git diff --check
```

Machine-specific paths and raw logs are intentionally absent.

The Windows local implementation now applies creation DACLs atomically and
performs ACL repair and final no-replace publication through validated pinned
handles. This is local source/type evidence for the final-component boundary,
not evidence against hostile writable ancestors, same-account concurrent path
mutation, filesystem-specific rename behavior, or power loss. Those conditions
stay in the protected platform lane below.

## Protected evidence not collected

The following remains open and cannot be inferred from Rust fakes or simulated
time:

- physical-capture splitmux keyframe placement and cross-track alignment,
  power-loss recovery while the muxer is active, Media ingest, and browser
  playback before and after manifest replacement;
- macOS, Windows, and Linux private-directory and live keystore behavior, key
  loss, atomic replacement under power loss,
  disk-full behavior, secure wipe policy, and privacy review;
- real R2 multipart create/part/HEAD/renew/complete/abort, CORS, expiry,
  throttling, outage, conditional postconditions, and lost-response injection;
- real D1 CAS contention, operation receipts, finalize job generations,
  callback ordering, object/job reconciliation, tombstone durability, and
  cleanup retries;
- end-to-end desktop crash/kill/restart at every state boundary while native
  capture continues, including sleep/wake and stale desktop segment repair;
- actual time to first playback/share and continued playback after final
  manifest replacement;
- wall-clock disk growth, upload bandwidth, reconnect catch-up, CPU, memory,
  queue depth, encoder latency, thermal behavior, and long-recording limits
  against the approved Cap baseline;
- production quota/cost, alert, incident-game-day, kill-switch, rollback, data
  retention/deletion, accessibility, privacy, security, product, media, and
  release-owner signoff.

Until the protected records exist, this slice is suitable for integration and
local conformance only. It is not a production promotion record and does not
close the protected acceptance gates in issue 26.
