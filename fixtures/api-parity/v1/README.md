# API parity v1 fixtures

`route-workflow-report.json` is the machine-readable Issue 30 inventory derived from the ignored,
pinned `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b` checkout. It contains no
customer data, credentials, request bodies, or upstream source code. Each source identity is a path,
symbol, and SHA-256 only.

Each row also has an exact completion decision. The schema distinguishes unfinished local adapter
or retirement-response work from overlapping provider, hardware, or human-approval gates and
records the production fail-closed behavior; the current v1 corpus has zero unfinished local rows.
The checker rejects any attempt to use a protected gate to erase pending repository-local work.

`operation-contract-catalog.json` is the normalized specification backlog for those same 288
identities. Seventy-six source-manifest-bound profiles describe the required success-or-retirement,
validation, authorization, idempotency/retry, and stable-failure cases without duplicating policy
text in every row. Its role is explicitly `specification_only_not_endpoint_execution_evidence`:
the checker will not let a profile promote an adapter, clear remaining local work, or manufacture a
retirement approval. The route report remains the authority for what has actually passed.

`contract-cases.json` freezes Frame's closed public errors, provider-neutral media statuses, and
current/N-1 compatibility decisions. Rust decodes and validates it in
`crates/domain/tests/api_workflow_contract_v1.rs`.

`frame-application::LegacyCompatibilityRegistryV1` also decodes the complete report in its focused
suite. With explicit synthetic fallback availability, it proves every unpromoted row chooses
fallback, exercises shared admission and the atomic execution-port boundary for every retained
row, and covers current/previous decisions for all release-managed client associations. This
registry evidence does not change a row's endpoint success state; per-operation business adapters,
released-client binaries, providers, and approved retirements remain separate gates.

The control-plane's fail-closed transport constructs the registry and implements that port with a
digest-only D1 claim/intent/completion/audit journal. Local SQLite conformance proves its atomic
and immutable SQL behavior. Its durable semantic-adapter allowlist is empty and production
fallback availability is false. A typed semantic allowlist promotes only the source-pinned
`cap-v1-05b6ba3f76daac22` `GET /api/status`, `cap-v1-ff19008f47194c43`
`GET /media-server`, `cap-v1-a1b180c5d123c870` `GET /api/changelog/status`, and
`cap-v1-16668b858461f386` `OPTIONS /api/changelog/status`, plus
`cap-v1-0fa8384f3666825b` `GET /api/changelog` and `cap-v1-237f41f3086a2d67`
`OPTIONS /api/changelog` contracts, `cap-v1-4f21920a947c4c84`
`GET /api/mobile/session/config`, the actor-bound D1 notification-list and preference
contracts (`cap-v1-14dcca6d36eee6b3` and `cap-v1-d130c840f654bd72`), and the desktop
custom-domain read (`cap-v1-ed9957ac480103b9`). These ten semantic
adapters carry all five per-operation evidence axes and exact path/method/query/request/response/header
tests. All production ingresses enforce their reported bucket through the bounded, keyed-digest D1
authority in migration `0034_compatibility_rate_limits.sql`; both notification reads also require
D1 business data. SQLite conformance proves exact fixed-window saturation/reset,
bucket and subject separation, bounded cleanup, regressed-clock rejection, and fail-closed behavior
when the authority is absent. The exact
`cap-v1-7773d3e70d1d5919` theme ACTION is also production enabled. The exact
active-organization, folder-assignment, library-placement, notification-write, developer, and
membership families add 21 locally complete ACTION contracts behind the explicit
`released_legacy_client_e2e` gate. The user/account, collaboration, video-property, library,
upload/storage, analytics, organization, media, integration, and billing/auth families extend that
exact boundary. In all, 111 server actions, 136 routes, 15 RPCs, and 14 workflows have exact local
success: 276 of 288 rows. Every row's repository-local work is complete. One hundred twenty-nine
contracts are production-authorized; 159 remain deliberately fail-closed behind explicit provider,
hardware, released-client, or human-approval evidence. The 12 rows without endpoint-success
evidence are locally specified retirement/declaration decisions whose accountable approval remains
protected; no external result or fallback is fabricated.

`theme-action.json` separates Frame's authenticated HTTP selector from the frozen abstract
`server_action`/`ACTION` identity. It pins the exact two-value input, Next response-cookie
serialization, last-write-wins/no-client-idempotency semantics, five-source closure, and empty
no-store 204 response.

`folder-assignment-actions.json` binds the three frozen abstract ACTION identities to their bounded
Frame selector, exact header/body idempotency contract, actor/tenant authorization split, normalized
storage postconditions, atomic browser-proof/audit/effect journal, replay/conflict/race behavior,
source-closure counts, and explicit released-client production gate.

`library-placement-actions.json` binds four organization/space root-placement ACTION identities to
their action-specific request fields and response objects. It freezes manager authority, the
actor-owned-video versus matching-share asymmetry, normalized root mutations, atomic one-use proof
and idempotency receipts, source-closure counts, and the released-client production gate.

`notification-actions.json` binds the mark-read and preference-write ACTION identities to one
authenticated Frame carrier. It preserves missing-versus-present optional fields, freezes exact
notification counts/read times and preference sibling-JSON preservation, requires atomic
idempotency/audit/proof consumption, returns an empty no-store 204, and keeps released-client E2E
promotion as an explicit protected gate.

`notification-read.json` binds the authenticated `GET /api/notifications` identity to its exact
actor/active-organization D1 scope, unread-first ordering, union-row fault isolation,
anonymous-view count folding/defaults, ISO date projection, source-shaped response/failure bodies,
and provider-tolerant `avatar: null` fallback.

`org-custom-domain.json` keeps two similar-looking contracts distinct. It binds the concrete
desktop GET to the literal 36-character API-key/session selector, mounted CORS policy,
actor-derived active-organization projection, case-sensitive URL normalization, independent nulls,
and ISO timestamp response. The separate `/api/org-custom-domain` ts-rest/Effect declaration stays
unpromoted because no pinned handler defines its declared boolean verification value or failures.

`video-domain-info.json` binds the concrete anonymous video-domain lookup to its full D1 source
closure. It corrects the earlier inferred session requirement, preserves first-shared-then-owner
fallback without inventing ordering, and retains the actual `domainVerified` ISO-string-or-false
wire union backed by the lossless custom-domain projection.

`desktop-session.json` binds the desktop sign-in handoff to the exact browser-session and
digest-only D1 authorities. It freezes the token/API-key parameter shapes, absolute login restart,
loopback versus deep-link routing, no-store hybrid fallback page, desktop CORS, and the deliberate
`1..=65535` port tightening that closes Cap's unconstrained loopback redirect/HTML input.

`desktop-compatibility.json` binds six retained desktop organization, branding, personal-storage,
profile, upload-progress, and video-delete routes to source-exact session/API-key authentication
and mounted CORS. It freezes the released clients' absent idempotency header, durable optional-key
replay, tenant-scoped D1 projections, one-MiB logo validation, and resumable RECORDINGS R2 deletion.

`declaration-only-dispositions.json` closes the repository-local investigation for the web
custom-domain and commercial-license declarations without fabricating handlers. It pins both
schemas, the commercial desktop callers, every missing authority decision, fail-closed production
behavior, and the repository-owner implementation-or-retirement approval that remains protected.

`developer-actions.json` binds all eight user-owned developer-dashboard ACTION identities to one
authenticated Frame carrier. It freezes nullable-logo and optional auto-top-up presence, exact
zero-row delete semantics, owner authority, atomic mutation/audit/proof/idempotency journals, and
the one-time credential response shapes. API keys are hashed and encrypted at rest, replay material
is locally AEAD-sealed and request-bound, every key-bearing debug view is redacted, and missing or
invalid local key authority fails closed. Production promotion remains behind released-client E2E.

`developer-api.json` binds the public-key recorder SDK, secret-key REST API, and daily storage cron
to eleven canonical route identities. It freezes production Origin enforcement, exact CORS and
error envelopes, source pagination/status projections, optional client idempotency for released
SDK compatibility, header-free R2 multipart SigV4 URLs, duration-and-size completion billing, and
once-per-UTC-day storage debits. D1 claims provider effects before R2, complete and abort resume
from an outbox, and credit/snapshot journals are atomic and append-only. The fixture also records
the corrected canonical IDs for the lookalike aliases in the original issue text.

`membership-actions.json` binds invite removal, single-member addition, bulk addition, batch and
single removal, and creator-inclusive space membership replacement to one authenticated Frame
carrier. It preserves missing-field defaults, present-empty-members behavior, duplicate and
unmatched bulk IDs, exact legacy response ordering, and creator protection while reasserting
active-tenant manager authority, organization membership, creator admin status,
authority-generation changes, and mutation-grant revocation in the same D1 transaction as the
receipt, audit, invalidation, and browser proof. The original three actions retain their explicit
released-client E2E gate; bulk add, batch remove, and single remove have no protected gate and are
production-enabled.

`folder-crud.json` binds the exact mobile `POST /api/mobile/folders` carrier and the three
`POST /api/erpc` Effect-RPC folder operations to one scoped atomic D1 port. It freezes mobile
session/API-key authentication and optional replay keys, the unbatched Effect `Exit` wire format,
raw branded folder IDs, update `Option` presence, typed parent/cycle failures, recursive-delete
reparenting, source manifests, and rollback evidence. Mobile create is production-enabled. The RPC
operations are locally complete but remain fail-closed behind human approval because Cap permits
same-organization cross-namespace parent edges that Frame deliberately rejects.

`collaboration-actions.json` binds the three mobile comment routes, the legacy web DELETE route,
and both comment ACTIONs to one source-pinned application contract and atomic D1 port. It freezes
the mobile session-or-36-character-API-key boundary, JavaScript trim and ISO projections, the web
route's missing-versus-empty query behavior, direct-reply deletion asymmetry, caller-parent
notification selection, whitespace/orphan creation behavior, best-effort create notifications,
transactional delete notification cleanup, bounded 100,000-row deletes, and replay evidence. All
six operations have five-axis local evidence and no protected production gate.

`user-account.json` binds the exact user-name route, two tagged Effect-RPC calls, two account
actions, and three development-only actions to one principal-derived application contract. It
freezes missing/null/empty name semantics, the `Exit` wire envelopes, ignored `UserUpdate.id`,
image Option layers, onboarding defaults, owner-or-any-membership account authority, and atomic
all-device credential revocation. The action carrier requires a matching idempotency key and a
one-use browser mutation proof consumed in the D1 transaction. Name, account patch, and sign-out
are production-enabled; image RPCs retain their R2 provider gate and devtools retain human approval.

`video-properties.json` binds ten deliberately asymmetric mobile, route, and browser-action
mutations to one atomic D1 port. It preserves ECMAScript versus raw whitespace handling, truthy
metadata replacement, JavaScript object spread, settings normalization, PBKDF2-HMAC-SHA256 wire
material, anonymous video/space password verification order, encrypted bounded password cookies,
and immutable replay evidence. Three mobile mutations retain their provider-execution gate and
edit-date retains human approval; the other six operations are production-enabled.

`library-id-reads.json` binds folder, organization, and space video-ID ACTIONs to exact source
success/failure envelopes and unordered projections. The D1 implementation reasserts the actor's
active tenant and folder/space authority, deliberately closing the pinned source's cross-tenant
lookup weakness without inventing ordering. All three operations are production-enabled.

`library-detail-reads.json` binds `getUserVideos` and `searchDashboardVideos` to their distinct
source projections. It freezes effective-date and prefix-rank ordering, ECMAScript query
normalization and LIKE escaping, nullable metadata/duration/owner fields, folder decoration,
comment/reaction counts, and screenshot-aware upload presence. The D1 adapter preserves search
visibility while restricting the source's owner-only user-video query to the actor's live active
tenant. Both operations are production-enabled without a protected provider gate.

`extension-auth.json` binds the four Chrome-extension consent, approval, revocation, and bootstrap
routes to Cap's exact 16-file handler/client closure. It freezes the side-effect-free consent GET,
Chromium redirect pin, same-origin form POST, UUID fragment handoff, literal authorization-token
precedence, actor-owned revoke, deterministic organization fallback/repair, and the hosted versus
self-hosted Pro branch. Frame returns the UUID once but stores only its digest, atomically rejects
an eleventh hourly mint, and requires an active pointer to be actor-owned before fallback. All four
routes are production-enabled without provider or hardware gates.

`extension-instant-recordings.json` binds the extension create, progress, and delete routes to the
exact 25-file domain/handler/storage/client closure. It preserves the 15-character Cap wire ID via
an immutable alias over a native UUID, emits a 900-second no-overwrite R2 PUT target, selects a
verified custom share domain when available, and keeps API-key precedence over session auth.
Progress retains the source timestamp fence and tightens it with monotonic byte/total checks.
Delete uses a durable D1 tombstone around strongly consistent actor-prefix R2 cleanup, preserving
the alias for audit and retry convergence. All three routes are production-enabled without an
external provider gate because Cloudflare R2 is Frame's in-scope storage authority.

`mobile-session.json` binds Cap's four mobile email challenge, verification, session-key, and
revocation routes to one source-pinned application contract and atomic D1 adapter. It freezes
ECMAScript whitespace and validation, destructive one-use challenges, hidden pending-invite user
provisioning, Cap NanoID-to-Frame UUID aliases, replace-all mobile keys, the middleware/handler
Bearer parsing asymmetry, and strict `cap`/`exp+cap` redirects. Session-key mint and revoke are
production-enabled. Email delivery and new-user verification fail closed behind provider execution;
the checked fixture and local test acknowledgement do not claim production email or Stripe success.

`mobile-bootstrap-caps.json` binds Cap's bootstrap, paginated cap list, cap detail, download,
playback, and delete routes to an owner-scoped D1 projection and Cloudflare R2 authority. It freezes
session-or-literal-36-character-API-key authentication, JavaScript pagination coercion, effective
creation ordering, nullable best-effort images/transcripts, source-specific playback and download
key selection, one-hour host-only signed GET URLs that remain compatible with Range requests, and
database-before-storage delete ordering. Foreign and already-deleted caps are indistinguishable;
all six routes reject request bodies and client idempotency keys and are production-enabled.

`mobile-uploads.json` binds the released mobile create, progress, and completion lifecycle to exact
session-or-literal-36-character-API-key authentication, actor/organization/folder authority,
collision-safe Cap aliases, and a no-overwrite Cloudflare R2 PUT capability. Progress preserves
Cap's truncation/nonnegative/total-clamp rules while rejecting unsafe JavaScript integers. Complete
requires the exact server-minted key and a nonempty matching R2 object, then records one immutable
provider intent without fabricating a media job, workflow receipt, or processing phase. Create and
progress are production-enabled; completion returns `503 provider_execution` until independently
admitted workflow-submission evidence exists.

`space-authorization.json` binds `getSpaceAccess` and `requireSpaceManager` to Cap's exact role
normalization, access projection, null result, and two manager error messages. The browser carrier
removes caller control of `userId`, reasserts the session actor's live active tenant, and scopes the
immutable legacy space alias before every D1 projection. Owner and creator IDs are returned only
through persisted global aliases; imported membership IDs are promoted exactly and native IDs use
a deterministic, bounded collision-retry path. Both read-only operations reject idempotency keys
and are production-enabled without a protected gate.

`messenger-retirement.json` freezes one tenant-opaque, non-retryable 410 response for all ten
messenger/support ACTION identities. The implementation and checker complete the repository-local
response work while preserving repository-owner approval as a protected gate; no production
retirement route is enabled and the quarantined messenger tables remain unavailable to product
reads or writes.

`video-lifecycle.json`, `core-storage.json`, and `upload-storage.json` bind the remaining video,
playlist, object, multipart, signed-upload, direct-upload, and edit-reconciliation carriers to exact
D1/R2 state transitions. They preserve method-specific GET/HEAD/RPC/action/workflow envelopes,
bounded bodies, immutable replay receipts, no-overwrite capabilities, and resumable storage effects.
Provider-backed completion paths retain their execution gates while locally owned D1/R2 paths are
enabled.

`analytics.json` binds seven HTTP, Effect-RPC, and action surfaces to optional viewer/share authority,
tenant-safe aggregates, atomic signup compare-and-swap, and immutable provider query/event intents.
Six Tinybird or notification effects remain provider-gated; the local signup mutation is enabled.

`organization-library.json` binds 21 provider-free organization, collection, branding, storage,
membership, and space actions to exact D1/R2 effects. It includes public PBKDF2 collection-password
verification without journaling plaintext, one-use browser grants, resumable prefix deletion, and
locally projected Google authorization state without claiming a Google network result.

`protected-media-contracts.json` binds 41 media routes, RPCs, actions, and internal workflows to
source-pinned validation plus atomic receipt/outbox staging. Sixteen require independently verified
hardware evidence and 25 require both hardware and provider evidence; every pending or corrupt state
returns `EXECUTION_EVIDENCE_REQUIRED` instead of a media success.

`protected-integrations.json` binds 45 provider-only desktop, Loom, organization, release, webhook,
mobile, action, RPC, and workflow carriers. Source-exact public, session/API-key, signed-state,
signed-webhook, password-proof, and parent-receipt branches are preserved. A server-owned request
vault keeps the entire provider plaintext outside D1, generated or natural replay remains reachable,
legacy IDs are resolved through native aliases, and only a digest-bound sealed terminal admitted by
immutable `independent_provider_executor` evidence may become a legacy response.

`protected-billing-auth.json` closes the final 17 NextAuth, checkout, billing, Stripe webhook,
administrator action, and reprocessing-workflow contracts. Two require provider evidence, 14 require
both accountable human approval and provider evidence, and the source-pinned developer-checkout
preflight is an exact local credentialed-CORS response. Stripe verification is raw-body and
timestamp bound, approvals precede provider execution, and no staged intent implies payment,
subscription, upload, cache-invalidation, or processing success.

`changelog-feed.json` is the exact 88,817-byte `JSON.stringify` response derived from the pinned 99
numeric MDX inputs. The generator verifies every input hash, a deterministic source-manifest digest,
and the response digest before writing or accepting the fixture.

Regenerate only from the exact pinned checkout:

```sh
python3 scripts/ci/check-api-workflow-parity.py --generate --require-reference
```

Normal CI runs the checker offline. If the reference or scope changes, create a new versioned corpus
instead of silently rewriting v1.
