---
name: axum-modern
description: Write, review, debug, or migrate Axum applications with the repository-pinned modern API surface. Use whenever work touches Axum routers, routes, handlers, state, extractors, responses, middleware, body limits, WebSockets, SSE, serving, graceful shutdown, dependency features, or Axum integration crates. Reject deprecated and removed APIs, map every historical Axum release line to current replacements, and verify each affected server target. Always pair with rust-modern for Rust and Cargo rules and pragmatic-tiger for cross-cutting engineering judgment.
---

# Axum Modern

Use the newest non-deprecated Axum idiom supported by the repository's pinned
dependency. Treat the exact version, enabled features, companion crates, target,
and runtime topology as one compatibility contract.

## Coordinate the three skills

Before handling an Axum task, locate and read these companion skills completely:

1. `../rust-modern/SKILL.md`
2. `../pragmatic-tiger/SKILL.md`

They are required, not optional background. If either companion is unavailable,
report it and do not claim this skill's rubric is satisfied. Do not reconstruct
or copy its policy into this skill as a fallback.

Apply authority in this order:

1. Explicit user requirements and repository instructions
2. `rust-modern` for Rust syntax, types, APIs, Cargo, errors, and test mechanics
3. This skill for Axum routing, state, extraction, response, middleware, serving,
   and migration decisions
4. `pragmatic-tiger` for risk, scope, invariants, performance, dependencies, and debt

Keep those ownership lines explicit. Identify a conflict and follow the higher
authority instead of silently blending policies.

## Establish the exact target

Do this before selecting an API or applying a migration:

1. Read the applicable manifests, workspace dependencies, lockfile,
   target-specific dependency tables, feature forwarding, build scripts, and
   architecture decisions.
2. Record the exact `axum`, `axum-core`, `tower`, `tower-http`, `hyper`, `http`,
   `http-body`, and runtime versions that actually participate. Do not infer a
   companion version from Axum's version.
3. Record enabled Axum features, target triples, server entry points, deployment
   topology, and whether code is an application, reusable integration library,
   or middleware crate.
4. Determine whether the task preserves the pin, upgrades within a release line,
   or crosses release lines. Never upgrade implicitly.
5. Prefer repository-pinned APIs over internet examples. For an explicit upgrade
   or "latest" request, verify crates.io, exact-version docs.rs, the official
   changelog, official releases, and tagged source before editing.
6. Verify any claimed source version from its manifest, lockfile, and syntax.
   Historical snippets are often mislabeled.

The dated release snapshot in [migration-ledger.md](references/migration-ledger.md)
is this skill's recorded authority for "latest." Re-verify it before every
upgrade decision; upstream `main` may describe an unreleased breaking line.

## Load only the references needed

- Read [project-profile.md](references/project-profile.md) for work in this
  repository; otherwise derive an equivalent profile from the target repository.
- Read [routing-and-state.md](references/routing-and-state.md) for route syntax,
  composition, fallbacks, `Router<S>`, `State`, `FromRef`, and request extensions.
- Read [extractors-and-responses.md](references/extractors-and-responses.md) for
  extractor order, body limits, custom extractors, rejections, and responses.
- Read [middleware-and-serving.md](references/middleware-and-serving.md) for layer
  scope/order, Tower interoperability, error handling, listeners, connect info,
  shutdown, WebSockets, SSE, and service tests.
- Read [migration-ledger.md](references/migration-ledger.md) for any legacy code,
  version change, removed symbol, uncertain example, or historical claim.
- Read [rubric.md](references/rubric.md) for every migration, review, or substantial
  implementation.

## Choose by semantic role

- Use a handler for endpoint behavior, a custom extractor for reusable request
  validation/conversion, and middleware for policy that wraps multiple endpoints.
- Use `State` plus `Router::with_state` for application state. Use `Extension`
  only for request extensions that middleware inserts or responses propagate.
- Use `FromRequestParts` when extraction does not consume the body and
  `FromRequest` when it does. Put the sole body-consuming extractor last.
- Return types implementing `IntoResponse`; use a domain error's `IntoResponse`
  implementation when handlers share a stable HTTP error contract.
- Use `middleware::from_fn` or `from_fn_with_state` for application-local async
  middleware. Use Tower services/layers for reusable or lower-level middleware.
- Use `axum::serve` for the supported simple listener path. Drop to Hyper or
  `hyper-util` only when the required connection control exceeds `serve`.
- Depend on `axum-core` rather than `axum` in integration libraries that only
  need core extractor/response traits.

## Enforce modernity

1. Reject deprecated APIs in changed production code even when they compile.
2. Reject removed APIs, copied compatibility shims, colon/wildcard *capture*
   syntax, pre-0.7 request-body generics, and pre-0.7 server/body forms. A
   deliberate literal segment beginning `:` or `*` is a current but manual-review
   exception, not a legacy capture.
3. Replace every legacy occurrence with the exact target-version mapping in the
   migration ledger. Treat behavior-sensitive mappings as review work, not bulk
   search-and-replace.
4. Keep prerelease and unreleased APIs out of stable code unless the repository
   explicitly opts into that exact release.
5. Preserve current APIs that resemble an older pattern. `Extension` remains
   valid for request extensions, and `into_make_service_with_connect_info`
   remains valid when connection metadata is required.
6. Do not suppress `deprecated` warnings. If a dependency makes the clean build
   impossible, isolate or upgrade that dependency within the authorized scope;
   otherwise report the blocker.

Run the heuristic audit from the repository root, narrowing paths when useful:

```sh
.agents/skills/axum-modern/scripts/audit-legacy-api.sh path/to/axum/code [more/paths ...]
```

Then compile every affected native server target with deprecations denied. The
exact-version compiler and primary documentation are authoritative; the script
only identifies candidates.

## Implement in dependency order

For migrations, use this order:

1. Version, features, and companion-crate contract
2. Imports and HTTP/body types
3. Route syntax and router composition
4. Application state and request extensions
5. Extractors, body limits, and rejections
6. Responses and application error mapping
7. Middleware scope, order, and error conversion
8. Listener, connect info, serving, and shutdown
9. Protocol upgrades/streams, tests, examples, and legacy scan

Keep the patch repository-shaped. Do not introduce Tower HTTP, Axum Extra,
WebSockets, SSE, custom extractors, or a new server boundary merely because Axum
supports them.

## Validate the actual server paths

At minimum:

1. Run the legacy audit and disposition every match.
2. Format and run the repository's targeted tests and lints.
3. Compile each affected server target with deprecations denied.
4. Compile client/Wasm or shared targets when dependency boundaries could change.
5. Exercise requests through the router when paths, extraction, middleware,
   response policy, body limits, fallbacks, or redirects change.
6. Exercise startup, graceful shutdown, and upgrade/stream lifecycles when their
   code changes.
7. Apply every hard gate and the scorecard in [rubric.md](references/rubric.md).

Report the exact versions/features/targets, legacy mappings, commands and
results, behavior exercised, and anything not validated. Do not call work
complete while a hard gate fails.
