# Frame security and privacy threat model

This model covers the public Frame host, portfolio boundary, Worker API,
Render web service, direct R2 transfer, media executors, and migration tools.
The default is same-origin Frame traffic with top-level portfolio navigation;
optional integration capabilities are disabled until their individual gates
pass.

## Trust boundaries

1. A browser and every sibling `*.engmanager.xyz` host are mutually
   untrusted. Same registrable domain is not shared identity.
2. Cloudflare terminates public traffic and routes API paths to the Worker;
   Render serves non-API HTML/assets. Neither origin trusts client forwarding
   headers.
3. The Worker owns D1/R2 bindings, auth decisions, upload intents, finalize,
   and public-data classification. Render owns no storage administrator token.
4. Direct R2 URLs are short-lived bearer capabilities constrained to method,
   key, headers, size/checksum policy, and expiry. Finalize independently
   verifies the resulting object.
5. Native media workers receive scoped job credentials and immutable object
   references, not production database access or browser sessions.
6. Migration tools run with isolated source/target scopes and produce redacted
   manifests. CI and previews never receive production credentials or data.

## Required controls

### Browser and identity

The detailed client flows, capability boundaries, abuse controls, and legacy-session migration
rules are defined in [Authentication protocol and threat model](authentication-protocol.md).

- Session cookies are host-only, `Secure`, `HttpOnly`, and `SameSite=Lax` or
  stricter. No `Domain=engmanager.xyz` cookie is permitted.
- Every mutation requires an unguessable CSRF token, exact expected `Origin`,
  and compatible Fetch Metadata. Missing or `null` origins fail closed outside
  explicitly reviewed non-browser clients.
- Return URLs are relative Frame paths from an allowlist. Protocol-relative,
  credentialed, encoded-host, user-info, alternate-port, and sibling-origin
  targets are rejected.
- Recorder, dashboard, account, and auth pages deny framing. The public player
  uses exact `frame-ancestors` origins only if embedding is enabled.
- CSP and Permissions Policy do not grant camera, microphone, display capture,
  geolocation, payment, or broad `https:` access to embedded content.
- `postMessage` checks exact origin, source window, protocol version, message
  type, size, one-use nonce where applicable, and replay state.

### API and cache

- The normal Frame browser API is same-origin and emits no CORS headers.
  Optional portfolio/browser clients use an exact-origin allowlist, explicit
  methods/headers, `Vary: Origin`, and never combine wildcard origin with
  credentials.
- `/api`, auth, account, upload, finalize, mutation, private share, health,
  error, WebSocket, and SSE responses are `no-store`. Origin privacy headers
  remain authoritative even when edge rules change.
- Only fingerprinted immutable assets receive a one-year public policy.
  Explicitly public share metadata has a short, reviewed policy and a scoped
  purge path. A cookie or authorization header always bypasses cache.
- Public errors use stable safe codes and random request IDs. They do not
  disclose existence across tenants or include stack traces, SQL, provider
  messages, object keys, signed URLs, bodies, cookies, tokens, emails, or
  private titles.

### Upload, object, and media

- Keys are derived from validated tenant/video/revision/role/profile values;
  clients cannot choose arbitrary prefixes. Published outputs are immutable.
- Multipart session, part, complete, abort, and finalize operations are
  tenant-scoped and idempotent. Expired, altered, wrong-method, and cross-
  tenant capabilities fail without an existence oracle.
- Finalize verifies method-bound intent, size, strong checksum when available,
  content type, media probe, part manifest, and unclaimed immutable target.
- Media parsing runs with bounded time, memory, disk, output, subprocess, and
  codec/plugin policy. Cancellation fences publication and cleans partials.
- Hold and deletion discover objects from manifests, never prefix guesses.
  Holds block both lifecycle and user deletion; deletion is tombstoned,
  idempotent, reconciled, and privacy-safe.

### Operations and supply chain

- Third-party Actions are pinned to full commit SHAs. Production secrets are
  environment-scoped and unavailable to pull requests and untrusted code.
- Worker, Render, and zone infrastructure each have one deployment authority.
  Concurrency prevents overlapping production migrations/releases.
- Telemetry uses random correlation IDs and bounded cardinality. Raw media,
  captions, personal identifiers, internal keys, signed URLs, bodies, and
  credentials are forbidden in logs, traces, metrics, alerts, artifacts, and
  support bundles.
- Credential exposure triggers revocation, rotation, audit, replay/fence
  review, cache purge only where needed, and a recorded incident timeline.

## Verification matrix

Release tests cover cross-tenant enumeration and mutation; session fixation,
replay, logout-all, and CSRF; open redirects and host poisoning; exact CORS and
preflight; CSP/framing/permissions; hostile messages; cache variance; signed
upload alteration; duplicate/out-of-order callbacks; untrusted media; secret
redaction; and forked-workflow credential isolation. Optional handoff, CORS,
and embed features have independent kill switches and remain off when their
matrix is incomplete.
