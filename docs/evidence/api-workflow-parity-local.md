# API and workflow parity v1 — local evidence

Evidence date: 2026-07-16. Reference:
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

This is reproducible local implementation evidence for one exact static endpoint and the shared
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
retirements. One source-pinned endpoint success is proven locally and the remaining 287
success/retirement gates remain explicit; none is inferred from a family-level Rust authority. The
report pins every contributing source file by SHA-256 and regenerates
[human-readable documentation](../generated/api-workflow-parity-v1.md).

## Endpoint-adapter and compatibility audit

The generated documentation now derives an executable coverage boundary from every inventory
row instead of leaving the remaining work implicit. The transport inventory is:

| Legacy transport kind | Rows | Executable endpoint success | Exact missing boundary |
|---|---:|---:|---|
| HTTP | 138 method rows / 128 unique paths | 1 | exact `GET /api/status` is promoted; the other 137 method rows still need adapters or protected/retirement evidence |
| Effect RPC | 15 | 0 | no `/api/erpc` operation is dispatched by the Rust router |
| Next server action | 121 | 0 | no action identity has a Rust transport adapter and endpoint contract |
| durable workflow | 14 | 0 | shared lease/fence tests cover idempotency/retry only; no legacy workflow invocation adapter has endpoint success evidence |

The current `apps/control-plane/src/routing.rs` router deliberately exposes target-native
`/api/v1` routes. Those routes are useful implementation dependencies, but they are not promoted
merely because they resemble a Cap path. The sole legacy exception is the explicit
`cap-v1-05b6ba3f76daac22` semantic adapter for exact `GET /api/status`. It is pinned to
`apps/web/app/api/status/route.ts` at SHA-256
`ba3eb1177da489a10f74c9dbc68e0db8324b695c82499e35d6f8d9da8aaf5797` and returns the same Fetch
contract: status `200`, content type `text/plain;charset=UTF-8`, and body `OK`. All other
unversioned Cap paths remain closed; there is no hidden redirect or claimed external fallback.

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

The committed production state remains fail-closed except for that one static adapter: its exact
row has endpoint/client evidence, while no external legacy fallback is proven. The production
registry returns a stable unavailable error for all other rows; the synthetic compatibility suite
enables fallback only to prove current/N-1 routing decisions. A report-driven case proves current
and previous web releases choose Frame only for the pinned status row while all unpromoted rows use
the explicitly synthetic fallback. The call-path audit finds a
production-shaped control-plane transport that constructs the same 288-row registry and a D1
implementation of the atomic execution/idempotency/audit port. The D1 port binds tenant scope,
operation identity, request fingerprint, idempotency key, fenced intent, completion receipt, and
append-only audit using digests only. Its durable semantic-adapter allowlist remains empty. The
typed static allowlist contains only the status operation, executes through the same coordinator
without adding a D1 dependency, and verifies the response receipt digest before rendering. Exact
method/path, empty-body, forbidden-idempotency, source identity, current/N-1, response, and
fail-closed negative cases are tested. No retained webhook route constructs
`D1WebhookReplayStoreV1` before a business effect. This is real centralized admission and durable
execution infrastructure plus one exact static handler, not a claim that 288 business handlers now
exist.

The exact evidence-axis counts remain honest:

| Axis | Local contract | Family contract | Dependency pending | Endpoint adapter pending | Protected pending | Retirement pending |
|---|---:|---:|---:|---:|---:|---:|
| success | 1 | 0 | 0 | 262 | 15 | 10 |
| validation | 0 | 200 | 63 | 0 | 15 | 10 |
| authorization | 0 | 200 | 63 | 0 | 15 | 10 |
| idempotency/retry | 14 | 197 | 53 | 0 | 14 | 10 |
| failure | 0 | 200 | 63 | 0 | 15 | 10 |

The compatibility suite executes current and previous release decisions for all 267 release-managed
client associations, proves both choose Frame only for the one report-promoted status contract and
otherwise choose the explicitly supplied synthetic fallback, and rejects older releases. It does
not launch a released client binary/build, prove any stateful business side effect, or prove the
external legacy fallback. Client associations still pending endpoint evidence are: desktop 31,
developer 21, extension 7, internal
worker 19, mobile 22, provider 3, scheduler 16, and web 185. Associations overlap where one
operation serves multiple clients. No additional endpoint redirect is authorized.

Residual classification is intentionally compound:

| Inventory class | Rows | Remaining local work | Intrinsically protected evidence |
|---|---:|---|---|
| exact static semantic adapter locally proven | 1 | none for the pinned request/response contract; production observation remains a rollout concern | none |
| retained family authority / endpoint adapter pending | 181 | frozen request/response adapter, business effect/effect-specific journal binding, endpoint E2E | none inherent beyond any downstream issue-specific gate |
| media adapter or protected execution pending | 63 | media route/action/workflow adapter and callback semantics | managed provider quota/outage/kill-switch plus required native/hardware execution |
| migration provider adapter pending | 18 | migration/export adapter and stable migration response | approved provider integration execution and credentials |
| billing/admin provider parity pending | 15 | handler, ledger/outbox, reconciliation adapter | billing sandbox events, refunds/disputes/failures, reviewer approval |
| proposed retirement pending | 10 | stable retirement/export response and removal plumbing | repository owner plus legal/privacy/customer-impact approval |

- Checkbox 2 now has bounded local implementation evidence: a typed, source-pinned Rust handler
  runs through the centralized registry/coordinator, and the shared validation, authorization,
  rate-limit, idempotency, stable-error, trace/audit, and atomic D1 execution boundaries remain
  exhaustive and fail-closed. The only promoted operation is exact `GET /api/status`; no family
  authority is mislabeled as another handler. Broad route-by-route semantics remain checkbox 7.
- Checkbox 7 still has local work for per-operation request/response and side-effect semantics.
  The 15 billing/provider rows require protected sandbox/ledger evidence, provider-backed migration
  and media rows retain their named protected gates, and the 10 retirement rows require
  accountable repository-owner/legal/privacy approval.
- Checkbox 12 has local dependency on those endpoint adapters and protected dependency on actual
  current/N-1 released desktop, mobile, extension, developer, and web consumer artifacts. The
  267-case registry suite is necessary compatibility-control evidence, not a substitute for those
  builds.

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
intent/result/audit postconditions, and immutable rows. It records one enabled static semantic
adapter, zero enabled durable semantic adapters, and one promoted endpoint success. This is
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

- The remaining 287 legacy operations do not have exact success/response/side-effect evidence for
  current and N-1 released clients, so no additional compatibility redirect is authorized.
- The D1 webhook replay and generic execution-journal implementations exist, but no retained
  provider webhook/business semantic adapter is wired to them and no deployed D1
  callback/replay/failure evidence exists; those provider endpoint successes remain unproven.
- Billing/admin rows lack protected provider-sandbox events, append-only ledger reconciliation,
  refunds/disputes/partial-failure cases, and reviewer approval.
- Proposed messenger/support retirements lack repository-owner approval, customer impact, export,
  legal/privacy, and dated deprecation evidence.
- Managed media quota/outage, kill-switch, native fallback, and protected cross-executor evidence
  remain dependent on issues 28–29.
- No production-shaped HTTP load, D1 contention, callback storm, cron overlap, provider outage, or
  multi-region observation was run. No production secrets or customer data were used.

These are release blockers for the corresponding rows, not reasons to weaken the local contract.
