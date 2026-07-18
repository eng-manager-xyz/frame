# Legacy protected media contracts v1

Frame retains 41 source-pinned Cap operations whose final effects require
GStreamer, a media engine, an accelerator, Cloudflare media/storage services,
or another protected provider boundary. Twenty-five operations also cross a
storage, signing, AI, transcription, CDN, or workflow-provider boundary. This
contract closes the Rust/D1 admission and evidence model without claiming that
an unavailable executor or provider has run.

The source authority is Cap commit
`6ba69561ac86b8efdb17616d6727f9638015546b`. The machine-readable inventory is
`fixtures/api-parity/v1/protected-media-contracts.json`.

## Inventory and carriers

`frame_application::LEGACY_PROTECTED_MEDIA_PROFILES` is the exhaustive
inventory:

- seven web routes for recovery, thumbnails, preview, AI, transcription status,
  and AI retry;
- sixteen media-server health/audio/video routes;
- `VideosGetThumbnails` over Effect RPC;
- seven server actions for processing, finalization, and editing; and
- ten allowlisted child workflows for recovery, processing, editing, AI, and
  transcription.

The application boundary limits JSON to 256 KiB, enforces exact required
fields, bounds the RPC batch at 50 videos, and rejects unknown fields. It
canonicalizes request, payload, principal, execution, and authority digests.
Secret-bearing URLs, upload/object keys, edit documents, webhook values, and
provider controls are replaced by sealed descriptors. Plaintext is accepted
only by the narrow `ProtectedMediaRequestVaultV1`; the production-unavailable
adapter fails closed.

Released carriers authenticate before every stage and replay:

- browser routes bind the exact live D1 session id, token version, token digest,
  actor, and active tenant;
- scheduler and media-service routes bind the exact provisioned secret subject,
  key version, and digest;
- thumbnail, preview, and thumbnail RPC reads bind either that exact session or
  a current video/space/public capability;
- public media health/status reads use rate-limited edge capabilities, while job
  reads bind the current D1 job id and revision; and
- all server actions consume a browser mutation grant after owner or
  organization-scope authorization.

No released protected-media carrier reads a caller `Idempotency-Key` header.
Reads and naturally keyed mutations use a server-generated natural replay
claim; server actions use the consumed grant id; workflows use their immutable
parent receipt.

## Composite video and AI authority

Authentication and video policy are separate bindings. Every target video has
one ordered proof:

- `owner_bypass`;
- `video_password`;
- `space_password`; or
- `unprotected_video_policy`.

The D1 live-authority view re-evaluates the exact session/service/capability and
all proofs. Owners bypass passwords. Organization or space members still need
a current matching password when any video/space password exists. Public
viewers must satisfy the exact address/domain restriction and password policy.
Password revision, membership, placement, target, session, service-secret, or
job drift removes the receipt from the live projection.

`/api/video/ai` binds current view policy plus the video's owner's exact AI
entitlement. `/api/videos/:videoId/retry-ai` additionally requires owner
policy. Entitlement state, revision, subject, and expiry are rechecked for
stage, replay, and evidence.

## D1 intent, replay, and workflow parents

Migration `0061_legacy_protected_media_expand.sql` stores no raw request,
provider payload, credential, password hash, or terminal body. Its protected
records are:

1. `legacy_protected_media_receipts_v1`, the immutable operation, exact
   credential/policy/entitlement, request descriptor, parent, terminal kind,
   and executor binding;
2. `legacy_protected_media_execution_outbox_v1`, the sanitized executor
   descriptor staged atomically with the receipt;
3. `legacy_protected_media_generated_replay_claims_v1`, the atomic natural
   replay owner;
4. provisioned executors plus bounded executor leases; and
5. `legacy_protected_media_execution_evidence_v1`, the independent execution
   and optional provider evidence with a typed sealed terminal reference.

The shared `legacy_protected_effect_parent_registry_v1` and
`legacy_protected_effect_parent_edges_v1` connect protected-media and
protected-integration workflows without a migration-order foreign-key cycle.
A scheduler calls `dispatch_legacy_protected_media_workflow_v1` with only the
child operation, parent family/receipt/request digest, payload, and time. The
carrier reloads the allowlisted edge and exact actor, tenant, credential,
ordered policy proofs, entitlement, creation time, and authority digest from
D1; it never accepts those authority fields from the scheduler. The immutable
parent receipt identity is also the workflow replay key.
A child stores its own operation-scoped authority digest and a separate exact
`parent_authority_binding_digest`. Edges are source-pinned and declare either
`same` or `child_derived` target binding. Anonymous integration parents use
a deterministic media-only parent capability tied to family, receipt,
creation time, parent authority digest, and live policy; no user credential is
invented. The cross-family video-status edge translates the parent's native
video UUID to its unique Cap video alias before constructing child policy
proofs, and rejects a payload whose target does not match that alias.

Natural claims coalesce pending work across generation boundaries. Terminal
claims can roll only after the 15-minute sealed-terminal retention window.
Replay, evidence, and workflow launch all join the same live-authority
projection and current non-dead-letter parent.

## Evidence and terminal delivery

An evidence insert must match an active independently provisioned executor and
lease, request digest, sanitized outbox digest, authority digest, executor kind,
terminal kind, and provider requirement. It atomically consumes the lease and
marks the outbox and receipt verified. Immutable triggers reject later mutation
or deletion.

D1 stores only `sealed_terminal_ref`, plaintext digest, terminal kind, and
expiry. `ProtectedMediaTerminalV1` validates JSON, redirect, binary, and event
stream invariants after a narrow resolver opens that reference. The
production-unavailable resolver fails closed. Signed R2 URLs, cookies, headers,
provider JSON, and media bytes therefore never enter D1 receipts or logs.

Cloudflare R2 is the object authority described by ADR 0002; GStreamer/provider
executors receive only the sanitized, digest-bound descriptor after admission.
Neither D1 nor this migration is an object store or a substitute for the
Cloudflare media package.

Secret-bearing workflow payloads pass through the same request-vault boundary
as HTTP, RPC, and action payloads. Until a production vault implementation is
configured, that boundary deliberately returns `Unavailable` before any D1
receipt or outbox row is written.

## Fail-closed behavior

Staging is not success. Pending HTTP carriers return
`503 EXECUTION_EVIDENCE_REQUIRED` with `Cache-Control: no-store`, a bounded
retry hint, and an opaque receipt id. RPC/action/workflow callers receive
`LegacyProtectedMediaStageOutcomeV1::ExecutionEvidenceRequired`. Verified
delivery still requires live authority, a live parent when applicable, and an
unexpired sealed terminal.

## Evidence

Run:

```sh
python3 -I scripts/ci/legacy-protected-media-sqlite-conformance.py
python3 scripts/ci/check-migrations.py
cargo test --locked -p frame-application --lib legacy_protected_media
cargo test --locked -p frame-control-plane --lib legacy_protected_media
```

The SQLite proof loads migrations 0001 through 0061 and verifies all 41 pinned
source contracts, exact owner/member/public-password policy, email
restrictions, revision and placement races, AI entitlement drift, same-family
and cross-family parents, anonymous parent capability, independent evidence,
atomic terminal finalization, forbidden raw columns, and foreign-key integrity.
