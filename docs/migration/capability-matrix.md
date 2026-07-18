# Capability and parity matrix

This matrix classifies the externally visible capability groups in the pinned
Cap reference. The linked issue owns implementation detail; the parity column
defines the evidence required before authority moves. “Replace” means preserve
the user contract with a new implementation. “Retain” means reuse a reviewed
native concept behind Frame contracts. “Migrate” means a temporary importer or
compatibility path. “Retire” requires an explicit response and customer path.

| Capability | Disposition | Frame owner | Required parity / failure evidence | Rollback |
|---|---|---|---|---|
| Sign-in, OAuth, magic links, sessions, account linking, logout, recovery | Replace | 13, 30, 31 | success/expiry/replay/fixation/CSRF/enumeration fixtures and compatible-session decision | old session verifier during overlap or approved forced login |
| Organizations, membership, invites, ownership, roles | Replace | 14, 30, 31 | exhaustive role/action matrix, cross-tenant negatives, invite/transfer races | authority flag per tenant and replayed writes |
| Spaces, folders, membership, video moves | Replace | 14, 30, 31 | policy/query conformance and concurrent move/removal invariants | retain source mapping and reversible authority |
| Video create, title, metadata, state, delete/restore | Replace | 06, 12, 15, 30 | lifecycle table, idempotency, privacy, retention, unknown-ID nondisclosure | source authority plus change replay |
| Screen/window/region capture and cursor modes | Retain behavior; replace adapters | 22–25, 33 | geometry, DPI, rotation, multi-monitor, permissions, device loss, exclusion | old desktop remains selectable until hardware gate |
| Microphone, system audio, camera, device settings | Retain behavior; replace adapters | 22–25, 33 | permissions/hotplug/default-device/Bluetooth/sleep and screen-only fallback | old desktop and safe source-disable paths |
| Recording pause/resume, clocks, A/V sync | Replace pipeline | 23, 25, 29 | offset/drift/discontinuity/long-duration tests | fall back to prior native pipeline |
| Instant segmentation, spool, reconnect, finalize | Replace | 19, 23, 26, 30 | crash boundary, offline quota, duplicate/out-of-order and no-resurrection tests | local spool remains recoverable; legacy finalize compatibility |
| Studio projects, isolated tracks, preview, edit, export | Replace with versioned importer | 23, 27, 33 | non-mutating legacy import report, journal recovery, preview/export goldens | preserve source project and old editor selection |
| Upload intent, multipart, resume, abort, finalize | Replace | 18, 19, 30 | conditional/multipart/range/CORS/auth/idempotency matrix | abort new sessions; preserve verified parts for recovery |
| Object storage and manifests | Replace with private R2 | 02, 18–21 | key/capability contracts, checksums, reconciliation, hold/delete/restore | source retention plus immutable migration manifest |
| S3-compatible, MinIO, BYO bucket, Drive | Migrate; retained adapters require separate approval | 02, 18–21 | capability preflight, credential isolation, export/backfill disposition | keep source until reconciliation and retention gate |
| Thumbnails, clips, spritesheets, audio extraction | Replace with capability routing | 03, 07, 28, 29 | managed/native cross-executor metadata and perceptual tolerances | profile kill switch to deterministic native fallback |
| Complex/long transcode, repair, composition | Retain native responsibility | 22, 23, 27–29 | codec/limit/fault/resource/cancel/soak matrix | prior native profile or stable unsupported result |
| Share links, privacy, passwords, deleted/processing states | Replace | 15, 21, 30, 32 | public/private metadata leak matrix, legacy URLs, cache revocation | versioned legacy resolver and cache purge |
| Player, range/seek, captions, fullscreen/PiP | Replace | 19, 29, 32 | supported browser/device, range validators, captions/a11y/recovery | previous compatible player bundle |
| Embeds and postMessage | Disabled initially; gated replacement | 32, 42, 43 | exact-origin/frame-ancestor/message schema/replay tests | kill switch leaves top-level share navigation |
| Comments, transcripts, moderation, analytics | Replace where retained | 15, 30, 32 | tenant/privacy/rate/replay/consent/retention tests | disable mutation, preserve readable export |
| Notifications and asynchronous workflows | Replace with outbox/idempotent handlers | 15, 30 | duplicate/out-of-order/provider failure and redaction tests | pause consumers and replay durable outbox |
| Imports and external storage integrations | Migrate/replace by provider decision | 15, 20, 30, 31 | credential scope, retry, quarantine, reconciliation | stop importer; source remains authoritative |
| Developer apps, domains, API keys, credits, usage | Replace | 13, 15, 30, 31 | key revocation, tenant scope, append-only ledger reconciliation | freeze mutations and export ledger |
| Billing/admin workflows | Replace only after explicit sandbox parity | 15, 30, 31 | webhook replay/signature, authorization, ledger, failure matrix | disable mutations; provider remains source of truth |
| Messenger/support data | Migrate for export; product surface off by default | 11, 15, 30 | schema/reconciliation/export and explicit retirement response | source retention per legal policy |
| Web auth/dashboard/settings/library surfaces | Replace with Leptos | 08, 31 | route-role matrix, forms, SSR privacy, a11y, performance | previous web release and compatible Worker |
| Desktop recorder/editor/settings UI | Replace with Leptos/Tauri | 08, 33 | typed IPC capability, lifecycle, recovery, a11y, OS matrix | previous signed desktop channel |
| Public API, server actions, webhooks, cron | Replace under `/api/v1` | 06, 07, 30, 36 | route/action inventory, stable errors, auth/idempotency/security and N/N-1 | parallel API major/deprecation window |
| Portfolio discovery link | New, independent integration | 37, 42–44 | cross-origin navigation and complete Frame-outage isolation | remove optional status, then link |

## Storage compatibility disposition

- Hosted Frame writes only to private R2 through the Worker or a narrowly
  scoped upload broker. The Render web process never receives bucket admin
  credentials and never proxies media bodies.
- Existing S3-compatible, MinIO, BYO, and Drive objects remain readable by the
  migration tool until their immutable manifests reconcile. New writes are
  rejected unless a retained adapter explicitly declares every required
  capability before upload starts.
- Credentials are references to a secret store, never manifest fields. A
  provider reader may list only its approved source scope. Reconciliation
  compares logical role, bytes, strong checksum when available, and media
  probes for critical objects.
- Source deletion is a separate, delayed approval after customer impact,
  legal hold, restore, and rollback windows close.

## Managed/native executor decision

Cloudflare Media is eligible only when the versioned profile's exact input,
duration, size, resolution, codec, output, privacy, region, and cost limits are
known to pass. A preflight miss selects native GStreamer before publication.
Provider failure may select the declared fallback once; deterministic output
keys and terminal-result fencing prevent two published results.
