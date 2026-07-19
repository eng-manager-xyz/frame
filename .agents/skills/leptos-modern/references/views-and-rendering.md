# Views and rendering modes

The examples are verified against the Frame pin recorded in `project-profile.md`.
Apply only after establishing the target repository's exact version, features,
target, and rendering architecture.

## Contents

- [Imports and component shape](#imports-and-component-shape)
- [Props and children](#props-and-children)
- [Events and form bindings](#events-and-form-bindings)
- [Conditional and list views](#conditional-and-list-views)
- [Rendering modes](#rendering-modes)
- [SSR and hydration correctness](#ssr-and-hydration-correctness)
- [Mounting](#mounting)
- [Development-only compile-time options](#development-only-compile-time-options)
- [Primary sources](#primary-sources)

## Imports and component shape

Prefer the current prelude and ordinary Rust function signatures:

```rust
use leptos::prelude::*;

#[component]
pub fn Counter(initial: i32) -> impl IntoView {
    let count = RwSignal::new(initial);

    view! {
        <button on:click=move |_| count.update(|n| *n += 1)>
            {move || count.get()}
        </button>
    }
}
```

There is no `Scope`/`cx` parameter in current component APIs. Components and
their props may be generic. Keep a typed return (`impl IntoView`) until differing
branches actually require type unification; do not erase every component into
`AnyView` by default.

Component lint attributes are forwarded by the current macro. Do not retain
historical workarounds based on the obsolete claim that component-level lint
attributes are always lost.

## Props and children

Choose the narrowest child type that matches invocation behavior:

| Need | Type |
| --- | --- |
| Invoke once | `Children` |
| Invoke repeatedly | `ChildrenFn` |
| Invoke mutably | `ChildrenFnMut` |
| Preserve the concrete view type | typed children variants |
| Iterate over separate child nodes | `ChildrenFragment` |

For optional children:

```rust
#[component]
fn Panel(#[prop(optional)] children: Option<Children>) -> impl IntoView {
    children.map(|children| children())
}
```

`#[prop(optional)]` strips `Option` at the call site: an `Option<T>` prop is
omitted or passed as a bare `T`, which the builder wraps in `Some`. Use
`#[prop(optional_no_strip)]` when callers must explicitly pass `Some(...)` or
`None`.

Use `#[prop(into)]` only when call-site ergonomics justify the widened accepted
types. Follow `rust-modern` for public API shape and conversion policy.

## Events and form bindings

Leptos event handlers are typed. Prefer the most direct interface:

- Use `on:event` when the event object is needed.
- Use `on:event:target` when the target element's typed API is needed; it yields
  a targeted wrapper, and methods such as `value()` exist only for supporting
  element types.
- Use `bind:value`, `bind:checked`, or another supported `bind:` property for
  genuine two-way form control with a writable signal.
- Use `NodeRef::new()` when code needs a persistent element handle rather than a
  one-event target.

Do not cast from a generic event target when the targeted form is available.
Do not confuse current `bind:` form bindings with the historical component child
binding syntax that became `let:` in 0.5.

## Conditional and list views

Use ordinary Rust branches when both branches have one concrete type or use an
explicit current unification tool:

- `Either::Left` / `Either::Right` for two types.
- `either!` to match an expression and construct `Either`/`EitherOfN` values.
- `Either!(A, B, C)` only in type position; this capitalized form is a type macro.
- `.into_any()` when deliberate type erasure is simpler than exposing a large
  branch type.

Use `<Show when=...>` when the conditional remains reactive and its fallback
semantics help. Any closure captured by the view must own `'static` data as
required; satisfy normal Rust capture rules rather than applying ad hoc clone
recipes.

Use `<ShowLet>` to conditionally unwrap an `Option`:

```rust
view! {
    <ShowLet some=move || selected.get() let:value fallback=|| "Nothing selected">
        <span>{value}</span>
    </ShowLet>
}
```

The prop is `some`, not `when`. `ShowLet` arrived in Leptos 0.8.8, so do not use
it on an earlier pin without upgrading.

For lists:

- Use `<For each=... key=... children=...>` for collections that reorder, insert,
  or delete. Keys must be stable identities, not current positions.
- Use an iterator rendered directly in `view!` for stable collections that are
  replaced as a unit.
- Avoid collecting into an erased view solely to satisfy a historical return-type
  pattern; current typed iterators and `Either` often preserve better types.

## Rendering modes

Treat mode as a build-time contract:

| Mode | Typical artifact | Browser effects | Server rendering |
| --- | --- | --- | --- |
| `csr` | Wasm client application | Yes | No |
| `hydrate` | Wasm client for server-rendered HTML | Yes | Consumes matching SSR HTML |
| `ssr` | Native server | Ordinary effects disabled | Yes |
| islands | Native SSR plus selected hydrated components | Only inside intentional islands | Yes |

Prefer target-specific dependency sections or carefully forwarded crate features
so each artifact enables its intended mode. Avoid enabling `csr`, `hydrate`, and
`ssr` indiscriminately in one target.

Islands are an architectural choice. The `islands-router` Leptos feature supports
routing in that choice; it is not a separate crate and is not the default router
for a conventional full-stack application.

## SSR and hydration correctness

Hydration walks existing DOM in current Leptos. The browser may normalize invalid
HTML before Wasm starts, so server and client can disagree even when the source
looks similar.

Require all of the following for hydrated surfaces:

1. Valid HTML nesting and table structure.
2. The same deterministic initial view on server and client.
3. No browser-only data influencing the pre-hydration branch without a stable SSR
   fallback.
4. Stable keyed-list identity.
5. Matching feature gates and shell boundaries.
6. A browser-level SSR-plus-hydration check when markup or async behavior changes.

Do not suppress hydration warnings without explaining and testing the mismatch.

`RenderHtml::to_html()` is appropriate for synchronous views and focused SSR unit
tests. It does not replace a full server integration for `Resource`, `Suspense`,
streaming, response options, route context, or hydration payloads. Use the
framework integration's in-order/out-of-order/async rendering path for those.

## Mounting

Current mounting lives under `leptos::mount`:

- `mount_to_body` and `mount_to` create CSR DOM.
- `hydrate_body` hydrates a full-body application.
- `hydrate_from` hydrates an existing subtree.

Match the function to the server-rendered boundary. Do not convert scoped
hydration islands into full-body hydration, or vice versa, as a mechanical API
update. Keep returned unmount handles alive or intentionally call `.forget()` in
accordance with the application lifecycle.

## Development-only compile-time options

`erase_components` can reduce development type complexity, and `cargo-leptos`
can manage it for supported development workflows. Keep it development-only.
A global `[build] rustflags` entry affects release builds too; do not add one for
this optimization. Use a controlled development command or repository-specific
configuration and validate production without the cfg.

Likewise, adopt `subsecond`, lazy view features, or preload behavior only when the
pin supports them and the repository chooses their runtime/build tradeoffs.

## Primary sources

- [Leptos 0.8.20 API](https://docs.rs/leptos/0.8.20/leptos/)
- [Leptos 0.8.20 mount module](https://docs.rs/leptos/0.8.20/leptos/mount/)
- [Leptos book: view syntax](https://book.leptos.dev/view/index.html)
- [Leptos book: hydration bugs](https://book.leptos.dev/ssr/24_hydration_bugs.html)
- [Leptos book: islands](https://book.leptos.dev/islands.html)
