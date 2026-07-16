# Object backfill, quarantine, and source-retention runbook

This runbook applies to object-backfill protocol v1. The current repository contains a
provider-free core and deterministic adapters only. Do not start a real migration until the
provider adapters, durable journal, credentials, approvals, budgets, and monitoring listed below
have their protected evidence records.

## Roles and required approvals

Assign distinct people or auditable service principals for:

- migration operator: creates manifests and controls workers;
- storage owner: approves provider/region/locator scope and cost budgets;
- tenant or customer owner: approves BYO/Drive access and quarantine dispositions;
- security owner: approves credential references, runtime scope, logs, and revocation;
- data owner: approves source-retention release and eventual deletion; and
- support lead: owns user-impact lists and customer communication.

An operator cannot infer approval from a successful API call or supply an approval UUID. Store the
real ticket/change record outside the manifest. The application accepts only an opaque owner
capability, and a trusted approval adapter must attest the authenticated subject, exact
manifest/entry/disposition or manifest/report scope, issue time, and expiry before the journal
transition.

## Preconditions

Before inventory, verify and attach evidence for all of these:

1. The source and target provider adapters pass capability conformance against the real accounts.
2. Provider, region, non-secret locator, and authority fingerprints match the approved scope.
3. Credential references resolve through the approved secret store; raw credentials do not appear
   in manifests, commands, logs, crash reports, or support attachments.
4. BYO/custom buckets and Google Drive have explicit customer permission, residency, and egress
   approval. A readable credential is not consent to copy.
5. Tenant allowlists and deny-by-default provider/region controls are active.
6. Object, logical-byte, bandwidth, concurrency, rate, cost, retry, and circuit budgets are approved.
7. Source retention, legal hold, observation window, and rollback dates are recorded.
8. D1 journal backup/restore, worker monitoring, alerts, and emergency pause/abort are rehearsed.
9. A test prefix and non-critical tenant cohort are selected. Production prefixes are excluded.

If any precondition is absent, stop after local/provider read-only validation. Do not construct a
manifest that implies the missing approval exists.

## Stage 1: read-only inventory and manifest review

Inventory every approved source authority independently in two complete stable-snapshot passes.
Reject snapshot changes, repeated or backward cursors, page-index gaps, empty continuation pages,
and rows outside the exact tenant/authority/side scope. Join records to the authoritative tenant,
video, and role ownership data; do not infer ownership from an object-key prefix alone. For every
entry record the exact source reference, canonical target key/revision, logical bytes, full SHA-256,
content type, media-probe policy, and optional opaque provider checksum. Never copy an ETag into the
SHA-256 field.

Persist the manifest through the immutable manifest port and export its digest to the change record.
Review these before initializing workers:

- duplicate source references or target keys;
- objects without authoritative tenant/video ownership;
- missing strong hashes or objects that cannot be streamed once to compute one;
- provider/region/locator drift;
- BYO/Drive objects outside the approved permission or residency scope;
- projected logical bytes, request counts, egress, and cost units; and
- critical roles whose sample playback probe is absent.

A changed manifest is a new manifest ID and approval, never a replacement under an existing ID.

## Stage 2: dry-run reconciliation

Run independent source and target inventory before enabling writes. Generate a reconciliation
report and its pure dry-run repair plan. A report is clean only when counts, logical bytes, role
counts, full hashes, required probes, and dispositions all agree. Save the report digest and
user-impact list.

Expected initial differences are `missing_target`; unexplained duplicate, orphan, ownership,
corruption, unplayable, metadata, or checkpoint differences block writes. A provider-unavailable
classification is not evidence that the object is absent.

The repair plan does not mutate. Review each proposed copy, quarantine, ownership investigation,
or orphan review before enabling workers.

## Stage 3: test-prefix rehearsal

Use a new manifest limited to synthetic objects in the approved test prefix. Exercise at least:

- process interruption after several chunks and deterministic resume;
- two competing workers, cross-entry/cross-tenant concurrency, and lease expiry/reclaim;
- an expired half-open lease owned by one tenant followed by work requested for another tenant,
  proving global reclaim clears the token before tenant selection;
- process death after a provider commit but before its journal checkpoint with every normal attempt
  and run budget already exhausted;
- destination commit with a lost acknowledgement;
- journal CAS commit with a lost acknowledgement;
- truncation, corruption, extra bytes, empty/oversized chunk, throttling, expired authorization,
  provider outage, and cancellation;
- circuit opening/cooldown, a single half-open probe, retry exhaustion, and one bounded
  owner-approved extra attempt;
- pause, resume, abort racing a blocked commit, stale reclaim racing a blocked commit, and read/write
  staging cleanup;
- missing, duplicate, orphan-source, orphan-target, ownership, metadata, inventory/HEAD drift,
  receipt-provenance, and checkpoint reconciliation;
- more-than-one-page inventory plus snapshot mutation, cursor/page faults, and cross-tenant rows;
- sample playback/probe for every real object role and provider class;
- a clean report followed by a retention-gate rehearsal that does not delete source objects;
- reference approval with a verified source and deliberately absent target, exclusion approval for
  both missing and corrupt sources, and forged/expired approval rejection; and
- deterministic disposition report ordering, referenced/excluded totals, and exact journal/report
  evidence matching at the retention gate.

Record throughput, peak memory, logical bytes, provider requests, egress, cost, retry/error rate,
and cleanup lag. Local synthetic results do not satisfy this stage.

## Stage 4: bounded tenant rollout

Start with one non-critical, explicitly approved tenant. Configure limits below the approved
ceiling and keep the source authoritative. Workers must:

- match the exact tenant/provider/region manifest scope;
- capability-preflight before every batch;
- stop admission at object/logical-byte/source-egress/target-cost budgets;
- honor per-object and per-chunk tenant plus source/target provider-region admission and
  cancellation using fresh trusted time;
- use journal CAS and fenced leases;
- conditionally create immutable targets behind a live durable commit fence; and
- require exact target post-read before recording success.

Watch journal age, active/expired leases, retries by class, circuit state, throughput, bytes, cost,
quarantines, source/target inventory lag, and cleanup failures. Pause immediately on ownership drift,
unexpected cost/egress, sustained target corruption, missing source growth, credential alerts, or a
dirty checkpoint comparison.

## Pause, resume, and abort

`pause` stops new claims and makes the next durable heartbeat or live commit-fence check reject
further publication. It does not synchronously erase the lease: workers must unwind and
idempotently release their read/probe/write sessions. Confirm no new claims or publications before
changing limits or provider configuration. The injected clock must never move behind the last
journal transition.

`resume` is allowed only from paused state. Recheck credentials, capabilities, circuit cooldown,
budgets, and the immutable manifest digest first. Never resume by constructing a new mutable
checkpoint in memory.

`abort` durably fences every active lease, signals cancellation, and moves the journal to terminal
aborted state. A destination adapter must call the supplied commit fence immediately before object
publication; a commit already waiting when abort wins must cancel without publishing. Adapters must
idempotently release read sessions and cancel uncommitted staging writes. Abort never deletes a
committed target or a source object. Run reconciliation after abort and retain both sides until an
owner decides their disposition.

## Quarantine and customer/support workflow

Every quarantine record needs a user-impact entry containing tenant/video/role, public-safe failure
class, object criticality, playback impact, first/last observed time, attempts, and support owner.
Keep raw provider errors, object references, hashes, and credentials out of customer-facing text.

Allowed owner dispositions are:

- retry approved: the owner confirms the source/permission/provider condition is corrected and
  authorizes exactly one additional attempt beyond the normal bound;
- reference approved: the owner explicitly keeps an approved source reference instead of a target;
- exclude approved: the owner explicitly accepts exclusion/data loss for the recorded object; or
- pending: no terminal decision and no source-release approval.

Reference or exclusion must include a real owner approval and customer-impact record. The core does
not invent these approvals. A support workaround is not an owner disposition.

The reconciliation meaning of each terminal disposition is precise:

- `reference approved` keeps the entry in expected source counts and requires a fresh streamed
  source hash/probe, but deliberately expects no target. A target at that key is unexplained and
  remains an orphan.
- `exclude approved` removes that entry from migrated source and target expectations only after the
  trusted adapter verifies the exact manifest/entry/disposition capability. Missing or corrupt
  bytes can then be an intentional exclusion, but the entry's count, bytes, role, original failure,
  approval subject/scope, and verification time remain in the report's auditable disposition
  evidence and totals.

Pending, mismatched, forged, or out-of-window approvals do not adjust totals and cannot make a
report clean. An approval is checked while live at the disposition transition; its durable
attestation is subsequently validated against the historical issue/expiry interval and transition
time rather than silently re-authorized by an operator.

## Reconciliation and rollback

After every cohort, inventory source and target independently with two matching complete snapshot
passes, and stream-verify every non-excluded inventory row and expected object. Exact locations
covered by a trusted terminal exclusion are omitted from migrated observations but remain bound to
the disposition evidence and totals; a location still expected by another entry is never hidden.
Zero unexplained differences means:

- exact source and target counts and logical bytes for their disposition-adjusted expectations;
- exact per-role counts;
- one full SHA-256 verification for every expected object;
- every required media probe is playable under the bound profile;
- referenced and excluded object/byte/role totals exactly match the verified journal dispositions;
  and
- no missing, duplicate, orphan, ownership, corruption, unplayable, metadata, checkpoint, or
  provider-unavailable discrepancy.

On a dirty report, pause the cohort and preserve source authority. Rollback consists of stopping new
claims, canceling uncommitted writes, restoring legacy reads to the source, retaining exact committed
targets for investigation, and reconciling both inventories. Do not overwrite an immutable target,
rewrite the manifest, or delete an object to make counts appear clean.

## Source-retention release

Source release is allowed only after journal completion, a cryptographically intact and non-stale
clean reconciliation bound to the exact manifest, the approved observation window, legal-hold
checks, rollback rehearsal, user-impact closure, and an unexpired data-owner capability verified by
the trusted approval adapter. The clean report must carry the exact reference/exclusion evidence
already recorded in the completed journal; omitting or changing an entry, failure, disposition, or
approval blocks release. The v1 coordinator only records the attested `ReleaseApproved` decision;
it does not delete anything.

Before a provider-specific deletion job runs, take and verify a final inventory snapshot, back up
the journal/reports, confirm target playback for critical roles, and require a separate destructive
change approval. Delete in bounded cohorts, log immutable receipts, and reconcile again. Any
unexpected difference stops deletion.

## Current protected evidence status

As of 2026-07-16, this repository has no collected real S3, MinIO, BYO/custom, Drive, or R2
credentials or rehearsal; no production D1 journal; no customer permission/owner disposition; no
production-scale throughput/cost record; and no source-deletion approval. Therefore this runbook is
not authorization to operate a real migration.
