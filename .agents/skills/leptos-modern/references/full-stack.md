# Routing, server functions, and server integration

The examples are verified against the dated compatible set in
`migration-ledger.md`. These crates version independently; establish the target
repository's actual set before editing.

## Contents

- [Adopt only the architecture in scope](#adopt-only-the-architecture-in-scope)
- [Router](#router)
- [Server functions](#server-functions)
- [Direct custom server-function errors](#direct-custom-server-function-errors)
- [Forms and progressive enhancement](#forms-and-progressive-enhancement)
- [Axum and Actix integration](#axum-and-actix-integration)
- [SSR async and streaming](#ssr-async-and-streaming)
- [WebSockets](#websockets)
- [Boundary review](#boundary-review)
- [Primary sources](#primary-sources)

## Adopt only the architecture in scope

Leptos can supply client routing, server routing integration, generated RPC-style
server functions, SSR, streaming, and islands. These are independent decisions.

Before adding any of them, answer:

1. Which system currently owns HTTP routing and authentication?
2. Is the UI CSR-only, full-body hydration, scoped hydration, or islands?
3. Are server functions already an accepted transport boundary?
4. Which native and Wasm feature combinations are valid?
5. Which framework integration already owns response status, headers, redirects,
   and error mapping?

Do not install a Leptos router or server integration in a repository whose
architecture intentionally keeps Axum/Actix and Leptos rendering separate.

## Router

For a conventional routed application, use the typed current API:

```rust
use leptos::prelude::*;
use leptos_router::{components::*, path};

#[component]
fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| "Not found">
                <ParentRoute path=path!("") view=Layout>
                    <Route path=path!("") view=Home />
                    <Route path=path!("items/:id") view=Item />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
```

Current rules:

- Use `path!` rather than legacy string-route forms.
- Put the not-found view in the required `fallback` prop on `Routes` or
  `FlatRoutes`.
- Use `ParentRoute` for nested layouts and an `<Outlet/>` in the parent view.
- Use `FlatRoutes` only when routes are intentionally non-nested; it should not
  contain nested routes.
- Use router `<A>` and `<Form>` when their progressive enhancement and relative
  path behavior match the application.
- `generate_route_list` is synchronous in the current API.

The `islands-router` feature supports intentional islands routing. Do not enable
it as a substitute for ordinary `leptos_router`, and do not infer that a project
using hydration automatically wants islands routing.

## Server functions

Use a server function when the repository intentionally wants one function
signature to generate a server endpoint and client call. It is not required for
SSR, and it is not automatically preferable to an existing typed HTTP client.

```rust
use leptos::prelude::*;

#[server]
pub async fn load_item(id: u64) -> Result<Item, ServerFnError> {
    // Authenticate and authorize on the server, then load the value.
    todo!()
}
```

Modern macro configuration uses typed `input`, `output`, or `protocol` arguments.
The string-based `encoding = "..."` form is legacy and may be deprecated; do not
introduce it. Use default HTTP encoding unless a measured or protocol-specific
need justifies another codec.

The generated endpoint is still an untrusted network boundary. Validate input,
authenticate, authorize, avoid exposing secrets in errors, and apply the
repository's request/body limits.

## Direct custom server-function errors

Leptos/server_fn 0.8 supports returning an application error directly. Replace
the old `NoCustomError`, `WrapError`, `ViaError`, `server_fn_error!`, and
`ServerFnError::WrappedServerError` patterns with an error that implements
`FromServerFnError`:

```rust
use leptos::prelude::*;
use leptos::server_fn::{
    codec::JsonEncoding,
    error::{FromServerFnError, ServerFnErrorErr},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum AppError {
    #[error("item not found")]
    NotFound,
    #[error("request failed")]
    Transport,
}

impl FromServerFnError for AppError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(_: ServerFnErrorErr) -> Self {
        Self::Transport
    }
}

#[server]
pub async fn load_item(id: u64) -> Result<Item, AppError> {
    todo!()
}
```

`type Encoder = JsonEncoding` is required; implementing only the conversion
method is incomplete. The error also needs the serialization traits required by
the chosen encoder. Map infrastructure errors without leaking sensitive detail.

## Forms and progressive enhancement

Prefer real HTML links and forms when the operation should work before Wasm
loads. Router `<A>` and `<Form>`, Leptos action forms, and server-action forms can
enhance navigation/submission while preserving the browser fallback.

Do not replace a semantically correct `<form method=... action=...>` with an
event-only handler unless the application explicitly gives up progressive
enhancement. Keep server validation authoritative even when client validation is
present.

## Axum and Actix integration

When the project adopts an official integration, use that integration's current
route registration and render handlers. For Axum this normally means generating
the route list and installing Leptos routes/server-function handling through
`leptos_axum`; analogous APIs exist in `leptos_actix`.

Do not preserve a separate explicit `handle_server_fns` route merely because a
0.5-era template had one when the current `leptos_routes` integration already
handles registered server functions. Conversely, do not remove a custom route
until verifying that the integration owns its path, context, middleware, body
limit, and error behavior.

Current `LeptosOptions` does not implement `Default`. Load it through the
repository's configuration path rather than constructing a pretend default.

Keep server-only dependencies and code behind native/`ssr` configuration, and
browser-only code behind Wasm/`hydrate` or `csr` configuration. Check both sides;
compiling only the server can hide broken generated client code.

## SSR async and streaming

Choose the integration rendering mode according to user-visible behavior:

- synchronous HTML for views with no pending async data;
- async/in-order rendering when ordered completion is required;
- out-of-order streaming when boundaries may resolve independently;
- partially blocked rendering when selected data must resolve before the shell
  while other content may stream.

Resources and `<Suspense>` participate in these modes. A local `.to_html()` unit
render is not evidence that resource serialization, route context, headers,
redirects, streaming order, or hydration data works. Exercise the integration
handler for those changes.

## WebSockets

`server_fn` 0.8 can use a typed `Websocket` protocol for streaming server
functions. It is an optional generated protocol, not a universal replacement for
an existing direct WebSocket service. Preserve the existing service when it owns
connection lifecycle, multiplexing, backpressure, protocol negotiation, or
non-Leptos clients unless a deliberate migration covers those requirements.

## Boundary review

Before completion, verify:

- route definitions and server route generation agree;
- server-function codecs and custom error encoders agree on both targets;
- authentication/authorization run on the server;
- server context is provided by the actual integration;
- status, headers, redirects, and not-found behavior are tested;
- request/body limits cover server-function endpoints, including streaming
  protocols where applicable;
- hydration receives the same shell and route state emitted by SSR;
- no client bundle contains server-only credentials or code.

Use `pragmatic-tiger` for the risk and dependency assessment and `rust-modern`
for error/API design; this reference only identifies Leptos-specific boundaries.

## Primary sources

- [leptos_router 0.8.14 API](https://docs.rs/leptos_router/0.8.14/leptos_router/)
- [leptos_axum 0.8.10 API](https://docs.rs/leptos_axum/0.8.10/leptos_axum/)
- [server_fn 0.8.13 API](https://docs.rs/server_fn/0.8.13/server_fn/)
- [FromServerFnError](https://docs.rs/server_fn/0.8.13/server_fn/error/trait.FromServerFnError.html)
- [Leptos book: router](https://book.leptos.dev/router/index.html)
- [Leptos book: server functions](https://book.leptos.dev/server/index.html)
- [Leptos book: SSR modes](https://book.leptos.dev/ssr/23_ssr_modes.html)
