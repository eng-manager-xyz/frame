# Leptos authenticated route cutover and rollback

This runbook promotes Issue 31 route families without creating a second data
authority. It applies after the local contract gate and every protected item in
the route matrix has evidence for the exact release SHA.

## Preconditions

1. Pin the immutable Frame release, route matrix digest, API/workflow parity
   report, D1 migration version, and legacy web revision.
2. Confirm the selected route family's typed load/action adapters pass
   owner/admin/member, denial/non-disclosure, validation, duplicate/retry,
   conflict, unavailable, and N/N-1 client cases.
3. Confirm real session bootstrap and every mutation use same-origin cookies,
   CSRF, tenant-scoped idempotency, expected revisions, and privacy-safe audit.
4. Attach current/previous browser visual, automated accessibility, manual
   keyboard/screen-reader, and protected p95 reports. Billing or provider
   families require their protected sandbox evidence and reconciliation.
5. Verify the legacy fallback is healthy and the rollback flag changes routing
   only. D1/R2 remain authoritative and no reverse migration is planned.

## Canary promotion

1. Enable one `web.*.v1` flag for the synthetic tenant, then one internal
   canary tenant. Never enable a mutation route independently of its matching
   authoritative API adapter.
2. Exercise canonical and legacy deep links, session expiry, denied roles,
   browser back/forward, refresh during pending work, duplicate form submit,
   retry after a safe failure, and an unsaved settings edit.
3. Check that private documents are `no-store`/`noindex`, no response or trace
   contains identity/token/provider/private-resource data, and unknown versus
   forbidden resources remain indistinguishable.
4. Compare API decisions and durable results with the legacy shadow record.
   Any unexplained authorization, billing, import, membership, or storage
   difference stops promotion; do not repair it in the browser.
5. Observe the declared window before expanding a single family. Record flag
   actor, tenant scope, time, release SHA, matrix digest, and evidence links.

## Rollback

1. Disable only the affected route-family flag and confirm canonical and legacy
   links reach the approved fallback or explicit maintenance response.
2. Leave D1/R2 migrations and acknowledged writes intact. Do not delete,
   rewind, reissue a billable action, or permit both legacy and Frame writers.
3. Reconcile every submitted idempotency key and expected revision before
   retry. An unknown result remains indeterminate until the authority is read.
4. Confirm private cache revocation inside 60 seconds and route rollback inside
   the charter's 15-minute objective. Preserve evidence and open an incident
   for any privacy, authorization, corruption, duplicate-billing, or lost-write
   symptom regardless of aggregate error rate.

## Current status

Local route and rendering contracts are available, but production adapters,
cross-browser/manual accessibility evidence, protected performance/provider
evidence, flag operation, and timed rollback are pending. This runbook must not
be used to claim production cutover until those conditions are satisfied.
