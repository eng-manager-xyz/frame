# Modern Axum middleware and serving

Use these rules for Axum 0.8.x and the compatible companion lines resolved by
the repository. Do not copy APIs from upstream's unreleased 0.9 work.

## Contents

- [Choose the middleware level](#choose-the-middleware-level)
- [Application middleware](#application-middleware)
- [Layer scope and order](#layer-scope-and-order)
- [Errors and response bodies](#errors-and-response-bodies)
- [Routing and backpressure boundaries](#routing-and-backpressure-boundaries)
- [Serving and graceful shutdown](#serving-and-graceful-shutdown)
- [Listener configuration and connect info](#listener-configuration-and-connect-info)
- [WebSockets](#websockets)
- [Server-sent events](#server-sent-events)
- [Service-level tests](#service-level-tests)
- [Macros and diagnostics](#macros-and-diagnostics)
- [Review checklist](#review-checklist)
- [Primary sources](#primary-sources)

## Choose the middleware level

| Requirement | Preferred current mechanism |
| --- | --- |
| Application-local async request/response policy | `middleware::from_fn` |
| Same policy with application state | `middleware::from_fn_with_state` |
| A type must work as both extractor and gate | `middleware::from_extractor` / `_with_state` |
| Small Tower map/then operation | `ServiceBuilder` combinator |
| Reusable configurable library middleware | Tower `Layer` + `Service` |
| One handler only | `Handler::layer` or `MethodRouter::layer` |
| Matched routes only | `route_layer` |
| Every existing route and fallback in a router | `layer` after building them |

Do not write a custom Tower future when `from_fn` expresses application-local
behavior clearly. Do not use `from_extractor` unless extractor reuse is real.
For a publishable integration, use Tower's service contract rather than tying
the package unnecessarily to Axum.

## Application middleware

A current stateful middleware accepts zero or more parts extractors, then one
body-consuming request extractor, and finally non-generic `Next`:

```rust
use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};

#[derive(Clone)]
struct AppState;

async fn request_policy(
    State(_state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}
```

Install it with the same state value the router will receive:

```rust
use axum::{middleware, routing::get, Router};

let state = AppState;
let app = Router::new()
    .route("/", get(|| async { "ok" }))
    .layer(middleware::from_fn_with_state(
        state.clone(),
        request_policy,
    ))
    .with_state(state);
```

`from_fn` cannot extract `State`; use `from_fn_with_state`. Request extensions
inserted by middleware are available to downstream `Extension<T>` extractors,
but they are not copied to response extensions automatically.

Return early with something implementing `IntoResponse` for an expected HTTP
decision. Keep `Next::run` at most once. Review whether response policy must wrap
early returns, downstream extractor rejections, fallbacks, and redirects.

## Layer scope and order

Both `Router::layer` and `route_layer` affect only routes already present. Add
routes/fallbacks first, then layer them. Routes appended later are unwrapped.

`route_layer` runs only for matched routes and preserves a 404 for unknown paths;
it panics when applied before any route. This commonly fits authorization.
`layer` also wraps the existing fallback and fits policy that must cover the
whole constructed router.

Repeated direct calls wrap bottom-to-top:

```text
request -> last .layer -> previous .layer -> handler
response <- last .layer <- previous .layer <- handler
```

Prefer `ServiceBuilder` for a stack because request order reads top-to-bottom:

```rust,ignore
let stack = ServiceBuilder::new()
    .layer(request_id)
    .layer(trace)
    .layer(timeout);
```

Test order whenever one layer inserts data another consumes, one can return
early, or response headers/security policy depend on nesting. Never infer order
from source adjacency alone.

Middleware added through router methods runs after route selection. To rewrite a
URI before matching, wrap the entire `Router` with a Tower layer and then serve
the resulting service using the current make-service shape.

## Errors and response bodies

Axum's routed service boundary is infallible: errors reaching Hyper can close a
connection without an HTTP response. Prefer converting expected middleware
failures into responses.

For an actually fallible Tower layer, place `HandleErrorLayer` outside it:

```rust,ignore
ServiceBuilder::new()
    .layer(HandleErrorLayer::new(|_: BoxError| async {
        StatusCode::REQUEST_TIMEOUT
    }))
    .layer(tower::timeout::TimeoutLayer::new(duration))
```

Distinguish companion APIs:

- Tower's timeout is fallible and needs error conversion.
- Tower HTTP can return a timeout status directly.
- Tower HTTP 0.6.11 already deprecates `TimeoutLayer::new`, `Timeout::new`, and
  `Timeout::layer` (since 0.6.7) in favor of explicit status-code constructors.
  Its basic `auth::require_authorization` module and `cors::any()` are deprecated
  too. Use `cors::Any`; replace basic authorization with application validation
  middleware or the current asynchronous authorization layer as requirements
  dictate. Do not postpone those migrations until a 0.7 upgrade; still use the
  exact companion line in the lockfile rather than blindly the ecosystem latest.

Use `middleware::ResponseAxumBodyLayer` (available since Axum 0.8.5) when a
service's response body must be normalized to `axum::body::Body`. Do not revive
the removed `BoxBody` or `box_body` helpers.

## Routing and backpressure boundaries

Axum routes by request and drives destination readiness inside the response
future. Avoid placing backpressure-sensitive services behind routing without an
explicit load-shedding/bounding design. When such a layer must govern the whole
application, wrap the complete router externally and handle its errors before
serving.

Handlers and ordinary application `from_fn` middleware are normally always
ready. Escalate to this review when integrating queueing, concurrency limits,
load shedding, retry, buffering, or a service whose `poll_ready` contract is
meaningful.

## Serving and graceful shutdown

Use a Tokio listener plus `axum::serve` for the basic current server:

```rust,ignore
let listener = tokio::net::TcpListener::bind(address).await?;
axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

`axum::serve` is intentionally simple. Use Hyper/Hyper Util only when required
connection-builder, executor, TLS, protocol, upgrade, or IO control cannot be
expressed through the current listener/service APIs. Do not reintroduce
`axum::Server`, `hyper::Server`, or `Server::bind`.

A graceful-shutdown future stops accepting new work; application tasks and
upgraded/streaming connections can have their own lifecycle. Coordinate them
explicitly and bound the total below the deployment platform's kill window.
Test startup failure, shutdown signal handling, in-flight request completion,
and forced/bounded termination where those semantics matter.

## Listener configuration and connect info

`Serve::tcp_nodelay` is removed. On Axum 0.8.9 configure each accepted stream
through `ListenerExt::tap_io`:

```rust,ignore
use axum::serve::ListenerExt;

let listener = tokio::net::TcpListener::bind(address)
    .await?
    .tap_io(|stream| {
        if let Err(error) = stream.set_nodelay(true) {
            tracing::debug!(%error, "failed to set TCP_NODELAY");
        }
    });
```

Use the matching make service when handlers extract peer metadata:

```rust,ignore
use std::net::SocketAddr;

axum::serve(
    listener,
    app.into_make_service_with_connect_info::<SocketAddr>(),
)
.await?;
```

Then extract `ConnectInfo<SocketAddr>`. Without the matching make-service setup,
the extractor rejects at runtime. Use `MockConnectInfo` in service tests. For a
custom listener, implement `Connected<IncomingStream<'_, L>>` for the metadata
type and verify it against the exact 0.8 listener contract.

Peer address is not automatically a trustworthy public client identity behind a
proxy. Apply the repository's trusted-proxy policy before consuming forwarding
headers.

## WebSockets

Enable Axum's `ws` feature intentionally. Route upgrades with `routing::any`
when both HTTP/1 GET and HTTP/2 CONNECT WebSockets must work:

```rust,ignore
use axum::{extract::ws::WebSocketUpgrade, routing::any, Router};

async fn upgrade(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.on_upgrade(handle_socket)
}

let app = Router::new().route("/ws", any(upgrade));
```

Current 0.8 message payloads use `Utf8Bytes` and `Bytes`, not owned `String` and
`Vec<u8>` variants. Prefer `Message::text(value)`, `Message::binary(value)`, or
an explicit `.into()` conversion.

`WebSocket::close` was removed. Send `Message::Close(frame)` and honor the close
handshake. A generic sink `.close().await` may still be current, so treat it as
a manual-review signal rather than an automatic violation.

Use split sink/stream halves for concurrent send and receive tasks. Configure
message/frame/read/write bounds deliberately. The removed message-count
`max_send_queue` does not translate numerically to `max_write_buffer_size`:
current write controls count bytes, and the buffer grows past
`write_buffer_size` only during write errors. Keep `max_write_buffer_size` above
`write_buffer_size` plus the largest permitted message, and use an
application-level bounded channel when producer queue count/backpressure is the
actual invariant. Current protocol helpers include
`selected_protocol`, `requested_protocols`, and `set_selected_protocol`; verify
their introduction patch before supporting an earlier 0.8 pin.

Test upgrades on a real listener when the HTTP protocol, close handshake,
connection metadata, or concurrent lifecycle is relevant.

## Server-sent events

Use `axum::response::sse::{Sse, Event, KeepAlive}`:

```rust,ignore
Sse::new(stream).keep_alive(KeepAlive::default())
```

Reject old `axum::sse` service-style APIs. Validate externally supplied event
metadata; setters have newline/repetition constraints, and `json_data` returns a
`Result`. Preserve disconnect cancellation and backpressure. Keep-alive timing
needs the relevant runtime support even though constructing `Sse` itself is less
feature-coupled on current patches.

## Service-level tests

Test a supplied-state `Router<()>` as a Tower service:

```rust,ignore
use axum::{body::{to_bytes, Body}, http::Request};
use tower::ServiceExt;

let response = app
    .oneshot(
        Request::builder()
            .uri("/health")
            .body(Body::empty())
            .expect("invariant: static test URI is valid"),
    )
    .await
    .expect("invariant: an Axum router service is infallible");

let body = to_bytes(response.into_body(), 1024)
    .await
    .expect("invariant: the infallible health body fits the 1 KiB test bound");
```

Use `Router::as_service` or `into_service` when request-body inference needs
help. Tower 0.5 uses `ServiceExt::ready`/`oneshot`; `ready_and` and `ReadyAnd` are
removed. `tower::ServiceExt` requires a direct dependency visible to the test
crate; do not add one incidentally. Never use Axum's private test helpers.

Cover status, body, response headers, 404/405, rejection mapping, limit edges,
middleware order, state/extension availability, and connect info. A real bound
listener is appropriate for upgrades, transport metadata, serving, and shutdown.

## Macros and diagnostics

With Axum's `macros` feature, current diagnostics/derives include:

- `#[debug_handler]`
- `#[debug_middleware]`
- `#[derive(FromRef)]`
- `#[derive(FromRequest)]`
- `#[derive(FromRequestParts)]`

Do not use obsolete `axum-debug` or `#[axum::async_trait]`. Enable `macros` only
when its value justifies the feature; diagnostics can be applied during
development and need not become an architectural dependency.

## Review checklist

Apply rubric gates G4 and G6-G9 plus scorecard categories 6-7. In this domain,
confirm:

- layer scope/order and the infallible error boundary are request-tested;
- listener, connect info, proxy trust, and bounded shutdown match deployment;
- WebSocket/SSE/stream limits and lifecycle are exercised when affected; and
- stable examples contain no unreleased 0.9 API.

## Primary sources

- [Axum 0.8.9 middleware](https://docs.rs/axum/0.8.9/axum/middleware/index.html)
- [Axum 0.8.9 error handling](https://docs.rs/axum/0.8.9/axum/error_handling/index.html)
- [Axum 0.8.9 `serve`](https://docs.rs/axum/0.8.9/axum/fn.serve.html)
- [Axum 0.8.9 `ListenerExt`](https://docs.rs/axum/0.8.9/axum/serve/trait.ListenerExt.html)
- [Axum 0.8.9 WebSockets](https://docs.rs/axum/0.8.9/axum/extract/ws/index.html)
- [Axum 0.8.9 SSE](https://docs.rs/axum/0.8.9/axum/response/sse/index.html)
- [Tower 0.5.3](https://docs.rs/tower/0.5.3/tower/)
