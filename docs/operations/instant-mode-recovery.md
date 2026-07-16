# Instant Mode recovery runbook

Use only privacy-safe session state, revision/fence buckets, segment counts,
byte totals, retry class, and stable error codes. Never log opaque IDs, object
keys, provider receipts, checksums, key handles, file paths, media bytes, source
labels, or exact user timing.

## Startup and crash recovery

1. Acquire a new journal owner fence with an atomic journal command. Do not
   reuse a remembered process fence.
2. Decode the complete journal with `InstantJournalCodec`. Reject a bad magic,
   unknown version/tag, truncation, trailing bytes, checksum mismatch, size
   overflow, missing operation receipt, or impossible state; do not synthesize
   defaults. The codec checksum is corruption detection, not storage
   authentication, so retain the CAS store's own access controls.
3. Acquire the existing runtime spool key handle. Key unavailable is a stable
   recovery error. Never generate a replacement key or open media as plaintext.
4. Recover committed spool entries and compare every index, immutable identity,
   and byte length to the journal. Remove adapter-owned incomplete temporary
   reservations; never journal them as segments.
5. Reconcile in-flight claims. A live lease remains owned by its worker. An
   expired lease may be claimed with the current fence and next attempt. A
   `ProbeRequired` part remains probe-only.
6. Reconcile the current multipart upload ID/generation and any sealed finalize
   request before issuing provider mutations.

Repeat these steps after a crash at any command boundary. A state not durably
present in the CAS journal did not happen from the coordinator's perspective.

## Network loss and reconnect

- Move capture to `CapturingOffline`; continue atomic local commits while quota
  remains. No upload claim is minted offline.
- Surface `LocallyRecoverable`, retained-byte total, and the stable offline
  code. Do not estimate share readiness from unverified network state.
- Warn before quota exhaustion. When reservation fails, stop accepting new
  segment bytes and preserve all already committed artifacts.
- On reconnect, inspect the current upload generation and expiry, then resume
  only eligible claims under the configured concurrency bound.
- Honor capped `Retry-After`. Repeated throttle/outage uses bounded exponential
  backoff and cannot block the capture callback or spool writer.

## Lost acknowledgement

- Journal CAS: reload and require the exact operation receipt. If absent, treat
  the local snapshot as stale and recompute from the loaded state.
- Multipart create/renew: inspect the session upload. Accept only an exact
  session-bound binding or a valid monotonic renewal for that same session.
- Part PUT: inspect the exact part. A matching receipt verifies it. `NotFound`
  permits a new PUT with a fresh body. Timeout, outage, throttle, or another
  ambiguous result stays `ProbeRequired`; do not re-upload.
- Multipart complete: inspect completion and validate manifest, ordered part
  digest, object version, and bytes.
- Server finalize: inspect the exact sealed request digest and D1/job
  generation. Pending is not failure and is not publication.
- Abort/job cancel: inspect the terminal postcondition and keep cleanup queued
  until confirmed.

## Upload expiry

1. Stop dispatching the expired generation.
2. Renew the same opaque upload ID with a strictly greater generation and
   expiry and unchanged part limits.
3. Journal that binding before issuing another claim.
4. Requeue every unverified segment. Existing exact verified receipts remain
   immutable only when the issue-19 adapter confirms renewal preserves them;
   otherwise reconcile each part before finalize.
5. Reject delayed receipts from the retired generation.

## Spool full, unavailable, or corrupt

- `SecureSpoolUnavailable` or `SpoolKeyUnavailable`: do not begin/resume
  Instant Mode. Offer an explicit non-Instant recording path only if its own
  storage contract is satisfied.
- `SpoolDiskFull`/quota exceeded: stop new reservations, retain the journal and
  committed files, and continue uploading already committed segments when
  possible. Never evict unverified bytes to make space.
- Corrupt/truncated entry or checksum mismatch: quarantine the session from
  finalize and surface `RecordingRecoveryRequired`. Do not upload modified
  bytes under the old identity.
- Temporary-file cleanup failure: retry adapter cleanup without creating a
  journal segment.
- Eviction: require the exact part durability proof, or the final object
  manifest when that tenant policy is selected. Preserve descriptor metadata.

## Finalize repair

1. Require exact indexes from zero, continuous time, at least one segment, and
   exact verified receipts for every part.
2. Complete multipart and journal its object manifest.
3. Seal the finalize request before calling the server. Never reconstruct a
   different request from a later journal revision.
4. Reconcile pending/lost results by sealed request digest. A stale job
   generation requires an authority repair, not a local increment.
5. Accept one publication identity and playable distribution master. A
   different complete/callback is an incident and remains unpublished.
6. Select managed/native derivative work only after `Ready`; derivative choice
   never rewrites the source object or upload journal.

## Cancel and delete

Write the journal tombstone first. Once sealed, every late part-complete,
finalize, and callback path must fail to publish. Then independently:

1. abort the live multipart upload and inspect a lost acknowledgement;
2. cancel the D1/finalize job and inspect a lost acknowledgement; and
3. wipe the authenticated local session spool.

Retry any unconfirmed branch. Do not remove the tombstone, reuse the session
ID, or mark cleanup complete because another branch succeeded. A delete after
`Ready` retains the immutable publication identity in audit state while
preventing resurrection.

## Rollout, kill switches, and escalation

Roll out only to internal/test tenants with separate switches for Instant
capture, live upload, and finalize publication. The live-upload kill switch
must leave local recording/journal recovery available while quota permits. The
finalize kill switch must leave the sealed request pending, not publish a
partial object. Rollback may select the legacy mode only for a new recording;
never move an in-progress v2 session into a legacy journal.

Escalate on repeated CAS conflict from one fence, receipt conflict, checksum or
spool-integrity failure, second publication identity, tombstone resurrection
attempt, non-monotonic upload/job generation, or cleanup that remains
unconfirmed beyond policy. Preserve redacted state-transition codes and
provider request correlation held in the provider system; do not attach media
or secrets to the incident.
