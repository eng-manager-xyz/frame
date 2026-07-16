---
title: "Define the migration charter, compatibility matrix, and measurable success SLOs"
labels:
  - "phase:p0"
  - "area:architecture"
  - "area:program"
  - "type:planning"
  - "risk:high"
depends_on: []
size: epic
---

# 01 · Define the migration charter, compatibility matrix, and measurable success SLOs

## Outcome

A signed migration charter defines what Frame will preserve, deliberately change, defer, or remove, and gives every later issue measurable parity and reliability targets.

## Current Cap reference

Cap at 6ba6956 is a mixed system: Tauri 2 plus substantial Rust media code, Next/React and TypeScript control-plane code, MySQL/Drizzle, S3-compatible and Google Drive storage, and native FFmpeg-oriented paths. Calling this a generic Rust rewrite would erase critical product and licensing decisions.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

None.

## Scope

Inventory user journeys, APIs, desktop targets, media formats, storage providers, data domains, integrations, self-hosting behavior, and operational promises. Establish platform support, service-level indicators, regression budgets, and provenance rules.

### Out of scope

Selecting detailed implementations for D1, object storage, GStreamer pipelines, or Leptos routing belongs to their dependent ADRs and implementation issues.

## Deliverables

- [ ] A versioned capability matrix with preserve, replace, change, defer, and retire dispositions.
- [ ] Supported OS, architecture, browser, codec/container, storage-provider, deployment, and self-hosting matrices.
- [ ] Baseline SLIs and approved regression budgets for recording quality, A/V sync, startup, processing latency, upload reliability, API latency, availability, accessibility, and recovery.
- [ ] A migration glossary, ownership map, decision log, and upstream provenance/license policy.
- [ ] A prioritized vertical-slice plan beginning with create → upload → process → share.

## Acceptance criteria

- [ ] Every externally visible Cap capability discovered at the pinned commit has an owner and disposition; no row is left unclassified.
- [ ] The charter states whether existing Rust crates are retained, adapted, replaced, or evaluated individually.
- [ ] Success and rollback thresholds use measured baselines or an explicitly approved product requirement, not arbitrary numbers.
- [ ] Storage-provider and self-hosting regressions are explicitly accepted or prohibited.
- [ ] Engineering, product, security, operations, and legal/provenance owners approve the document.

## Required test evidence

- Published inventory scripts or queries and their raw counts.
- A reviewable baseline report linked to issue 04.
- Recorded sign-off and unresolved decisions with owners and due dates.

## Risks and open questions

- Scope can expand indefinitely unless non-goals and phase gates are enforced.
- Upstream mixed licensing can constrain source reuse even when behavior is compatible.

## Rollout and rollback

Documentation-only. Revert the charter if approval is withdrawn; dependent implementation cannot enter production until a replacement charter is approved.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
