# Share/player rollout and rollback

This runbook promotes the v1 contract in privacy-first phases. A later phase
may begin only when the prior phase has its named production evidence. The
public page must never call Media Transformations or a native executor per
viewer request; readiness is based on a persisted derivative descriptor.

## Preflight

1. Run the local gate in `docs/evidence/share-player-local.md` from a clean,
   locked checkout.
2. Verify the production Worker adapter returns byte-equivalent unavailable
   bodies for missing, private, cross-tenant, failed, deleted, and revoked
   shares. Confirm logs contain only bounded error classes and digests.
3. Capture R2 GET/HEAD range traces for 0-, open-ended, suffix, `If-Range`,
   validator mismatch, invalid, and multi-range requests. Confirm no object key
   or signed URL appears in HTML, JSON, redirects, logs, or `postMessage`.
4. Capture crawler output for public, unlisted, processing, password, private,
   deleted, and embed responses. Only public ready top-level content may emit
   public title/description/OpenGraph fields.
5. Exercise Chromium, Firefox, WebKit, mobile Safari, and mobile Chromium with
   captions, speed, seek, fullscreen/PiP support detection, retry, reduced
   motion, keyboard, VoiceOver/TalkBack, and the supported screen reader.
6. Prove comment and analytics replay/rate fences in production-like D1 and
   validate transcript moderation/retention jobs. Consent denial must produce
   no analytics record.
7. Verify each custom domain through the ownership workflow, then test revoked,
   cross-tenant, wildcard, subdomain, Host-header, canonical, cookie, CSP, and
   cache isolation.

## Promotion

1. Deploy alternate internal share routes with embeds, comments, mutations,
   analytics, custom domains, and password verification disabled.
2. Enable public ready top-level rendering for synthetic/internal tenants.
   Compare status, range bytes, duration, captions, crawler fields, cache
   headers, and object access with the legacy route.
3. Canary a small percentage of default-domain public links. Hold for at least
   the maximum CDN invalidation window plus one analytics/replay window.
4. Enable unlisted and processing states. Re-run public-to-private and deletion
   probes continuously; any stale title, media byte, caption, transcript, or
   comment is a stop condition.
5. Enable exact-origin embeds for one reviewed ancestor. Run hostile
   `postMessage` origins/sources/schemas/replays and CSP framing probes.
6. Enable password and authenticated private top-level routes only after the
   password service, session isolation, enumeration, and recovery review.
7. Enable read-only transcript/comments, then moderated mutations, then
   explicit-consent analytics. Enable custom domains one verified binding at a
   time.

## Abort and rollback

Immediately disable the affected surface on any metadata/object-key leak,
cross-tenant result, stale privacy/delete result, unexpected analytics record,
wildcard frame/message origin, range corruption, or accessibility stop-ship.

- Disable comments/analytics mutations independently; preserve authorized
  read/export paths.
- Disable embed without disabling top-level public shares.
- Revoke the custom-domain binding and purge its exact host partition.
- Route public rendering to the legacy player only when the legacy path passes
  the same current authorization decision. Never fall back around a denial.
- For any uncertain state, return the generic unavailable response with
  `no-store`, `noindex,nofollow`, no media/caption/transcript request, and no
  OpenGraph fields.

After rollback, purge the exact tenant/share/privacy/deletion/domain revisions,
verify the issue 21 invalidation SLO from default and custom domains, preserve
redacted traces, and require security/operations approval before resuming.
