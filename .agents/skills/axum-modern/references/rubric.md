# Axum modernity rubric

Apply this rubric to every Axum migration, review, and substantial
implementation. Hard gates block completion; the scorecard distinguishes a
compiling change from a complete, repository-shaped result.

## Contents

- [Hard gates](#hard-gates)
- [One-hundred-point scorecard](#one-hundred-point-scorecard)
- [Per-change checklist](#per-change-checklist)
- [Semantic review questions](#semantic-review-questions)
- [Evidence record](#evidence-record)
- [Completion rule](#completion-rule)

## Hard gates

A single failure blocks completion regardless of score.

### G1. Required skill coordination

- Read `rust-modern` and `pragmatic-tiger` completely before acting.
- Keep Rust/Cargo/test mechanics with `rust-modern` and cross-cutting
  scope/risk/performance/dependency judgment with `pragmatic-tiger`.
- Do not claim this rubric passes when a required companion is unavailable.

### G2. Exact compatibility contract

- Record exact Axum/Axum Core and participating HTTP, body, Hyper, Tower, Tower
  HTTP, runtime, and integration-crate versions.
- Record enabled features, targets, server entry points, and deployment topology.
- Distinguish pin-preserving work from an explicit patch-line or release-line upgrade.
- Do not infer direct use from a transitive lockfile entry.

### G3. Primary-source provenance

- Verify changed or uncertain API claims with exact-version docs.rs, packaged or
  tagged source, crates.io, and official changelogs/releases.
- Do not use upstream `main`, search snippets, old examples, or model memory as
  the sole authority for stable code.
- Re-verify the ledger's dated "latest" snapshot for any upgrade decision.

### G4. No deprecated, removed, yanked, or unreleased production API

- The legacy audit has no unexplained production-code match.
- Every affected server target compiles with deprecations denied.
- No compatibility shim reintroduces a removed API under a local name.
- No yanked/prerelease/unreleased API is introduced into a stable target.
- No `deprecated` warning suppression is added.

Generated/vendor code and clearly historical documentation may contain legacy
tokens only when outside production scope and explicitly reported. Fixtures that
prove the audit rejects legacy code are expected exceptions.

### G5. Complete migration disposition

- Inventory every legacy occurrence in scope before declaring the migration done.
- Review every intervening release-line boundary from evidenced source to target.
- Map each occurrence to a verified replacement, deliberate architectural
  removal, or documented no-one-for-one-replacement decision.
- Preserve semantics for route matching, status codes, extraction/rejection,
  body bounds, state scope, middleware order, connection metadata, and shutdown.

### G6. Current Axum semantics

- Route captures use brace syntax and extracted shapes match exactly.
- `Router<S>` is treated as missing state; served routers have state supplied.
- Application state uses `State`; `Extension` is reserved for legitimate
  request/response extension data.
- At most one body-consuming extractor appears last.
- Routed services/middleware have an infallible HTTP boundary.
- `Next`, Body, serving, response, WebSocket, and SSE APIs match the exact target.

### G7. Body, protocol, and lifecycle safety

- Every untrusted buffering path has an intentional effective body limit.
- Optional extraction does not hide malformed input.
- WebSocket/SSE/streaming changes preserve limits, backpressure, cancellation,
  frames/trailers as applicable, and close/disconnect behavior.
- Serving changes preserve startup error handling and bounded graceful shutdown.

### G8. Architecture and dependency contract

- Existing ownership of listeners, routes, authentication, proxy trust, state,
  response policy, control-plane APIs, and target boundaries remains intact
  unless the task explicitly changes it.
- No Axum Extra, Tower HTTP, Hyper Util, protocol feature, or server boundary is
  introduced incidentally.
- Old HTTP-stack generations do not cross the current Axum-facing type boundary.

### G9. Exact-target validation

- Format, tests, lints, and deprecation-denied checks pass for every affected
  native server target.
- Client/Wasm/shared targets are checked when target dependencies or shared
  modules could change.
- Request-level behavior is exercised for routing, extraction, limits,
  middleware, fallbacks, redirects, or response changes.
- Real-listener tests cover upgrades, transport metadata, serving, or shutdown
  when a service-only test cannot prove behavior.

### G10. Claims stay within evidence

- Do not claim "latest," "drop-in," "safe," "bounded," "proxy-aware," or
  "backwards compatible" without evidence appropriate to the claim.
- Report unvalidated features, targets, deployment behavior, and environmental
  blockers explicitly.

## One-hundred-point scorecard

Score each category at full, half, or zero credit. For an inapplicable category,
mark it `N/A`, explain why, remove its points from the denominator, and normalize
the result to 100. Never use `N/A` to bypass a hard gate.

### 1. Version and source accuracy — 15 points

- **0:** Wrong/unknown target, uses an unavailable API, or relies on non-primary guidance.
- **7.5:** Main versions/features are known but one relevant companion, patch,
  target, or source claim remains unresolved.
- **15:** Exact versions/features/targets are recorded and every changed or
  uncertain signature is verified in exact-version primary sources.

### 2. Migration coverage and replacements — 15 points

- **0:** Gives a denylist or bulk rename while skipping relevant release boundaries.
- **7.5:** Source/target and major replacements are identified, but one occurrence,
  point feature, yank, or behavioral caveat is unresolved.
- **15:** Every occurrence and intervening boundary has an actionable,
  behavior-preserving disposition with no stale companion replacement.

### 3. Routing and state — 15 points

- **0:** Uses old path syntax, wrong composition, wrong `Router<S>` meaning, or
  request extensions as accidental global state.
- **7.5:** Routes and state compile but fallback, nesting, trailing slash,
  capture, or layer coverage remains untested/ambiguous.
- **15:** Path/capture semantics, composition, 404/405/fallback behavior, missing
  state, substates, and request extensions are explicit and verified.

### 4. Extraction, bodies, and rejections — 15 points

- **0:** Body consumer order is wrong, buffering is unbounded, or malformed input
  is silently treated as absent.
- **7.5:** Primary extractor flow is current but a limit edge, optional semantic,
  custom rejection, or third-party extractor remains unverified.
- **15:** Extraction role/order, limits and edge tests, custom traits, optional
  semantics, and rejection mapping are current and behaviorally covered.

### 5. Responses and HTTP error contract — 10 points

- **0:** Uses removed body/response forms, leaks internal errors, or changes status semantics accidentally.
- **5:** Responses compile but one header, status, redirect, error, or stream-body
  behavior is implicit.
- **10:** Current `IntoResponse`/Body forms preserve status, headers, client error
  contract, redirect semantics, and streaming requirements.

### 6. Middleware and Tower boundary — 10 points

- **0:** Layer scope/order is wrong or a fallible service error can escape to Hyper.
- **5:** Middleware is current but scope/order, readiness, error conversion, or
  response-body normalization lacks evidence.
- **10:** Abstraction level, route coverage, stack order, infallible boundary,
  URI/backpressure placement, and Tower interoperability are deliberate and tested.

### 7. Serving, upgrades, and lifecycle — 10 points

- **0:** Uses removed server/listener APIs or breaks shutdown/protocol lifecycle.
- **5:** Current serving compiles but connect info, shutdown bounds, upgrade,
  cancellation, or listener behavior is not exercised.
- **10:** Listener/make-service setup, serving, connect info, startup, bounded
  shutdown, and affected protocol lifecycles are proven at the required level.

### 8. Repository fit and evidence — 10 points

- **0:** Crosses architecture boundaries, adds incidental dependencies/features,
  or provides no reproducible validation record.
- **5:** Change is mostly scoped but one target boundary, dependency rationale,
  behavioral test, or evidence item is incomplete.
- **10:** Change is minimal and repository-shaped; boundaries, dependency graph,
  commands/results, behavior, exceptions, and unvalidated items are reproducible.

## Per-change checklist

- [ ] Required sibling skills read; ownership boundaries preserved.
- [ ] Exact Axum and companion versions, features, targets, and topology recorded.
- [ ] Preserve-pin versus upgrade intent explicit.
- [ ] Exact tagged/docs.rs sources used; upstream `main` excluded from stable claims.
- [ ] All published/yanked/intervening releases relevant to migration reviewed.
- [ ] Every legacy occurrence has a disposition.
- [ ] Brace route syntax, capture count, slash behavior, and conflicts verified.
- [ ] Merge/nest/service/fallback method matches semantic role.
- [ ] `Router<S>`, `State`, `FromRef`, and `Extension` roles are correct.
- [ ] When present, the sole body consumer follows all parts-only extractors.
- [ ] Optional extraction separates absence from malformed input.
- [ ] Effective body limit covers each untrusted buffering path.
- [ ] Rejections and application errors map to stable client responses.
- [ ] Response status, headers, redirect, and body semantics are preserved.
- [ ] Middleware abstraction, scope, order, and infallible error boundary verified.
- [ ] Listener, connect info, proxy trust, and shutdown match deployment behavior.
- [ ] WebSocket/SSE/stream limits and lifecycle verified when affected.
- [ ] No incidental dependency, feature, or architecture expansion.
- [ ] Audit and exact-target deprecation-denied compilation pass.
- [ ] Targeted request and real-listener behavior tests pass as applicable.
- [ ] Evidence and anything not validated are reported.

## Semantic review questions

### Routing and state

- Is this a same-level merge, a prefix-stripping nest, or an arbitrary service?
- What should `/path`, `/path/`, unknown path, and wrong method return?
- Does each `Router<S>` still need `S`, or has state already been supplied?
- Is this value global state, a derived substate, or per-request identity?

### Extraction and response

- Does each extractor read only parts or consume the body?
- Is absence valid, or is `Option` hiding malformed/coverage failure?
- Which layer actually enforces the body bound for this extractor?
- Which rejection statuses/bodies are part of the public contract?
- Does a redirect replacement preserve the exact status and method semantics?

### Middleware

- Must policy cover unmatched paths and fallbacks, or matched routes only?
- In what request/response order do layers execute, and which return early?
- Can an error escape the infallible Axum boundary?
- Does URI rewriting or backpressure require wrapping the complete router?

### Serving and protocols

- Does the service need only `axum::serve`, or genuinely require lower-level connection control?
- Is connection metadata installed by the matching make service and interpreted through trusted-proxy policy?
- What happens to in-flight requests, background tasks, streams, and upgrades on shutdown?
- Are frame/message/body bounds and close/disconnect behavior tested?

## Evidence record

Record this compact block in a migration/review handoff:

```text
Compatibility contract:
  axum / axum-core:
  companions:
  features:
  targets and server paths:
  preserve pin or upgrade:

Migration disposition:
  source -> target:
  release boundaries reviewed:
  replacements/removals/no-equivalent decisions:
  audit exceptions:

Behavior verified:
  routes/fallbacks/statuses:
  extraction/rejections/limits:
  middleware order/policy:
  listener/shutdown/protocols:

Commands and results:
  legacy audit:
  fmt/check/clippy/test:
  deprecations denied:
  request/real-listener tests:
  boundary/dependency checks:

Rubric:
  hard gates G1-G10:
  normalized score:
  unvalidated or blocked:
```

## Completion rule

Call the work complete only when:

1. every applicable hard gate passes;
2. the normalized score is at least **90/100**;
3. no score category is zero;
4. every legacy match and validation failure has a documented disposition; and
5. the handoff states exact evidence and limitations.

A lower score can describe an interim review, never a completed modern migration.
