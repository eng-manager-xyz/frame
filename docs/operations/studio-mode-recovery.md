# Studio Mode recovery runbook

This runbook applies to Studio v1 journals. Never edit a journal, original media
file, or receipt by hand. Preserve the project directory before investigating a
codec or checksum failure.

## Initial triage

1. Read the bounded journal through `StudioDocumentCodec::decode_journal`.
2. Stop if the envelope checksum, schema, project ID, or invariants fail. Copy
   the directory for investigation; do not guess a state transition.
3. Acquire ownership through journal CAS. Confirm both the expected revision and
   fence, then increment the fence. Callbacks bearing the prior fence are stale.
4. Verify that the pending asset/edit/render identity is byte-for-byte equal to
   the identity permitted at the prior boundary. Never repair an identity drift
   by choosing whichever file happens to exist.
5. Open every journal through `DurableStudioJournal`, consume each pending
   render into a `RenderJournalAuthorization`, and supply all recovered
   authorizations to `StudioRenderCoordinator::new` before dispatching any new
   export. A fresh dispatch must bind its exact `RenderPrepared` authorization
   into `AuthorizedRenderDispatch`; there is no unauthorised `start` overload.
6. Select the exact action from the table below.

## Boundary matrix

| Last durable boundary | Recovery action |
| --- | --- |
| `Created`, `RecordingGraphPrepared` | delete only unstarted temporary files; no original exists |
| `CaptureStarted` | reopen the exact graph with `FilesystemStudioRecordingSession::recover`, then ask the native bridge to resume each partial or seal all isolated tracks independently |
| `TempAssetReserved` | delete the named uncommitted temporary asset |
| `TempAssetDurable`, `AssetCommitRequested` | probe the exact ID/name/track/timing/checksum/length identity, then atomically commit or reconcile the original |
| `AssetCommitted` | retain the immutable original and continue recording |
| `RecordingStopped`, `EditSaveCommitted` | open the editor at the durable revision |
| `EditSavePrepared` | compare the pending edit digest with the persisted project; commit exactly once or retain the prior revision |
| `RenderPrepared`, `RenderRunning`, `RenderFinalizing`, `RenderCancelled` | match the full render-spec identity, cancel if still active, delete the exact partial output, probe for absence, then open the editor |
| `RenderCommitted` | match the full render-spec identity and verify the final output checksum/length and receipt, then open the editor |
| `FailedRecoverably` | preserve artifacts and the exact pending identity; require an explicit operator/user decision |

A generic exit from `FailedRecoverably` is valid only when there is no pending
identity (or when a durable original has already been proven). Pending assets,
edits, and renders resolve only through their exact `AssetCommitted`,
`EditSaveCommitted`, `RenderCommitted`, or `RenderCancelled` boundary. The
journal rejects an exit that drops or substitutes the pending value.

At no point is a temporary file renamed over an existing original. The commit
adapter must return the exact durable metadata or the core rejects it.

An interrupted recording seal may contain a mixture of
`recording-partials/<asset>.recording-partial` and
`temporary/<asset>.media`. Reopen only with the original four-track graph and
byte ceiling. Recovery rehashes every retained file, appends only to partials,
and refuses to mutate a track already sealed into the temporary namespace.
Dropping the session preserves these files; deletion requires the separate
journal recovery decision for that exact asset identity.

## Lost acknowledgements

- Journal CAS: reload and require the operation receipt, command digest, outcome
  digest, boundary, and the exact pending asset/edit/render value.
- Temporary asset commit: probe the opaque original ID and require every stored
  field to match the temporary descriptor with only its state changed to
  `DurableOriginal`.
- Render start: probe export ID. Accept only the exact fence and full render-spec
  digest (source set, plan, profile, and output). A matching partial is deleted;
  an absent or mismatched result is ambiguous and is not blindly retried.
  `Absent` after a lost acknowledgement is not terminal proof because renderer
  publication can be delayed. Probe, absence, or cleanup uncertainty retains
  the output reservation until an exact fenced cancel, exact partial cleanup,
  and a second absence probe all succeed.
- Render commit: require the exact callback identity, 100% progress, output
  checksum/length, and renderer committed postcondition before writing a receipt
  that repeats the immutable fence, source-set, plan, render-spec, profile, and
  output identities. CAS-persist that receipt inside `PendingRender` before
  marking the reservation releasable.

## Cancellation and hardware failure

Cancellation is valid during preparing, decoding, compositing, encoding, muxing,
and finalizing. Send cancel with the active fence and deadline, delete only the
partial matching the active fence, render-spec digest, and output name, then
probe until it is absent or report cleanup failure.

After restart, use `reconcile_recovered_cleanup` for a recovered running or
partial render. A matching committed output instead uses
`adopt_recovered_commit(export_id)`. It has no checksum/length arguments and
uses only renderer postconditions and the structured terminal receipt in
journal v2. If the renderer committed in the crash window before the prior
coordinator wrote that receipt, adoption first CAS-persists the full receipt and
only then installs a releasable session. Both paths retain the project/output
reservation until an explicit terminal release; any mismatch remains
quarantined for investigation. Recovered cleanup always issues the exact fenced
cancel and cleanup before its final absence probe, even when its first probe was
`Absent`.

The production filesystem ports use per-project or per-asset create-new locks,
same-directory temporary files, file sync, atomic rename, and directory sync.
Do not move a temporary across filesystems, delete a lock from a live process,
or replace an existing original. Project edit storage also persists the maximum
claimed journal fence, so a superseded process cannot save merely because its
old ticket still matches its old store instance. A leftover lock or an original
lacking its canonical sidecar requires inspection; recovery may finish the
sidecar only after verifying the exact expected original identity.

On a hardware encoder failure:

1. Verify the failed session was actually using hardware and reported the
   hardware-failure class.
2. Delete and verify absence of the hardware partial.
3. Recompile the render graph with the software backend while preserving the
   exact source-set digest, canonical edit-plan digest, profile, output name,
   fence, and full render-spec digest.
4. Use new export and operation IDs. Transfer the output reservation only after
   the failed partial is confirmed absent. If software capability is
   unavailable, fail the export safely and leave the project/editor intact.

## Rollback

Studio v1 rollout is read-only legacy inspection first, new recording opt-in
second, and editing/export last. Rollback disables creation of new v1 projects
but retains:

- v1 decoder and recovery support;
- immutable original assets and their checksums;
- the legacy Cap source directory; and
- access to the legacy editor during the published rollback window.

Do not down-convert a v1 project in place. Export or copy into a separate target.
