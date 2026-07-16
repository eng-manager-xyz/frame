# Parity fixture evidence

The `parity-v1` corpus is the locally reproducible evidence seed for issues 04
and 29. It contains generated JSON metadata only. It does not contain Cap
artifacts, customer data, recorded media, credentials, or results from a real
provider or capture device.

Run the local gate from the repository root:

```sh
python3 scripts/ci/check-parity-evidence.py
```

The command verifies every artifact SHA-256 and byte count, rejects duplicate
JSON keys and sensitive content, recomputes baseline statistics, checks the
charter-critical scenario matrix, and executes both comparator controls. A
valid corpus reports the fast lane as `PASS`, the deliberate mismatch gate as
`FIRED`, and the full lane as `BLOCKED`.

To assert that release-quality protected evidence exists, run:

```sh
python3 scripts/ci/check-parity-evidence.py --require-full
```

That command intentionally exits with status 2 while the corpus contains only
protected placeholders. Provider and representative-hardware jobs must attach
their immutable records to the protected release evidence described in
[Verification evidence](README.md); local metadata can never satisfy that
gate.

## Evidence classes and lanes

| Class | Meaning | May satisfy |
|---|---|---|
| `test_definition` | Generated request/oracle metadata; no observed result | Scenario inventory only |
| `local_simulated` | Deterministic local comparator or statistical sample | Fast contract gate |
| `protected_placeholder` | A required context with all observation/result fields null | Nothing; blocks full promotion |

The fast lane covers happy and failure cases for auth, tenant isolation,
upload, media jobs, public sharing, and authority cutover. Its baseline samples
use a fixed seed and fixed sequences; their means and population variances are
useful for proving the statistical machinery, not for claiming real latency,
memory, quality, or synchronization performance.

The full lane adds native OS/hardware capture and soak contexts for macOS,
Windows, and Linux plus isolated Cloudflare Media and cross-executor contexts.
Those entries deliberately have `status: not_collected`,
`is_live_evidence: false`, null observation fields, and a blocking promotion
effect. They must not be edited into synthetic passes.

## Comparator controls

`diffs/cap-frame-match.json` is a positive control whose declared values agree.
`diffs/cap-frame-intentional-mismatch.json` is a negative control that models a
privacy regression: a non-public share is exposed instead of remaining
unavailable. The checker independently recomputes both comparisons and fails
unless the positive control passes and the exact declared negative-control
mismatches fire.

Both controls are declared expected behavior generated for this repository;
neither claims to be captured output from the pinned Cap revision.

## Provenance, privacy, and immutability

[`manifest.json`](../../fixtures/parity/v1/manifest.json) records the pinned Cap
revision used only as an oracle identity, generated-data provenance,
redistribution scope, privacy assertions, lanes, and immutable artifact hashes.
The checker rejects unlisted files, path traversal, symlinks, media payloads,
workstation paths, email-like values, secret-bearing markers, and sensitive
keys.

Once a phase gate consumes `parity-v1`, do not rewrite its artifacts or refresh
hashes in place. Create `fixtures/parity/v2`, document the reason for
supersession, preserve the old corpus, and review any new provenance or
redistribution terms. Protected provider/hardware evidence remains outside the
local corpus and must retain its own tool, runtime, OS, hardware, provider,
profile, usage, cost, timestamp, and artifact digests.
