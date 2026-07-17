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

Catch-all Next files are recorded as transport wrappers rather than duplicate product operations.
Ordinary helpers, UI routes, and native desktop IPC are explicitly excluded. When the ignored
pinned Cap checkout exists, the checker re-extracts all rows and compares source checksums and the
entire classified report. Offline CI still validates the committed schema, identities, evidence
axes, generated documentation, dispositions, and retirement records.

Each row records legacy path or operation, methods, symbols and source hashes, client families,
auth class, policy version, `frame.api.v1`, issue owners, Rust authority, disposition, body/rate/
idempotency policy, deprecation state, and five independent evidence axes: success, validation,
authorization, idempotency/retry, and failure.

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

The v1 registry currently contains one deliberately narrow promotion:
`cap-v1-05b6ba3f76daac22`, exact `GET /api/status`, pinned to the Cap source hash recorded in the
report. Its typed static adapter returns `200`, `text/plain;charset=UTF-8`, body `OK` through the
common coordinator without acquiring a D1 dependency. The semantic-adapter allowlist, source
identity, empty-body/forbidden-idempotency request policy, response digest, current/N-1 decision,
and hostile-path negatives are tested together. The durable-adapter allowlist remains empty; no
other report row inherits this evidence by route family or implementation authority.

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

Except for the exact status adapter above, until those fields become evidence-backed
`local_contract` (and protected evidence where required), a proven route-level fallback remains
authoritative; in the current production registry, an unproven fallback also fails closed.
