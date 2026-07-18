# Frame public contract v1

## Boundary and data classification

`frame-client` is the only supported Rust boundary for anonymous Frame data.
It owns wire DTOs, version negotiation, validated URL construction, redacted
errors, and an optional bounded native HTTP adapter. The dependency direction
is one way: an external consumer may pin Frame; no Frame manifest may depend on
the EngManager portfolio repository.

Authenticated first-party DTOs live in the separate
`frame-authenticated-client` package. They are never re-exported from
`frame-client`, do not expand this anonymous allowlist, and are not supported
as an external portfolio integration contract.

The v1 allowlist is deliberately small:

| Classification | Public v1 fields | Never public |
| --- | --- | --- |
| Service | `service`, coarse `status`, release identifier, contract major, coarse capability names | binding IDs, database or bucket names, regions, counts, dependency status, stack traces |
| Share | explicitly public title and description, availability, duration, canonical share URL, and the versioned coarse public processing projection described below | owner identity, tenant/organization ID, comments, transcript, local recovery state, provider state, session identifiers, private/deleted/failed existence |
| Playback | same-origin capability paths, approved media type, range support, reviewed caption descriptors | R2 keys, signed URLs, provider IDs, checksums, credentials, ungoverned objects |
| Error | stable safe code/message, opaque request ID, retry advice | response body excerpts, internal causes, queries, cookies, authorization values |

Anything not named in the allowlist is internal. An anonymous private,
deleted, failed, unknown, malformed, or policy-invalid share is the same
byte-for-byte `unavailable` representation. Processing may expose only its
canonical public page and an optional `processing_status`; it exposes no title,
metadata, or media path. The v1 status can report only `uploading` or
`finalizing`, an optional real progress value in basis points, a retry flag,
and the matching coarse `upload_delayed` or `finalize_delayed` code. Local
storage, recovery, cancellation, raw network/provider errors, byte counts,
object keys, and identifiers remain private.

The Worker publishes this projection only from retained D1 truth. A current
finalize row may produce an indeterminate `finalizing` status or the coarse
retrying/delayed variant. Missing legacy detail leaves the optional field
absent; contradictory, terminal, or otherwise unsafe state fails closed to
the same unavailable representation. Migration
`0035_instant_finalize_public_share_index.sql` keeps both latest-row projections
covering and deterministic; migration conformance rejects a temporary sort or
either missing index lookup. The Worker never invents a percentage. The richer
desktop `frame-media` progress model is not a wire contract.

## Endpoint inventory

All endpoints use the configured origin and the `/api/v1` prefix. There is no
second API hostname.

| Method and path | Contract | Authentication | Notes |
| --- | --- | --- | --- |
| `GET /api/v1/health` | `Health` | anonymous | Exactly the public DTO; the legacy `/health` dependency diagnostics are not included. |
| `GET /api/v1/public/shares/{public_id}` | `PublicShareSummary` | anonymous | Invalid and non-public identifiers degrade to the indistinguishable unavailable body. |
| `GET`, `HEAD`, `OPTIONS /api/v1/public/shares/{public_id}/media` | governed public bytes | anonymous | Only active, clean, public derivatives; bounded single-range semantics and an exact-origin CORS policy. |

Identifiers are bounded ASCII capability segments. DTO media paths are exact
same-origin paths; redirects and arbitrary derivative paths are not part of
the client contract. Caption descriptors are an additive v1 field reserved
for a separately advertised capability; the current Worker emits an empty
caption list.

## Version and header policy

- The URL major and `api_version.major` are both normative and must agree.
- Clients send `Accept: application/json`. JSON DTO responses use
  `Content-Type: application/json`; other media types fail closed.
- No ambient cookie, bearer token, proxy, or cross-origin credential is used
  by the anonymous client. The native adapter disables redirects and ambient
  proxy discovery.
- Unknown JSON fields and unknown capability names are additive within a
  major and must be ignored. Removing a field, changing a type or meaning,
  weakening privacy, or changing endpoint semantics requires a new major.
- A client accepts exactly the major it was compiled for. An incompatible
  major returns the stable redacted `incompatible_version` error.
- When major `N` is introduced, the server retains `N-1` for at least 180 days
  after a dated deprecation notice and until every pinned first-party consumer
  has moved. The v1-only launch has no predecessor to retain.

Capabilities gate optional behavior; they never override the contract major.
`instant_processing_status` advertises the public-safe projection, but each
share still validates independently and legacy processing responses may omit
the optional field. Consumers must degrade to a static link or last-known-good
snapshot when a capability or Frame itself is unavailable.

## Transport policy

The default policy is a three-second operation deadline, 64 KiB maximum JSON
body, and at most two attempts. Configuration is bounded to 30 seconds, 1 MiB,
and three attempts. Only idempotent methods are retryable, only transport or
deadline failures qualify, and dropping the returned future cancels in-flight
Reqwest work. Redirects, redirect headers, oversized bodies, non-JSON content,
and malformed responses fail closed without retaining or logging response
data.

The client intentionally does not perform public media downloads. A consumer
must resolve a validated same-origin media capability separately; support for
any reviewed CDN origin would require a new explicit API, not a redirect
exception in the JSON client.

## Release rules

Canonical fixtures and `contract.schema.json` are reviewed source artifacts.
Rust fixture tests, the fixture policy checker, native/wasm compilation, and
the forbidden-dependency gate must pass in the same change. Publish to
crates.io remains disabled; the initial consumer pins a full 40-character Git
revision and commits the resolved root lockfile.
