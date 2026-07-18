# Share and player contract v1

Status: local contract implemented; production adapters and protected evidence pending.

Source reference: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Boundary

`frame-web::share_player` is the provider-neutral decision boundary for public,
unlisted, tenant, private, password, top-level, and embedded playback. It owns
authorization-state collapse, route scoping, metadata policy, single-range
planning, embed messages, comment commands, transcript documents, analytics
consent, custom-domain bindings, and cache partitions. It contains no D1 row,
R2 key, signed provider URL, Media Transformations request, Cloudflare type, raw
password, session token, email address, or browser fingerprint.

The server may render a ready page only after an adapter supplies an authorized
`PublicShareSummary` and the web boundary verifies that its canonical share ID,
media path, and every caption path belong to the requested route. Managed and
native derivative executors both produce the same
`/api/v1/public/shares/{share}/media` browser descriptor; executor identity and
object names do not cross the boundary. Production currently has no web-to-
Worker share adapter and therefore stays in the generic unavailable state.

## Privacy and metadata matrix

The machine-readable source of truth is
`fixtures/web-share-player/v1/contract.json`. The important rules are:

| Resource | Anonymous | Same tenant | Owner/exact grant | Metadata |
| --- | --- | --- | --- | --- |
| public ready | ready | ready | ready | title/description/OG, indexable only top-level |
| unlisted ready | ready | ready | ready | title only after authorization; noindex |
| tenant ready | unavailable | ready | ready | authorized response only; noindex/no-store |
| private ready | unavailable | unavailable | ready | authorized response only; noindex/no-store |
| password ready | challenge | challenge | ready | challenge is generic; no title/thumbnail/OG |
| processing | privacy authorization still applies | privacy authorization still applies | processing | always generic and noindex |
| failed/deleted/unavailable | unavailable | unavailable | unavailable | byte-equivalent generic policy |

An invalid ID, cross-tenant identity, expired grant, route/descriptor mismatch,
disabled embed, private iframe request, failed media job, deletion, and missing
row all become the same 404 resolution. A top-level password challenge is the
only deliberate existence disclosure. Its verified proof must be a bounded
digest scoped to the exact share and an unexpired server-side grant. Password
input, hashing, attempt counters, and recovery never belong in HTML or this
contract.

Public ready top-level pages may emit title, description, canonical URL, and
OpenGraph text. Processing, password, embed, unlisted, authenticated, and
unavailable responses are noindex. No v1 page emits an OpenGraph thumbnail:
omitting it is safer than accepting an object or bearer URL before the public
thumbnail adapter and revocation evidence exist. Embed canonical URLs point to
the top-level `/s/{share}` route so crawlers do not create a second identity.

## Playback and accessibility

The SSR fallback uses semantic headings, a labelled native `<video controls>`
element, `playsinline`, metadata-only preload, captions, transcript links, and
static keyboard instructions. Hydration adds keyboard-operable play/pause,
ten-second seek, a closed speed set (0.5× through 2×), fullscreen, picture in
picture, retry, and a polite status region. Fullscreen and picture in picture
remain feature- and browser-policy gated. Remote playback and download controls
are not offered. CSS removes non-essential transitions and animation when
`prefers-reduced-motion: reduce` is active.

The media adapter must implement one RFC 9110 byte range, `If-Range`, exact
`Content-Range`, `Accept-Ranges: bytes`, 206 for valid ranges, and 416 for
invalid or multi-range input. A validator mismatch deliberately returns the
whole current representation. The planner is integer-only and bounds every
position to the 24-hour contract maximum. It never signs or transforms an
object per viewer request: it addresses an already persisted derivative through
the same-origin public media capability.

## Embed protocol

Embedding is disabled unless configuration supplies at least one exact origin.
The response CSP emits only those origins in `frame-ancestors`; non-embed pages
retain `frame-ancestors 'none'` and `X-Frame-Options: DENY`.

Inbound messages use `frame.embed-command.v1`, the exact share ID, a strictly
increasing non-zero sequence, and one closed command: play, pause, seek, set
playback rate, or request state. The browser requires an exact configured
`MessageEvent.origin`, verifies that `source` is the parent window, rejects
unknown JSON fields, validates all command bounds, and advances the replay
fence only after validation. It silently drops invalid messages so it does not
become a cross-origin oracle. An accepted command receives
`frame.embed-reply.v1` only at the already validated origin; wildcard
`postMessage` targets are forbidden.

## Comments, transcript, and analytics

Comment commands carry only digests and exact tenant/share scopes. The contract
requires an unexpired comment grant, closed text/reaction kinds, bounded text,
same-share reply scope, bounded timeline position, a repository rate snapshot,
and an immutable operation/payload digest. A same-operation/same-payload replay
is idempotent; a different payload is rejected. Tenant policy can disable
anonymous comments or require pre-moderation. The SSR fallback never submits a
comment without the adapter and grant.

Transcript documents have a validated BCP-47-like language, 24-hour duration
cap, 20,000-segment/1 MiB aggregate bounds, ordered timestamps, bounded speaker
labels, and control-character rejection. Debug output reports counts and timing
only, never spoken text. Caption WebVTT remains the accessible no-JavaScript
fallback.

Analytics has no implicit consent. A disabled or ungranted policy returns an
ignore decision before persistence. A record requires exact tenant/share/
policy scope, unexpired consent, a digest-only session identity, a closed event
kind, bounded clock skew and position, a rate snapshot, and idempotent replay.
The type has no IP address, user agent, referrer, device fingerprint, free-form
error, email, or user name field. Retention is closed to 1–90 days.

## Legacy routes, domains, and caches

`/s/{id}` and `/embed/{id}` remain canonical. `/share/{id}` returns a permanent
redirect to `/s/{id}` and never propagates query data. IDs containing escaping,
slashes, percent encoding, or traversal are unavailable.

Custom-domain resolution accepts only an exact lower-case, non-wildcard,
non-IDN host with a live binding for the exact share, a valid tenant digest,
and a positive revision. Revocation or any subdomain mismatch is unavailable.
Cookies remain host-only; a custom domain must never receive the canonical
Frame session cookie. Every viewer response is `private, no-store, max-age=0`
in v1. Internal invalidation partitions include tenant, share, privacy,
deletion, and domain revisions so a privacy/delete/domain transition cannot
reuse an older partition.

## Protected completion boundary

The following fixture identifiers are intentionally verbatim. They are pending
and cannot be converted to pass by a unit test or document edit:

- `production_share_authorization_adapter`
- `d1_comment_transcript_analytics_adapters`
- `r2_range_and_revocation_traces`
- `verified_custom_domain_routing`
- `password_hash_and_delivery_service`
- `browser_device_accessibility_execution`
- `cdn_privacy_invalidation_slo`
- `canary_and_rollback_observation`

Until those are attached, this document establishes the executable local
contract, not production acceptance or issue closure.
