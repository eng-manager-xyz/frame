# Legacy protected billing and authentication contract v1

Frame closes the local portion of the final 17 Cap authentication, billing,
and administrator contracts. Sixteen fail closed until independent external
evidence exists; the developer-checkout CORS preflight is the one local exact
terminal response. The implementation is pinned to Cap commit
`6ba69561ac86b8efdb17616d6727f9638015546b` and its complete machine-readable
inventory is
[`protected-billing-auth.json`](../../fixtures/api-parity/v1/protected-billing-auth.json).

## Inventory

| Operation ID | Carrier | Cap surface | Local authority | Required evidence |
| --- | --- | --- | --- | --- |
| `cap-v1-46bda1c18ffba076` | route | `GET /api/auth/:nextauth*` | public/verified flow-token request | provider |
| `cap-v1-82a39c991fae1050` | route | `POST /api/auth/:nextauth*` | public/verified flow-token request | provider |
| `cap-v1-78537fb518df75ec` | route | `POST /api/desktop/subscribe` | active session or released 36-character API key | human + provider |
| `cap-v1-572763e7b4977abd` | route | `OPTIONS /api/developer/credits/checkout` | anonymous allowlisted CORS origin | local 204 |
| `cap-v1-60b06cc5ab45f187` | route | `POST /api/developer/credits/checkout` | active app owner with credit account | human + provider |
| `cap-v1-af61fa5c8fc453cf` | route | `POST /api/settings/billing/guest-checkout` | anonymous public checkout | human + provider |
| `cap-v1-e596f65c43ee2a82` | route | `POST /api/settings/billing/manage` | active browser session | human + provider |
| `cap-v1-96230bf1f2da3d00` | route | `POST /api/settings/billing/subscribe` | active browser session | human + provider |
| `cap-v1-856dfea22b9d979c` | route | `GET /api/settings/billing/usage` | active browser session | human + provider |
| `cap-v1-1e5f228815a2a8b7` | route | `POST /api/webhooks/stripe` | verified Stripe raw-body signature | human + provider |
| `cap-v1-b2d19e91b05834cf` | route | `POST /api/commercial/checkout` | anonymous public checkout | human + provider |
| `cap-v1-90a6eb69c3fd7b4b` | action | `getVideoReplaceUploadUrl` | pinned messenger administrator + live video | human + provider |
| `cap-v1-e488991f97723847` | action | `invalidateVideoCache` | pinned messenger administrator + live video | human + provider |
| `cap-v1-14ea978608dcf07e` | action | `adminReprocessVideo` | pinned messenger administrator + live video | human + provider |
| `cap-v1-dfd7a4c3d234ccd7` | action | `getPurchaseForMeta` | active browser session | human + provider |
| `cap-v1-0553f2fcdacfe2a9` | action | `manageBilling` | active browser session | human + provider |
| `cap-v1-5a990f470c701cec` | workflow | `adminReprocessVideoWorkflow` | originating pinned administrator + live video | human + provider |

The three administrator video surfaces deliberately reproduce the Cap source
check against `richie@cap.so`; the value comes from
`apps/web/lib/messenger/constants.ts` at the pinned commit. That is local
authorization evidence, not permission to execute an upload, invalidation, or
reprocessing job.

Commercial licensing identities use their released wire prefix. The pinned
contracts declare `/commercial/*`, while Cap's `HttpLive` API mount, homepage
caller, and Next rewrite expose `/api/commercial/*`; therefore checkout is
`cap-v1-b2d19e91b05834cf` at `POST /api/commercial/checkout`, and the separate
declaration-only activation identity is `cap-v1-700b21489623a3e4` at
`POST /api/commercial/activate`.

## Request and replay contract

The application profile fixes method, path, authentication class, rate-limit
bucket, body limit, content type, required fields, idempotency mode, target pointer, provider, and
the SHA-256 manifest for every Cap source file. The adapter then:

1. validates the exact carrier kind and method;
2. authenticates the browser session, Cap API key, public flow, or Stripe signature;
3. validates JSON or source-required form bodies, body limits, and operation-specific values;
4. canonicalizes administrator share URLs to video IDs;
5. persists cleartext only for the source-pinned safe request vocabulary;
   every unknown field or container is digest-redacted, in addition to known
   credential keys such as `accessToken`, `refreshToken`, `clientSecret`, and
   `apiKey`;
6. binds the canonical request to a principal digest and replay-key digest;
   the principal includes a credential discriminator and the exact session
   id/hash-key-version/digest or API-key id/digest revocation tuple;
   secret-bearing NextAuth transports are first sent through the narrow trusted
   request-vault interface, and only their opaque reference plus deterministic
   plaintext-envelope digest may enter D1;
7. reasserts live user/app/video authority inside the write transaction and
   atomically inserts a receipt, provider outbox row, and, when required, a
   human-approval request; browser actions consume their one-use mutation
   grant in that same batch, while Stripe inserts its delivery-audit row; and
8. returns `503 PROTECTED_EXECUTION_EVIDENCE_REQUIRED` with the immutable
   receipt ID.

The preflight is deliberately outside that staging sequence. At the pinned
commit, `corsMiddleware` is registered before `withAuth`; Hono terminates
`OPTIONS` locally. Frame returns `204` without a D1 receipt, reflects only the
configured web origin or the pinned localhost/Tauri origins, sets credentialed
CORS, and keeps hostile origins unreflected. The preflight header list retains
exactly Cap's four pinned request headers; released Cap callers do not send an
`Idempotency-Key` header.

Route POST keys are optional because the released Cap desktop and browser
callers do not supply one. A valid caller key is honored; otherwise Frame
derives an internal five-minute namespace from the operation, principal, exact
body, and digest-only edge/browser context. D1 separately claims
`(operation, principal, request digest)` atomically: a pending claim remains
reachable indefinitely, a terminal claim remains replayable for 15 minutes,
and only then may a later intentional checkout replace it. Concurrent no-key
requests therefore converge even at a namespace boundary. Stripe event IDs and
caller-supplied keys never enter this generated-claim path. Forbidden-key GET
operations reject caller keys but use the same internal generated continuation
instead of an unreachable random nonce. Reusing a caller/natural replay key
with changed request bytes remains a conflict.

Stripe verification uses `HMAC-SHA256` over `timestamp.raw_body`, accepts any
valid `v1` signature in the header, requires lowercase hex, and enforces a
five-minute timestamp window. The principal namespace is the stable verified
Stripe endpoint, not the rotating delivery signature, so newly signed retries
of the same event converge. D1 separately appends each signature-header digest
to an immutable delivery audit, alongside the exact raw-body digest and a
minimized event projection. It never stores the Stripe secret or the event's
customer payload.

NextAuth v4 POSTs accept both JSON and
`application/x-www-form-urlencoded`. Sign-in and sign-out require a CSRF form
token that matches the URL-decoded NextAuth CSRF cookie. GET and POST flows bind
the method, exact decoded path/query/form values, sorted host-only NextAuth
cookies, and digest-only edge/browser context into the principal. GET OAuth or
email callbacks additionally require state/code/id-token/token/error material
and a NextAuth flow cookie. Thus `/session`, `/csrf`, `/signin`, `/signout`,
`/providers`, callbacks, and cookie-less discovery requests never share a
global `None` principal. The exact OAuth/PKCE/CSRF/cookie transport is sealed
outside D1; randomized opaque references are excluded from request identity,
while the deterministic plaintext digest is included. NextAuth routes use the
existing `auth_session.v1` edge-source bucket. Thirteen
human-protected billing/admin operations use `billing_admin.v1`, capped at
eight attempts per minute by principal or, for anonymous checkout, edge
source. Stripe first uses a dedicated 120/minute edge bucket before reading or
HMAC-checking the body, then a separate 120/minute stable verified-endpoint
capacity after signature validation. This replaces the old eight-per-minute
billing bucket without letting rotating signatures reset admission. Missing
D1/hash-key authority fails closed.

## Evidence state machine

The 16 protected operations enter this state machine; the local CORS
preflight never does. Human-protected operations begin with a receipt in
`awaiting_human_approval`, an approval request in `pending`, and an outbox in
`blocked_human_approval`. Provider-only NextAuth operations begin in
`awaiting_provider_evidence` with a provider-pending outbox.

A request-path identity cannot write either evidence table. Database triggers
require:

- `independent_human_approver` evidence with a subject digest, evidence digest,
  and change ticket before a human-blocked outbox can advance;
- an approved human decision, when required, before provider evidence can be
  inserted;
- `independent_provider_executor` evidence bound to the original request and a
  sealed typed-HTTP response digest before provider success can be recorded;
- all required evidence before the receipt can become `verified`.

The receipt insert trigger repeats the current user/app/video authority query
inside the same D1 batch, and replay selection includes that same exact live
credential and resource predicate. Provider-evidence insertion rechecks it at
`verified_at_ms`; administrator workflow execution also requires its exact
parent action, still-live credential, approved request, and human evidence.
Revocation, expiry, app transfer, or video deletion between admission,
staging, replay, and execution therefore cannot create or reveal a protected
result. Server-action batches assert and delete the one-use browser mutation
grant alongside the receipt; an exact replay consumes a fresh grant only after
an in-transaction replay assertion succeeds. Every attempted mutation consumes
its exact grant even when validation, rate limiting, or newly revoked authority
prevents staging; a follow-up accepts only proof that this exact grant is
already absent and never touches an unrelated grant.

Receipts, approval requests, outboxes, generated replay claims, and evidence
rows reject arbitrary updates and deletes. Provider evidence contains only a
validated opaque response reference and plaintext-envelope digest: OAuth
cookies/tokens, checkout and portal URLs, signed upload URLs, and other
capability material cannot enter D1 or `Debug`. Projection remains `503` until
the narrow trusted resolver returns a digest-matching typed status, redirect,
ordered `Set-Cookie` list, content type, and bounded body. A rejected approval
can only produce `rejected`; it can never be converted into provider success.

This state machine never treats any of the following as complete merely
because an intent was staged: checkout creation, subscription mutation,
payment settlement, developer-credit grant, billing-portal creation, session
issuance, email/OAuth completion, upload signing, cache invalidation, or video
reprocessing.

## Carrier wiring

The isolated carrier is
`apps/control-plane/src/legacy_protected_billing_auth_web_runtime.rs`.
The central router maps the nine distinct route shapes above (including
method-specific NextAuth and developer-checkout variants). The local OPTIONS
profile terminates inside `route_response`; the other ten method/path
contracts enter their exact protected carrier.

The authenticated action dispatcher should map these symbols to
`server_action_response`: `getVideoReplaceUploadUrl`, `invalidateVideoCache`,
`adminReprocessVideo`, `getPurchaseForMeta`, and `manageBilling`. It must pass
its already CSRF-authenticated actor and caller idempotency key. The workflow
dispatcher should map `adminReprocessVideoWorkflow` to `workflow_response` and
must preserve the initiating administrator rather than replacing it with an
anonymous scheduler identity.

Required secrets/configuration:

- `STRIPE_WEBHOOK_SECRET` for raw-body webhook verification;
- optional `FRAME_AUTH_FLOW_TOKEN` only when a caller presents
  `X-Frame-Auth-Flow-Token`; public NextAuth discovery may omit it; and
- the existing browser-session configuration used by the shared authenticated
  web runtime; and
- trusted protected-request vault and terminal-response resolver bindings. The
  checked fallback implementations are deliberately unavailable and fail
  closed; released Cap clients are never required to supply a new header.

The developer checkout carrier adds credentialed allowlisted CORS headers to
`POST` outcomes, including failures and evidence gates. `OPTIONS` is anonymous
and locally terminal because the pinned CORS middleware runs before Cap's auth
middleware; CORS itself does not authorize `POST`.

## Verification

Run:

```sh
python3 -I scripts/ci/legacy-protected-billing-auth-sqlite-conformance.py
cargo test --locked -p frame-application legacy_protected_billing_auth --lib
cargo test --locked -p frame-control-plane --lib legacy_protected_billing_auth
```

The SQLite proof checks all 17 source manifests, the 14/2/1
human/provider/local split, atomic
staging rollback, concurrent generated replay claims and terminal retention,
human-before-provider ordering, provider-only NextAuth staging, sealed request
and response non-disclosure, evidence-gated terminal transitions, immutability,
and foreign-key integrity.
