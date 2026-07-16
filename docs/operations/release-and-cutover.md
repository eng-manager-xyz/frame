# Release, cutover, and rollback runbook

This runbook joins issues 34, 35, and 44. It separates deployable application
rollback from durable-data authority and from the EngManager portfolio link.

## Release manifest

Every candidate records:

- Git commit and source cleanliness;
- contract major and fixture digest;
- D1 migration level and expand/contract classification;
- Worker version, Render deploy, desktop/media artifact digests, and SBOM;
- frame-client revision consumed by the portfolio, if enabled;
- synthetic suite result, reconciliation digest, provider profile versions,
  and approver identity;
- the immediately previous compatible release for each deployable layer.

## Pre-production gates

1. Format, warnings-as-errors lint, unit, contract, migration, native/wasm,
   Worker dry-run, web production smoke, GStreamer, dependency-boundary,
   secret, license, vulnerability, and fixture checks pass.
2. Empty and supported-upgrade D1 migrations pass with foreign-key/integrity
   probes. Restore into an isolated target meets RPO/RTO and validates auth,
   row relationships, object manifests, and playback.
3. Current and N-1 consumer fixtures pass. A seeded breaking contract fails.
4. The hermetic walking slice creates, uploads, processes, shares, ranges,
   cancels, retries, and deletes without provider credentials.
5. Protected staging proves the exact Worker route, direct R2 transfer,
   managed/native fallback, Render readiness/shutdown, cache privacy, and
   synthetic playback with bounded cost.
6. Security/privacy, capacity, region/plan, support/on-call, cost/quota, and
   rollback owners approve the immutable release manifest.

## Data authority states

`legacy_authoritative` -> `shadow` -> `dual_write_replay` -> `frame_canary` ->
`frame_authoritative` -> `legacy_read_only` -> `legacy_retained` ->
`legacy_decommissioned`.

Only the audited control may advance a tenant/domain. Fencing occurs before a
writer changes. Each transition records the last source change, replay
checkpoint, comparison digest, mismatch count, rollback deadline, and actor.
An unexplained mismatch, stale replay, failed write, privacy defect, or lost
fence blocks advancement.

## Launch sequence

1. Deploy compatible Worker/API and Render default host. Verify the release
   manifest and external synthetic checks.
2. Attach `frame.engmanager.xyz` DNS-only; wait for Render certificate and
   prove direct HTTPS.
3. Enable Cloudflare proxy in Full (strict), then the broad `/api*` Worker
   route. Prove strict first-segment handling and cache bypass before adding
   performance cache rules.
4. Observe internal canary traffic. Move WAF/rate controls from log to enforce
   only after false-positive review.
5. Add the static portfolio link to a limited surface, then normal placement.
   Status, handoff, browser CORS, and player embed are separate releases.
6. Hold the observation window. Close all critical/high defects and reconcile
   data/objects before disabling the Render default hostname or legacy paths.

## Layered rollback

| Layer | Trigger | Action | Data effect |
|---|---|---|---|
| Optional portfolio data | timeout/staleness/privacy defect | disable poller/status | none; static link remains |
| Portfolio discovery | Frame launch regression | remove flagged link/card | none |
| Optional handoff/embed/CORS | auth/browser/security defect | activate feature kill switch | existing Frame sessions remain valid per policy |
| Cache/WAF/rate | private HIT or false positive | disable exact rule and scoped purge | no D1/R2 deletion |
| Media profile | quota/drift/failure | disable profile and route one fenced fallback | immutable prior outputs remain |
| Worker | API regression | deploy previous compatible version or remove `/api*` route | migrations remain; use forward fix |
| Render | web regression | instant rollback to named deploy; re-enable default host for diagnosis | stateless |
| Edge proxy | TLS/route incident | return CNAME to DNS-only after certificate check | no application data change |
| DNS | unrecoverable host incident | restore/remove exact `frame` record only | no apex/shop change |
| Data authority | mismatch/write failure | fence Frame writer, replay acknowledged changes, return prior authority | preserve both ledgers and all data |
| Credential | suspected disclosure | revoke, rotate, redeploy, audit capabilities and replay | no media deletion |

Removing the Worker route must leave non-API Render traffic intact. No
rollback rewinds a destructive migration, deletes source media, performs a
zone-wide purge, or changes apex/shop records.

## Incident evidence

Monitors label the failing boundary: DNS/TLS, edge route/cache, Render,
Worker/API, D1, R2, Media, native queue, auth/privacy, or consumer contract.
Alerts contain only random request IDs, release/version, coarse operation,
safe error class, and runbook link. A game day seeds each boundary, measures
detection and rollback, and attaches a redacted timeline to the release.
