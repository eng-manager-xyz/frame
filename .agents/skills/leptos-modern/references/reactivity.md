# Reactivity and async state

The examples are verified against the Frame pin recorded in `project-profile.md`.
Verify exact signatures against the target repository's pinned docs before use.

## Contents

- [Ownership and storage](#ownership-and-storage)
- [Signals and derived state](#signals-and-derived-state)
- [Memos](#memos)
- [Effects](#effects)
- [Resources, actions, and async work](#resources-actions-and-async-work)
- [Reactive stores](#reactive-stores)
- [Migration traps](#migration-traps)
- [Primary sources](#primary-sources)

## Ownership and storage

Leptos 0.5 removed the explicit `Scope`/`cx` parameter and moved reactive values
under an `Owner`. Arena-backed handles such as `RwSignal<T>` are `Copy` and are
disposed with their owner. Do not revive `Scope`, thread a context parameter, or
extend an arena handle beyond its owner.

Choose storage deliberately:

| Requirement | Preferred form |
| --- | --- |
| Owner-scoped, `Send + Sync` state | `signal`, `RwSignal::new`, `Memo::new` |
| Owner-scoped, thread-local `!Send` state | `signal_local`, `RwSignal::new_local`, matching local constructors |
| Reference-counted state that must outlive/move outside an owner | `arc_signal`, `ArcRwSignal::new`, `ArcMemo` |
| Non-reactive owner-scoped value | `StoredValue::new` or `StoredValue::new_local` |

Do not choose a local constructor only to make bounds disappear. Confirm that all
creation, access, and execution stay on the originating thread. Conversely, do
not wrap a browser-only type to force it through a sync API when a local primitive
expresses the requirement.

Current context APIs obtain the owner implicitly: use `provide_context(value)`,
`use_context::<T>()`, and `expect_context::<T>()`. Remove historical `cx` arguments.
`expect_context::<Scope>()` has no replacement because `Scope` itself was removed;
identify the application value the old code was actually trying to access.

## Signals and derived state

Use the associated constructors or short modern helpers:

```rust
use leptos::prelude::*;

let (count, set_count) = signal(0_i32);
let selected = RwSignal::new(false);

set_count.update(|count| *count += 1);
selected.update(|selected| *selected = !*selected);

let doubled = move || count.get() * 2;
```

Prefer `.update()` for in-place mutation. Avoid a `.get()` plus `.set()` pair
when a single update expresses the same transition and avoids cloning the whole
value.

Reading with `.get()`, `.read()`, or their untracked forms has different cloning,
guard, and tracking behavior. Choose by semantics, release borrows before a
write, and follow `rust-modern` for ownership and cloning decisions.

Use `Signal<T>` when an API should accept either a fixed or reactive value.
`MaybeSignal<T>` is deprecated; it is not the modern abstraction.

## Memos

Use a closure for cheap derived state. Use `Memo::new` when computation is
expensive enough to cache or when equality-based downstream notification
suppression matters:

```rust
let filtered = Memo::new(move |_| {
    items.get()
        .into_iter()
        .filter(|item| item.matches(query.get()))
        .collect::<Vec<_>>()
});
```

The callback receives the prior value as `Option<&T>`, but `Memo::new` still
computes the candidate value. `PartialEq` then determines whether subscribers are
notified. Do not claim that the prior-value argument automatically short-circuits
the computation. Use `Memo::new_with_compare` for an explicit comparison policy
or `Memo::new_owning` when taking ownership of the previous value is materially
useful.

Do not memoize cheap arithmetic or formatting by habit. A memo adds a reactive
node and comparison work.

## Effects

Effects synchronize reactive values with a non-reactive external system: a DOM
API outside the view system, storage, logging, analytics, or another runtime.
They are not the normal way to derive one signal from another.

```rust
Effect::new(move |_| {
    external_system_set(count.get());
});
```

Effects run after the current synchronous work, on the next tick. Ordinary
effects do not run during server rendering. Use `Effect::new_isomorphic` only
when the side effect is intentionally valid on both server and client.

Use `Effect::watch` to isolate tracked dependencies from reads inside the
handler:

```rust
Effect::watch(
    move || count.get(),
    move |current, previous, previous_handler_output| {
        record_transition(*current, previous.copied());
        previous_handler_output.unwrap_or_default() + 1
    },
    false,
);
```

The dependency closure tracks reactive reads; Leptos does not hash or equality-
compare its returned value to decide whether to run the handler. The handler
receives the current dependency value, the previous dependency value, and the
previous handler output. The third argument is not an initialization flag.
`immediate` controls whether the handler runs on the first dependency evaluation.

The free `watch` function and `create_effect` are deprecated in 0.8; use
`Effect::watch` and `Effect::new`.

## Resources, actions, and async work

Choose by intent:

| Intent | Primitive |
| --- | --- |
| Initial or dependency-driven async data with SSR serialization/hydration | `Resource` |
| Browser-local or `!Send` async data that remains pending on the server | `LocalResource` |
| One async computation with no reactive source | `OnceResource` |
| Explicitly dispatched user/event work | `Action` |
| Generated client/server transport boundary | server function; see `full-stack.md` |

A resource source is reactive. When it changes, the fetcher runs again:

```rust
let user = Resource::new(
    move || user_id.get(),
    |id| async move { load_user(id).await },
);
```

`Resource` requires serializable, `Send + Sync + 'static` data plus the selected
codec bounds because SSR can serialize the resolved value and hydrate it on the
client. Use `LocalResource` for an async closure that is inherently local:

```rust
let local_user = LocalResource::new(move || {
    let id = user_id.get();
    async move { load_local_user(id).await }
});
```

`LocalResource::new` takes a closure that returns a future, not a bare `async`
block. Reactive reads performed synchronously in that closure become its load
dependencies. The old two-closure `create_local_resource(source, fetcher)` used a
separate source with equality suppression; the one-closure current API may refetch
when a dependency notifies even if its resulting value is equal. If preserving
that behavior matters, place an equality-suppressing `Memo` at the dependency
boundary and verify refetch behavior with a test. It does not load during SSR.

Do not remove `.as_deref()` merely because the code contains a local resource;
remove it only when it exclusively unwrapped the `SendWrapper` used by the 0.7
API and the current inner type does not require that dereference.

Actions are lazy until dispatch:

```rust
let save = Action::new(|input: &SaveInput| {
    let input = input.clone();
    async move { save_record(input).await }
});

save.dispatch(input);
let is_saving = save.pending();
let result = save.value();
```

For arena-backed `Action`, `.pending()` returns `Memo<bool>` and `.value()` returns
`MappedSignal<Option<O>>`; they are not writable signals. Pick among `new`,
`new_unsync`, `new_local`, and `new_unsync_local` from the exact-version bounds:
they distinguish whether the closure/future and input/output are `Send`, whether
execution stays on the creation thread, and whether access can cross threads.
Do not universally replace all constructors with `Action::new`.

## Reactive stores

Use `reactive_stores` for granular field-level reactivity over structured data
when it clearly reduces broad cloning or invalidation. It is a companion crate
with its own version line; resolve its compatible version independently.

Prefer a plain signal when updates replace the whole value or when field-level
paths add no value. Preserve stable keys for keyed collection updates. Do not add
a store abstraction only because a type has multiple fields.

## Migration traps

- `create_signal` is deprecated, not removed, in 0.8; still reject it.
- `create_resource` and `create_local_resource` are removed legacy constructors;
  use `Resource::new` and `LocalResource::new` with their different signatures.
- `create_isomorphic_effect` maps to `Effect::new_isomorphic`, not `Effect::new`.
- `store_value` maps to `StoredValue::new` or `new_local` after checking bounds.
- `Action::input_local` and `value_local` are deprecated aliases for `input` and
  `value`; the returned values are read-only mapped signals.
- A resource models loading; an action models dispatch. Do not translate only by
  matching types.
- Arena-backed and `Arc*` primitives express different lifetimes. Do not perform
  a mechanical rename between them.

## Primary sources

- [Leptos 0.8.20 API](https://docs.rs/leptos/0.8.20/leptos/)
- [Reactive graph 0.2.14 API](https://docs.rs/reactive_graph/0.2.14/reactive_graph/)
- [Leptos book: reactivity](https://book.leptos.dev/reactivity/index.html)
- [Leptos book: async resources](https://book.leptos.dev/async/10_resources.html)
- [Leptos book: actions](https://book.leptos.dev/async/13_actions.html)
