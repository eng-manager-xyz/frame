---
name: leptos-modern
description: Write, review, debug, or migrate Leptos applications with the repository-pinned modern API surface. Use whenever work touches Leptos components, views, signals, effects, resources, actions, stores, routing, server functions, SSR, hydration, CSR, islands, or Leptos dependency/features. Reject deprecated and removed APIs, map legacy code to current replacements, and verify each enabled rendering target. Always pair with rust-modern for Rust and Cargo rules and pragmatic-tiger for cross-cutting engineering judgment.
---

# Leptos Modern

Use the newest non-deprecated Leptos idiom supported by the repository's pinned
dependency. Treat the pin, enabled features, compilation target, and rendering
mode as one compatibility contract.

## Coordinate the three skills

Before handling a Leptos task, locate and read these companion skills completely:

1. `../rust-modern/SKILL.md`
2. `../pragmatic-tiger/SKILL.md`

They are required, not optional background. If either companion is unavailable,
report it and do not claim this skill's rubric is satisfied. Do not reconstruct
or copy its policy into this skill as a fallback.

Apply authority in this order:

1. Explicit user requirements and repository instructions
2. `rust-modern` for Rust syntax, types, APIs, Cargo, errors, and test mechanics
3. This skill for Leptos APIs, reactive semantics, views, rendering modes, and migrations
4. `pragmatic-tiger` for risk, scope, invariants, performance, dependencies, and debt

Do not copy rules owned by the sibling skills into this skill. If policies
conflict, identify the conflict and follow the higher authority rather than
silently blending them.

## Establish the exact target

Do this before selecting an API or applying a migration:

1. Read the applicable `Cargo.toml`, workspace dependencies, `Cargo.lock`, target-specific dependency tables, feature forwarding, build scripts, and architecture decisions.
2. Record the exact `leptos` version, companion-crate versions, enabled features, Rust target, and each valid mode: `csr`, `hydrate`, `ssr`, or intentional islands.
3. Verify any claimed source version against its manifest, lockfile, and syntax. Historical snippets are often mislabeled; migrate from the evidence, not the label.
4. Determine whether the task preserves the pin, upgrades within a release line, or migrates release lines. Never upgrade implicitly.
5. Prefer repository-pinned APIs over internet examples. For an explicit upgrade or "latest" request, verify crates.io, exact-version docs.rs pages, official releases, and source before editing.
6. Resolve every Leptos-family crate independently. Workspace release tags and companion crates do not necessarily share the `leptos` crate's patch number.

Treat the dated release snapshot in `references/migration-ledger.md` as the sole
"latest" authority in this skill. Re-verify it before every upgrade decision.

## Load only the references needed

- Read [reactivity.md](references/reactivity.md) for signals, memos, effects, resources, actions, stores, ownership, and local versus sync storage.
- Read [views-and-rendering.md](references/views-and-rendering.md) for components, children, events, lists, control flow, HTML, CSR, SSR, hydration, islands, and mounting.
- Read [full-stack.md](references/full-stack.md) for routing, server functions, custom errors, integrations, streaming, forms, and WebSockets.
- Read [migration-ledger.md](references/migration-ledger.md) for any legacy code, version change, deprecation, removed symbol, or uncertain historical example.
- Read [project-profile.md](references/project-profile.md) when working in this repository; otherwise derive an equivalent profile from the target repository.
- Read [rubric.md](references/rubric.md) for every migration, review, or substantial implementation.

## Choose by semantic role

- Use a signal for mutable reactive state, a cheap derived closure for cheap derivation, and a memo only when caching or downstream change suppression is valuable.
- Use an effect only to synchronize reactive state with a non-reactive external system. Do not derive application state with effects.
- Use a resource for reactive asynchronous loading, an action for explicitly dispatched work, and a server function only when the application intentionally adopts that transport and integration.
- Use `Resource` for SSR-aware serializable data and `LocalResource` for browser-local or `!Send` work. Keep local-only browser types out of server paths.
- Use keyed `<For>` for changing collections and a direct iterator for stable collections.
- Use standard routing by default. Enable `islands-router` only for an intentional islands architecture.

## Enforce modernity

1. Reject deprecated APIs in all changed production code even when the compiler still accepts them.
2. Reject removed APIs, copied compatibility shims, and context-parameter-era syntax.
3. Replace each legacy occurrence using the exact target-version mapping in the migration ledger. Treat context-sensitive mappings as review items, not search-and-replace rules.
4. Keep prerelease-only APIs out of stable code unless the repository explicitly opts into the prerelease.
5. Preserve legitimate current APIs that resemble historical workarounds. For example, remove `LocalResource` wrapper-era `.as_deref()` only when it solely unwraps the old `SendWrapper`.
6. Treat deprecation warnings as errors for the affected target. The compiler and exact-version documentation are authoritative; the audit script is a fast heuristic.

Run the audit from the repository root, narrowing the path when appropriate:

```sh
.agents/skills/leptos-modern/scripts/audit-legacy-api.sh path/to/leptos/code [more/paths ...]
```

Then compile or check every affected feature/target with deprecations denied.
Prefer the repository's existing command and add `-D deprecated` through its
supported lint mechanism or a scoped `RUSTFLAGS` invocation. Do not make a
workspace-wide build configuration change merely to run this check.

## Implement in dependency order

For migrations, use this order:

1. Version and feature contract
2. Imports and prelude
3. Reactive primitives and ownership
4. Components, props, children, and views
5. Resources, actions, and async boundaries
6. Router and server integrations
7. SSR shell, hydration entry points, and browser mounting
8. Tests, examples, documentation, and legacy scan

Keep the patch scoped. Do not introduce a router, full-body hydration, server
functions, islands, or a dependency merely because Leptos supports it.

## Validate the actual modes

Run checks separately for each affected mode. A successful native SSR build does
not validate a Wasm hydration build, and a CSR build does not validate server
boundaries.

At minimum:

1. Run the legacy audit and inspect every match.
2. Format and run the repository's targeted tests and lints.
3. Check native `ssr` if affected.
4. Check `wasm32-unknown-unknown` with `hydrate` and/or `csr` if affected.
5. Exercise SSR plus hydration when initial HTML or async rendering changed; require valid, deterministic, matching initial markup.
6. Apply the hard gates and scorecard in `references/rubric.md`.

Report the version/features/modes used, legacy mappings made, commands run, and
anything not validated. Do not call a migration complete while a hard gate fails.
