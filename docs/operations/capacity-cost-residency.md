# Capacity, cost, residency, rollback, and self-hosting

Capacity approval requires at least 30% headroom at measured launch load.
Measure request rate, concurrent uploads, queued jobs, native job-seconds, D1
row operations, and R2 bytes for a 30-minute sustained run and five-minute
burst. Break down p50/p95/p99, errors, cancellation, recovery, CPU/GPU/RSS,
scratch space, and queue age by safe service/profile/release dimensions.

Provider tests use synthetic inputs, isolated namespaces, scoped credentials,
concurrency one, a hard timeout, exact cleanup, and a named numeric cost cap.
Record rate-card revision, usage units, estimated/billed amount, and one durable
idempotency key per billable effect. A charge without one terminal state or a
replayed charge blocks release. The checked-in cost maximum is deliberately
null; source control cannot approve spend.

If headroom is below 30%, block promotion and record a scaling action with
owner role, required units, quota lead time, cost delta, completion gate, and
rollback. Never relax timeouts, output validation, or privacy to create
capacity.

Residency defaults to one approved primary location. Backups and provider
execution need explicit location records. Cross-region copies and automatic
failover stay off until customer/security approval, target readiness,
acknowledged-write replay, and reconciliation gates pass.

Self-hosting is an operator-managed preview. It requires compatible web/API,
private S3/R2-like objects, SQLite/D1-like metadata, native media, TLS, a tested
backup target, capacity, and outbound-provider policy. Public buckets, unsigned
desktop updates, telemetry containing personal data, and production managed
media without a provider contract are unsupported. Support bundles use the
allowlisted generator; operators retain their own secrets and incident
contacts.
