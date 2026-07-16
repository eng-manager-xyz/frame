# API and workflow parity v1 — local evidence

Evidence date: 2026-07-16. Reference:
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

This is reproducible local implementation evidence, not endpoint, production, provider, billing,
or retirement sign-off.

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
retirements. All 288 endpoint success/retirement gates remain explicit; none is mislabeled as an
endpoint E2E pass. The report pins every contributing source file by SHA-256 and regenerates
[human-readable documentation](../generated/api-workflow-parity-v1.md).

## Endpoint-adapter and compatibility audit

The generated documentation now derives an executable coverage boundary from every inventory
row instead of leaving the remaining work implicit. The transport inventory is:

| Legacy transport kind | Rows | Executable endpoint success | Exact missing boundary |
|---|---:|---:|---|
| HTTP | 138 method rows / 128 unique paths | 0 | 119 rows are under unversioned `/api/**`; 19 are `/commercial/**` or `/media-server/**`; no row is already under `/api/v1` |
| Effect RPC | 15 | 0 | no `/api/erpc` operation is dispatched by the Rust router |
| Next server action | 121 | 0 | no action identity has a Rust transport adapter and endpoint contract |
| durable workflow | 14 | 0 | shared lease/fence tests cover idempotency/retry only; no legacy workflow invocation adapter has endpoint success evidence |

The current `apps/control-plane/src/routing.rs` router deliberately exposes target-native
`/api/v1` routes. Those routes are useful implementation dependencies, but they do not exactly
match any of the 128 inventoried Cap HTTP paths and therefore cannot be promoted as legacy
endpoint evidence. Unversioned Cap paths currently reach `UnknownApi`/`NotApi`; there is no hidden
redirect or fallback adapter in the Rust dispatcher.

The call-path audit also found these deliberate gaps:

- `ApiGatewayV1::admit_mutation` has focused application tests, but no call from the
  control-plane dispatcher or a retained legacy route adapter. Central policy types therefore do
  not prove centralized admission for checkbox 2.
- `LegacyRoutePolicyV1::decide` is exercised by domain tests only. No production route selects
  `ServeFrameV1`, `UseLegacyFallback`, or `RetirementResponse` from a real client release and row
  evidence record.
- `D1WebhookReplayStoreV1` has a real D1-shaped adapter and local conformance, but no retained
  webhook dispatch path constructs it before a business effect.

The exact evidence-axis counts remain honest:

| Axis | Local contract | Family contract | Dependency pending | Endpoint adapter pending | Protected pending | Retirement pending |
|---|---:|---:|---:|---:|---:|---:|
| success | 0 | 0 | 0 | 263 | 15 | 10 |
| validation | 0 | 200 | 63 | 0 | 15 | 10 |
| authorization | 0 | 200 | 63 | 0 | 15 | 10 |
| idempotency/retry | 14 | 197 | 53 | 0 | 14 | 10 |
| failure | 0 | 200 | 63 | 0 | 15 | 10 |

The current/N-1 fixture proves only the compatibility decision data type. It does not launch a
released client, route a request, assert the endpoint response/side effect, or prove fallback.
Client associations still pending endpoint and current/N-1 E2E evidence are: desktop 31,
developer 21, extension 7, internal worker 19, mobile 22, provider 3, scheduler 16, and web 186.
Associations overlap where one operation serves multiple clients. Consequently checkboxes 2, 7,
and 12 remain local gaps and no endpoint redirect is authorized.

## Local contract and security result

The focused Rust suites cover:

- bounded bodies/content types and required/forbidden idempotency keys;
- authentication, tenant authorization non-disclosure, CSRF/origin decisions, rate-limit errors,
  and redacted correlation/audit data;
- compatibility-decision types and an evidence-gated route-fallback policy in unit tests;
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

- No legacy endpoint family has success/validation/authorization/replay/failure E2E coverage for
  current and N-1 clients, so no compatibility redirect is authorized.
- The D1 replay implementation exists, but no retained provider webhook route is wired to it and no
  deployed D1 callback/replay/failure evidence exists; the memory adapter remains local-only.
- Billing/admin rows lack protected provider-sandbox events, append-only ledger reconciliation,
  refunds/disputes/partial-failure cases, and reviewer approval.
- Proposed messenger/support retirements lack repository-owner approval, customer impact, export,
  legal/privacy, and dated deprecation evidence.
- Managed media quota/outage, kill-switch, native fallback, and protected cross-executor evidence
  remain dependent on issues 28–29.
- No production-shaped HTTP load, D1 contention, callback storm, cron overlap, provider outage, or
  multi-region observation was run. No production secrets or customer data were used.

These are release blockers for the corresponding rows, not reasons to weaken the local contract.
