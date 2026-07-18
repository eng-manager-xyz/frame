# Storage governance local evidence

Evidence date: 2026-07-16. Production evidence: **false**.

## Implemented contract

- Closed 12-role lifecycle inventory covering source, outputs, segments, thumbnails, captions,
  avatars, manifests, multipart sessions, and backups.
- Centralized authorization for read, range, immutable write, list, copy, sign, delete, restore,
  export, cache purge, and custom-domain management.
- Fail-closed direct-origin, cross-tenant, signed-route, custom-domain, and managed-media surfaces.
- Exact signed-grant tenant/object/revision/checksum/generation/operation/expiry and presented-nonce
  HMAC binding, server-generated secret, versioned key rotation, durable revocation, and revalidated
  persistence wire format.
- Exact-origin CORS and hardened content/range response headers.
- Immutable cache generations and executable positive/negative purge-and-absence probes.
- Typed URL-free managed-media input, deterministic derivative identity, and incident kill switch.
- Bounded quota, lifecycle manifest, audit chain, retention, legal hold, restore, export, deletion, and
  privacy-safe erasure-proof contracts.
- Tenant-bound D1 repositories and Worker adapters for governed objects, signed routes, custom
  domains, quota reservations, privacy transitions, deletion evidence, and audit persistence.

## Executed local gates

```text
cargo test -p frame-domain storage_governance --lib
12 passed; 0 failed

cargo test -p frame-application storage_governance --lib
3 passed; 0 failed

cargo test -p frame-application --test storage_governance_v1
7 passed; 0 failed

cargo test -p frame-control-plane --lib
60 passed; 0 failed

cargo clippy -p frame-domain -p frame-ports -p frame-application -p frame-control-plane \
  --all-targets -- -D warnings
passed

cargo check -p frame-control-plane --target wasm32-unknown-unknown
passed

RUSTDOCFLAGS='-D warnings' cargo doc -p frame-domain -p frame-ports \
  -p frame-application -p frame-control-plane --no-deps
passed

Python sqlite3 ordered application of migrations 0001 through 0021
passed; PRAGMA foreign_key_check returned zero rows
```

The integration penetration matrix evaluates all 11 operations across same-origin, direct-origin,
signed, custom-domain, and managed-media surfaces for a cross-tenant actor and separately proves that
direct origin is denied even to the resource owner. The lifecycle rehearsal covers all 12 roles,
hold/release, exact stage evidence, cache purge, backup deletion, absence verification, completion,
forged evidence, manifest swap, concurrent outstanding quota reservations, hold races, executable
positive/negative cache absence probes, durable HMAC revocation, and privacy-safe proof
serialization. The Worker route suite also covers the closed raw-path classifier and exact range
parser. These are local/simulated observations, not production Cloudflare evidence.

The repository-wide migration scanner and compiled-Worker D1 conformance now pass all 21 ordered
migrations under pinned Wrangler 4.111.0. The business authority expansion is deliberately split
across migrations 0011 and 0016–0021 so each provider migration remains below D1's
compound-expression ceiling; storage migration `0013_storage_governance_runtime.sql` remains
expand-only and passes in the complete chain.

## Protected promotion evidence

The following need authorized Cloudflare/provider accounts, deployed secrets, licensed/private media,
or human review and are not claimed by this local artifact:

- private-bucket/public-access and encryption-at-rest inspection;
- timed positive-cache, cached-404, overwrite, privacy-change, and stale-delete observations on the
  real custom domain;
- provider purge API, backup retention/expiry, restore, and verified erasure rehearsal;
- real signed URL, range, disposition, CORS, CSP, and cache headers at every edge path;
- managed-media private-origin isolation, malformed-media handling, outage, and kill-switch exercise;
- malware scanner corpus and quarantine review;
- signing/purge/provider credential rotation and audit-log inspection;
- production quota/cost alarms, owner/security/privacy approval, and incident drill.

Promotion must attach redacted timestamps, status codes, configuration digests, and reviewer identity.
Do not attach URLs, object keys, tenant data, tokens, request bodies, media, or provider secrets.
