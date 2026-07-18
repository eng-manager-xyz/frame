# API and workflow parity contract v1

Issue 30 replaces hidden TypeScript authority with versioned Rust contracts while preserving a
safe route-level fallback until a client-specific E2E gate passes. The behavioral reference is
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`; the target public major is
`frame.api.v1` under `/api/v1`.

This document describes the local contract, not a production-parity attestation. The generated
[parity report](../generated/api-workflow-parity-v1.md) deliberately keeps adapter, protected
provider, and retirement approvals pending.

## Exhaustive inventory boundary

`scripts/ci/check-api-workflow-parity.py` extracts and checksum-pins:

- direct Next route methods, mounted Hono routes, Effect `HttpApiEndpoint` definitions, and the
  supported ts-rest desktop/public contract snapshot;
- every exported action in a file-level `use server` module and inline actions carrying their own
  `use server` directive;
- Effect RPC operations, exported durable workflow implementations, the Effect Loom workflow,
  and the non-workflow-directory dispatch/recovery entrypoints that own durable side effects.

Declaration-only pins are insufficient for operations whose behavior lives behind a shared
transport. Every Mobile row therefore also pins the concrete catch-all handler, and every Effect
RPC row pins the `/api/erpc` transport, RPC/auth layer, family handler, and called service/policy
files. Organization actions in the audited migration slice additionally pin imported authorization,
role, plan, normalization, and membership helpers when those helpers determine semantics. The
checker enforces this source closure offline and verifies every digest against the pinned Cap clone
when it is available.

Provider classification is also identity-driven. Thirty-one additional rows pin only the minimal
directly effect-bearing Cap sources for Google Drive, Vercel, Stripe, Resend, Groq, OAuth, Discord,
Tinybird, storage/media, Dub, Deepgram, and GitHub behavior. Those pins add
`provider_execution` to completion without feeding source-path words back into route-family or
disposition classification. `POST /api/commercial/activate` (`cap-v1-700b21489623a3e4`) and web
`GET /api/org-custom-domain` (`cap-v1-9323d0178c5a63b5`) are guarded by a separate invariant:
contract declarations alone are dependency evidence, not executable authority. Their exhaustive
repository audit is complete, production remains fail-closed, and a repository owner must approve
a concrete implementation contract or retirement; neither is mislabeled as provider execution.

Catch-all Next files are recorded as transport wrappers rather than duplicate product operations.
Ordinary helpers, UI routes, and native desktop IPC are explicitly excluded. When the ignored
pinned Cap checkout exists, the checker re-extracts all rows and compares source checksums and the
entire classified report. Offline CI still validates the committed schema, identities, evidence
axes, generated documentation, dispositions, and retirement records.

Each row records legacy path or operation, methods, symbols and source hashes, client families,
auth class, policy version, `frame.api.v1`, issue owners, Rust authority, disposition, body/rate/
idempotency policy, deprecation state, and five independent evidence axes: success, validation,
authorization, idempotency/retry, and failure. A separate per-row completion decision records the
remaining repository-local adapter or retirement-response work, any overlapping protected gate,
the pending retirement authority, and the production fail-closed behavior. Protected evidence
never erases unfinished local work.

Executable source wins over path-based auth inference. For example,
`GET /api/video/domain-info` contains no session lookup and is therefore registered as anonymous;
its D1 adapter also preserves the source's ISO-timestamp-or-false verification value and explicit
404 disclosure instead of silently applying the family defaults.

## Central admission and errors

`frame-domain::api_workflow` and `frame-application::api_workflow` define the common boundary.
Adapters must construct an `ApiRequestPolicyV1` and pass mutation metadata through
`ApiGatewayV1::admit_mutation` before reading a body or invoking an authority.

The policy enforces:

- an 8 MiB absolute API body ceiling, lower route-specific limits, and exact media-type allowlists;
- required, optional, or forbidden tenant-scoped idempotency keys;
- named rate-limit and audit buckets containing only safe tokens;
- authentication before authorization, exact browser-origin/CSRF decisions, and rate-limit
  outcomes with bounded retry delays;
- one `ApiErrorV1` schema with closed codes and no provider, object key, signed URL, token, email,
  cookie, raw body, or private title fields.

After authentication, an unknown identifier and a known-but-forbidden tenant identifier both
produce `not_found`/404. Adapters must not distinguish them by body, headers, timing class, cache
policy, or telemetry labels.

Correlation IDs are opaque safe tokens and are redacted by `Debug`. Traces and audit rows use the
policy's static action/bucket labels; raw path parameters and request bodies are not labels.

Promoted compatibility operations now enforce those labels through migration
`0034_compatibility_rate_limits.sql` instead of supplying a synthetic `Allowed` decision. The
fixed window is 60 seconds: `service_misc.v1` allows 120 requests per keyed source,
`client_compatibility.v1` allows 12 (including the 88,817-byte feed),
`organization_library.v1` allows 12 per authenticated principal, and
`collaboration_notifications.v1` allows 30 per principal. Cap's developer API freezes a 60-request
window for both `developer_api.v1` and its multipart `upload_storage.v1` calls. Public subject keys are HMAC digests of
Cloudflare's canonical client address; authenticated subject keys are HMAC digests of the trusted
principal. Both reuse the protected auth hash-key rotation under a distinct domain separator, and
neither raw value is stored. A missing keyring, D1 binding, migration, or successful postcondition
returns service-unavailable; a full bucket reaches the typed compatibility admission and returns
the stable `rate_limited`/429 contract with `Retry-After: 60`. Each request deletes at most 16 expired rows and the schema
caps total cardinality, so abuse cannot turn admission into unbounded D1 work.

## Provider-neutral derivatives

`DerivativeRequestV1` accepts only a profile name, immutable source version, and idempotency key.
There is no executor field. `PublicMediaStatusV1` exposes the stable states `queued`, `running`,
`indeterminate`, `succeeded`, `failed`, and `cancelled`; outputs contain governed roles and public
Frame paths rather than bucket keys or signed provider URLs.

Quota, outage, timeout, invalid input, output rejection, and cancellation are closed public failure
classes. Cloudflare eligibility, kill switches, native fallback, provider attempts, and binding
responses remain private execution facts owned by issues 28–29. A provider timeout with an
unknown outcome is `indeterminate`, not a fabricated percentage or failure. Reconciliation uses
the original provider idempotency key before any further state transition.

## Webhooks and secrets

`WebhookVerifierV1` signs `decimal_timestamp + "." + raw_body` with HMAC-SHA-256 and accepts only
the exact lowercase `v1=<64 hex>` form. Verification is constant-time across active keys. It fails
closed for:

- bodies over 1 MiB;
- timestamps outside the configured window (maximum 15 minutes);
- malformed, unknown, expired, or not-yet-active keys;
- a body changed by even one byte;
- duplicate replay digests or replay-store unavailability.

`WebhookKeyRingV1` permits at most four uniquely named, bounded-lifetime secrets, supporting an
overlap window for rotation. Secret values never implement serialization and are redacted from
`Debug`. `D1WebhookReplayStoreV1` implements the production-shaped atomic insert-if-absent contract
over migration `0015_api_workflow_replay.sql`, with bounded expired-row cleanup. The in-memory
adapter is for local tests only; provider callback routes still must wire the D1 adapter before any
business effect.

Provider-specific header parsing happens outside this verifier, but must normalize into this exact
timestamp/signature/body contract before any business effect. A verified event then uses the
provider event ID as the business idempotency key.

## Durable workflow and outbox rules

`DurableWorkflowV1` is the shared lease/fence model for scheduled jobs, callbacks, imports, media
coordination, notification outbox delivery, and other retryable effects:

1. A claim increments both attempt and monotonically increasing fence, and grants a bounded lease.
2. Checkpoints compare the current fence and advance exactly one step.
3. An expired lease can be reclaimed; the old holder can never checkpoint or complete.
4. `plan_provider_submission` stores one stable provider idempotency key before the network effect
   and returns either `Submit` or `ReconcileExisting`; the latter must never resubmit.
5. A rejected result can terminate. An indeterminate result enters `waiting_for_provider` and is
   not claimable by the ordinary executor; a reconciliation query must confirm or reject it.
6. Retry delays and attempt counts are bounded. Terminal success, failure, and cancellation are
   not claimable.

Cancellation is rejected while a provider effect is submitted, confirmed, or indeterminate; the
adapter must obtain a provider cancellation/absence fact before exposing a terminal cancellation.

Persistence adapters must compare state revision/fence in the same transaction that writes a
checkpoint, terminal result, or outbox receipt. Crash recovery may repeat a read or provider query,
but never a billable mutation under a different key. Poison messages remain quarantined with a
redacted failure class and cannot block unrelated tenants.

## Legacy client compatibility and deprecation

The API supports the current and immediately previous released client for the current API major.
Unknown majors and older releases receive `upgrade_required`; unknown enum values never start work.
`LegacyRoutePolicyV1` adds the per-route strangle gate:

- `ServeFrameV1` requires endpoint-level contract evidence and the client-family flag;
- otherwise a supported client uses the legacy fallback while it remains available;
- a retirement response requires repository-owner approval and a documented migration path;
- removing fallback without evidence/approval is an invalid contract state.

Deprecation has no implicit date. The generated row must name an earliest removal, migration path,
and approval before a retirement can be promoted. Current inventory retirement proposals therefore
remain pending and cannot turn an accidental 404 into policy.

The v1 evidence set currently contains 276 local-success rows: 136 routes, 15 Effect RPC
operations, 111 server actions, and 14 workflows. Every one of the 288 rows has complete
repository-local work. The production registry enables 129 ungated contracts and fails closed for
159 rows with released-client, hardware, provider, or accountable-human evidence gates. Of those,
147 already have exact local carrier/business contracts; the remaining 12 are locally specified
declaration or retirement decisions that cannot be promoted without repository-owner approval.
Exact
`GET /api/status`
(`cap-v1-05b6ba3f76daac22`) returns `200`, `text/plain;charset=UTF-8`, body `OK`. Exact
`GET /media-server` (`cap-v1-ff19008f47194c43`) returns the compact Hono JSON metadata document
from the pinned Cap `apps/media-server/src/app.ts`, including its ordered endpoint list, with
`application/json`. The production Wrangler declaration includes the query-safe
`frame.engmanager.xyz/media-server*` compatibility fence; raw routing admits
only exact `/media-server`, preserves the same response with a query, and
returns a no-store 404 for suffix lookalikes. Exact `GET /api/changelog/status` (`cap-v1-a1b180c5d123c870`) preserves the
pinned `URLSearchParams.get("version")` behavior against changelog `99.mdx` version `0.5.6`, returns
compact `{"hasUpdate":true|false}` JSON, and emits the source-defined wildcard CORS headers. Its
exact `OPTIONS` operation (`cap-v1-16668b858461f386`) returns an empty `204` with the same three
CORS headers and no content type. The changelog GET registration pins the route, changelog loader,
and latest content source hashes and regeneration verifies that `99.mdx` remains the highest
numeric slug. Exact `GET /api/changelog` (`cap-v1-0fa8384f3666825b`) reproduces the complete
99-entry feed as the pinned 88,817-byte `JSON.stringify` body. Its source manifest covers every
numeric MDX file, and its route, loader, CORS utility, body, and manifest digests are independently
locked. Its `OPTIONS` companion (`cap-v1-237f41f3086a2d67`) preserves the empty `204`; both methods
preserve the request-origin/configured-origin reflection and `null` fallback from `getCorsHeaders`.
Exact `GET /api/mobile/session/config` (`cap-v1-4f21920a947c4c84`) pins both
`packages/web-domain/src/Mobile.ts` and the `getAuthConfig` handler in
`apps/web/app/api/mobile/[...route]/route.ts`. It remains public (`public_or_flow_token`), accepts
no request body or idempotency key, and returns exact compact JSON for all four combinations of
`googleAuthAvailable` and `workosAuthAvailable`. The values come only from non-empty
Worker `GOOGLE_CLIENT_ID` and `WORKOS_CLIENT_ID` bindings; binding values never enter the request,
response, or fingerprint. Cap's pinned `@effect/platform` 0.92.1 transport defines default JSON
encoding as `application/json` and passes that encoding content type to its JSON response builder,
so the adapter preserves that exact header rather than inferring it from current Effect behavior.
Exact session-authenticated `GET /api/notifications/preferences`
(`cap-v1-d130c840f654bd72`) is the D1 read. Its source closure pins the standalone route,
`getCurrentUser`, NextAuth session callback, `users.preferences` schema, API middleware exclusion,
Next runtime configuration, package declaration, and Next 16.2.1 lock resolution. Frame verifies
only its host-only browser session cookie and browser client kind; it does not add API-key,
organization, Origin, CORS, CSRF, or user-status route policy. The actor-only D1 query reads
`users.preferences_json`, defaults a missing/null/schema-invalid notifications object as a whole,
defaults only an omitted `pauseAnonViews` field within an otherwise valid object, strips unknown
fields, and returns the exact compact five-boolean JSON order. Missing or invalid sessions return
the pinned `401` body `{"error":"Unauthorized"}`. Preference query, row decoding, and serialization
failures return the pinned `500` body `{"error":"Failed to fetch user preferences"}`; session/auth
repository and configuration failures remain outside that source handler's caught preference-query
failure path.

Exact session-authenticated `GET /api/notifications` (`cap-v1-14dcca6d36eee6b3`) is the second D1
read. Its source closure pins the route, response union, session lookup, notification/user schema,
and image-resolution boundary. Frame scopes the query to the recipient and that actor's active
organization, preserves unread-first/created-desc ordering, omits malformed union rows and authored
rows with missing users without failing valid siblings, folds anonymous-view counts into `view`,
and always emits all four count keys. Missing or failed avatar resolution yields source-compatible
`avatar: null`; it does not fail the list. The route preserves the exact compact response and
source-shaped `401`/`500` failures, and applies the principal-scoped collaboration-notifications
rate limit before reading business data.

All nine typed semantic adapters run through the bounded typed-adapter registry. The seven static
response adapters have no D1 business-data dependency, while every production ingress requires the
shared D1 rate-limit authority and both notification reads additionally require their real D1
business authorities. The ingress representation owns and bounds the raw body, canonical
application/x-www-form-urlencoded query multimap (including ordered duplicates), normalized
allowlisted headers, exact matched path parameters, authenticated principal with an optional tenant, and
optional resource revision. Responses likewise own bounded body bytes and headers. Each adapter
derives its request fingerprint from only its canonical semantic inputs; transport callers cannot
supply a fingerprint. Static and future D1 business-authority adapters share this typed boundary,
but the synthetic compatibility journal is not a business authority, is absent from the promotion
registry, and retains an empty durable allowlist. Its presence therefore cannot promote a report
row or authorize a real effect. The independent semantic-adapter registrations pin operation ID, method, path,
source hashes, client family, exact auth policy, empty-body/forbidden-idempotency policy, response
digest, retry behavior, authorization failures, rate-limit failures, query variants, and
hostile-path negatives. The durable-adapter allowlist remains empty; no other report row inherits
this evidence by route family or implementation authority.

The Navbar `updateActiveOrganization` server action
(`cap-v1-a3b4c805d409bc7c`) resolves only as `server_action`/`ACTION` with its frozen
`action://...#updateActiveOrganization` identity; it never enters raw HTTP resolution. Its typed
session boundary derives the actor from the trusted authenticated principal, deterministically maps
the Cap NanoID, and calls a D1 business adapter whose atomic batch updates only the active
organization, preserves the default, derives the revision server-side, and journals
`active_organization_set`. The authenticated same-origin compatibility-action ingress consumes its
one-use browser proof in that same D1 batch and returns a no-store void response after the internal
`/dashboard` invalidation effect. Its typed browser client accepts a Cap NanoID, but the Leptos
dashboard picker intentionally remains on Frame's native UUID/revision mutation and exposes no Cap
NanoID projection. A released legacy-client E2E journey is therefore still a protected gate; the
callable ingress is not promotion-authorized for compatibility cutover, and the production
registry returns the stable unavailable response for this operation.

The otherwise-unused `setTheme` server action
(`cap-v1-7773d3e70d1d5919`) has an abstract `server_action`/`ACTION` identity distinct from the
Frame HTTP selector, accepts only `light` or `dark`, forbids client idempotency, and produces the
exact `theme={value}; Path=/` response cookie with void/no-store completion. The Frame hydration
toggle calls this ingress and reapplies only an exact `light`/`dark` cookie during bootstrap. The
pinned Cap `Contexts.tsx` source describes separate JS-cookie persistence behavior and is not a
caller of the server action.

Cap's three exact folder-assignment actions are locally proven:
`addVideosToFolder` (`cap-v1-f5daa7be337a2979`), `removeVideosFromFolder`
(`cap-v1-1af3645bf2ae7168`), and `moveVideoToFolder` (`cap-v1-eaf277e644aa4b92`). Their abstract
`server_action`/`ACTION` identities remain distinct from the bounded same-origin Frame selector.
The selector requires a one-use browser mutation proof, an exact body/header idempotency-key match,
the trusted active tenant, and the `organization_library.v1` limiter. Add/remove fail the whole
canonical list unless every tenant video is actor-owned; move instead permits a tenant video only
when the actor is a manager of the selected direct, organization, personal-root, or exact-space
context. The D1 boundary reasserts live actor, organization, membership, folder, space, video,
assignment, and revision snapshots in the same batch as the normalized mutation, typed storage
postcondition, tenant/actor/action-scoped receipt, business audit, validated cache effects, and
browser-grant consumption. Same-key replay returns the original receipt, conflicting reuse fails,
and a two-contender race yields one apply and one replay. The typed browser client clears cached
workspace state before every valid send and decodes only Cap's exact add/remove object or move-void
success. These contracts remain production fail-closed behind `released_legacy_client_e2e`; the
local D1 and client proofs do not claim a released Cap journey.

The four organization/space library-placement actions use the same bounded selector and
`organization_library.v1` admission. Their typed inputs and success projections remain
action-specific. Organization placement requires actor-owned videos, while space placement keeps
the source-observed matching-share authorization asymmetry. Migration
`0037_legacy_library_placement_expand.sql` and its D1 adapter bind live tenant/membership/space/video
authority, normalized root storage, exact postconditions, one-use proof consumption, audit,
invalidation, and idempotent receipt into one transaction. All four are locally complete and remain
production fail-closed behind released-client E2E.

The two notification-write actions share the schema
`frame.web-notification-action-request.v1` and the `collaboration_notifications.v1` bucket. Their
decoder preserves missing versus present optional fields; mark-read applies the exact count and
read-time transition, while preference writes preserve unrelated preference JSON. Migration
`0038_legacy_notification_actions_expand.sql` makes each mutation, authority assertion, proof
consumption, receipt, audit, and invalidation atomic. Both return the source-compatible empty
no-store `204` and remain gated from production.

The eight developer-dashboard actions are principal-owned and therefore use trusted principal-only
session context rather than an active organization. The schema
`frame.web-developer-action-request.v1` preserves nullable-logo and optional auto-top-up presence,
zero-row delete success, normalized domains, and the one-time create/regenerate key response shapes.
Migration `0039_legacy_developer_actions_expand.sql` provides the atomic D1 authority. API keys are
hashed and encrypted at rest; replayable credential material is locally AEAD-sealed and
request-bound; secret-bearing debug output is redacted; and absent or invalid secret authority
fails closed. All eight remain behind released-client E2E.

The public developer SDK, secret-key REST API, and storage cron are separately bound to their
eleven canonical route identities. Public `cpk_` credentials require an exact configured Origin
for production apps; `csk_` credentials remain server-only; the cron requires a constant-time
`CRON_SECRET` Bearer comparison. Migration `0054_legacy_developer_api_expand.sql` adds immutable
operation receipts, multipart sessions, R2 provider outbox state, append-only credit transactions,
and daily storage snapshots. Released SDKs omit idempotency headers, so mutations accept a durable
optional key and derive a one-execution server key when absent. Completion bills the larger of
declared duration and the 2,500,000-byte-per-second size floor, capped at four hours; daily storage
charges 3.33 microcredits per live video-minute exactly once per UTC day. Cloudflare D1 and the
`RECORDINGS` R2 binding are the local authorities, so all eleven routes have no protected gate.

The three membership actions use `frame.web-membership-action-request.v1`, trusted active-tenant
context, the `organization_library.v1` bucket, and migration
`0040_legacy_membership_actions_expand.sql`. Invite removal, one-member insertion, and
creator-inclusive replacement preserve missing role/members defaults, reject explicit null, treat
present empty members as authoritative, and cap submitted targets at 500. The D1 boundary reasserts
manager and organization-member authority, forces the creator to admin, revokes affected mutation
grants through authority-generation changes, and atomically records the final membership,
idempotency receipt, audit, invalidation, and consumed proof. Remove/add return exact no-store `200`
success objects; replacement additionally returns its count. All three remain production
fail-closed behind released-client E2E.

The related mobile PATCH (`cap-v1-05776c542380771e`) remains unpromoted. Its session-or-API-key and
owner-or-membership rules are typed, but the exact fresh bootstrap still depends on provider image
URL signing and Cap root folders with nullable `spaceId`; Frame does not approximate either output.

The four folder CRUD carriers share one scoped atomic D1 boundary in migration
`0041_legacy_folder_crud_expand.sql`. Exact mobile `POST /api/mobile/folders` uses session-or-API-key
auth, optional caller idempotency, name trimming, default color, and a personal-root projection; it
is production-enabled. `FolderCreate`, `FolderDelete`, and `FolderUpdate` preserve Effect's
single-request RPC envelope, branded raw IDs, `Option` presence, and typed `Exit`/`Die`/`Defect`
failure shapes. Their local contracts prove parent/cycle checks, descendant handling, recursive
reparent/delete, replay, and rollback. They remain fail-closed behind human approval because the
pinned implementation admits same-organization cross-namespace parent edges that Frame rejects to
preserve its scope invariant.

The eight user/account identities share the source-pinned contract in
`frame-application::legacy_user_account` and migration
`0042_legacy_user_account_expand.sql`. Exact `POST /api/settings/user/name` preserves JavaScript
field presence and returns JSON `true`; the shared `/api/erpc` carrier preserves the tagged
`UserCompleteOnboardingStep` and `UserUpdate` `Exit` envelopes, including the ignored payload user
ID and nested image Option. Account patch and all-device sign-out use the authenticated
compatibility-action carrier, require a matching header/body idempotency key, and consume the
validated browser mutation grant in the same D1 batch as authority reassertion, mutation, receipt,
effect, and audit. Name, patch, and sign-out are production-enabled. Image-bearing RPC branches
fail closed pending R2 provider execution, and the three environment-first devtools remain behind
human approval.

The six retained collaboration mutations use migration
`0043_legacy_collaboration_expand.sql` and one operation-specific D1 journal while keeping their
source asymmetries at ingress. Mobile comment/reaction creation and exact-comment deletion accept
the host session or Cap's literal second space-delimited 36-character API-key token. Mobile create
applies ECMAScript trim, owner/shared-active-organization video authority, and ISO response dates;
create notifications run after commit and failures are swallowed. The web DELETE route preserves
missing-versus-empty `commentId`, its anonymous `400`, and deletion of only the actor's target and
direct replies. The delete ACTION uses the caller-supplied parent branch for transactional
notification cleanup, while the new-comment ACTION deliberately accepts whitespace, empty roots,
and orphan/cross-video parents without video authority. Every mutation binds tenant, actor,
operation, fingerprint, receipt, effect, and audit; deletes abort beyond 100,000 staged comments or
notifications. All six have five-axis local evidence and no protected production gate.

The ten retained video-property mutations use migration
`0044_legacy_video_properties_expand.sql`, 16 checked-in D1 statements, and one atomic authority
that keeps native checksummed metadata isolated from Cap's arbitrary truthy legacy metadata.
Mobile title/password use ECMAScript trim while browser title/password preserve raw whitespace;
date/title metadata follows JavaScript spread rules, settings preserve unknown keys while
normalizing playback speed, and anonymous verification checks the video hash before joined-space
hashes. PBKDF2-HMAC-SHA256 material and the encrypted bounded password cookie never persist
plaintext. Three mobile operations retain provider-execution gates and edit-date retains human
approval; metadata, edit-title, password actions, and settings are production-enabled.

The three library-ID reads use migration `0045_legacy_library_id_reads_expand.sql` and an exact
same-origin ACTION carrier. Folder, organization, and space reads preserve source-shaped
success/failure objects and deliberately leave ID arrays unordered. Frame additionally reasserts
the actor's active tenant and folder/space membership, closing the pinned source's cross-tenant
lookup weakness rather than reproducing it. All three are production-enabled.

The two library-detail reads use migration `0046_legacy_library_detail_reads_expand.sql` and the
same fail-closed ACTION admission boundary. `getUserVideos` preserves the source's effective-date
ordering, nullable JSON metadata, distinct comment/reaction counts, folder decoration, and
screenshot-aware upload marker while constraining every owned video to the actor's live active
tenant. `searchDashboardVideos` preserves ECMAScript whitespace/UTF-16 normalization, literal
LIKE escaping, prefix rank, effective-date order, nullable owner/duration fields, visibility, and
the eight-result cap. Screenshot, floating-point seconds, and generated effective-date shadows
avoid lossy projection through Frame's narrower native row. Both reads are production-enabled.

The two provider-free space-authorization ACTIONs reuse the existing organization, space, member,
and global user aliases without adding another migration. `getSpaceAccess` preserves Cap's exact
owner/creator precedence, invalid-role normalization, `canManage` truth table, access object, and
null projection; `requireSpaceManager` returns the same object or the two exact pinned error
messages. The same-origin carrier removes caller control of `userId`, rejects idempotency keys, and
reasserts the session actor's live active tenant plus the immutable space alias on every D1 read.
Owner and creator wire IDs must be persisted global aliases: exact imported membership IDs are
promoted into that authority, while native users use deterministic SHA-256/Crockford candidates,
database uniqueness, and eight bounded collision retries. No UUID truncation is emitted. Both
operations are production-enabled with no protected gate.

The four Chrome-extension auth/bootstrap routes use migration
`0048_legacy_extension_auth_expand.sql` and exact path carriers. The start GET validates a pinned
HTTPS `*.chromiumapp.org` redirect and remains free of credential-minting effects; its consent HTML
escapes email, redirect, state, and cancel fragment. Approval accepts only the bounded URL-encoded
same-origin form, restarts expired sessions through login, and returns a random UUID in the
Chromium fragment. D1 stores only the UUID digest and atomically performs insert, trailing-hour
count/delete, and postcondition assertion, so an eleventh key leaves no residue. Revoke preserves
Cap's 36-character-key-first/session-fallback middleware and deletes only an actor-owned digest.
Bootstrap selects active-owned, deterministic owned, then oldest live membership, repairs a
dangling pointer, and derives the organization owner's entitlement: hosted Cap uses subscription
state while `NEXT_PUBLIC_IS_CAP=false` is unlimited. Requiring active-owned is an intentional
cross-tenant pointer hardening that does not change valid source results. All four routes are
provider-free and production-enabled.

The three Chrome-extension instant-recording routes use migration
`0051_legacy_extension_instant_recordings_expand.sql`, a 25-file source closure, and exact API-key
or browser-session carriers. Create atomically maps a Cap NanoID alias to a native UUID video and
returns an actor/alias-scoped, metadata-bound R2 PUT capability with a 900-second expiry and
`If-None-Match: *`. The persisted object key remains `{actor}/{alias}/result.mp4`; verified custom
domains affect only the share URL. Progress clamps uploaded bytes, preserves Cap's timestamp fence,
and rejects uploaded/total regressions as successful no-ops. Delete first commits a native
tombstone and pending cleanup operation, deletes every R2 object under the exact actor/alias prefix,
then finalizes D1 while retaining the alias. Wrong-owner progress/delete returns 404 to avoid tenant
disclosure. These R2-backed routes have no separate provider-execution gate.

The six mobile bootstrap/cap routes use migration
`0052_legacy_mobile_bootstrap_caps_expand.sql`, owner-scoped D1 projections, and the canonical
`RECORDINGS` R2 bucket. Bootstrap and list bind the session actor's accessible active organization;
detail, playback, download, and delete bind the immutable 15-character video alias to an owned
native video and return 404 for both foreign and absent targets. Private media URLs use one-hour
SigV4 GET capabilities whose only signed header is `host`, preserving mobile byte-range playback.
The download projection retains screenshot, processed MP4, and raw-upload selection; playback
retains every source-specific playlist key and a best-effort transcript. Delete deliberately
matches Cap's database-before-provider ordering with an immutable D1 continuation and one bounded
prefix list/delete rather than claiming retry convergence the source did not have.

The three released mobile upload routes use migration
`0056_legacy_mobile_uploads_expand.sql`, exact session-or-36-character-API-key authentication, and
the canonical `RECORDINGS` R2 bucket. Create reasserts organization membership/ownership and the
optional actor-created personal folder, dual-writes native and Cap identities, and returns an
1800-second, content-type-bound, `If-None-Match: *` PUT capability for the exact minted raw key.
Progress applies Cap's truncation, nonnegative, and uploaded-to-total clamp while tightening D1
numbers to JavaScript-safe integers. Completion requires that exact key and a nonempty R2 HEAD with
matching optional length, then records one immutable provider-pending intent. It deliberately does
not create a media job, workflow receipt, or processing phase. Create and progress are
production-enabled; completion remains fail-closed as `503 provider_execution` until independent
workflow-submission evidence is admitted.

Desktop `GET /api/desktop/session/request` is bound to the source-pinned sign-in bridge without a
provider gate. Frame verifies the host-only browser credential through `AuthService`, exports only
that exact request cookie with its matching minimum idle/absolute D1 expiry, or returns a random
UUID v4 once while retaining only its `legacy_source = 'desktop'` digest. Exact parameter shapes,
absolute login restart, loopback redirect, `cap-desktop://signin` deep link, 1800ms hybrid fallback,
desktop CORS, and no-store headers are preserved. Frame deliberately restricts Cap's unconstrained
port string to a decimal TCP port in `1..=65535` and emits escaped JSON/HTML plus a restrictive CSP.

Six additional provider-free desktop carriers are production-enabled through migration
`0050_legacy_desktop_compatibility_expand.sql`: organization and profile reads, organization
branding, personal Google Drive selection, upload progress, and video deletion. They retain Cap's
36-character API-key-first/session-fallback authentication and exact mounted desktop CORS policy.
Mutations accept the released clients' missing idempotency header by deriving a per-execution key,
while a supplied key remains durably replayable and rejects a changed request. D1 owns live
tenant/role projections, branding metadata, progress timestamp arbitration, and deletion state;
video deletion then resumes an `effect_pending` operation until every exact actor/video prefix
object is removed from the `RECORDINGS` R2 bucket, with legal holds failing closed.

## Adapter boundary and remaining gates

Existing identity, organization, business, multipart/storage, governance, backfill, and media
authorities are mapped by family. The mapping is not proof that all 288 legacy-shaped operations
are served by a Frame adapter. Promotion still requires, per row or approved retirement:

- success, validation, authorization/non-disclosure, replay/retry, and stable-failure E2E tests;
- N and N-1 desktop/mobile/extension/developer client journeys before any redirect;
- signed provider callback route wiring plus deployed D1 replay/fault evidence;
- protected billing sandbox, ledger reconciliation, production-shaped load/fault runs, and named
  retirement approvals;
- issue 28/29 managed quota/outage/kill-switch/native-fallback evidence.

`OrganisationSoftDelete` (`cap-v1-5cd4cac9da73f975`) is a retained organization operation, but its
pinned service deletes Cap-managed S3 prefixes and Tinybird tenant data before completing database
cleanup. Its completion record consequently requires both exact local adapter/provider-effect
orchestration and protected provider execution. The local adapter/orchestration contract is now
complete; the presence of a local organization authority still does not make provider execution
local-only.

The same compound completion rule applies to every exact provider identity locked in the generator.
Their repository-local adapters, redaction, replay, outbox, and evidence state machines are
complete, but they remain unavailable until protected provider execution is independently proven.
Commercial activation remains unavailable for a different reason: its declaration/caller audit is
complete, while a repository owner must approve implementation or retirement.

Operation-level transport overrides are source-pinned rather than inferred from route families.
The mobile folder-create route therefore records session-or-API-key authentication and optional
idempotency, the folder RPCs preserve optional idempotency, and the four space actions accept
bounded `multipart/form-data` instead of JSON. Those space actions also pin their optional icon
storage and SVG-sanitization graph and retain a protected provider-execution gate.

The production exceptions are the 129 ungated exact contracts enumerated by the generated report:
71 HTTP routes, seven RPCs, 50 server actions, and one workflow. The other 159 rows have complete
local work but retain one or more released-client, hardware, provider, or human-approval gates.
For every protected row, the current production registry fails closed until independently admitted
evidence satisfies the reported gate; an unproven fallback also fails closed.
