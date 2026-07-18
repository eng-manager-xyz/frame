# API and workflow parity v1 — local evidence

Evidence date: 2026-07-17. Reference:
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

This is reproducible local implementation evidence for all 288 source-pinned static, D1, R2,
mobile, desktop, extension, developer, transcript, organization, collaboration, storage, analytics,
media, integration, billing, and authentication identities plus the shared compatibility boundary.
It is not production-provider, hardware, released-client, approval, or retirement sign-off. Two
hundred seventy-six contracts have all five local evidence axes: 136 routes, 15 Effect RPCs, 111
server actions, and 14 workflows. The production registry enables 129 ungated contracts; 147
locally proven operations preserve protected gates and 12 locally complete declaration/retirement
rows still require accountable approval.

## Inventory result

`python3 scripts/ci/check-api-workflow-parity.py --generate --require-reference` passed against the
pinned checkout. The machine-readable report contains 288 distinct operations:

| Kind | Count |
|---|---:|
| HTTP routes | 138 |
| Effect RPC operations | 15 |
| Server actions, including inline actions | 121 |
| Durable workflow/dispatch/recovery entrypoints | 14 |

Disposition totals are 227 replace, 35 migrate, 16 protected-parity-required, and 10 proposed
retirements. Two hundred seventy-six source-pinned endpoint contracts are proven locally across all
five evidence axes. The other 12 rows have complete local declaration or retirement contracts but
cannot claim endpoint success or retirement authorization. Production promotion or retirement
authorization remains pending for 159 protected rows. No
success is inferred from a family-level Rust authority. The report pins every enumerated
declaration and implementation-bearing source by SHA-256 and regenerates
[human-readable documentation](../generated/api-workflow-parity-v1.md).

Every row now carries a closed completion decision in addition to its five evidence axes, and all
288 report `local_work: complete`. One hundred twenty-nine are production-enabled. The other 159
remain fail-closed behind one or more genuine released-client, hardware, provider, or human-approval
gates. This split is machine-validated so a protected gate cannot conceal an unimplemented local
route, action, RPC, or workflow.

The generated `fixtures/api-parity/v1/operation-contract-catalog.json` also assigns all 288
identities one of 81 normalized, source-manifest-bound specifications covering the required
success-or-retirement, validation, authorization, idempotency/retry, and failure cases. This closes
ambiguity in the implementation backlog, not the endpoint gap: the catalog declares itself
specification-only, records 276 locally tested success contracts and 129 promotion-authorized
operations, and preserves all 159 protected production or retirement gates. The parity checker rejects catalog
entries that promote incomplete work or stop failing closed.

The pinned source closure now includes the concrete catch-all handler for all 22 Mobile route rows
and the HTTP transport, RPC layer, family handler, and called service/policy sources for all 15
Effect RPC rows. The 14-row organization audit additionally pins imported authorization, role,
normalization, plan, and space-membership helpers where they determine behavior. It found no new
provider dependency in those 14 rows. Separately, `OrganisationSoftDelete`
(`cap-v1-5cd4cac9da73f975`) deletes Cap-managed S3 prefixes and Tinybird tenant data. Its local D1
authority, redacted provider intent, replay, and immutable evidence contract is complete, while
`provider_execution` remains protected. The same 45-operation integration family closes the local
side of Google Drive, Stripe, Resend, OAuth, Discord, media/storage, GitHub, Loom, release, and
space-icon provider effects without fabricating their outcomes. The final 17 auth/billing/admin
contracts likewise stage provider intents and, where required, human approval before execution.
The same audit corrects the mobile folder route to session-or-API-key auth,
preserves the released clients' optional idempotency behavior, and records multipart rather than
JSON admission for the four space actions.
`POST /api/commercial/activate` (`cap-v1-700b21489623a3e4`) is different: the pinned Cap snapshot has
contract declarations and desktop callers but no handler or concrete licensing authority. The
source audit is complete and provider-free; production stays fail-closed behind a repository-owner
implementation-or-retirement decision rather than a fabricated provider gate.

## Endpoint-adapter and compatibility audit

The generated documentation now derives an executable coverage boundary from every inventory
row instead of leaving the remaining work implicit. The transport inventory is:

| Legacy transport kind | Rows | Locally proven endpoint success | Exact missing boundary |
|---|---:|---:|---|
| HTTP | 138 method rows / 128 unique paths | 136 | two declaration-only routes have complete local disposition audits but require repository-owner implementation-or-retirement approval |
| Effect RPC | 15 | 15 | all RPC contracts are locally exact; protected provider or human evidence remains fail-closed where named |
| Next server action | 121 | 111 | 10 messenger/support actions have a complete retirement response but require repository-owner retirement approval |
| durable workflow | 14 | 14 | every workflow has an exact local scheduler/carrier and immutable retry/evidence contract; protected executors remain external gates |

A source-level audit promoted both `/api/changelog/status` method rows and both `/api/changelog`
method rows only after adding exact query, content-corpus, JSON, empty-204, and CORS-header
transport semantics. The full feed pins every one of the 99 MDX inputs, its 88,817-byte compact
body, and request/configured-origin reflection including the `null` fallback. Media status/health
and other protected rows now have exact request/business-effect staging adapters; their executable
unavailable response remains failure evidence until independently admitted execution evidence
exists.

The current `apps/control-plane/src/routing.rs` router exposes target-native `/api/v1` routes and
exact source-pinned legacy carriers. Similar-looking paths are never promoted by resemblance.
Source-pinned compatibility routes use exact D1/R2 or fail-closed protected authorities; 71 routes,
seven RPCs, 50 actions, and one workflow are production-enabled. The Frame
`POST /api/v1/web/compatibility-actions/{operation-id}` selector is not a legacy identity: it
constructs the frozen abstract `server_action`/`ACTION` envelope internally. Protected local
actions resolve by those abstract identities, stage only admissible local effects, and remain
denied from claiming external success.
Exact `GET /api/status` (`cap-v1-05b6ba3f76daac22`) is pinned to
`apps/web/app/api/status/route.ts` at SHA-256
`ba3eb1177da489a10f74c9dbc68e0db8324b695c82499e35d6f8d9da8aaf5797` and returns the same Fetch
contract: status `200`, content type `text/plain;charset=UTF-8`, and body `OK`. Exact
`GET /media-server` (`cap-v1-ff19008f47194c43`) is pinned to
`apps/media-server/src/app.ts` at SHA-256
`b3ba5fc1c8e93bd6896aa4399c283cc33a73e7777275816a11334fd71b75fc57` and returns status `200`,
content type `application/json`, and the exact compact metadata object with its source-ordered
15-entry endpoint list. The checked-in production Wrangler route
`frame.engmanager.xyz/media-server*`, same-origin owner matrix, and loopback
live-runner self-test prove that exact path and query-bearing requests reach
the adapter while trailing-slash, child, and lookalike paths fail closed. A
protected public trace remains required before claiming provider deployment.
Exact `GET /api/changelog/status` (`cap-v1-a1b180c5d123c870`) and
`OPTIONS /api/changelog/status` (`cap-v1-16668b858461f386`) pin the route at SHA-256
`c2a3c107fce46765286e5a5e14fc3b21959e22b50070ecdc45f3d3d16ea5541b`; GET additionally pins the
loader and latest numeric changelog source (`99.mdx`, version `0.5.6`). GET returns exact compact
`hasUpdate` JSON for missing, empty, stale, and matching versions plus the three wildcard CORS
headers. OPTIONS returns empty `204`, the same headers, and no content type. All other
legacy-shaped Cap paths remain closed; there is no hidden redirect or claimed external fallback.

Exact `GET /api/changelog` (`cap-v1-0fa8384f3666825b`) and `OPTIONS /api/changelog`
(`cap-v1-237f41f3086a2d67`) pin route SHA-256
`b47371ce19a03def1b675996615e1c48af41651bc48ca479d0a97bd9e7167b04`, the loader and CORS
utilities, all 99 numbered MDX file hashes, source-manifest SHA-256
`dace60a24a816766681282e4569eda38e16fd85c96a9b2ab311a59351ef58b2d`, and exact body SHA-256
`333c789a76f6f496f94e5e2a47a192fe0c9f87165971689c9c297e5eb43b7499`. GET returns that exact
99-entry compact JSON body; OPTIONS returns an empty `204`. Both preserve Cap's credentialed CORS
origin selection without a provider or database dependency.

Exact `GET /api/mobile/session/config` (`cap-v1-4f21920a947c4c84`) pins the public endpoint
declaration at `packages/web-domain/src/Mobile.ts` SHA-256
`331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b` and its concrete
`getAuthConfig` handler at `apps/web/app/api/mobile/[...route]/route.ts` SHA-256
`02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79`. The inventory now records
`public_or_flow_token`, a zero-byte body, forbidden idempotency, and the Mobile current-release
caller. A typed Worker authority reduces `GOOGLE_CLIENT_ID` and `WORKOS_CLIENT_ID` to non-empty
JavaScript-string truthiness
booleans; no binding value or caller-controlled input can cross that boundary. All four states have
distinct stable fingerprints and exact compact JSON bodies. The pinned `@effect/platform` 0.92.1
source proves the schema's default JSON encoding is `application/json` and the builder supplies that
exact encoding content type to the JSON response.

Exact `GET /api/notifications/preferences` (`cap-v1-d130c840f654bd72`) is pinned to the standalone
route SHA-256 `3692f8854c0c050f5168f89acb1d03dc1c31d4529000e0b5e140078e8d3ce975` and its complete
session/schema/Next transport closure. The Worker accepts only the Frame host-only browser session
cookie and browser client kind; it does not add organization, API-key, Origin, CORS, CSRF, or
route-level user-status rules. Its actor-only query reads `users.preferences_json`. Missing, null,
or whole-object-invalid values produce all five `false` fields; an otherwise valid object with an
omitted `pauseAnonViews` preserves the four required booleans and defaults only that field to
`false`; unknown fields are stripped. Success uses the exact compact camel-case field order and
`application/json`. Missing/invalid sessions return exact `401` JSON. Preference query, decoding,
and serialization failures return exact `500` JSON. Authentication repository, keyring, policy,
and Worker infrastructure failures remain outside the pinned handler's caught preference-query
failure path and use Frame's generic unavailable response.

Exact `GET /api/notifications` (`cap-v1-14dcca6d36eee6b3`) pins the route, API response schema,
session lookup, notification/user persistence, and image-resolution boundary. The D1 read is scoped
to the authenticated recipient and that user's active organization, orders unread rows first and
then by descending creation time, and left-joins authors only for non-anonymous rows. It preserves
Cap's per-row fault isolation: malformed union members and missing authored users are omitted while
valid siblings remain. Anonymous views default a missing/null name to `Anonymous Viewer`; grouped
`anon_view` counts fold into `view`, and all four count keys are always present. Date fields use the
same millisecond ISO JSON representation. Avatar resolution is explicitly provider-tolerant, so a
missing or failed image effect returns `avatar: null` instead of failing the response. Missing
sessions retain the source's `401` body/content type, D1/projection failures retain its exact `500`
JSON, and successful reads use compact source-ordered JSON. The production ingress applies the
reported principal-scoped collaboration-notifications rate limit before the D1 read.

Exact Navbar `updateActiveOrganization` (`cap-v1-a3b4c805d409bc7c`) is registered only as
`server_action`/`ACTION`. Its local-evidence dispatcher binds the mutation actor to the trusted
session principal, maps the 15-character Cap NanoID to the migration UUID, executes the D1
membership-only active-selection batch, preserves `default_organization_id`, derives the revision
server-side, journals `active_organization_set`, and yields `/dashboard` invalidation followed by a
void result. The authenticated same-origin Frame ingress validates a session-bound one-use browser
mutation proof and consumes that grant atomically in the same D1 batch as the selection, operation,
and audit writes. Stale, replaced, missing, and denied proofs cannot leave a partial business
mutation; denial consumes the proof separately after the rolled-back mutation attempt.

The typed browser client accepts only a Cap NanoID and clears its active-organization cache before
dispatch because an interrupted response is indeterminate. The Leptos dashboard picker still uses
Frame's native UUID/revision route and exposes no Cap NanoID projection. Consequently the report
keeps `released_legacy_client_e2e` as a protected-only completion gate. The production registry
rejects this action with the stable unavailable response even though the local implementation and
provider-free SQLite conformance pass. The mobile PATCH (`cap-v1-05776c542380771e`) is deliberately
not promoted: although its
session-or-API-key and owner-or-membership rules are pinned, exact output still requires provider
image signing and nullable-space root-folder projection.

Exact `setTheme` (`cap-v1-7773d3e70d1d5919`) is the production-enabled server action. It preserves
the abstract `server_action`/`ACTION` identity behind the same authenticated same-origin Frame
selector, accepts only `light` or `dark`, forbids a client idempotency key, consumes its one-use
browser proof, and returns empty no-store `204` with the exact
`Set-Cookie: theme={light|dark}; Path=/` effect. Its retry model is last-write-wins without client
idempotency. The Leptos toggle calls this typed action and changes the body theme only after success;
bootstrap reapplies only an exact `light` or `dark` cookie and otherwise preserves the existing
body/system theme. The pinned Cap `Contexts.tsx` file documents a separate JS-cookie persistence
behavior and does not call the otherwise-unused `setTheme` action.

The exact add, remove, and move folder-assignment ACTIONs are locally proven by a source-pinned
application contract, migration `0036_legacy_folder_assignment_expand.sql`, 25 bounded D1 queries,
an authenticated same-origin carrier, and a typed browser client. Add/remove reject a complete list
unless every tenant video is actor-owned; move accepts a tenant video only for a manager-authorized
selected context, including a creator-owned personal/root folder with nullable `space_id`. The D1
batch reasserts live actor, active tenant, membership, folder/space/video revisions, normalized
direct/shared/space storage, exact postconditions, tenant/actor/action-scoped idempotency, business
audit, cache effects, and one-use browser-grant consumption. SQLite conformance covers the full
migration chain, eight semantic cases, replay/conflict/race, all-or-nothing ownership, foreign-owned
manager moves, personal roots, dirty multiplicity, and secret redaction. The browser client clears
workspace cache before every valid send and rejects malformed success shapes. All three remain
production unavailable behind `released_legacy_client_e2e`; this local proof is not a released Cap
client run.

The four library-placement ACTIONs are locally proven through migration
`0037_legacy_library_placement_expand.sql`, bounded D1 statements, the authenticated carrier, and
the typed browser client. Organization add/remove require actor-owned videos; space add/remove
preserve the source's matching-share authorization asymmetry. The atomic boundary reasserts the
active tenant, membership, space, video, and storage graph while consuming the one-use proof and
writing the action-scoped receipt, audit, invalidation, and exact normalized root mutation. Their
action-specific success objects are no-store `200` responses. All four remain production unavailable
behind `released_legacy_client_e2e`.

The two notification-write ACTIONs are locally proven through migration
`0038_legacy_notification_actions_expand.sql`. Mark-read preserves the missing-versus-present
notification selector and exact count/read-time transition; preference writes preserve unrelated
JSON siblings and the source-observed optional anonymous-view field. Both bind actor authority,
idempotency, audit, invalidation, and browser-proof consumption into one D1 transaction and return
the exact empty no-store `204`. They remain production unavailable behind released-client E2E.

All eight user-owned developer ACTIONs are locally proven through migration
`0039_legacy_developer_actions_expand.sql`, principal-only session authority, a typed browser
carrier, and local SQLite conformance. The contracts preserve nullable-logo and optional
auto-top-up presence, exact zero-row delete behavior, domain normalization, and one-time create or
regenerate credential responses. Public keys are hashed, secret material is encrypted at rest,
credential replay material is locally AEAD-sealed and request-bound, and key-bearing debug output is
redacted. Missing or invalid local secret authority fails closed. Production promotion remains
behind released-client E2E.

The eleven developer SDK/REST/cron routes are locally proven through migration
`0054_legacy_developer_api_expand.sql`, the checked `legacy_developer_api` query family, pinned
transitive source closures, and `legacy-developer-api-sqlite-conformance.py`. Evidence covers
public-versus-secret key selection, production Origin enforcement, exact usage/video/status
projections, header-free multipart SigV4 capabilities, duration-and-size completion billing,
atomic insufficient-credit rollback, resumable R2 complete/abort outboxes, and once-per-UTC-day
storage snapshots. The released SDK's absent idempotency header is preserved as optional; supplied
keys replay immutable receipts. All eleven are production-enabled with D1 and R2 as in-scope local
authorities and no provider-execution gate.

Six membership ACTIONs are locally proven through migration
`0040_legacy_membership_actions_expand.sql`. Invite removal, single and bulk addition, batch and
single removal, and creator-inclusive replacement derive the active organization from the trusted
session, preserve missing role/members defaults, submitted ordering/duplicates, unmatched removal
behavior, and present-empty-members precedence, and cap submitted targets at 500. The D1
transaction reasserts tenant and manager authority, organization membership, creator protection,
exact final membership, authority-generation changes, mutation-grant revocation, receipt, audit,
invalidation, and browser-proof consumption. The original three actions remain unavailable behind
released-client E2E; bulk add, batch remove, and single remove have no protected gate and are
production-enabled.

The four folder CRUD carriers are locally proven through migration
`0041_legacy_folder_crud_expand.sql`, 29 bounded D1 statements, and exact raw mobile/Effect-RPC
ingress. Mobile `POST /api/mobile/folders` trims the name, defaults the color, binds an optional
caller idempotency key, creates a personal root, and returns the exact folder projection; it is
production-enabled. `FolderCreate`, `FolderDelete`, and `FolderUpdate` preserve Effect's one-request
JSON envelope, Option presence, branded-string behavior, typed Exit/Die/Defect failures, scoped
parent/cycle checks, descendant updates, and recursive reparent/delete semantics. Their local
contracts pass, but the pinned Cap implementation permits cross-namespace parent edges that Frame
deliberately refuses; production therefore fails closed behind explicit human approval rather than
silently weakening the scope invariant.

All eight user/account identities are locally proven through migration
`0042_legacy_user_account_expand.sql`, 40 checked-in D1 statements, exact route/RPC/action ingress,
and ten SQLite cases. The proof preserves missing/null/empty names, onboarding merge/default
behavior, lossless organization projections, nested image Option handling, owner-or-membership
account access, all-device session/API-key revocation, environment-first devtools, digest-only
replay evidence, and atomic rollback. A dedicated case consumes the one-use browser mutation grant
with the account mutation and rejects a stale grant without changing user or evidence state. The
name route, account patch, and sign-out are production-enabled; image RPCs remain R2-gated and the
three devtools remain human-approval-gated. This is local D1/compile evidence, not a deployed
Cloudflare provider observation.

All ten video-property identities are locally proven through migration
`0044_legacy_video_properties_expand.sql`, 16 checked-in D1 statements, exact mobile/route/action
ingress, and three SQLite cases spanning every mutation, replay, password order, stale rollback,
and evidence immutability. The source-specific contract preserves ECMAScript versus raw whitespace,
truthy metadata replacement, JavaScript object spread, settings normalization, PBKDF2-HMAC-SHA256
wire material, and anonymous video-then-space password verification. Native checksummed metadata
remains isolated in its own column. Three mobile mutations retain provider execution and edit-date
retains human approval; the other six operations are production-enabled.

The folder, organization, and space video-ID reads are locally proven through migration
`0045_legacy_library_id_reads_expand.sql`, eight bounded D1 statements, exact same-origin action
ingress, and SQLite conformance. They preserve source-shaped success/failure objects and unordered
ID arrays while reasserting active-tenant and folder/space authority, so Frame does not reproduce
the pinned source's cross-tenant lookup weakness. All three operations are production-enabled.

`getUserVideos` and `searchDashboardVideos` are locally proven through migration
`0046_legacy_library_detail_reads_expand.sql`, five checked-in D1 statements, exact same-origin
action ingress, and provider-free SQLite conformance. The first preserves every source projection
field, effective-date ordering, scope-specific folder decoration, distinct text/emoji counts, and
screenshot-aware upload presence while closing its owner-only cross-tenant weakness. The second
preserves ECMAScript whitespace/UTF-16 normalization, LIKE escaping, prefix rank, visibility,
nullable floating-point seconds, and the eight-result cap. Both operations are production-enabled.

Desktop `GET /api/desktop/org-custom-domain` (`cap-v1-ed9957ac480103b9`) is now locally proven and
production-enabled through the lossless `0030_legacy_org_custom_domain_projection.sql` projection,
exact raw GET/OPTIONS routing, desktop session-or-36-character-API-key authentication, bounded D1
active-organization selection, and the pinned Hono CORS allowlist. It preserves independent nullable
fields, case-sensitive URL prefixing, exact snake-case JSON, and the runtime ISO timestamp string;
missing projections for existing organizations fail closed as incomplete imports. The separate web
`GET /api/org-custom-domain` declaration (`cap-v1-9323d0178c5a63b5`) remains explicitly unpromoted
behind a repository-owner implementation-or-retirement decision:
its ts-rest and Effect declarations claim `domain_verified: boolean | null`, but no pinned handler
defines authorization, boolean derivation, or runtime failures. Frame does not coerce the concrete
desktop timestamp into invented web behavior.

Anonymous `GET /api/video/domain-info` (`cap-v1-10e17d0e86b49830`) is locally proven and
production-enabled against that same lossless custom-domain projection. The source handler never
reads a session, so the catalog now records anonymous ingress and its observable 404 rather than
inventing tenant non-disclosure. Checked D1 queries preserve the source's first active shared
organization, owner-organization fallback, and unordered `LIMIT 1` choices; the HTTP carrier keeps
the timestamp-as-ISO-string or literal `false` response union and all four source failure bodies.
Provider-free SQLite evidence covers shared precedence, owner fallback, missing video, and revoked
shares.

Desktop `GET /api/desktop/session/request` (`cap-v1-768895bc99380850`) is locally proven and
production-enabled. The application contract pins the full 12-file handler/client/API-middleware/auth/schema
closure and all redirect/HTML wire branches. The control-plane reuses `AuthService` for the exact
host-only cookie, binds expiry to the authenticated session ID and actor, and performs a single
active-user `INSERT ... RETURNING` for digest-only desktop UUID keys. Focused SQLite evidence proves
inactive/deleted actors cannot mint, duplicate digests cannot be rebound, a sibling session cannot
supply expiry, and the earlier idle/absolute expiry wins. The only intentional source tightening is
the numeric `1..=65535` loopback port plus escaped hybrid state/CSP; no provider observation is
claimed.

The six operations in `desktop-compatibility.json` are also locally proven and production-enabled.
Their source manifests bind the raw organization, branding, storage-selection, profile, progress,
and delete carriers to the exact desktop authentication/CORS boundary. Migration
`0050_legacy_desktop_compatibility_expand.sql`, checked-in single-statement D1 queries, focused Rust
tests, and SQLite conformance cover actor-scoped reads, admin-only branding with sibling metadata
preservation and bounded image validation, personal Google Drive activation, stale-progress no-ops,
durable optional-key replay/conflict, and resumable deletion. The deletion proof commits the D1
tombstone before removing the exact actor/video prefix from the `RECORDINGS` R2 bucket, rejects a
legal hold, and completes its receipt only after provider cleanup. The Worker R2 orchestration
compiles locally; this is not a claim of deployed Cloudflare object deletion.

`LegacyCompatibilityRegistryV1` now decodes the exact report during Rust tests and registers all
288 identities. For every one of the 278 retained rows, an evidence-enabled test case passes
through `ApiGatewayV1::admit_mutation` and exercises body/content-type validation,
authentication/non-disclosure, rate limits, required/forbidden idempotency headers, stable public
errors, and privacy-safe trace/audit labels. A separate case proves that all 10 retirement rows
remain on fallback without approval and can never fabricate Frame success. The API parity workflow
runs this suite explicitly. `LegacyEndpointCoordinatorV1` additionally sends every retained row
through one execution-port contract binding the operation ID, request fingerprint, idempotency key,
audit labels, and durable receipt. The local port conformance covers completion, exact replay,
conflicting key reuse, in-flight work, and every closed execution-failure mapping. The registry
also resolves all 288 exact identities and all 138 raw HTTP method patterns, including the pinned
dynamic/catch-all forms, while rejecting encoded, dot, empty, backslash, semicolon, and control
character paths without URL decoding.

The committed production state enables exactly 129 contracts: 71 exact HTTP routes, seven RPCs,
50 server actions, and one workflow across the source-pinned static, D1, R2, mobile, desktop,
extension, developer, transcript, organization, collaboration, storage, and analytics families.
No external legacy fallback is claimed. Every enabled ingress enforces its
reported bucket through the keyed-digest, bounded D1 authority in migration
`0034_compatibility_rate_limits.sql`; missing limiter authority fails closed and a saturated bucket
reaches the typed rate-limit error. The other 159 rows preserve their exact released-client,
hardware, human-approval, provider-execution, declaration, or retirement gates. The runtime deserializes
`local_work`, `protected_gates`, and `production_behavior` and rejects startup if source-pinned
allowlists disagree with those decisions.

The production registry returns a stable unavailable error for all 159 unpromoted rows; a
report-driven runtime test admits the exact primary caller for each and proves every one returns
`temporarily_unavailable`. The test-only local-evidence registry admits protected local operations
solely to exercise their completed adapters and atomic proof/effect transactions.
Synthetic fallback is enabled only in compatibility decision tests. The call-path audit finds a
production-shaped control-plane transport that constructs the same 288-row registry and a D1
implementation of the atomic execution/idempotency/audit port. The D1 port binds tenant scope,
operation identity, request fingerprint, idempotency key, fenced intent, completion receipt, and
append-only audit using digests only. Its durable semantic-adapter allowlist remains empty. The
production allowlists cover 71 exact routes, seven RPCs, 50 exact actions, and one workflow;
protected local operations are registered separately as gated. Exact method/path/query,
body/idempotency, source identity,
authorization, rate-limit, retry, response/header, and fail-closed negative cases are tested. No
retained webhook route constructs `D1WebhookReplayStoreV1` before a business effect. This is real
centralized admission and durable execution infrastructure plus 276 exact local contracts; the 12
declaration/retirement rows remain intentionally non-executable until approved.

The exact evidence-axis counts remain honest:

| Axis | Local contract | Family contract | Dependency pending | Endpoint adapter pending | Protected pending | Retirement pending |
|---|---:|---:|---:|---:|---:|---:|
| success | 276 | 0 | 0 | 2 | 0 | 10 |
| validation | 276 | 0 | 2 | 0 | 0 | 10 |
| authorization | 276 | 0 | 2 | 0 | 0 | 10 |
| idempotency/retry | 276 | 0 | 2 | 0 | 0 | 10 |
| failure | 276 | 0 | 2 | 0 | 0 | 10 |

The compatibility suite executes current and previous release decisions for all 267 release-managed
client associations. Its test-only evidence registry serves the 276 exact local contracts and uses
an explicitly synthetic fallback only in compatibility-decision tests; the control-plane production
registry separately admits the 129 completion-authorized contracts. Older releases are rejected.
The suite does not launch a released client binary/build, satisfy any protected journey, or prove
an external legacy fallback. The 12 client associations still pending endpoint success are all web
declaration/retirement decisions.
Associations overlap where one
operation serves multiple clients. No additional endpoint redirect is authorized.

Residual classification is intentionally compound:

| Inventory class | Rows | Remaining local work | Intrinsically protected evidence |
|---|---:|---|---|
| exact contracts locally proven and production-enabled | 129 | none | none |
| exact contracts locally proven but production-gated | 147 | none | one or more released-client, hardware, provider, or accountable-human evidence gates |
| declaration-only disposition audits | 2 | none; handler absence and required decisions are source-pinned | repository-owner implementation-or-retirement approval |
| proposed retirement responses | 10 | none; stable tenant-opaque retirement responses are complete | repository owner plus legal/privacy/customer-impact approval |

- Checkbox 2 now has bounded local implementation evidence: a typed, source-pinned Rust handler
  runs through the centralized registry/coordinator, and the shared validation, authorization,
  rate-limit, idempotency, stable-error, trace/audit, and atomic D1 execution boundaries remain
  exhaustive and fail-closed. The production-enabled operations are exact `GET /api/status`,
  `GET /media-server`, `GET`/`OPTIONS` for `/api/changelog/status` and `/api/changelog`,
  `GET /api/mobile/session/config`, both notification reads, and the exact source-pinned D1/R2
  families enumerated in the generated report, including developer API, transcripts, extension,
  desktop compatibility, mobile bootstrap, mobile uploads, and the developer-checkout CORS
  preflight. Another 147 exact local contracts
  remain unavailable behind released-client, hardware, human-approval, or provider gates; no family
  authority is mislabeled as another handler.
- Checkbox 7 now has complete repository-local per-operation request/response, authorization,
  retry, failure, and side-effect-or-evidence-staging semantics for every identity. The report has
  zero rows with unfinished `local_work`; protected external evidence remains explicitly separate.
- Checkbox 12 is protected-pending: actual current/N-1 released desktop, mobile, extension,
  developer, and web artifacts plus their browser/client journeys require consumer-repository
  builds and execution outside this repository. The 267-association registry suite is necessary
  compatibility-control evidence, not a substitute for those builds.

## Local contract and security result

The focused Rust suites cover:

- bounded bodies/content types and required/forbidden idempotency keys;
- authentication, tenant authorization non-disclosure, CSRF/origin decisions, rate-limit errors,
  and redacted correlation/audit data;
- exhaustive registry admission plus evidence-gated current/N-1 route-fallback decisions;
- provider-neutral derivative requests and queued/running/indeterminate/success/failure/cancelled
  public status fixtures;
- exact-origin redirects, HTTPS host allowlists, IP-literal rejection, and post-DNS private,
  loopback, link-local, shared, reserved, translation, multicast, documentation, and
  unspecified-address rejection;
- HMAC-SHA-256 against RFC 4231, exact signature grammar, key overlap/expiry, timestamp windows,
  byte-for-byte body integrity, 1 MiB body limit, atomic replay rejection, unavailable-store fail
  closure, and secret redaction.

Migration `0015_api_workflow_replay.sql`, the bound `INSERT OR IGNORE ... RETURNING` query, and
`D1WebhookReplayStoreV1` provide the production-shaped replay adapter. The provider-free SQLite
conformance launches two concurrent claimers and observes exactly one claim/one duplicate, proves
the claim survives a connection restart, rejects malformed/overlong/updated rows, prunes expired
rows in bounded 2+1 batches, and preserves an unexpired row. Focused control-plane module tests and
strict Clippy pass; this remains local SQLite/compile evidence rather than deployed D1 evidence.

Migration `0026_legacy_api_execution.sql` and its five bound queries provide the fail-closed legacy
execution journal. A second provider-free SQLite conformance launches two complete-transaction
contenders and observes one winner/one replay; proves exact replay after restart, conflicting-key
reuse rejection, tenant-scoped key digests, losing-reservation write fencing, complete
intent/result/audit postconditions, and immutable rows. It records zero enabled synthetic durable
semantic adapters, 276 inventory contracts proven locally, and 129 production-enabled contracts.
Protected local contracts are covered by their focused application, D1, and carrier conformance
suites while production admission preserves their explicit gates. This is likewise
local SQLite/compile evidence, not a deployed D1 or released-client/provider result.

Migration `0042_legacy_user_account_expand.sql`, its 40 checked-in queries, and the focused
user/account SQLite conformance separately prove all eight identities and the browser-action
authority fence. The retained evidence artifact contains only counts and gate classifications; it
stores no session token, CSRF value, idempotency key, provider credential, or customer data.

Migration `0043_legacy_collaboration_expand.sql`, its 28 checked-in queries, and the focused
collaboration SQLite conformance prove the six comment mutations separately across all five axes.
The retained evidence covers mobile owner/shared authority and API-key/session ingress, exact
mobile projections, web query presence, authored direct-reply deletion, caller-parent notification
selection, whitespace/empty-root/orphan creation, best-effort post-commit notification failure,
transactional delete-notification rollback, 100,000/100,001 row bounds, immutable receipts, replay,
conflicting reuse, and stale-authority rollback. This remains provider-free local D1/compile
evidence; it does not claim a deployed Render/Cloudflare observation.

Migration `0044_legacy_video_properties_expand.sql`, its 16 checked-in queries, and the focused
video-property SQLite conformance prove all ten mutations, ordered password verification, exact
replay, stale-authority rollback, native metadata isolation, and immutable evidence without
persisting plaintext passwords. Migration `0045_legacy_library_id_reads_expand.sql`, its eight
queries, and focused SQLite conformance prove all three active-tenant reads and cross-tenant
non-disclosure. Both remain local D1/compile evidence; their explicit provider/human gates are not
reported as deployed observations.

The source-pinned space-authorization application contract, its five bounded D1 queries, strict
same-origin carrier, and focused SQLite conformance prove both provider-free ACTIONs across all
five axes. Evidence covers owner-over-membership and creator-over-membership precedence, the dirty
non-owner `owner` normalization, Frame manager/viewer translation, contributor/unknown null roles,
the complete `canManage` truth table, exact access/null/error projections, active-tenant alias
scoping, missing/deleted/tombstoned non-disclosure, and forbidden client idempotency. Owner and
creator wire identities never use UUID truncation: imported member aliases are promoted into the
persisted globally unique user-alias authority, and native aliases have deterministic bounded
collision retry and non-drift proof. This is local SQLite/compile evidence, not a deployed
Cloudflare observation.

Migration `0048_legacy_extension_auth_expand.sql`, its 11 bounded queries, and the focused
extension-auth SQLite conformance prove the four provider-free routes across all five axes. The
evidence covers HTML/reflected-value escaping, exact redirect fragments, Fetch-Metadata/Origin
admission, digest-only random UUID storage, ten-key strict-hour admission with rollback of the
eleventh candidate, API-key precedence, actor-owned deletion, active-owned/owned/oldest-membership
selection, atomic active-pointer repair, free 300-second limits, and hosted/non-hosted Pro policy.
The active-owned check and empty-extension-host rejection are documented security tightenings;
neither grants broader authority than Cap. This is local SQLite/compile evidence, not a released
extension or deployed Cloudflare observation.

Migration `0051_legacy_extension_instant_recordings_expand.sql`, its 24 checked-in statements, the
focused Rust suites, and the instant-recordings SQLite conformance prove the three extension media
routes across all five axes. Evidence covers API-key/session precedence, active-tenant and R2
integration authority, immutable NanoID-to-UUID aliases, exact actor/alias object keys, metadata-
bound no-overwrite presigning, verified custom-domain share URLs, bounded JSON/date validation,
clamping, timestamp and byte monotonicity, equal-retry convergence, wrong-owner non-disclosure,
two-phase prefix cleanup, tombstone/alias retention, rollback, and foreign-key integrity. The
signer and D1/R2 orchestration compile locally; no production credential, object, or customer data
was used, so this is not a deployed Cloudflare observation.

Migration `0049_legacy_mobile_session_expand.sql`, its bounded checked-in queries, and the focused
mobile-session SQLite conformance prove the four mobile session routes across all five local axes.
The evidence covers encrypted email-delivery handoff, digest-only one-use challenges, destructive
invalid and expired verification, visible/hidden/new-user branches, pending-invite provisioning,
Cap NanoID aliases, replace-all mobile keys, exact redirect and Bearer parsing behavior, stale
authority rollback, receipts, audit, and foreign-key integrity. Session-key mint and revoke are
production-enabled. Email request and new-user verification remain fail-closed behind explicit
provider execution, so this evidence does not claim deployed email, Stripe, or Cloudflare success.

Migration `0052_legacy_mobile_bootstrap_caps_expand.sql`, its 14 checked-in statements, focused
Rust suites, and SQLite conformance prove the six mobile bootstrap, list, detail, playback,
download, and delete routes across all five local axes. The evidence covers API-key-first/browser-
session authentication, active-tenant and owner-only projections, JavaScript pagination, exact
metadata/comment/source mapping, best-effort thumbnails and transcripts, path-style private-image
resolution, and host-only one-hour R2 GET signing that leaves `Range` selectable by the player.
Delete acquires the R2 authority, atomically commits the D1 tombstone and immutable continuation,
performs Cap's single bounded prefix cleanup, and only then records completion; a provider failure
therefore leaves the source-compatible deleted/404 retry state. The D1/R2 orchestration and signer
compile locally, but this is not a deployed Cloudflare credential or object observation.

Migration `0054_legacy_developer_api_expand.sql`, its checked query family, focused Rust suites,
and SQLite conformance prove all eleven developer SDK/REST/cron routes across five local axes.
Public/secret key selection, production Origin checks, multipart SigV4, duration-and-size billing,
credit rollback, resumable R2 effect outboxes, and once-per-day snapshots are locally exercised.
All eleven are production-enabled; no production key, credit account, or R2 object was used.

Migration `0055_legacy_transcripts_expand.sql`, its bounded D1 operations/outbox queries, focused
Rust suites, and SQLite conformance prove retry, edit, read, translation listing, and translation
submission semantics. Retry/edit/read/available-translations are production-enabled with exact
D1/R2 authority. Translation records a restart-safe provider intent and remains fail-closed behind
`provider_execution`; no Groq result is fabricated.

Migration `0056_legacy_mobile_uploads_expand.sql`, its 20 checked statements, focused Rust suites,
and SQLite conformance prove released create/progress/complete semantics. Create dual-writes native
and Cap rows and returns an exact no-overwrite R2 PUT target. Progress proves safe-integer
truncation/clamp and tenant non-disclosure. Completion proves exact-key, nonempty-object, and
optional-length admission, then persists an immutable provider-pending intent while the Cap phase
remains `uploading` and `media_jobs` remains empty. Create and progress are production-enabled;
completion truthfully returns `503 provider_execution`. This is local D1/R2 contract evidence, not
a deployed upload or workflow observation.

The committed `contract-cases.json` fixture is decoded by Rust and rejects executor/provider-shape
leakage. The pinned Cap extraction is a snapshot contract for legacy identities; it does not imply
that Frame currently serves the same endpoint path.

## Retry, workflow fault, and local load result

Focused tests exercise duplicate/terminal claims, stale-fence rejection, lease-expiry crash
recovery, one-step checkpoint compare-and-set, bounded retry exhaustion, cancellation, provider
submission fencing, indeterminate-result reconciliation, and partial-provider-failure completion.

The deterministic local fault-load case executes 5,000 workflows. Half are completed normally;
half simulate a crash after checkpoint, reclaim with a higher fence, reject the stale holder, and
complete exactly once. Every late claim observes a terminal result. This checks state-machine
boundedness and race contracts only; it is not an HTTP throughput, D1 contention, Worker duration,
provider-capacity, or SLO measurement.

## Evidence intentionally unavailable

- No operation has remaining repository-local contract work. One hundred sixty rows still lack
  production promotion or retirement authorization because their exact protected evidence has not
  been collected.
- Provider webhook, integration, media, analytics, and billing adapters now have exact D1 replay,
  authorization, redaction, outbox, and immutable evidence contracts. No deployed provider,
  hardware, callback, or failure observation is claimed by those local records.
- Billing/admin rows still lack protected provider-sandbox events, reconciliation and
  partial-failure observations, and accountable reviewer approval.
- Commercial activation and web custom-domain have exhaustive declaration/caller audits but no
  executable handlers. Their only remaining work is the protected repository-owner choice to
  approve a concrete Frame authority contract or retirement; production remains fail-closed.
- Proposed messenger/support retirements lack repository-owner approval, customer impact, export,
  legal/privacy, and dated deprecation evidence.
- Managed media quota/outage, kill-switch, native fallback, and protected cross-executor evidence
  remain dependent on issues 28–29.
- No production-shaped HTTP load, D1 contention, callback storm, cron overlap, provider outage, or
  multi-region observation was run. No production secrets or customer data were used.

These are release blockers for the corresponding rows, not reasons to weaken the local contract.
