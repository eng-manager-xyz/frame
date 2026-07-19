# Leptos migration ledger

This ledger inventories every published `leptos` crate version through
2026-07-19 and maps release-line changes to the current stable 0.8.20 surface.
It is a migration index, not a substitute for compiling against the exact pin.

## Contents

- [How to use the ledger](#how-to-use-the-ledger)
- [Release truth and current baseline](#release-truth-and-current-baseline)
- [Complete published version inventory](#complete-published-version-inventory)
- [Release-line migration history](#release-line-migration-history)
- [Current 0.8 deprecation replacement table](#current-08-deprecation-replacement-table)
- [Removed and contextual legacy patterns](#removed-and-contextual-legacy-patterns)
- [0.9 prerelease watchlist](#09-prerelease-watchlist)
- [Primary sources](#primary-sources)

## How to use the ledger

1. Identify the source and target `leptos` versions plus every companion crate.
2. Read each intervening release-line row; do not jump directly to symbol renames.
3. Search for both deprecated and removed patterns.
4. For every match, verify the replacement in the target version's docs/source
   and preserve its reactive, async, SSR, ownership, and error semantics.
5. Compile each real target/mode with deprecations denied.

Statuses used below:

- **Deprecated**: accepted by the current stable compiler but forbidden by this
  skill in changed production code.
- **Removed**: unavailable on current stable; a migration is required.
- **Contextual**: no universal replacement; inspect architecture and behavior.
- **Historical only**: describes an intermediate migration. The same method name
  may have a legitimate new meaning in current trait APIs.

## Release truth and current baseline

Use crates.io and exact-version docs.rs pages as dependency/API truth. GitHub
workspace tags are useful narrative sources but are not a reliable crate-version
resolver: the workspace crates version independently. Notable examples are a
`v0.6.0` tag with no published `leptos` 0.6.0 crate and no published 0.8.18 crate,
despite workspace release/tag history around those numbers.

Verified snapshot:

- Latest stable `leptos`: **0.8.20**, published 2026-06-25, MSRV 1.88.
- Latest prerelease: **0.9.0-beta**, published 2026-07-18; opt-in only.
- Current compatible companion versions are not all 0.8.20. At this snapshot:
  `leptos_router` 0.8.14, `leptos_meta` 0.8.6, `leptos_axum` 0.8.10,
  `leptos_actix` 0.8.7, `server_fn` 0.8.13, `reactive_stores` 0.4.3, and
  `cargo-leptos` 0.3.7.

Re-resolve all of these before an upgrade; do not turn this snapshot into a
workspace-wide exact pin without checking compatibility and project policy.

## Complete published version inventory

Every version string here was read from the crates.io index. `†` means yanked.
Missing numbers called out below were not published for the `leptos` crate.

| Release line | Published versions |
| --- | --- |
| 0.0 | `0.0.1`, `0.0.3`, `0.0.4`, `0.0.5`, `0.0.6†`, `0.0.7`, `0.0.8`, `0.0.9`, `0.0.10`, `0.0.11`, `0.0.12`, `0.0.13†`, `0.0.14`, `0.0.15`, `0.0.16`, `0.0.17`, `0.0.18`, `0.0.19`, `0.0.20`, `0.0.21`, `0.0.22`; no `0.0.2` |
| 0.1 | `0.1.0-alpha`, `0.1.0-beta`, `0.1.0`, `0.1.1`, `0.1.2`, `0.1.3` |
| 0.2 | `0.2.0-alpha`, `0.2.0-alpha2`, `0.2.0-beta`, `0.2.0`, `0.2.1`, `0.2.2`, `0.2.3`, `0.2.4`, `0.2.5` |
| 0.3 | `0.3.0-alpha`, `0.3.0-alpha2`, `0.3.0`, `0.3.1` |
| 0.4 | `0.4.0`, `0.4.1`, `0.4.2`, `0.4.3`, `0.4.4`, `0.4.5`, `0.4.6`, `0.4.7`, `0.4.8`, `0.4.9`, `0.4.10` |
| 0.5 | `0.5.0-alpha`, `0.5.0-alpha2`, `0.5.0-beta`, `0.5.0-beta2`, `0.5.0-rc1`, `0.5.0-rc2`, `0.5.0-rc3`, `0.5.0`, `0.5.1`, `0.5.2`, `0.5.3`, `0.5.4`, `0.5.5†`, `0.5.6†`, `0.5.7` |
| 0.6 | `0.6.0-beta`, `0.6.0-rc1`, `0.6.1`, `0.6.2`, `0.6.3`, `0.6.4`, `0.6.5`, `0.6.6`, `0.6.7`, `0.6.8`, `0.6.9`, `0.6.10`, `0.6.11`, `0.6.12`, `0.6.13`, `0.6.14`, `0.6.15`; no stable `0.6.0` crate |
| 0.7 prerelease | `0.7.0-preview†`, `0.7.0-preview2†`, `0.7.0-alpha`, `0.7.0-beta`, `0.7.0-beta2`, `0.7.0-beta4`, `0.7.0-beta5`, `0.7.0-beta6†`, `0.7.0-beta7`, `0.7.0-gamma`, `0.7.0-gamma2`, `0.7.0-gamma3`, `0.7.0-rc0`, `0.7.0-rc1`, `0.7.0-rc2`, `0.7.0-rc3`; no `beta3` |
| 0.7 stable | `0.7.0`, `0.7.1`, `0.7.2`, `0.7.3`, `0.7.4`, `0.7.5`, `0.7.6`, `0.7.7`, `0.7.8` |
| 0.8 prerelease | `0.8.0-alpha`, `0.8.0-beta`, `0.8.0-rc1`, `0.8.0-rc2`, `0.8.0-rc3` |
| 0.8 stable | `0.8.0`, `0.8.1`, `0.8.2`, `0.8.3`, `0.8.4`, `0.8.5`, `0.8.6`, `0.8.7†`, `0.8.8`, `0.8.9`, `0.8.10`, `0.8.11`, `0.8.12`, `0.8.13`, `0.8.14`, `0.8.15`, `0.8.16`, `0.8.17`, `0.8.19`, `0.8.20`; no published `0.8.18` |
| 0.9 prerelease | `0.9.0-alpha`, `0.9.0-beta` |

Do not select a yanked version for new work. When a lockfile already contains one,
assess the reason and migrate under the repository's dependency policy.

## Release-line migration history

### 0.0 through 0.1: initial reactive component model

These releases established typed HTML/views, `IntoView`, signals, component props,
and server functions. Historical code commonly passes `Scope`/`cx`, calls
`create_signal(cx, value)`, uses `create_rw_signal`, `store_value`, or
`MaybeSignal`, and puts a `view` prop on `<For>`.

Direct-to-current action: remove the context parameter and use owner-scoped
constructors (`signal`, `RwSignal::new`, `StoredValue::new`/`new_local`,
`Signal`). Put `<For>` item rendering in children. Re-evaluate ownership; do not
only delete the `cx` token.

### 0.2: SSR modes, document metadata, and routing growth

0.2 introduced the four SSR strategies, `Html`/`Body` metadata, redirects, and
typed route paths. Historical APIs included `resource.read(cx)`, `.with(cx, ...)`,
and `NodeRef::new(cx)`. Configuration renamed `site_address` to `site_addr`.

Direct-to-current action: use context-free trait methods and `NodeRef::new()`.
Select the current integration rendering strategy deliberately. Treat the old
resource method migration as historical: current reactive traits may legitimately
provide `.read()`/`.with()` again with new signatures.

### 0.3: slots, styles, typed events, and richer server functions

0.3 added slots, style handling, typed window events, resource updates,
`expect_context`, GET/custom server encodings, complex server-function arguments,
HTTP methods, and partially blocked rendering. Deprecated APIs were removed
during this line.

Direct-to-current action: use current component slots and typed event APIs; use
typed server-function `input`/`output`/`protocol` configuration instead of the
legacy string `encoding` form; verify partially blocked behavior in the current
integration rather than copying an old renderer call.

### 0.4: server integrations and stable-by-default tooling

0.4 adopted Axum `State`/substates, made stable Rust the default, moved CSR to an
explicit opt-in, added automatic server-function registration, `<Await>`, form
bindings, and async routing capabilities.

Direct-to-current action: use the repository's integration and feature model.
Do not copy old explicit registration handlers when current `leptos_routes`
already owns them. The historical component child binding changed to `let:` in
0.5; do not confuse it with today's element `bind:value`/`bind:checked` syntax.

### 0.5: owner model and context-parameter removal

0.5 was the first major migration boundary: `Scope`/`cx` was removed in favor of
reactive ownership; islands and SSG arrived; attributes could be spread;
components became more Rust-like and generic; callbacks were introduced;
`<For view=...>` became children; component child binding became `let:`; and
`generate_route_list` became synchronous. The feature `experimental-islands`
became `islands`.

At this transition, resource `.read()` became `.get()` and resolved-value
`.with()` became `.map()`. That is a historical 0.4-to-0.5 rule, not a current
blanket ban on the names `read` or `with`.

Direct-to-current action: reconstruct owner/lifetime intent, remove context args,
use current constructors and children, use the `islands` feature only for an
intentional islands architecture, and do not `.await` current
`generate_route_list`.

### 0.6: server_fn rewrite and modern server frameworks

0.6 rewrote `server_fn` around streaming/custom codecs, structured errors and
middleware, and moved the Axum integration to Axum 0.7. The first published stable
crate in this line was 0.6.1.

Direct-to-current action: migrate old server-function registration, encoding,
middleware, and error patterns through the current typed APIs. If
`leptos_routes` already handles server functions, remove a redundant explicit
`handle_server_fns` route only after checking middleware, context, and path
coverage.

### 0.7: reactive and rendering rewrite

0.7 rewrote the reactive graph and rendering stack. It introduced the current
prelude direction, Rust-like constructors, typed views, `Either`, an explicit
HTML shell, synchronous configuration loading, `hydrate_body`, typed `path!`,
required route fallbacks, `ParentRoute`/`FlatRoutes`, `Send + Sync` defaults with
local variants, resources usable as futures and `<Suspend>`, `Arc*` signals, read/write
guards, reactive stores, current form bindings, and stable islands.

Hydration changed from generated IDs to walking the existing DOM. Valid HTML and
identical deterministic initial markup became hard correctness requirements.

Direct-to-current action: select arena versus `Arc*` lifetime and sync versus
local storage; replace erased/boxed legacy views only when current typed forms fit;
rebuild router definitions with `path!` and fallbacks; migrate old resource,
action, mount, and SSR calls using their exact current signatures. Do not call
APIs "0.8-only" merely because they remain modern: `Effect::watch`,
`Action::new_local`, `on:*:target`, `Either`, and development component erasure
already existed in 0.7.

### 0.8: current stable line

0.8 aligned official integration with Axum 0.8 and added direct custom
server-function errors, typed WebSocket server-function protocols, improved local
resources/actions, and the `islands-router` feature. `Suspend::new` now accepts
values implementing `IntoFuture`. `LeptosOptions` no longer has a `Default`
implementation.

Patches through 0.8.20 added or refined facilities including lazy routes/server
functions in companion crates, `ShowLet` (available from 0.8.8), subsecond
development support, additional codecs, variable-keyed stores, the `Either!`
type macro, lazy preload behavior, nonce construction, and request body limits.
Check the exact crate and patch for each facility; not all are `leptos` features
and not all appeared in 0.8.0.

Direct-to-current action: use the table below, exact companion-crate docs, and
separate SSR/Wasm checks. Do not infer an API's availability from the workspace
release tag alone.

## Current 0.8 deprecation replacement table

These mappings are verified for the current 0.8.20 family. A clean compiler with
`-D deprecated` is the enforcement authority.

| Deprecated API | Preferred API | Required review |
| --- | --- | --- |
| `create_signal(value)` | `signal(value)` | Choose `signal_local` when the value is intentionally `!Send`. |
| `create_rw_signal(value)` | `RwSignal::new(value)` | Choose `new_local` or `ArcRwSignal` for the actual storage/lifetime. |
| `create_trigger()` | `ArcTrigger::new()` | Confirm a data-less notification is still the right model. |
| `create_memo(f)` | `Memo::new(f)` | Do not add a memo when a cheap closure suffices. |
| `create_owning_memo(f)` | `Memo::new_owning(f)` | Preserve ownership and explicit changed-flag semantics. |
| `create_selector(f)` | `Selector::new(f)` | Preserve key/hash and equality behavior. |
| `create_selector_with_fn(...)` | `Selector::new_with_fn(...)` | Preserve the custom comparison. |
| `create_effect(f)` | `Effect::new(f)` | Effects are for external synchronization and do not run in ordinary SSR. |
| `create_render_effect(f)` | `RenderEffect::new(f)` | Preserve immediate render-effect semantics; do not substitute an ordinary effect. |
| free `watch(...)` | `Effect::watch(...)` | Handler arguments are current dep, prior dep, and prior handler output; no hashing/equality gate. |
| `create_action(f)` | `Action::new(f)` | Select `new_unsync`, `new_local`, or `new_unsync_local` when bounds/threading require them. |
| `Action::input_local()` | `Action::input()` | Result is a read-only mapped signal. |
| `Action::value_local()` | `Action::value()` | Result is `MappedSignal<Option<O>>`, not a writable signal. |
| `store_value(value)` | `StoredValue::new(value)` | Use `new_local` for intentional `!Send` storage. |
| `create_node_ref()` | `NodeRef::new()` | Preserve the concrete element type. |
| `MaybeSignal<T>` | `Signal<T>` | Use stored/derived/local constructors that preserve static-versus-reactive behavior. |
| `create_query_signal(key)` | `query_signal(key)` | Preserve URL serialization and navigation behavior. |
| `create_query_signal_with_options(...)` | `query_signal_with_options(...)` | Preserve `NavigateOptions`. |
| `NoCustomError`, `WrapError`, `ViaError`, `server_fn_error!`, `WrappedServerError` | Return a direct custom error implementing `FromServerFnError` | Provide `type Encoder`, serialization, and safe infrastructure-error mapping. |
| `#[server(encoding = "...")]` | typed `input`, `output`, or `protocol` | Verify codec traits and both generated targets. |

## Removed and contextual legacy patterns

| Legacy pattern | Current direction | Status / caveat |
| --- | --- | --- |
| `Scope`, `cx: Scope`, `view! { cx, ... }`, constructors taking `cx` | Owner-scoped, context-free APIs | Removed since 0.5; reassess lifetimes rather than only deleting tokens. |
| `provide_context(cx, value)` | `provide_context(value)` | Context now uses the current owner implicitly; verify current `Send + Sync` bounds. |
| `use_context::<T>(cx)` / `expect_context::<T>(cx)` | `use_context::<T>()` / `expect_context::<T>()` | Context now uses the current owner implicitly; preserve optional versus required lookup semantics. |
| `expect_context::<Scope>()` | no direct replacement | A reactive scope is no longer an application context value; identify the real ownership/data need. |
| `cx.children()` | explicit `children: Children` (or matching child type), then `children()` | Component children are props; choose one-shot, repeated, typed, or fragment children deliberately. |
| `create_isomorphic_effect` | `Effect::new_isomorphic` | Removed; use only for a side effect intentionally valid on server and client. |
| `create_resource(source, fetcher)` | `Resource::new(source, fetcher)` | Removed; preserve SSR serialization, codec, and `Send + Sync + 'static` requirements. |
| `create_local_resource(source, fetcher)` | `LocalResource::new(move || { let source = source.get(); async move { fetch(source).await } })` | Removed; the modern signature is one tracked fetcher closure and remains pending in SSR. Check whether the old equality-suppressing source semantics must be restored with a memo. |
| `create_multi_action` | `MultiAction::new` | Removed; confirm multi-dispatch semantics are needed. |
| `create_server_action` | `ServerAction::new` | Removed; requires an actual server-function type. |
| `create_server_multi_action` | `ServerMultiAction::new` | Removed; preserve concurrent dispatch behavior. |
| `<For ... view=...>` | children / inline item renderer | Removed historical component API. |
| component child `bind:name` | `let:name` | Historical 0.5 migration; unrelated to current form-element `bind:`. |
| feature `experimental-islands` | `islands` | Removed name; islands remain opt-in architecture. |
| `leptos::ssr::render_to_string(...)` | `RenderHtml::to_html()` for sync tests, or current integration/streaming renderer | Contextual; async resources and hydration need integration rendering. |
| old root `mount_to_body(cx, ...)`, `hydrate(...)`, or ID-based hydration | current `leptos::mount` functions | Contextual; preserve full-body versus subtree/island boundaries. |
| explicit `handle_server_fns` beside current `leptos_routes` | integration-owned registration when equivalent | Contextual; compare middleware, context, body limits, and route ownership first. |
| `LocalResource` result `.as_deref()` from the 0.7 `SendWrapper` | read the current inner value directly | Contextual; keep `.as_deref()` when the application's inner `Option`/smart pointer genuinely needs it. |
| resource `.read(cx)` / `.with(cx, ...)` | context-free current trait methods | Historical only; do not globally ban current `.read()` or `.with()`. |
| `site_address` config | `site_addr` | Historical rename; verify the current config loader/schema. |

## 0.9 prerelease watchlist

`0.9.0-beta` is not the default target while the stable line is 0.8. Opt in only
through an explicit prerelease upgrade and migrate all companion crates together.
Items to re-evaluate include:

- lazy rendering/hydration behind the `lazy` feature;
- the serde_qs 1.0 transition and nested form/query deserialization;
- `ToggleEvent` and event API changes;
- stabilized signal call syntax and the new `SignalOrFn` abstraction;
- removal of legacy `_with_handle` mounting variants;
- Rust 2024-edition changes inside the Leptos crate family;
- removals that were only deprecated in 0.8, especially the `create_*` helpers and
  wrapped custom server errors.

Do not copy a 0.9 example into a 0.8 repository because it looks newer. Verify
the beta's exact docs, release notes, generated code on native and Wasm, and the
repository's prerelease policy.

## Primary sources

- [All leptos crate versions](https://crates.io/crates/leptos/versions)
- [Leptos 0.8.20 API](https://docs.rs/leptos/0.8.20/leptos/)
- [Leptos 0.9.0-beta API](https://docs.rs/leptos/0.9.0-beta/leptos/)
- [Leptos releases](https://github.com/leptos-rs/leptos/releases)
- [0.1 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.1.0)
- [0.2 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.2.0)
- [0.3 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.3.0)
- [0.4 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.4.0)
- [0.5 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.5.0)
- [0.6 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.6.1)
- [0.7 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.7.0)
- [0.8 release notes](https://github.com/leptos-rs/leptos/releases/tag/v0.8.0)
