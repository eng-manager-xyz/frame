# Axum release and migration ledger

Use this ledger to translate historical Axum code to the exact stable target.
Inventory first, then migrate each intervening release-line boundary. Do not
infer an API from version labels or upstream `main`.

## Contents

- [Recorded release authority](#recorded-release-authority)
- [Complete published inventory](#complete-published-inventory)
- [Yanked releases](#yanked-releases)
- [Release-line transitions](#release-line-transitions)
- [Canonical current replacements](#canonical-current-replacements)
- [Modern 0.8 point features](#modern-08-point-features)
- [Companion generation boundaries](#companion-generation-boundaries)
- [Stable versus unreleased main](#stable-versus-unreleased-main)
- [Migration procedure](#migration-procedure)
- [Primary sources](#primary-sources)

## Recorded release authority

Snapshot date: **2026-07-19**.

- Latest published stable: **Axum 0.8.9**, released 2026-04-14.
- Axum 0.8.9 MSRV: **Rust 1.80**.
- Latest compatible Axum Core resolved in this repository: **0.5.6**.
- No Axum prerelease newer than 0.8.9 was published at the snapshot.
- Upstream `main` is developing a breaking 0.9 line and is not stable 0.8
  documentation.
- The exact packaged Axum 0.8.9 source contains no active public Axum-owned
  `#[deprecated]` item. Modernization still matters because old lines contain
  many removed names, changed semantics, route syntax traps, and deprecated
  companion APIs.

Re-verify crates.io, exact-version docs.rs, the official release, tagged source,
and the tagged changelog whenever "latest" affects a decision. Treat a new
release as a reason to refresh this ledger, not permission to upgrade a project.

## Complete published inventory

This is every crates.io Axum artifact present at the snapshot: **94 releases**.
Dates are crates.io UTC creation dates. `YANKED` artifacts must never be selected
for a new resolution.

```text
0.8.9           2026-04-14  active
0.8.8           2025-12-20  active
0.8.7           2025-11-14  active
0.8.6           2025-09-30  active
0.8.5           2025-09-28  active
0.8.4           2025-04-30  active
0.8.3           2025-03-28  active
0.8.2           2025-01-21  YANKED
0.8.1           2025-01-01  active
0.8.0           2025-01-01  active
0.8.0-rc.1      2024-12-17  historical prerelease
0.8.0-alpha.1   2024-10-05  historical prerelease

0.7.9           2024-11-16  active
0.7.8           2024-11-15  active
0.7.7           2024-09-27  active
0.7.6           2024-09-20  active
0.7.5           2024-03-24  active
0.7.4           2024-01-13  active
0.7.3           2023-12-29  active
0.7.2           2023-12-04  active
0.7.1           2023-11-27  active
0.7.0           2023-11-27  active

0.6.20          2023-08-03  active
0.6.19          2023-07-17  active
0.6.18          2023-04-30  active
0.6.17          2023-04-25  active
0.6.16          2023-04-18  active
0.6.15          2023-04-12  active
0.6.14          2023-04-11  active
0.6.13          2023-04-11  active
0.6.12          2023-03-22  active
0.6.11          2023-03-13  active
0.6.10          2023-03-03  active
0.6.9           2023-02-27  active
0.6.8           2023-02-24  active
0.6.7           2023-02-17  active
0.6.6           2023-02-12  active
0.6.5           2023-02-11  active
0.6.4           2023-01-24  active
0.6.3           2023-01-20  active
0.6.2           2023-01-09  active
0.6.1           2022-11-29  active
0.6.0           2022-11-25  active
0.6.0-rc.5      2022-11-18  historical prerelease
0.6.0-rc.4      2022-11-09  historical prerelease
0.6.0-rc.3      2022-11-09  YANKED prerelease
0.6.0-rc.2      2022-09-11  historical prerelease
0.6.0-rc.1      2022-08-23  historical prerelease

0.5.17          2022-10-20  active
0.5.16          2022-09-10  active
0.5.15          2022-08-09  active
0.5.14          2022-07-25  YANKED
0.5.13          2022-07-15  active
0.5.12          2022-07-10  active
0.5.11          2022-07-02  active
0.5.10          2022-06-28  active
0.5.9           2022-06-20  active
0.5.8           2022-06-18  active
0.5.7           2022-06-08  active
0.5.6           2022-05-16  active
0.5.5           2022-05-10  active
0.5.4           2022-04-26  active
0.5.3           2022-04-19  active
0.5.2           2022-04-19  YANKED
0.5.1           2022-04-03  active
0.5.0           2022-03-31  active

0.4.8           2022-03-02  active
0.4.7           2022-03-01  active
0.4.6           2022-02-22  active
0.4.5           2022-01-31  active
0.4.4           2022-01-13  active
0.4.3           2021-12-21  active
0.4.2           2021-12-06  active
0.4.1           2021-12-06  active
0.4.0           2021-12-02  active

0.3.4           2021-11-17  active
0.3.3           2021-11-13  active
0.3.2           2021-11-08  active
0.3.1           2021-11-06  active
0.3.0           2021-11-02  active

0.2.8           2021-10-07  active
0.2.7           2021-10-06  active
0.2.6           2021-10-02  active
0.2.5           2021-09-18  active
0.2.4           2021-09-10  active
0.2.3           2021-08-26  active
0.2.2           2021-08-26  YANKED
0.2.1           2021-08-24  active
0.2.0           2021-08-23  active

0.1.3           2021-08-06  active
0.1.2           2021-08-01  active
0.1.1           2021-07-30  active
0.1.0           2021-07-30  active

0.0.0           2021-07-22  active placeholder
```

`0.0.0` is a placeholder, not a usable historical application baseline.
Historical prereleases are evidence for diagnosing prerelease source only; do
not target them in new stable code.

## Yanked releases

| Version | Disposition |
| --- | --- |
| `0.8.2` | Reject. Official changelog cites an unforeseen breaking change and issue/PR 3190. |
| `0.6.0-rc.3` | Reject. It did not compile in release mode. |
| `0.5.14` | Reject. Official changelog records an accidental breaking change. |
| `0.5.2` | Reject. Official changelog records an accidental breaking change. |
| `0.2.2` | Reject. Crates.io has no explicit reason; 0.2.3 repaired an accidental `BoxRoute` `Sync` regression, but connecting that to the yank is an inference. |

An existing lockfile can preserve a yanked version. Report it and require an
explicit authorized dependency update; never silently rewrite the lockfile.

## Release-line transitions

Review every boundary between the evidenced source and target. Point releases
can matter too, especially when the target uses a later 0.8 facility.

### 0.0/0.1 to 0.2

- Remove `prelude`; import explicitly.
- Replace `RoutingDsl`, free `axum::route`, and free nesting with
  `Router::new().route(...)` and router methods.
- Replace `UrlParams`/`UrlParamsMap` with `Path<T>`.
- `extract::Body` became `RawBody`, which is also gone now; target
  `axum::body::Body`.
- Replace old `axum::sse` services with `response::sse::Sse`/`Event`.
- Replace old WebSocket routing helpers with the `WebSocketUpgrade` extractor.
- Replace `BoxStdError` according to role: Axum error wrapper or Tower-facing
  `BoxError`.

### 0.2 to 0.3

- Return a plain router rather than a route-composition-specific generic type.
- Remove `Route::boxed`, `BoxRoute`, public `Nested`/`Or`, and `routing::Layered`.
- Replace `Router::or` with `Router::merge` and resolve route conflicts.
- Move handler method routing from `axum::handler` to `axum::routing`.
- Remove `Router::check_infallible`; routed services must expose an infallible
  boundary, with fallible work converted to responses.
- Treat conflicting routes as a startup panic, not silent shadowing.

### 0.3 to 0.4

- Use unified `MethodRouter` and `routing::{get, post, get_service, ...}` rather
  than handler/service method-router modules.
- Replace `HandleErrorExt` with current error-handling service/layer APIs.
- `box_body` first became `boxed`; both are obsolete now. Target `Body::new`.
- Replace `PathParamsRejection` with current `PathRejection` variants.
- Remove old `IntoResponse` body-associated types; current responses normalize
  to Axum's body.
- Use `axum-core` for integration traits when the full framework is unnecessary.

### 0.4 to 0.5

- Remove `AddExtensionLayer`. Use `State`/`with_state` for application state or
  current `Extension(value)` only for request-extension data.
- Replace removed `Redirect::found` by intended semantics. Current constructors
  are 303/307/308; build an explicit 302 response if exact FOUND behavior matters.
- Replace `Headers` response wrapper with response tuples, header arrays/maps, or
  an authorized current header helper.
- Replace `InvalidJsonBody` handling with `JsonDataError`/`JsonSyntaxError` and
  keep non-exhaustive matching.
- Start every route with `/`.
- Replace deprecated `extractor_middleware` with current
  `middleware::from_extractor` only when extractor reuse is intended; otherwise
  prefer `from_fn`.

### 0.5 to 0.6

- Move normal application state from `Extension` to `State<T>` plus
  `.with_state(state)`; derive focused state through `FromRef`.
- Split custom extraction into `FromRequestParts<S>` and `FromRequest<S>`.
- Rewrite old `impl<S, B> FromRequest<S, B>`/`FromRequestParts<S, B>` forms to
  current state-only trait generics and current `Request`/`Body` parameters.
- Put the sole body-consuming extractor last.
- Remove `RequestParts` and `BodyAlreadyExtracted`; use `http::request::Parts`
  inside the current trait or a full `Request`.
- Replace `ContentLengthLimit` with `DefaultBodyLimit::max` when extractor-local
  limiting matches intent. Use a service-level limit only when policy requires it.
- Use `route_service`, `nest_service`, and `fallback_service` for arbitrary
  Tower services; handler/router forms stay with `route`, `nest`, and `fallback`.
- Replace implicit trailing-slash redirects with explicit paths/redirects or an
  authorized current `axum-extra` `route_with_tsr`.
- Treat `with_state` as supplying a missing state, not a router constructor.
  Remove `inherit_state` and old `RouterService` patterns.

### 0.6 to 0.7

- Move the HTTP stack to Hyper/HTTP/http-body 1.x-compatible types.
- Remove request-body type parameters from `Router`, `MethodRouter`, `Handler`,
  `Next`, extractors, and related services.
- Replace concrete/re-exported `hyper::Body` with `axum::body::Body` in Axum code.
- Replace `RawBody` with `Body` and `BodyStream` with
  `Body::into_data_stream()` for data frames or `http_body_util::BodyStream` when
  trailers/all frames are required. Current 0.8 `Body` is not itself a `Stream`.
- Replace `Empty`, `Full`, `BoxBody`, `box_body`, and `boxed` with
  `Body::empty`, `Body::from`, and `Body::new`.
- Replace `axum::Server` with a runtime listener plus `axum::serve`.
- Move old Axum `TypedHeader`/`headers` usage to an authorized current companion
  or parse headers directly.
- Replace misspelled `DefaultOnFailedUpdgrade`/`OnFailedUpdgrade` names.
- Remove `WebSocketUpgrade::max_send_queue`. It counted messages and has no
  numeric drop-in replacement. Current write-buffer controls count bytes and
  govern different behavior; derive a new byte budget or add an application-level
  bounded channel for the original queue/backpressure invariant.

### 0.7 to 0.8

- Replace `/:id` and `/*rest` with `/{id}` and `/{*rest}`. Old syntax panics.
- Ensure routed handlers/services satisfy new `Sync` requirements.
- Make tuple/tuple-struct `Path` shapes exactly match the capture count.
- Remove `WebSocket::close`; send `Message::Close` and complete the handshake.
- Adapt WebSocket message payloads from `String`/`Vec<u8>` assumptions to
  `Utf8Bytes`/`Bytes`.
- Adapt custom listeners to generic `serve` and current `serve::Listener` APIs.
- Replace removed `Serve::tcp_nodelay` with `ListenerExt::tap_io` per accepted IO.
- Do not use `Option<Path<T>>` as an error suppressor; malformed present values
  reject. `Option<Query<T>>` is removed.
- Do not map the moved `Host` extractor to deprecated `axum_extra::extract::Host`.
  Select literal-host, URI-authority, or trusted-proxy behavior explicitly.
- Route WebSockets with `any` when HTTP/2 CONNECT support is required.

## Canonical current replacements

| Reject or review legacy form | Current Axum 0.8.9 disposition |
| --- | --- |
| `axum::prelude::*` | Use explicit imports. |
| `RoutingDsl`, free `axum::route` | Use `Router::new().route(...)`. |
| `axum::handler::{get, post, ...}` | Use `axum::routing::{get, post, ...}`. |
| `Router::or` | Use `Router::merge`. |
| `/:id`, `/*rest` | Use `/{id}`, `/{*rest}`. |
| `.without_v07_checks()` with old captures | Use brace captures and remove it. Retain only for an intentional literal `:`/`*` segment with explicit tests. |
| `UrlParams`, `UrlParamsMap` | Use `Path<T>` or, when justified, `RawPathParams`. |
| `Extension<T>` as normal app state | Use `State<T>` + `.with_state`; use `FromRef` for substates. |
| `AddExtensionLayer` | Use `Extension(value)` only for request extensions; otherwise state APIs. |
| `RequestParts`, `BodyAlreadyExtracted` | Use `http::request::Parts` with `FromRequestParts`, or full `Request`. |
| `#[axum::async_trait]` on extractor impls | Use current native async trait method implementations. |
| `FromRequest<S, B>` / `FromRequestParts<S, B>` body generic | Remove `B`; use the current state generic and Axum `Request`/`Body`. |
| `ContentLengthLimit` | Use `DefaultBodyLimit::max(...)` or an intentional service-level limit. |
| `extractor_middleware` | Use `from_extractor[_with_state]`; use `from_fn` for middleware-only policy. |
| service passed to `route`/`nest`/`fallback` | Use `route_service`/`nest_service`/`fallback_service`. |
| automatic trailing-slash redirect | Define behavior explicitly or authorize `route_with_tsr`. |
| `BoxBody`, `box_body`, `boxed` | Use `Body::new`. |
| response `Empty`/`Full` | Use `Body::empty`/`Body::from`. |
| `RawBody` | Use `axum::body::Body`. |
| `BodyStream` | Use `Body::into_data_stream()` or `http_body_util::BodyStream` by frame needs. |
| `axum::Server`, `Server::bind` | Use a Tokio listener plus `axum::serve`. |
| `Serve::tcp_nodelay` | Use `ListenerExt::tap_io` and configure each accepted stream. |
| generic `Next<B>` | Use non-generic `Next`. |
| old body generic on routers/handlers/extractors | Remove it; current request body is Axum `Body`. |
| `axum::TypedHeader`, `axum::headers` | Use authorized `axum-extra` typed-header support or direct current HTTP parsing. |
| `axum::extract::Host` | Literal Host: typed header; URI authority: `Uri`; proxy-aware: custom trusted-proxy extractor. |
| deprecated `axum_extra::extract::Host`/`Scheme` | Replace with purpose-specific current extraction; there is no universal safe mapping. |
| deprecated `axum_extra::extract::OptionalPath<T>` | Use `Option<Path<T>>` with current semantics. |
| `Option<Query<T>>` | Use `Query<T>` fields/defaults, local `Result`, or current `OptionalQuery<T>` when absence must differ. |
| `WebSocketUpgrade::max_send_queue` | No numeric drop-in. Choose byte-based write limits and/or an application bounded channel from the required invariant. |
| Axum `WebSocket::close()` | Send `Message::Close`; generic sink `.close()` requires manual review. |
| WS `Text(String)`/`Binary(Vec<u8>)` assumptions | Use current constructors/conversions to `Utf8Bytes`/`Bytes`. |
| old `axum::sse` service | Use `response::sse::{Sse, Event, KeepAlive}`. |
| `Redirect::found` | Select 303/307/308 intentionally, or build exact 302 response. |
| `Headers` response wrapper | Use response parts/tuples and current headers. |
| `axum-debug` | Use Axum macros' `debug_handler`/`debug_middleware` when enabled. |
| Tower HTTP `TimeoutLayer::new`, `Timeout::new`, `Timeout::layer` | On 0.6.7+, use the corresponding explicit `with_status_code` constructor. |
| Tower HTTP `cors::any()` | Use the `cors::Any` unit struct. |
| Tower HTTP basic `auth::require_authorization` | Use application validation middleware or current `AsyncRequireAuthorizationLayer` by contract. |

No mapping authorizes a new companion dependency. Apply the repository's
dependency policy before selecting Axum Extra, Tower HTTP, Hyper Util, headers,
or HTTP Body Util.

## Modern 0.8 point features

Foreground facilities only when the repository's exact patch contains them:

| Patch | Relevant addition/change |
| --- | --- |
| `0.8.0` | New 0.8 baseline: brace route syntax, exact tuple paths, HTTP/2 WebSockets, and generic listener/IO; retains `NoContent` and method-not-allowed fallback introduced in 0.7.8. |
| `0.8.1` | Documentation/readme correction only. |
| `0.8.2` | Yanked; never target. |
| `0.8.3` | Optional `Json`/`Extension`, `Message: From<Bytes>`, WebSocket read-buffer control. |
| `0.8.4` | `Router::reset_fallback`, WebSocket selected protocol, serve task-leak fix. |
| `0.8.5` | Strict JSON trailing-character rejection, optional `Multipart`, `Event::into_data_writer`/`EventDataWriter` for formatted SSE data, `ResponseAxumBodyLayer`, invalid redirect becomes 500, MSRV 1.78; paired Axum Core 0.5.3 adds `DefaultBodyLimit::apply`. |
| `0.8.6` | Re-release for docs.rs; no code change. |
| `0.8.7` | Relaxed implicit `Send`/`Sync` bounds on router service wrappers. |
| `0.8.8` | `route_layer` documentation clarification. |
| `0.8.9` | Flexible WebSocket requested/selected protocols, connect endpoint fix, multipart limit diagnostics, MSRV 1.80. |

## Companion generation boundaries

Keep companion versions independent. Axum 0.8.9's tagged dependency ranges are
not a command to install each ecosystem's newest breaking line.

### Hyper 0.14 to 1.x

- Concrete old `hyper::Body` no longer crosses the Axum boundary; use Axum Body.
- Use `axum::serve` for ordinary Axum apps. Use Hyper Util connection builders
  only for advanced connection control.
- Hyper 1 and Tower expose distinct `Service` traits; adapt with the current
  Hyper Util bridge when crossing that boundary.

### HTTP Body 0.4 to 1.x

- `poll_data`/`poll_trailers` became frame-based `poll_frame`.
- Combinators live in `http-body-util`.
- Select collect/map/frame/stream adapters according to whether trailers matter.

### Tower 0.4 to 0.5

- Replace `ready_and`/`ReadyAnd` with `ready`/`Ready`.
- Replace `Either::A/B` with `Either::Left/Right`.
- Re-check retry policy signatures and readiness/error behavior.

### Tower HTTP

This repository's `tower-http 0.6.11` is transitive through Reqwest, not an Axum
middleware dependency. Do not edit it as if the web router owned it.

Tower HTTP 0.6.11 already deprecates the old timeout constructors in favor of
explicit status-code forms, `cors::any()` in favor of `cors::Any`, and the basic
`auth::require_authorization` module. An explicit 0.7 upgrade also requires
reviewing removed no-op feature names and changed feature coupling. Do not assume
that deprecated 0.6.11 APIs remain acceptable merely because 0.7 is not pinned.

Multiple HTTP/Hyper/Tower generations may legitimately appear transitively.
Flag old-generation types only when they cross the Axum-facing API boundary or
create an unintended duplicate direct dependency.

## Stable versus unreleased main

Do not present these upstream-`main` items as Axum 0.8.9 APIs:

- `ListenerExt::limit_connections`
- `Serve::with_executor`
- `MethodRouter::method_filter`
- `RawPathParams::from_request_extensions`
- `Redirect` implementing `IntoResponseParts`
- Redirect constructors accepting arbitrary `Into<String>` (0.8.9 takes `&str`)
- `serve` accepting arbitrary response body types
- new matchit captures with static prefixes/suffixes
- changed `serve` future output or nested fallback merging

This list is a guard, not an exhaustive 0.9 preview. Exact tagged docs must
always outrank a branch page or search result.

## Migration procedure

1. Identify the evidenced source version and exact target version.
2. Reject a yanked/prerelease target unless the repository already pins it and
   the task explicitly addresses that state.
3. Read every intervening release-line section and the exact patch changelogs
   relevant to used facilities.
4. Inventory each legacy occurrence before editing.
5. Map each occurrence to a verified replacement, architectural removal, or
   explicit no-one-for-one-replacement decision.
6. Preserve behavior: paths, status codes, URI visibility, state scope,
   rejection shape, body bounds, layer order, connection metadata, shutdown,
   and protocol lifecycle.
7. Run the legacy audit, exact-target compiler with deprecations denied, targeted
   request tests, and the rubric.
8. Report every exception and every unvalidated target. Never call a migration
   complete while a hard gate fails.

## Primary sources

- [Axum 0.8.9 exact documentation](https://docs.rs/axum/0.8.9/axum/)
- [Axum 0.8.9 packaged changelog](https://docs.rs/crate/axum/0.8.9/source/CHANGELOG.md)
- [Axum 0.8.9 official release](https://github.com/tokio-rs/axum/releases/tag/axum-v0.8.9)
- [Axum published versions](https://crates.io/api/v1/crates/axum/versions)
- [Axum tagged 0.8.9 changelog](https://github.com/tokio-rs/axum/blob/axum-v0.8.9/axum/CHANGELOG.md)
- [Axum Core 0.5.6](https://docs.rs/axum-core/0.5.6/axum_core/)
- [Hyper 1 upgrade guide](https://hyper.rs/guides/1/upgrading/)
- [HTTP Body Util 0.1.4](https://docs.rs/http-body-util/0.1.4/http_body_util/)
- [Tower 0.5.3](https://docs.rs/tower/0.5.3/tower/)
