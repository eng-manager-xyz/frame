# Cloudflare cache, R2, WAF, and rollback runbook

## Authority adoption

1. Inventory every DNS record, zone setting, Worker route, and ordered rule in
   each cache/WAF/rate phase. Name the portfolio zone state and Frame account
   state keys, backends, token owners, backups, lock behavior, and reviewers.
2. In the portfolio state, model and import each complete phase entrypoint plus
   apex, `www`, shop/store, Stripe, and existing Worker behavior. The first plan
   must be a no-op or contain only reviewed representation changes.
3. Disable every legacy whole-phase PUT in the portfolio bootstrap before an
   authoritative apply. Never leave a fallback that can overwrite Terraform.
4. Add the exact Frame CNAME DNS-only, verify CAA/AAAA and Render certificate,
   then proxy with Full (strict). Add Frame entries inside the existing ordered
   phase resources; do not create another phase entrypoint.
5. Import the existing private R2 bucket into the disjoint Frame account state.
   Review adoption of CORS/lifecycle resources if the provider cannot import
   them, apply the saved plan manually, and require a zero-drift follow-up.

Staging and production use separate remote state keys, bucket names, exact
origins, credentials, and protected environments. Remote state is encrypted,
locked, versioned, access logged, and backed up. A scheduled trusted plan
alerts on drift but never auto-applies or destroys resources.

## Effective cache verification

The origin emits `no-store` on every Worker response and private/dynamic web
page. Hydration assets use content fingerprints plus
`public, max-age=31536000, immutable`. Cloudflare starts bypass-first and must
not override Origin Cache Control, `Set-Cookie`, authorization, or private/no-
store responses.

For every method/host/auth/cookie/privacy/range/upgrade variant, request twice
with unique synthetic IDs and retain status, safe headers, age, cache status,
and body digest only. API/auth/private/upload/health/mutation responses must be
`DYNAMIC` or `BYPASS`, never `HIT`. A fingerprinted asset must progress to
`HIT`; a changed fingerprint must return new bytes. Public share HTML remains
bypassed until a separate privacy/cache review enables it.

On privacy change or deletion, enumerate canonical `/s/{id}`, `/embed/{id}`,
and any approved immutable derivative URLs or versioned `frame:` tags. Run:

```text
python3 -I scripts/ops/frame_cache_purge.py --url https://frame.engmanager.xyz/s/ID
python3 -I scripts/ops/frame_cache_purge.py --url https://frame.engmanager.xyz/s/ID --apply --confirm-host frame.engmanager.xyz
```

The first command is a dry-run. Apply requires protected
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ZONE_ID`. Query strings, credentials,
other hosts, wildcards, duplicates, more than 30 entries, and non-`frame:` tags
fail locally. The tool never sends `purge_everything`.

## Direct transfer and CORS

The private bucket admits only environment-specific exact Frame origins,
GET/HEAD/PUT, and the small checksum/content/range header allowlist. It exposes
only range, length, and ETag response headers. There is no wildcard or
portfolio origin, and the bucket/custom domain remains private.

Runtime intents default to the existing brokered transport and accept an
explicit `direct` mode only when the Worker signing variables/secrets are
present. Local contracts create an opaque random staging key and bind PUT,
exact content length/type/checksum, no-overwrite, and five-minute expiry;
finalize HEAD-verifies staging, reserves quota, conditionally publishes the
canonical private object, and commits D1 authority before cleanup. The
scheduled reconciler deletes expired staging after a short in-flight grace.
See `docs/evidence/direct-upload-local.md` for the exact contract.

Production direct mode remains disabled until a security owner approves and
proves credential rotation, hosted SigV4/checksum behavior, the CORS abuse
matrix, observed metadata finalize, idempotency, quota, quarantine, and
lifecycle behavior. A signed URL remains reusable until expiry; no one-use or
provider byte-cap claim is made until the hosted R2 controls prove it.

## WAF and rate limits

Create independent Frame-host budgets for auth, OTP, upload intent, finalize,
comments, views/reactions, and job mutation. Combine trusted edge/session/IP
signals; a public share ID is never the sole limiter. Observe and dashboard
first, review false positives across normal navigation/range/reconnect flows,
then enable challenge/block with a named owner, dated threshold, expiry, and
one-rule kill switch. Cloudflare Access may protect staging/admin diagnostics,
not the public app/player.

## Restore and rollback

Restore a copied state backup into an isolated backend, refresh read-only, and
prove resource identity before any apply. Roll back the smallest Frame entry:
disable an exact cache/WAF/rate rule, purge exact Frame variants, remove the
Worker route, or return the Frame CNAME to DNS-only. Never restore a stale
whole-phase ruleset, touch apex/shop, make R2 public, delete the bucket, or use
a zone-wide purge. Retain the before/after plan, state version, rule IDs,
operator, timing, cache/CORS probes, and incident decision.
