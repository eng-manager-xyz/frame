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
disposition classification. `POST /commercial/activate` (`cap-v1-261c3cb23ca88bf9`) is guarded by
a separate invariant: contract declarations alone are dependency evidence, not a concrete Frame
commercial licensing authority and not proof of an external provider.

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
`collaboration_notifications.v1` allows 30 per principal. Public subject keys are HMAC digests of
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

The v1 registry currently contains seven deliberately narrow static promotions, one exact D1 read,
and one exact D1 business-action contract. Exact `GET /api/status`
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

All eight typed semantic adapters run through the bounded typed-adapter registry. The seven static
response adapters have no D1 business-data dependency, while every production ingress requires the
shared D1 rate-limit authority and the preference read additionally requires its real D1 business
authority. The ingress representation owns and bounds the raw body, canonical
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

The ninth exact contract is the Navbar `updateActiveOrganization` server action
(`cap-v1-a3b4c805d409bc7c`). It resolves only as `server_action`/`ACTION` with its frozen
`action://...#updateActiveOrganization` identity; it never enters raw HTTP resolution. Its typed
session boundary derives the actor from the trusted authenticated principal, deterministically maps
the Cap NanoID, and calls a D1 business adapter whose atomic batch updates only the active
organization, preserves the default, derives the revision server-side, and journals
`organization_read`. The internal completion effect requires `/dashboard` invalidation followed by
a void action result. The contract is locally proven, but production remains fail-closed until a
Leptos server-action ingress consumes that effect.

The related mobile PATCH (`cap-v1-05776c542380771e`) remains unpromoted. Its session-or-API-key and
owner-or-membership rules are typed, but the exact fresh bootstrap still depends on provider image
URL signing and Cap root folders with nullable `spaceId`; Frame does not approximate either output.

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
orchestration and protected provider execution; the presence of a local organization authority does
not make that row local-only.

The same compound completion rule applies to the 36 exact transitive-provider identities locked in
the generator. They remain unavailable until both repository-local adapter/orchestration work and
protected provider execution are complete. Commercial activation remains unavailable for a
different reason: Frame still needs a concrete licensing authority and exact adapter before any
provider question can be evaluated.

Operation-level transport overrides are source-pinned rather than inferred from route families.
The mobile folder-create route therefore records session-or-API-key authentication and optional
idempotency, the folder RPCs preserve optional idempotency, and the four space actions accept
bounded `multipart/form-data` instead of JSON. Those space actions also pin their optional icon
storage and SVG-sanitization graph and retain a protected provider-execution gate.

Except for the seven exact static adapters and the locally proven business action above, until those fields become evidence-backed
`local_contract` (and protected evidence where required), a proven route-level fallback remains
authoritative; in the current production registry, an unproven fallback also fails closed.
