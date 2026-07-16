---
title: "Capture Cap parity fixtures, API snapshots, media goldens, and performance baselines"
labels:
  - "phase:p0"
  - "area:test"
  - "area:program"
  - "type:test"
  - "risk:high"
depends_on: [01]
size: epic
---

# 04 · Capture Cap parity fixtures, API snapshots, media goldens, and performance baselines

## Outcome

A reproducible, legally usable oracle allows Frame and Cap behavior to be compared throughout the migration.

## Current Cap reference

The pinned Cap repository includes many Rust recording tests and extensive route, action, workflow, schema, and media behavior, but Frame has no consolidated cross-system fixture corpus or measured migration baseline.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#01](./01-p0-migration-charter-parity-slos.md)

## Scope

Capture sanitized API request/response contracts, route inventories, schema samples, UI journeys, media input/output goldens, failure cases, and timing/resource measurements across the supported matrix.

### Out of scope

Fixing parity gaps belongs to the implementation issue that owns the behavior. Production customer data may not be copied into fixtures.

## Deliverables

- [ ] A machine-readable manifest tied to the Cap SHA, tool versions, OS/hardware, licenses, and checksums.
- [ ] Sanitized API and workflow snapshots covering success, authorization, validation, retry, and error envelopes.
- [ ] Golden video/audio/cursor/edit fixtures with objective probe metadata and perceptual comparison guidance.
- [ ] Baseline reports for resource use, startup, sustained recording, sync, upload, processing, playback, and recovery.
- [ ] A deterministic runner that produces a human-readable parity diff.

## Acceptance criteria

- [ ] Fixture provenance and redistribution rights are recorded; no secret, token, personal data, or customer media is present.
- [ ] Every charter-critical flow has at least one happy-path and one failure-path fixture.
- [ ] Media fixtures cover short, long, variable-frame-rate, multi-monitor, high-DPI, device-loss, sleep/wake, and interrupted-recording cases selected by the charter.
- [ ] Baseline variance and hardware context are reported so regression budgets are statistically meaningful.
- [ ] CI can run a documented fast subset while dedicated runners can execute the full matrix.

## Required test evidence

- Fixture manifest and checksum verification output.
- Baseline report with repeat count and variance.
- A sample Cap-versus-Frame diff containing an intentionally introduced mismatch.

## Risks and open questions

- Golden tests can encode existing bugs as requirements.
- Media artifacts can be large, copyrighted, or hardware-specific.

## Rollout and rollback

Fixtures are versioned and immutable once used for a phase gate. Superseding a fixture requires a change record and preserves the old result for audit.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
