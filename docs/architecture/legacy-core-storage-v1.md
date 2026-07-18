# Legacy core storage v1

Frame retains twelve Cap routes for desktop downloads, playback, storage-object proxying, signed
uploads, multipart uploads, and recording completion. The source contract is pinned to
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`; the 27-source manifest digest is
`c673b6b50de8c2cb1ac7d94bb5dda52f13fcafb1ba7e0ab3a683e33f6d0e49f3`.

The authority boundary is D1 plus Cloudflare R2. Frame does not create a parallel S3 provider
adapter. A legacy video alias maps to a native video UUID, and
`legacy_mobile_cap_media_v1.object_prefix` remains the storage-key authority. This matters for
backfills: the first key segment can be a historical Cap user alias and must not be rewritten to
the current Frame actor UUID.

## Read transports

`GET /api/download` reproduces Cap's user-agent and client-platform selection and redirects to the
same-origin Apple Silicon, Apple Intel, Windows, or Linux download page.

`GET|HEAD /api/playlist` reasserts video visibility in D1 and reads only the selected tenant's R2
prefix. It supports MP4, MediaConvert, local HLS, raw preview, enhanced audio, transcription, and
desktop segment manifests. Generated segment playlists validate completion state, init markers,
bounded entries, finite durations, and exact zero-padded keys before signing one-hour GET URLs.
HEAD follows the same selection path and emits the same status and headers without a response body.

`GET|HEAD /api/storage/object` accepts either ordinary viewing authority or Cap's exact storage
token. Token verification uses HMAC-SHA256 with
`FRAME_LEGACY_CORE_STORAGE_NEXTAUTH_SECRET` and binds `videoId`, `key`, and `expiresAt`; malformed,
expired, mismatched, or excessively future-dated tokens are ignored. The key must remain beneath
the persisted video prefix. The R2 proxy supports one byte range and produces matching GET/HEAD
200 or 206 metadata. Multi-range and unsatisfiable requests return 416.

## Upload transports

All seven POST transports use Cap's API-key precedence with browser-session fallback, bounded exact
JSON, owner-scoped video authority, and an active R2 integration. A supplied `Idempotency-Key` is
hashed before persistence and binds replay to the exact request. Released Cap clients do not send
that header, so Frame generates a per-execution key while still recording an immutable operation.

Signed single and batch uploads issue exact R2 PUT capabilities. Every capability signs the object
key, approved metadata, content type, expiry, and `If-None-Match: *`. Cap's default `method=post`
cannot preserve the required no-overwrite invariant on R2; Frame therefore returns the PUT shape
used by the released desktop and CLI clients. D1 stores an immutable capability intent with the
tenant, mapped and legacy video identities, integration, role, requested method, and operation. An
intent is not proof that R2 received bytes.

Multipart initiation creates the provider session in R2, stores its provider upload ID only in D1,
and returns an opaque UUIDv7 external ID. Part signing rechecks actor, tenant, video, prefix,
subpath, state, and expiry, then binds the exact part number and optional MD5. Abort is two-phase:
the D1 operation and session first become provider-pending, R2 abort executes, and D1 records the
terminal state. A provider failure leaves the durable continuation pending; a retry resumes that
session-bound operation even after the upload's signing window expires and cannot create a second
claim. Completion retries likewise compare the exact request digest with the operation already
bound to the pending session.

Before multipart completion is admitted, Frame reasserts the active organization's owner-level Pro
seat. Free-plan raw recorder uploads must report a duration, and every reported duration above the
pinned five-minute limit plus its 30-second stop/finalize grace is rejected. This check runs before
the provider-pending completion transition, so later provider enablement cannot bypass Cap's plan
backstop. The rejected incomplete R2 session is aborted through the same journaled two-phase path
on a best-effort basis; cleanup failure never changes the policy response.

## Explicit provider gates

Multipart completion and desktop-segment recording completion have complete local orchestration,
but production remains fail-closed at `provider_execution`.

Multipart completion atomically stores ordered `(part number, ETag, bytes)` evidence, total bytes,
the parts digest, session binding, and an `effect_pending` operation. It does not call R2 complete
until independently reviewed provider-execution evidence covers retry reconciliation, object HEAD
verification, native governance promotion, and failure recovery.

Recording completion returns `{ "success": true, "status": "already-complete" }` immediately for
`desktopMP4`, matching Cap's already-complete branch. `desktopSegments` stores one immutable
`provider_pending` finalization intent and returns a
stable 503 provider gate. Frame does not claim that Cloudflare Media remux/transcode work, callback
delivery, output verification, or billing reconciliation happened.

## Durable state

Migration `0053_legacy_core_storage_expand.sql` adds:

- an immutable request/idempotency/result operation journal;
- external-to-provider multipart session bindings;
- append-only completion parts;
- no-overwrite object capability intents;
- provider-pending recording finalization intents; and
- transaction-local `changes()` postcondition assertions.

The SQLite proof exercises historical-prefix authority, tenant non-disclosure, replay immutability,
multipart completion and abort transitions, no-overwrite capability governance, metadata updates,
and the recording provider gate. Run it with:

```sh
python3 scripts/ci/legacy-core-storage-sqlite-conformance.py
```
