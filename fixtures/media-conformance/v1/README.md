# Media conformance v1 fixtures

This directory is the immutable, synthetic-only definition for Issue 29. It
contains no customer media and no claim that a Cloudflare binding, physical
capture device, hardware codec, permission prompt, or one-hour soak ran.

`matrix.json` derives the test dimensions and cases from the migration charter
and the versioned media-service contract. `offline-cases.json` drives the
deterministic comparator suite. `fuzz-corpus.json` freezes non-secret parser and
state-machine seeds. `protected-lanes.json` is a fail-closed admission plan: all
protected records remain `not_collected` until a trusted run supplies an
artifact digest and provenance through the documented review process.

Run `python3 -I scripts/ci/check-media-conformance.py` for strict fixture and
source validation. Add `--evidence` and `--dashboard` to write ignored local
JSON artifacts. Never edit v1 evidence after a release consumes it; create a
new version instead.
