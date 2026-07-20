# Modern Axum extractors and responses

Use these rules for Axum 0.8.x. Verify feature gates and signatures against the
repository's exact patch before applying an example elsewhere.

## Contents

- [Extractor model](#extractor-model)
- [Handler contract](#handler-contract)
- [Ordering and body ownership](#ordering-and-body-ownership)
- [Built-in extractor choices](#built-in-extractor-choices)
- [Optional and fallible extraction](#optional-and-fallible-extraction)
- [Body limits](#body-limits)
- [Custom extractors](#custom-extractors)
- [Response construction](#response-construction)
- [Application errors](#application-errors)
- [Bodies and streaming](#bodies-and-streaming)
- [Review checklist](#review-checklist)
- [Primary sources](#primary-sources)

## Extractor model

Select extraction by what it consumes:

| Need | Current mechanism | Body-consuming? |
| --- | --- | --- |
| Router state | `State<T>` | No |
| Path captures | `Path<T>` | No |
| Query string | `Query<T>` | No |
| Headers/method/URI | `HeaderMap`, `Method`, `Uri` | No |
| Request extension | `Extension<T>` | No |
| Connection metadata | `ConnectInfo<T>` with a matching make service | No |
| JSON/form | `Json<T>`, `Form<T>` | Yes |
| Buffered bytes/text | `Bytes`, `String` | Yes |
| Multipart | `Multipart` with the `multipart` feature | Yes |
| Full request/body | `Request`, `axum::body::Body` | Yes |

Use `FromRequestParts<S>` for a custom extractor that only reads request parts.
Use `FromRequest<S>` when it consumes or transforms the request body. Do not add
the removed request-body generic from pre-0.7 signatures.

## Handler contract

Axum 0.8.9 implements ordinary function handlers with zero through 16
parameters. For an extractor-bearing handler, the exact generated contract is:

- the callable is `Clone + Send + Sync + 'static`;
- its returned future is `Send`, and the handler's boxed future is also
  `'static`;
- its output implements `IntoResponse`;
- the first zero or more parameters implement `FromRequestParts<S> + Send`;
- the final parameter implements `FromRequest<S, M> + Send`; and
- extractor-bearing handler state is `Send + Sync + 'static`.

A parts-only extractor can occupy the final parameter because Axum bridges
`FromRequestParts` through the current `ViaParts` marker. The final position
does not require a body consumer; it merely reserves the only position that may
consume the body. Consolidate more than 16 request inputs into meaningful typed
extractors or state instead of working around the handler limit.

When inference hides a contract failure, use `#[axum::debug_handler]` for a
focused diagnostic if the repository already enables Axum's `macros` feature.
Do not add that feature incidentally.

## Ordering and body ownership

Extractors execute left to right. When a handler has a body-consuming
`FromRequest` extractor, every parts-only extractor must precede it and the body
consumer must be final:

```rust
use axum::{
    body::Bytes,
    extract::State,
    http::HeaderMap,
};

#[derive(Clone)]
struct AppState;

async fn ingest(
    State(_state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) {
    let _ = (headers, body);
}
```

Do not buffer a body in middleware or a custom extractor and then expect a later
extractor to read it unless the middleware deliberately reconstructs and replaces
the body. Prefer one owner and one parsing path.

The full `Request` and `Body` are also body-consuming extractors and therefore
must be last. `middleware::Next` is not a handler extractor.

## Built-in extractor choices

- Deserialize path/query/body data into named structs when field meaning matters.
- Use `Path<(A, B)>` only when positional meaning is obvious and the tuple length
  exactly matches captures. Axum 0.8 rejects mismatched tuple counts.
- Use `HeaderMap` or typed parsing with an explicitly authorized companion crate.
  Axum's old `TypedHeader` and `headers` re-export are gone.
- Use `RawPathParams` only when application code truly needs untyped access to all
  captures; prefer `Path<T>` for validation and names.
- Use `MatchedPath` for route templates in telemetry, not for business routing.
- Use `OriginalUri` only when prefix stripping from nesting matters.
- Use `State` for application state and `Extension` for data carried in request
  extensions. Do not treat the latter as obsolete in its legitimate role.

`Host` requires a semantic decision. Axum's extractor moved out in 0.8 and the
later `axum_extra::extract::Host` is itself deprecated. For the literal Host
header, an authorized `axum_extra::TypedHeader<headers::Host>` is one option.
For proxy-aware authority, implement explicit trusted-proxy policy from URI and
forwarding headers. Do not use an unverified one-for-one replacement for a
security decision.

## Optional and fallible extraction

Use `Result<Extractor, Rejection>` when a handler intentionally handles a
malformed value itself:

```rust
use axum::{
    extract::rejection::JsonRejection,
    http::StatusCode,
    Json,
};
use serde_json::Value;

async fn ingest(
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Json<Value>, StatusCode> {
    payload.map_err(|_| StatusCode::BAD_REQUEST)
}
```

Use `Option<T>` only when that exact extractor implements the current optional
trait and absence is valid. Optional semantics are extractor-specific:

- malformed present input normally rejects rather than becoming `None`;
- `Option<Path<T>>` is `None` only when captures are truly absent;
- `Query<T>` does not implement current optional extraction in Axum 0.8;
- optional `Json`, `Multipart`, and `Extension` support depends on the exact
  0.8 patch and enabled features.

For query input, model optional fields inside `T` when an empty/missing query has
the same meaning. If absence from an empty query must be distinguished, evaluate
the current `axum-extra` `OptionalQuery` only under the repository's dependency
policy. Never use `Option<Query<T>>` as a catch-all rejection suppressor.

Rejection enums are commonly `#[non_exhaustive]`; keep a catch-all arm. Map
expected client failures deliberately and avoid exposing internal error chains.

## Body limits

Axum applies a default 2 MiB limit to `Bytes` and built-in extractors that use it,
including `String`, `Json`, and `Form`; current `Multipart` also honors the
default limit. Set the policy explicitly when endpoint needs differ:

```rust
use axum::{
    extract::DefaultBodyLimit,
    routing::post,
    Router,
};

let app = Router::new().route(
    "/reports",
    post(ingest).layer(DefaultBodyLimit::max(16 * 1024)),
);
# async fn ingest(_: axum::body::Bytes) {}
```

Understand the scope:

- `DefaultBodyLimit` configures Axum extractors that opt into that mechanism.
- It does not universally cap arbitrary `Request`/`Body` consumers or every
  third-party extractor.
- Inside a custom `FromRequest` extractor, current Axum Core provides
  `DefaultBodyLimit::apply(&mut request)`. Apply it before delegating to a
  limit-aware built-in extractor when the custom extractor owns a specific
  override; this does not turn arbitrary manual body reads into limit-aware reads.
- A Tower HTTP request-body limit can enforce a service-level limit when that is
  the actual requirement, but adding `tower-http` needs dependency approval.
- `DefaultBodyLimit::disable()` is not a modernization step. Use it only with an
  independently enforced bound or a verified streaming design.
- `axum::body::to_bytes(body, limit)` requires an explicit maximum in current
  Axum. Use a real bound for untrusted input, not `usize::MAX` by habit.

Test below-limit, at-limit, and above-limit requests through the actual layers.

## Custom extractors

Implement only the narrow trait needed. This current parts-only shape needs no
`async_trait` helper:

```rust
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};

struct ApiKey(String);

impl<S> FromRequestParts<S> for ApiKey
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let value = parts
            .headers
            .get("x-api-key")
            .ok_or((StatusCode::UNAUTHORIZED, "missing API key"))?
            .to_str()
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid API key"))?;
        Ok(Self(value.to_owned()))
    }
}
```

When migrating a 0.6 custom body extractor, remove the request-body generic as
well as the macro-era trait implementation. The current ordinary shape is
`impl<S> FromRequest<S> for Payload` with
`async fn from_request(request: Request, state: &S)`. Do not preserve forms such
as `impl<S, B> FromRequest<S, B>`, a `Request<B>` parameter, or body bounds on
`B`; extract from Axum's current `Request`/`Body` and preserve any deliberate
size or frame handling explicitly.

Delegate to built-in extractors where possible so their parsing/rejection
semantics remain consistent. When wrapping an arbitrary extractor that may or
may not consume the body, current Axum documentation requires separate
`FromRequestParts` and `FromRequest` implementations with compatible bounds.

In an integration library that only exposes extractor/response traits, depend on
`axum-core` when sufficient. Keep application-only conveniences in an Axum
application crate.

## Response construction

Prefer composable `IntoResponse` values:

```rust
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Serialize)]
struct Created {
    id: String,
}

async fn create() -> impl IntoResponse {
    (
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(Created { id: "42".into() }),
    )
}

fn normalized(value: impl IntoResponse) -> Response {
    value.into_response()
}
```

Use:

- `StatusCode`, tuples, header arrays/maps, and `Json` for ordinary responses;
- `Response<Body>`/the `Response` alias when status, headers, or body vary across
  branches and a single concrete return type is clearer;
- `NoContent` for an intentional current empty success response;
- `Redirect::to` (303), `temporary` (307), or `permanent` (308) according to
  required semantics.

`Redirect::found` had 302 semantics and has no exact modern constructor. If 302
must be preserved, build a controlled `Response` with `StatusCode::FOUND` and a
validated `Location` header. Do not casually replace 302 with 303/307/308.

## Application errors

For shared endpoint errors, implement `IntoResponse` once and keep handler
signatures typed:

```rust
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

enum ApiError {
    NotFound,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        };
        (status, Json(json!({ "error": code }))).into_response()
    }
}
```

Do not return a raw internal error through a generic string conversion. Separate
logging/diagnostic detail from the stable client contract. Let `rust-modern` own
general Rust error design; this section governs the HTTP mapping.

## Bodies and streaming

Use Axum's current body type and constructors:

- `Body::empty()` for an empty body;
- `Body::from(value)` for supported in-memory values;
- `Body::new(http_body)` to adapt a compatible body;
- `Body::from_stream(stream)` for a data stream;
- `Body::into_data_stream()` when consuming data frames only;
- `http_body_util::BodyStream` when trailers/all frames are required and that
  companion dependency is part of the contract.

The removed Axum `Full`, `Empty`, `BoxBody`, `box_body`, `boxed`, `RawBody`, and
`axum::extract::BodyStream` forms are not modern replacements. A current
same-named type such as `http_body_util::BodyStream` belongs to a different
companion API and must be judged by its exact path. Preserve streaming
backpressure, disconnect cancellation, and any trailers with integration tests.

## Review checklist

Apply rubric gates G4-G9 and scorecard categories 4-5. In this domain, confirm:

- when present, the sole body consumer is bounded and follows all parts-only
  extractors;
- optional extraction separates absence from malformed input;
- rejections, response status/headers, and redirect semantics are preserved; and
- current companion body types are distinguished by their fully qualified path.

## Primary sources

- [Axum 0.8.9 extract module](https://docs.rs/axum/0.8.9/axum/extract/index.html)
- [Axum Core 0.5.6 extractor traits](https://docs.rs/axum-core/0.5.6/axum_core/extract/index.html)
- [Axum 0.8.9 response module](https://docs.rs/axum/0.8.9/axum/response/index.html)
- [Axum 0.8.9 body module](https://docs.rs/axum/0.8.9/axum/body/index.html)
- [Axum 0.8.9 `DefaultBodyLimit`](https://docs.rs/axum/0.8.9/axum/extract/struct.DefaultBodyLimit.html)
- [Axum 0.8.9 `Redirect`](https://docs.rs/axum/0.8.9/axum/response/struct.Redirect.html)
