# Storage governance v1

Status: domain, application, D1 authority, and Worker route adapters implemented; deployed provider
observations remain protected gates.

## Security objective

Every object operation is authorized from a typed tenant-scoped record before an adapter sees an
object identifier. The public failure surface does not distinguish a missing object from an object
owned by another tenant. No public bucket, provider URL, guessed prefix, or provider listing is an
authority source.

The executable policy lives in `frame-domain::storage_governance`; the only application entry point
for grant minting, response policy, export, privacy changes, and deletion is
`frame-application::StorageGovernanceService`.

## Threat model

Protected assets are source media, generated media, captions, avatars, manifests, multipart state,
backups, signed grants, custom-domain bindings, object checksums, retention policy, legal holds, and
erasure evidence.

Trust boundaries are:

1. browser or embed to the same-origin application;
2. the application to the private object adapter;
3. a short-lived signed route to an immutable object generation;
4. a verified custom domain to the same authorization policy;
5. the private object adapter to a managed media transformation;
6. the lifecycle coordinator to origin, cache, backup, and erasure-verification adapters.

The policy assumes an attacker can know tenant and object identifiers, replay old URLs, vary case and
origin headers, race privacy/deletion changes, seed cached 404s, submit hostile media, enumerate
prefixes, collide idempotency inputs, and make an adapter return stale or mismatched data. Provider
credentials, cache purge authority, legal-hold authority, and malware-scanner identity are service
credentials and never browser capabilities.

## Object-role inventory

`GovernedObjectRole::ALL` is the closed lifecycle inventory:

| Role | Read | Generate/write | Delete source of truth |
| --- | --- | --- | --- |
| source | authorized playback/editor | immutable upload | manifest |
| recording segment | authorized editor | immutable upload | manifest |
| thumbnail | authorized playback | media service | manifest |
| preview | authorized playback | media service | manifest |
| spritesheet | authorized playback | media service | manifest |
| audio | authorized playback | media service | manifest |
| export | authorized owner/export worker | native renderer | manifest |
| caption | authorized playback/editor | caption adapter | manifest |
| avatar | authorized tenant member | profile service | manifest |
| manifest | internal plus authorized export | coordinator | direct record |
| multipart session | internal upload worker | upload coordinator | manifest/session record |
| backup copy | deletion verifier only | backup policy | manifest |

Provider listings and guessed prefixes may be used to discover anomalies, but never to declare an
export or deletion complete. `LifecycleInventory` rejects duplicate identifiers, cross-tenant rows,
zero sizes, more than 16,384 objects, arithmetic overflow, and omission of a role declared by the
manifest coverage set.

## Access matrix

All rows also require an active object, exact tenant binding, and the operation-specific surface.

| Operation | Viewer | Editor | Admin/owner | Internal service | Anonymous |
| --- | --- | --- | --- | --- | --- |
| read/range | same tenant | same tenant | same tenant | scoped purpose | public verified domain or exact signed grant |
| immutable write | no | yes | yes | media/backfill only | no |
| list | no | yes | yes | backfill/deletion only | no |
| copy | no | yes | yes | backfill only | no |
| sign | no | yes | yes | no | no |
| delete/restore | no | no | yes | deletion may delete | no |
| export | no | no | yes | export service | no |
| cache purge | no | no | yes | deletion service | no |
| custom domain | no | no | yes | no | no |

The raw direct-origin surface is denied for every principal. A managed-media surface accepts only the
media-processor service and only read, range, or immutable-write operations. Cross-tenant requests
are denied before grant, domain, or role evaluation.

## Signed and cached reads

A signed grant binds exactly:

- tenant;
- governed object identifier;
- immutable object revision;
- cache generation through the governed record;
- read or range-read operation;
- expiry of at most 15 minutes;
- an opaque nonce digest.

The browser carries the high-entropy server-generated secret in the `FrameStorage` authorization
header. The application computes a versioned HMAC before policy evaluation; D1 persists only that
digest and the exact grant contract. The raw secret is never persisted, placed in a query string, or
logged, and a grant record is not accepted as a self-authenticating client token. Key rotation keeps
explicit verification versions without changing the original expiry.

Changing privacy increments the cache generation and requires purge of both positive and negative
cache variants. The prior generation is never overwritten. The invalidation deadline is bounded to
60 seconds by the local contract and migration charter.
Expired or prior-generation grants fail even if a cache still holds bytes.

Custom domains are canonical lowercase DNS names, have a non-zero verification version, bind to one
tenant, and must be active. Unlisted/private custom-domain reads still need an exact signed grant.

## Browser response policy

The response builder validates the media content type and emits `nosniff`, a sandboxed CSP, range
support, a safe fixed content-disposition filename, explicit cache control, and CORP. CORS uses exact
origin membership, only `GET`, `HEAD`, and `OPTIONS`, only range/conditional request headers, no
credential wildcard, and `Vary: Origin`. User-provided filenames never enter response headers.

## Untrusted media and managed transformations

Pending, rejected, quarantined, tombstoned, and erased media cannot be read. The managed-media port
accepts a typed private object handle—not a URL—so arbitrary schemes, hosts, redirects, DNS rebinding,
and private-network targets are unrepresentable. It requires a clean source/segment, exact tenant,
bounded size, immutable revision, checksum, and cache generation. Derivative cache identity hashes
all of those fields with the normalized profile digest.

The managed-media incident kill switch denies new invocations before any provider call. Existing
immutable outputs remain subject to normal authorization and manifest reconciliation.

## Encryption and secret handling

Origin buckets remain private and must have provider encryption-at-rest enabled. Provider tokens,
custom-domain verification secrets, signing keys, and purge credentials live only in the deployment
secret facility, are scoped by purpose, and are never stored in object metadata, D1, logs, manifests,
or evidence artifacts. Rotation must overlap verification keys without extending a grant's original
expiry. Production encryption configuration and rotation rehearsal are protected evidence; local
tests do not assert provider state.

## Security review decision

The Worker applies the policy to upload, public playback, signed reads, privacy changes, and managed
media source/output boundaries; its D1 adapter persists grants, domains, quota reservations,
manifest deletion workflows/evidence, cache operations, and audit records. Production promotion
still requires private-bucket inspection, provider cache timing, custom-domain routing tests,
malware-scanner validation, secret rotation, and an authorized review of real headers and range
behavior.
