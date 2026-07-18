# API and workflow parity v1 — local evidence

Evidence date: 2026-07-16. Reference:
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

This is reproducible local implementation evidence for seven exact static operations, one exact D1
notification-preferences read, one exact D1 business-action contract, and the shared
compatibility boundary, not broad endpoint parity, production, provider, billing, or retirement
sign-off.

## Inventory result

`python3 scripts/ci/check-api-workflow-parity.py --generate --require-reference` passed against the
pinned checkout. The machine-readable report contains 288 distinct operations:

| Kind | Count |
|---|---:|
| HTTP routes | 138 |
| Effect RPC operations | 15 |
| Server actions, including inline actions | 121 |
| Durable workflow/dispatch/recovery entrypoints | 14 |

Disposition totals are 245 replace, 18 migrate, 15 protected-parity-required, and 10 proposed
retirements. Nine source-pinned endpoint contracts are proven locally across all five evidence axes,
and the remaining 279 success/retirement gates remain explicit; none is inferred from a
family-level Rust authority. The report pins every enumerated declaration and implementation-bearing
source by SHA-256 and regenerates
[human-readable documentation](../generated/api-workflow-parity-v1.md).

Every row now carries a closed completion decision in addition to its five evidence axes. Exactly
280 rows name unfinished repository-local adapter or retirement-response work. Of those, 123 also
name genuine provider, hardware, or human-approval gates; 157 are local-only. This split is
machine-validated so a protected gate cannot conceal an unimplemented local route.

The generated `fixtures/api-parity/v1/operation-contract-catalog.json` also assigns all 288
identities one of 54 normalized, source-manifest-bound specifications covering the required
success-or-retirement, validation, authorization, idempotency/retry, and failure cases. This closes
ambiguity in the implementation backlog, not the endpoint gap: the catalog declares itself
specification-only, records only nine locally tested success contracts and eight fully promotable
operations, and preserves all 279 endpoint or retirement gates. The parity checker rejects catalog
entries that promote incomplete work or stop failing closed.

The pinned source closure now includes the concrete catch-all handler for all 22 Mobile route rows
and the HTTP transport, RPC layer, family handler, and called service/policy sources for all 15
Effect RPC rows. The 14-row organization audit additionally pins imported authorization, role,
normalization, plan, and space-membership helpers where they determine behavior. It found no new
provider dependency in those 14 rows. Separately, `OrganisationSoftDelete`
(`cap-v1-5cd4cac9da73f975`) deletes Cap-managed S3 prefixes and Tinybird tenant data; it therefore
has both unfinished local adapter/orchestration work and an explicit `provider_execution` gate.
An exact-ID follow-up pins the minimal directly effect-bearing implementation sources for 36 more
Google Drive, Vercel, Stripe, Resend, Groq, OAuth, Discord, Tinybird, media/storage, Dub, Deepgram,
GitHub, and space-icon-storage-backed rows. Those rows likewise retain unfinished local
adapter/orchestration work plus `provider_execution`; adding a source pin does not silently change
their route taxonomy. The same audit corrects the mobile folder route to session-or-API-key auth,
preserves the released clients' optional idempotency behavior, and records multipart rather than
JSON admission for the four space actions.
`POST /commercial/activate` (`cap-v1-261c3cb23ca88bf9`) is different: the pinned Cap snapshot has
contract declarations but no concrete Frame licensing authority. It remains provider-free and
dependency-pending rather than being assigned a fabricated protected provider.

## Endpoint-adapter and compatibility audit

The generated documentation now derives an executable coverage boundary from every inventory
row instead of leaving the remaining work implicit. The transport inventory is:

| Legacy transport kind | Rows | Executable endpoint success | Exact missing boundary |
|---|---:|---:|---|
| HTTP | 138 method rows / 128 unique paths | 8 | exact `GET /api/status`, `GET /media-server`, `GET`/`OPTIONS` for both `/api/changelog/status` and `/api/changelog`, `GET /api/mobile/session/config`, and `GET /api/notifications/preferences` are promoted; the other 130 method rows still need adapters or protected/retirement evidence |
| Effect RPC | 15 | 0 | no `/api/erpc` operation is dispatched by the Rust router |
| Next server action | 121 | 1 | `updateActiveOrganization` has an exact internal ACTION adapter; 120 remain pending, and its production Leptos ingress is still fail-closed |
| durable workflow | 14 | 0 | shared lease/fence tests cover idempotency/retry only; no legacy workflow invocation adapter has endpoint success evidence |

A source-level audit promoted both `/api/changelog/status` method rows and both `/api/changelog`
method rows only after adding exact query, content-corpus, JSON, empty-204, and CORS-header
transport semantics. The full feed pins every one of the 99 MDX inputs, its 88,817-byte compact
body, and request/configured-origin reflection including the `null` fallback;
media status/health rows depend on live executor state; and the remaining reads, mutations,
actions, RPCs, and workflows need request-shape plus business-effect adapters. Their executable
unavailable response is failure evidence, not a fabricated success.

The current `apps/control-plane/src/routing.rs` router deliberately exposes target-native
`/api/v1` routes. Those routes are useful implementation dependencies, but they are not promoted
merely because they resemble a Cap path. Seven legacy method exceptions are explicit static
adapters and notification preferences is an exact authenticated D1 read; the active-organization operation resolves separately by its exact ACTION identity and
never synthesizes an HTTP path.
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

Exact Navbar `updateActiveOrganization` (`cap-v1-a3b4c805d409bc7c`) is registered only as
`server_action`/`ACTION`. Its test-only internal dispatcher binds the mutation actor to the trusted
session principal, maps the 15-character Cap NanoID to the migration UUID, executes the D1
membership-only active-selection batch, preserves `default_organization_id`, derives the revision
server-side, records `organization_read`, and yields `/dashboard` invalidation followed by a void
result. The mobile PATCH (`cap-v1-05776c542380771e`) is deliberately not promoted: although its
session-or-API-key and owner-or-membership rules are pinned, exact output still requires provider
image signing and nullable-space root-folder projection.

Desktop `GET /api/desktop/org-custom-domain` (`cap-v1-ed9957ac480103b9`) is also deliberately not
promoted. A test-only typed adapter and migration `0030_legacy_org_custom_domain_projection.sql`
preserve Cap's actor-derived active-organization lookup, independent nullable fields, case-sensitive
URL prefixing, exact snake-case JSON, CORS, and runtime ISO timestamp string. The provider-free
SQLite conformance proves projection integrity and fail-closed reads. Production routing remains
blocked until a source-pinned import/backfill proves that every existing active organization has a
lossless projection row; the pinned Cap runtime's ISO string also conflicts with its stale declared
boolean schema, so Frame does not silently coerce the value.

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

The committed production state remains fail-closed except for those seven static adapters and the
notification-preferences D1 read: their exact rows have endpoint/client evidence, while no external legacy fallback is proven. Every promoted ingress now enforces its reported bucket through the
keyed-digest, bounded D1 authority in migration `0034_compatibility_rate_limits.sql`; missing
limiter authority fails closed and a saturated bucket reaches the typed rate-limit error. The
active-organization business action also has exact local evidence, but remains internal until a
Leptos ingress consumes its dashboard-invalidation-then-void effect. The production registry returns
a stable unavailable error for all unpromoted rows; a report-driven runtime test admits the exact
primary caller for each of the 279 unpromoted identities and proves every one
returns `temporarily_unavailable`. The synthetic compatibility suite enables fallback only to prove
routing decisions. A report-driven case proves current and previous
web releases choose Frame only for the pinned status, notification preferences, and changelog OPTIONS rows, desktop releases
choose Frame only for both changelog GET rows, the internal-worker caller chooses Frame only for the pinned
media-server root, and all unpromoted rows use the explicitly synthetic fallback. The call-path
audit finds a
production-shaped control-plane transport that constructs the same 288-row registry and a D1
implementation of the atomic execution/idempotency/audit port. The D1 port binds tenant scope,
operation identity, request fingerprint, idempotency key, fenced intent, completion receipt, and
append-only audit using digests only. Its durable semantic-adapter allowlist remains empty. The
typed semantic allowlist contains exactly the status, media-server-root, two changelog-status, two
full changelog-feed operations, mobile session config, and notification preferences. Its seven static response bodies have no D1 business-data dependency, but their production ingresses require the shared limiter; the notification read additionally uses its actor-bound D1 authority, and
verifies each response receipt digest before rendering. Exact method/path/query, empty-body,
forbidden-idempotency, source identity, authorization, rate-limit, retry, response/header, and
fail-closed negative cases are tested. No retained
webhook route constructs
`D1WebhookReplayStoreV1` before a business effect. This is real centralized admission and durable
execution infrastructure plus seven exact static operations, one exact D1 preference read, and one
exact D1 business action, not a claim that 288 business handlers now
exist.

The exact evidence-axis counts remain honest:

| Axis | Local contract | Family contract | Dependency pending | Endpoint adapter pending | Protected pending | Retirement pending |
|---|---:|---:|---:|---:|---:|---:|
| success | 9 | 0 | 0 | 254 | 15 | 10 |
| validation | 9 | 191 | 63 | 0 | 15 | 10 |
| authorization | 9 | 191 | 63 | 0 | 15 | 10 |
| idempotency/retry | 23 | 188 | 53 | 0 | 14 | 10 |
| failure | 9 | 191 | 63 | 0 | 15 | 10 |

The compatibility suite executes current and previous release decisions for all 267 release-managed
client associations, proves both choose Frame only for the report-promoted status, changelog, and
mobile session-config
contracts and otherwise choose the explicitly supplied synthetic fallback, and rejects older
releases. The
separate internal-worker association reaches only the exact media-server metadata adapter. It does
not launch a released client binary/build, prove any stateful business side effect, or prove the
external legacy fallback. Client associations still pending endpoint evidence are: desktop 29,
developer 21, extension 7, internal worker 18, mobile 21, provider 3, scheduler 16, and web 181.
Associations overlap where one
operation serves multiple clients. No additional endpoint redirect is authorized.

Residual classification is intentionally compound:

| Inventory class | Rows | Remaining local work | Intrinsically protected evidence |
|---|---:|---|---|
| exact static semantic adapters locally proven | 7 | none for the pinned request/response contracts; production observation remains a rollout concern | none |
| exact D1 notification-preferences read locally proven | 1 | none for the pinned request/response contract; production observation remains a rollout concern | none |
| retained local-only/dependency/ingress-pending rows | 157 | frozen request/response adapter, concrete authority where missing, business effect/effect-specific journal binding, endpoint E2E | none established by the pinned operation graph |
| mobile active-organization bootstrap provider effect pending | 1 | exact fresh-bootstrap adapter and nullable-space root-folder projection | provider image signing |
| exact-ID transitive provider effects pending | 36 | exact adapter, provider-effect orchestration, effect-specific journal binding, endpoint E2E | pinned Google Drive, Vercel, Stripe, Resend, Groq, OAuth, Discord, Tinybird, media/storage, Dub, Deepgram, GitHub, space-icon storage, and organization-deletion effects |
| independently classified media protected execution pending | 43 | media route/action/workflow adapter and callback semantics | managed provider quota/outage/kill-switch plus required native/hardware execution |
| migration provider adapter pending | 18 | migration/export adapter and stable migration response | approved provider integration execution and credentials |
| billing/admin provider parity pending | 15 | handler, ledger/outbox, reconciliation adapter | billing sandbox events, refunds/disputes/failures, reviewer approval |
| proposed retirement pending | 10 | stable retirement/export response and removal plumbing | repository owner plus legal/privacy/customer-impact approval |

- Checkbox 2 now has bounded local implementation evidence: a typed, source-pinned Rust handler
  runs through the centralized registry/coordinator, and the shared validation, authorization,
  rate-limit, idempotency, stable-error, trace/audit, and atomic D1 execution boundaries remain
  exhaustive and fail-closed. The only promoted operations are exact `GET /api/status`,
  `GET /media-server`, `GET`/`OPTIONS` for `/api/changelog/status` and `/api/changelog`,
  `GET /api/mobile/session/config`, `GET /api/notifications/preferences`, and the exact internal
  active-organization ACTION; no family authority is mislabeled
  as another handler. Broad route-by-route semantics remain checkbox 7.
- Checkbox 7 still has local work for per-operation request/response and side-effect semantics.
  The exact unfinished completion residual is 157 local-only rows plus 123 rows with both local work and a protected
  provider, hardware, or approval gate. The latter group includes 36 exact-ID transitive provider
  operations, the separate mobile active-organization bootstrap, 43 independently classified
  media-execution rows, 18 migrations, 15 billing/provider rows, and 10 retirement proposals. None
  is presented as route completion.
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
intent/result/audit postconditions, and immutable rows. It records eight enabled semantic adapters
(seven static plus one actor-bound D1 read), zero enabled synthetic durable semantic adapters, and
nine promoted endpoint successes (those eight plus the separately journaled D1 business action). This is
likewise local SQLite/compile evidence, not a deployed D1 or stateful business-effect result.

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

- The remaining 279 legacy operations do not have exact success/response/side-effect evidence for
  current and N-1 released clients, so no additional compatibility redirect is authorized.
- The D1 webhook replay and generic execution-journal implementations exist, but no retained
  provider webhook/business semantic adapter is wired to them and no deployed D1
  callback/replay/failure evidence exists; those provider endpoint successes remain unproven.
- Billing/admin rows lack protected provider-sandbox events, append-only ledger reconciliation,
  refunds/disputes/partial-failure cases, and reviewer approval.
- Commercial activation has only pinned request/response declarations; a concrete Frame licensing
  authority, state model, and exact adapter remain repository-local dependency work.
- Proposed messenger/support retirements lack repository-owner approval, customer impact, export,
  legal/privacy, and dated deprecation evidence.
- Managed media quota/outage, kill-switch, native fallback, and protected cross-executor evidence
  remain dependent on issues 28–29.
- No production-shaped HTTP load, D1 contention, callback storm, cron overlap, provider outage, or
  multi-region observation was run. No production secrets or customer data were used.

These are release blockers for the corresponding rows, not reasons to weaken the local contract.
