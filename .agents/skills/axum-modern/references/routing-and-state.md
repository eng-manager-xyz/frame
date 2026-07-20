# Modern Axum routing and state

Use these rules for Axum 0.8.x. Verify signatures against the repository's exact
patch before applying them to another release line.

## Contents

- [Route syntax](#route-syntax)
- [Route composition](#route-composition)
- [Fallbacks and method misses](#fallbacks-and-method-misses)
- [The Router state parameter](#the-router-state-parameter)
- [Application state](#application-state)
- [Substates with FromRef](#substates-with-fromref)
- [Request extensions](#request-extensions)
- [Routing review checklist](#routing-review-checklist)
- [Primary sources](#primary-sources)

## Route syntax

Use matchit 0.8 syntax on Axum 0.8:

```rust
use axum::{routing::get, Router};

let app = Router::new()
    .route("/users/{user_id}", get(show_user))
    .route("/assets/{*path}", get(asset));
# async fn show_user() {}
# async fn asset() {}
```

Apply these rules:

- Use `/{name}` for one segment.
- Use `/{*name}` for a wildcard tail.
- Start every route with `/`.
- Define both trailing-slash forms explicitly when both are valid, or add an
  intentional redirect. Axum no longer redirects trailing slashes automatically.
- Expect route registration to panic for invalid or conflicting paths; cover
  generated route tables with startup/tests.
- Never migrate `/:name` to `/{:name}` or `/*name` to `/{name}`. The exact
  replacements are `/{name}` and `/{*name}`.
- Do not call `Router::without_v07_checks()` to preserve old capture syntax. It
  treats leading `:` or `*` literally; it does not restore 0.7 parameter
  semantics. Retain it only when the public route intentionally contains a
  literal segment beginning with one of those characters, document that intent,
  and request-test the literal path.

Axum 0.8 makes tuple and tuple-struct `Path` extraction require exactly the same
number of captures. Prefer a named deserializable struct when parameter meaning
matters:

```rust
use axum::extract::Path;
use serde::Deserialize;

#[derive(Deserialize)]
struct UserPath {
    organization_id: String,
    user_id: String,
}

async fn show(Path(path): Path<UserPath>) -> String {
    format!("{}/{}", path.organization_id, path.user_id)
}
```

## Route composition

- Use one `MethodRouter` per path: `.route("/items", get(list).post(create))`.
- Use `merge` for routers at the same path level.
- Use `nest` for an Axum router mounted below a non-root prefix.
- Use `route_service` for one infallible Tower service at one path.
- Use `nest_service` for an arbitrary infallible service mounted below a prefix.
- Do not pass a `Router` to `route_service`; use `nest` or `merge`.
- Do not nest at `/`; use `merge`.

Nesting strips the matched prefix from the URI seen by the inner router/service.
Extract `OriginalUri` only when code genuinely needs the pre-nesting URI. Captures
in a dynamic outer prefix are visible to inner `Path` extraction, so verify the
full capture set after introducing dynamic nesting.

`nest("/foo", ...)` and a wildcard route are not interchangeable:

- nesting strips the prefix and has router fallback inheritance semantics;
- a wildcard route retains the whole URI;
- their empty/trailing path matches differ.

Choose from required behavior, not shorter syntax.

## Fallbacks and method misses

- Use `Router::fallback(handler)` for unmatched paths handled by a handler.
- Use `fallback_service` only for an arbitrary infallible service.
- Use `method_not_allowed_fallback(handler)` to customize matched-path/wrong-
  method behavior on current 0.8 patches.
- Distinguish `404 Not Found` from `405 Method Not Allowed` in tests.
- Review fallback inheritance when nesting. An inner explicit fallback wins;
  otherwise the nested router inherits the outer fallback.
- Review fallback conflicts when merging; do not silently discard policy.

Use `route_layer` when middleware should execute only after a route matches. It
is useful for authentication that must not turn unknown paths into authorization
errors. Use `layer` when the policy intentionally covers fallbacks and the whole
router. Confirm the exact layer coverage with request tests.

## The Router state parameter

Interpret `Router<S>` as **a router still missing state `S`**, not a router that
contains `S`.

```rust
use axum::{extract::State, routing::get, Router};

#[derive(Clone)]
struct AppState;

fn routes() -> Router<AppState> {
    Router::new().route(
        "/health",
        get(|State(_): State<AppState>| async { "ok" }),
    )
}

let app: Router<()> = routes().with_state(AppState);
```

Only a router with no missing state (`Router<()>`, normally written `Router`)
can be passed directly to `axum::serve` or converted into a make service.

Use these return shapes:

- Return `Router<AppState>` from a route-construction function that deliberately
  leaves state for its caller to provide.
- Return `Router`/`Router<()>` from a self-contained function that already calls
  `.with_state(state)`.
- When a helper provides one state but must compose into a router missing some
  caller-selected state, make the resulting missing-state type generic only when
  the composition requires it.

Do not annotate a post-`with_state` router as `Router<AppState>` merely because
it uses `AppState`; that annotation says the opposite.

## Application state

Use `State<T>` for global application state:

```rust
use axum::{extract::State, routing::get, Router};
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
}

struct Config;

async fn health(State(_state): State<AppState>) -> &'static str {
    "ok"
}

let state = AppState { config: Arc::new(Config) };
let app = Router::new()
    .route("/health", get(health))
    .with_state(state);
```

The state must satisfy the current router/handler bounds, normally
`Clone + Send + Sync + 'static`. Put expensive shared data behind an appropriate
shared owner rather than cloning it per request. Let `rust-modern` decide the
Rust ownership mechanism and `pragmatic-tiger` assess broader state risk.

Call `with_state` after composing routers that share a state when that keeps
types simple. When nested routers have independently supplied state, ensure the
resulting missing-state types still compose and verify fallback/middleware
behavior.

## Substates with FromRef

Use `FromRef<AppState>` when handlers or reusable routers should extract a focused
substate without knowing the entire application state:

```rust
use axum::extract::{FromRef, State};
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    database: Arc<Database>,
}

struct Database;

impl FromRef<AppState> for Arc<Database> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.database)
    }
}

async fn handler(State(_database): State<Arc<Database>>) {}
```

If using a derive supplied by Axum macros, first verify that the `macros` feature
is enabled. Do not enable it merely to avoid a small explicit implementation.

For library routers, prefer a generic outer state with the required
`FromRef` bound when that is genuinely part of the library contract. An
application router can stay concrete.

## Request extensions

`Extension<T>` is current and valid for data attached to an individual request,
such as authenticated identity inserted by middleware. It is not the preferred
container for global application state.

```rust
use axum::Extension;

#[derive(Clone)]
struct CurrentUser;

async fn account(Extension(_user): Extension<CurrentUser>) {}
```

Ensure a covering layer inserts the extension on every path that extracts it.
Missing required extensions reject the request. Use `Option<Extension<T>>` only
when absence is a valid branch, not as a way to hide middleware coverage bugs.

## Routing review checklist

Apply rubric gates G4-G6 and G8-G9 plus scorecard category 3. In this domain,
confirm:

- captures, trailing slashes, nesting, fallbacks, and 404/405 behavior are tested;
- every `Router<S>` annotation describes missing state correctly;
- `State` and request `Extension` retain their distinct roles; and
- middleware scope preserves unknown-path behavior.

## Primary sources

- [Axum 0.8.9 `Router`](https://docs.rs/axum/0.8.9/axum/struct.Router.html)
- [Axum 0.8.9 routing module](https://docs.rs/axum/0.8.9/axum/routing/index.html)
- [Axum 0.8.9 `State`](https://docs.rs/axum/0.8.9/axum/extract/struct.State.html)
- [Axum 0.8.9 `FromRef`](https://docs.rs/axum/0.8.9/axum/extract/trait.FromRef.html)
- [Axum 0.8.9 `Extension`](https://docs.rs/axum/0.8.9/axum/struct.Extension.html)
