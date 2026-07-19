# Multi-runtime delivery local evidence

Frame-local policy validates every checked-in workflow, every third-party
action pin, the untrusted/secret boundary, a same-push unfiltered production
sentinel, non-cancelling production concurrency, prebuilt-artifact deployment,
expand-first migration ordering, Render checks-pass authority, independent
canonical smoke, and disjoint account/zone infrastructure ownership.
Serialized protected workflows retain a bounded FIFO with `queue: max`, while
main validation runs use unique run-ID groups and only superseded pull-request
runs may be cancelled.

Reproduce the local evidence with:

```text
python3 -I scripts/ci/check-workflow-policy.py
python3 -I scripts/ci/test-workflow-policy.py
ruby scripts/ci/check-yaml-syntax.rb
ruby scripts/ci/check-render-blueprint.rb
scripts/ci/release-change-plan.sh HEAD HEAD
python3 -I scripts/ci/check-frame-client-contract.py
```

The mutation suite copies the workflow graph to an isolated temporary tree and
proves that a path-filtered production trigger, non-`always()` final sentinel,
delayed `workflow_run` sentinel, advisory release step, and mutable/malformed
action reference are each rejected. It also rejects unconditional main-run
cancellation, ref-grouped main runs, removal of the production queue, and
relocation of fatal-critical GStreamer policy or required media steps into a
different job. It never invokes a provider or exposes a secret.

Provider environment settings, successful and deliberately failed remote
runs, per-SHA Render trigger traces, Workers Builds disablement, canonical
smoke, consumer-repository CI, and a timed rollback remain protected evidence.
Their absence blocks production promotion and is not represented as a local
pass.
