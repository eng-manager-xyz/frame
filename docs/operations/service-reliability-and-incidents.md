# Service reliability, observability, on-call, and incidents

The machine-readable service catalog and charter SLOs live in
`fixtures/operational-hardening/v1/service-catalog.json`. Owners are roles, not
personal contact data. The protected contact registry must resolve every role,
exercise acknowledgement, and retain its result outside the repository.

Telemetry is allowlisted. It carries a random UUIDv4 correlation ID, full
release SHA, environment, service, coarse operation/result class, duration,
and bounded job fields. It never carries media, captions, bodies, provider
messages, tokens, signed URLs, raw email, private titles, tenant/user IDs,
object keys, or filesystem paths. Generate a support artifact only through:

```sh
python3 -I scripts/ci/support-bundle.py \
  --events target/safe-events.ndjson --output target/support-bundle.json
```

Unknown fields and non-catalog operations fail closed. Dashboards show
availability, latency, error budget, dependencies, queue age, reconciliation,
cost, and capacity using bounded dimensions. Alert pages include only service,
release, environment, safe class, random correlation ID, and runbook.

## Incident protocol

1. Open an incident ID, assign incident commander, operations, communications,
   and scribe roles, and preserve the first alert/failure.
2. Freeze promotion; classify worker, D1, queue/job, object, media worker,
   desktop update, client, auth/privacy, or edge/web boundary.
3. Apply the narrow kill switch or compatible rollback. Preserve durable state
   and use forward schema fixes.
4. Reconcile acknowledged writes, D1 rows, manifests, final/staging objects,
   provider usage, and billing before declaring recovery.
5. Confirm SLO recovery, close temporary access, rotate exposed credentials,
   and retain a redacted timeline with owner/action follow-ups.

### Worker or API

Check readiness, safe result-class rate, release revision, dependency panels,
and D1/R2/Media health. Stop rollout before retrying. Roll back the Worker only
to a schema-compatible digest; never bypass auth, replay, or authority fences.

### D1

Stop new authority transitions. Distinguish contention, migration mismatch,
quota, and unavailable service using aggregate signals. Reclaim only expired
leases and restore only into an isolated database per the DR runbook.

### Queue or job

Alert on oldest ready job, expired lease, recovery-required age, dead letters,
and charge-without-terminal-state. Fence stale attempts. Do not reset attempts,
resubmit indeterminate external effects, or publish a partial output.

### Object

Compare the authoritative manifest to exact HEAD/range observations; provider
listings are diagnostic only. Disable publication or signing on checksum,
privacy, hold, or generation drift. Never guess prefixes for cleanup.

### Media worker

Stop claims, allow bounded lease expiry, quarantine unverified staging, and
enable only cataloged capacity. Capture coarse graph/error class, not command
lines, paths, plugin environment, or media.

### Desktop update

Stop the affected channel, retain the updater manifest and signature result,
and compare clean-machine aggregate failures by OS/architecture/release. Never
disable signature verification. Use the previous signed compatible channel or
forward fix.

### Client contract

Confirm contract major, release, safe decode class, and N/N-1 fixtures. Disable
only the optional integration surface. Do not log response bodies or turn an
unknown enum/field into work.

Dashboard delivery, pager acknowledgement, screenshots, and human game-day
timelines are protected evidence and are not inferred from local simulations.
