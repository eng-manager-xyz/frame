# Legacy upload/storage compatibility v1

Frame retains nine Cap operations at commit `6ba69561ac86b8efdb17616d6727f9638015546b`: three Effect RPCs, four upload/download actions, the stale-edit reconciliation workflow, and `shareCap`. Their source identities remain RPC, action, and workflow identities; `/api/v1/web/compatibility-actions/:operation_id` is only Frame's authenticated browser selector.

All nine operations are provider-free in the parity catalog. D1 mutations and ordinary R2 reads, writes, listings, and deletes execute locally. No provider-success placeholder or protected gate is used.

## Authority model

- Cap NanoIDs are resolved through the existing user, organization, space, and video alias tables. Native UUIDs never cross the compatibility wire.
- RPC reads reproduce Cap's public policy: the owner bypasses view gates; other callers need an explicit Cap organization/space share membership or a public video. Non-owners must satisfy any video/space password with the sealed verified-password cookie, and public fallback also enforces the organization's allowed-email restriction.
- Upload creation requires live membership in the requested organization and an optional live folder in that organization. Reusing an existing video is owner-only. The free-plan duration fence is evaluated before capability issuance.
- `downloadVideo` is source-corrected to required owner session. `getVideoDownloadInfo` uses its distinct source permission helper and admits the owner, original-organization members, explicitly shared-organization members, and explicitly shared-space members without incorrectly applying the RPC public-view gate.
- Native `shared_videos` is intentionally same-tenant and native `space_videos` is a Frame placement authority. Migration 0057 therefore imports those rows into Cap-specific organization/space share projections; `shareCap` atomically replaces the projections without weakening native invariants.

## Storage behavior

The create action returns a Cloudflare R2 exact-key immutable PUT target (`presignedPostData: null`) in place of Cap's S3 POST union. The target binds content type, Cap metadata, expiry, and `If-None-Match: *`. Its original response is stored in the D1 operation receipt so an idempotency retry never mints a second capability.

Download RPC selection is source-exact:

1. A screenshot lists the owner/video prefix, sorts supported suffixes newest-first, and signs the selected object.
2. A `webMP4` uses a nonempty `result.mp4`, then falls back to the progress row's `rawFileKey`.
3. A `desktopMP4` signs `result.mp4` without a HEAD requirement.
4. Segment/local/MediaConvert sources return Effect `Option.None`.

`deleteVideoResultFile` deletes the Cap progress projection and records `storage_pending` in one D1 transaction. R2 deletion retries the one exact `result.mp4` key up to three times. Failure leaves a resumable intent; only a successful R2 delete moves the operation and intent to `complete`.

## Progress and reconciliation

Migration 0057 adds the missing `started_at_ms` field to the existing one-row-per-video Cap progress projection. `VideoUploadProgressUpdate` stores `min(uploaded,total)` and updates only when the incoming timestamp is not older. A stale update still returns Effect success `true`, matching Cap.

The reconciliation workflow deletes only a snapshot that still has the exact `owner/video/source/original.mp4` raw key, phase, timestamp, and progress value observed by the runtime. The strict source thresholds are:

- `error`: immediately;
- `complete` or `generating_thumbnail`: older than five minutes;
- `processing` at zero percent: older than fifteen minutes;
- other `processing`: older than ten minutes;
- `uploading`: never.

## Replay and evidence

Mutation receipts bind source operation ID, actor, video, idempotency digest, and request digest. Key reuse with different input conflicts. Create/share/progress/reconcile complete atomically with their D1 effects. Delete has the sole `storage_pending -> complete` continuation. Evidence and capability rows are immutable; the share projections are intentionally mutable only through an atomic replacement batch.

The canonical machine-readable contract is [upload-storage.json](../../fixtures/api-parity/v1/upload-storage.json). SQLite coverage is in `scripts/ci/legacy-upload-storage-sqlite-conformance.py`.
