# Authentication recovery and operator runbook

This runbook covers privacy-safe diagnosis and recovery for Frame authentication. It assumes the
protocol in [Authentication protocol and threat model](../security/authentication-protocol.md).
Commands that mutate production D1, provider configuration, or session authority require the
protected environment, an authorized operator, and a recorded correlation/change ID.

## Non-negotiable handling rules

- Never print, decrypt for inspection, or attach a raw session/API key, OTP, OAuth code/verifier,
  cookie, delivery destination, sealed envelope, provider token, or hash-key material.
- Use aggregate counts, stable reason codes, opaque IDs, lease timestamps, and correlation IDs.
- Do not “fix” a user by inserting an identity, provider account, tenant grant, session, or
  verification row. Use the repository workflow or the incident remains an authority bypass.
- Preserve the audit/outbox transaction. A manual data edit that omits its audit event is invalid.
- A rollback restores one prior authority. It never enables two independent session writers.

## Triage

1. Record incident/change ID, environment, affected client kind, first/last observed timestamps,
   release revision, and stable public error class.
2. Check dependency readiness and aggregate counts: active/revoked sessions, pending/leased
   deliveries, pending OAuth flows/reservations, rate buckets, and recent audit reason counts.
3. Confirm clock health before interpreting expiry or lease state.
4. Determine scope: one user, one client kind/provider, one tenant, or global.
5. Stop rollout if there is replay, cross-user linkage, audit loss, plaintext leakage, an unbounded
   queue, or an unexplained global denial increase.

## Common recovery procedures

### Delivery backlog or dispatcher crash

1. Verify the durable outbox has bounded pending rows and inspect counts by state/age only.
2. Confirm expired leases are reclaimable and suppressed/expired deliveries are being removed.
3. Restart or replace the dispatcher; do not reset attempts or copy sealed payloads into logs.
4. Allow fenced leases to reclaim work. A stale worker acknowledgement must remain a no-op.
5. If the provider is unavailable, schedule bounded retries. Exhausted entries require a new user
   request; never extend the original verification expiry.

### Logout-all or recovery dispute

1. Locate the correlation ID and confirm an allow audit for logout-all/recovery.
2. Confirm the user session version advanced and every prior active session is revoked.
3. Confirm session-bound OTP/OAuth link continuations and reservations were purged.
4. Attempt a synthetic replay of an old credential in the isolated environment; it must deny with
   a stable revoked/version/replay reason and must not create a new continuation.
5. Never decrement a session version. If access must resume, complete a new verification/recovery.

### Suspected credential or hash-key exposure

1. Stop distribution and revoke the exposed credential class at its authority.
2. For a user credential, revoke the session family or increment the user session version.
3. For a hash key, introduce a new active version, retain the minimum approved fallback window,
   and monitor active-key migration. Rate histories must merge rather than reset.
4. Remove a fallback version only after every supported client/migration gate passes or after an
   approved forced-login decision.
5. Run the secret scanner and review artifacts/log exports; record evidence without the value.

### OAuth provider outage or callback mismatch

1. Separate `AdapterFailure` from invalid state/PKCE/redirect/audience reasons.
2. Confirm the configured callback and audience exactly match the provider environment.
3. Do not relax state, PKCE S256, callback, audience, expiry, or one-time reservation checks.
4. A consumed or failed flow is not manually resurrected. Start a new flow after recovery.
5. For account linking, confirm the originating session is still active before retrying.

### Rate-limit saturation

1. Inspect counts by action/dimension, never raw identifier/source/device digests.
2. Confirm the global bucket denies before new attacker-controlled buckets are allocated.
3. Confirm expired buckets are collected and total bucket cardinality remains within the hard cap.
4. Do not clear a single target’s history to make a login succeed. Adjust policy only through a
   reviewed release and preserve global/source/device protection.

## Session migration rehearsal

1. Export a redacted source inventory containing only counts by format/client/state/expiry class.
2. Run compatibility fixtures for valid, expired, revoked, rotated/replayed, version-mismatched,
   and unknown credentials for every supported client.
3. Exercise the one-time migration grant with concurrent replay and rollback injection.
4. Verify the Frame expiry never exceeds the source absolute expiry.
5. Rehearse disabling the legacy validator and, separately, restoring it as the sole validator.
6. Publish the approved forced-login communication if any class cannot migrate safely.

## Exit criteria

Recovery is complete only when the original invariant passes, queues/rate state are bounded, audit
coverage is intact, secrets were not exposed during diagnosis, synthetic replay is denied, and an
owner records the outcome. Production authorization additionally requires the D1/provider and
cross-client protected evidence listed in `docs/evidence/README.md`.
