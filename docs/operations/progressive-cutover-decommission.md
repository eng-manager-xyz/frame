# Progressive cutover, rollback, retention, and legacy decommission

This is the Issue-35 production runbook. It joins the metadata authority,
object-backfill, route-family, media, client, and operational-hardening controls
without turning any local test into production evidence. The checked-in policy
can recommend `GO` or `NO_GO`; it cannot change a route, writer, queue, storage
grant, executor, client policy, credential, DNS record, or legacy resource.

The immutable release record is the production authority for a run. It carries
the exact Git SHA, stage, protected cohort digests, approved thresholds,
timestamps, evidence digests, authority snapshots, owner-role attestations, and
the previous compatible rollback pointers. Do not put tenant identifiers,
credentials, raw rows, object keys, signed URLs, media, captions, emails, or
personal contact details in that record or in dashboard output.

## Owners and separation of duties

| Responsibility | Repository role | Required decision |
|---|---|---|
| release sequencing and communication | `release_commander_role` | opens/closes freezes and proposes each stage |
| MySQL/D1 authority and replay | `data_migration_owner_role` | verifies one writer, replay drain, audit chain, and final metadata report |
| objects and source retention | `storage_migration_owner_role` | verifies manifests, provider inventories, source access, and release scope |
| route families and client versions | `application_owner_role` | verifies current/N-1 behavior and legacy write denial |
| jobs and media profiles | `media_operations_owner_role` | verifies per-profile routing, in-flight disposition, quality, capacity, and cost |
| privacy, credentials, and evidence | `security_approver_role` | approves privacy results, evidence handling, rotation, and revocation receipts |
| customer impact | `support_lead_role` | owns notices, exclusions, support budget, and final report |
| rollback execution | `incident_commander_role` | calls stop/rollback and owns the timed timeline |
| irreversible decision | `repository_owner_role` | approves rollback expiry and finalization after independent reviews |

The same actor may operate a control only where the protected policy allows it,
but no operator self-approves the irreversible gate, credential revocation, or
source release. Repository role names are not personal contact information;
the protected incident and approval systems resolve them to current people.

## Command and evidence index

Run the provider-free local gates from the repository root:

```sh
python3 -I scripts/ci/check-cutover-decommission.py
python3 -I scripts/ci/cutover_go_no_go.py --self-test
python3 scripts/ci/cutover-authority-conformance.py
cargo test -p frame-control-plane --test cutover_authority_v1
python3 -I scripts/migration/test_local.py
```

The local dashboard examples are synthetic and are expected to include both
`GO` and `NO_GO` results. A synthetic `GO` proves only that the deterministic
logic recognizes a complete fixture; every report says
`authorizes_transition: false` and `production_authority_changed: false`.

For a protected observation, export an aggregate-only bounded snapshot from the
approved telemetry/evidence system, calculate its immutable digest there, and
run the evaluator in an owner-private encrypted working directory:

```sh
python3 -I scripts/ci/cutover_go_no_go.py \
  --snapshot /protected/cutover/stage-snapshot.json \
  --output /protected/cutover/dashboard-report.json
```

The evaluator returning zero is a recommendation, not a signature. The release
commander attaches the input and output digests to the external stage record;
independent approvers validate source authenticity, freshness, thresholds, and
authority state before any separate control command is submitted.

| Evidence | Checked-in contract | Protected record |
|---|---|---|
| staged policy and dashboard | `fixtures/cutover-decommission/v1/cutover-policy.json` | signed stage input/output and approver digests |
| cohort dimensions | `cohorts.json` | stable tenant digests, consent, exclusions, support owner |
| media routing | `routing-disposition.json` | live flag snapshots, provider/native readiness, in-flight inventory |
| metadata reconciliation | `reconciliation-contract.json` and Issue-16 tooling | final MySQL/D1 manifest and zero-difference report |
| object reconciliation | same contract and Issue-20 core | final provider inventories, manifests, probes, dispositions |
| decommission | `decommission-plan.json` | per-resource action/denial receipts and observation timeline |
| outstanding production work | `protected-evidence.json` | external immutable evidence store entries |

## Freeze, catch-up, and release identity

At `T-7d`, the release commander creates the candidate record and freezes
feature additions. Only an approved blocker fix, security fix, or evidence-only
change can enter; every such change creates a new release SHA, rebuilds the
exact artifacts, reruns gates, and supersedes—not edits—the candidate record.
Schema changes remain expand-compatible. No contract migration, profile
revision, threshold, cohort selector, or source-retention rule changes in place.

At `T-24h`, freeze destructive legacy schema and storage operations, verify the
named previous compatible releases, confirm alert delivery and on-call
acknowledgement, and capture pre-cutover usage/cost baselines. Confirm the
rollback path has enough legacy and native capacity through the rollback
deadline. Freeze does not suppress security incident response or required
customer deletion/legal-hold processing; those events stop the stage and are
handled under their dedicated policy.

At `T-2h`, stop cohort expansion and drain forward replay to the signed lag
budget. Resolve every dead letter and indeterminate provider effect. Run the
charter-critical shadow queries in the current phase window, verify the complete
authority audit chain, and take two complete independent object inventories per
side. A missing observation, stale snapshot, unowned discrepancy, or mutable
evidence pointer is `NO_GO`.

At `T-30m`, the release commander publishes the redacted stage notice, records
the exact proposed authority changes, checks current/N-1 clients and minimum
adoption, and confirms customer/support exclusions. The data, storage, media,
security, support, incident, and final owner roles acknowledge the record.

## Canary cohorts

`cohorts.json` freezes stable, tenant-atomic selection. Production membership is
an explicit protected allowlist for internal/high-risk/BYO cases or an approved
hash bucket for representative hosted cases. Membership cannot be recomputed
with a new salt during a stage. A tenant and route family move atomically; never
sample individual requests in a way that can select two writers.

The complete matrix covers:

- synthetic, internal, and representative tenant classes;
- hosted R2 plus Cap-managed S3, MinIO, BYO S3-compatible, and Google Drive
  migration inputs, with consent/residency gates where required;
- current and N-1 web, macOS desktop, and Windows desktop clients across
  supported Chrome, Firefox, Safari, and Edge combinations;
- managed Cloudflare, native GStreamer, managed-to-native fallback, and
  retained external-provider adapter modes; and
- authorization, privacy, billing, webhook, long upload, cancellation,
  deletion, organization membership, and indeterminate provider workflows.

Active incidents, unresolved retention/legal holds, unsupported adapters,
unsafe client versions, unowned quarantines, absent support ownership, or
unapproved consent/residency exclude a tenant and block any stage that would no
longer be representative. Exclusion is not a way to hide a failing canary.

## Authority controls

Record a before and proposed-after value for every authority dimension. The
metadata field uses the exact Issue-17 scoped phases
`legacy_authoritative`, `shadow_read`, `dual_write`, `d1_authoritative`,
`rolled_back`, and `finalized`; broader program labels never get written into
that table. The other dimensions use their own versioned controls:

| Dimension | Control boundary | Pre-expiry rollback |
|---|---|---|
| metadata | tenant/domain writer and epoch fence | fence D1, reverse-replay acknowledged canary writes, reconcile, restore legacy |
| objects | manifest-bound storage authority/source-release gate | stop new Frame grants, retain all committed objects, reconcile the exact manifest |
| routes | route-family plus tenant cohort flag | disable only the affected family and restore its compatible fallback |
| jobs | job/profile revision plus cohort flag | stop admission and disposition every in-flight attempt before one fenced fallback |
| storage | tenant storage mode and upload-grant policy | stop new grants; preserve completed multipart objects and resolve unknown finalizes |
| executor | per-profile executor revision | disable affected revision; preserve/quarantine any indeterminate attempt |
| clients | minimum version plus additive server capability | restore compatible server capability without repeating any effect |

Each mutation verifies tenant/domain, writer, and exact epoch at the same
transactional boundary. A status read is not a reusable authorization token.
Any evidence of two writers, a stale fence, a write by an excluded client, or a
legacy schedule still publishing work stops expansion immediately.

## Staged timeline and checkpoints

The checked-in basis points are maximum eligible traffic, not a requirement to
fill the bucket. Explicit allowlists always precede percentage selection.

| Stage | Frame maximum | Minimum observation | Authority intent | Exit checkpoint |
|---|---:|---:|---|---|
| `preflight` | 0% | none | legacy writes; Frame shadow only | P0-P5 signed, rollback rehearsal ready, synthetic/local gates green |
| `internal` | 1% | 24h | Frame only for exact internal tenant/family scopes | clean SLO/parity/reconciliation/support window |
| `representative_5` | 5% | 48h | bounded representative scopes | all storage/platform/media/high-risk dimensions represented |
| `representative_25` | 25% | 72h | bounded representative scopes | no unexplained difference; capacity and client budgets hold |
| `majority_50` | 50% | 7d | Frame primary, legacy fenced and replayable | production-scale rollback record remains fresh and viable |
| `full_reversible` | 100% | 14d | Frame authoritative; legacy read-only and retained | final reconciliations, support/cost reports, rollback-expiry proposal |
| `irreversible_finalize` | 100% | separate approval | Frame finalized; retention clock starts | no automatic next stage; controlled decommission follows |

For every stage:

1. Pin the candidate release, previous compatible release, policy/fixture
   digests, cohort record, client versions, metadata epoch, object manifests,
   route/job/storage/executor flags, and provider contract revisions.
2. Run a fresh dashboard evaluation. Review every group—phase readiness, SLOs,
   parity, support, reconciliation, backlog, clients, capacity, rollback,
   managed media, and evidence. Missing and stale values fail.
3. Reconcile all currently acknowledged writes and in-flight jobs. Confirm no
   unowned critical/high blocker and no severity-1/2 cutover case remains open.
4. Obtain the stage record signatures. Submit separate compare-and-swap control
   commands for only the approved scopes; the dashboard never submits them.
5. Immediately verify the exact writer/epoch and flag snapshots, then exercise
   create/upload/process/share/privacy/delete and high-risk workflows for the
   selected cohort. Confirm legacy writes are denied where Frame owns the scope.
6. Hold the entire observation period. Keep privacy, acknowledged-write loss,
   corruption, and duplicate billing/publication budgets at absolute zero.
7. At window end, take a fresh dashboard snapshot and reconciliation. A `GO`
   may propose the next stage; a `NO_GO` pauses expansion and invokes stop or
   rollback according to severity.

Automatic stop conditions include any failed/missing/stale gate, privacy or
cross-tenant exposure, corrupt output, acknowledged-write loss, duplicate
billing/publication, two writers, stale fencing, unexplained parity or
reconciliation, lost rollback readiness, or exceeded provider/native capacity.
The release, support, security, or incident roles can also stop manually when a
change cannot be explained, a provider contract/price changed, customer impact
is unowned, or the evidence chain cannot be verified. Aggregate availability
never overrides a zero-tolerance event.

## Per-profile jobs and in-flight disposition

`routing-disposition.json` binds every profile in
`fixtures/media-jobs/v1/catalog.json`, including all four hybrid managed/native
profiles, ten native-only profiles, and two retained external-provider adapter
profiles. Each starts on the legacy adapter, advances by exact profile revision,
and retains a profile-scoped legacy rollback until expiry. Hybrid operational
failure may select native GStreamer exactly once; that failure fallback is not
the same as a program cutover rollback.

On stop or rollback:

- queued work has its exact lease canceled/expired before one admission on the
  selected rollback executor;
- claimed but unstarted work is fenced and releases capacity before admission;
- started provider/native work is never resubmitted to discover its result;
  query by idempotency and wait for a bounded terminal state;
- staged output publishes only after manifest/checksum and live fence checks;
  otherwise quarantine it without publication;
- published output remains immutable and reconciles to exactly one logical
  publication and billable effect;
- cancellation continues idempotently until terminal, then reconciles staging
  and billing; and
- indeterminate work is quarantined, blocks profile expansion, and requires an
  owner disposition.

Never delete a committed output, overwrite a deterministic key, guess a
provider result, repeat a billable effect, publish a partial artifact, or route
to an unapproved executor. `production_readiness: not_collected` on every
checked-in profile is intentional; live adapter/capacity evidence belongs in
the protected stage record.

## Communications

The release commander and support lead prepare versioned templates before the
freeze. Each message contains the release/stage ID, public impact summary,
start/expected review time, supported client action, status location, and next
update time. It contains no tenant list, private resource title, raw failure,
credential, object locator, or internal approval identity.

- `T-7d`: internal freeze and owner checklist.
- `T-24h`: on-call/support readiness and approved affected-customer notice.
- stage start: redacted authority/cohort summary and next checkpoint.
- stop: affected family/profile, safe symptom class, mitigation, and next
  update; do not speculate about indeterminate writes or provider effects.
- rollback: start/end timestamps, scoped authority restored, canary-write and
  reconciliation status, customer action if any, and incident link.
- full reversible: observation deadline, retained legacy boundary, and explicit
  statement that decommission has not happened.
- finalization/decommission: independent approval, retention boundaries,
  supported clients, cost/support monitoring, and retrospective schedule.

Support volume is evaluated against the signed stage budget. Any open cutover
severity-1/2 case is an automatic stop. Individual cases stay in the protected
support system; the dashboard contains only bounded aggregates.

## Timed rollback rehearsal

Complete a representative production-scale rehearsal before Frame becomes
primary and repeat it when the release, schema, replay adapter, authority
control, job disposition, provider contract, or rollback topology materially
changes. Local SQLite tests are useful but cannot satisfy this requirement.

Start the timer immediately before pausing cohort expansion:

1. freeze new stage admissions and page the incident/data/storage/media owners;
2. snapshot all authority epochs and route/job/storage/executor/client flags;
3. stop new Frame mutations for the scope and disposition every in-flight job;
4. fence D1 at the live epoch, reverse-replay every acknowledged canary write
   in order, and prove target-ledger idempotency after an injected lost ack;
5. reconcile legacy metadata and object/publication state with zero unexplained
   differences, including unknown upload finalize and provider outcomes;
6. submit bounded rollback evidence to the audited control and restore exactly
   one legacy writer plus the approved route/profile fallbacks;
7. prove current and N-1 synthetic reads/writes, privacy/cache revocation,
   playback, upload, and cancellation, then stop the timer; and
8. verify the audit hash chain, retained Frame data, alert timeline, customer
   impact, and absence of repeated billable/provider effects.

The elapsed time must be at most 900,000 ms. The protected report binds the
start/end monotonic timestamps, release/stage/cohort digests, before/after
authority snapshots, canary ledger range, replay/reconciliation digests,
in-flight dispositions, synthetic result, alert delivery, and independent
approval. Do not claim success if a canary write was discarded, manually
patched without evidence, or remains indeterminate.

## Final reconciliation

At the end of `full_reversible`, freeze authority changes and bind metadata and
objects to the same final checkpoint window. For MySQL/D1, compare global,
tenant, table, relationship, and charter-critical-query scopes across row
counts, primary-key sets, foreign-key relationships, field hashes, aggregates,
policy semantics, sampled API behavior, and replay checkpoints. Attach every
reject/quarantine owner disposition and require zero unexplained differences.

For objects, use the exact immutable Issue-20 manifest and two independent
complete inventory passes on each source and target. Compare object and logical
byte counts, per-role counts, full SHA-256, required media probes, missing,
duplicate and orphan objects, ownership, corruption/playability, checkpoints,
publication provenance, and trusted reference/exclusion dispositions for every
provider class. A provider ETag is not a content hash.

The combined immutable manifest binds the release SHA, source snapshot/checkpoint,
target migration level, exact window, stage, tool/input/report digests,
generation time, and approver digests. Reports are aggregate and redacted;
protected rows, raw locators, credentials, and media remain in approved stores.
Reconciliation success changes no authority and triggers no automatic cleanup.

## Irreversible gate

Only `full_reversible` may propose `irreversible_finalize`. The repository owner
acts after independent data, storage, application, media, security, support,
incident, and release reviews. Required protected evidence includes:

- signed P0-P5 and every stage record for the exact release;
- the complete approved observation windows and zero unowned critical/high
  blockers;
- successful production-scale rollback within 15 minutes with preserved writes;
- final MySQL/D1 and object reports with zero unexplained differences;
- explicit rollback-expiry, source-retention/legal-hold, and customer approvals;
- denied legacy writer/client/credential probes and drained scheduled work; and
- active post-cutover monitoring, customer/support plan, and cost baseline.

The automated evaluator checks those fields but always reports
`authorizes_transition: false`. Finalization is a separate authenticated,
audited authority action. If any evidence is missing, stale, mutable, or points
to a different release/stage/window, remain `full_reversible` with legacy
read-only retention. After finalization, recovery follows the Issue-34 backup
and DR procedure; do not invent an undocumented legacy fallback.

## Legacy retention and decommission

`decommission-plan.json` is an inventory and sequence, not an execution script.
Every item is `planned_not_executed`, and destructive actions are deliberately
not automated. Retain rollback sources read-only and encrypted until the signed
expiry; retain migration evidence append-only under approved compliance/incident
policy; retain customer content until migration, customer policy, legal hold,
and verified target rules all permit a separately approved release.

After the irreversible gate, act one item at a time and attach before/after
state, denial/health probes, timestamp, actor digest, exact scope, and receipt:

1. stop legacy web/API and media admission, drain/disposition jobs, observe,
   then scale named services to zero;
2. return an explicit versioned retirement/migration response for supported
   legacy routes before later removal;
3. stop queue publishers, drain leases/retries/dead letters, and separately
   remove queues only when no durable receipt depends on them;
4. revoke the MySQL application writer, keep the encrypted source read-only,
   and perform deletion only under a later retention/legal-hold job;
5. remove application writes from legacy buckets/providers, retain exact
   manifest-scoped source objects, and never run account/prefix-wide cleanup;
6. rotate and revoke each legacy credential capability, prove the old key ID is
   denied, and never revive a secret for rollback—issue a new scoped credential;
7. remove only named legacy DNS/edge records after TTL, certificate, and route
   observation; never change unrelated apex or shop resources;
8. disable schedules, prove two intervals produce no new work, then remove the
   versioned schedule definition;
9. deny unsafe legacy client writes with stable migration guidance only after
   current/N-1/adoption gates pass;
10. keep legacy dashboards/alerts read-only through post-cutover observation,
    verify replacements, then archive with historical links;
11. mark legacy runbooks historical, link the replacement, and retain incident
    context; and
12. compare pre/post usage and invoices, remove idle resources, and close
    provider contracts only after proving no active dependency.

Keep post-cutover SLO, privacy, support, reconciliation, provider cost, and
credential-denial monitoring active. Produce the immutable decommission
checklist, redacted customer/support report, cost delta, and retrospective with
timeline, decisions, SLO/support, parity, reconciliation, rollback readiness,
customer impact, cost, security/privacy, and owned follow-ups.

## Protected status

No production canary, observation window, signed phase/stage record, protected
cohort membership, managed-media cost/quality report, production-scale rollback,
final reconciliation, rollback-expiry approval, source release, legacy write
denial, credential revocation, resource decommission, cost delta, customer
report, or retrospective is claimed by this repository change. The exact
fourteen missing record classes are frozen in
`fixtures/cutover-decommission/v1/protected-evidence.json` and must remain
`not_collected` until their external procedures actually run.

Until those records exist, Frame has a deterministic and reviewable execution
contract, but Issue 35's production outcome and acceptance criteria remain
open. Local evidence may not replace protected evidence.
