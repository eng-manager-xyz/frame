# Frame migration charter

This charter is the authority for the Cap-to-Frame migration. It fixes the
scope, compatibility rules, measurable gates, and rollback boundaries used by
issues 01–44. A code path is not considered complete merely because it builds:
its contract, privacy behavior, failure behavior, and rollback must also pass.

## Reference and provenance

- Behavior reference: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.
- Portfolio reference:
  `matthewharwood/engmanager.xyz@1de52bc8f25793dea3697e67765d53785c05cdfa`.
- Reference checkouts live under ignored `.tmp/` directories. They are inputs
  to inventories and compatibility tests, never vendored dependencies.
- New Frame code is independently implemented unless a file records the exact
  upstream path, commit, license, and adaptation obligations.
- Only synthetic or explicitly redistributable fixtures may enter the
  repository, CI artifacts, dashboards, or support bundles.

## Product boundary

Frame retains the recording, editing, upload, processing, sharing,
collaboration, organization, developer, and administrative workflows listed
in the [capability matrix](capability-matrix.md). The migration replaces the
web/control plane with Rust, Leptos, D1, and R2 while keeping native media work
behind GStreamer. Retained clients receive versioned compatibility contracts;
unretained behavior receives an explicit retirement response and migration
path.

Cloudflare Media Transformations is a derivative executor, not the product's
video library. The `[stream]` managed-library binding is disabled. Capture,
timeline composition, unsupported formats, long or complex work, and provider
fallback remain native GStreamer responsibilities.

`frame.engmanager.xyz` is a dedicated Frame origin. The EngManager portfolio
links to it through top-level navigation and has no request-time, cookie,
identity, availability, or media-processing dependency on Frame. Optional
status, handoff, CORS, and player embedding are independently gated and remain
off until their security tests pass.

## Supported release matrix

| Surface | Initial support | Gate |
|---|---|---|
| Web | Current and previous stable Chrome, Firefox, Safari, and Edge | Contract, browser, accessibility, privacy, and performance suites |
| Desktop | Current macOS and Windows; Linux remains an explicit preview until capture packaging evidence passes | Signed build, clean-machine install, permissions, capture, A/V sync, recovery, and updater tests |
| API | `/api/v1`, current and N-1 released clients | Golden fixtures, consumer builds, additive-change tests, breaking-version rejection |
| Media | WebM/VP8/Opus baseline; H.264/AAC inputs and MP4 outputs where licensed and runtime-supported | Metadata, seek, A/V sync, perceptual, and fallback conformance |
| Storage | Hosted private R2 is authoritative | Tenant isolation, conditional/range/multipart, reconciliation, deletion, hold, and restore tests |
| Legacy storage | S3-compatible, MinIO, user-owned buckets, and Drive are migration inputs until an owner approves a retained adapter | Inventory, capability preflight, export/backfill, and customer-impact record |

Unsupported capabilities fail during preflight with a stable public code. They
never silently select a weaker privacy, durability, or output contract.

## Service objectives

The values below are release gates for synthetic and canary traffic. A change
to a target requires a decision-log entry; missing data does not authorize
relaxing the target.

| Indicator | Objective | Window / measurement |
|---|---:|---|
| Public landing and share availability | 99.9% | rolling 30 days, edge probes |
| Versioned API availability | 99.9% | rolling 30 days, non-rate-limited requests |
| Landing response latency | p95 <= 750 ms | edge-to-browser synthetic |
| API response latency | p95 <= 500 ms | excluding asynchronous media completion |
| Upload-intent/finalize success | 99.9% | synthetic direct-R2 journey |
| Public playback start | p95 <= 2 s | warm CDN, supported browser/network profile |
| 60-second 1080p recording time-to-share | p95 <= 30 s | accepted input profile, capacity available |
| A/V start offset | absolute <= 80 ms | declared device matrix |
| A/V drift | absolute <= 50 ms after 60 minutes | declared device matrix |
| Data/object reconciliation | zero unexplained differences | every migration/cutover gate |
| Durable-state RPO | <= 5 minutes | backup and replay exercise |
| Service RTO | <= 60 minutes | isolated restore exercise |
| Application rollback | <= 15 minutes | timed production-shaped rehearsal |
| Privacy/cache revocation | <= 60 seconds | private/delete transition probe |

Error budgets never excuse a privacy leak, cross-tenant access, corrupted
media, duplicate billing, or loss of an acknowledged write. Those are
release-blocking incidents independent of aggregate availability.

## Resource and cost budgets

- Every remote media invocation records executor, profile version, input and
  output bytes, elapsed time, result class, and a non-sensitive correlation
  identifier.
- Normal pull requests use provider-neutral fakes. Billable R2/Media tests run
  only from a protected environment with synthetic fixtures, isolated
  prefixes, concurrency one, a hard timeout, and explicit cleanup.
- Managed Media has a per-profile kill switch. Quota, beta behavior, billing
  drift, unsupported input, or output-conformance failure routes exactly one
  retry-safe result to native GStreamer or returns a stable failure.
- No automated cleanup may enumerate or delete outside its test namespace.
- Capacity approval requires 30% headroom at the measured launch load and a
  monthly provider estimate attached to the release record.

## Compatibility rules

1. Public JSON is versioned by major API path and additive within a major.
   Consumers ignore unknown fields and capabilities. Unknown values never
   trigger work.
2. D1 changes follow expand, migrate, observe, then contract. A deployed Worker
   must remain compatible with the live N-1 web client and schema.
3. Commands and callbacks use tenant-scoped idempotency keys. Repetition may
   return the prior result but may not repeat a billable or durable effect.
4. Object keys are tenant-scoped, immutable, versioned, and non-sensitive.
   Provider etags are validators, not assumed content hashes.
5. Provider, database, object key, signed URL, raw body, cookie, token, email,
   and private-title details stay out of public errors and telemetry.
6. Legacy routes either pass their frozen fixture or return a documented
   retirement/migration response. Accidental 404 parity is not retirement.

## Authority and rollback

At every cutover boundary exactly one system is authoritative. Shadow reads
and replay do not create a second writer. Authority transitions are scoped,
audited, fenced, reversible during the observation window, and reconciled
before the next transition.

Rollback restores a compatible application release and replays acknowledged
writes. It never deletes D1/R2 data, rewinds a destructive migration, publishes
a partial object, or changes unrelated EngManager DNS/cache resources. Schema
problems use a forward fix. Object formats remain readable for the documented
retention window.

## Approval record

The repository owner is the final product and release authority for this
program. Security-sensitive changes require the threat-model checklist;
storage and data changes require reconciliation/restore evidence; adapted
upstream code requires provenance review; production deployment requires the
protected environment and its reviewer. The release manifest records the
approving actor and immutable Git SHA rather than embedding names in this
charter.
