# Managed Media change watch and game day

Review official Cloudflare documentation and account notices at least every
seven days while managed profiles are enabled. Record observation time,
contract/limit/pricing/region digests, breaking-change decision, and owner role.
An overdue or unexplained change disables affected profile revisions; it does
not silently update limits.

The managed profiles are `optimized_clip_v1`, `thumbnail_v1`,
`spritesheet_v1`, and `audio_extract_v1`. Each has an independent revision kill
switch and native GStreamer fallback. Alert on quota, outage/error rate, output
drift, usage/cost, charge without terminal state, fallback capacity, duplicate
publication, and manifest/object drift.

Run the credential-free state-machine game with:

```sh
python3 -I scripts/ci/operational-game.py \
  --evidence target/evidence/operational-game-local.json
```

It seeds outage, quota, and breaking output for every profile and proves exact
profile disablement, one fenced native fallback under duplicate delivery, one
deterministic final key, one logical publication/billable effect, and staging
reconciliation. It also localizes Worker, D1, queue/job, object, native worker,
desktop-update, and client faults and exercises capacity/region decisions. It
does not call Cloudflare or prove alert delivery.

## Protected remote game

Require a protected environment, numeric cost approval, synthetic fixture,
isolated prefix, concurrency one, hard timeout, scoped cleanup, and adequate
native fallback capacity. Record immutable release/profile/provider contract
revisions before invocation. Seed outage, quota, timeout, cancellation, output
drift, duplicate/out-of-order delivery, and recovery.

Disable the affected revision before routing new work. Fence in-flight attempts
and quarantine unverified outputs. A fallback is native or the explicitly
retained legacy adapter—never a weaker unknown executor. Reconcile D1 state,
usage/billing, staging and final HEADs, exact manifests, checksum/provenance,
and cancellation. Replay must not repeat provider invocation, billing, or
publication. Restore managed routing only after contract/output/cost probes and
owner approval.

## Native capacity and fallback

Reserve native units for each enabled managed profile, measure sustained and
burst fallover with 30% headroom, and alert before exhaustion. When capacity is
insufficient, reject with a stable class or queue within the approved SLO; do
not create unbounded queues, overwrite outputs, or retry an indeterminate
effect.
