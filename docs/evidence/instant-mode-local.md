# Instant Mode local evidence

Status: provider-free local contract, filesystem-encryption, and native
GStreamer segmentation evidence. This record does not claim physical-capture
behavior, live OS-credential-store behavior, R2 multipart behavior, D1/job
behavior, browser playback, wall-clock performance, or production completion.

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

## Remaining repository integration gap

The exact finalize state machine exists, but its D1 transaction, multipart
postcondition, immutable object-manifest, and retained-job ports are not yet
wired into one control-plane adapter. That deliverable remains a local gap; it
is not being reclassified as protected provider evidence.

The missing call path is explicit:

```text
desktop Instant journal
  -> InstantFinalizePort::reconcile / inspect
  -> [no concrete HTTP client or wire DTO]
  -> [no control-plane Instant finalize route]
  -> [no D1 Instant request/operation/publication authority table]
  -> r2_multipart_completions_v1 + media job + object manifest + playable result
  -> [no atomic reconciliation adapter]
```

`InstantFinalizeRequest` and its opaque identities are currently internal Rust values. The bounded
`InstantJournalCodec` can persist the full local journal inside `frame-media`, but it is not a
versioned client/server finalize protocol, and the control-plane cannot import that crate because
`frame-media` carries the native GStreamer dependency while the control-plane targets Wasm. The
desktop crate also does not depend on `frame-media`, so no production caller reaches the finalize
trait. Control-plane media completion does reconcile generic media-job and object-manifest state,
but it neither accepts an Instant request digest/job generation/multipart receipt nor returns an
`InstantFinalizeReceipt`. The fake `InstantFinalizePort` tests therefore prove the state-machine
semantics only; they do not satisfy issue 26's server-finalize deliverable.

## Reproduction commands

Run from the repository root:

```bash
cargo test -p frame-media --test instant_mode_contract
FRAME_NATIVE_H264_AAC_APPROVED=approved-v1 \
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media \
  native_execution::tests::approved_instant_profile_writes_recoverable_fmp4_segments
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media --all-targets
cargo clippy -p frame-media --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p frame-media --no-deps
cargo fmt --all -- --check
git diff --check
```

Machine-specific paths and raw logs are intentionally absent.

## Protected evidence not collected

The following remains open and cannot be inferred from Rust fakes or simulated
time:

- physical-capture splitmux keyframe placement and cross-track alignment,
  power-loss recovery while the muxer is active, Media ingest, and browser
  playback before and after manifest replacement;
- macOS, Windows, and Linux private-directory and keystore behavior, actual
  AES-GCM/XChaCha protection, key loss, atomic replacement under power loss,
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

Until the remaining adapter wiring and protected records exist, this slice is
suitable for integration and local conformance only. It is not a production
promotion record and does not close the protected acceptance gates in issue 26.
